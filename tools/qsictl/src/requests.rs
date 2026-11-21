// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Every request of QSI's families that is one request and one reply,
//! as a subcommand; the reply printed whole.

use clap::{Subcommand, ValueEnum};
use qsi::Empty;
use qsi::any::{AnyReply, AnyRequest};
use qsi::base::{self, PowerAction};
use qsi::host::model::IplSource;
use qsi::host::{dumps, host, panel, partition, partitions, profiles, refcodes, vres};
use qsisess::Reply;

use qsictl::link::Link;

/// A shut down's kind.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Kind {
    /// The system ends its work within a delay.
    Delayed,
    /// The system shuts down at once.
    System,
    /// The VMM is stopped.
    Immediate,
}

/// An IPL source.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Source {
    /// Side A.
    A,
    /// Side B.
    B,
    /// The side the system recorded.
    Recorded,
    /// Removable media.
    D,
}

/// The requests.
#[derive(Subcommand, Debug)]
pub enum Ask {
    /// Who the host is.
    Identify,
    /// The host's state.
    State,
    /// The host's clock.
    Time,
    /// The host's reference code history.
    Codes {
        /// Before this place.
        #[arg(long)]
        before: Option<u64>,
        /// How many.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// Power the host off, or restart it.
    Power {
        /// Restart rather than power off.
        #[arg(long)]
        restart: bool,
        /// End each partition at once.
        #[arg(long)]
        immediate: bool,
        /// Seconds to wait first.
        #[arg(long, default_value_t = 0)]
        delay: u32,
    },
    /// The host's resources.
    Resources,
    /// The host's settings.
    Settings,
    /// The host's hardware.
    Hardware,
    /// Every partition.
    List,
    /// One partition's definition and status.
    Get {
        /// The partition.
        id: u32,
    },
    /// The adapters of one partition, or of every one.
    Adapters {
        /// The partition; every one when left out.
        id: Option<u32>,
    },
    /// Make a partition; with an ID or settings, partition version 3's
    /// create.
    Create {
        /// Its name.
        name: String,
        /// Its ID; the host gives the lowest free when left out.
        #[arg(long)]
        id: Option<u32>,
        /// What it is for.
        #[arg(long)]
        description: Option<String>,
        /// Keep its control panel from consoles.
        #[arg(long)]
        no_panel: bool,
        /// Shut it down delayed with the host, with this delay.
        #[arg(long)]
        with_host_delay: Option<u32>,
    },
    /// A partition's settings.
    SettingsOf {
        /// The partition.
        id: u32,
    },
    /// Change a partition's settings, running or not.
    SetSettings {
        /// The partition.
        id: u32,
        /// What it is for.
        #[arg(long, default_value = "")]
        description: String,
        /// Keep its control panel from consoles.
        #[arg(long)]
        no_panel: bool,
        /// Shut it down delayed with the host, with this delay; at once
        /// when left out.
        #[arg(long)]
        with_host_delay: Option<u32>,
    },
    /// When each partition's activation began.
    Activations,
    /// When each of a partition's profiles was last activated.
    ProfileUses {
        /// The partition.
        id: u32,
    },
    /// The host's power-on settings.
    PowerOn,
    /// Change the host's power-on settings.
    SetPowerOn {
        /// Start no partition with the host.
        #[arg(long)]
        no_start: bool,
        /// End partitions at once at a power off.
        #[arg(long)]
        immediate: bool,
        /// The wait for each system, seconds.
        #[arg(long, default_value_t = 300)]
        delay: u32,
    },
    /// The control panel functions the host has for a partition.
    PanelFunctions {
        /// The partition.
        id: u32,
    },
    /// What a partition's dumps take; every partition's when left out.
    DumpSpace {
        /// The partition.
        id: Option<u32>,
    },
    /// Delete a partition.
    Delete {
        /// The partition.
        id: u32,
        /// Its virtual disks too.
        #[arg(long)]
        disks: bool,
    },
    /// Activate a partition.
    Activate {
        /// The partition.
        id: u32,
        /// The profile; its default when left out.
        #[arg(long, default_value = "")]
        profile: String,
        /// The IPL source, for this activation.
        #[arg(long)]
        source: Option<Source>,
    },
    /// Shut a partition down.
    Shutdown {
        /// The partition.
        id: u32,
        /// How.
        #[arg(long, value_enum, default_value = "immediate")]
        kind: Kind,
        /// A delayed shut down's seconds.
        #[arg(long, default_value_t = 0)]
        delay: u32,
        /// Activate it again after.
        #[arg(long)]
        restart: bool,
    },
    /// Restart a partition at once.
    Restart {
        /// The partition.
        id: u32,
    },
    /// A partition's profiles.
    Profiles {
        /// The partition.
        id: u32,
    },
    /// One profile.
    Profile {
        /// The partition.
        id: u32,
        /// The profile.
        name: String,
    },
    /// Save what an activated partition holds as a profile.
    SaveCurrent {
        /// The partition.
        id: u32,
        /// The profile's name.
        name: String,
        /// The revision it replaces.
        #[arg(long)]
        revision: Option<u64>,
    },
    /// Make a profile the partition's default.
    SetDefault {
        /// The partition.
        id: u32,
        /// The profile.
        name: String,
    },
    /// A partition's reference code history.
    History {
        /// The partition.
        id: u32,
        /// Before this place.
        #[arg(long)]
        before: Option<u64>,
        /// How many.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },
    /// A control panel function.
    Panel {
        /// The partition.
        id: u32,
        /// The function, 1 to 99.
        function: u8,
    },
    /// A partition's dumps.
    Dumps {
        /// The partition.
        id: u32,
    },
    /// Take a dump of a held partition.
    TakeDump {
        /// The partition.
        id: u32,
    },
    /// The console's screen now.
    Screen {
        /// The partition.
        id: u32,
    },
    /// The storage pools.
    Pools,
    /// The virtual disks of a pool, or of every one.
    Vdisks {
        /// The pool; every one when left out.
        pool: Option<String>,
    },
    /// The optical images.
    Images,
    /// The tape volumes.
    Volumes,
    /// A partition's optical and tape drives.
    Drives {
        /// The partition.
        id: u32,
    },
    /// The virtual switches.
    Switches,
    /// The partitions' ports on a switch, or on every one.
    Ports {
        /// The switch; every one when left out.
        switch: Option<String>,
    },
    /// The host's devices for physical I/O: released, held, or why not.
    Io,
    /// The host's physical tape drives.
    Tape,
    /// A running partition's processors.
    Partition {
        /// What to do.
        #[command(subcommand)]
        what: PartitionAsk,
    },
    /// Make and change storage pools.
    Pool {
        /// What to do.
        #[command(subcommand)]
        what: PoolAsk,
    },
    /// Make and change virtual switches.
    Switch {
        /// What to do.
        #[command(subcommand)]
        what: SwitchAsk,
    },
}

/// A storage pool's protection.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Protection {
    /// One disk.
    None,
    /// Two disks, each a copy.
    Mirrored,
    /// Three or more, one parity a stripe.
    Raid5,
    /// Four or more, two parities a stripe.
    Raid6,
}

/// What is asked of the storage pools.
#[derive(Subcommand, Debug)]
pub enum PoolAsk {
    /// The disks a pool may take.
    Disks,
    /// Make a pool of disks, by their names.
    Create {
        /// Its name.
        name: String,
        /// Its protection.
        #[arg(long, value_enum)]
        protection: Protection,
        /// Its disks.
        #[arg(long, value_delimiter = ',', required = true)]
        disks: Vec<String>,
        /// Its hot spares.
        #[arg(long, value_delimiter = ',')]
        spares: Vec<String>,
        /// The foreign disks' names typed again: they are erased.
        #[arg(long, value_delimiter = ',')]
        confirm: Vec<String>,
    },
    /// Delete an empty pool.
    Delete {
        /// The pool.
        name: String,
    },
    /// Rename a pool.
    Rename {
        /// The pool.
        name: String,
        /// Its new name.
        to: String,
    },
    /// Put a disk in a failed or missing unit's place.
    Replace {
        /// The pool.
        pool: String,
        /// The unit's number.
        unit: u32,
        /// The disk.
        disk: String,
        /// The disk is foreign and may be erased.
        #[arg(long)]
        confirm: bool,
    },
    /// Give a pool a hot spare.
    AddSpare {
        /// The pool.
        pool: String,
        /// The disk.
        disk: String,
        /// The disk is foreign and may be erased.
        #[arg(long)]
        confirm: bool,
    },
    /// Take a hot spare from a pool.
    RemoveSpare {
        /// The pool.
        pool: String,
        /// The disk.
        disk: String,
    },
    /// Make a pool the media repository; none when left out.
    Designate {
        /// The pool.
        pool: Option<String>,
    },
    /// Grow a pool (a QS/VS host refuses it: its pools' sizes are fixed).
    Extend {
        /// The pool.
        pool: String,
        /// Its new disks.
        #[arg(long, value_delimiter = ',', required = true)]
        disks: Vec<String>,
        /// The foreign disks' names typed again.
        #[arg(long, value_delimiter = ',')]
        confirm: Vec<String>,
    },
}

/// How an uplink's interfaces are bonded.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Bond {
    /// One carries the traffic; another takes over.
    ActiveBackup,
    /// LACP.
    Lacp,
}

