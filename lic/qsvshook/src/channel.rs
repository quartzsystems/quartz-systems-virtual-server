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

/// One request to the partition manager, as the harness encoded it.
fn partd_request(rest: &str) -> Vec<String> {
    let Ok(request) = hex::decode(rest.trim()) else {
        return error("partd <hex>");
    };
    crate::partition::relay::client()
        .exchange(&request)
        .map_or_else(error, |answer| {
            vec![format!("{} {}", word::HEX, hex::encode(answer))]
        })
}

/// The partition hook's commands: what the harness does to a VMM
/// that no request to the partition manager does.
fn partition(rest: &str) -> Vec<String> {
    use crate::partition::relay;
    use partd::Calls as _;
    let mut words = rest.splitn(3, ' ');
    let (verb, id) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let argument = words.next().unwrap_or("");
    let Ok(id) = id.parse::<u32>() else {
        return error("partition pid|kill|send <id> ...");
    };
    let pid = || -> Option<Pid> {
        let all = relay::client().list().ok()?;
        let pid = all.iter().find(|p| p.id == id)?.pid;
        Pid::from_raw(i32::try_from(pid).ok()?)
    };
    match verb {
        "pid" => pid().map_or_else(
            || error("the partition has no VMM"),
            |pid| vec![format!("{} {}", word::OK, pid.as_raw_nonzero())],
        ),
        "kill" => {
            let signal = match argument {
                "TERM" => Signal::TERM,
                "KILL" => Signal::KILL,
                // Has `qsvmm` say what state its processors are in.
                "USR1" => Signal::USR1,
                other => return error(format!("partition kill <id> TERM|KILL|USR1, not {other}")),
            };
            match pid() {
                Some(pid) => done(kill_process(pid, signal)),
                None => error("the partition has no VMM"),
            }
        }
        "send" => done(
            std::os::unix::net::UnixStream::connect(relay::console_socket(id))
                .and_then(|mut console| console.write_all(format!("{argument}\n").as_bytes())),
        ),
        other => error(format!("partition pid|kill|send, not {other}")),
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
    // A guest's file system may have the identifier of the host's own:
    // both come from one build.
    done(mount(device, directory, "xfs", flags, Some(c"nouuid")))
}

/// Copy bytes between files and devices: a megabyte at a time, and
/// into a file a megabyte of zeros is left a hole.
fn copy(rest: &str) -> Vec<String> {
    use std::os::unix::fs::FileExt;
    let words: Vec<&str> = rest.split(' ').collect();
    let [from, skip, to, seek, bytes] = words.as_slice() else {
        return error("copy <from> <skip> <to> <seek> <bytes>");
    };
    let (Ok(skip), Ok(seek), Ok(bytes)) = (
        skip.parse::<u64>(),
        seek.parse::<u64>(),
        bytes.parse::<u64>(),
    ) else {
        return error("copy <from> <skip> <to> <seek> <bytes>");
    };
    let run = || -> io::Result<u64> {
        let source = File::open(from)?;
        let target = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(to)?;
        let holes = target.metadata()?.file_type().is_file();
        if holes && target.metadata()?.len() < seek + bytes {
            target.set_len(seek + bytes)?;
        }
        let mut chunk = vec![0u8; 1 << 20];
        let (mut at, mut written) = (0u64, 0u64);
        while at < bytes {
            let n = usize::try_from((bytes - at).min(1 << 20)).unwrap_or(1 << 20);
            source.read_exact_at(&mut chunk[..n], skip + at)?;
            if !holes || chunk[..n].iter().any(|b| *b != 0) {
                target.write_all_at(&chunk[..n], seek + at)?;
                written += n as u64;
            }
            at += n as u64;
        }
        target.sync_all()?;
        Ok(written)
    };
    match run() {
        Ok(written) => vec![format!("{} {written}", word::OK)],
        Err(e) => error(e),
    }
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

/// A field of a `/proc/<pid>/status`.
fn status_field<'t>(status: &'t str, name: &str) -> &'t str {
    status
        .lines()
        .find_map(|l| l.strip_prefix(name)?.strip_prefix(':'))
        .map_or("", str::trim)
}

/// The state and the processor last run on, from a task's `stat`: the
/// first and the thirty-seventh field after the name, and its user and
/// system ticks, the twelfth and thirteenth.
fn stat_fields(stat: &str) -> (String, String, u64) {
    let rest: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap_or("")
        .split_ascii_whitespace()
        .collect();
    let field = |i: usize| rest.get(i).copied().unwrap_or("?").to_string();
    let ticks = |i: usize| rest.get(i).and_then(|t| t.parse::<u64>().ok()).unwrap_or(0);
    (field(0), field(36), ticks(11) + ticks(12))
}

