// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A change of the network made from the local console is on trial
//! first: it is applied, and written to `network.toml` only once each
//! interface it changes has carrier and an address with it, or the
//! operator says to keep it anyway. A change that has neither after
//! thirty seconds is taken back: the configuration of before is put on
//! the interfaces again and the file was never touched. So a mistyped
//! address cannot leave a host that is reached over that network
//! unreachable. The interfaces a change leaves alone are not judged.
//!
//! A change can instead be kept only when it is confirmed
//! ([`Trial::begin_confirmed`]): whatever the interfaces say, it is
//! taken back when its window ends unless the operator confirmed it
//! within it. A console reached over the network it changes uses it: an
//! address the operator cannot reach is never kept.

use std::time::{Duration, Instant};

use state::{Interface, Network, Store};

use crate::manage::{Kernel, Manager};

/// How long a change may be without carrier or address.
pub const WINDOW: Duration = Duration::from_secs(30);
/// The longest window a change kept only when confirmed may have.
pub const MAX_WINDOW: Duration = Duration::from_secs(600);

/// The window of a change kept only when confirmed, of `seconds`: none
/// is [`WINDOW`]; more than [`MAX_WINDOW`] is refused.
pub fn window(seconds: u32) -> Result<Duration, String> {
    let asked = Duration::from_secs(u64::from(seconds));
    if seconds == 0 {
        Ok(WINDOW)
    } else if asked > MAX_WINDOW {
        Err(format!(
            "a change is confirmed within {} seconds at most, not {seconds}",
            MAX_WINDOW.as_secs()
        ))
    } else {
        Ok(asked)
    }
}

/// What a trial has come to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not decided yet.
    Wait,
    /// The change is kept and written.
    Keep,
    /// The configuration of before is restored.
    Restore,
}

/// A change on trial.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trial {
    /// The configuration before the change.
    pub old: Network,
    /// The change.
    pub new: Network,
    /// When it is taken back.
    pub deadline: Instant,
    /// The operator said to keep it whatever the interface says.
    pub confirmed: bool,
    /// It is kept only when confirmed, whatever the interfaces say.
    pub only_confirmed: bool,
    /// How long it had.
    pub window: Duration,
}

/// The roles (as written) of the interfaces of `new` that are not in
/// `old` as they are: what a change touches, and is judged by.
pub fn changed(old: &Network, new: &Network) -> Vec<String> {
    new.interfaces
        .iter()
        .filter(|n| !old.interfaces.contains(n))
        .map(|n: &Interface| n.role.clone())
        .collect()
}

impl Trial {
    /// A trial of `new` over `old`, begun at `now`.
    pub fn begin(old: Network, new: Network, now: Instant) -> Self {
        Self {
            old,
            new,
            deadline: now + WINDOW,
            confirmed: false,
            only_confirmed: false,
            window: WINDOW,
        }
    }

    /// A trial of `new` over `old`, begun at `now`, kept only when it is
    /// confirmed within `window`.
    pub fn begin_confirmed(old: Network, new: Network, now: Instant, window: Duration) -> Self {
        Self {
            deadline: now + window,
            only_confirmed: true,
            window,
            ..Self::begin(old, new, now)
        }
    }

    /// What the trial has come to at `now`, the interface being
    /// `working` (carrier and an address) or not.
    pub fn judge(&self, working: bool, now: Instant) -> Verdict {
        if self.confirmed || (working && !self.only_confirmed) {
            Verdict::Keep
        } else if now >= self.deadline {
            Verdict::Restore
        } else {
            Verdict::Wait
        }
    }

    /// Seconds until the change is taken back.
    pub fn seconds_left(&self, now: Instant) -> u32 {
        u32::try_from(self.deadline.saturating_duration_since(now).as_secs()).unwrap_or(0)
    }
}

