# Architecture: the locked decisions

QS/VS (Quartz Systems/Virtual Server) is a hypervisor appliance whose only
job is to run QS/400 instances as partitions. These decisions are locked;
later phases build on them. A change to one is a decision of its own, not
a side effect of other work.

| Area | Decision |
| --- | --- |
| Base | x86_64 only. Linux 6.18.y LTS, the same pin as QS/400, built `LLVM=1`, monolithic (`MODULES=n`), EFI stub, initramfs embedded; the bzImage is the UKI; two ESP slots A and B |
| Userland | 100% Rust, static musl. Rust PID 1 `qsvsinit` supervising a fixed service table. No systemd, no shell, no SSH |
| Hypervisor | KVM. Own VMM `qsvmm`, one process per partition, on the rust-vmm crates. No QEMU on the host |
| Partition IPL | No guest firmware: the VMM loads the partition's UKI from its virtual disk directly |
| Virtual devices | virtio over PCI only: scsi, net (vhost-net), console, rng |
| Storage | Storage pools are xfs file systems; virtual disks are raw files; software RAID as on QS/400, presented through a ublk block device |
| Virtual Ethernet | VLAN-aware kernel bridges, tap devices with vhost-net |
| Management | QSI server on TCP 2300 with mutual TLS; Service Tools console locally; nothing else listens |
| Disk layout | GPT: ESP (slot A and B UKIs), an A/B record partition, `QSVSDATA` (xfs). Storage pools on other disks |
| Identity | Release `V1R0M0` from `VERSION` (`1.0.0`). Fixes `QVnnnnn`. Message prefix `QVS`. Environment prefix `QSVS_`. Kernel command-line keys `qsvs.` |

## Each in a sentence or two

**Base.** There is one architecture and no abstraction over it. The
kernel is the only inherited component: Linux 6.18.y, pinned in
`kernel/VERSION` and `kernel/SHA256` to the same version and sum as
QS/400 at the pinned commit, so both products run on one kernel line and
a bump is one decision made in two repositories. It is built with
`LLVM=1` and no modules; the initramfs is embedded and the bzImage, an
EFI stub image, is the UKI the firmware loads. The ESP holds two of
them, slot A at `EFI/BOOT/BOOTX64.EFI` and slot B at `EFI/QSVS/B.EFI`, so
an update writes the other slot and a bad one is one firmware boot entry
away from the good one. The command line is baked and the firmware's is
ignored: `console=ttyS0,115200 console=tty0 panic=-1 quiet`, then
`qsvs.slot=A` or `qsvs.slot=B`; the media UKI carries `qsvs.install=1`
instead of a slot.

**Userland.** Every process above the kernel is Rust, linked statically
against musl; no crate compiles or links C, and `deny.toml` enforces it.
PID 1 is `qsvsinit`, which supervises a fixed table of services: there
is no general init system to configure, no shell to drop to and no SSH
to reach one.

**Hypervisor.** Partitions run on KVM. The VMM is QS/VS's own, `qsvmm`,
built on the rust-vmm crates, one process per partition so a partition's
failure and its resource limits are a process's. QEMU never runs on the
host; it is a test tool that stands in for a real machine.

**Partition IPL.** A partition has no firmware. A QS/400 system disk
carries its UKIs on an ESP; the VMM reads the UKI of the slot to IPL
from the partition's virtual disk and starts it directly.

**Virtual devices.** A partition sees virtio devices on a PCI bus and
nothing else: virtio-scsi for disks, tape and optical, virtio-net backed
by vhost-net, a virtio console and virtio-rng. No emulated legacy
hardware.

**Storage.** A storage pool is an xfs file system and a virtual disk is
a raw file in it. Protection is the software RAID QS/400 has, served to
the kernel as a block device through ublk, with the pool's xfs on top.

**Virtual Ethernet.** A virtual Ethernet is a VLAN-aware kernel bridge.
Each partition adapter is a tap device on it, driven by vhost-net; a
physical port or bond joins a bridge to carry it outside.

**Management.** The host is managed by QS/MC over QSI, a Quartz
protocol, served on TCP 2300 with mutual TLS. The Service Tools console
on the local display and the serial port covers what must work before
the network does: host basics, networking and disks. Nothing else
listens on the network.

**Disk layout.** The boot disk is GPT: the ESP with both slots' UKIs; a
small partition holding the A/B record (two copies with a sequence and a
CRC, QS/400's neutral `abrec` under QS/VS's magic `QSVSREC` and its own
partition type); and `QSVSDATA`, an xfs for the host's own state.
Storage pools live on other disks, so the host can be reinstalled
without touching a partition's data.

**Identity.** The release is `V1R0M0`, from `VERSION` (`1.0.0`), through
QS/400's `vrm`. Fixes are numbered `QVnnnnn`, messages are prefixed
`QVS`, every environment variable QS/VS reads starts `QSVS_` and every
kernel command-line key `qsvs.`.

## What follows from them for the build

- The build pipeline is QS/400's `buildkit`, a git dependency pinned by
  revision with `vrm`, `diskimg` and `abrec`, all at one commit
  (`docs/PHASE0.md`). `tools/build` is a clap enum over it and the QS/VS
  `Product` description.
- Builds are reproducible: fixed `SOURCE_DATE_EPOCH` and
  `KBUILD_BUILD_*`, a deterministic cpio, fixed GPT GUIDs, FAT volume id
  and timestamps, a fixed xfs UUID, and a signed content-hash manifest.
- Host reference codes are in families of their own (`docs/REFCODES.md`).
