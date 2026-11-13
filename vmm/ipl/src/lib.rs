// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The IPL loader of a partition. A partition has no firmware: the
//! loader does what the firmware's boot entry and QS/400's EFI stub
//! would, from the host's side of the load-source disk.
//!
//! 1. The partition table: the ESP and the LIC partition by type.
//! 2. The LIC record: the side the partition asked for, taken once,
//!    and the keylock position and console mode written when the
//!    activation sets them.
//! 3. The side's UKI, read from the ESP's FAT volume.
//! 4. The kernel loaded, the page tables, GDT and zero page written.
//!
//! - [`gpt`], [`fat`]: bounded readers of the guest's disk.
//! - [`lic`]: the LIC record through QS/400's `licpart`.
//! - [`load`]: the bzImage into guest memory.
//! - [`part`]: a partition as a device.
//!
//! The disk is the guest's to write. A damaged or hostile one gives a
//! [`Fault`], which the VMM posts as a reference code; nothing here
//! panics, reads outside the disk or allocates by a size the disk
//! names beyond the fixed bounds.

#![forbid(unsafe_code)]

pub mod fat;
pub mod gpt;
pub mod lic;
pub mod load;
pub mod part;

use arch::layout::Ram;
use licpart::{Device, Slot};
use uuid::{Uuid, uuid};
use vm_memory::GuestMemoryMmap;

use fat::FatError;
use lic::{RecordError, Recorded, Settings};
use part::Part;

/// The GPT type of QS/400's LIC partition.
pub use licpart::LIC_TYPE_GUID as LIC_TYPE;

/// The GPT type of an EFI system partition.
pub const ESP_TYPE: Uuid = uuid!("c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
/// Side A's UKI on the guest's ESP: the removable-media path.
pub use diskimg::SLOT_A_UKI as SIDE_A_PATH;
/// Side B's UKI on the guest's ESP.
pub use diskimg::SLOT_B_UKI as SIDE_B_PATH;
/// The most bytes a UKI may have.
pub const MAX_IMAGE_BYTES: usize = 128 << 20;

/// The side asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Side A.
    A,
    /// Side B.
    B,
    /// The side the partition itself asked for, by its LIC record:
    /// the once-only slot when there is one, which is then cleared,
    /// else the default slot, else side A.
    Recorded,
}

/// A step of the IPL that was done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The partition table was read.
    TableRead,
    /// The LIC record was read.
    RecordRead(Recorded),
    /// The record was written: the keylock position, the console
    /// mode, or the once-only slot cleared.
    RecordWritten,
    /// The load source has no LIC record yet and none was needed.
    NoRecord,
    /// The side's UKI was read.
    ImageRead {
        /// The side.
        side: Slot,
        /// Its bytes.
        bytes: usize,
    },
    /// The kernel is loaded.
    Loaded,
}

/// Why a partition cannot IPL.
#[derive(Debug, thiserror::Error)]
pub enum Fault {
    /// The load source could not be read.
    #[error("the load source could not be read")]
    Io,
    /// The load source has no usable partition table.
    #[error("no partition table: {0}")]
    NoTable(gpt::GptError),
    /// The load source has no EFI system partition.
    #[error("no EFI system partition")]
    NoEsp,
    /// The ESP holds no volume that can be read.
    #[error("the EFI system partition: {0}")]
    BadEsp(FatError),
    /// The ESP has no UKI for the side.
    #[error("no UKI for side {0:?}")]
    NoImage(Slot),
    /// The side's UKI is not a file the loader takes.
    #[error("the UKI of side {0:?}: {1}")]
    BadImage(Slot, FatError),
    /// The load source has no LIC partition and the IPL needs its record.
    #[error("no LIC partition")]
    NoLic,
    /// The LIC record cannot be used.
    #[error("{0}")]
    BadRecord(RecordError),
    /// The UKI is not a kernel that can be started.
    #[error("{0}")]
    NotKernel(load::LoadError),
}

