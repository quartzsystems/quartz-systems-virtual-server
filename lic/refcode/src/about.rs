// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What a posted code is about (`supervise::control::About`), made the
//! same way by every poster: the names, location codes and serial
//! numbers a person finds each resource by.

pub use supervise::control::{About, about as kind};

fn of(kind: &str, name: &str) -> About {
    About {
        kind: kind.into(),
        name: name.into(),
        ..About::default()
    }
}

/// A disk unit: its name, location code and serial number.
#[must_use]
pub fn disk_unit(name: &str, location: &str, serial: &str) -> About {
    About {
        location: location.into(),
        serial: serial.into(),
        ..of(kind::DISK_UNIT, name)
    }
}

/// A storage pool.
#[must_use]
pub fn pool(name: &str) -> About {
    of(kind::STORAGE_POOL, name)
}

/// A virtual disk, named as one: `VD03 in pool POOL1`.
#[must_use]
pub fn vdisk(pool: &str, name: &str) -> About {
    of(kind::VIRTUAL_DISK, &format!("{name} in pool {pool}"))
}

/// A physical network interface: its name and location code.
#[must_use]
pub fn interface(name: &str, location: &str) -> About {
    About {
        location: location.into(),
        ..of(kind::NETWORK_INTERFACE, name)
    }
}

/// A virtual switch.
#[must_use]
pub fn switch(name: &str) -> About {
    of(kind::VIRTUAL_SWITCH, name)
}

/// A partition: its ID, name and serial number.
#[must_use]
pub fn partition(id: u32, name: &str, serial: &str) -> About {
    About {
        id,
        serial: serial.into(),
        ..of(kind::PARTITION, name)
    }
}

/// A slot of the host's code: `A` or `B`.
#[must_use]
pub fn slot(letter: &str) -> About {
    of(kind::CODE_SLOT, letter)
}

/// A supervised service.
#[must_use]
pub fn service(name: &str) -> About {
    of(kind::SERVICE, name)
}

/// The code a partition's own system posted.
#[must_use]
pub fn system_code(code: &str) -> About {
    of(kind::SYSTEM_CODE, code)
}
