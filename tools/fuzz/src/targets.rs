// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The targets: every place bytes a guest controls are read.
//!
//! | Target | What a guest controls |
//! |---|---|
//! | `gpt` | the load source's partition table |
//! | `fat` | the ESP's FAT volume |
//! | `lic` | the LIC partition's record |
//! | `disk` | a whole load source: table, ESP and record together |
//! | `bzimage` | the UKI the loader is given |
//! | `scsi` | a SCSI command and its data |
//! | `virtqueue` | guest memory with the virtio-scsi queues and the virtio-rng queue in it |
//! | `devices` | reads and writes of the I/O ports, PCI configuration space and the virtio BARs |

use std::fs::File;
use std::io::{self, Cursor};
use std::sync::{Arc, Mutex, OnceLock};

use arch::layout::{
    CMOS_PORT, ECAM_START, I8042_PORT, PCI_CONFIG_PORT, PCI_MEM_START, Ram, SERIAL_PORT, SLEEP_PORT,
};
use devices::Memory;
use devices::bus::Bus;
use devices::legacy::cmos::Cmos;
use devices::legacy::power::Power;
use devices::legacy::reset::Reset;
use devices::legacy::serial::Serial;
use devices::pci::PciBus;
use devices::platform::{MsiRoute, Platform, Requests};
use devices::virtio::scsi::backing::Bounce;
use devices::virtio::scsi::backing::memory::MemoryStore;
use devices::virtio::scsi::lun::Lun;
use devices::virtio::scsi::{cdb, serve_control, serve_requests};
use devices::virtio::{Activation, VirtioDevice, VirtioPci, rng};
use ipl::lic::{ConsoleMode, Keylock, Settings};
use ipl::{SIDE_A_PATH, SIDE_B_PATH, Side};
use virtio_queue::{Queue, QueueT};
use vm_memory::{Bytes, GuestAddress};
use vmm_sys_util::eventfd::EventFd;

use crate::engine::Target;

fn kernel() -> Vec<u8> {
    ipl::load::build(3000)
}

fn disk_seed() -> Vec<u8> {
    let k = kernel();
    ipl::build_disk(&[(SIDE_A_PATH, &k), (SIDE_B_PATH, &k)], true)
}

fn gpt_seeds() -> Vec<Vec<u8>> {
    let mut disk = disk_seed();
    disk.truncate(34 * 512);
    vec![disk]
}

fn gpt(data: &[u8]) {
    // The disk is said to be larger than the bytes, as a real one is.
    let mut sealed = data.to_vec();
    ipl::gpt::seal(&mut sealed);
    for (table, disk_bytes) in [
        (data, data.len() as u64),
        (data, 1 << 40),
        (&sealed[..], 1 << 40),
    ] {
        if let Ok(parts) = ipl::gpt::read(&mut Cursor::new(table), disk_bytes) {
            for part in parts {
                assert!(part.first_lba <= part.last_lba);
                assert!(part.offset().checked_add(part.bytes()).is_some());
            }
        }
    }
}

fn fat_seeds() -> Vec<Vec<u8>> {
    let k = kernel();
    let mut small = ipl::fat::build(false, &[(SIDE_A_PATH, &k), (SIDE_B_PATH, &k)]);
    // The FAT, the root directory and the first clusters: the rest of
    // the volume is zeros a mutation need not carry.
    small.truncate(64 * 1024);
    vec![small]
}

fn fat(data: &[u8]) {
    const LIMIT: usize = 1 << 20;
    // A volume as long as the bytes, and one the bytes are the start of.
    for bytes in [data.len() as u64, 8 << 20] {
        if let Ok(mut volume) = ipl::fat::Volume::open(Cursor::new(data), bytes) {
            for path in [SIDE_A_PATH, SIDE_B_PATH, "EFI", "X/Y/Z.EFI"] {
                if let Ok(file) = volume.read_file(path, LIMIT) {
                    assert!(file.len() <= LIMIT);
                }
            }
        }
    }
}