/// What is asked of the virtual switches.
#[derive(Subcommand, Debug)]
pub enum SwitchAsk {
    /// Make an internal switch.
    Create {
        /// Its name.
        name: String,
        /// Its VLANs.
        #[arg(long, value_delimiter = ',')]
        vlans: Vec<u16>,
        /// Its MTU; the host's default when 0.
        #[arg(long, default_value_t = 0)]
        mtu: u32,
    },
    /// Change a switch's VLANs and MTU.
    Change {
        /// The switch.
        name: String,
        /// Its VLANs.
        #[arg(long, value_delimiter = ',')]
        vlans: Vec<u16>,
        /// Its MTU; the host's default when 0.
        #[arg(long, default_value_t = 0)]
        mtu: u32,
    },
    /// Rename a switch.
    Rename {
        /// The switch.
        name: String,
        /// Its new name.
        to: String,
    },
    /// Delete a switch.
    Delete {
        /// The switch.
        name: String,
        /// Even when a profile names it.
        #[arg(long)]
        force: bool,
    },
    /// Give a switch an uplink of interfaces, by location code.
    Uplink {
        /// The switch.
        switch: String,
        /// The interfaces.
        #[arg(long, value_delimiter = ',', required = true)]
        interfaces: Vec<String>,
        /// How several are bonded.
        #[arg(long, value_enum)]
        bond: Option<Bond>,
        /// The VLAN carried untagged.
        #[arg(long)]
        native: Option<u16>,
    },
    /// Take a switch's uplink away.
    ClearUplink {
        /// The switch.
        switch: String,
    },
}