/// A partition ready to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipl {
    /// The side loaded.
    pub side: Slot,
    /// The address the boot processor starts at.
    pub entry: u64,
}

/// A side's letter.
pub fn side_letter(side: Slot) -> char {
    match side {
        Slot::A => 'A',
        Slot::B => 'B',
    }
}

fn side_path(side: Slot) -> &'static str {
    match side {
        Slot::A => SIDE_A_PATH,
        Slot::B => SIDE_B_PATH,
    }
}

/// Read the side's UKI and the LIC record from `disk`, a load source
/// of `disk_bytes`, without loading anything: the first three steps.
pub fn read<D: Device>(
    disk: &mut D,
    disk_bytes: u64,
    side: Side,
    settings: Settings,
    now: u64,
    step: &mut dyn FnMut(Step),
) -> Result<(Slot, Vec<u8>), Fault> {
    let table = gpt::read(disk, disk_bytes).map_err(|e| match e {
        gpt::GptError::Io => Fault::Io,
        e => Fault::NoTable(e),
    })?;
    step(Step::TableRead);
    let esp = table
        .iter()
        .find(|p| p.type_guid == ESP_TYPE)
        .ok_or(Fault::NoEsp)?;
    let lic = table.iter().find(|p| p.type_guid == licpart::LIC_TYPE_GUID);

    // The record is needed to know the side asked for and to hold the
    // settings; without either, a disk with no LIC partition loads.
    let take = side == Side::Recorded;
    let needed = take || settings.any();
    let recorded = match lic {
        Some(lic) => {
            let part = Part::new(&mut *disk, lic.offset(), lic.bytes());
            let recorded = lic::prepare(part, settings, take, now).map_err(Fault::BadRecord)?;
            if let Some(recorded) = recorded {
                step(Step::RecordRead(recorded));
                if recorded.written {
                    step(Step::RecordWritten);
                }
            }
            recorded
        }
        None if needed => return Err(Fault::NoLic),
        None => None,
    };
    if recorded.is_none() {
        step(Step::NoRecord);
    }
    let side = match side {
        Side::A => Slot::A,
        Side::B => Slot::B,
        // A disk that has not IPLed yet has asked for nothing: side A.
        Side::Recorded => recorded.map_or(Slot::A, |r| r.side),
    };

    let part = Part::new(&mut *disk, esp.offset(), esp.bytes());
    let mut volume = fat::Volume::open(part, esp.bytes()).map_err(|e| match e {
        FatError::Io => Fault::Io,
        e => Fault::BadEsp(e),
    })?;
    let image = volume
        .read_file(side_path(side), MAX_IMAGE_BYTES)
        .map_err(|e| match e {
            FatError::Io => Fault::Io,
            FatError::NotFound => Fault::NoImage(side),
            e => Fault::BadImage(side, e),
        })?;
    step(Step::ImageRead {
        side,
        bytes: image.len(),
    });
    Ok((side, image))
}

/// IPL a partition: read `disk` and load the side's kernel into `mem`.
#[allow(clippy::too_many_arguments)] // each is one thing the IPL is given
pub fn ipl<D: Device>(
    disk: &mut D,
    disk_bytes: u64,
    side: Side,
    settings: Settings,
    now: u64,
    mem: &GuestMemoryMmap,
    ram: &Ram,
    step: &mut dyn FnMut(Step),
) -> Result<Ipl, Fault> {
    let (side, image) = read(disk, disk_bytes, side, settings, now, step)?;
    let entry = load::load(mem, ram, &image).map_err(Fault::NotKernel)?;
    step(Step::Loaded);
    Ok(Ipl { side, entry })
}

