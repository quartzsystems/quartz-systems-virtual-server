// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The eight steps that write a load source: the disks' tables, their
//! protection, the data file system, both slots, the A/B record, the
//! firmware's boot entries and the first state, all of `base/loadsource`
//! for the product's names, from media already verified. Every step
//! starts from what the one before left; the first writes each disk's
//! table and clears the heads of its partitions, so an installation that
//! was cut off anywhere is simply started over. A product's [`Target`]
//! is then its names and its words, and [`Writer::step`].
//!
//! [`Target`]: crate::Target

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use abrec::Slot;
use appliance::LoadSourceNames;
use loadsource::boot::{self, EFIVARS};
use loadsource::disks::{self, LoadSource};
use loadsource::layout::SLOT_A_PATH;
use loadsource::payload::Payload;
use loadsource::record::{self, HostRecord};
use loadsource::set::{BLOCK, Member, MemberState, SetLabel};
use loadsource::{prepare, sys};
use rustix::mount::{MountFlags, UnmountFlags, mount, unmount};
use state::{Host, Store};
use uuid::Uuid;

use crate::Answers;

/// Where the new data file system is mounted while its state is written.
const TARGET: &str = "/run/target";

/// What the progress display calls each step, in order; the product
/// gives each its code.
pub const STEP_NAMES: [&str; 8] = [
    "Partitioning the disks",
    "Setting up protection",
    "Making the data file system",
    "Copying slot A",
    "Copying slot B",
    "Writing the A/B record",
    "Writing boot entries",
    "Writing state",
];

/// What is the product's in writing its load source.
#[derive(Clone, Copy, Debug)]
pub struct Product {
    /// Its load source's names.
    pub names: &'static LoadSourceNames,
    /// Its name, which the firmware's boot entries carry.
    pub name: &'static str,
    /// The code and text it logs when the writing begins.
    pub started: (&'static str, &'static str),
    /// The code and text it logs when the boot entries were not
    /// written (a machine whose firmware keeps none still starts).
    pub boot_not_written: (&'static str, &'static str),
}

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

/// A load source being written.
pub struct Writer {
    product: &'static Product,
    payload: Payload,
    /// The disks as load sources, once they are partitioned.
    sources: Vec<LoadSource>,
    log: Box<dyn FnMut(&str)>,
}

impl Writer {
    /// The writer of `product`'s load source from verified media.
    pub fn new(product: &'static Product, payload: Payload, log: Box<dyn FnMut(&str)>) -> Self {
        Self {
            product,
            payload,
            sources: Vec::new(),
            log,
        }
    }

    /// Say something for the installer's log; never a password.
    pub fn log(&mut self, line: &str) {
        (self.log)(line);
    }

    /// Carry out step `index` of [`STEP_NAMES`] with `answers`.
    ///
    /// # Errors
    /// The step failed: why, in a person's words.
    pub fn step(&mut self, index: usize, answers: &Answers) -> Result<(), String> {
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
                let done = self.copy(self.product.names.slot_b_path, &uki);
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

    fn partition(&mut self, answers: &Answers) -> Result<(), String> {
        self.sources.clear();
        let (code, said) = self.product.started;
        (self.log)(&format!("{code} {said}"));
        // An ESP with nothing in it: the slots are copied by their own steps.
        let esp = prepare::esp_image(self.product.names, &[], &[])?;
        for disk in &answers.disks {
            let node = Path::new("/dev").join(&disk.name);
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&node)
                .map_err(|e| format!("{}: {e}", node.display()))?;
            let guids = prepare::guids_from(&[random()?, random()?, random()?, random()?]);
            let bytes = disks::disk_bytes(&disk.name);
            prepare::write_table(
                self.product.names,
                &mut file,
                bytes,
                &esp,
                &guids,
                &mut |_, _| {},
            )
            .map_err(|e| format!("disk at {}: {e}", disk.location))?;
            file.sync_all().map_err(text)?;
            sys::reread_partitions(&file)
                .map_err(|e| format!("disk at {}: the table was not read: {e}", disk.location))?;
        }
        let give_up = Instant::now() + Duration::from_secs(15);
        for disk in &answers.disks {
            loop {
                if let Some(source) = disks::load_source(self.product.names, &disk.name) {
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
            written: u64::try_from(appliance::time::now_secs()).unwrap_or(0),
            members: vec![
                member(0, MemberState::Active),
                member(1, MemberState::Resuming),
            ],
        };
        disks::labels(self.product.names, &self.sources)
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
            self.product.names,
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
        let mut records = disks::records(self.product.names, &self.sources).map_err(text)?;
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
        let made = boot::make_default(
            Path::new(EFIVARS),
            self.product.names,
            self.product.name,
            &esps,
            Slot::A,
        );
        if let Err(e) = made {
            let (code, said) = self.product.boot_not_written;
            (self.log)(&format!("{code} {said}: {e}"));
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