fn pool_request(a: PoolAsk) -> vres::Request {
    use vres::Request as V;
    match a {
        PoolAsk::Disks => V::Disks(Empty),
        PoolAsk::Create {
            name,
            protection,
            disks,
            spares,
            confirm,
        } => V::PoolCreate(vres::PoolCreate {
            name,
            protection: match protection {
                Protection::None => vres::Protection::None,
                Protection::Mirrored => vres::Protection::Mirrored,
                Protection::Raid5 => vres::Protection::Raid5,
                Protection::Raid6 => vres::Protection::Raid6,
            },
            disks,
            spares,
            confirm,
        }),
        PoolAsk::Delete { name } => V::PoolDelete(vres::PoolDelete { name }),
        PoolAsk::Rename { name, to } => V::PoolRename(vres::PoolRename { name, to }),
        PoolAsk::Replace {
            pool,
            unit,
            disk,
            confirm,
        } => V::UnitReplace(vres::UnitReplace {
            pool,
            unit,
            disk,
            confirm,
        }),
        PoolAsk::AddSpare {
            pool,
            disk,
            confirm,
        } => V::SpareAdd(vres::SpareAdd {
            pool,
            disk,
            confirm,
        }),
        PoolAsk::RemoveSpare { pool, disk } => V::SpareRemove(vres::SpareRemove { pool, disk }),
        PoolAsk::Designate { pool } => V::Designate(vres::Designate {
            pool: pool.unwrap_or_default(),
        }),
        PoolAsk::Extend {
            pool,
            disks,
            confirm,
        } => V::PoolExtend(vres::PoolExtend {
            pool,
            disks,
            confirm,
        }),
    }
}