/// A load source for the tests and the fuzzer's corpus: a table, an
/// ESP with the given files, a LIC partition, blank or with a first
/// record.
pub fn build_disk(files: &[(&str, &[u8])], with_record: bool) -> Vec<u8> {
    const SECTOR: usize = 512;
    let esp = fat::build(false, files);
    let esp_sectors = (esp.len() / SECTOR) as u64;
    let lic_sectors = (4 * licpart::SLOT_BYTES / SECTOR) as u64;
    let esp_first = 64;
    let lic_first = esp_first + esp_sectors;
    let sectors = lic_first + lic_sectors + 64;
    let mut disk = gpt::build(
        sectors,
        &[
            (ESP_TYPE, esp_first, lic_first - 1),
            (
                licpart::LIC_TYPE_GUID,
                lic_first,
                lic_first + lic_sectors - 1,
            ),
        ],
    );
    disk.resize(usize::try_from(sectors).unwrap_or(0) * SECTOR, 0);
    let at = usize::try_from(esp_first).unwrap_or(0) * SECTOR;
    disk[at..at + esp.len()].copy_from_slice(&esp);
    if with_record {
        let blank = std::io::Cursor::new(vec![0u8; 4 * licpart::SLOT_BYTES]);
        if let Ok((part, _)) = licpart::LicPart::init(blank, 1_700_000_000) {
            let bytes = part.into_device().into_inner();
            let at = usize::try_from(lic_first).unwrap_or(0) * SECTOR;
            disk[at..at + bytes.len()].copy_from_slice(&bytes);
        }
    }
    disk
}

#[cfg(test)]
mod tests {
    use super::*;
    use lic::{ConsoleMode, Keylock};
    use std::io::Cursor;
    use vm_memory::GuestAddress;

    fn memory() -> (GuestMemoryMmap, Ram) {
        let ram = Ram::for_bytes(128 << 20).unwrap();
        (
            GuestMemoryMmap::from_ranges(&[(GuestAddress(0), 128 << 20)]).unwrap(),
            ram,
        )
    }

    fn sides() -> (Vec<u8>, Vec<u8>) {
        (load::build(5000), load::build(9000))
    }

    fn run(
        disk: &mut Cursor<Vec<u8>>,
        side: Side,
        settings: Settings,
    ) -> (Result<Ipl, Fault>, Vec<Step>) {
        let (mem, ram) = memory();
        let bytes = disk.get_ref().len() as u64;
        let mut steps = Vec::new();
        let result = ipl(disk, bytes, side, settings, 0, &mem, &ram, &mut |s| {
            steps.push(s);
        });
        (result, steps)
    }

    #[test]
    fn each_side_loads_its_own_uki_with_the_steps_in_order() {
        let (a, b) = sides();
        let image = build_disk(&[(SIDE_A_PATH, &a), (SIDE_B_PATH, &b)], true);
        let mut disk = Cursor::new(image);
        let (result, steps) = run(&mut disk, Side::B, Settings::default());
        assert_eq!(result.unwrap().side, Slot::B);
        assert!(matches!(steps[0], Step::TableRead));
        assert!(matches!(steps[1], Step::RecordRead(r) if r.side == Slot::A));
        assert_eq!(
            steps[2],
            Step::ImageRead {
                side: Slot::B,
                bytes: b.len()
            }
        );
        assert_eq!(steps[3], Step::Loaded);
        let (result, steps) = run(&mut disk, Side::Recorded, Settings::default());
        assert_eq!(result.unwrap().side, Slot::A);
        assert_eq!(
            steps[2],
            Step::ImageRead {
                side: Slot::A,
                bytes: a.len()
            }
        );
    }

    #[test]
    fn the_settings_are_in_the_record_before_the_kernel_runs() {
        let (a, _) = sides();
        let mut disk = Cursor::new(build_disk(&[(SIDE_A_PATH, &a)], true));
        let settings = Settings {
            keylock: Some(Keylock::Manual),
            console: Some(ConsoleMode::Serial),
        };
        let (result, steps) = run(&mut disk, Side::A, settings);
        assert!(result.is_ok());
        assert!(steps.contains(&Step::RecordWritten));
        let (_, steps) = run(&mut disk, Side::A, Settings::default());
        assert!(matches!(
            steps[1],
            Step::RecordRead(r) if r.keylock == Keylock::Manual && r.console == ConsoleMode::Serial
        ));
    }

