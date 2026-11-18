// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition's optical and tape drives as the host works with them
//! while the partition runs.
//!
//! Their requests come on the control socket (`crate::control`):
//!
//! ```text
//! load <slot> <name> <capacity MiB> <protected>   the file comes with it
//! unload <slot> <forced>
//! media
//! ```
//!
//! The answer is a line: `ok <name>` or `refused <why>`; for `media`,
//! a line a drive, `<slot> <optical|tape> <name or -> <prevented>`, and
//! then `end`.
//!
//! What a guest does by itself, an eject or an unload, is reported on
//! standard error as `MEDIA <slot> unloaded <name>`, which the
//! partition manager reads beside the reference codes.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use devices::virtio::scsi::Unit;
use devices::virtio::scsi::backing::ImageFile;
use devices::virtio::scsi::optical::{self, Optical};
use devices::virtio::scsi::passed::PassedTape;
use devices::virtio::scsi::tape::{self, Tape};

use crate::config::Partition;

/// The control socket's file in the partition's socket directory.
pub const CONTROL_SOCKET: &str = "control.sock";
/// How often the drives are asked what the guest did.
const LOOK: Duration = Duration::from_millis(250);

/// The drives, by slot.
#[derive(Clone, Default)]
pub struct Drives {
    /// The optical drives.
    pub optical: Vec<(u32, Arc<Optical>)>,
    /// The tape drives.
    pub tape: Vec<(u32, Arc<Tape>)>,
}

fn other(what: String) -> io::Error {
    io::Error::other(what)
}

/// Units by slot.
pub type SlotUnits = Vec<(u32, Arc<dyn Unit>)>;

/// The drives of `partition`, with what is to be loaded at the start,
/// and each as a unit of its own function, by slot.
pub fn open(partition: &Partition) -> io::Result<(Drives, SlotUnits)> {
    let mut drives = Drives::default();
    let mut units: Vec<(u32, Arc<dyn Unit>)> = Vec::new();
    for o in &partition.optical {
        let drive = Arc::new(
            Optical::new(&o.serial).ok_or_else(|| other(format!("optical slot {}", o.slot)))?,
        );
        if let Some(path) = &o.media {
            let image = ImageFile::new(File::open(path)?)?;
            drive
                .load(Box::new(image), &o.name)
                .map_err(|e| other(format!("optical slot {}: {e}", o.slot)))?;
        }
        units.push((o.slot, drive.clone()));
        drives.optical.push((o.slot, drive));
    }
    for t in &partition.tape {
        if let Some(path) = &t.drive {
            let fd = crate::net::descriptor(path)
                .ok_or_else(|| other(format!("tape slot {}: no descriptor", t.slot)))?;
            let link = crate::sys::inherited_stream(fd)
                .map_err(|e| other(format!("tape slot {}: {e}", t.slot)))?;
            units.push((t.slot, Arc::new(PassedTape::new(Box::new(link)))));
            continue;
        }
        let drive =
            Arc::new(Tape::new(&t.serial).ok_or_else(|| other(format!("tape slot {}", t.slot)))?);
        if let Some(path) = &t.volume {
            let file = OpenOptions::new()
                .read(true)
                .write(!t.protected)
                .open(path)?;
            drive
                .mount(Box::new(file), &t.name, t.capacity_mib << 20, t.protected)
                .map_err(|e| other(format!("tape slot {}: {e}", t.slot)))?;
        }
        units.push((t.slot, drive.clone()));
        drives.tape.push((t.slot, drive));
    }
    Ok((drives, units))
}

/// The first optical drive's medium, for IPL source D: its file and
/// its bytes.
pub fn first_medium(partition: &Partition) -> io::Result<Option<(File, u64)>> {
    let Some(path) = partition.optical.first().and_then(|o| o.media.as_ref()) else {
        return Ok(None);
    };
    let file = File::open(path)?;
    let bytes = file.metadata()?.len();
    Ok(Some((file, bytes)))
}

