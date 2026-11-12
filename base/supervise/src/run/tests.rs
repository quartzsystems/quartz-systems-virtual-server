// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The supervisor over real processes (`/bin/sh`), with a clock the
//! test moves: a back-off is a jump, not a wait.

use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::process::{Pid, WaitOptions, waitpid};

use super::{Occurrence, Supervisor};
use crate::table::Service;

const fn sh(
    name: &'static str,
    order: u32,
    needs: &'static [&'static str],
    script: &'static [&'static str],
) -> Service {
    Service {
        name,
        binary: "/bin/sh",
        args: script,
        order,
        needs,
        ready_within: Duration::from_secs(5),
    }
}

/// Drives a supervisor the way PID 1 does.
struct Driver {
    sup: Supervisor,
    now: Instant,
    seen: Vec<Occurrence>,
}

impl Driver {
    fn new(table: &'static [Service]) -> Self {
        let now = Instant::now();
        let mut sup = Supervisor::new(table, now).unwrap();
        let seen = sup.advance(now);
        Self { sup, now, seen }
    }

    /// One turn: read what is readable, reap what ended.
    fn turn(&mut self) {
        let ready: Vec<_> = {
            let fds = self.sup.fds();
            let mut polls: Vec<PollFd<'_>> = fds
                .iter()
                .map(|(_, fd)| PollFd::new(fd, PollFlags::IN))
                .collect();
            let wait = Timespec {
                tv_sec: 0,
                tv_nsec: 20_000_000,
            };
            let _ = poll(&mut polls, Some(&wait));
            fds.iter()
                .zip(&polls)
                .filter(|(_, p)| !p.revents().is_empty())
                .map(|((token, _), _)| *token)
                .collect()
        };
        for token in ready {
            let more = self.sup.readable(token, self.now);
            self.seen.extend(more);
        }
        for status in self.sup.status() {
            let Some(pid) = i32::try_from(status.pid).ok().and_then(Pid::from_raw) else {
                continue;
            };
            if let Ok(Some((_, how))) = waitpid(Some(pid), WaitOptions::NOHANG) {
                let (ours, more) = self.sup.exited(status.pid, &format!("{how:?}"), self.now);
                assert!(ours);
                self.seen.extend(more);
            }
        }
    }

    /// Turn until `done`, moving the clock to each deadline once
    /// nothing else happens.
    fn until(&mut self, what: &str, done: impl Fn(&[Occurrence]) -> bool) {
        let give_up = Instant::now() + Duration::from_secs(20);
        let mut quiet = 0;
        while !done(&self.seen) {
            assert!(Instant::now() < give_up, "{what}: {:#?}", self.seen);
            let before = self.seen.len();
            self.turn();
            quiet = if self.seen.len() == before {
                quiet + 1
            } else {
                0
            };
            if quiet >= 25
                && let Some(at) = self.sup.deadline()
            {
                self.now = self.now.max(at);
                let more = self.sup.advance(self.now);
                self.seen.extend(more);
                quiet = 0;
            }
        }
    }

    fn state(&self, name: &str) -> String {
        let status = self.sup.status();
        status
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .state
            .clone()
    }
}

fn has(seen: &[Occurrence], wanted: &Occurrence) -> bool {
    seen.contains(wanted)
}

#[test]
fn services_start_in_order_and_stop_in_reverse() {
    static TABLE: [Service; 2] = [
        sh(
            "second",
            20,
            &["first"],
            &[
                "-c",
                "echo hello; echo oops >&2; printf R >&0; exec sleep 30",
            ],
        ),
        sh("first", 10, &[], &["-c", "printf R >&0; exec sleep 30"]),
    ];
    let mut d = Driver::new(&TABLE);
    // Only the first is started until it is ready.
    assert!(matches!(
        d.seen.as_slice(),
        [Occurrence::Started {
            service: "first",
            ..
        }]
    ));
    assert_eq!(d.state("second"), "waiting");
    d.until("all ready", |s| has(s, &Occurrence::AllReady));
    let at = |wanted: &dyn Fn(&Occurrence) -> bool| d.seen.iter().position(wanted).unwrap();
    let first_ready = at(&|o| *o == Occurrence::Ready { service: "first" });
    let second_started = at(&|o| {
        matches!(
            o,
            Occurrence::Started {
                service: "second",
                ..
            }
        )
    });
    assert!(first_ready < second_started);
    d.until("output", |s| {
        s.iter()
            .filter(|o| matches!(o, Occurrence::Output { .. }))
            .count()
            == 2
    });
    assert!(has(
        &d.seen,
        &Occurrence::Output {
            service: "second",
            error: false,
            line: "hello".into()
        }
    ));
    assert!(has(
        &d.seen,
        &Occurrence::Output {
            service: "second",
            error: true,
            line: "oops".into()
        }
    ));
    assert!(d.sup.all_ready());

    // The stop: the second ends before the first is asked to.
    d.seen.clear();
    let more = d.sup.begin_stop(Duration::from_secs(5), d.now);
    d.seen.extend(more);
    assert_eq!(d.state("second"), "stopping");
    assert_eq!(d.state("first"), "ready");
    d.until("ended", |s| has(s, &Occurrence::Ended));
    assert!(d.sup.ended());
    assert_eq!(d.state("first"), "ended");
    assert_eq!(d.sup.status()[0].pid, 0);
}

