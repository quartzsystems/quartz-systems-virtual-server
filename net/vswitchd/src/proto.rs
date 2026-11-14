// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The switch service's socket (`docs/VSWITCHD.md`): one request and
//! one answer per connection, each an `ipc` frame, as the partition
//! manager's and the pool service's. A request that fails answers with
//! every `QVS` message that applies and changes nothing. Two requests
//! carry more than frames:
//!
//! - **open port**: after an answer that gives the port, the tap's open
//!   file follows on one byte (`base/fdpass`).
//! - **subscribe**: after its answer, every event follows, a frame
//!   each, for as long as the connection is open.

use qvsmsg::Issued;
use vswitch::port::PortSpec;
use vswitch::{Limits, Switch, Uplink};

/// The service's name in the service table.
pub const NAME: &str = "vswitchd";
/// The socket's file in the run directory.
pub const SOCKET_FILE: &str = "qsvsvswitchd.sock";
/// The longest request or answer.
pub const MAX_FRAME: usize = 1024 * 1024;

/// What a request that failed answers with: every message.
pub type Refused = Vec<Issued>;

/// The socket of the service whose run directory is `run_dir`.
pub fn socket(run_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(run_dir).join(SOCKET_FILE)
}

/// One interface of an uplink, as it is now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    /// Its location code.
    pub location: String,
    /// Its kernel name; empty when it is not on the host.
    pub name: String,
    /// Whether it has carrier.
    pub carrier: bool,
    /// Its speed, when it has a link and its driver says.
    pub speed_mbps: u32,
}
ipc::wire_struct!(Member {
    location,
    name,
    carrier,
    speed_mbps
});

/// An uplink's state.
pub mod uplink_state {
    /// The switch has no uplink: it is internal.
    pub const NONE: &str = "None";
    /// Every interface has carrier.
    pub const UP: &str = "Up";
    /// A bond with an interface down and another up.
    pub const DEGRADED: &str = "Degraded";
    /// No interface has carrier.
    pub const DOWN: &str = "Down";
    /// An interface of it is not on the host.
    pub const MISSING: &str = "Missing";
}

/// A switch, as a list shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SwitchStatus {
    /// The switch as it is kept.
    pub switch: Option<Switch>,
    /// Its bridge in the kernel.
    pub bridge: String,
    /// One of [`uplink_state`].
    pub uplink_state: String,
    /// The uplink's interfaces.
    pub members: Vec<Member>,
    /// The partitions' ports on it.
    pub ports: u32,
    /// Why it was not built; empty when it was.
    pub failure: String,
}
ipc::wire_struct!(SwitchStatus {
    switch,
    bridge,
    uplink_state,
    members,
    ports,
    failure
});

/// One network interface of the host, as Display network hardware
/// shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// Its kernel name.
    pub name: String,
    /// Its location code.
    pub location: String,
    /// Its MAC address.
    pub mac: String,
    /// Its driver.
    pub driver: String,
    /// Its speed in Mb/s; 0 with no link or none said.
    pub speed_mbps: u32,
    /// Whether it has carrier.
    pub carrier: bool,
    /// The largest MTU it takes; 0 when its driver does not say.
    pub max_mtu: u32,
    /// What it is: `Management`, `Uplink of PRODNET`, `Bond member of
    /// PRODNET`, `Not used`.
    pub what: String,
    /// The switch it is an uplink of; empty for none.
    pub switch: String,
    /// Whether it is the management interface.
    pub management: bool,
}
ipc::wire_struct!(Interface {
    name,
    location,
    mac,
    driver,
    speed_mbps,
    carrier,
    max_mtu,
    what,
    switch,
    management
});

/// Counters of a port, from the partition's side: what it sent and
/// what it was sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// Bytes the partition sent.
    pub sent_bytes: u64,
    /// Frames the partition sent.
    pub sent_packets: u64,
    /// Frames the partition sent that were dropped.
    pub sent_dropped: u64,
    /// Bytes sent to the partition.
    pub received_bytes: u64,
    /// Frames sent to the partition.
    pub received_packets: u64,
    /// Frames for the partition that were dropped.
    pub received_dropped: u64,
}
ipc::wire_struct!(Counters {
    sent_bytes,
    sent_packets,
    sent_dropped,
    received_bytes,
    received_packets,
    received_dropped
});

/// A partition's port.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PortStatus {
    /// What it was opened as.
    pub spec: PortSpec,
    /// Its tap in the kernel.
    pub tap: String,
    /// Whether its tap is there with carrier: a VMM has it open.
    pub link: bool,
    /// Its counters.
    pub counters: Counters,
}
ipc::wire_struct!(PortStatus {
    spec,
    tap,
    link,
    counters
});

