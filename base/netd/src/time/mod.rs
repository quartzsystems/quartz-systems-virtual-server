// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Keeping the clock in step with the configured time servers, through
//! QS/400's `ntp`: one poll over the servers in order, the offset
//! slewed when it is small and stepped when it is not.

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ntp::poll::{Polled, poll};
use ntp::{Clock, Decision, Policy};
use state::doc::host::split_server;

/// An offset past this is stepped; up to it, slewed. Nothing is too
/// far: an appliance with a wrong RTC must still come right.
const POLICY: Policy = Policy {
    min: 0,
    max: i64::MAX,
    threshold: Some(128_000_000),
};
/// How long one server may take to answer.
const TIMEOUT: Duration = Duration::from_secs(2);
/// The shortest and the longest time between polls.
const POLL_MIN: u64 = 64;
const POLL_MAX: u64 = 1024;
/// How soon to try again when no server answered.
const RETRY: u64 = 15;

/// What the interface machine tells the time keeper.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shared {
    /// The configured servers.
    pub servers: Vec<String>,
    /// Whether the interface has its address.
    pub online: bool,
}

/// What one poll did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Synced {
    /// The clock was right already.
    Kept(String),
    /// It was slewed by this many nanoseconds.
    Slewed(String, i64),
    /// It was stepped by this many nanoseconds.
    Stepped(String, i64),
    /// No server gave a usable answer.
    Failed(String),
}

/// Each server's first address, IPv4 before IPv6; a name that does not
/// resolve is left out.
pub fn resolve(servers: &[String]) -> Vec<(String, SocketAddr)> {
    servers
        .iter()
        .filter_map(|server| {
            let (host, port) = split_server(server)?;
            let mut addrs: Vec<SocketAddr> = (host, port).to_socket_addrs().ok()?.collect();
            addrs.sort_by_key(SocketAddr::is_ipv6);
            Some((server.clone(), *addrs.first()?))
        })
        .collect()
}

/// Poll `servers` once and adjust `clock`.
pub fn sync_once(servers: &[String], clock: &dyn Clock) -> Synced {
    let resolved = resolve(servers);
    if resolved.is_empty() {
        return Synced::Failed("no time server's name resolves".into());
    }
    match poll(&resolved, clock, &POLICY, TIMEOUT, None) {
        Polled::Sample {
            sample,
            decision,
            applied: Ok(()),
        } => match decision {
            Decision::Slew(n) => Synced::Slewed(sample.server, n),
            Decision::Step(n) => Synced::Stepped(sample.server, n),
            Decision::Keep | Decision::TooFar(_) => Synced::Kept(sample.server),
        },
        Polled::Sample {
            sample,
            applied: Err(e),
            ..
        } => Synced::Failed(format!(
            "{}: the clock was not adjusted: {e}",
            sample.server
        )),
        Polled::Failed(reasons) => {
            let all: Vec<String> = reasons.iter().map(|(s, e)| format!("{s}: {e}")).collect();
            Synced::Failed(all.join("; "))
        }
    }
}

/// Sleep `secs`, a second at a time, ending early when `shared` changes.
fn wait(shared: &Mutex<Shared>, seen: &Shared, secs: u64) {
    for _ in 0..secs {
        std::thread::sleep(Duration::from_secs(1));
        if *shared.lock().unwrap_or_else(PoisonError::into_inner) != *seen {
            return;
        }
    }
}

/// Keep `clock` in step for ever. `synced` is told of the first
/// synchronization and of every step, so it can post and write the RTC.
pub fn keep(shared: &Arc<Mutex<Shared>>, clock: &dyn Clock, mut synced: impl FnMut(&str)) -> ! {
    let mut interval = POLL_MIN;
    let mut announced = false;
    loop {
        let now = shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if now.servers.is_empty() || !now.online {
            announced = false;
            wait(shared, &now, 5);
            continue;
        }
        let pause = match sync_once(&now.servers, clock) {
            Synced::Stepped(server, _) => {
                synced(&server);
                announced = true;
                interval = POLL_MIN;
                interval
            }
            Synced::Slewed(server, _) | Synced::Kept(server) => {
                if !announced {
                    synced(&server);
                    announced = true;
                }
                interval = (interval * 2).min(POLL_MAX);
                interval / 2
            }
            Synced::Failed(why) => {
                eprintln!("time: {why}");
                interval = POLL_MIN;
                RETRY
            }
        };
        wait(shared, &now, pause);
    }
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;

    use ntp::clock::MemClock;
    use ntp::server;

    use super::*;

    const S: i64 = 1_000_000_000;

    fn time_server(at: i64) -> server::Running {
        let clock: Arc<dyn Clock> = Arc::new(MemClock::at(at));
        server::serve(
            UdpSocket::bind("127.0.0.1:0").unwrap(),
            clock,
            Arc::new(Mutex::new(None)),
            false,
            None,
        )
        .unwrap()
    }

    #[test]
    fn a_far_clock_is_stepped_and_a_near_one_slewed() {
        let base = 1_900_000_000 * S;
        // A day off, as a host with a dead RTC battery is.
        let far = time_server(base + 86_400 * S);
        let servers = vec![format!("127.0.0.1:{}", far.addr().port())];
        let mine = MemClock::at(base);
        assert!(
            matches!(sync_once(&servers, &mine), Synced::Stepped(_, n) if (n - 86_400 * S).abs() < S)
        );
        assert_eq!(mine.steps().len(), 1);

        let near = time_server(base + 50_000_000);
        let servers = vec![format!("127.0.0.1:{}", near.addr().port())];
        let mine = MemClock::at(base);
        assert!(matches!(sync_once(&servers, &mine), Synced::Slewed(..)));
        assert!(mine.steps().is_empty());
    }

    #[test]
    fn servers_that_do_not_resolve_or_answer_are_a_failure_with_its_reason() {
        let mine = MemClock::at(0);
        let Synced::Failed(why) = sync_once(&["no such host.invalid".into()], &mine) else {
            panic!("synced");
        };
        assert!(why.contains("resolves"), "{why}");
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let servers = vec![format!("127.0.0.1:{}", quiet.local_addr().unwrap().port())];
        assert!(matches!(sync_once(&servers, &mine), Synced::Failed(_)));
    }

    #[test]
    fn the_first_working_server_is_used() {
        let base = 1_900_000_000 * S;
        let quiet = UdpSocket::bind("127.0.0.1:0").unwrap();
        let good = time_server(base + 10 * S);
        let servers = vec![
            format!("127.0.0.1:{}", quiet.local_addr().unwrap().port()),
            format!("127.0.0.1:{}", good.addr().port()),
        ];
        let mine = MemClock::at(base);
        let Synced::Stepped(server, _) = sync_once(&servers, &mine) else {
            panic!("not stepped");
        };
        assert_eq!(server, servers[1]);
    }
}