fn lic_seeds() -> Vec<Vec<u8>> {
    let disk = ipl::build_disk(&[], true);
    let table = ipl::gpt::read(&mut Cursor::new(&disk), disk.len() as u64).unwrap_or_default();
    let mut seeds = Vec::new();
    for part in table.iter().filter(|p| p.type_guid != ipl::ESP_TYPE) {
        let (start, bytes) = (part.offset(), part.bytes());
        if let (Ok(start), Ok(bytes)) = (usize::try_from(start), usize::try_from(bytes)) {
            let mut record = disk[start..start + bytes].to_vec();
            // Both copies' slots, without the unused space after them.
            record.truncate(2 * 64 * 1024);
            let mut set = Cursor::new(record.clone());
            let settings = Settings {
                keylock: Some(Keylock::Manual),
                console: Some(ConsoleMode::Serial),
            };
            let _ = ipl::lic::prepare(&mut set, settings, 1);
            seeds.push(record);
            seeds.push(set.into_inner());
        }
    }
    seeds
}

fn lic(data: &[u8]) {
    // The bytes as a partition, and as a record's payload under a
    // good checksum.
    let framed = ipl::lic::sealed(&data[..data.len().min(60_000)]);
    for partition in std::iter::once(data.to_vec()).chain(framed) {
        lic_partition(&partition);
    }
}

fn lic_partition(data: &[u8]) {
    for settings in [
        Settings::default(),
        Settings {
            keylock: Some(Keylock::Secure),
            console: Some(ConsoleMode::Local),
        },
    ] {
        let mut dev = Cursor::new(data.to_vec());
        if let Ok(Some(recorded)) = ipl::lic::prepare(&mut dev, settings, 1) {
            // What was written reads back.
            if settings.any() {
                let again = ipl::lic::prepare(&mut dev, Settings::default(), 1);
                assert!(matches!(again, Ok(Some(r)) if r.sequence == recorded.sequence));
            }
        }
    }
}

fn disk_seeds() -> Vec<Vec<u8>> {
    let mut disk = disk_seed();
    // The table and the start of the ESP; the record is beyond, and a
    // disk cut short of it is an input like any other.
    disk.truncate(64 * 512 + 96 * 1024);
    vec![disk]
}

fn disk(data: &[u8]) {
    let mut sealed = data.to_vec();
    ipl::gpt::seal(&mut sealed);
    for side in [Side::A, Side::B, Side::Recorded] {
        let mut dev = Cursor::new(sealed.clone());
        let read = ipl::read(
            &mut dev,
            data.len() as u64,
            side,
            Settings::default(),
            1,
            &mut |_| {},
        );
        if let Ok((_, image)) = read {
            assert!(image.len() <= ipl::MAX_IMAGE_BYTES);
            let _ = ipl::load::check(&image);
        }
    }
}

/// Guest memory for the loader, made once: 128 MiB that are only
/// touched where an image is written.
fn loader_memory() -> &'static (Memory, Ram) {
    static MEMORY: OnceLock<(Memory, Ram)> = OnceLock::new();
    MEMORY.get_or_init(|| {
        let ram = Ram::for_bytes(128 << 20).expect("128 MiB is a partition's least");
        let memory =
            Memory::from_ranges(&[(GuestAddress(0), 128 << 20)]).expect("anonymous memory");
        (memory, ram)
    })
}

fn bzimage_seeds() -> Vec<Vec<u8>> {
    vec![kernel()]
}

fn bzimage(data: &[u8]) {
    let (memory, ram) = loader_memory();
    if let Ok(entry) = ipl::load::load(memory, ram, data) {
        assert!(entry < ram.low);
    }
}

fn luns() -> Vec<Lun> {
    vec![
        Lun::new(Box::new(MemoryStore::new(128 * 512, true)), "FUZZ-0").expect("a LUN"),
        Lun::new(Box::new(MemoryStore::new(16 * 512, false)), "FUZZ-1").expect("a LUN"),
    ]
}

