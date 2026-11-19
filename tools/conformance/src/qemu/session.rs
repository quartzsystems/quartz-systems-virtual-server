// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One running guest: `base/qemu`'s session, with the test hooks'
//! channel on its second serial port.

use std::ops::{Deref, DerefMut};
use std::time::Instant;

use crate::error::HarnessError;
use crate::qemu::Launch;
use crate::qemu::channel::Channel;

/// A guest under test. Killed when dropped.
pub struct Session {
    inner: driver::Session,
    channel: Option<Channel>,
}

impl Session {
    /// Launch QEMU.
    pub fn spawn(launch: &Launch) -> Result<Self, HarnessError> {
        Ok(Self {
            inner: driver::Session::spawn(launch)?,
            channel: None,
        })
    }

    /// The test hooks' channel, connected and answering, by `deadline`.
    pub fn channel(&mut self, deadline: Instant) -> Result<&mut Channel, String> {
        if self.channel.is_none() {
            let socket = self
                .inner
                .hook()
                .ok_or("this guest was launched without the channel")?;
            let mut channel = Channel::connect(socket, deadline)?;
            channel.wait_ready(deadline)?;
            self.channel = Some(channel);
        }
        self.channel.as_mut().ok_or_else(|| "no channel".into())
    }

    /// Kill the guest, as a power loss does, and take what it still
    /// had to say. The channel goes with it.
    pub fn kill(&mut self) {
        self.inner.kill();
        self.channel = None;
    }
}

impl Deref for Session {
    type Target = driver::Session;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Session {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
