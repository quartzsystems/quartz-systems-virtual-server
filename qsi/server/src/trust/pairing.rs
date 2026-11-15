// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The pins and the pairing password, shared by every connection.
//!
//! The password is QS/400's (`qsitrust::pair::one_time_password`): made
//! by the host, never chosen, shown once at the local console. It is
//! kept in this process's memory alone, never in a file or the log;
//! it is good for one pairing, [`LIFETIME_SECS`] and [`ATTEMPTS`]
//! attempts, then gone.

use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use qsitrust::Fingerprint;
use qsitrust::pair::{Guard, Paired};

use super::consoles::{Console, Consoles};

/// How long a pairing password is good for.
pub const LIFETIME_SECS: u64 = 15 * 60;
/// How many attempts it is good for.
pub const ATTEMPTS: u32 = 5;

/// The password out, as the local console shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Password {
    /// The password: shown once, then never again.
    pub text: String,
    /// When it is gone, in seconds since the epoch.
    pub until: u64,
    /// Attempts left.
    pub attempts: u32,
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the password itself.
        f.debug_struct("Password")
            .field("until", &self.until)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

/// What happened that the host says: a reference code and an event
/// each (`docs/QSISERVER.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Happened {
    /// A console paired: its name and fingerprint.
    Paired(Console),
    /// An attempt failed its confirmation; the attempts left.
    Failed(u32),
    /// The password expired or ran out of attempts unused.
    Expired,
    /// A console was removed at the host.
    Removed(Console),
}

/// Why something asked of the trust was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Two consoles are paired already.
    Full,
    /// No such console.
    NoConsole,
    /// The consoles could not be kept: the state store's error.
    NotKept(String),
}

struct Inner {
    consoles: Consoles,
    password: Option<Password>,
    happened: Vec<Happened>,
}

/// The pins and the password: one for the whole server, shared by
/// every connection.
#[derive(Clone)]
pub struct Trust {
    inner: Arc<Mutex<Inner>>,
    /// Where `qsi-consoles.toml` is; none keeps nothing (tests).
    state_dir: Option<PathBuf>,
    now: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl fmt::Debug for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Trust").finish_non_exhaustive()
    }
}

