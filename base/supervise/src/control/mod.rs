// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! PID 1's control socket: one request and one answer per connection,
//! each an `ipc` frame.

#[cfg(unix)]
mod socket;

#[cfg(unix)]
pub use socket::{Client, answer, exchange, serve};

/// What a posted reference code is about: a resource a person finds by
/// its name, its location code and its serial number.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct About {
    /// Its kind: one of [`about`].
    pub kind: String,
    /// A number of its kind's: a partition's ID; 0 for none.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub id: u32,
    /// The name a person finds it by.
    #[serde(default)]
    pub name: String,
    /// Its location code, where it has one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub location: String,
    /// Its serial number, where it has one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
}
ipc::wire_struct!(About {
    kind,
    id,
    name,
    location,
    serial
});

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's signature
fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The kinds of [`About`].
pub mod about {
    /// A disk unit.
    pub const DISK_UNIT: &str = "disk-unit";
    /// A storage pool.
    pub const STORAGE_POOL: &str = "storage-pool";
    /// A virtual disk.
    pub const VIRTUAL_DISK: &str = "virtual-disk";
    /// A physical network interface.
    pub const NETWORK_INTERFACE: &str = "network-interface";
    /// A virtual switch.
    pub const VIRTUAL_SWITCH: &str = "virtual-switch";
    /// A partition: its ID and its name.
    pub const PARTITION: &str = "partition";
    /// A slot of the appliance's own code.
    pub const CODE_SLOT: &str = "code-slot";
    /// A supervised service.
    pub const SERVICE: &str = "service";
    /// The appliance itself, or a part of it with no kind of its own.
    pub const SYSTEM: &str = "system";
    /// A reference code a partition's own system posted.
    pub const SYSTEM_CODE: &str = "system-code";
    /// A PCI device: its location code.
    pub const DEVICE: &str = "device";
    /// A physical tape drive: its location code.
    pub const TAPE_DRIVE: &str = "tape-drive";
}

/// The appliance's code slots, as its A/B record and this IPL have them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotState {
    /// The slot this IPL came from: `A` or `B`; empty without a record.
    pub booted: String,
    /// The record's default slot.
    pub default: String,
    /// The slot pending; empty when none is.
    pub pending: String,
    /// The pending slot was activated: the restart into it is under way
    /// or its trial began.
    pub activated: bool,
    /// The release last confirmed in slot A and in slot B; empty for
    /// none.
    pub confirmed: Vec<String>,
    /// The slot that did not confirm, when this IPL came back after it.
    pub fell_back: String,
}
ipc::wire_struct!(SlotState {
    booted,
    default,
    pending,
    activated,
    confirmed,
    fell_back
});

/// One service, as status shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceStatus {
    /// Its name.
    pub name: String,
    /// `waiting`, `starting`, `ready`, `restarting`, `stopped`,
    /// `stopping` or `ended`.
    pub state: String,
    /// Its process, 0 when it has none.
    pub pid: u32,
    /// How often it has failed.
    pub failures: u32,
}
ipc::wire_struct!(ServiceStatus {
    name,
    state,
    pid,
    failures
});

/// The host, as status shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostStatus {
    /// What the status screen shows, in its order: key and value.
    pub facts: Vec<(String, String)>,
    /// Every service of the table, in start order.
    pub services: Vec<ServiceStatus>,
}
ipc::wire_struct!(HostStatus { facts, services });

impl HostStatus {
    /// The value of one fact.
    pub fn fact(&self, key: &str) -> Option<&str> {
        self.facts
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// One service.
    pub fn service(&self, name: &str) -> Option<&ServiceStatus> {
        self.services.iter().find(|s| s.name == name)
    }
}

/// Where the host is in its life.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Power {
    /// The IPL: the base services are not all ready yet.
    #[default]
    Starting,
    /// The base is ready.
    Running,
    /// Shutting down.
    Ending,
}
ipc::wire_enum!(Power { 0 => Starting, 1 => Running, 2 => Ending });

/// The host's state: what a management console is told of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostState {
    /// Where it is in its life.
    pub power: Power,
    /// The current reference code: its code and its text, empty
    /// before the first.
    pub code: String,
    /// Its text.
    pub text: String,
    /// The release that runs.
    pub release: String,
    /// The slot it runs from (`A`, `B`, or empty).
    pub slot: String,
    /// A power down asked for later: the seconds until it begins.
    pub power_in: Option<u32>,
}
ipc::wire_struct!(HostState {
    power,
    code,
    text,
    release,
    slot,
    power_in
});

/// The longest delay a power down may be asked with, in seconds.
pub const MAX_POWER_DELAY: u32 = 99_999;

ipc::rpc! {
    request Request, service Control, client Calls, dispatch dispatch;
    /// The status of the host and of every service.
    0 => fn status / Status () -> HostStatus;
    /// Shut down in order and power off.
    1 => fn power_off / PowerOff () -> ();
    /// Shut down in order and restart.
    2 => fn restart / Restart () -> ();
    /// Post a reference code: a service's, through PID 1.
    3 => fn post / Post (code: String, text: String) -> ();
    /// A fact for the status screen.
    4 => fn note / Note (key: String, value: String) -> ();
    /// Set the host's name, display zone and time servers: PID 1
    /// writes the host's state file and takes the name and the zone
    /// into use.
    5 => fn set_host / SetHost (name: String, time_zone: String, ntp_servers: Vec<String>) -> Result<(), String>;
    /// The host's state.
    6 => fn state / State () -> HostState;
    /// Power off or restart, after `delay` seconds; `immediate` ends
    /// each partition at once instead of through its system. `by` is
    /// who asked, for the log and the reference code.
    7 => fn power_down / PowerDown (restart: bool, immediate: bool, delay: u32, by: String) -> Result<(), String>;
    /// Post a reference code with what it is about.
    8 => fn post_about / PostAbout (code: String, text: String, about: Vec<About>) -> ();
    /// Take the appliance's own service dump, for `by`: its name, or
    /// why not. It is written in the background.
    9 => fn take_dump / TakeDump (by: String) -> Result<String, String>;
    /// The code slots.
    10 => fn slots / Slots () -> SlotState;
    /// `slot` holds a new release, written on every load-source disk:
    /// it is pending, not activated.
    11 => fn install_slot / InstallSlot (slot: String) -> Result<(), String>;
    /// Nothing is pending.
    12 => fn cancel_slot / CancelSlot () -> Result<(), String>;
    /// Restart into the pending slot, for `by`: its attempt is spent
    /// and the firmware boots it next.
    13 => fn activate_slot / ActivateSlot (by: String) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trips() {
        let status = HostStatus {
            facts: vec![("slot".into(), "A".into())],
            services: vec![ServiceStatus {
                name: "logd".into(),
                state: "ready".into(),
                pid: 7,
                failures: 0,
            }],
        };
        let back: HostStatus = ipc::decode(&ipc::encode(&status)).unwrap();
        assert_eq!(back, status);
        assert_eq!(back.fact("slot"), Some("A"));
        assert_eq!(back.service("logd").unwrap().pid, 7);
        assert!(back.service("netd").is_none());
    }
}