/// The control socket, made as root: root's, and nobody else's.
pub fn bind(dir: &Path) -> io::Result<UnixListener> {
    let path = dir.join(CONTROL_SOCKET);
    let _ = fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

impl Drives {
    /// Carry out one of the drives' requests.
    pub fn carry_out(&self, line: &str, file: Option<File>) -> String {
        let words: Vec<&str> = line.split_whitespace().collect();
        let slot = |w: Option<&&str>| w.and_then(|s| s.parse::<u32>().ok());
        let refused = |why: &str| format!("refused {why}");
        match words.first().copied() {
            Some("load") => {
                let (Some(slot), Some(name), Some(cap), Some(prot)) = (
                    slot(words.get(1)),
                    words.get(2),
                    words.get(3).and_then(|c| c.parse::<u64>().ok()),
                    words.get(4).and_then(|p| p.parse::<bool>().ok()),
                ) else {
                    return refused("load <slot> <name> <capacity> <protected>");
                };
                let Some(file) = file else {
                    return refused("no file came with the request");
                };
                if let Some((_, d)) = self.optical.iter().find(|(s, _)| *s == slot) {
                    return match ImageFile::new(file) {
                        Ok(image) => match d.load(Box::new(image), name) {
                            Ok(()) => format!("ok {name}"),
                            Err(e) => refused(e),
                        },
                        Err(e) => refused(&e.to_string()),
                    };
                }
                if let Some((_, d)) = self.tape.iter().find(|(s, _)| *s == slot) {
                    if d.mounted().is_some() {
                        return refused("a volume is mounted");
                    }
                    return match d.mount(Box::new(file), name, cap << 20, prot) {
                        Ok(()) => format!("ok {name}"),
                        Err(e) => refused(&e.to_string()),
                    };
                }
                refused("no optical or tape device in that slot")
            }
            Some("unload") => {
                let (Some(slot), forced) = (slot(words.get(1)), words.get(2) == Some(&"true"))
                else {
                    return refused("unload <slot> <forced>");
                };
                if let Some((_, d)) = self.optical.iter().find(|(s, _)| *s == slot) {
                    return match d.unload(forced) {
                        Ok(name) => format!("ok {name}"),
                        Err(optical::UnloadError::Prevented) => refused("prevented"),
                        Err(e) => refused(&e.to_string()),
                    };
                }
                if let Some((_, d)) = self.tape.iter().find(|(s, _)| *s == slot) {
                    return match d.unmount(forced) {
                        Ok(name) => format!("ok {name}"),
                        Err(tape::UnmountError::Prevented) => refused("prevented"),
                        Err(e) => refused(&e.to_string()),
                    };
                }
                refused("no optical or tape device in that slot")
            }
            Some("media") => {
                let mut out = String::new();
                for (slot, d) in &self.optical {
                    let name = d.loaded().unwrap_or_else(|| "-".into());
                    let _ = writeln!(out, "{slot} optical {name} {}", d.prevented());
                }
                for (slot, d) in &self.tape {
                    let name = d.mounted().unwrap_or_else(|| "-".into());
                    let _ = writeln!(out, "{slot} tape {name} {}", d.prevented());
                }
                out.push_str("end");
                out
            }
            _ => refused("load, unload, media, hold, kept or dump"),
        }
    }

    /// Make every tape volume's writes durable: at the partition's end.
    pub fn sync(&self) {
        for (slot, d) in &self.tape {
            if !d.sync() {
                eprintln!("qsvmm: tape slot {slot}: the volume's writes were not made durable");
            }
        }
    }

    /// Report what the guest did by itself.
    fn look(&self) {
        for (slot, d) in &self.optical {
            for optical::Notice::Ejected(name) in d.notices() {
                eprintln!("MEDIA {slot} unloaded {name}");
            }
        }
        for (slot, d) in &self.tape {
            for tape::Notice::Unloaded(name) in d.notices() {
                eprintln!("MEDIA {slot} unloaded {name}");
            }
        }
    }
}

/// Watch the drives for what the guest does, on a thread.
pub fn spawn(drives: Drives) -> io::Result<()> {
    if drives.optical.is_empty() && drives.tape.is_empty() {
        return Ok(());
    }
    thread::Builder::new()
        .name("drives".into())
        .spawn(move || {
            crate::confine::enter_or_end(crate::confine::Kind::Drives);
            loop {
                drives.look();
                thread::sleep(LOOK);
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn iso(blocks: usize) -> File {
        let path =
            std::env::temp_dir().join(format!("qsvmm-media-{}-{blocks}", std::process::id()));
        let mut f = File::create(&path).unwrap();
        f.write_all(&vec![0u8; blocks * 2048]).unwrap();
        File::open(&path).unwrap()
    }

    #[test]
    fn requests_load_unload_and_list_the_drives() {
        let drives = Drives {
            optical: vec![(3, Arc::new(Optical::new("OPT").unwrap()))],
            tape: vec![(4, Arc::new(Tape::new("TAP").unwrap()))],
        };
        assert_eq!(
            drives.carry_out("load 3 QS400 0 false", Some(iso(8))),
            "ok QS400"
        );
        assert_eq!(
            drives.carry_out("media", None),
            "3 optical QS400 false\n4 tape - false\nend"
        );
        assert!(
            drives
                .carry_out("load 3 X 0 false", None)
                .starts_with("refused")
        );
        assert!(
            drives
                .carry_out("load 9 X 0 false", Some(iso(8)))
                .contains("no optical")
        );
        assert_eq!(drives.carry_out("unload 3 false", None), "ok QS400");
        assert!(
            drives
                .carry_out("unload 3 false", None)
                .starts_with("refused")
        );
        assert!(drives.carry_out("eject", None).starts_with("refused"));
        assert!(drives.carry_out("", None).starts_with("refused"));
    }
}
