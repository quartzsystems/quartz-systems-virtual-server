// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Locating QEMU, choosing acceleration and building the command line.

pub mod channel;
pub mod firmware;
pub mod golden;
pub mod image;
pub mod serial;
pub mod session;
pub mod term;
pub mod vars;

use std::env;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use crate::error::HarnessError;

/// The system emulator binary.
pub const QEMU_BINARY: &str = "qemu-system-x86_64";
/// Environment override for the QEMU binary path.
pub const QEMU_ENV: &str = "QSVS_QEMU";
/// Set to run without KVM even where `/dev/kvm` is usable.
pub const NO_KVM_ENV: &str = "QSVS_NO_KVM";
/// The serial number of the boot disk.
pub const DISK_SERIAL: &str = "QSVSHD0";

/// A further disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drive {
    /// The raw image.
    pub path: PathBuf,
    /// Its serial number.
    pub serial: String,
    /// Its place on the PCI bus, counted from the first the harness
    /// gives out, when the launch pins places: the disk then has the
    /// same location code whichever other disks are there.
    pub place: Option<u8>,
}

/// How to launch the guest.
// Each flag is a switch of its own on QEMU's command line.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct Launch {
    /// QEMU binary.
    pub qemu: PathBuf,
    /// OVMF code image.
    pub firmware: PathBuf,
    /// A writable OVMF variable store, for a boot that goes by a
    /// firmware boot entry. The firmware keeps no variables when `None`.
    pub vars: Option<PathBuf>,
    /// Use KVM, with the host's processor so the guest sees VMX or SVM.
    pub kvm: bool,
    /// The disk image, if attached.
    pub disk: Option<PathBuf>,
    /// An ISO attached as a CD-ROM, if any.
    pub cdrom: Option<PathBuf>,
    /// Guest memory, MiB.
    pub memory_mb: u32,
    /// Guest processors.
    pub cpus: u32,
    /// Sockets, cores a socket and threads a core, instead of `cpus`.
    pub topology: Option<(u32, u32, u32)>,
    /// Disks after the boot disk.
    pub extra: Vec<Drive>,
    /// Every drive is a snapshot: the boot leaves the files as they
    /// were. Off, the guest's writes (and the firmware's) are kept.
    pub snapshot: bool,
    /// The MAC of a virtio NIC on QEMU's user network; no NIC when `None`.
    pub nic: Option<String>,
    /// A Unix socket for the guest's second serial port: the test
    /// hooks' channel.
    pub hook: Option<PathBuf>,
    /// A Unix socket for QEMU's monitor.
    pub monitor: Option<PathBuf>,
    /// SMBIOS OEM strings (type 11): the test hooks' switches.
    pub oem: Vec<String>,
    /// An image attached as a USB stick: the installation ISO as it is
    /// when written to one.
    pub usb: Option<PathBuf>,
    /// The firmware starts the CD-ROM or the USB stick before any disk:
    /// an installation, whatever the disks hold.
    pub media_first: bool,
    /// The guest's serial port takes what the harness types.
    pub keys: bool,
    /// Disks are at the places their [`Drive::place`] says.
    pub pinned: bool,
    /// The machine's memory is allocated before it runs. A machine that
    /// runs a partition needs it where the harness is itself in a
    /// virtual machine: memory the outer host brings in while a nested
    /// guest runs has reached that guest damaged (`docs/PHASE3.md`).
    pub prealloc: bool,
}

/// The PCI device number of a pinned disk's place.
const FIRST_PLACE: u8 = 0x10;

/// Find the system emulator: the override variable, else `PATH`.
pub fn locate() -> Result<PathBuf, HarnessError> {
    if let Some(p) = env::var_os(QEMU_ENV) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path)
        .map(|d| d.join(QEMU_BINARY))
        .find(|p| p.is_file())
        .ok_or(HarnessError::QemuNotFound {
            binary: QEMU_BINARY,
            env: QEMU_ENV,
        })
}

/// Whether `/dev/kvm` opens read-write on this host.
pub fn kvm_usable() -> bool {
    cfg!(target_arch = "x86_64")
        && OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok()
}

/// KVM is used when it is usable and `QSVS_NO_KVM` is unset.
pub fn kvm_wanted() -> bool {
    env::var_os(NO_KVM_ENV).is_none() && kvm_usable()
}

fn join(prefix: &str, path: &Path, suffix: &str) -> OsString {
    let mut s = OsString::from(prefix);
    s.push(path);
    s.push(suffix);
    s
}