impl Trust {
    /// The trust kept in `state_dir`, read now; a file refused is the
    /// store's error.
    ///
    /// # Errors
    /// The state file is refused.
    pub fn open(
        state_dir: PathBuf,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Result<Self, String> {
        let consoles = state::Store::at(&state_dir)
            .load::<Consoles>()
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        Ok(Self::with(consoles, Some(state_dir), now))
    }

    /// Trust over `consoles`, kept in `state_dir` when there is one.
    #[must_use]
    pub fn with(
        consoles: Consoles,
        state_dir: Option<PathBuf>,
        now: Arc<dyn Fn() -> u64 + Send + Sync>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                consoles,
                password: None,
                happened: Vec::new(),
            })),
            state_dir,
            now,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn keep(&self, consoles: &Consoles) -> Result<(), Refusal> {
        match &self.state_dir {
            Some(dir) => state::Store::at(dir)
                .save(consoles)
                .map_err(|e| Refusal::NotKept(e.to_string())),
            None => Ok(()),
        }
    }

    /// The consoles paired now.
    #[must_use]
    pub fn consoles(&self) -> Consoles {
        self.lock().consoles.clone()
    }

    /// The password out, if one is; an expired one is gone first.
    #[must_use]
    pub fn password(&self) -> Option<Password> {
        self.expire();
        self.lock().password.clone()
    }

    /// Make a pairing password, in place of any out. Refused when two
    /// consoles are paired.
    ///
    /// # Errors
    /// [`Refusal::Full`]; [`Refusal::NotKept`] when no random password
    /// could be made.
    pub fn start_pairing(&self) -> Result<Password, Refusal> {
        let text =
            qsitrust::pair::one_time_password().map_err(|e| Refusal::NotKept(e.to_string()))?;
        let now = (self.now)();
        let mut inner = self.lock();
        if !inner.consoles.room() {
            return Err(Refusal::Full);
        }
        let p = Password {
            text,
            until: now + LIFETIME_SECS,
            attempts: ATTEMPTS,
        };
        inner.password = Some(p.clone());
        Ok(p)
    }

    /// End pairing early: the password is gone.
    pub fn end_pairing(&self) {
        self.lock().password = None;
    }

    /// Forget a password that has expired or has no attempts left,
    /// saying so once.
    pub fn expire(&self) {
        let now = (self.now)();
        let mut inner = self.lock();
        if inner
            .password
            .as_ref()
            .is_some_and(|p| now >= p.until || p.attempts == 0)
        {
            inner.password = None;
            inner.happened.push(Happened::Expired);
        }
    }

    /// Remove a console: it can connect no more. Its session, if any, is
    /// the caller's to close.
    ///
    /// # Errors
    /// [`Refusal::NoConsole`]; [`Refusal::NotKept`].
    pub fn remove(&self, fingerprint: &Fingerprint) -> Result<Console, Refusal> {
        let mut inner = self.lock();
        let hex = fingerprint.hex();
        let at = inner
            .consoles
            .consoles
            .iter()
            .position(|c| c.fingerprint == hex)
            .ok_or(Refusal::NoConsole)?;
        let mut kept = inner.consoles.clone();
        let gone = kept.consoles.remove(at);
        self.keep(&kept)?;
        inner.consoles = kept;
        inner.happened.push(Happened::Removed(gone.clone()));
        Ok(gone)
    }

    /// A pinned console had a session from `address`.
    pub fn contact(&self, fingerprint: &Fingerprint, address: &str) {
        let now = (self.now)();
        let mut inner = self.lock();
        let hex = fingerprint.hex();
        let mut kept = inner.consoles.clone();
        let Some(c) = kept.consoles.iter_mut().find(|c| c.fingerprint == hex) else {
            return;
        };
        c.last_contact = now;
        c.address = address.chars().take(64).collect();
        // Not keeping when the time is kept is no reason to refuse it.
        if self.keep(&kept).is_ok() {
            inner.consoles = kept;
        }
    }

    /// What happened since the last call.
    #[must_use]
    pub fn happened(&self) -> Vec<Happened> {
        std::mem::take(&mut self.lock().happened)
    }

    /// A pairing guard for one connection from `address`.
    #[must_use]
    pub fn guard(&self, address: &str) -> PairingGuard {
        PairingGuard {
            trust: self.clone(),
            address: address.to_string(),
        }
    }
}

impl qsitrust::Pins for Trust {
    fn pinned(&self, peer: &Fingerprint) -> bool {
        self.lock().consoles.by_print(peer).is_some()
    }

    fn pairing_open(&self) -> bool {
        self.password().is_some() && self.lock().consoles.room()
    }
}

/// The trust crate's guard for one connection's pairing exchange.
#[derive(Debug)]
pub struct PairingGuard {
    trust: Trust,
    address: String,
}

impl Guard for PairingGuard {
    fn password(&mut self) -> Option<String> {
        self.trust.expire();
        let mut inner = self.trust.lock();
        if !inner.consoles.room() {
            return None;
        }
        let p = inner.password.as_mut()?;
        // The attempt is counted before anything is answered.
        p.attempts = p.attempts.saturating_sub(1);
        Some(p.text.clone())
    }

    fn failed(&mut self) {
        let mut inner = self.trust.lock();
        let left = inner.password.as_ref().map_or(0, |p| p.attempts);
        inner.happened.push(Happened::Failed(left));
        if left == 0 {
            inner.password = None;
            inner.happened.push(Happened::Expired);
        }
    }