/// The threads of a process: `line <tid> <state> <processor> <ticks>
/// <allowed> <name>` for each.
fn threads(rest: &str) -> Vec<String> {
    let dir = format!("/proc/{}/task", rest.trim());
    let Ok(entries) = fs::read_dir(&dir) else {
        return error(format!("{dir}: no such process"));
    };
    let mut lines = Vec::new();
    for entry in entries.flatten() {
        let tid = entry.file_name().to_string_lossy().into_owned();
        let status = fs::read_to_string(entry.path().join("status")).unwrap_or_default();
        let stat = fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
        let (state, processor, ticks) = stat_fields(&stat);
        lines.push(format!(
            "{} {tid} {state} {processor} {ticks} {} {}",
            word::LINE,
            status_field(&status, "Cpus_allowed_list"),
            status_field(&status, "Name"),
        ));
    }
    lines.extend(ok());
    lines
}

/// Every process of the host: `line <pid> <kthread> <processor>
/// <allowed> <cgroup> <name>` for each.
fn tasks() -> Vec<String> {
    let mut lines = Vec::new();
    for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
        let pid = entry.file_name().to_string_lossy().into_owned();
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let stat = fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
        let cgroup = fs::read_to_string(entry.path().join("cgroup")).unwrap_or_default();
        let (_, processor, _) = stat_fields(&stat);
        lines.push(format!(
            "{} {pid} {} {processor} {} {} {}",
            word::LINE,
            u8::from(status_field(&status, "Kthread") == "1"),
            status_field(&status, "Cpus_allowed_list"),
            cgroup.trim().rsplit("::").next().unwrap_or("?"),
            status_field(&status, "Name"),
        ));
    }
    lines.extend(ok());
    lines
}

/// What each processor is running now, by the scheduler's own account
/// in debugfs: `line <cpu> <pid>` for each.
fn curr() -> Vec<String> {
    use rustix::mount::{MountFlags, mount};
    const DEBUG: &str = "/sys/kernel/debug";
    let file = Path::new(DEBUG).join("sched/debug");
    if !file.exists() {
        let _ = mount("debugfs", DEBUG, "debugfs", MountFlags::empty(), None);
    }
    let Ok(text) = fs::read_to_string(&file) else {
        return error(format!("{}: not there", file.display()));
    };
    let mut lines = Vec::new();
    let mut cpu = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("cpu#") {
            cpu = rest
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .map(str::to_string);
        } else if let Some((key, value)) = line.split_once(':')
            && key.trim() == ".curr->pid"
            && let Some(cpu) = cpu.take()
        {
            lines.push(format!("{} {cpu} {}", word::LINE, value.trim()));
        }
    }
    lines.extend(ok());
    lines
}

/// `as <uid> kill <pid>` and `as <uid> read <path>`: whether the user
/// may signal the process, or read the file. A process of that user
/// tries it: this binary, started as the user, with `try`.
fn as_user(rest: &str) -> Vec<String> {
    use std::os::unix::process::CommandExt;
    let mut words = rest.splitn(3, ' ');
    let (uid, verb, what) = (
        words.next().unwrap_or(""),
        words.next().unwrap_or(""),
        words.next().unwrap_or(""),
    );
    let Ok(uid) = uid.parse::<u32>() else {
        return error("as <uid> kill|read <what>");
    };
    let tried = std::process::Command::new("/proc/self/exe")
        .args([crate::TRY, verb, what])
        .uid(uid)
        .gid(uid)
        .output();
    match tried {
        Ok(output) => {
            let said = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if output.status.success() {
                ok()
            } else {
                error(said)
            }
        }
        Err(e) => error(e),
    }
}

/// `qsvshook try kill <pid>` and `qsvshook try read <path>`: try it as
/// whoever this process is, say why not, and exit 0 when it could.
pub fn try_it(verb: &str, what: &str) -> i32 {
    let result: Result<(), String> = match verb {
        "kill" => what
            .parse::<i32>()
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| "no process".to_string())
            .and_then(|pid| rustix::process::test_kill_process(pid).map_err(|e| e.to_string())),
        "read" => File::open(what)
            .and_then(|mut file| file.read(&mut [0u8; 1]))
            .map(|_| ())
            .map_err(|e| e.to_string()),
        other => Err(format!("try kill|read, not {other}")),
    };
    match result {
        Ok(()) => 0,
        Err(why) => {
            println!("{why}");
            1
        }
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
        command::PARTD => partd_request(rest),
        command::THREADS => threads(rest),
        command::TASKS => tasks(),
        command::CURR => curr(),
        command::AS => as_user(rest),
        command::PARTITION => partition(rest),
        command::MOUNT => mount_ro(rest),
        command::UNMOUNT => done(rustix::mount::unmount(
            rest,
            rustix::mount::UnmountFlags::empty(),
        )),
        command::SCSI_PROBE => scsi_probe(rest),
        command::COPY => copy(rest),
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