fn switch_request(a: SwitchAsk) -> vres::Request {
    use vres::Request as V;
    match a {
        SwitchAsk::Create { name, vlans, mtu } => {
            V::SwitchCreate(vres::SwitchSpec { name, vlans, mtu })
        }
        SwitchAsk::Change { name, vlans, mtu } => {
            V::SwitchChange(vres::SwitchSpec { name, vlans, mtu })
        }
        SwitchAsk::Rename { name, to } => V::SwitchRename(vres::SwitchRename { name, to }),
        SwitchAsk::Delete { name, force } => V::SwitchDelete(vres::SwitchDelete { name, force }),
        SwitchAsk::Uplink {
            switch,
            interfaces,
            bond,
            native,
        } => V::UplinkSet(vres::UplinkSet {
            switch,
            uplink: vres::Uplink {
                interfaces,
                bond: bond.map(|b| match b {
                    Bond::ActiveBackup => vres::Bond::ActiveBackup,
                    Bond::Lacp => vres::Bond::Lacp,
                }),
                native_vlan: native,
            },
        }),
        SwitchAsk::ClearUplink { switch } => V::UplinkClear(vres::UplinkClear { switch }),
    }
}

/// What is asked of a running partition.
#[derive(Subcommand, Debug)]
pub enum PartitionAsk {
    /// Change a running shared partition's entitlement: its units, its
    /// capping, its weight. What is left out stays.
    Entitlement {
        /// The partition.
        id: u32,
        /// Processing units, as `0.50`.
        #[arg(long, value_parser = units)]
        units: Option<u32>,
        /// Capped.
        #[arg(long, conflicts_with = "uncapped")]
        capped: bool,
        /// Uncapped.
        #[arg(long)]
        uncapped: bool,
        /// The uncapped weight, 0 to 255.
        #[arg(long)]
        weight: Option<u32>,
    },
}

/// Processing units as hundredths: `1`, `0.5`, `0.50`.
fn units(text: &str) -> Result<u32, String> {
    let (whole, part) = text.split_once('.').unwrap_or((text, ""));
    let whole: u32 = whole
        .parse()
        .map_err(|_| format!("{text} is not a number of units"))?;
    let part = match part.len() {
        0 => 0,
        1 => part
            .parse::<u32>()
            .map(|p| p * 10)
            .map_err(|e| e.to_string())?,
        2 => part.parse::<u32>().map_err(|e| e.to_string())?,
        _ => return Err(format!("{text}: hundredths at most")),
    };
    whole
        .checked_mul(100)
        .and_then(|w| w.checked_add(part))
        .ok_or_else(|| format!("{text} is too many units"))
}

fn settings(description: String, no_panel: bool, delay: Option<u32>) -> partition::Settings {
    partition::Settings {
        description,
        panel: !no_panel,
        with_host: if delay.is_some() {
            partition::WithHost::Delayed
        } else {
            partition::WithHost::Immediate
        },
        delay: delay.unwrap_or(0),
    }
}

fn source(s: Source) -> IplSource {
    match s {
        Source::A => IplSource::A,
        Source::B => IplSource::B,
        Source::Recorded => IplSource::Recorded,
        Source::D => IplSource::D,
    }
}