fn drive(
    a: &mut Vec<OsString>,
    id: &str,
    path: &Path,
    serial: &str,
    boot: Option<usize>,
    place: Option<u8>,
) {
    a.push("-drive".into());
    // A write of zeros leaves a hole in the image file: a mirrored
    // set's first synchronization does not fill the host's disk.
    a.push(join(
        &format!("if=none,id={id},format=raw,discard=unmap,detect-zeroes=unmap,file="),
        path,
        "",
    ));
    a.push("-device".into());
    let index = boot.map_or_else(String::new, |i| format!(",bootindex={i}"));
    let at = place.map_or_else(String::new, |p| format!(",addr={:#x}", FIRST_PLACE + p));
    a.push(format!("virtio-blk-pci,drive={id},serial={serial}{index}{at}").into());
}

/// The QEMU argument list: `q35`, OVMF by pflash, the serial port on
/// standard output.
pub fn args(l: &Launch) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-machine", "q35"].map(OsString::from).to_vec();
    if l.kvm {
        a.extend(["-enable-kvm", "-cpu", "host"].map(OsString::from));
    }
    a.extend(["-m".into(), l.memory_mb.to_string().into()]);
    if l.prealloc {
        a.push("-mem-prealloc".into());
    }
    let smp = match l.topology {
        Some((sockets, cores, threads)) => format!(
            "{},sockets={sockets},cores={cores},threads={threads}",
            sockets * cores * threads
        ),
        None => l.cpus.to_string(),
    };
    a.extend(["-smp".into(), smp.into()]);
    a.push("-drive".into());
    a.push(join(
        "if=pflash,format=raw,readonly=on,file=",
        &l.firmware,
        "",
    ));
    if let Some(vars) = &l.vars {
        a.push("-drive".into());
        a.push(join("if=pflash,format=raw,file=", vars, ""));
    }
    // With the media first the disks follow it, in their order.
    let after = |i: usize| l.media_first.then_some(i + 1);
    let first = if l.media_first { ",bootindex=0" } else { "" };
    if let Some(disk) = &l.disk {
        drive(&mut a, "hd", disk, DISK_SERIAL, after(0), None);
    }
    for (i, extra) in l.extra.iter().enumerate() {
        drive(
            &mut a,
            &format!("hd{}", i + 1),
            &extra.path,
            &extra.serial,
            after(i + 1),
            extra.place.filter(|_| l.pinned),
        );
    }
    if let Some(cdrom) = &l.cdrom {
        a.push("-drive".into());
        a.push(join(
            "if=none,id=cd,media=cdrom,format=raw,file=",
            cdrom,
            "",
        ));
        a.push("-device".into());
        a.push(format!("ide-cd,drive=cd,bus=ide.0{first}").into());
    }
    if let Some(usb) = &l.usb {
        a.push("-drive".into());
        a.push(join(
            "if=none,id=stick,format=raw,readonly=on,file=",
            usb,
            "",
        ));
        a.extend(["-device", "qemu-xhci,id=xhci", "-device"].map(OsString::from));
        a.push(format!("usb-storage,bus=xhci.0,drive=stick,removable=on{first}").into());
    }
    if l.snapshot {
        a.push("-snapshot".into());
    }
    a.extend(
        [
            "-rtc",
            "base=utc",
            "-vga",
            "std",
            "-nographic",
            "-serial",
            "stdio",
            "-no-reboot",
        ]
        .map(OsString::from),
    );
    if let Some(hook) = &l.hook {
        a.push("-chardev".into());
        a.push(join("socket,id=hook,path=", hook, ",server=on,wait=off"));
        a.extend(["-serial", "chardev:hook"].map(OsString::from));
    }
    a.push("-monitor".into());
    match &l.monitor {
        Some(monitor) => a.push(join("unix:", monitor, ",server=on,wait=off")),
        None => a.push("none".into()),
    }
    match &l.nic {
        Some(mac) => {
            a.extend(["-netdev", "user,id=net0", "-device"].map(OsString::from));
            a.push(format!("virtio-net-pci,netdev=net0,mac={mac}").into());
        }
        None => a.extend(["-nic", "none"].map(OsString::from)),
    }
    if !l.oem.is_empty() {
        let values: Vec<String> = l.oem.iter().map(|v| format!("value={v}")).collect();
        a.push("-smbios".into());
        a.push(format!("type=11,{}", values.join(",")).into());
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch() -> Launch {
        Launch {
            qemu: "/usr/bin/qemu-system-x86_64".into(),
            firmware: "/fw/OVMF_CODE_4M.fd".into(),
            vars: None,
            kvm: true,
            disk: Some("/t/disk.img".into()),
            cdrom: None,
            memory_mb: 1024,
            cpus: 2,
            topology: None,
            extra: Vec::new(),
            snapshot: true,
            nic: None,
            hook: None,
            monitor: None,
            oem: Vec::new(),
            usb: None,
            media_first: false,
            keys: false,
            pinned: false,
            prealloc: false,
        }
    }

    fn words(l: &Launch) -> Vec<String> {
        args(l)
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    fn has(words: &[String], run: &[&str]) -> bool {
        words.windows(run.len()).any(|w| w == run)
    }

    #[test]
    fn kvm_brings_the_host_processor() {
        let with = words(&launch());
        assert_eq!(
            &with[..5],
            ["-machine", "q35", "-enable-kvm", "-cpu", "host"]
        );
        let without = words(&Launch {
            kvm: false,
            ..launch()
        });
        assert!(!without.iter().any(|w| w == "-enable-kvm" || w == "host"));
    }

    #[test]
    fn memory_is_allocated_first_only_when_asked() {
        assert!(!words(&launch()).contains(&"-mem-prealloc".into()));
        let asked = words(&Launch {
            prealloc: true,
            ..launch()
        });
        assert!(asked.contains(&"-mem-prealloc".into()));
    }

    #[test]
    fn firmware_is_pflash_and_the_drives_are_what_was_asked() {
        let w = words(&launch());
        assert!(w.contains(&"if=pflash,format=raw,readonly=on,file=/fw/OVMF_CODE_4M.fd".into()));
        assert!(w.contains(
            &"if=none,id=hd,format=raw,discard=unmap,detect-zeroes=unmap,file=/t/disk.img".into()
        ));
        assert!(w.contains(&"virtio-blk-pci,drive=hd,serial=QSVSHD0".into()));
        assert!(!w.iter().any(|x| x.contains("media=cdrom")));
        assert!(w.contains(&"-snapshot".into()));
        assert!(has(&w, &["-nic", "none"]));
        assert!(has(&w, &["-monitor", "none"]));
        assert!(has(&w, &["-smp", "2"]));

        let iso = words(&Launch {
            disk: None,
            cdrom: Some("/t/media.iso".into()),
            vars: Some("/t/vars.fd".into()),
            ..launch()
        });
        assert!(iso.contains(&"if=none,id=cd,media=cdrom,format=raw,file=/t/media.iso".into()));
        assert!(iso.contains(&"if=pflash,format=raw,file=/t/vars.fd".into()));
        assert!(!iso.iter().any(|x| x.contains("id=hd")));
    }

    #[test]
    fn the_base_suites_devices_are_added_when_asked_for() {
        let w = words(&Launch {
            topology: Some((2, 2, 1)),
            extra: vec![Drive {
                path: "/t/second.img".into(),
                serial: "QSVSHD1".into(),
                place: Some(1),
            }],
            snapshot: false,
            nic: Some("52:54:00:12:34:56".into()),
            hook: Some("/t/hook.sock".into()),
            monitor: Some("/t/mon.sock".into()),
            oem: vec!["qsvs.hook=churn".into(), "qsvs.hook=flaky-fails".into()],
            ..launch()
        });
        assert!(has(&w, &["-smp", "4,sockets=2,cores=2,threads=1"]));
        assert!(
            w.contains(
                &"if=none,id=hd1,format=raw,discard=unmap,detect-zeroes=unmap,file=/t/second.img"
                    .into()
            )
        );
        assert!(w.contains(&"virtio-blk-pci,drive=hd1,serial=QSVSHD1".into()));
        assert!(!w.contains(&"-snapshot".into()));
        assert!(has(
            &w,
            &[
                "-netdev",
                "user,id=net0",
                "-device",
                "virtio-net-pci,netdev=net0,mac=52:54:00:12:34:56"
            ]
        ));
        assert!(!has(&w, &["-nic", "none"]));
        // The channel is the second serial port, after the console.
        let console = w.iter().position(|x| x == "stdio").unwrap();
        let channel = w.iter().position(|x| x == "chardev:hook").unwrap();
        assert!(console < channel);
        assert!(w.contains(&"socket,id=hook,path=/t/hook.sock,server=on,wait=off".into()));
        assert!(has(
            &w,
            &["-monitor", "unix:/t/mon.sock,server=on,wait=off"]
        ));
        assert!(has(
            &w,
            &[
                "-smbios",
                "type=11,value=qsvs.hook=churn,value=qsvs.hook=flaky-fails"
            ]
        ));
    }
}
