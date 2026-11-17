# QS/VS (Quartz Systems/Virtual Server)

A hypervisor appliance whose only job is to run QS/400 instances as
partitions. It is built the way QS/400 is built: the Linux kernel is the
only inherited component, and everything else is Rust. It has no web UI
and no shell. A separate appliance, QS/MC (Quartz Systems/Management
Console), manages it over a Quartz protocol called QSI, served by the
host's one listener, and a small Service Tools console on the local
display and serial port covers host basics, networking, disks and
pairing.

The build pipeline is QS/400's, taken as a library: `buildkit`, `vrm`,
`diskimg` and `abrec` are git dependencies on
`https://github.com/quartzsystems/quartz-systems-400`, all at one commit.
So are what the installer and the local console are made of: `console`,
`scrkit`, `svcusers`, `protect`, `ublk`, `blockdev`; and QSI's messages,
session and trust rules: `qsi`, `qsisess`, `qsitrust`, which the QSI
host server and `qsictl` alone speak. One crate that is
QS/400's own is taken too, at the same commit: `licpart`, for the
guest's LIC record, by the IPL loader alone. And the partition service
channel's protocol, `qsipart` (QS/400's `docs/QSID.md`), which the
partition manager alone speaks.

## Phases

| Phase | What it builds | Record |
|---|---|---|
| 0 | Toolchain and build: a clean clone builds a reproducible, bootable image whose Rust PID 1 prints a banner and proves KVM is usable | `docs/PHASE0.md` |
| 1 | PID 1 and base system: the supervised service table, state, logging, the management network and time, the host inventory, the A/B record with slot confirmation, the orderly shutdown | `docs/PHASE1.md` |
| 2 | Installer, ISO and Service Tools: the installer and its ISO, the mirrored load source with its storage service, the Service Tools console | `docs/PHASE2.md` |
| 3 | VMM core: `qsvmm`, and one QS/400 system taken from a disk image to a completed IPL in a partition | `docs/PHASE3.md` |
| 4 | Partition manager, profiles, processors and memory: `partd`, definitions and profiles, activation with resource accounting, dedicated and shared processors, huge-page memory, the state machine, auto start | `docs/PHASE4.md` |
| 5 | Virtual storage, optical and tape: storage pools, virtual disks, the media library, optical and tape devices, IPL source D | `docs/PHASE5.md` |
| 6 | Virtual Ethernet: virtual switches and their uplinks, the switch service, partitions' adapters and ports, virtio-net over vhost-net | `docs/PHASE6.md` |
| 7 | Partition service interface: virtio-console with the service and console ports, the service channel in `partd`, the system's codes and states, the shut down kinds, the panel functions, holding and dumps, the console as a full screen | `docs/PHASE7.md` |
| 8 | QSI host server: the one listener, pairing at Service Tools option 4, the host's families translated to the services, events on disk, `qsictl` | `docs/PHASE8.md` |
| 9 | Service, dumps and code update: problems from posted codes, the host's dump, the update image installed into the other slot with its rollback, Service Tools option 8 | `docs/PHASE9.md` |
| **10 (current)** | Physical I/O and tape pass-through: whole PCI devices with their IOMMU groups, released at Service Tools option 10 and named in profiles; a physical tape drive passed through command by command; a running shared partition's entitlement | `docs/PHASE10.md` |
| 11 | Release engineering and hardening | |

Work that is not done is listed in the current phase's record,
`docs/PHASE10.md`. Code and documents mark it `BLOCKED(phase10): reason`
(one line) or `NEEDS-CODY` (Cody's hardware or sign-off). No other
marker form. The records of earlier phases keep the markers they were
written with.

## Ground rules

- Every source file starts with exactly this header (the MIT license text
  is `LICENSE` in the repository root):
  ```
  // Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
  // licensed under the terms of the MIT license which can be found in the
  // root directory of this project.
  ```
  `cargo run -p build -- doctor` fails if any `.rs` file lacks it.
- Pure Rust, static musl, x86_64 only. No crate may compile or link C
  (`cc`, `cmake`, `bindgen`, `links =`). `deny.toml` enforces it, with the
  same rules as QS/400's; `cargo run -p build -- deny` checks it.
- Claude commits at each natural checkpoint as the repository's configured
  git user (Cody) only, with a one-line imperative message under 60
  characters. No `Co-Authored-By`, no AI attribution of any kind. Never
  push, rewrite history, branch or tag. Carry on until the phase is
  finished.
- Crates live in category folders, never at the repository root. Source
  files are grouped into folders by concern, not left flat under `src/`.
  Comments are short.
- QS/400 crates are git dependencies pinned to one commit (`rev = "..."`),
  the same commit for every crate, recorded in `docs/PHASE0.md`. Never
  path dependencies, never copies. Do not depend on QS/400's `svctools`:
  it is QS/400's own. `licpart` is the one QS/400-specific crate QS/VS
  depends on, and only the IPL loader (`vmm/ipl`) uses it, to read the
  guest's LIC record and write its IPL attributes. `console` is neutral
  since QS/400's handoff 14 and is taken without its `session` feature.
  `qsipart`, the service channel's protocol, is taken whole: nothing of
  it is restated, and `qsvmm` never parses it.
- Everything under `base/` is product-neutral: QS/MC will pin those
  crates from this repository. What QS/MC's installer and local console
  will also need goes there: the installer's screens, the common
  panels, service tools users. So does the load source and its
  storage service: QS/MC has the same mirrored load source. Only
  QS/VS's menu and its host-specific panels are under `svc/`. A `base/`
  crate takes an `Appliance` description from its caller, has no
  `qsvs` or `QSVS` in its code outside tests and comments, and depends
  on no crate of `lic/`. `build doctor` and a unit test fail otherwise.
- What is on an installed disk and is QS/VS's own (the partition
  types and names, the records' magics, the set's label) is
  `release::LOAD_SOURCE`. None of it may change: a test there holds
  each value.