#[test]
fn a_service_that_keeps_failing_is_stopped_after_five_tries() {
    static TABLE: [Service; 2] = [
        sh("steady", 10, &[], &["-c", "printf R >&0; exec sleep 30"]),
        sh("flaky", 20, &[], &["-c", "exit 3"]),
    ];
    let mut d = Driver::new(&TABLE);
    d.until("gave up", |s| {
        has(s, &Occurrence::GaveUp { service: "flaky" })
    });
    let delays: Vec<Option<u64>> = d
        .seen
        .iter()
        .filter_map(|o| match o {
            Occurrence::Failed {
                service: "flaky",
                restart_in,
                ..
            } => Some(restart_in.map(|d| d.as_secs())),
            _ => None,
        })
        .collect();
    assert_eq!(delays, [Some(1), Some(2), Some(4), Some(8), None]);
    assert_eq!(d.state("flaky"), "stopped");
    let flaky = d
        .sup
        .status()
        .into_iter()
        .find(|s| s.name == "flaky")
        .unwrap();
    assert_eq!((flaky.failures, flaky.pid), (5, 0));
    // The other service is untouched, and the table is not all ready.
    d.until("steady", |s| {
        has(s, &Occurrence::Ready { service: "steady" })
    });
    assert!(!d.sup.all_ready());
    assert!(!has(&d.seen, &Occurrence::AllReady));
    let more = d.sup.begin_stop(Duration::from_secs(5), d.now);
    d.seen.extend(more);
    d.until("ended", |s| has(s, &Occurrence::Ended));
}

#[test]
fn a_killed_service_is_started_again() {
    static TABLE: [Service; 1] = [sh("only", 10, &[], &["-c", "printf R >&0; exec sleep 30"])];
    let mut d = Driver::new(&TABLE);
    d.until("ready", |s| has(s, &Occurrence::AllReady));
    let first = d.sup.status()[0].pid;
    super::signal(first, rustix::process::Signal::KILL);
    d.until("restarted", |s| {
        s.iter()
            .filter(|o| matches!(o, Occurrence::Ready { .. }))
            .count()
            == 2
    });
    let again = d.sup.status()[0].clone();
    assert_ne!(again.pid, first);
    assert_eq!((again.state.as_str(), again.failures), ("ready", 1));
    let more = d.sup.begin_stop(Duration::from_secs(5), d.now);
    d.seen.extend(more);
    d.until("ended", |s| has(s, &Occurrence::Ended));
}

#[test]
fn a_service_that_is_never_ready_is_killed_and_counted() {
    static TABLE: [Service; 1] = [sh("mute", 10, &[], &["-c", "exec sleep 30"])];
    let mut d = Driver::new(&TABLE);
    d.until("failed", |s| {
        s.iter().any(|o| matches!(o, Occurrence::Failed { .. }))
    });
    let Some(Occurrence::Failed { how, .. }) = d
        .seen
        .iter()
        .find(|o| matches!(o, Occurrence::Failed { .. }))
    else {
        unreachable!();
    };
    assert!(how.starts_with("was not ready in time"), "{how}");
    let more = d.sup.begin_stop(Duration::from_millis(100), d.now);
    d.seen.extend(more);
    d.until("ended", |s| has(s, &Occurrence::Ended));
}

#[test]
fn a_missing_binary_is_a_failure_not_a_panic() {
    static TABLE: [Service; 1] = [Service {
        name: "ghost",
        binary: "/nonexistent/ghost",
        args: &[],
        order: 1,
        needs: &[],
        ready_within: Duration::from_secs(1),
    }];
    let mut d = Driver::new(&TABLE);
    d.until("gave up", |s| {
        has(s, &Occurrence::GaveUp { service: "ghost" })
    });
    assert!(matches!(
        &d.seen[0],
        Occurrence::Failed { how, .. } if how.starts_with("could not be started")
    ));
}