    #[test]
    fn each_thing_missing_has_its_own_fault() {
        let (a, _) = sides();
        let good = build_disk(&[(SIDE_A_PATH, &a)], true);
        let fault = |image: Vec<u8>, side: Side| {
            run(&mut Cursor::new(image), side, Settings::default())
                .0
                .unwrap_err()
        };

        assert!(matches!(
            fault(vec![0u8; 1 << 20], Side::A),
            Fault::NoTable(_)
        ));
        assert!(matches!(fault(good[..600].to_vec(), Side::A), Fault::Io));
        assert!(matches!(
            fault(good.clone(), Side::B),
            Fault::NoImage(Slot::B)
        ));

        // The ESP's type changed to something else: no ESP.
        let mut no_esp = gpt::build(4096, &[(licpart::LIC_TYPE_GUID, 64, 1087)]);
        no_esp.resize(4096 * 512, 0);
        assert!(matches!(fault(no_esp, Side::A), Fault::NoEsp));

        // The volume overwritten.
        let mut bad_esp = good.clone();
        bad_esp[64 * 512..64 * 512 + 512].fill(0xEE);
        assert!(matches!(fault(bad_esp, Side::A), Fault::BadEsp(_)));

        // The record damaged.
        let lic_at = good.len() - (64 + 4 * licpart::SLOT_BYTES / 512) * 512;
        let mut bad_record = good.clone();
        bad_record[lic_at + 40] ^= 0xFF;
        assert!(matches!(fault(bad_record, Side::A), Fault::BadRecord(_)));

        // A file that is not a kernel.
        let not_kernel = build_disk(&[(SIDE_A_PATH, &[0x4D; 5000])], true);
        assert!(matches!(fault(not_kernel, Side::A), Fault::NotKernel(_)));

        // The disk cut short inside the ESP.
        let truncated = good[..64 * 512 + 30_000].to_vec();
        let bytes = truncated.len() as u64;
        let (mem, ram) = memory();
        let mut disk = Cursor::new(truncated);
        let cut = ipl(
            &mut disk,
            bytes,
            Side::A,
            Settings::default(),
            0,
            &mem,
            &ram,
            &mut |_| {},
        );
        assert!(cut.is_err());
    }

    #[test]
    fn a_disk_without_a_lic_partition_loads_when_its_record_is_not_needed() {
        let (a, _) = sides();
        let esp = fat::build(false, &[(SIDE_A_PATH, &a)]);
        let esp_sectors = (esp.len() / 512) as u64;
        let mut image = gpt::build(esp_sectors + 128, &[(ESP_TYPE, 64, 64 + esp_sectors - 1)]);
        image.resize((esp_sectors as usize + 128) * 512, 0);
        image[64 * 512..64 * 512 + esp.len()].copy_from_slice(&esp);
        let (result, steps) = run(
            &mut Cursor::new(image.clone()),
            Side::A,
            Settings::default(),
        );
        assert_eq!(result.unwrap().side, Slot::A);
        assert!(steps.contains(&Step::NoRecord));
        let (result, _) = run(
            &mut Cursor::new(image.clone()),
            Side::Recorded,
            Settings::default(),
        );
        assert!(matches!(result, Err(Fault::NoLic)));
        let settings = Settings {
            keylock: Some(Keylock::Normal),
            console: None,
        };
        let (result, _) = run(&mut Cursor::new(image), Side::A, settings);
        assert!(matches!(result, Err(Fault::NoLic)));
    }