- The local console edits no state file. It asks: the network service
  to apply and keep a network change, PID 1 for the host's settings
  and for power, the storage service for the disks. PID 1's loop never
  waits on `/data`: a write there is another thread's.
- No password is kept, logged or shown: only its salted hash.
- Every byte a guest controls is hostile: a register write, a virtqueue
  descriptor, a SCSI command, every byte of its disks (`docs/VMM.md`).
  Code that reads one does not panic on it, read or write outside what
  the guest was given, or allocate by a size it names. A new reader of
  guest bytes gets a target in `tools/fuzz`.
- A release image's partition manager accepts only virtual disks of
  storage pools as backing stores (`POOL/NAME`); a host path is a test
  image's alone (`cfg(test_hooks)` in `qsvspartd`).
- The pool service (`stg/poold`) is a service of its own, apart from
  the load source's: a fault in pool code cannot take `/data` away.
  The pool labels are the truth; `pools.toml` is written from them.
- The switch service (`net/vswitchd`) is a service of its own, apart
  from the network service: a fault in switch code cannot take the
  management network away. The management interface is never a
  switch's uplink, and the host has no address on any switch, bridge,
  bond or tap and forwards nothing (`docs/NETWORK.md`). A partition's
  port passes only its adapter's address on its own VLANs, by the
  bridge's controls and a tc filter; `NETFILTER` stays off. `qsvmm` is
  handed its taps open and opens no network device by name.
