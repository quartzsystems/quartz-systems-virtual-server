// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The channel: the harness's way to the control socket and the files
//! of a running host, over the second serial port. PID 1 supervises it
//! like any service.

use std::convert::Infallible;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use netd::control::Calls as _;
use rustix::fs::OFlags;
use rustix::process::{Pid, Signal, kill_process};
use rustix::termios::{OptionalActions, tcgetattr, tcsetattr};
use state::Store;
use supervise::control::{Calls as _, Client};

use crate::proto::{END, PORT, command, parse_status, status_lines, word};

/// The largest file `cat` sends.
const CAT_MAX: u64 = 1 << 20;

fn ok() -> Vec<String> {
    vec![word::OK.to_string()]
}

fn error(why: impl std::fmt::Display) -> Vec<String> {
    vec![format!("{} {why}", word::ERROR)]
}

fn done<E: std::fmt::Display>(result: Result<(), E>) -> Vec<String> {
    result.map_or_else(error, |()| ok())
}

fn control() -> Client {
    Client::new(release::CONTROL_SOCKET)
}

fn status() -> Vec<String> {
    control()
        .status()
        .map_or_else(error, |status| status_lines(&status))
}

fn kill(service: &str) -> Vec<String> {
    let status = match parse_status(&status()) {
        Ok(status) => status,
        Err(e) => return error(e),
    };
    let pid = status
        .service(service)
        .and_then(|s| i32::try_from(s.pid).ok())
        .and_then(Pid::from_raw);
    match pid {
        Some(pid) => done(kill_process(pid, Signal::KILL)),
        None => error(format!("{service} has no process")),
    }
}

fn cat(path: &str) -> Vec<String> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > CAT_MAX => error(format!("{path}: {} bytes", meta.len())),
        Ok(_) => fs::read(path).map_or_else(
            |e| error(format!("{path}: {e}")),
            |bytes| vec![format!("{} {}", word::HEX, hex::encode(bytes))],
        ),
        Err(e) => error(format!("{path}: {e}")),
    }
}

fn put(rest: &str) -> Vec<String> {
    let Some((path, contents)) = rest.split_once(' ') else {
        return error("put <path> <hex>");
    };
    match hex::decode(contents) {
        Ok(bytes) => done(state::atomic_write(Path::new(path), &bytes)),
        Err(e) => error(e),
    }
}

fn ls(dir: &str) -> Vec<String> {
    match fs::read_dir(dir) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| format!("{} {}", word::NAME, e.file_name().to_string_lossy()))
                .collect();
            names.sort();
            names
        }
        Err(e) => error(format!("{dir}: {e}")),
    }
}

/// The kernel log's lines that contain `text`, read from its start.
fn kmsg(text: &str) -> Vec<String> {
    let flags = i32::try_from(OFlags::NONBLOCK.bits()).unwrap_or(0);
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open("/dev/kmsg");
    let mut log = match opened {
        Ok(log) => log,
        Err(e) => return error(e),
    };
    let mut lines = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        match log.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let record = String::from_utf8_lossy(&buf[..n]);
                let said = record.split_once(';').map_or("", |(_, rest)| rest);
                let said = said.lines().next().unwrap_or("");
                if said.contains(text) {
                    lines.push(format!("{} {said}", word::LINE));
                }
            }
            // Records overwritten before they were read.
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
            Err(_) => break,
        }
    }
    lines
}

/// The answer to one command line.
fn answer(line: &str) -> Vec<String> {
    let (name, rest) = line.split_once(' ').unwrap_or((line, ""));
    match name {
        command::PING => vec![format!("pong {rest}").trim_end().to_string()],
        command::STATUS => status(),
        command::KILL => kill(rest),
        command::CAT => cat(rest),
        command::PUT => put(rest),
        command::LS => ls(rest),
        command::KMSG => kmsg(rest),
        command::POWER_OFF => done(control().power_off()),
        command::RESTART => done(control().restart()),
        command::NET_RELOAD => {
            let socket = release::appliance(|_| {}, |_, _| {}).net_socket();
            match netd::control::Client::new(socket).reload() {
                Ok(result) => done(result),
                Err(e) => error(e),
            }
        }
        other => error(format!("no command `{other}`")),
    }
}

/// The serial port as bytes: no echo, no line editing, no translation.
fn raw(port: &File) {
    if let Ok(mut settings) = tcgetattr(port) {
        settings.make_raw();
        let _ = tcsetattr(port, OptionalActions::Now, &settings);
    }
}

/// Run the channel. Says it is ready once the port is open.
pub fn run() -> io::Result<Infallible> {
    let port = OpenOptions::new().read(true).write(true).open(PORT)?;
    raw(&port);
    if crate::on(crate::CHURN) {
        std::thread::spawn(|| {
            let appliance = release::appliance(|_| {}, |_, _| {});
            crate::churn::run(&Store::at(appliance.state_dir()))
        });
    }
    supervise::ready::signal()?;
    let mut reader = BufReader::new(port.try_clone()?);
    let mut port = port;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let command = line.trim();
        if command.is_empty() {
            continue;
        }
        let mut out = answer(command).join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(END);
        out.push('\n');
        port.write_all(out.as_bytes())?;
        port.flush()?;
    }
}