fn scsi_seeds() -> Vec<Vec<u8>> {
    let cdbs: [&[u8]; 12] = [
        &[0x12, 0, 0, 0, 96, 0],
        &[0x12, 1, 0x83, 0, 255, 0],
        &[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[0x28, 0, 0, 0, 0, 4, 0, 0, 2, 0],
        &[0x2A, 0, 0, 0, 0, 4, 0, 0, 2, 0],
        &[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        &[0x1A, 0, 0x3F, 0, 255, 0],
        &[0x5A, 0, 0x08, 0, 0, 0, 0, 0, 255, 0],
        &[0x42, 0, 0, 0, 0, 0, 0, 0, 24, 0],
        &[0x93, 8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 4, 0, 0],
        &[0x9E, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 32, 0, 0],
        &[0xA0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0],
    ];
    cdbs.iter()
        .map(|c| {
            let mut input = vec![0u8; 32];
            input[..c.len()].copy_from_slice(c);
            // Data-out: an UNMAP list of one range, then filler.
            input.extend_from_slice(&[0, 22, 0, 16, 0, 0, 0, 0]);
            input.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 0]);
            input.extend_from_slice(&[0x5A; 1024]);
            input
        })
        .collect()
}

/// A command (32 bytes) and its data-out.
fn scsi(data: &[u8]) {
    let mut block = [0u8; 32];
    let n = data.len().min(32);
    block[..n].copy_from_slice(&data[..n]);
    let mut out = &data[n..];
    let luns = luns();
    let mut bounce = Bounce::new(4096);
    if let Ok(command) = cdb::decode(&block) {
        for lun in &luns {
            let _ = lun.execute(command, luns.len(), &mut out, &mut io::sink(), &mut bounce);
        }
    }
}

const QUEUE_MEMORY: usize = 0x4_0000;
const DESC: u32 = 0x1000;
const AVAIL: u32 = 0x2000;
const USED: u32 = 0x3000;

fn queue_at(memory: &Memory, size: u16) -> Option<Queue> {
    let mut queue = Queue::new(size).ok()?;
    queue.set_desc_table_address(Some(DESC), Some(0));
    queue.set_avail_ring_address(Some(AVAIL), Some(0));
    queue.set_used_ring_address(Some(USED), Some(0));
    queue.set_ready(true);
    queue.is_valid(memory).then_some(queue)
}

fn virtqueue_seeds() -> Vec<Vec<u8>> {
    // One request: a header, data-out, and room for the response and
    // data-in, as four descriptors; made available.
    let mut m = vec![0u8; 0x1_0000];
    let mut descriptor = |index: usize, addr: u64, len: u32, flags: u16, next: u16| {
        let at = DESC as usize + index * 16;
        m[at..at + 8].copy_from_slice(&addr.to_le_bytes());
        m[at + 8..at + 12].copy_from_slice(&len.to_le_bytes());
        m[at + 12..at + 14].copy_from_slice(&flags.to_le_bytes());
        m[at + 14..at + 16].copy_from_slice(&next.to_le_bytes());
    };
    descriptor(0, 0x8000, 51, 1, 1);
    descriptor(1, 0x9000, 1024, 1, 2);
    descriptor(2, 0xA000, 108, 3, 3);
    descriptor(3, 0xB000, 1024, 2, 0);
    m[AVAIL as usize + 2] = 1;
    let mut seeds = Vec::new();
    for command in [
        &[0x28u8, 0, 0, 0, 0, 1, 0, 0, 2, 0][..],
        &[0x2A, 0, 0, 0, 0, 1, 0, 0, 2, 0],
    ] {
        let mut seed = m.clone();
        seed[0x8000] = 1;
        seed[0x8000 + 19..0x8000 + 19 + command.len()].copy_from_slice(command);
        seeds.push(seed);
    }
    seeds
}

/// Guest memory whose first bytes are the input: the queues'
/// descriptor table and rings are in it, so all of them are the
/// guest's to write.
fn virtqueue(data: &[u8]) {
    let Ok(memory) = Memory::from_ranges(&[(GuestAddress(0), QUEUE_MEMORY)]) else {
        return;
    };
    let _ = memory.write_slice(&data[..data.len().min(QUEUE_MEMORY)], GuestAddress(0));
    let luns = luns();
    let mut bounce = Bounce::new(4096);
    if let Some(mut queue) = queue_at(&memory, 64) {
        serve_requests(&mut queue, &memory, &luns, &mut bounce);
    }
    if let Some(mut queue) = queue_at(&memory, 64) {
        serve_control(&mut queue, &memory);
    }
    if let (Some(mut queue), Ok(source)) = (queue_at(&memory, 64), File::open("/dev/zero")) {
        rng::fill(&mut queue, &memory, &source);
    }
}

/// A platform that grants everything and does nothing.
struct Nothing(Mutex<u32>);

