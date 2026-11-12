// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS as the base installer sees it: the machine's disks and links
//! from the inventory, and the steps that write a load source. Every
//! step starts from what the one before left; the first writes each
//! disk's table and clears the heads of its partitions, so an
//! installation that was cut off anywhere is simply started over.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use abrec::Slot;
use hostdisk::boot::{self, EFIVARS};
use hostdisk::disks::{self, LoadSource};
use hostdisk::layout::{SLOT_A_PATH, SLOT_B_PATH};
use hostdisk::record::{self, HostRecord};
use hostdisk::set::{BLOCK, Member, MemberState, SetLabel};
use hostdisk::{prepare, sys};
use install::{Answers, DiskChoice, Step, Target};
use panels::forms::LinkChoice;
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use state::{Host, Store};
use uuid::Uuid;

use crate::media::Payload;

/// Where the new data file system is mounted while its state is written.
const TARGET: &str = "/run/target";

/// The steps, each with the code posted when it is done.
const STEPS: &[Step] = &[
    Step {
        name: "Partitioning the disks",
        code: refcode::INSTALL_PARTITIONED.code,
    },
    Step {
        name: "Setting up protection",
        code: refcode::INSTALL_PROTECTED.code,
    },
    Step {
        name: "Making the data file system",
        code: refcode::INSTALL_FORMATTED.code,
    },
    Step {
        name: "Copying slot A",
        code: refcode::INSTALL_SLOT_A.code,
    },
    Step {
        name: "Copying slot B",
        code: refcode::INSTALL_SLOT_B.code,
    },
    Step {
        name: "Writing the A/B record",
        code: refcode::INSTALL_RECORD.code,
    },
    Step {
        name: "Writing boot entries",
        code: refcode::INSTALL_BOOT.code,
    },
    Step {
        name: "Writing state",
        code: refcode::INSTALL_STATE.code,
    },
];

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Sixteen random bytes from the kernel.
fn random() -> Result<[u8; 16], String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| format!("/dev/urandom: {e}"))?;
    Ok(bytes)
}

/// QS/VS, being installed.
pub struct QsVs {
    payload: Payload,
    /// Disks that are not to be offered: the media and the seed volume.
    skip: Vec<String>,
    /// The disks as load sources, once they are partitioned.
    sources: Vec<LoadSource>,
    log: Box<dyn FnMut(&str)>,
}

impl QsVs {
    /// The installer over verified media. `skip` names the disks that
    /// hold the media and the answers.
    pub fn new(payload: Payload, skip: Vec<String>, log: Box<dyn FnMut(&str)>) -> Self {
        Self {
            payload,
            skip,
            sources: Vec::new(),
            log,
        }
    }

    /// Bring every Ethernet link up, so that carrier can be shown.
    pub fn links_up() {
        let Ok(mut rtnl) = netlink::route::Rtnl::open() else {
            return;
        };
        for link in appliance::links::ethernet() {
            let _ = rtnl.set_up(link.index, true);
        }
    }