    /// The guest's own request, as `slic` writes it in a partition.
    fn ask(disk: &mut Cursor<Vec<u8>>, source: licpart::IplSource) {
        let at = disk.get_ref().len() - (64 + 4 * licpart::SLOT_BYTES / 512) * 512;
        let bytes = (4 * licpart::SLOT_BYTES) as u64;
        let mut part = licpart::LicPart::open(Part::new(&mut *disk, at as u64, bytes));
        let mut record = part.read().unwrap();
        source.store(&mut record).unwrap();
        part.write(&mut record).unwrap();
    }

    fn asked(disk: &mut Cursor<Vec<u8>>) -> licpart::IplSource {
        let at = disk.get_ref().len() - (64 + 4 * licpart::SLOT_BYTES / 512) * 512;
        let bytes = (4 * licpart::SLOT_BYTES) as u64;
        let mut part = licpart::LicPart::open(Part::new(&mut *disk, at as u64, bytes));
        licpart::IplSource::of(&part.read().unwrap())
    }

    #[test]
    fn the_recorded_side_is_what_the_partition_asked_for() {
        let (a, b) = sides();
        let mut disk = Cursor::new(build_disk(&[(SIDE_A_PATH, &a), (SIDE_B_PATH, &b)], true));
        let side = |disk: &mut Cursor<Vec<u8>>, side| {
            let (result, steps) = run(disk, side, Settings::default());
            (result.unwrap().side, steps.contains(&Step::RecordWritten))
        };
        // Nothing asked: side A, and the record is not written.
        assert_eq!(side(&mut disk, Side::Recorded), (Slot::A, false));

        // Side B for one IPL: taken, cleared, and the next is side A.
        let once = licpart::IplSource {
            next: Some(Slot::B),
            default: None,
        };
        ask(&mut disk, once);
        assert_eq!(side(&mut disk, Side::Recorded), (Slot::B, true));
        assert_eq!(asked(&mut disk), licpart::IplSource::default());
        assert_eq!(side(&mut disk, Side::Recorded), (Slot::A, false));

        // Side B from now on: every IPL, and the record stays.
        let default = licpart::IplSource {
            next: None,
            default: Some(Slot::B),
        };
        ask(&mut disk, default);
        assert_eq!(side(&mut disk, Side::Recorded), (Slot::B, false));
        assert_eq!(side(&mut disk, Side::Recorded), (Slot::B, false));
        assert_eq!(asked(&mut disk), default);
    }

    #[test]
    fn an_explicit_side_overrides_and_leaves_the_request() {
        let (a, b) = sides();
        let mut disk = Cursor::new(build_disk(&[(SIDE_A_PATH, &a), (SIDE_B_PATH, &b)], true));
        let once = licpart::IplSource {
            next: Some(Slot::B),
            default: Some(Slot::B),
        };
        ask(&mut disk, once);
        let (result, steps) = run(&mut disk, Side::A, Settings::default());
        assert_eq!(result.unwrap().side, Slot::A);
        assert!(!steps.contains(&Step::RecordWritten));
        assert_eq!(asked(&mut disk), once);
        // The request is still there for the activation that takes it.
        let (result, steps) = run(&mut disk, Side::Recorded, Settings::default());
        assert_eq!(result.unwrap().side, Slot::B);
        assert!(matches!(steps[1], Step::RecordRead(r) if r.once && r.side == Slot::B));
        assert_eq!(asked(&mut disk).next, None);
    }

    #[test]
    fn a_blank_lic_partition_starts_side_a_and_is_left_blank() {
        let (a, _) = sides();
        let image = build_disk(&[(SIDE_A_PATH, &a)], false);
        let before = image.clone();
        let mut disk = Cursor::new(image);
        let (result, steps) = run(&mut disk, Side::Recorded, Settings::default());
        assert_eq!(result.unwrap().side, Slot::A);
        assert!(steps.contains(&Step::NoRecord));
        assert_eq!(disk.into_inner(), before);
    }
}