impl Platform for Nothing {
    fn irqfd(&self, _: u32, _: &EventFd) -> io::Result<()> {
        Ok(())
    }
    fn allocate_gsi(&self) -> Option<u32> {
        let mut next = self.0.lock().ok()?;
        *next += 1;
        Some(23 + *next)
    }
    fn route_msi(&self, _: u32, _: Option<MsiRoute>) -> io::Result<()> {
        Ok(())
    }
    fn ioeventfd(&self, _: u64, _: &EventFd) -> io::Result<()> {
        Ok(())
    }
    fn remove_ioeventfd(&self, _: u64, _: &EventFd) -> io::Result<()> {
        Ok(())
    }
}

/// A virtio device that takes its queues and leaves them.
struct Idle(Option<Activation>);

impl VirtioDevice for Idle {
    fn device_type(&self) -> u32 {
        devices::virtio::TYPE_RNG
    }
    fn class(&self) -> u32 {
        0x00_FF_00
    }
    fn features(&self) -> u64 {
        0
    }
    fn queue_sizes(&self) -> Vec<u16> {
        vec![64, 64, 256]
    }
    fn read_config(&self, _: u64, data: &mut [u8]) {
        data.fill(0);
    }
    fn write_config(&mut self, _: u64, _: &[u8]) {}
    fn activate(&mut self, activation: Activation) -> io::Result<()> {
        self.0 = Some(activation);
        Ok(())
    }
    fn reset(&mut self) {
        self.0 = None;
    }
}

fn devices_seeds() -> Vec<Vec<u8>> {
    // Each access: a kind, three bytes of address, a length, eight
    // bytes of data. A driver's first steps: turn memory decoding on,
    // read the features, write the status.
    let access = |kind: u8, address: u32, len: u8, data: u64| {
        let mut a = vec![kind];
        a.extend_from_slice(&address.to_le_bytes()[..3]);
        a.push(len);
        a.extend_from_slice(&data.to_le_bytes());
        a
    };
    let mut seed = Vec::new();
    seed.extend(access(4, 0, 4, 0x8000_0804)); // configuration address: device 1, command
    seed.extend(access(4, 4, 2, 2)); // memory decoding on
    seed.extend(access(3, 0x04, 4, 0)); // device features
    seed.extend(access(2, 0x14, 1, 1));
    seed.extend(access(2, 0x14, 1, 3));
    seed.extend(access(2, 0x08, 4, 1));
    seed.extend(access(2, 0x0C, 4, 1));
    seed.extend(access(2, 0x14, 1, 0x0B));
    seed.extend(access(2, 0x1C, 2, 1));
    seed.extend(access(2, 0x14, 1, 0x0F));
    seed.extend(access(0, 0x71, 1, 0)); // the clock
    seed.extend(access(0, 0x3F8, 1, u64::from(b'Q')));
    seed.extend(access(6, 0x8010, 4, 0xFFFF_FFFF)); // size a BAR through ECAM
    vec![seed]
}