/// Settle `trial` at `now` on `manager`: the trial still open, or what
/// the change came to. A change kept is written to `store`; one taken
/// back, or that could not be written, has the configuration of before
/// put on the interfaces again (`host_name` the host's name).
pub fn settle<K: Kernel>(
    trial: Trial,
    manager: &mut Manager<K>,
    store: &Store,
    host_name: &str,
    now: Instant,
) -> Result<Trial, Last> {
    let touched = changed(&trial.old, &trial.new);
    let failing = manager
        .statuses()
        .into_iter()
        .find(|s| touched.contains(&s.role) && (!s.carrier || s.address.is_empty()));
    match trial.judge(failing.is_none(), now) {
        Verdict::Wait => Ok(trial),
        Verdict::Keep => Err(match store.save(&trial.new) {
            Ok(()) => Last::Kept,
            Err(e) => {
                manager.reload(trial.old, host_name.to_string(), now);
                Last::Restored(format!("it could not be written: {e}"))
            }
        }),
        Verdict::Restore => {
            let why = if trial.only_confirmed {
                format!(
                    "it was not confirmed within {} seconds",
                    trial.window.as_secs()
                )
            } else {
                failing.map(|s| s.line).unwrap_or_default()
            };
            manager.reload(trial.old, host_name.to_string(), now);
            Err(Last::Restored(why))
        }
    }
}

/// How the last change went, for status.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Last {
    /// No change was made since the service started.
    #[default]
    None,
    /// A change is on trial.
    Trying,
    /// The change was kept and written.
    Kept,
    /// The change was taken back, or could not be written.
    Restored(String),
}

impl Last {
    /// The word status has for it.
    pub fn word(&self) -> &'static str {
        match self {
            Last::None => crate::control::TRIAL_NONE,
            Last::Trying => crate::control::TRIAL_TRYING,
            Last::Kept => crate::control::TRIAL_KEPT,
            Last::Restored(_) => crate::control::TRIAL_RESTORED,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_is_kept_when_it_works_or_is_confirmed_and_restored_otherwise() {
        let now = Instant::now();
        let mut trial = Trial::begin(Network::default(), Network::default(), now);
        assert_eq!(trial.judge(false, now), Verdict::Wait);
        assert_eq!(trial.seconds_left(now), 30);
        assert_eq!(trial.judge(true, now), Verdict::Keep);
        let late = now + WINDOW;
        let before = now + WINDOW.saturating_sub(Duration::from_secs(1));
        assert_eq!(trial.judge(false, before), Verdict::Wait);
        assert_eq!(trial.judge(false, late), Verdict::Restore);
        assert_eq!(trial.seconds_left(late), 0);
        // The operator's word outweighs the interface.
        trial.confirmed = true;
        assert_eq!(trial.judge(false, now), Verdict::Keep);
        assert_eq!(Last::Restored("x".into()).word(), "restored");
        assert_eq!(Last::default().word(), "none");
    }

    #[test]
    fn a_change_kept_only_when_confirmed_is_not_kept_for_working() {
        let now = Instant::now();
        let window = Duration::from_secs(120);
        let mut trial = Trial::begin_confirmed(Network::default(), Network::default(), now, window);
        assert_eq!(trial.judge(true, now), Verdict::Wait);
        assert_eq!(trial.seconds_left(now), 120);
        assert_eq!(trial.judge(true, now + window), Verdict::Restore);
        trial.confirmed = true;
        assert_eq!(trial.judge(false, now), Verdict::Keep);
        assert_eq!(super::window(0), Ok(WINDOW));
        assert_eq!(super::window(600), Ok(MAX_WINDOW));
        assert!(super::window(601).is_err());
    }

    #[test]
    fn a_change_touches_only_the_interfaces_that_differ() {
        let public = Interface {
            role: "public".into(),
            ..Interface::default()
        };
        let private = Interface {
            role: "private".into(),
            vlan: Some(20),
            carries_route: false,
            ..Interface::default()
        };
        let old = Network {
            interfaces: vec![public.clone(), private.clone()],
        };
        let mut new = old.clone();
        assert!(changed(&old, &new).is_empty());
        new.interfaces[1].vlan = Some(30);
        assert_eq!(changed(&old, &new), ["private"]);
        assert_eq!(
            changed(
                &Network::default(),
                &Network::from(Interface {
                    vlan: Some(7),
                    ..Interface::default()
                })
            ),
            [""]
        );
    }
}
