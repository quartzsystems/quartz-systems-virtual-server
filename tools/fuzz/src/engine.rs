// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The loop. An input is run with the counters cleared; a panic is
//! caught and is a finding, as is a run that takes too long; an input
//! that made a counter reach a size it had not reached is kept and
//! mutated in its turn.

use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::cov;
use crate::mutate::{Rng, mutate};

/// Inputs kept at most.
const CORPUS_MAX: usize = 4096;
/// A run longer than this is a finding.
pub const SLOW: Duration = Duration::from_secs(5);

/// One thing to fuzz.
pub struct Target {
    /// Its name.
    pub name: &'static str,
    /// What it reads.
    pub about: &'static str,
    /// Inputs to start from: valid ones, so the mutations go deep.
    pub seeds: fn() -> Vec<Vec<u8>>,
    /// Run one input. It may refuse the input any way it likes; it
    /// may not panic.
    pub run: fn(&[u8]),
}

/// Something found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// The input.
    pub input: Vec<u8>,
    /// What happened.
    pub what: String,
}

/// How a run of the loop went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Inputs run.
    pub runs: u64,
    /// Inputs kept.
    pub kept: usize,
    /// Counters that were ever not zero.
    pub edges: usize,
    /// The first finding, which ends the loop.
    pub finding: Option<Finding>,
}

/// What the last panic said, for the finding.
static PANIC: Mutex<String> = Mutex::new(String::new());
/// When the run under way started, in milliseconds since the loop
/// did; 0 between runs.
static STARTED: AtomicU64 = AtomicU64::new(0);
/// The input under way, for the watchdog.
static CURRENT: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Have a panic recorded, not printed.
pub fn quiet_panics() {
    panic::set_hook(Box::new(|info| {
        if let Ok(mut said) = PANIC.lock() {
            *said = info.to_string();
        }
    }));
}

/// The size class of a counter: what counts as "more" of an edge.
fn class(count: u8) -> u8 {
    match count {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        4..=7 => 8,
        8..=15 => 16,
        16..=31 => 32,
        32..=127 => 64,
        128..=255 => 128,
    }
}

/// Whether the counters show a class not yet in `seen`; `seen` takes it.
fn novel(seen: &mut Vec<u8>, counters: &[u8]) -> bool {
    if seen.len() < counters.len() {
        seen.resize(counters.len(), 0);
    }
    let mut new = false;
    for (known, count) in seen.iter_mut().zip(counters) {
        let class = class(*count);
        if class & !*known != 0 {
            *known |= class;
            new = true;
        }
    }
    new
}

/// The input a watchdog found still running after `limit`, and for
/// how long; `None` while all is well.
pub fn stuck(since: Instant, limit: Duration) -> Option<(Vec<u8>, Duration)> {
    let started = STARTED.load(Ordering::Relaxed);
    if started == 0 {
        return None;
    }
    let now = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
    let ran = Duration::from_millis(now.saturating_sub(started));
    (ran > limit).then(|| (CURRENT.lock().map(|c| c.clone()).unwrap_or_default(), ran))
}

fn execute(target: &Target, input: &[u8], since: Instant) -> Option<String> {
    if let Ok(mut current) = CURRENT.lock() {
        current.clear();
        current.extend_from_slice(input);
    }
    cov::reset();
    let begun = Instant::now();
    let millis = u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX);
    STARTED.store(millis.max(1), Ordering::Relaxed);
    let outcome = panic::catch_unwind(AssertUnwindSafe(|| (target.run)(input)));
    STARTED.store(0, Ordering::Relaxed);
    match outcome {
        Err(_) => Some(PANIC.lock().map_or_else(|_| "panic".into(), |s| s.clone())),
        Ok(()) if begun.elapsed() > SLOW => Some(format!("ran for {:?}", begun.elapsed())),
        Ok(()) => None,
    }
}

/// Fuzz `target` for `limit` from `seed`. `since` is the instant the
/// watchdog counts from.
pub fn fuzz(target: &Target, seed: u64, limit: Duration, since: Instant) -> Report {
    let mut rng = Rng::new(seed);
    let mut reached = Vec::new();
    let mut corpus: Vec<Vec<u8>> = Vec::new();
    let mut report = Report {
        runs: 0,
        kept: 0,
        edges: 0,
        finding: None,
    };
    let begun = Instant::now();
    let mut pending = (target.seeds)();
    pending.push(Vec::new());
    loop {
        let input = match pending.pop() {
            Some(first) => first,
            None if begun.elapsed() >= limit => break,
            None => {
                let from = &corpus[rng.below(corpus.len())];
                let other = &corpus[rng.below(corpus.len())];
                mutate(from, other, &mut rng)
            }
        };
        report.runs += 1;
        if let Some(what) = execute(target, &input, since) {
            report.finding = Some(Finding { input, what });
            break;
        }
        let new = cov::with(|counters| novel(&mut reached, counters));
        // Every seed is kept; without counters, a sample of the rest.
        let keep = new || corpus.len() < 8 || (cov::is_empty() && rng.below(64) == 0);
        if keep && corpus.len() < CORPUS_MAX {
            corpus.push(input);
        } else if keep {
            let at = rng.below(corpus.len());
            corpus[at] = input;
        }
    }
    report.kept = corpus.len();
    report.edges = reached.iter().filter(|s| **s != 0).count();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> Vec<Vec<u8>> {
        vec![b"quartz".to_vec()]
    }

    #[test]
    fn a_target_that_never_panics_has_no_finding() {
        let target = Target {
            name: "length",
            about: "",
            seeds,
            run: |input| {
                let _ = input.len();
            },
        };
        let report = fuzz(&target, 1, Duration::from_millis(50), Instant::now());
        assert!(report.finding.is_none());
        assert!(report.runs > 100);
        assert!(report.kept >= 2);
    }

    #[test]
    fn a_panic_is_caught_and_its_input_kept() {
        quiet_panics();
        let target = Target {
            name: "panics",
            about: "",
            seeds,
            run: |input| assert!(!input.contains(&0xFF), "found the byte"),
        };
        let report = fuzz(&target, 2, Duration::from_secs(20), Instant::now());
        let finding = report.finding.expect("the byte is found within the time");
        assert!(finding.input.contains(&0xFF));
        assert!(finding.what.contains("found the byte"));
    }

    #[test]
    fn a_counter_that_grows_is_new_coverage_once() {
        let mut reached = Vec::new();
        assert!(novel(&mut reached, &[0, 1, 0]));
        assert!(!novel(&mut reached, &[0, 1, 0]));
        assert!(novel(&mut reached, &[0, 5, 0]));
        assert!(!novel(&mut reached, &[0, 4, 0]));
        assert!(novel(&mut reached, &[1, 0, 0, 200]));
        assert_eq!(class(255), 128);
    }
}