/// A run of accesses to a partition's ports and PCI memory.
fn devices_target(data: &[u8]) {
    let Ok(memory) = Memory::from_ranges(&[(GuestAddress(0), 0x10_0000)]) else {
        return;
    };
    let memory = Arc::new(memory);
    let platform: Arc<dyn Platform> = Arc::new(Nothing(Mutex::new(0)));
    let requests: Arc<Requests> = Arc::new(Box::new(|_| {}));
    let mut ports = Bus::new();
    let Ok(irq) = EventFd::new(0) else {
        return;
    };
    let serial = Arc::new(Mutex::new(Serial::new(irq, Box::new(io::sink()))));
    ports.insert(u64::from(SERIAL_PORT), 8, serial.clone());
    ports.insert(u64::from(CMOS_PORT), 2, Arc::new(Mutex::new(Cmos::new())));
    ports.insert(
        u64::from(I8042_PORT),
        5,
        Arc::new(Mutex::new(Reset::new(requests.clone()))),
    );
    ports.insert(
        u64::from(SLEEP_PORT),
        2,
        Arc::new(Mutex::new(Power::new(requests))),
    );
    let mut pci = PciBus::new();
    #[allow(clippy::cast_possible_truncation)] // 3 GiB
    let base = PCI_MEM_START as u32;
    let Ok(function) = VirtioPci::new(Box::new(Idle(None)), platform, memory, base, base + 0x8000)
    else {
        return;
    };
    pci.add(Arc::new(Mutex::new(function)));
    let pci = Arc::new(Mutex::new(pci));
    ports.insert(u64::from(PCI_CONFIG_PORT), 8, pci.clone());

    for access in data.as_chunks::<13>().0.iter().take(4096) {
        let address = u64::from(u32::from_le_bytes([access[1], access[2], access[3], 0]));
        let len = usize::from(access[4] % 8) + 1;
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&access[5..13]);
        let Ok(pci) = pci.lock() else { return };
        match access[0] % 8 {
            0 => {
                drop(pci);
                ports.write(address & 0xFFFF, &bytes[..len]);
            }
            1 => {
                drop(pci);
                ports.read(address & 0xFFFF, &mut bytes[..len]);
            }
            2 => {
                pci.write_memory(PCI_MEM_START + (address & 0x1_FFFF), &bytes[..len]);
            }
            3 => {
                pci.read_memory(PCI_MEM_START + (address & 0x1_FFFF), &mut bytes[..len]);
            }
            4 => {
                drop(pci);
                ports.write(u64::from(PCI_CONFIG_PORT) + (address & 7), &bytes[..len]);
            }
            5 => {
                drop(pci);
                ports.read(
                    u64::from(PCI_CONFIG_PORT) + (address & 7),
                    &mut bytes[..len],
                );
            }
            6 => {
                pci.write_memory(ECAM_START + (address & 0xF_FFFF), &bytes[..len]);
            }
            _ => {
                pci.read_memory(ECAM_START + (address & 0xF_FFFF), &mut bytes[..len]);
                if let Ok(mut serial) = serial.lock() {
                    serial.receive(&bytes[..len]);
                }
            }
        }
    }
}

/// Every target.
pub const TARGETS: &[Target] = &[
    Target {
        name: "gpt",
        about: "the load source's partition table",
        seeds: gpt_seeds,
        run: gpt,
    },
    Target {
        name: "fat",
        about: "the ESP's FAT volume",
        seeds: fat_seeds,
        run: fat,
    },
    Target {
        name: "lic",
        about: "the LIC record, read and written through licpart",
        seeds: lic_seeds,
        run: lic,
    },
    Target {
        name: "disk",
        about: "a whole load source, as the IPL loader reads it",
        seeds: disk_seeds,
        run: disk,
    },
    Target {
        name: "bzimage",
        about: "the UKI the loader is given",
        seeds: bzimage_seeds,
        run: bzimage,
    },
    Target {
        name: "scsi",
        about: "a SCSI command and its data, decoded and carried out",
        seeds: scsi_seeds,
        run: scsi,
    },
    Target {
        name: "virtqueue",
        about: "guest memory holding the virtio-scsi and virtio-rng queues",
        seeds: virtqueue_seeds,
        run: virtqueue,
    },
    Target {
        name: "devices",
        about: "accesses to the I/O ports, PCI configuration space and the virtio BARs",
        seeds: devices_seeds,
        run: devices_target,
    },
];

/// The target called `name`.
pub fn find(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_seed_runs_and_is_one_its_target_accepts() {
        for target in TARGETS {
            let seeds = (target.seeds)();
            assert!(!seeds.is_empty(), "{}", target.name);
            for seed in &seeds {
                (target.run)(seed);
            }
            (target.run)(&[]);
        }
        // The seeds are valid: the readers take them, so mutations
        // start from inside the formats and not at their first check.
        assert!(
            !ipl::gpt::read(&mut Cursor::new(&gpt_seeds()[0]), 1 << 30)
                .unwrap()
                .is_empty()
        );
        let fat = &fat_seeds()[0];
        assert!(ipl::fat::Volume::open(Cursor::new(fat), 8 << 20).is_ok());
        assert!(ipl::load::check(&bzimage_seeds()[0]).is_ok());
        let mut record = Cursor::new(lic_seeds()[0].clone());
        assert!(matches!(
            ipl::lic::prepare(&mut record, Settings::default(), 1),
            Ok(Some(_))
        ));
        assert!(find("scsi").is_some());
        assert!(find("none").is_none());
    }
}
