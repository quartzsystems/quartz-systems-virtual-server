// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! When a failed service is started again: after 1, 2, 4 ... seconds,
//! 30 at most; never again after five failures in ten minutes.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The first delay.
const FIRST: Duration = Duration::from_secs(1);
/// The longest delay.
const LONGEST: Duration = Duration::from_secs(30);
/// Failures within [`WINDOW`] that end the restarts.
const LIMIT: usize = 5;
/// The window the failures are counted in. A service that stays up
/// this long starts again from the first delay.
const WINDOW: Duration = Duration::from_secs(600);

/// What to do about a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Start it again after this long.
    Restart(Duration),
    /// Leave it stopped.
    Stop,
}

/// One service's failures.
#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// When it failed, within the window.
    recent: VecDeque<Instant>,
    /// Failures in a row.
    streak: u32,
    /// Failures in all.
    total: u32,
}

impl Policy {
    /// The service failed at `now`, having been started at `started`.
    pub fn failed(&mut self, started: Instant, now: Instant) -> Verdict {
        if now.saturating_duration_since(started) >= WINDOW {
            self.streak = 0;
        }
        self.streak += 1;
        self.total += 1;
        self.recent.push_back(now);
        while let Some(first) = self.recent.front()
            && now.saturating_duration_since(*first) >= WINDOW
        {
            self.recent.pop_front();
        }
        if self.recent.len() >= LIMIT {
            return Verdict::Stop;
        }
        let doubled = FIRST.saturating_mul(1u32.checked_shl(self.streak - 1).unwrap_or(u32::MAX));
        Verdict::Restart(doubled.min(LONGEST))
    }

    /// Failures in all.
    pub fn total(&self) -> u32 {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn five_quick_failures_stop_the_restarts() {
        let mut p = Policy::default();
        let mut now = Instant::now();
        for delay in [1, 2, 4, 8] {
            assert_eq!(p.failed(now, now), Verdict::Restart(secs(delay)));
            now += secs(delay);
        }
        assert_eq!(p.failed(now, now), Verdict::Stop);
        assert_eq!(p.total(), 5);
    }

    #[test]
    fn slow_failures_back_off_to_thirty_seconds_and_go_on() {
        let mut p = Policy::default();
        let mut now = Instant::now();
        let mut delays = Vec::new();
        // One failure every three minutes: never five in ten.
        for _ in 0..8 {
            let Verdict::Restart(d) = p.failed(now, now + secs(170)) else {
                panic!("stopped");
            };
            delays.push(d.as_secs());
            now += secs(180);
        }
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn a_long_run_starts_the_delays_again() {
        let mut p = Policy::default();
        let t = Instant::now();
        assert_eq!(p.failed(t, t), Verdict::Restart(secs(1)));
        assert_eq!(p.failed(t, t + secs(1)), Verdict::Restart(secs(2)));
        // Up for ten minutes, then down.
        let up = t + secs(10);
        assert_eq!(p.failed(up, up + WINDOW), Verdict::Restart(secs(1)));
    }
}