/// One entry of a switch's forwarding table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FdbRow {
    /// The address.
    pub mac: String,
    /// Its VLAN; 0 for none.
    pub vlan: u16,
    /// Where it is: a partition's port, `uplink`, or the bridge itself.
    pub port: String,
    /// The partition, for a partition's port.
    pub partition: u32,
    /// The slot, for a partition's port.
    pub slot: u32,
    /// `static`, `learned` or `local`.
    pub origin: String,
}
ipc::wire_struct!(FdbRow {
    mac,
    vlan,
    port,
    partition,
    slot,
    origin
});

/// Who is on one VLAN of a switch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VlanUse {
    /// The switch.
    pub switch: String,
    /// The VLAN.
    pub vlan: u16,
    /// Whether the uplink carries it untagged.
    pub native: bool,
    /// The partitions' ports on it: partition, slot and whether tagged.
    pub ports: Vec<(u32, u32, bool)>,
}
ipc::wire_struct!(VlanUse {
    switch,
    vlan,
    native,
    ports
});

/// A physical interface the host's inventory has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Nic {
    /// Its kernel name.
    pub name: String,
    /// Its location code.
    pub location: String,
    /// Its MAC address.
    pub mac: String,
    /// Its driver.
    pub driver: String,
    /// Its speed in Mb/s, when it has a link.
    pub speed_mbps: Option<u32>,
    /// Whether it has carrier.
    pub carrier: bool,
}

/// An Ethernet adapter of a partition's profile, as the partition
/// manager says it: what names a switch and a VLAN of it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Adapter {
    /// The partition.
    pub partition: u32,
    /// Its name.
    pub name: String,
    /// The profile.
    pub profile: String,
    /// The slot.
    pub slot: u32,
    /// The switch.
    pub switch: String,
    /// The port VLAN.
    pub vlan: u16,
    /// The tagged VLANs.
    pub tagged: Vec<u16>,
    /// Its MAC address.
    pub mac: String,
    /// Whether the partition is activated only with its port.
    pub required: bool,
}
ipc::wire_struct!(Adapter {
    partition,
    name,
    profile,
    slot,
    switch,
    vlan,
    tagged,
    mac,
    required
});

/// What a port that was opened is: its tap's open file follows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Given {
    /// The port.
    pub spec: PortSpec,
    /// Its tap.
    pub tap: String,
    /// The switch's MTU.
    pub mtu: u32,
}
ipc::wire_struct!(Given { spec, tap, mtu });

/// What changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A switch was made, changed or deleted: its status, or none when
    /// it is gone.
    Switch {
        /// Its name.
        name: String,
        /// Its status now.
        status: Option<SwitchStatus>,
        /// When.
        time: u64,
    },
    /// An uplink's interface lost or regained carrier.
    Uplink {
        /// The switch.
        switch: String,
        /// The interface's location code.
        location: String,
        /// Whether it has carrier now.
        carrier: bool,
        /// The uplink's state now.
        state: String,
        /// When.
        time: u64,
    },
    /// A partition's port was opened or closed.
    Port {
        /// The partition.
        partition: u32,
        /// The slot.
        slot: u32,
        /// The switch.
        switch: String,
        /// Whether it is there now.
        open: bool,
        /// When.
        time: u64,
    },
}
ipc::wire_enum!(Event {
    0 => Switch { name, status, time },
    1 => Uplink { switch, location, carrier, state, time },
    2 => Port { partition, slot, switch, open, time },
});

ipc::rpc! {
    request Request, service Switches, client Calls, dispatch dispatch;
    /// Every switch.
    0 => fn switches / Switches () -> Vec<SwitchStatus>;
    /// Make an internal switch.
    1 => fn create / Create (name: String, vlans: Vec<u16>, mtu: u32) -> Result<SwitchStatus, Refused>;
    /// Change a switch's VLANs and MTU, while partitions run on it.
    2 => fn change / Change (name: String, vlans: Vec<u16>, mtu: u32) -> Result<(), Refused>;
    /// Delete a switch no partition has a port on. One a profile names
    /// is deleted only when `force`.
    3 => fn delete / Delete (name: String, force: bool) -> Result<(), Refused>;
    /// Rename a switch no profile names.
    4 => fn rename / Rename (name: String, to: String) -> Result<(), Refused>;
    /// Set a switch's uplink, or move it to other interfaces.
    5 => fn set_uplink / SetUplink (name: String, uplink: Uplink) -> Result<(), Refused>;
    /// Clear a switch's uplink: it is internal from then on.
    6 => fn clear_uplink / ClearUplink (name: String) -> Result<(), Refused>;
    /// Every network interface of the host and what it is.
    7 => fn interfaces / Interfaces () -> Vec<Interface>;
    /// The partitions' ports: of one switch, or every one when empty.
    8 => fn ports / Ports (switch: String) -> Vec<PortStatus>;
    /// A switch's forwarding table.
    9 => fn fdb / Fdb (switch: String) -> Result<Vec<FdbRow>, Refused>;
    /// Who is on which VLAN of every switch.
    10 => fn vlans / Vlans () -> Vec<VlanUse>;
    /// Open a partition's port: the tap's open file follows.
    11 => fn open_port / OpenPort (spec: PortSpec) -> Result<Given, Refused>;
    /// Close a partition's port in `slot`, or every one of its with 0.
    12 => fn close_port / ClosePort (partition: u32, slot: u32) -> u32;
    /// The limits.
    13 => fn limits / GetLimits () -> Limits;
    /// Change the limits.
    14 => fn change_limits / ChangeLimits (limits: Limits) -> Result<(), Refused>;
    /// Nothing; then every event, a frame each.
    15 => fn subscribe / Subscribe () -> ();
    /// The partitions that run now: every port of another is closed.
    /// How many were. The partition manager's request when it starts.
    16 => fn reconcile / Reconcile (running: Vec<u32>) -> u32;
}

