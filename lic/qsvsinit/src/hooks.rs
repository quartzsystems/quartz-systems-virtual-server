// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The test hooks of PID 1, compiled under `cfg(test_hooks)` only: a
//! test image has them, a release image does not. A hook is switched
//! on for one IPL by an SMBIOS OEM string, `qsvs.hook=<name>`, which
//! the harness gives QEMU; the image itself is the same every IPL.

use std::time::Duration;

use hostinv::Roots;
use supervise::Service;

/// The hook that keeps a slot on trial from confirming.
pub const REFUSE_CONFIRM: &str = "refuse-confirm";

/// The hook that keeps a pending slot's trial from confirming, as an
/// update whose services never become ready: the trial's deadline then
/// restarts the host into the other slot.
pub const FAIL_TRIAL: &str = "fail-trial";

/// The hook that gives a trial 90 seconds to confirm, not 15 minutes.
pub const SHORT_TRIAL: &str = "short-trial";

/// The hook that holds the host's reserve of memory while the huge
/// pages are asked for, so the kernel gives fewer than asked.
pub const HOLD_MEMORY: &str = "hold-memory";

/// The test hooks' binary.
const BINARY: &str = "/qsvs/bin/qsvshook";

/// A service that fails on demand.
pub const FLAKY: Service = Service {
    name: "flaky",
    binary: BINARY,
    args: &["flaky"],
    order: 80,
    needs: &[],
    ready_within: Duration::from_secs(10),
};

/// The harness's way to the control socket: the second serial port.
pub const CHANNEL: Service = Service {
    name: "hook",
    binary: BINARY,
    args: &["channel"],
    order: 90,
    needs: &[],
    ready_within: Duration::from_secs(10),
};

/// What the partitions say, relayed to the serial port for the harness.
pub const PARTITION: Service = Service {
    name: "partition",
    binary: BINARY,
    args: &["partition"],
    order: 95,
    needs: &[],
    ready_within: Duration::from_secs(10),
};

/// Whether the hook `name` is on for this IPL.
pub fn has(name: &str) -> bool {
    let wanted = format!("{}hook={name}", release::KEY_PREFIX);
    hostinv::oem_strings(&Roots::default()).contains(&wanted)
}

/// What a partition's SMBIOS product is (`vmm/arch`'s `PRODUCT`).
const PARTITION_PRODUCT: &str = "QS/VS partition";
/// Where on a passed disk the probe's marker is: past any table.
const MARKER_AT: u64 = 1 << 20;
/// The marker's first bytes.
const MARKER: &str = "QSVS-PIO ";

/// The test image `IPLed` as a partition's system: the PCI functions it
/// was given whole, each listed, and on each `NVMe` disk the marker the
/// last partition left read, a new one written and read back from the
/// device. The physical I/O suites read the lines the host relays.
pub fn guest_probe(host: &mut crate::host::Host) {
    use std::fs;
    use std::os::unix::fs::FileExt;
    use std::path::Path;

    let sys = Path::new("/sys");
    let read = |p: &Path| {
        fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    if read(&sys.join("class/dmi/id/product_name")) != PARTITION_PRODUCT {
        return;
    }
    let serial = read(&sys.join("class/dmi/id/product_serial"));
    let devices = sys.join("bus/pci/devices");
    let mut names: Vec<String> = fs::read_dir(&devices)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let dir = devices.join(&name);
        let class = read(&dir.join("class"));
        let driver = fs::read_link(dir.join("driver"))
            .ok()
            .and_then(|t| Some(t.file_name()?.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "-".into());
        if class.starts_with("0x06") || driver == "virtio-pci" {
            continue;
        }
        let ids = format!(
            "{}:{}",
            read(&dir.join("vendor")),
            read(&dir.join("device"))
        );
        host.line(&format!("PIO GUEST {name} {ids} {class} {driver}"));
    }
    let mut disks: Vec<String> = fs::read_dir(sys.join("block"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("nvme"))
        .collect();
    disks.sort();
    for disk in disks {
        let path = Path::new("/dev").join(&disk);
        let Ok(file) = fs::OpenOptions::new().read(true).write(true).open(&path) else {
            host.line(&format!("PIO DISK {disk} cannot be opened"));
            continue;
        };
        let mut was = [0u8; 64];
        let found = match file.read_exact_at(&mut was, MARKER_AT) {
            Ok(()) => {
                let text = String::from_utf8_lossy(&was);
                let text = text.trim_end_matches('\0').trim_end();
                if text.starts_with(MARKER) {
                    text.to_string()
                } else {
                    "none".into()
                }
            }
            Err(e) => format!("unreadable: {e}"),
        };
        let marker = format!("{MARKER}{serial}");
        let mut block = [0u8; 4096];
        block[..marker.len()].copy_from_slice(marker.as_bytes());
        let written = file
            .write_all_at(&block, MARKER_AT)
            .and_then(|()| file.sync_all());
        // From the device, not the page cache.
        let _ = rustix::fs::fadvise(
            &file,
            MARKER_AT,
            std::num::NonZeroU64::new(4096),
            rustix::fs::Advice::DontNeed,
        );
        let mut back = [0u8; 4096];
        let same =
            written.is_ok() && file.read_exact_at(&mut back, MARKER_AT).is_ok() && back == block;
        host.line(&format!(
            "PIO DISK {disk} found {found} wrote {marker} read back {}",
            if same { "ok" } else { "differs" }
        ));
    }
}
