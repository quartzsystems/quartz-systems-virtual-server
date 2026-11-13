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
| Partition IPL | No guest firmware: the VMM loads the side's UKI from the partition's load-source disk and starts it through the 64-bit Linux boot protocol, with ACPI tables of its own. The keylock position and the console mode are written to the guest's LIC record before the IPL |
| Virtual devices | virtio over PCI for everything a partition does I/O with: scsi, net (vhost-net), console, rng. Four devices beside them for what virtio has no place for: a 16550, a CMOS clock, the reset line, the ACPI power-off register |
| Partition process | `qsvmm` opens what it needs as root, then runs as an unprivileged user with no capability. It exits with a code that says why the partition ended; a guest restart ends it |
| Storage | Storage pools are xfs file systems; virtual disks are raw files; software RAID as on QS/400, presented through a ublk block device |
| Virtual Ethernet | VLAN-aware kernel bridges, tap devices with vhost-net |
| Management | QSI server on TCP 2300 with mutual TLS; Service Tools console locally; nothing else listens |
| Disk layout | A load-source disk is GPT: ESP (slot A and B UKIs), a record partition, `QSVSDATA`. One disk, or two mirrored: each then has its own ESP and record, and `QSVSDATA` is one xfs on a protected set over the two. Two units that each ran without the other are not reconciled by the host: the operator chooses. Storage pools on other disks |
| Local console | Service Tools on the display and the serial port, each its own session; the installer on both at once. QS/400's `console` and screen kit |
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
carries its UKIs on an ESP, one file a side; the IPL loader (`vmm/ipl`)
reads the partition table, finds the ESP and the LIC partition by type,
reads the side's UKI from the FAT volume, loads it as the bzImage it is
and enters it at its 64-bit entry, with the memory map and the ACPI
root pointer in the zero page. The kernel's command line is baked in
and it ignores a loader's, so the side is chosen by which file is
loaded and nothing else can be passed that way. What an operator sets
at a panel, the keylock position and the console mode, the host writes
into the guest's LIC record before the IPL, through QS/400's `licpart`:
the one QS/400-specific crate QS/VS depends on, used by the loader
alone. The guest's disk is the guest's to write, so the loader reads it
as a hostile one (`docs/VMM.md`); a load source it cannot IPL from
gives a reference code that names what is missing. What QS/400 does in
a partition, and what it cannot do yet without firmware variables
(choose the side of its next IPL), is `docs/QS400_GUEST.md`.

**Virtual devices.** A partition does its I/O through virtio devices on
a PCI bus: virtio-scsi for disks, tape and optical, virtio-net backed
by vhost-net, a virtio console and virtio-rng. Phase 3 has virtio-scsi
with disk LUNs and virtio-rng. A virtual disk's backing store is opened
for direct I/O and a guest's flush is a flush of the host's device: the
guest's crash consistency is its own and rests on that. Beside the
virtio devices are four that virtio has no place for and a kernel
expects to find: a 16550 for its messages and its console, a CMOS
clock, the keyboard controller's reset line, and the ACPI sleep
register that powers the partition off. There is no other emulated
hardware: no display, no keyboard, no IDE. The interrupt controllers
and the PIT are KVM's (`docs/VMM.md`).

**The partition process.** `qsvmm` starts as root, opens KVM, the
partition's disks, its log and its console socket, and then becomes an
unprivileged user for good, with no capability and no way to gain one;
the guest runs only after that. It reports reference codes on its
standard error and whoever started it posts them. It ends when the
partition does, with an exit code that says why: powered off, restart
asked for, guest fault, could not IPL, VMM error, ended by the host. A
guest restart ends the process; starting it again is the partition
manager's, which is Phase 4. Until then nothing of a release image
starts `qsvmm`: a test image starts one partition, from a profile
written by hand (`docs/PROFILE.md`).

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