    fn paired(&mut self, console: &Paired) {
        let now = (self.trust.now)();
        let mut inner = self.trust.lock();
        // Good for one pairing.
        inner.password = None;
        let entry = Console {
            name: Consoles::kept_name(&console.name),
            fingerprint: console.fingerprint.hex(),
            paired: now,
            last_contact: now,
            address: self.address.chars().take(64).collect(),
        };
        let mut kept = inner.consoles.clone();
        if !kept.room() || kept.by_print(&console.fingerprint).is_some() {
            return;
        }
        kept.consoles.push(entry.clone());
        if self.trust.keep(&kept).is_ok() {
            inner.consoles = kept;
            inner.happened.push(Happened::Paired(entry));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use qsitrust::Pins as _;

    use super::*;

    fn clock() -> (Arc<AtomicU64>, Arc<dyn Fn() -> u64 + Send + Sync>) {
        let t = Arc::new(AtomicU64::new(1_794_000_000));
        let read = t.clone();
        (t, Arc::new(move || read.load(Ordering::Relaxed)))
    }

    fn print(n: u8) -> Fingerprint {
        Fingerprint([n; 32])
    }

    fn paired(n: u8, name: &str) -> Paired {
        Paired {
            fingerprint: print(n),
            name: name.into(),
        }
    }

    #[test]
    fn a_password_pairs_once_and_opens_pairing_only_while_it_is_out() {
        let (_, now) = clock();
        let trust = Trust::with(Consoles::default(), None, now);
        assert!(!trust.pairing_open());
        let p = trust.start_pairing().unwrap();
        assert_eq!(p.attempts, ATTEMPTS);
        assert!(trust.pairing_open());
        let mut guard = trust.guard("192.0.2.7:50000");
        assert_eq!(guard.password(), Some(p.text.clone()));
        guard.paired(&paired(1, "MC1"));
        assert!(trust.pinned(&print(1)));
        assert!(!trust.pairing_open());
        // Good for one pairing: the next attempt has none.
        assert_eq!(trust.guard("x").password(), None);
        let c = trust.consoles();
        assert_eq!(c.consoles[0].name, "MC1");
        assert_eq!(c.consoles[0].address, "192.0.2.7:50000");
        assert!(matches!(&trust.happened()[..], [Happened::Paired(c)] if c.name == "MC1"));
    }

    #[test]
    fn five_failures_or_fifteen_minutes_end_the_password() {
        let (t, now) = clock();
        let trust = Trust::with(Consoles::default(), None, now);
        trust.start_pairing().unwrap();
        let mut guard = trust.guard("x");
        for left in (0..ATTEMPTS).rev() {
            assert!(guard.password().is_some());
            guard.failed();
            assert_eq!(trust.password().map_or(0, |p| p.attempts), left);
        }
        assert!(guard.password().is_none());
        let happened = trust.happened();
        assert_eq!(happened.last(), Some(&Happened::Expired));
        assert_eq!(
            happened
                .iter()
                .filter(|h| matches!(h, Happened::Failed(_)))
                .count(),
            ATTEMPTS as usize
        );

        trust.start_pairing().unwrap();
        t.fetch_add(LIFETIME_SECS - 1, Ordering::Relaxed);
        assert!(trust.pairing_open());
        t.fetch_add(1, Ordering::Relaxed);
        assert!(!trust.pairing_open());
        assert_eq!(trust.happened(), [Happened::Expired]);
    }

    #[test]
    fn two_consoles_at_most_and_a_removed_one_is_not_pinned() {
        let (_, now) = clock();
        let trust = Trust::with(Consoles::default(), None, now);
        for (n, name) in [(1, "MC1"), (2, "MC2")] {
            trust.start_pairing().unwrap();
            trust.guard("x").password();
            trust.guard("x").paired(&paired(n, name));
        }
        assert_eq!(trust.start_pairing(), Err(Refusal::Full));
        assert!(!trust.pairing_open());
        let gone = trust.remove(&print(1)).unwrap();
        assert_eq!(gone.name, "MC1");
        assert!(!trust.pinned(&print(1)));
        assert!(trust.pinned(&print(2)));
        assert_eq!(trust.remove(&print(1)), Err(Refusal::NoConsole));
        assert!(trust.start_pairing().is_ok());
    }

    #[test]
    fn the_password_is_never_in_debug_or_the_file() {
        let (_, now) = clock();
        let dir = std::env::temp_dir().join(format!("qsiserver-trust-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let trust = Trust::open(dir.clone(), now.clone()).unwrap();
        let p = trust.start_pairing().unwrap();
        assert!(!format!("{p:?}").contains(&p.text));
        trust.guard("x").password();
        trust.guard("x").paired(&paired(3, "MC\u{7}3"));
        let file = std::fs::read_to_string(dir.join("qsi-consoles.toml")).unwrap();
        assert!(!file.contains(&p.text));
        assert!(file.contains(&print(3).hex()));
        // Read back as kept, the name without its control character.
        let again = Trust::open(dir.clone(), now).unwrap();
        assert_eq!(again.consoles().consoles[0].name, "MC3");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
