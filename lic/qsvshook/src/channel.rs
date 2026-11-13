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

use stgd::Calls as _;

use crate::proto::{END, PORT, command, parse_status, status_lines, storage_lines, word};

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
    // PID 1 itself is asked to shut down, as an operator's `kill` asks.
    if service == command::INIT {
        return done(kill_process(Pid::INIT, Signal::TERM));
    }
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

fn storage() -> Vec<String> {
    stgd::Client::of(&release::appliance(|_| {}, |_, _| {}))
        .status()
        .map_or_else(error, |status| storage_lines(&status))
}

/// Megabytes the load writes each turn; 0 stops it.
static LOAD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Where the load writes.
const LOAD_DIR: &str = "/data/load";

/// Write files under `/data` for as long as [`LOAD`] says, each one
/// whole and synced before the next: `file-<n>` holds the byte `n`
/// over and over, so what a later read finds can be checked.
fn load_writer() {
    use std::sync::atomic::Ordering;
    let _ = fs::create_dir_all(LOAD_DIR);
    let mut n: u64 = 0;
    loop {
        let megabytes = LOAD.load(Ordering::Relaxed);
        if megabytes == 0 {
            return;
        }
        n += 1;
        let path = format!("{LOAD_DIR}/file-{n}");
        let chunk = vec![u8::try_from(n % 251).unwrap_or(0); 1 << 20];
        let written = File::create(&path).and_then(|mut f| {
            for _ in 0..megabytes {
                f.write_all(&chunk)?;
            }
            f.sync_all()
        });
        // The name of each file that is whole goes to a list, synced too.
        if written.is_ok() {
            let _ = OpenOptions::new()
                .create(true)
                .append(true)
                .open(format!("{LOAD_DIR}/done"))
                .and_then(|mut f| {
                    writeln!(f, "{n} {megabytes}")?;
                    f.sync_all()
                });
        }
        // Old files go, so the disk does not fill.
        if n > 4 {
            let _ = fs::remove_file(format!("{LOAD_DIR}/file-{}", n - 4));
        }
    }
}

fn load(megabytes: &str) -> Vec<String> {
    use std::sync::atomic::Ordering;
    let Ok(megabytes) = megabytes.trim().parse::<u64>() else {
        return error("load <megabytes>");
    };
    let before = LOAD.swap(megabytes, Ordering::Relaxed);
    if before == 0 && megabytes > 0 {
        std::thread::spawn(load_writer);
    }
    ok()
}

/// A file's length and the sum of its bytes.
fn sum(path: &str) -> Vec<String> {
    match fs::read(path) {
        Ok(bytes) => {
            let sum: u64 = bytes.iter().map(|b| u64::from(*b)).sum();
            vec![format!("{} {} {sum}", word::OK, bytes.len())]
        }
        Err(e) => error(format!("{path}: {e}")),
    }
}

/// The partition hook's commands.
fn partition(rest: &str) -> Vec<String> {
    use crate::partition::{END_FILE, PID_FILE, START_FILE, runner};
    let (verb, argument) = rest.split_once(' ').unwrap_or((rest, ""));
    let pid = || -> Option<Pid> {
        let text = fs::read_to_string(PID_FILE).ok()?;
        Pid::from_raw(text.trim().parse().ok()?)
    };
    match verb {
        "start" => done(fs::write(START_FILE, b"")),
        "pid" => pid().map_or_else(
            || error("no partition is running"),
            |pid| vec![format!("{} {}", word::OK, pid.as_raw_nonzero())],
        ),
        "kill" => {
            let signal = match argument {
                "TERM" => Signal::TERM,
                "KILL" => Signal::KILL,
                other => return error(format!("partition kill TERM|KILL, not {other}")),
            };
            match pid() {
                Some(pid) => done(kill_process(pid, signal)),
                None => error("no partition is running"),
            }
        }
        "send" => {
            let socket = runner::console_socket(&runner::profile_name());
            done(
                std::os::unix::net::UnixStream::connect(socket).and_then(|mut console| {
                    console.write_all(
                        format!(
                            "{argument}
"
                        )
                        .as_bytes(),
                    )
                }),
            )
        }
        "end" => fs::read_to_string(END_FILE).map_or_else(
            |_| error("the partition has not ended"),
            |how| vec![format!("{} {}", word::OK, how.trim())],
        ),
        other => error(format!("partition start|pid|kill|send|end, not {other}")),
    }
}

/// Mount the xfs file system on a device read-only: a guest's, to see
/// from the host what the guest wrote.
fn mount_ro(rest: &str) -> Vec<String> {
    use rustix::mount::{MountFlags, mount};
    let Some((device, directory)) = rest.split_once(' ') else {
        return error("mount <device> <directory>");
    };
    if let Err(e) = fs::create_dir_all(directory) {
        return error(e);
    }
    let flags = MountFlags::RDONLY | MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC;
    done(mount(device, directory, "xfs", flags, None))
}

fn scsi_probe(device: &str) -> Vec<String> {
    match crate::partition::probe::probe(Path::new(device)) {
        Ok(lines) => lines
            .into_iter()
            .map(|line| format!("{} {line}", word::LINE))
            .chain(ok())
            .collect(),
        Err(e) => error(e),
    }
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
        command::STORAGE => storage(),
        command::LOAD => load(rest),
        command::SUM => sum(rest),
        command::PARTITION => partition(rest),
        command::MOUNT => mount_ro(rest),
        command::UNMOUNT => done(rustix::mount::unmount(
            rest,
            rustix::mount::UnmountFlags::empty(),
        )),
        command::SCSI_PROBE => scsi_probe(rest),
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
