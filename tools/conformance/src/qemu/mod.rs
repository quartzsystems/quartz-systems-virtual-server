// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Locating QEMU, choosing acceleration and building the command line.

pub mod firmware;
pub mod serial;
pub mod session;
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

/// How to launch the guest.
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
}

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

fn join(prefix: &str, path: &Path) -> OsString {
    let mut s = OsString::from(prefix);
    s.push(path);
    s
}

/// The QEMU argument list: `q35`, OVMF by pflash, the serial port on
/// standard output, no network. Every drive is a snapshot, so a boot
/// leaves the images as built.
pub fn args(l: &Launch) -> Vec<OsString> {
    let mut a: Vec<OsString> = ["-machine", "q35"].map(OsString::from).to_vec();
    if l.kvm {
        a.extend(["-enable-kvm", "-cpu", "host"].map(OsString::from));
    }
    a.extend(["-m".into(), l.memory_mb.to_string().into()]);
    a.extend(["-smp".into(), l.cpus.to_string().into()]);
    a.push("-drive".into());
    a.push(join("if=pflash,format=raw,readonly=on,file=", &l.firmware));
    if let Some(vars) = &l.vars {
        a.push("-drive".into());
        a.push(join("if=pflash,format=raw,file=", vars));
    }
    if let Some(disk) = &l.disk {
        a.push("-drive".into());
        a.push(join("if=none,id=hd,format=raw,file=", disk));
        a.push("-device".into());
        a.push("virtio-blk-pci,drive=hd,serial=QSVSHD0".into());
    }
    if let Some(cdrom) = &l.cdrom {
        a.push("-drive".into());
        a.push(join("if=none,id=cd,media=cdrom,format=raw,file=", cdrom));
        a.push("-device".into());
        a.push("ide-cd,drive=cd,bus=ide.0".into());
    }
    a.extend(
        [
            "-snapshot",
            "-rtc",
            "base=utc",
            "-vga",
            "std",
            "-nographic",
            "-serial",
            "stdio",
            "-monitor",
            "none",
            "-nic",
            "none",
            "-no-reboot",
        ]
        .map(OsString::from),
    );
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
        }
    }

    fn words(l: &Launch) -> Vec<String> {
        args(l)
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
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
    fn firmware_is_pflash_and_the_drives_are_what_was_asked() {
        let w = words(&launch());
        assert!(w.contains(&"if=pflash,format=raw,readonly=on,file=/fw/OVMF_CODE_4M.fd".into()));
        assert!(w.contains(&"if=none,id=hd,format=raw,file=/t/disk.img".into()));
        assert!(!w.iter().any(|x| x.contains("media=cdrom")));
        assert!(w.contains(&"-snapshot".into()));

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
}
