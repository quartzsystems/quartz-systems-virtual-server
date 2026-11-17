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
    for line in tape_probe(sys) {
        host.line(&line);
    }
    // Every line above is whole once this one is out.
    host.line("PIO DONE");
}

/// A passed tape drive as the guest sees it: what its INQUIRY said, a
/// block written and read back through `st`, a WRITE BUFFER the host
/// must refuse; then, on a thread, TEST UNIT READY every two seconds for
/// ten minutes, each change said on the console.
fn tape_probe(sys: &std::path::Path) -> Vec<String> {
    use std::fs;
    use std::io::{Read, Write};

    let read = |p: std::path::PathBuf| {
        fs::read_to_string(p)
            .map(|s| s.trim().to_string())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    let mut tapes: Vec<String> = fs::read_dir(sys.join("class/scsi_tape"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("st") && n[2..].bytes().all(|b| b.is_ascii_digit()))
        .collect();
    tapes.sort();
    for st in tapes {
        let dev = sys.join("class/scsi_tape").join(&st).join("device");
        out.push(format!(
            "PIO TAPE {st} {} {}",
            read(dev.join("vendor")),
            read(dev.join("model"))
        ));
        let mut block = vec![0u8; 4096];
        block[..15].copy_from_slice(b"QSVS-PIO-TAPE 1");
        let path = std::path::Path::new("/dev").join(&st);
        // The rewinding device writes a filemark and rewinds on close.
        let written = fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|mut f| f.write_all(&block));
        let mut back = vec![0u8; 4096];
        let read_back =
            written.and_then(|()| fs::File::open(&path).and_then(|mut f| f.read_exact(&mut back)));
        out.push(format!(
            "PIO TAPE {st} write read back {}",
            match read_back {
                Ok(()) if back == block => "ok".to_string(),
                Ok(()) => "differs".to_string(),
                Err(e) => format!("error {e}"),
            }
        ));
        let sg = fs::read_dir(dev.join("scsi_generic"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .next()
            .unwrap_or_default();
        let firmware = [0x3B, 0x05, 0, 0, 0, 0, 0, 0, 0x10, 0];
        out.push(format!(
            "PIO TAPE {st} WRITE BUFFER {}",
            sg_command(&sg, &firmware, &[0u8; 16])
        ));
        let tur = sg;
        let st_name = st.clone();
        std::thread::spawn(move || {
            let mut last = String::new();
            for _ in 0..300 {
                let now = sg_command(&tur, &[0; 6], &[]);
                if now != last {
                    if let Ok(mut c) = fs::OpenOptions::new().write(true).open("/dev/console") {
                        let line = format!("PIO TAPE {st_name} TUR {now}\r\n");
                        let _ = c.write_all(line.as_bytes());
                    }
                    last = now;
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    }
    out
}

/// `struct sg_io_hdr`.
#[repr(C)]
struct SgIoHdr {
    interface_id: i32,
    dxfer_direction: i32,
    cmd_len: u8,
    mx_sb_len: u8,
    iovec_count: u16,
    dxfer_len: u32,
    dxferp: *mut libc::c_void,
    cmdp: *const u8,
    sbp: *mut u8,
    timeout: u32,
    flags: u32,
    pack_id: i32,
    usr_ptr: *mut libc::c_void,
    status: u8,
    masked_status: u8,
    msg_status: u8,
    sb_len_wr: u8,
    host_status: u16,
    driver_status: u16,
    resid: i32,
    duration: u32,
    info: u32,
}

/// Send `cdb` with `data_out` to the generic device `sg`: `status S
/// sense K/ASC` as it ended, or `error E`.
fn sg_command(sg: &str, cdb: &[u8], data_out: &[u8]) -> String {
    use std::os::fd::AsRawFd;
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!("/dev/{sg}"))
    {
        Ok(f) => f,
        Err(e) => return format!("error {e}"),
    };
    let mut sense = [0u8; 32];
    let mut data = data_out.to_vec();
    let mut hdr = SgIoHdr {
        interface_id: i32::from(b'S'),
        dxfer_direction: if data.is_empty() { -1 } else { -2 },
        cmd_len: u8::try_from(cdb.len()).unwrap_or(6),
        mx_sb_len: 32,
        iovec_count: 0,
        dxfer_len: u32::try_from(data.len()).unwrap_or(0),
        dxferp: if data.is_empty() {
            std::ptr::null_mut()
        } else {
            data.as_mut_ptr().cast()
        },
        cmdp: cdb.as_ptr(),
        sbp: sense.as_mut_ptr(),
        timeout: 60_000,
        flags: 0,
        pack_id: 0,
        usr_ptr: std::ptr::null_mut(),
        status: 0,
        masked_status: 0,
        msg_status: 0,
        sb_len_wr: 0,
        host_status: 0,
        driver_status: 0,
        resid: 0,
        duration: 0,
        info: 0,
    };
    // SAFETY: SG_IO reads the header and the buffers it points to, each
    // of the length the header says, and writes the header and the
    // sense buffer; every buffer lives on this frame for the call. A
    // test image's alone.
    #[allow(unsafe_code)]
    let r = unsafe { libc::ioctl(file.as_raw_fd(), 0x2285, &raw mut hdr) };
    if r < 0 {
        return format!("error {}", std::io::Error::last_os_error());
    }
    if hdr.host_status != 0 {
        return format!("host status {:#x}", hdr.host_status);
    }
    let (key, asc) = if hdr.sb_len_wr >= 13 {
        (sense[2] & 0x0F, sense[12])
    } else {
        (0, 0)
    };
    format!("status {:#x} sense {key:#x}/{asc:#x}", hdr.status)
}