/// The request a subcommand asks.
#[must_use]
#[allow(clippy::too_many_lines)] // an arm a request, nothing more
pub fn request(a: Ask) -> AnyRequest {
    use AnyRequest as R;
    match a {
        Ask::Identify => R::Base(base::Request::Identify(Empty)),
        Ask::State => R::Base(base::Request::State(Empty)),
        Ask::Time => R::Base(base::Request::Time(Empty)),
        Ask::Codes { before, limit } => {
            R::Base(base::Request::Refcodes(base::Refcodes { before, limit }))
        }
        Ask::Power {
            restart,
            immediate,
            delay,
        } => R::Base(base::Request::Power(base::Power {
            action: if restart {
                PowerAction::Restart
            } else {
                PowerAction::Off
            },
            immediate,
            delay,
        })),
        Ask::Resources => R::Host(host::Request::Host(Empty)),
        Ask::Settings => R::Host(host::Request::Settings(Empty)),
        Ask::Hardware => R::Host(host::Request::Hardware(Empty)),
        Ask::Io => R::Host(host::Request::Devices(Empty)),
        Ask::Tape => R::Vres(vres::Request::TapeDrives(Empty)),
        Ask::Partition {
            what:
                PartitionAsk::Entitlement {
                    id,
                    units,
                    capped,
                    uncapped,
                    weight,
                },
        } => R::Partition(partition::Request::Entitlement(partition::Entitlement {
            id,
            units,
            sharing: if capped {
                Some(qsi::host::model::Sharing::Capped)
            } else if uncapped {
                Some(qsi::host::model::Sharing::Uncapped)
            } else {
                None
            },
            weight,
        })),
        Ask::List => R::Partitions(partitions::Request::List(Empty)),
        Ask::Get { id } => R::Partitions(partitions::Request::Get(partitions::Get { id })),
        Ask::Adapters { id } => {
            R::Partitions(partitions::Request::Adapters(partitions::Adapters {
                id: id.unwrap_or(0),
            }))
        }
        Ask::Create {
            name,
            id: None,
            description: None,
            no_panel: false,
            with_host_delay: None,
        } => R::Partition(partition::Request::Create(partition::Create { name })),
        Ask::Create {
            name,
            id,
            description,
            no_panel,
            with_host_delay,
        } => R::Partition(partition::Request::CreateWith(partition::CreateWith {
            id,
            name,
            settings: settings(description.unwrap_or_default(), no_panel, with_host_delay),
        })),
        Ask::SettingsOf { id } => {
            R::Partition(partition::Request::GetSettings(partition::GetSettings {
                id,
            }))
        }
        Ask::SetSettings {
            id,
            description,
            no_panel,
            with_host_delay,
        } => R::Partition(partition::Request::SetSettings(partition::SetSettings {
            id,
            settings: settings(description, no_panel, with_host_delay),
        })),
        Ask::Activations => R::Partitions(partitions::Request::Activations(Empty)),
        Ask::ProfileUses { id } => {
            R::Profiles(profiles::Request::Activated(profiles::Activated { id }))
        }
        Ask::PowerOn => R::Host(host::Request::PowerOn(Empty)),
        Ask::SetPowerOn {
            no_start,
            immediate,
            delay,
        } => R::Host(host::Request::SetPowerOn(host::PowerOn {
            start_partitions: !no_start,
            power_off: if immediate {
                host::PowerOff::Immediate
            } else {
                host::PowerOff::Delayed
            },
            delay,
        })),
        Ask::PanelFunctions { id } => R::Panel(panel::Request::Functions(panel::Functions { id })),
        Ask::DumpSpace { id } => R::Dumps(dumps::Request::Space(dumps::GetSpace {
            id: id.unwrap_or(0),
        })),
        Ask::Delete { id, disks } => {
            R::Partition(partition::Request::Delete(partition::Delete { id, disks }))
        }
        Ask::Activate {
            id,
            profile,
            source: s,
        } => R::Partition(partition::Request::Activate(partition::Activate {
            id,
            profile,
            source: s.map(source),
            keylock: None,
        })),
        Ask::Shutdown {
            id,
            kind,
            delay,
            restart,
        } => R::Partition(partition::Request::ShutDown(partition::ShutDown {
            id,
            kind: match kind {
                Kind::Delayed => partition::ShutdownKind::Delayed,
                Kind::System => partition::ShutdownKind::System,
                Kind::Immediate => partition::ShutdownKind::Immediate,
            },
            delay,
            restart,
        })),
        Ask::Restart { id } => R::Partition(partition::Request::Restart(partition::Restart { id })),
        Ask::Profiles { id } => R::Profiles(profiles::Request::List(profiles::List { id })),
        Ask::Profile { id, name } => {
            R::Profiles(profiles::Request::Get(profiles::Get { id, name }))
        }
        Ask::SaveCurrent { id, name, revision } => {
            R::Profiles(profiles::Request::SaveCurrent(profiles::SaveCurrent {
                id,
                name,
                revision,
            }))
        }
        Ask::SetDefault { id, name } => {
            R::Profiles(profiles::Request::SetDefault(profiles::SetDefault {
                id,
                name,
            }))
        }
        Ask::History { id, before, limit } => {
            R::Refcodes(refcodes::Request::History(refcodes::History {
                id,
                before,
                limit,
            }))
        }
        Ask::Panel { id, function } => R::Panel(panel::Request::Function(panel::Function {
            id,
            function,
            args: panel::PanelArgs {
                source: None,
                ipl_type: None,
                keylock: None,
            },
        })),
        Ask::Dumps { id } => R::Dumps(dumps::Request::List(dumps::List { id })),
        Ask::TakeDump { id } => R::Dumps(dumps::Request::Take(dumps::Take { id })),
        Ask::Screen { id } => R::Console(qsi::host::console::Request::Screen(
            qsi::host::console::Screen { id },
        )),
        Ask::Pools => R::Vres(vres::Request::Pools(Empty)),
        Ask::Vdisks { pool } => R::Vres(vres::Request::Vdisks(vres::Vdisks {
            pool: pool.unwrap_or_default(),
        })),
        Ask::Images => R::Vres(vres::Request::Images(Empty)),
        Ask::Volumes => R::Vres(vres::Request::Volumes(Empty)),
        Ask::Drives { id } => R::Vres(vres::Request::Drives(vres::Drives { id })),
        Ask::Switches => R::Vres(vres::Request::Switches(Empty)),
        Ask::Ports { switch } => R::Vres(vres::Request::Ports(vres::Ports {
            switch: switch.unwrap_or_default(),
        })),
        Ask::Pool { what } => R::Vres(pool_request(what)),
        Ask::Switch { what } => R::Vres(switch_request(what)),
    }
}

/// A refusal, every message of it.
#[must_use]
pub fn refused(reply: &Reply) -> String {
    qsictl::pair::refusal(reply)
}

/// Ask, and print the reply whole.
///
/// # Errors
/// The request was refused, or no reply came.
pub fn run(link: &mut Link, a: Ask) -> Result<(), String> {
    let reply = link.ask(&request(a))?;
    if reply.refusal().is_some() {
        return Err(refused(&reply));
    }
    match reply.decode() {
        Ok(Some(r)) => print(&r),
        Ok(None) => println!("(a reply this console does not read)"),
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

fn print(r: &AnyReply) {
    println!("{r:#?}");
}

#[cfg(test)]
mod tests {
    use super::units;

    #[test]
    fn units_are_read_as_hundredths() {
        assert_eq!(units("1"), Ok(100));
        assert_eq!(units("0.5"), Ok(50));
        assert_eq!(units("1.25"), Ok(125));
        assert!(units("1.255").is_err());
        assert!(units("x").is_err());
    }
}
