// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Locating QEMU, choosing acceleration and building the command line.

use std::env;
use std::ffi::OsString;
use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use crate::{Error, wire};

/// The system emulator binary.
pub const QEMU_BINARY: &str = "qemu-system-x86_64";
/// The management NIC's address on QEMU's user network, which a
/// forwarded port reaches.
pub const USER_GUEST: &str = "10.0.2.15";
/// The PCI device number of the CD-ROM's virtio-scsi controller: fixed,
/// so it moves no other device's place.
pub const CDROM_PLACE: u8 = 0x1E;

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

/// A further NIC of the host, an uplink candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UplinkNic {
    /// Its MAC address.
    pub mac: String,
    /// What is on its far side.
    pub far: Far,
    /// The largest MTU its driver takes, when it is to have one.
    pub mtu: Option<u32>,
}

/// What is on an uplink NIC's far side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Far {
    /// The harness's station (`wire`), over QEMU's `dgram` backend.
    Station(wire::Ports),
    /// QEMU's user network, with a port of the harness's host forwarded
    /// to an address and port on it: `(host port, guest address, port)`.
    User(Option<(u16, String, u16)>),
}

/// The PCI device number of the first uplink NIC: its location code is
/// the same whatever else the machine has.
pub const FIRST_UPLINK_PLACE: u8 = 0x18;

/// How to launch the guest.
// Each flag is a switch of its own on QEMU's command line.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default)]
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
    /// The boot disk's serial number.
    pub disk_serial: String,
    /// An ISO attached as a virtio-scsi CD-ROM, if any. Not IDE: the
    /// q35 IDE CD-ROM hangs in OVMF on a share of boots.
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
    /// Further NICs after the management one, at fixed places.
    pub uplinks: Vec<UplinkNic>,
    /// Ports of the harness's forwarded to ports on the management
    /// address (QEMU's user network gives it [`USER_GUEST`]): `(host
    /// port, guest port)`.
    pub forwards: Vec<(u16, u16)>,
    /// The processor's model and features in place of the host's, under
    /// KVM: `host,-avx2,-adx`.
    pub cpu: Option<String>,
    /// An emulated IOMMU with interrupt remapping (Phase 10): the host's
    /// kernel can give its devices to partitions.
    pub iommu: bool,
    /// Emulated PCI devices for the host to give partitions, at fixed
    /// places.
    pub passed: Vec<Passed>,
    /// OVMF's Secure Boot build: the machine has SMM and the firmware's
    /// flash is the secure kind (`firmware` and `vars` say which).
    pub secure_boot: bool,
}

/// A device for the host to give to a partition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Passed {
    /// An Intel Ethernet adapter (e1000e) on QEMU's user network, which
    /// answers DHCP.
    Nic {
        /// Its place on the root bus.
        place: u8,
        /// Its MAC address.
        mac: String,
    },
    /// An `NVMe` controller with one namespace kept in `path`.
    Nvme {
        /// Its place.
        place: u8,
        /// Its namespace's file.
        path: PathBuf,
        /// Its serial number.
        serial: String,
    },
    /// Two `NVMe` functions of one device, which share an isolation
    /// group: functions 0 and 1 of the place, each with a namespace in
    /// its file. Not Intel's: Linux takes an Intel root-complex
    /// endpoint for isolated whatever its neighbours
    /// (`pci_quirk_rciep_acs`), so two e1000e would not share one.
    Pair {
        /// Its place.
        place: u8,
        /// The functions' namespaces.
        paths: [PathBuf; 2],
        /// The functions' serial numbers.
        serials: [String; 2],
    },
    /// A disk's virtio controller as function 0 and an `NVMe` function
    /// as function 1 of one place: one group, which the host keeps
    /// once the disk is a pool's.
    Shared {
        /// Its place.
        place: u8,
        /// The disk's file.
        path: PathBuf,
        /// The disk's serial number.
        serial: String,
        /// The `NVMe` function's namespace.
        beside: PathBuf,
        /// The `NVMe` function's serial number.
        beside_serial: String,
    },
}

/// The PCI device number of a pinned disk's place.
const FIRST_PLACE: u8 = 0x10;

