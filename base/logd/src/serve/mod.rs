// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The log service: records from its datagram socket and the kernel's
//! from `/dev/kmsg`, written under the log directory.

use std::convert::Infallible;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::Path;
use std::time::Duration;

use appliance::Appliance;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::OFlags;
use supervise::signal::{Signals, TERM};

use crate::files::kernel::KernelLog;
use crate::files::{Limits, Logs, boot_id};
use crate::record::Record;

/// The kernel's log device.
const KMSG: &str = "/dev/kmsg";
/// How long written records may wait for the disk.
const SYNC_EVERY: Duration = Duration::from_secs(5);
/// The largest datagram taken.
const DATAGRAM_MAX: usize = 64 * 1024;

/// The service's state.
pub struct Server {
    socket: UnixDatagram,
    kmsg: Option<File>,
    logs: Logs,
    kernel: KernelLog,
    buf: Vec<u8>,
    /// `SIGTERM`, when the service takes it to end cleanly.
    term: Option<Signals>,
}

impl Server {
    /// Bind `socket`, open the logs in `dir` and, with `kmsg`, the
    /// kernel's log device.
    pub fn open(
        socket: &Path,
        dir: &Path,
        boot: &str,
        limits: Limits,
        kmsg: Option<&Path>,
    ) -> io::Result<Self> {
        // A socket left by a service that ended.
        let _ = fs::remove_file(socket);
        let socket = UnixDatagram::bind(socket)?;
        socket.set_nonblocking(true)?;
        let kmsg = kmsg.and_then(|path| {
            let flags = i32::try_from(OFlags::NONBLOCK.bits()).ok()?;
            OpenOptions::new()
                .read(true)
                .custom_flags(flags)
                .open(path)
                .ok()
        });
        Ok(Self {
            socket,
            kmsg,
            logs: Logs::open(dir, boot, limits),
            kernel: KernelLog::open(dir, boot, limits.kernel_bytes, limits.kernel_boots),
            buf: vec![0; DATAGRAM_MAX],
            term: None,
        })
    }

    fn records(&mut self) {
        while let Ok(n) = self.socket.recv(&mut self.buf) {
            if let Ok(record) = ipc::decode::<Record>(&self.buf[..n]) {
                self.logs.write(&record);
            }
        }
        // One sync for all that came together: a sync a record would
        // leave the service behind its senders at an IPL.
        self.logs.settle();
    }

    fn kernel(&mut self) {
        let Some(kmsg) = &mut self.kmsg else { return };
        loop {
            // One record a read. EPIPE says records were overwritten
            // before they were read; the next read goes on.
            match kmsg.read(&mut self.buf) {
                Ok(0) => break,
                Ok(n) => self.kernel.record(&String::from_utf8_lossy(&self.buf[..n])),
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
                Err(_) => break,
            }
        }
    }

    /// Wait up to `wait` for something to write, and write it. Nothing
    /// arriving brings what was written to the disk. True when the
    /// service was asked to end: everything sent before is written.
    pub fn turn(&mut self, wait: Duration) -> bool {
        let timeout = Timespec {
            tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
            tv_nsec: i64::from(wait.subsec_nanos()),
        };
        let (records, kernel, term) = {
            let mut fds = vec![PollFd::new(&self.socket, PollFlags::IN)];
            if let Some(kmsg) = &self.kmsg {
                fds.push(PollFd::from_borrowed_fd(kmsg.as_fd(), PollFlags::IN));
            }
            if let Some(term) = &self.term {
                fds.push(PollFd::from_borrowed_fd(term.fd(), PollFlags::IN));
            }
            if poll(&mut fds, Some(&timeout)).is_err() {
                return false;
            }
            let kmsg = usize::from(self.kmsg.is_some());
            let ready = |i: usize| fds.get(i).is_some_and(|f| !f.revents().is_empty());
            (
                ready(0),
                kmsg == 1 && ready(1),
                self.term.is_some() && ready(1 + kmsg),
            )
        };
        let ending = term && self.term.as_mut().is_some_and(|t| !t.read().is_empty());
        if records || ending {
            self.records();
        }
        if kernel || ending {
            self.kernel();
        }
        if ending || (!records && !kernel) {
            self.logs.sync();
            self.kernel.sync();
        }
        ending
    }
}

/// Run the log service of `appliance`. Says it is ready once its
/// socket is bound, ends when it is told to (`SIGTERM`) with
/// everything written, and returns only when it cannot start.
pub fn run(appliance: &Appliance, limits: Limits) -> io::Result<Infallible> {
    let mut server = Server::open(
        &appliance.log_socket(),
        &appliance.log_dir(),
        &boot_id(),
        limits,
        Some(Path::new(KMSG)),
    )?;
    server.term = Some(Signals::take(&[TERM])?);
    supervise::ready::signal()?;
    loop {
        if server.turn(SYNC_EVERY) {
            std::process::exit(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Sender;
    use crate::files::{HISTORY, SYSTEM, rotated};
    use crate::record::Severity;

    #[test]
    fn what_was_said_before_the_service_started_is_written_first() {
        let dir = std::env::temp_dir().join(format!("logd-serve-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("logd.sock");
        let mut sender = Sender::new(&socket);
        sender.send(Record::code("init", Severity::Info, "C0000001", "started"));
        sender.send(Record::new("init", Severity::Info, "early"));
        assert_eq!(sender.waiting(), 2);

        // A stand-in for /dev/kmsg: one record, then the end.
        let kmsg = dir.join("kmsg");
        fs::write(&kmsg, "6,1,2000000,-;kernel says\n").unwrap();
        let logs = dir.join("log");
        let mut server =
            Server::open(&socket, &logs, "boot-9", Limits::default(), Some(&kmsg)).unwrap();
        assert!(sender.flush());
        sender.send(Record::new("netd", Severity::Error, "late"));
        server.turn(Duration::from_millis(200));
        server.turn(Duration::from_millis(50));

        let system = fs::read_to_string(rotated(&logs, SYSTEM, 0)).unwrap();
        let said: Vec<String> = system
            .lines()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                format!(
                    "{} {}",
                    v["source"].as_str().unwrap(),
                    v["message"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(said, ["init started", "init early", "netd late"]);
        assert_eq!(
            fs::read_to_string(logs.join(HISTORY))
                .unwrap()
                .lines()
                .count(),
            1
        );
        assert_eq!(
            fs::read_to_string(logs.join("kernel/boot-9.log")).unwrap(),
            "[    2.000000] <6> kernel says\n"
        );
        // Junk on the socket is ignored.
        UnixDatagram::unbound()
            .unwrap()
            .send_to(b"junk", &socket)
            .unwrap();
        server.turn(Duration::from_millis(50));
        assert_eq!(
            fs::read_to_string(rotated(&logs, SYSTEM, 0))
                .unwrap()
                .lines()
                .count(),
            3
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
