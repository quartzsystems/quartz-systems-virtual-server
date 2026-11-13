// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! `qsvshook partition`: the service that runs the one partition of a
//! test image. With no profile volume it does nothing. With one it
//! starts `qsvmm` at once, and again whenever the channel asks.

use std::convert::Infallible;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use qsvmm::end::End;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use supervise::control::{Calls as _, Client};

use super::{
    END_FILE, PID_FILE, PROFILE, PROFILE_FILE, START_FILE, VMM, VOLUME_LABEL, code_of, relayed,
    reported,
};

/// The host's serial port.
const HOST_SERIAL: &str = "/dev/ttyS0";
/// Where the profile volume is mounted for the read.
const MOUNT: &str = "/run/partition-volume";
/// The name a partition goes by when its profile gives none.
const NAMELESS: &str = "PARTITION";

/// The profile on the volume, when there is one.
fn profile_from_volume() -> Option<String> {
    let device = seed::volume::find(VOLUME_LABEL)?;
    fs::create_dir_all(MOUNT).ok()?;
    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC;
    mount(&device, MOUNT, "vfat", flags, None).ok()?;
    let text = fs::read_to_string(Path::new(MOUNT).join(PROFILE_FILE)).ok();
    let _ = unmount(MOUNT, UnmountFlags::empty());
    let _ = fs::remove_dir(MOUNT);
    text
}

/// The console socket of the partition called `name`.
pub fn console_socket(name: &str) -> PathBuf {
    Path::new(release::RUN_DIR)
        .join("partitions")
        .join(name)
        .join("console.sock")
}

/// The name in the profile the host has now.
pub fn profile_name() -> String {
    fs::read_to_string(PROFILE)
        .ok()
        .and_then(|text| qsvmm::profile::parse(&text).ok())
        .map_or_else(|| NAMELESS.to_string(), |p| p.name)
}

fn serial_line(line: &str) {
    if let Ok(mut port) = OpenOptions::new().write(true).open(HOST_SERIAL) {
        let _ = port.write_all(format!("{line}\r\n").as_bytes());
    }
}

fn post(code: &str, text: &str) {
    // PID 1 serves one request at a time: a busy moment is waited out.
    for _ in 0..10 {
        if Client::new(release::CONTROL_SOCKET)
            .post(code.to_string(), text.to_string())
            .is_ok()
        {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    eprintln!("PID 1 did not take: {code} {text}");
}

/// Post the codes `qsvmm` reports; relay its other lines.
fn follow_reports(name: String, stderr: impl Read + Send + 'static) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            match code_of(&line) {
                Some((code, text)) => post(code, text),
                None => serial_line(&reported(&name, &line)),
            }
        }
    })
}

/// Relay the partition's serial output, a line at a time, for as long
/// as the console socket is there.
fn follow_serial(name: String) {
    thread::spawn(move || {
        let socket = console_socket(&name);
        let mut console = None;
        for _ in 0..200 {
            match UnixStream::connect(&socket) {
                Ok(stream) => {
                    console = Some(stream);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(25)),
            }
        }
        let Some(console) = console else {
            return;
        };
        let mut line = Vec::new();
        let mut reader = BufReader::new(console);
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    let text = String::from_utf8_lossy(&line);
                    serial_line(&relayed(&name, text.trim_end_matches(['\r', '\n'])));
                }
            }
        }
    });
}

/// Run `qsvmm` once, to its end.
fn run_once() {
    let name = profile_name();
    let _ = fs::remove_file(END_FILE);
    let spawned = Command::new(VMM)
        .args(["--profile", PROFILE])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            post(
                refcode::PARTITION_NOT_IPLED.code,
                &refcode::PARTITION_NOT_IPLED.text_with(&format!("{name}, {VMM}: {e}")),
            );
            let _ = fs::write(END_FILE, "exit none\n");
            return;
        }
    };
    let _ = fs::write(PID_FILE, format!("{}\n", child.id()));
    let reports = child.stderr.take().map(|e| follow_reports(name.clone(), e));
    follow_serial(name.clone());
    let status = child.wait();
    let _ = fs::remove_file(PID_FILE);
    // Every code `qsvmm` reported is posted before its end is.
    if let Some(reports) = reports {
        let _ = reports.join();
    }
    let (end, how) = match status {
        Ok(status) => match (status.code(), status.signal()) {
            (Some(code), _) => (End::from_exit(Some(code)), format!("exit {code}")),
            (None, Some(signal)) => (End::Killed, format!("signal {signal}")),
            (None, None) => (End::Killed, "signal ?".to_string()),
        },
        Err(e) => (End::VmmError, format!("wait: {e}")),
    };
    let code = end.code();
    post(code.code, &code.text_with(&format!("{name}, {how}")));
    let _ = fs::write(END_FILE, format!("{how}\n"));
}

/// Run the service. Says it is ready at once: the host's base does
/// not wait for a partition.
pub fn run() -> io::Result<Infallible> {
    supervise::ready::signal()?;
    let _ = fs::remove_file(START_FILE);
    let mut start = match profile_from_volume() {
        Some(text) => {
            fs::write(PROFILE, text)?;
            true
        }
        None => false,
    };
    loop {
        if start {
            run_once();
        }
        // The channel asks for a start by making the file.
        while File::open(START_FILE).is_err() {
            thread::sleep(Duration::from_millis(100));
        }
        let _ = fs::remove_file(START_FILE);
        start = true;
    }
}