/// A client: each call is its own connection.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct Client {
    path: std::path::PathBuf,
}

/// How long a client waits for an answer.
pub const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

#[cfg(unix)]
impl Client {
    /// A client of the socket at `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn connect(&self) -> Result<std::os::unix::net::UnixStream, ipc::WireError> {
        let stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        stream.set_read_timeout(Some(ANSWER_WAIT))?;
        stream.set_write_timeout(Some(ANSWER_WAIT))?;
        Ok(stream)
    }

    /// One request, encoded, and its encoded answer.
    pub fn exchange(&self, request: &[u8]) -> Result<Vec<u8>, ipc::WireError> {
        let mut stream = self.connect()?;
        ipc::frame::write_frame(&mut stream, request)?;
        ipc::frame::read_frame_max(&mut stream, MAX_FRAME)
    }

    /// Open the port `spec`: what was given, with the tap's open file.
    pub fn open_port_file(
        &self,
        spec: PortSpec,
    ) -> Result<Result<(Given, std::fs::File), Refused>, ipc::WireError> {
        let mut stream = self.connect()?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(&Request::OpenPort { spec }))?;
        let answer = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        let given: Result<Given, Refused> = ipc::decode(&answer)?;
        let given = match given {
            Ok(given) => given,
            Err(refused) => return Ok(Err(refused)),
        };
        let mut fds = fdpass::recv(&stream, 1)?;
        let Some(fd) = fds.pop() else {
            return Err(ipc::WireError::from(std::io::Error::other(
                "the tap's file did not follow",
            )));
        };
        Ok(Ok((given, std::fs::File::from(fd))))
    }

    /// Subscribe: every event, as it comes, until `f` returns false or
    /// the connection ends.
    pub fn events(&self, mut f: impl FnMut(Event) -> bool) -> Result<(), ipc::WireError> {
        let mut stream = self.connect()?;
        stream.set_read_timeout(None)?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(&Request::Subscribe {}))?;
        let _ = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
        loop {
            let frame = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
            if !f(ipc::decode(&frame)?) {
                return Ok(());
            }
        }
    }
}

#[cfg(unix)]
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        self.exchange(&ipc::encode(req))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_socket_carries_round_trips() {
        let spec = PortSpec {
            partition: 3,
            slot: 5,
            switch: "PRODNET".into(),
            vlan: 10,
            tagged: vec![20, 30],
            mac: [2, 1, 2, 3, 4, 5],
        };
        let r = Request::OpenPort { spec: spec.clone() };
        let back: Request = ipc::decode(&ipc::encode(&r)).unwrap();
        assert!(matches!(back, Request::OpenPort { spec: s } if s == spec));
        let status = SwitchStatus {
            switch: Some(Switch {
                name: "PRODNET".into(),
                id: 1,
                vlans: vec![10],
                mtu: 1500,
                uplink: Some(Uplink {
                    interfaces: vec!["P1".into()],
                    bond: None,
                    native_vlan: Some(10),
                }),
            }),
            uplink_state: uplink_state::UP.into(),
            ..SwitchStatus::default()
        };
        let back: SwitchStatus = ipc::decode(&ipc::encode(&status)).unwrap();
        assert_eq!(back, status);
        let e = Event::Uplink {
            switch: "PRODNET".into(),
            location: "P1".into(),
            carrier: false,
            state: uplink_state::DOWN.into(),
            time: 7,
        };
        let back: Event = ipc::decode(&ipc::encode(&e)).unwrap();
        assert_eq!(back, e);
        let v = VlanUse {
            switch: "A".into(),
            vlan: 10,
            native: true,
            ports: vec![(1, 2, false)],
        };
        let back: VlanUse = ipc::decode(&ipc::encode(&v)).unwrap();
        assert_eq!(back, v);
    }
}