**Disk layout.** The disk a host starts from is its load source. It is
GPT: the ESP with both slots' UKIs; a small partition holding the A/B
record (two copies with a sequence and a CRC, QS/400's neutral `abrec`
under QS/VS's magic `QSVSREC` and its own partition type); and
`QSVSDATA`, an xfs for the host's own state, mounted at `/data`.
Storage pools live on other disks, so the host can be reinstalled
without touching a partition's data.

**The mirrored load source.** The load source is one disk, or two with
mirrored protection; parity is for storage pools, later. The firmware
must be able to start the host from either disk alone, so nothing the
firmware or PID 1 reads before the protected set is up can be inside
it:

- Each disk has its own ESP with both slots' UKIs, and the firmware has
  a boot entry for each slot of each disk. `BootOrder` names the
  default slot on the first disk, then on the second, then the other
  slot on each.
- Each disk has its own record partition. The A/B record is the copy
  with the highest valid sequence on either disk, and a write goes to
  both with one sequence, so a disk that was away is brought level at
  the next write.
- `QSVSDATA` is one xfs. Unprotected, it is the data partition.
  Mirrored, it is on a protected set over the two disks' data
  partitions: QS/400's `protect`, served to the kernel as a block
  device through `ublk` by the storage service, `qsvsstgd`.

`protect` keeps nothing on a member that says what it is, so the host
records the set itself, in the record partition of every disk and
outside the set (`base/loadsource/src/set.rs`): the set's identity and
size, each member by the GUID of its data partition with its state and
the location code and serial number its disk had, whether the set
was stopped in order, and when the copy was written. Beside it is the map of a resynchronization under
way, so a power loss does not start one over. The record partition so
holds three records, each as two copies under `abrec`: the A/B record,
the set's label, the resume map.

The storage service is the first of the service table. It assembles the
set, names the device, and only then does PID 1 mount `/data`; at a
shutdown `/data` is unmounted before the service is ended, and the
service marks the label clean as its last act. A label found open at
an IPL means the host was cut off while it ran: the two members may
differ in whatever was being written, so the second is written again
from the first (a resynchronization; the map lets it go on from where a
further power loss stopped it). A member that is missing or failed is
recorded as such on the other before the host goes on, the host runs on
the one, and an attention code is posted at every IPL while that lasts.
A member that comes back, or a disk put in its place, is written from
the other while the host runs.

The device is added so that it outlives its server
(`ublk`'s recovery): when the storage service ends without stopping
it, the kernel holds the requests to `/data` and the next server takes
them over, and a file system mounted on it stays mounted. PID 1
restarts the service by the rules of every service. When those end the
restarts, `/data` has no server: that is the fatal code `B2003000`
(`docs/REFCODES.md`).

Two things follow that an operator should know. After a power loss the
whole data partition is written again, not only what was in flight:
there is no write-intent bitmap (storage pools will need one in
`protect` before Phase 5). And a mirrored pair has no third party: if
each disk is started alone in turn, each records the other as missing
and goes on, and when both are back nothing on them says which the
operator wants.

**Units that ran apart.** The host does not pick a winner. Each disk
holds its own copy of the set's label; when one disk's copy has its
own unit in step and the other's not, and the other disk's copy says
the reverse, the storage service assembles nothing, is not ready, and
posts `A2003040`. PID 1 then mounts no `/data` and starts no other
service. It runs the Service Tools console for one panel, on the
display and the serial port at once and without a sign-on (the users
are on `/data`), which names both units with their location codes,
serial numbers and the times their disks were last written, and asks
which to keep. The console asks the storage service; that writes the
kept unit's label to both disks with the other unit stale
(`C2003050`), assembles the set, and the IPL goes on as any other,
the other unit being written again from the one kept. Nothing bounds
the wait. A unit that ran alone while the other never started is no
such case: only one copy then says the other was missing, and it is
the newer. The rule waits for Cody to confirm it
(`docs/OPEN_QUESTIONS.md` item 20).

**The slot rule.** A slot is chosen by firmware boot entry, as QS/400
chooses its own: there is a `Boot####` entry for each slot's UKI,
`BootOrder` names the default first, and a slot on trial is booted once
through `BootNext`. The A/B record holds what the firmware cannot: the
default slot, the pending slot or none, the attempts left for the
pending slot, and the release last confirmed in each slot. PID 1 reads
the slot it was booted from on the command line. When that is the
pending slot and the base services have all reported ready, it confirms:
the record's default becomes that slot and `BootOrder` is rewritten
through efivarfs to match. A pending slot that does not confirm was only
ever `BootNext`, so the next IPL comes up on the old default; that IPL
clears the pending field and posts an attention code. A pending slot
gets that one trial: the attempts left are 1 when it is set and 0 after. Writing the other
slot is the update phase's.

**The A/B record, version 2.** The payload is sixteen bytes
(`base/loadsource/src/record.rs`): the default slot; the pending slot, 0
for none; the attempts left; a reserved byte; the release last confirmed
in slot A and in slot B, four bytes each (a flag, then V, R and M); four
reserved bytes. Version 1 was Phase 0's four bytes, which nothing ever
read; a copy of that version is not a valid copy, and PID 1 writes a new
record for the slot it booted from when it finds no valid one.

**The local console.** A person at the machine has the display and the
first serial port. Both are QS/400's `console`: the 24x80 grid, drawn on
the framebuffer with its own font and on a serial terminal with ANSI
sequences, with its field editor and its keys. The screens are built
with QS/400's screen kit. The installer shows one set of screens on
both at once. On a running host the Service Tools console service gives
each its own session: the display shows the status screen until Enter
is pressed; the serial port stays PID 1's, with reference codes and the
status screen as lines, until Enter is pressed there, and is PID 1's
again at sign-off. The console edits no state file: it asks the network
service for a network change, which is tried and taken back if it does
not work, PID 1 for the host's settings and for power, and the storage
service for the disks. Service tools users are its own
(`docs/SERVICETOOLS.md`).

**Installation.** The media UKI carries `qsvs.install=1`, and PID 1
then runs the installer in place of the service table. The ISO carries
the two slots' UKIs and a manifest of them signed with the build's key;
the installer's initramfs holds the public key, and media that does not
verify is refused before anything is written (`docs/INSTALL.md`). The
host's identity, a key pair and a self-signed certificate, is made at
the first IPL of what was installed.

**The base crates.** What every Quartz appliance needs under its own
services is in `base/`: the appliance description, state, supervision,
logging, the management network with time, the seed, since Phase 2
the installer's screens, the panels every service console has, service
tools users and the host's identity, and since Phase 3 the load source
and the storage service that protects it: QS/MC is an appliance with a
load source too, and gets the same mirrored one. These were to
live in the QS/MC repository; that repository is still empty, so they
are written here and QS/MC will pin them from this repository, as this
repository pins QS/400's. Everything under `base/` is therefore
product-neutral. It takes an `Appliance` description from its caller
(the product identifier, the data and run directories, how to post a
reference code, the release banner, the seed label, and what its load
source carries that is its own: the GPT types and names of its
partitions, the ESP's vendor directory, the magics of its records) and
contains no QS/VS name outside its tests and comments, and no `base/`
crate depends on a crate of `lic/`. So a QS/VS disk and a QS/MC disk
have the same layout and neither appliance takes the other's for its
own. A test fails the build otherwise
(`tools/build/src/host/neutral.rs`, also a row of `build doctor`). The
QS/VS side is thin: `lic/release` has the description and QS/VS's
load-source names, `lic/refcode` says which reference code each base
event is, `qsvsstgd`, `qsvslogd` and `qsvsnetd` are the base services
run as QS/VS (the first with QS/VS's inventory, for the disks' serial
numbers and location codes), `lic/qsvsinstall` is the
installer's disks, media and state under the base screens, and
`svc/tools` is QS/VS's menu, system status and disk units over the
base panels.

**Identity.** The release is `V1R0M0`, from `VERSION` (`1.0.0`), through
QS/400's `vrm`. Fixes are numbered `QVnnnnn`, messages are prefixed
`QVS`, every environment variable QS/VS reads starts `QSVS_` and every
kernel command-line key `qsvs.`.

## What follows from them for the build

- The build pipeline is QS/400's `buildkit`, a git dependency pinned by
  revision with `vrm`, `diskimg` and `abrec`, all at one commit
  (`docs/PHASE0.md`). The console, the screen kit, the users, the
  protection engine and `ublk` are QS/400's too, at the same commit. `tools/build` is a clap enum over it and the QS/VS
  `Product` description.
- Builds are reproducible: fixed `SOURCE_DATE_EPOCH` and
  `KBUILD_BUILD_*`, a deterministic cpio, fixed GPT GUIDs, FAT volume id
  and timestamps, a fixed xfs UUID, and a signed content-hash manifest.
- Host reference codes are in families of their own (`docs/REFCODES.md`).
- There are two kinds of image. The release image is what ships. The
  test image is the release image with the test hooks added (the
  `qsvshook` binary, and PID 1 built under `cfg(test_hooks)`); it is
  built by `--test-image` under `test-image/` in the target root, so
  nothing of it is ever among the release artifacts, and its manifest
  names it a test image (`docs/PHASE1.md`).