/// Find the system emulator: the variable `qemu_env`, else `PATH`.
pub fn locate(qemu_env: &str) -> Result<PathBuf, Error> {
    if let Some(p) = env::var_os(qemu_env) {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok(p);
        }
    }
    let path = env::var_os("PATH").unwrap_or_default();
    env::split_paths(&path)
        .map(|d| d.join(QEMU_BINARY))
        .find(|p| p.is_file())
        .ok_or_else(|| Error::QemuNotFound {
            binary: QEMU_BINARY,
            env: qemu_env.to_string(),
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

/// KVM is used when it is usable and the variable `no_kvm_env` is
/// unset.
pub fn kvm_wanted(no_kvm_env: &str) -> bool {
    env::var_os(no_kvm_env).is_none() && kvm_usable()
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
#[allow(clippy::too_many_lines)] // one switch after another
pub fn args(l: &Launch) -> Vec<OsString> {
    // Interrupt remapping wants the I/O APIC in QEMU, not in KVM.
    let machine = match (l.iommu && l.kvm, l.secure_boot) {
        (true, true) => "q35,kernel-irqchip=split,smm=on",
        (true, false) => "q35,kernel-irqchip=split",
        (false, true) => "q35,smm=on",
        (false, false) => "q35",
    };
    let mut a: Vec<OsString> = ["-machine", machine].map(OsString::from).to_vec();
    if l.secure_boot {
        a.extend(["-global", "driver=cfi.pflash01,property=secure,value=on"].map(OsString::from));
    }
    if l.iommu {
        a.extend(["-device", "intel-iommu,intremap=on,caching-mode=on"].map(OsString::from));
    }
    if l.kvm {
        a.extend(["-enable-kvm", "-cpu"].map(OsString::from));
        a.push(l.cpu.as_deref().unwrap_or("host").into());
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
        drive(&mut a, "hd", disk, &l.disk_serial, after(0), None);
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
        a.push(format!("virtio-scsi-pci,id=cdscsi,addr={CDROM_PLACE:#x}").into());
        a.push("-device".into());
        a.push(format!("scsi-cd,drive=cd,bus=cdscsi.0{first}").into());
    }
    if let Some(usb) = &l.usb {
        // The installation ISO is media; any other image a stick to
        // write to.
        let iso = usb
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("iso"));
        a.push("-drive".into());
        a.push(join(
            if iso {
                "if=none,id=stick,format=raw,readonly=on,file="
            } else {
                "if=none,id=stick,format=raw,file="
            },
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
            let mut backend = String::from("user,id=net0");
            for (host, guest) in &l.forwards {
                let _ = write!(
                    backend,
                    ",hostfwd=tcp:127.0.0.1:{host}-{USER_GUEST}:{guest}"
                );
            }
            a.extend(["-netdev".into(), backend.into(), "-device".into()]);
            a.push(format!("virtio-net-pci,netdev=net0,mac={mac}").into());
        }
        None => a.extend(["-nic", "none"].map(OsString::from)),
    }
    for (i, up) in l.uplinks.iter().enumerate() {
        let backend = match &up.far {
            Far::Station(p) => format!(
                "dgram,id=up{i},local.type=inet,local.host=127.0.0.1,local.port={},remote.type=inet,remote.host=127.0.0.1,remote.port={}",
                p.qemu, p.harness
            ),
            Far::User(None) => format!("user,id=up{i}"),
            Far::User(Some((host, guest, port))) => {
                format!("user,id=up{i},hostfwd=tcp:127.0.0.1:{host}-{guest}:{port}")
            }
        };
        a.extend(["-netdev".into(), backend.into(), "-device".into()]);
        let place = u32::from(FIRST_UPLINK_PLACE) + u32::try_from(i).unwrap_or(0);
        let mtu = up
            .mtu
            .map_or_else(String::new, |m| format!(",host_mtu={m}"));
        a.push(
            format!(
                "virtio-net-pci,netdev=up{i},mac={},addr={place:#x}{mtu}",
                up.mac
            )
            .into(),
        );
    }
    for (i, p) in l.passed.iter().enumerate() {
        match p {
            Passed::Nic { place, mac } => {
                a.extend(["-netdev".into(), format!("user,id=pio{i}").into()]);
                a.push("-device".into());
                a.push(format!("e1000e,netdev=pio{i},mac={mac},addr={place:#x}").into());
            }
            Passed::Nvme {
                place,
                path,
                serial,
            } => {
                a.push("-drive".into());
                a.push(join(
                    &format!("if=none,id=pio{i},format=raw,file="),
                    path,
                    "",
                ));
                a.push("-device".into());
                a.push(format!("nvme,drive=pio{i},serial={serial},addr={place:#x}").into());
            }
            Passed::Shared {
                place,
                path,
                serial,
                beside,
                beside_serial,
            } => {
                a.push("-drive".into());
                a.push(join(
                    &format!("if=none,id=pio{i},format=raw,file="),
                    path,
                    "",
                ));
                a.push("-device".into());
                a.push(
                    format!(
                        "virtio-blk-pci,drive=pio{i},serial={serial},addr={place:#x}.0,multifunction=on"
                    )
                    .into(),
                );
                a.push("-drive".into());
                a.push(join(
                    &format!("if=none,id=pio{i}b,format=raw,file="),
                    beside,
                    "",
                ));
                a.push("-device".into());
                a.push(
                    format!("nvme,drive=pio{i}b,serial={beside_serial},addr={place:#x}.1").into(),
                );
            }
            Passed::Pair {
                place,
                paths,
                serials,
            } => {
                for (f, (path, serial)) in paths.iter().zip(serials).enumerate() {
                    a.push("-drive".into());
                    a.push(join(
                        &format!("if=none,id=pio{i}f{f},format=raw,file="),
                        path,
                        "",
                    ));
                    let multi = if f == 0 { ",multifunction=on" } else { "" };
                    a.push("-device".into());
                    a.push(
                        format!("nvme,drive=pio{i}f{f},serial={serial},addr={place:#x}.{f}{multi}")
                            .into(),
                    );
                }
            }
        }
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
            kvm: true,
            disk: Some("/t/disk.img".into()),
            disk_serial: "HD0".into(),
            memory_mb: 1024,
            cpus: 2,
            snapshot: true,
            ..Launch::default()
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
        assert!(w.contains(&"virtio-blk-pci,drive=hd,serial=HD0".into()));
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
        // The CD-ROM is virtio-scsi at its fixed place, never IDE.
        assert!(has(
            &iso,
            &[
                "-device",
                "virtio-scsi-pci,id=cdscsi,addr=0x1e",
                "-device",
                "scsi-cd,drive=cd,bus=cdscsi.0"
            ]
        ));
        assert!(!iso.iter().any(|x| x.contains("ide-cd")));
        assert!(iso.contains(&"if=pflash,format=raw,file=/t/vars.fd".into()));
        assert!(!iso.iter().any(|x| x.contains("id=hd")));
    }

    #[test]
    fn the_base_suites_devices_are_added_when_asked_for() {
        let w = words(&Launch {
            topology: Some((2, 2, 1)),
            extra: vec![Drive {
                path: "/t/second.img".into(),
                serial: "HD1".into(),
                place: Some(1),
            }],
            snapshot: false,
            nic: Some("52:54:00:12:34:56".into()),
            hook: Some("/t/hook.sock".into()),
            monitor: Some("/t/mon.sock".into()),
            oem: vec!["app.hook=churn".into(), "app.hook=flaky-fails".into()],
            ..launch()
        });
        assert!(has(&w, &["-smp", "4,sockets=2,cores=2,threads=1"]));
        assert!(
            w.contains(
                &"if=none,id=hd1,format=raw,discard=unmap,detect-zeroes=unmap,file=/t/second.img"
                    .into()
            )
        );
        assert!(w.contains(&"virtio-blk-pci,drive=hd1,serial=HD1".into()));
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
                "type=11,value=app.hook=churn,value=app.hook=flaky-fails"
            ]
        ));
    }

    #[test]
    fn ports_are_forwarded_to_the_management_address() {
        let w = words(&Launch {
            nic: Some("52:54:00:12:34:56".into()),
            forwards: vec![(40000, 2300), (40001, 22)],
            ..launch()
        });
        assert!(has(
            &w,
            &[
                "-netdev",
                "user,id=net0,hostfwd=tcp:127.0.0.1:40000-10.0.2.15:2300,hostfwd=tcp:127.0.0.1:40001-10.0.2.15:22"
            ]
        ));
    }

    #[test]
    fn the_devices_to_give_away_carry_the_serials_given() {
        let w = words(&Launch {
            passed: vec![
                Passed::Pair {
                    place: 0x1C,
                    paths: ["/t/a".into(), "/t/b".into()],
                    serials: ["PAIR0".into(), "PAIR1".into()],
                },
                Passed::Shared {
                    place: 0x1D,
                    path: "/t/d".into(),
                    serial: "DISK".into(),
                    beside: "/t/n".into(),
                    beside_serial: "BESIDE".into(),
                },
            ],
            ..launch()
        });
        assert!(w.contains(&"nvme,drive=pio0f0,serial=PAIR0,addr=0x1c.0,multifunction=on".into()));
        assert!(w.contains(&"nvme,drive=pio0f1,serial=PAIR1,addr=0x1c.1".into()));
        assert!(w.contains(&"nvme,drive=pio1b,serial=BESIDE,addr=0x1d.1".into()));
    }
}