    fn partition(&mut self, answers: &Answers) -> Result<(), String> {
        self.sources.clear();
        (self.log)(&format!(
            "{} {}",
            refcode::INSTALL_STARTED.code,
            refcode::INSTALL_STARTED.text
        ));
        // An ESP with nothing in it: the slots are copied by their own steps.
        let esp = prepare::esp_image(&[], &[])?;
        for disk in &answers.disks {
            let node = Path::new("/dev").join(&disk.name);
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&node)
                .map_err(|e| format!("{}: {e}", node.display()))?;
            let guids = prepare::guids_from(&[random()?, random()?, random()?, random()?]);
            let bytes = disks::disk_bytes(&disk.name);
            prepare::write_table(&mut file, bytes, &esp, &guids, &mut |_, _| {})
                .map_err(|e| format!("disk at {}: {e}", disk.location))?;
            file.sync_all().map_err(text)?;
            sys::reread_partitions(&file)
                .map_err(|e| format!("disk at {}: the table was not read: {e}", disk.location))?;
        }
        let give_up = Instant::now() + Duration::from_secs(15);
        for disk in &answers.disks {
            loop {
                if let Some(source) = disks::load_source(&disk.name) {
                    self.sources.push(source);
                    break;
                }
                if Instant::now() > give_up {
                    return Err(format!(
                        "the partitions of the disk at {} did not appear",
                        disk.location
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        Ok(())
    }

    /// The bytes of the data file system: the data partition, or the
    /// smaller of the two.
    fn data_bytes(&self) -> u64 {
        let least = self.sources.iter().map(|s| s.data_bytes).min().unwrap_or(0);
        least / BLOCK * BLOCK
    }

    fn protect(&mut self, answers: &Answers) -> Result<(), String> {
        if !answers.mirrored() {
            return Ok(());
        }
        let member = |i: usize, state| Member {
            partition: self.sources[i].data_guid,
            state,
            serial: answers.disks[i].serial.clone(),
            location: answers.disks[i].location.clone(),
        };
        // The second unit is written by the host's first IPL, from the
        // first: nothing says the two disks hold the same bytes now.
        let label = SetLabel {
            id: Uuid::from_bytes(random()?),
            blocks: self.data_bytes() / BLOCK,
            clean: true,
            members: vec![
                member(0, MemberState::Active),
                member(1, MemberState::Resuming),
            ],
        };
        disks::labels(&self.sources)
            .map_err(text)?
            .write(&label.encode())
            .map_err(text)?;
        Ok(())
    }

    fn format(&mut self, now: i64) -> Result<(), String> {
        let first = self.sources.first().ok_or("no disk is partitioned")?;
        let mut data = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&first.data)
            .map_err(|e| format!("{}: {e}", first.data.display()))?;
        prepare::format_data(
            &mut data,
            self.data_bytes(),
            Uuid::from_bytes(random()?),
            now,
        )?;
        data.sync_all().map_err(text)
    }

    /// Write a slot's UKI to every disk's ESP and read it back.
    fn copy(&mut self, path: &str, uki: &[u8]) -> Result<(), String> {
        for source in &self.sources {
            let open = || {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&source.esp)
                    .map_err(|e| format!("{}: {e}", source.esp.display()))
            };
            let esp = open()?;
            diskimg::fat::write_file(&esp, path, uki)
                .map_err(|e| format!("{}: {e}", source.esp.display()))?;
            esp.sync_all().map_err(text)?;
            let back = diskimg::fat::read_file_at(open()?, path)
                .map_err(|e| format!("{}: {e}", source.esp.display()))?;
            if back != uki {
                return Err(format!(
                    "{path} on {} does not read back as it was written",
                    source.esp.display()
                ));
            }
        }
        Ok(())
    }

    fn record(&mut self) -> Result<(), String> {
        let mut records = disks::records(&self.sources).map_err(text)?;
        let (_, written) = record::write_all(&mut records, HostRecord::INITIAL).map_err(text)?;
        if written != self.sources.len() {
            return Err(format!(
                "the record was written to {written} of {} disks",
                self.sources.len()
            ));
        }
        Ok(())
    }

    /// A machine whose firmware keeps no entries still starts the
    /// disks by their default path, so this does not fail the step.
    fn boot_entries(&mut self) {
        let esps: Vec<_> = self.sources.iter().map(|s| s.esp_at).collect();
        if let Err(e) = boot::make_default(Path::new(EFIVARS), release::NAME, &esps, Slot::A) {
            (self.log)(&format!(
                "{} {}: {e}",
                refcode::INSTALL_BOOT_NOT_WRITTEN.code,
                refcode::INSTALL_BOOT_NOT_WRITTEN.text
            ));
        }
    }

    fn state(&mut self, answers: &Answers) -> Result<(), String> {
        if let Some(clock) = answers.time.clock {
            set_clock(clock)?;
        }
        let first = self.sources.first().ok_or("no disk is partitioned")?;
        fs::create_dir_all(TARGET).map_err(|e| format!("{TARGET}: {e}"))?;
        let flags = MountFlags::NOSUID | MountFlags::NODEV | MountFlags::NOEXEC;
        mount(&first.data, TARGET, "xfs", flags, None)
            .map_err(|e| format!("{}: {e}", first.data.display()))?;
        let written = (|| {
            let dir = PathBuf::from(TARGET).join("state");
            fs::create_dir_all(&dir).map_err(text)?;
            let store = Store::at(&dir);
            store.save(&answers.network).map_err(text)?;
            let now = u64::try_from(appliance::time::now_secs()).unwrap_or(0);
            let users = users::first(&answers.user_id, &answers.password_hash, now)?;
            state::atomic_write(&dir.join(users::FILE), &users).map_err(text)?;
            // The host's file is last: it is what says the host is configured.
            store
                .save(&Host {
                    name: Some(answers.host.clone()),
                    time_zone: answers.time.zone.clone(),
                    ntp_servers: answers.time.servers.clone(),
                })
                .map_err(text)
        })();
        rustix::fs::sync();
        let unmounted =
            unmount(TARGET, UnmountFlags::empty()).map_err(|e| format!("{TARGET}: {e}"));
        written.and(unmounted)
    }
}

/// Set the system clock and the RTC.
fn set_clock(secs: i64) -> Result<(), String> {
    use rustix::time::{ClockId, Timespec, clock_settime};
    clock_settime(
        ClockId::Realtime,
        Timespec {
            tv_sec: secs,
            tv_nsec: 0,
        },
    )
    .map_err(|e| format!("the clock was not set: {e}"))?;
    // A machine without an RTC keeps the time until it powers off.
    let _ = netd::sys::clock::write_rtc();
    Ok(())
}

impl Target for QsVs {
    fn product(&self) -> &'static str {
        release::NAME
    }

    fn banner(&self) -> String {
        format!("{} ({})", release::banner(), release::PRODUCT)
    }

    fn what(&self) -> &'static [&'static str] {
        &[
            "This installs the hypervisor on this machine: it erases the disks",
            "you choose, makes them the load source the host starts from, and",
            "sets the host name, the management network, the time and the first",
            "service tools user. Partitions are then made and run from the",
            "management console, QS/MC.",
        ]
    }

