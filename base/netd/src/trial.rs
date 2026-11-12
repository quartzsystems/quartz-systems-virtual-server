// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A change of the management network made from the local console is
//! on trial first: it is applied, and written to `network.toml` only
//! once the interface has carrier and an address with it, or the
//! operator says to keep it anyway. A change that has neither after
//! thirty seconds is taken back: the configuration of before is put on
//! the interface again and the file was never touched. So a mistyped
//! address cannot leave a host that is reached over that network
//! unreachable.

use std::time::{Duration, Instant};

use state::Network;

/// How long a change may be without carrier or address.
pub const WINDOW: Duration = Duration::from_secs(30);

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
}

impl Trial {
    /// A trial of `new` over `old`, begun at `now`.
    pub fn begin(old: Network, new: Network, now: Instant) -> Self {
        Self {
            old,
            new,
            deadline: now + WINDOW,
            confirmed: false,
        }
    }

    /// What the trial has come to at `now`, the interface being
    /// `working` (carrier and an address) or not.
    pub fn judge(&self, working: bool, now: Instant) -> Verdict {
        if working || self.confirmed {
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
}