- `qsvmm` opens what it needs as root and then runs as its partition's
  own unprivileged user (23000 plus the partition's ID) with no
  capability; it reports reference codes on standard error and the
  partition manager posts them. `partd` alone starts it, and gives it
  its whole configuration: `qsvmm` reads no profile and decides nothing.
- The service channel is `partd`'s (`docs/SERVICE.md`): it binds the
  two ports' listening sockets, hands them to `qsvmm`, and speaks
  `qsipart` on them. `qsvmm` carries the bytes and reads none. What a
  system sends is hostile: bounded, never a panic, nonsense closes the
  channel and the partition runs on. A partition held with its memory
  is dumped from outside, into the media repository's pool.
- What acts on a partition in a release image is the QSI host server,
  for a pinned console: Service Tools lists them, PID 1 has them ended
  before it stops the services. The test hooks carry the harness's
  requests to `partd`'s socket, in a test image only.
- The QSI host server (`qsi/server`, `lic/qsvsqsid`) is the one
  listener: TCP 2300 on the management addresses, TLS 1.3 both ways,
  pinned consoles only (two at most), and while a pairing password is
  out an unknown peer may only pair. It is a service of its own, as user
  22990 with no capability; the sockets it asks are shared with its
  group and nobody else (`release::share_with_server`). It owns no state
  but its pins and its event log, translates each request to the service
  that owns the state, and restates nothing of QSI: its messages, session
  and pairing are QS/400's crates. A pairing password is shown once at
  Service Tools and kept in memory only; the request log names the
  console and its user, and never a password, a key or a screen.
- `tools/qsictl` is a developer's console for Linux and Windows, in no
  image.
- A request that fails answers with `QVS` messages from the catalogue
  (`lic/qvsmsg`), every one that applies; `docs/MESSAGES.md` lists
  them and a unit test holds the two together. No message borrows
  another vendor's wording.
- What `partd` wants done to the machine it asks of its `System`
  (`part/partd/src/manager`), so every request and every end of a VMM
  runs in a unit test. `part/resource` is arithmetic only: it reads no
  file and writes none.
- Test hooks are compiled only into a test image (`--test-image`,
  `cfg(test_hooks)`, the `qsvshook` binary). Nothing of them may reach a
  release artifact; the userland and manifest steps check.
- No IBM text, product names or marks anywhere. The names in use are
  QS/VS, QS/MC and QS/400.
- The problem service (`svc/problem`, `lic/qsvsprobd`) is a service of
  its own: it reads the reference code history and opens, counts and
  clears problems by the catalogue's table (`lic/qvsmsg`), which holds
  every attention and fatal code; a unit test holds it to
  `docs/PROBLEMS.md`. A posting says what it is about (`supervise::About`).
- A host dump (`svc/hostdump`) is service data in a standard ZIP with
  its manifest, taken by PID 1; it never holds the host's key, the users'
  hashes or the consoles' pins (`hostdump::take::LEFT_OUT`), and a test
  fails the build if a secret file is not left out.
- A code update (`svc/update`, `lic/qsvsupdd`) is verified against the
  running image's trusted keys before a byte is written, and nothing
  turns that off. It writes only the slot that does not run; PID 1 alone
  writes the A/B record and the firmware's variables. A pending slot's
  first IPL keeps a copy of state; a slot that does not confirm is
  followed by the old one, which puts it back.
- Physical I/O (`part/pio`, `docs/PHYSICALIO.md`): a device is given
  whole with its IOMMU group, only when the IOMMU and interrupt
  remapping are on, every function resets alone and signals message
  interrupts; no setting relaxes any of it. The load source's and the
  pools' controllers, the management interface, every uplink and the
  console's display are never released. Only Service Tools option 10
  releases or takes back; a console names released devices in
  profiles. A device moves only at activation and at the partition's
  end, is reset before and after, and is fenced with its problem until
  the host restarts when a reset fails. `qsvmm` gets VFIO's descriptors
  from `partd` and maps the partition's memory, all of it and nothing
  else, for the device's DMA.
- A physical tape drive is passed through by `partd`'s relay
  (`SG_IO`, as root), never by `qsvmm`, which holds one end of a
  socket; every command is checked against the allow-list
  (`stg/tape/src/passthrough.rs`) in both. One holder at a time; no
  media changer.
- Anything not done is listed in `docs/PHASE10.md`.

## Locked decisions

See `docs/ARCHITECTURE.md`. Short form:

- **Base.** x86_64 only. Linux 6.18.y LTS, the same pin as QS/400, built
  `LLVM=1`, monolithic (`MODULES=n`), EFI stub, initramfs embedded. The
  bzImage is the UKI. Two ESP slots, A and B.
- **Userland.** 100% Rust, static musl. Rust PID 1 `qsvsinit` supervising
  a fixed service table. No systemd, no shell, no SSH.
- **Hypervisor.** KVM. Own VMM `qsvmm`, one process per partition, on the
  rust-vmm crates. No QEMU on the host.
- **Partition IPL.** No guest firmware: the VMM loads the partition's UKI
  from its virtual disk directly. The side is the activation's, or
  the one the partition itself asked for in its LIC record
  (`licpart::IplSource`), taken once an activation. Seven bytes at the reset vector end
  the partition when its guest restarts by a jump there; nothing of
  the VMM's runs before the guest's kernel.
- **Virtual devices.** virtio over PCI only: scsi (disks; an optical
  drive and a tape drive each on a function of its own), net
  (vhost-net), console, rng.
- **Partition manager.** `partd`, a supervised service: definitions and
  profiles in state, activation with resource accounting, every `qsvmm`
  in its own control group, the states Not activated, Starting,
  Running, Shutting down, Error. Driven over a local socket
  (`docs/PARTD.md`), which the QSI host server will sit on. Its
  partitions go on when it is started again.
- **Processors.** A processor is one host core: both threads with SMT,
  kept to one partition at a time by core scheduling. Dedicated: whole
  cores, pinned. Shared: virtual processors over a pool's cores, with
  processing units (1.00 a core), capped or uncapped by weight. The
  host reserves cores for itself (one by default).
- **Memory.** 2 MB huge pages, reserved by PID 1 at the IPL: no
  overcommit, no ballooning. The host reserves memory for itself (the
  larger of 2 GiB and 5%).
- **Changing resources.** At activation only, but a running shared
  partition's entitlement (units, capping, weight) within its profile
  and its pool, by the kernel's controls (`docs/RESOURCES.md`).
- **Physical I/O.** A whole PCI device with its IOMMU group, released
  at Service Tools, named in a profile, given at activation; the
  IOMMU, interrupt remapping, resets and message interrupts required;
  reset before and after, fenced on a failed reset. A tape drive passed
  through command by command through an allow-list
  (`docs/PHYSICALIO.md`, `docs/STORAGE.md`).
- **Partition identity.** A serial number made once, from the host's
  and the partition's ID, in the definition; SMBIOS tables carry it.
  The keylock is the host's, written at every activation.
- **Whose disks.** The host takes the disk it IPLed from and the
  members that disk's label names, and no other.
- **Storage.** A storage pool is one protected set of whole disks
  (none, mirrored, RAID 5, RAID 6; QS/400's `protect`) with one xfs on
  it, presented through `ublk`, its size fixed when made, each disk
  carrying a member label of this host. Virtual disks are raw files,
  thin or thick, one partition's at a time, handed to its VMM open. A
  pool that fills holds its partitions (`A2005160`). The media library
  (optical images, AWSTAPE tape volumes) is in the pool designated as
  the media repository (`docs/STORAGE.md`).
- **Virtual Ethernet.** VLAN-aware kernel bridges, tap devices with
  vhost-net.
- **Service channel.** One virtio-console a partition with the
  service port and the console port, as QS/400's `docs/QSID.md` names
  them; `partd` speaks the protocol, `qsvmm` carries bytes. Starting
  until the system reports its IPL complete; a system with no channel
  is Running once its kernel is entered and the wait is over. Shut down
  delayed, operating system or immediate. A terminal code or a fault
  holds the partition in Error with its memory, for a dump (an ELF
  core in the media repository's pool). The panel functions are the
  host's but 21. The host's own shut down goes through each system, in
  reverse start order. One console viewer at a time.
- **Management.** One listener: the QSI host server on TCP 2300 of the
  management addresses, TLS 1.3 with both ends pinned, consoles paired
  at Service Tools option 4; the Service Tools console locally; nothing
  else listens (`docs/QSISERVER.md`).
- **Disk layout.** A load-source disk is GPT: ESP (slot A and B UKIs), a
  record partition, `QSVSDATA` (xfs, mounted at `/data`). Storage pools
  on other disks.
- **Load-source protection.** None (one disk) or mirrored (two).
  Mirrored, each disk has its own ESP and record partition, and
  `QSVSDATA` is one xfs on a protected set over the two data partitions
  (QS/400's `protect`), served through `ublk` by the storage service,
  which starts before `/data` is mounted and outlives its own restart.
  The set's label and its resume map are beside the A/B record. Parity
  is for storage pools.
- **Units that ran apart.** When each disk of a mirrored pair ran
  without the other, the host picks no winner: it posts `A2003040`,
  mounts no `/data`, starts nothing else and asks at the console which
  unit to keep; the other is written again from it. Confirmed
  (`docs/OPEN_QUESTIONS.md` item 20).
- **The slot rule.** A slot is chosen by firmware boot entry; there is
  one for each slot of each load-source disk. The A/B record
  (version 2) holds the default slot, the pending slot, the attempts
  left and the release last confirmed in each slot; with two disks the
  copy with the highest sequence is the record and a write goes to
  both. A pending slot confirms when the base services are all ready:
  the record's default and the firmware's `BootOrder` become it.
- **Problems.** One record a condition, opened from a posted reference
  code by the catalogue's table, kept by the problem service, closed by
  a person (`docs/PROBLEMS.md`).
- **Host dump.** Service data, not memory, in one ZIP with its
  manifest, on `/data`, bounded (`docs/DUMPS.md`).
- **Code update.** One signed image of both slots' UKIs, installed into
  the slot that does not run on every load-source disk, activated by a
  restart; a trial unconfirmed in 15 minutes rolls back
  (`docs/UPDATE.md`).
- **Installation.** The media UKI runs the installer in place of the
  service table. The ISO carries the slot UKIs and a manifest of them
  signed by the build's key; the installer verifies it before writing.
  Attended on the display and serial port at once, or unattended from
  an `[install]` table of a seed file.
- **Local console.** Service Tools: sign-on with a service tools user
  ID, a session each for the display and the serial port. The serial
  port is PID 1's, in lines, while nobody is signed on there. Option 4
  pairs management consoles; option 8 is the service functions
  (problems, host dumps, update, code levels, shutting down every
  partition) and option 7 shows the problem log.
- **Time zones.** Fixed UTC offsets only, in V1.
- **Base crates.** `base/` is product-neutral and shared with QS/MC,
  the load source and its storage service included.
- **Identity.** Release `V1R0M0` from `VERSION` (`1.0.0`). Fixes
  `QVnnnnn`. Message prefix `QVS`. Environment prefix `QSVS_`. Kernel
  command-line keys `qsvs.`.
- **Kernel pin.** `kernel/VERSION` and `kernel/SHA256`, the same as
  QS/400's at the pinned commit. A bump is its own commit.
- **Command line.** Baked: `console=ttyS0,115200 console=tty0 panic=-1
  quiet`, then `qsvs.slot=A` or `qsvs.slot=B`; the media UKI carries
  `qsvs.install=1` instead of a slot.
- **Reproducible.** Fixed `SOURCE_DATE_EPOCH` and `KBUILD_BUILD_*`,
  deterministic cpio, GPT, FAT and xfs, signed content-hash manifest.

## Layout

```
VERSION         1.0.0, the release (V1R0M0)
kernel/         VERSION, SHA256, config/qsvs.config
tools/          host tools
  qsictl/         the developer's QSI console, Linux and Windows, in no image (docs/QSICTL.md)
  build/          the orchestrator, a thin binary over buildkit: cargo run -p build -- <cmd>
    src/product/    QS/VS as buildkit's Product: names, kernel symbols, initramfs, media; the two image kinds
    src/image/      the userland step and the hooks check, the A/B record on the disk image, the manifest's kind, the update image
    src/host/       doctor rows, the license header check, the neutrality check of base/, `test`, `fuzz`
  conformance/    the QEMU harness
    src/qemu/       launch, OVMF, serial capture, the serial port as a 24x80 grid, typed keys, golden files, firmware variables, image copies, the test hooks' channel
    src/suite/      Phase 0's steps; base/ has the ten suites of Phase 1; install/ the fifteen of Phase 2 and of the answers to its questions; partition/ the ten of Phase 3 and the sixteen of Phase 4, with the partition manager's client over the hooks' channel; storage/ Phase 5's, with the pool service's client; network/ Phase 6's; service/ Phase 7's; qsi/ Phase 8's, a console over the management network; servicing/ Phase 9's; physio/ Phase 10's
    golden/         every installer and Service Tools screen, as text
  fuzz/           qsvsfuzz, the Rust fuzzer: LLVM's coverage counters, the mutator, the loop, a target for every reader of guest bytes
base/           what every Quartz appliance needs; product-neutral
  appliance/      the Appliance description with its load-source names, events, civil time, the Ethernet links
  state/          typed TOML state files: schema versions, migration, atomic writes
  supervise/      the service table, restart policy, the runner, the control protocol, signals
  logd/           the NDJSON system log, the reference code history, the kernel log per boot
  netd/           the management interface (DHCP or static, VLAN, carrier, reload, a change on trial) and NTP
  seed/           seed.toml on a labelled FAT volume, with the [install] table
  users/          service tools user IDs: the password rules and the sign-on rule over QS/400's svcusers
  panels/         panels with help, the console on display and serial, the common forms and Service Tools panels, the unit to keep
  install/        the installer's screens and the answers of an unattended installation
  identity/       the host's key pair and self-signed certificate
  loadsource/     the load-source disks: which disks are the host's own, the GPT layout, the A/B record and the set's label on every disk, units that ran apart, the boot entries, the media's payload
  stgd/           the storage service: the mirrored load source assembled, served through ublk, resynchronized by its write-intent map, rebuilt; its socket
  fdpass/         open files handed between processes over a Unix socket
lic/            PID 1, release, installer, base services
  qsvsinit/       PID 1
    src/ipl/        the IPL in order: mounts and the host's control group, the command line, the storage step and the unit to keep, the clock, the state step, the partitions' memory; the hand-off to the installer
    src/host/       the running host: the loop, the service table, the control socket, the partitions' end and the shutdown
    src/slot/       the slot rule: the A/B record at IPL, confirmation, the firmware boot entries
    src/console/    /dev/console and /dev/ttyS0, the status screen
    src/kvm/        the KVM probe and its report
    src/sys/        reaping, the power button, the panic path
    src/hooks.rs    PID 1's test hooks, cfg(test_hooks) only
  qsvslogd/       base/logd run as QS/VS
  qsvsnetd/       base/netd run as QS/VS
  qsvsstgd/       base/stgd run as QS/VS, with QS/VS's inventory
  qsvspartd/      part/partd run as QS/VS, with QS/VS's cores, memory and serial number
  qsvspoold/      stg/poold run as QS/VS, with QS/VS's pool disk names, inventory and identity
  qsvsvswitchd/   net/vswitchd run as QS/VS, with the inventory's interfaces, netd's management device and partd's adapters
  qsvsqsid/       qsi/server run as QS/VS: its identity read as root, its directory and local socket, then user 22990
  qsvsprobd/      svc/problem run as QS/VS
  qsvsupdd/       svc/update run as QS/VS, over PID 1's slots
  qvsmsg/         the message catalogue: QVS messages, first-level and second-level text, the problem table
  qsvsinstall/    the installer: base/install over QS/VS's media, disks and state
  qsvshook/       the test hooks' binary: the harness's channel, a failing service, the state churn
    src/partition/  what the partitions say, relayed to the serial port; the probe of requests no guest sends; a console viewer and a dump's read; a hostile system
  release/        V1R0M0 over QS/400's vrm, the banner, the prefixes, the Appliance description, QS/VS's load-source and pool disk names
  refcode/        host reference codes (docs/REFCODES.md), the code of each base event, the poster
  hostinv/        the host inventory, the processors' threads, and the location codes (docs/LOCATION.md)
vmm/            the VMM: one process for one partition
  arch/           x86_64 boot state: the memory map, page tables and GDT, registers, the zero page, the reset vector, the SMBIOS tables; CPUID and MSR policy with a core's sibling threads, the local APIC
  acpi/           the tables in place of firmware: RSDP, XSDT, FADT, MADT, MCFG, the DSDT
  devices/        the I/O port bus; the 16550, the CMOS clock, the reset line, the power-off register; the PCI bus, configuration space, MSI-X; virtio-pci, virtio-scsi, virtio-net, virtio-console, virtio-rng; a host's function passed through (pci/passthrough) over VFIO (sys/vfio)
    src/virtio/scsi/  the command decoder, a disk LUN and its hold for space, the optical drive, the tape drive, the backing stores
    src/virtio/console/  the control queue's messages, a port's link to the host, the device
  ipl/            the IPL loader: bounded GPT, FAT and ISO 9660 readers, the LIC record through licpart, the bzImage into guest memory, IPL source D
  qsvmm/          the binary: the configuration it is given, the exit codes, the machine on KVM with huge-page memory, the processors' loop, their pinning and their pause, a partition kept after a fault, the serial log and console socket, the control socket (media, hold, dump), the service channel's ports on their sockets, the dump writer, the privilege drop
part/           the partition manager
  model/          a partition's definition and its profiles, as state files and on the wire; validation with every error
  resource/       cores and memory: installed, configurable, available; what an activation gets; pools; the cgroup tree as a plan; partitions.toml
  svcchan/        one partition's service channel on the host's side: qsipart's frames read as hostile, the hello, requests and their answers, pings
  console/        a partition's console port: the whole frame kept, one viewer at a time (crate console-port)
  pio/            physical I/O: what the host uses, each device as seen, physio.toml, what an activation is given, the kernel's PCI files
  dump/           a partition's dump: the ELF core with its notes and header record, written sparsely; the limits of the dumps kept
  partd/          the service: the socket's protocol, the partitions in state, the manager, and the machine behind it; the fuzz target of what a system sends
    src/manager/    the partitions as they are: the requests, activation and the ends of a VMM, the configuration a VMM is given, the service channel, the console, the panel, dumps, physical I/O and drives, entitlement
    src/serve/      the service on Linux: control groups, starting qsvmm, watching it, auto start, taking up what runs, the channel's connections, a console's viewer, histories and dumps on disk, VFIO and binding, the tape relay over SG_IO
stg/            storage
  pool/           a pool's member label, a pool disk's layout, a pool to be made and its limits, what each disk is
  vdisk/          virtual disks: raw files in a pool, thin or thick
  media/          the media library: optical images, tape volumes, imports
  tape/           a tape volume: the AWSTAPE image, a drive's view of it; pass-through's allow-list and relay frames
  poold/          the pool service: pools assembled from their labels, served through ublk, rebuilt; virtual disks, the library, what partitions hold; its socket
net/            virtual Ethernet
  nl/             what QS/400's netlink lacks: VLAN-filtering bridges, port rules and VLANs, the forwarding table, a port's tc filter, taps
  vswitch/        switches as kept: vswitch.toml, uplinks, limits, a port's VLANs, MAC addresses, every error
  vswitchd/       the switch service: switches built from state, uplinks and bonds, ports handed over open, uplink carrier; its socket
qsi/            the QSI host server
  server/         qsiserver: the pins and pairing, each family translated to the services, the events and their log on disk, a console's connection; on Linux the listener, the streams' far ends, the pumps, the local socket; the fuzz target of what a console sends
  local/          qsilocal: the server's local socket, for Service Tools option 4
svc/            Service Tools
  tools/          qsvstools: QS/VS's menu, system status, the partition list, disk units, disk configuration, storage pools, the network's switches and management console pairing over base/panels; option 8's service functions and the problem log; option 10, I/O for partitions; the two sessions; the one panel PID 1 runs it for
  problem/        the problem service: problems, their log, the history read from where it got to; its socket
  hostdump/       the host's dump: the ZIP writer, what is taken and left out, the dumps kept
  update/         code update: the update image and its manifest, verification, installing into the other slot; its socket
docs/           ARCHITECTURE, PHASE0 to PHASE10, PHYSICALIO, PROBLEMS, UPDATE, QS400_GUEST, VMM, PARTD, SERVICE, PANEL, DUMPS, POOLD, VSWITCHD, NETWORK, STORAGE, RESOURCES, PROFILE, QSISERVER, QSICTL, MESSAGES, INSTALL, SERVICETOOLS, REFCODES, SEED, LOCATION, OPEN_QUESTIONS
```

Category folders are created as phases need them. The full set: `tools/`
(host tools), `base/` (shared with QS/MC), `lic/` (PID 1, release,
installer, base services), `vmm/`, `stg/`, `net/`, `qsi/`, `svc/`
(Service Tools). Phase 2 has `tools/`, `base/`, `lic/` and `svc/`;
Phase 3 adds `vmm/`, Phase 4 `part/` (the partition manager), Phase 5
`stg/` (storage), Phase 6 `net/` (virtual Ethernet); Phase 7's crates
are in `part/`; Phase 8 adds `qsi/`; Phase 9's are in `svc/`; Phase
10's are in `part/`, `vmm/` and `stg/`.

## Commands

The pipeline runs on Linux (WSL Ubuntu works); keep the output on ext4
with `QSVS_TARGET_DIR`.

```
cargo run -p build -- doctor          host prerequisites, the license headers, base/ neutral
cargo run -p build -- all             fetch, kernel, userland, initramfs, uki, disk, iso, manifest
cargo run -p build -- all --test-image   the same with the test hooks, under <target>/test-image/
cargo run -p build -- test            Phase 0's suite on the release image; the ten base suites, the fifteen of Phase 2, the ten of Phase 3, the sixteen of Phase 4, Phase 5's, Phase 6's, Phase 7's, Phase 8's, Phase 9's and Phase 10's on the test image; then the fuzz targets, 60 s each
cargo run -p build -- test --suite dhcp  one suite alone (--suite list names them)
cargo run -p build -- test --media iso   boot the installation ISO to the installer's first screen
QSVS_GOLDEN=update cargo run -p build -- test --suite install-serial   write the golden screens instead of comparing
cargo run -p build -- fuzz            build the fuzzer with coverage counters and run every target (--target NAME, --secs N, --jobs N)
cargo run -p build -- update          the update image of the release just built (--test-image)
cargo run -p build -- deny            cargo-deny and the C-linkage check
cargo run -p build -- notices         rewrite lic/release/NOTICES.txt (--check in CI)
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p qsvsinit -p qsvslogd -p qsvsnetd -p qsvsstgd -p qsvspoold -p qsvsvswitchd -p qsvspartd -p qsvstools -p qsvsqsid -p qsvsprobd -p qsvsupdd -p qsvsinstall -p qsvshook -p qsvmm --all-targets --target x86_64-unknown-linux-musl -- -D warnings
cargo build -p qsictl                 the developer's console (on Windows too)
cargo test --workspace
QVS_MESSAGES=update cargo test -p qvsmsg   write the table of docs/MESSAGES.md from the catalogue
QVS_PROBLEMS=update cargo test -p qvsmsg   write the table of docs/PROBLEMS.md from the catalogue
```

Single steps: `fetch`, `kernel`, `userland`, `initramfs`, `uki`, `disk`,
`iso`, `manifest`; each takes `--test-image`. `-v` streams tool output;
logs are always under `target/logs/`.

Environment: `QSVS_TARGET_DIR`, `QSVS_JOBS`, `QSVS_SIGNING_KEY`,
`QSVS_TRUSTED_KEYS`, `QSVS_BUILD_ID`, `QSVS_RELEASE`, `QSVS_OVMF_CODE`,
`QSVS_OVMF_VARS`, `QSVS_QEMU`, `QSVS_NO_KVM`, `QSVS_BOOT_TIMEOUT`,
`QSVS_ALLOW_NO_KVM` (`docs/PHASE0.md` says what each does), and
`QSVS_QS400_IMAGE`, the QS/400 disk image the partition suites IPL
(`docs/PHASE4.md` says how to make it), and `QSVS_QS400_ISO`, QS/400's
installation ISO with a response file, which `pool-install` uploads and
installs from (`docs/PHASE5.md`), and `QSVS_FIXES`, the fixes an
update image names, and `QSVS_UPDATE_IMAGE`, the update image of
another release signed by the same key, which the update suites install
(`docs/PHASE9.md`).
`QSVS_TEST_HOOKS` is the build's own: the userland step sets it, on for
a test image and off for a release image, whatever the caller has.

## Documents

- `docs/ARCHITECTURE.md`: the locked decisions, a sentence each.
- `docs/PHASE0.md`: the toolchain: host prerequisites, the pinned QS/400
  commit, the environment, its acceptance table.
- `docs/PHASE1.md`: the base system: what exists, how to run it, kernel
  additions, the QS/400 crates reused, hashes, the acceptance table.
- `docs/PHASE2.md`: the installer, the mirrored load source and Service
  Tools: what exists, how to run it, the pin, kernel additions, hashes,
  the acceptance table, the checklist for the first real machine, what
  was not done then.
- `docs/PHASE3.md`: the VMM core, and before it the answers to Phase
  2's questions as they were applied: what exists, how to run it, the
  crates added, the acceptance table, what is not done.
- `docs/PHASE4.md`: the partition manager, and before it the new pin:
  what exists, how to run it, what the pin broke, kernel additions,
  hashes, the acceptance table, what remains for a real machine.
- `docs/PHASE5.md`: storage, and before it the new pin and the load
  source's write-intent map: what exists, how to run it, kernel
  additions, hashes, the acceptance table, what remains for a real
  machine.
- `docs/PHASE6.md`: virtual Ethernet: what exists, how to run it,
  kernel additions, what belongs in QS/400's `netlink`, the throughput
  baseline, hashes, the acceptance table, what remains for a real
  machine.
- `docs/PHASE7.md`: the partition service interface, and before it the
  new pin: what exists, how to run it, what the pin broke, hashes, the
  acceptance table, what remains for a real machine.
- `docs/PHASE8.md`: the QSI host server, and before it the new pin and
  the channel TLV: what exists, how to run it, hashes, the acceptance
  table, the joint gate with QS/MC, what remains for a real machine.
- `docs/PHASE9.md`: service, dumps and code update: what exists, how
  to run it, the pin, hashes, the acceptance table, what remains for a
  real machine.
- `docs/PHASE10.md`: physical I/O and tape pass-through: what exists,
  how to run it, the pin and what it broke, kernel additions, the
  IOMMU's cost, hashes, the acceptance table, what remains for a real
  machine.
- `docs/PHYSICALIO.md`: what can be given, the rules, release and
  assignment, reset and fencing, what the guest sees.
- `docs/PROBLEMS.md`: problems: what opens, counts, clears and closes
  one, the table of codes.
- `docs/UPDATE.md`: code update: the image, verification, install,
  activation, rollback, the state copy, `build update`.
- `docs/QSISERVER.md`: the QSI host server: the listener, its limits,
  who may connect, pairing, sessions and the mapping, streams, the event
  log, logging, what each failure does.
- `docs/QSICTL.md`: the developer's console.
- `docs/SERVICE.md`: the service channel on the host's side: states,
  codes, shut down, holding, the console, what happens when either
  side restarts.
- `docs/PANEL.md`: a partition's control panel functions.
- `docs/DUMPS.md`: a partition's dumps and the host's: how they are taken, where they
  are kept, the format and how to read one.
- `docs/NETWORK.md`: switches, uplinks, VLANs, adapters, MAC addresses,
  what a partition may send, what the host exposes, limits, failures.
- `docs/VSWITCHD.md`: the switch service's socket, which QSI's
  virtual Ethernet maps onto.
- `docs/STORAGE.md`: pools, member labels, protection, virtual disks,
  the media library, the tape image format, limits, what happens on
  each failure.
- `docs/POOLD.md`: the pool service's socket, which QSI's storage family
  maps onto.
- `docs/QS400_GUEST.md`: what an unmodified QS/400 does in a partition
  without firmware, what it reads of a partition's identity, and what
  QS/400 would have to change.
- `docs/VMM.md`: the machine: the configuration `qsvmm` is given, the
  memory map, processors, devices, interrupts, ACPI, SMBIOS, exit
  codes, the threat model for guest-controlled input.
- `docs/PARTD.md`: the partition manager: a partition's states, the
  socket's requests and events, activation, auto start, what happens
  when it is started again.
- `docs/RESOURCES.md`: processors and memory: what a processor is,
  installed, configurable and available, pools, what an activation
  gets, the mapping to the kernel's controls, weight.
- `docs/PROFILE.md`: a partition's definition and its profiles.
- `docs/MESSAGES.md`: the `QVS` messages.
- `docs/INSTALL.md`: the operator's guide to installing, attended and
  unattended.
- `docs/SERVICETOOLS.md`: every panel of the local console.
- `docs/REFCODES.md`: the host reference codes, their range and blocks.
- `docs/SEED.md`: `seed.toml`, the `[install]` table and the seed volume.
- `docs/LOCATION.md`: the location codes of the inventory.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- QS/400's `docs/BUILDKIT.md` (in its repository): the pipeline and the
  `Product` description.