    fn disks(&mut self) -> Vec<DiskChoice> {
        hostinv::read(&hostinv::Roots::default(), None)
            .disks
            .into_iter()
            .filter(|d| !self.skip.contains(&d.name) && !d.name.starts_with("sr"))
            .map(|d| DiskChoice {
                name: d.name,
                location: d.location,
                model: d.model,
                serial: d.serial,
                bytes: d.bytes,
            })
            .collect()
    }

    fn min_disk_bytes(&self) -> u64 {
        hostdisk::layout::min_disk_bytes()
    }

    fn links(&mut self) -> Vec<LinkChoice> {
        hostinv::read(&hostinv::Roots::default(), None)
            .interfaces
            .into_iter()
            .map(|i| LinkChoice {
                name: i.name,
                mac: i.mac,
                location: i.location,
                carrier: i.carrier,
            })
            .collect()
    }

    fn default_host(&self) -> String {
        let first = appliance::links::ethernet().first().map(|l| l.mac);
        Host::default_name(release::HOST_PREFIX, first)
    }

    fn now(&self) -> i64 {
        appliance::time::now_secs()
    }

    fn steps(&self) -> &'static [Step] {
        STEPS
    }

    fn step(&mut self, index: usize, answers: &Answers) -> Result<(), String> {
        match index {
            0 => self.partition(answers),
            1 => self.protect(answers),
            2 => self.format(appliance::time::now_secs()),
            3 => {
                let uki = std::mem::take(&mut self.payload.slot_a);
                let done = self.copy(SLOT_A_PATH, &uki);
                self.payload.slot_a = uki;
                done
            }
            4 => {
                let uki = std::mem::take(&mut self.payload.slot_b);
                let done = self.copy(SLOT_B_PATH, &uki);
                self.payload.slot_b = uki;
                done
            }
            5 => self.record(),
            6 => {
                self.boot_entries();
                Ok(())
            }
            7 => self.state(answers),
            _ => Err(format!("there is no step {index}")),
        }
    }

    fn failed(&self) -> &'static str {
        refcode::INSTALL_FAILED.code
    }

    fn log(&mut self, line: &str) {
        (self.log)(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_are_the_eight_of_the_progress_display_each_with_its_code() {
        assert_eq!(STEPS.len(), 8);
        for (i, step) in STEPS.iter().enumerate() {
            assert!(step.code.starts_with("C2004"), "{}", step.code);
            assert!(refcode::find(step.code).is_some());
            assert!(STEPS[..i].iter().all(|s| s.code < step.code));
        }
        assert_eq!(STEPS[0].name, "Partitioning the disks");
        assert_eq!(STEPS[7].name, "Writing state");
    }
}
