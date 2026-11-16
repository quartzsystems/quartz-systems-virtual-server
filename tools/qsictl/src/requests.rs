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
    /// Make a partition.
    Create {
        /// Its name.
        name: String,
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
        Ask::List => R::Partitions(partitions::Request::List(Empty)),
        Ask::Get { id } => R::Partitions(partitions::Request::Get(partitions::Get { id })),
        Ask::Adapters { id } => {
            R::Partitions(partitions::Request::Adapters(partitions::Adapters {
                id: id.unwrap_or(0),
            }))
        }
        Ask::Create { name } => {
            R::Partition(partition::Request::Create(partition::Create { name }))
        }
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
    }
}

/// A refusal, every message of it.
#[must_use]
pub fn refused(reply: &Reply) -> String {
    match &reply.outcome {
        qsisess::Outcome::Refused {
            msgid, text, also, ..
        } => {
            let mut lines = vec![format!("{msgid} {text}")];
            lines.extend(also.iter().map(|n| format!("{} {}", n.msgid, n.text)));
            lines.join("\n")
        }
        qsisess::Outcome::Result { .. } => "not refused".into(),
    }
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
