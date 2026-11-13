# QS/VS (Quartz Systems/Virtual Server)

A hypervisor appliance whose only job is to run QS/400 instances as
partitions. It is built the way QS/400 is built: the Linux kernel is the
only inherited component, and everything else is Rust. It has no web UI,
no shell and no remote management of its own. A separate appliance, QS/MC
(Quartz Systems/Management Console), manages it over a Quartz protocol
called QSI, and a small Service Tools console on the local display and
serial port covers host basics, networking and disks.

The build pipeline is QS/400's, taken as a library: `buildkit`, `vrm`,
`diskimg` and `abrec` are git dependencies on
`https://github.com/quartzsystems/quartz-systems-400`, all at one commit.
So are what the installer and the local console are made of: `console`,
`scrkit`, `svcusers`, `protect`, `ublk`, `blockdev`. One crate that is
QS/400's own is taken too, at the same commit: `licpart`, for the
guest's LIC record, by the IPL loader alone.

## Phases

| Phase | What it builds | Record |
|---|---|---|
| 0 | Toolchain and build: a clean clone builds a reproducible, bootable image whose Rust PID 1 prints a banner and proves KVM is usable | `docs/PHASE0.md` |
| 1 | PID 1 and base system: the supervised service table, state, logging, the management network and time, the host inventory, the A/B record with slot confirmation, the orderly shutdown | `docs/PHASE1.md` |
| 2 | Installer, ISO and Service Tools: the installer and its ISO, the mirrored load source with its storage service, the Service Tools console | `docs/PHASE2.md` |
| **3 (current)** | VMM core: `qsvmm`, and one QS/400 system taken from a disk image to a completed IPL in a partition | `docs/PHASE3.md` |
| 4 | Partition manager, profiles, processors and memory | |
| 5 | Virtual storage, optical and tape | |
| 6 | Virtual Ethernet | |
| 7 | Partition service interface | |
| 8 | QSI host server and QS/MC integration | |
| 9 | Service, dumps and code update | |
| 10 | Physical I/O and tape pass-through | |
| 11 | Release engineering and hardening | |

Work that is not done is listed in the current phase's record,
`docs/PHASE3.md`. Code and documents mark it `BLOCKED(phase3): reason`
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
- `qsvmm` opens what it needs as root and then runs as an unprivileged
  user with no capability; it reports reference codes on standard
  error and whoever started it posts them. Nothing of a release image
  starts it before Phase 4: only the test hooks' partition service does.
- Test hooks are compiled only into a test image (`--test-image`,
  `cfg(test_hooks)`, the `qsvshook` binary). Nothing of them may reach a
  release artifact; the userland and manifest steps check.
- No IBM text, product names or marks anywhere. The names in use are
  QS/VS, QS/MC and QS/400.
- Anything not done is listed in `docs/PHASE3.md`.

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
  from its virtual disk directly. Seven bytes at the reset vector end
  the partition when its guest restarts by a jump there; nothing of
  the VMM's runs before the guest's kernel.
- **Virtual devices.** virtio over PCI only: scsi, net (vhost-net),
  console, rng.
- **Storage.** Storage pools are xfs file systems; virtual disks are raw
  files; software RAID as on QS/400, presented through a ublk block device.
- **Virtual Ethernet.** VLAN-aware kernel bridges, tap devices with
  vhost-net.
- **Management.** QSI server on TCP 2300 with mutual TLS; the Service
  Tools console locally; nothing else listens.
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
- **Installation.** The media UKI runs the installer in place of the
  service table. The ISO carries the slot UKIs and a manifest of them
  signed by the build's key; the installer verifies it before writing.
  Attended on the display and serial port at once, or unattended from
  an `[install]` table of a seed file.
- **Local console.** Service Tools: sign-on with a service tools user
  ID, a session each for the display and the serial port. The serial
  port is PID 1's, in lines, while nobody is signed on there. Options
  4 and 8 of its menu come with Phase 8 and Phase 9.
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
  build/          the orchestrator, a thin binary over buildkit: cargo run -p build -- <cmd>
    src/product/    QS/VS as buildkit's Product: names, kernel symbols, initramfs, media; the two image kinds
    src/image/      the userland step and the hooks check, the A/B record on the disk image, the manifest's kind
    src/host/       doctor rows, the license header check, the neutrality check of base/, `test`, `fuzz`
  conformance/    the QEMU harness
    src/qemu/       launch, OVMF, serial capture, the serial port as a 24x80 grid, typed keys, golden files, firmware variables, image copies, the test hooks' channel
    src/suite/      Phase 0's steps; base/ has the ten suites of Phase 1; install/ the fifteen of Phase 2 and of the answers to its questions; partition/ the ten of Phase 3
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
  loadsource/     the load-source disks: the GPT layout, the A/B record and the set's label on every disk, units that ran apart, the boot entries, the media's payload
  stgd/           the storage service: the mirrored load source assembled, served through ublk, resynchronized, rebuilt; its socket
lic/            PID 1, release, installer, base services
  qsvsinit/       PID 1
    src/ipl/        the IPL in order: mounts, the command line, the storage step and the unit to keep, the clock, the state step; the hand-off to the installer
    src/host/       the running host: the loop, the service table, the control socket, shutdown
    src/slot/       the slot rule: the A/B record at IPL, confirmation, the firmware boot entries
    src/console/    /dev/console and /dev/ttyS0, the status screen
    src/kvm/        the KVM probe and its report
    src/sys/        reaping, the power button, the panic path
    src/hooks.rs    PID 1's test hooks, cfg(test_hooks) only
  qsvslogd/       base/logd run as QS/VS
  qsvsnetd/       base/netd run as QS/VS
  qsvsstgd/       base/stgd run as QS/VS, with QS/VS's inventory
  qsvsinstall/    the installer: base/install over QS/VS's media, disks and state
  qsvshook/       the test hooks' binary: the harness's channel, a failing service, the state churn
    src/partition/  the one partition a test image starts: the runner, and the probe of requests no guest sends
  release/        V1R0M0 over QS/400's vrm, the banner, the prefixes, the Appliance description, QS/VS's load-source names
  refcode/        host reference codes (docs/REFCODES.md), the code of each base event, the poster
  hostinv/        the host inventory and the location codes (docs/LOCATION.md)
vmm/            the VMM: one process for one partition
  arch/           x86_64 boot state: the memory map, page tables and GDT, registers, the zero page, the reset vector; CPUID and MSR policy, the local APIC
  acpi/           the tables in place of firmware: RSDP, XSDT, FADT, MADT, MCFG, the DSDT
  devices/        the I/O port bus; the 16550, the CMOS clock, the reset line, the power-off register; the PCI bus, configuration space, MSI-X; virtio-pci, virtio-scsi, virtio-rng
    src/virtio/scsi/  the command decoder, a disk LUN, the direct-I/O backing store
  ipl/            the IPL loader: bounded GPT and FAT readers, the LIC record through licpart, the bzImage into guest memory
  qsvmm/          the binary: the profile, the exit codes, the machine on KVM, the processors' loop, the serial log and console socket, the privilege drop
svc/            Service Tools
  tools/          qsvstools: QS/VS's menu, system status and disk units over base/panels; the two sessions; the one panel PID 1 runs it for
docs/           ARCHITECTURE, PHASE0, PHASE1, PHASE2, PHASE3, QS400_GUEST, VMM, PROFILE, INSTALL, SERVICETOOLS, REFCODES, SEED, LOCATION, OPEN_QUESTIONS
```

Category folders are created as phases need them. The full set: `tools/`
(host tools), `base/` (shared with QS/MC), `lic/` (PID 1, release,
installer, base services), `vmm/`, `stg/`, `net/`, `qsi/`, `svc/`
(Service Tools). Phase 2 has `tools/`, `base/`, `lic/` and `svc/`;
Phase 3 adds `vmm/`.

## Commands

The pipeline runs on Linux (WSL Ubuntu works); keep the output on ext4
with `QSVS_TARGET_DIR`.

```
cargo run -p build -- doctor          host prerequisites, the license headers, base/ neutral
cargo run -p build -- all             fetch, kernel, userland, initramfs, uki, disk, iso, manifest
cargo run -p build -- all --test-image   the same with the test hooks, under <target>/test-image/
cargo run -p build -- test            Phase 0's suite on the release image; the ten base suites, the fifteen of Phase 2 and the ten of Phase 3 on the test image; then the fuzz targets, 60 s each
cargo run -p build -- test --suite dhcp  one suite alone (--suite list names them)
cargo run -p build -- test --media iso   boot the installation ISO to the installer's first screen
QSVS_GOLDEN=update cargo run -p build -- test --suite install-serial   write the golden screens instead of comparing
cargo run -p build -- fuzz            build the fuzzer with coverage counters and run every target (--target NAME, --secs N, --jobs N)
cargo run -p build -- deny            cargo-deny and the C-linkage check
cargo run -p build -- notices         rewrite lic/release/NOTICES.txt (--check in CI)
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p qsvsinit -p qsvslogd -p qsvsnetd -p qsvsstgd -p qsvstools -p qsvsinstall -p qsvshook -p qsvmm --all-targets --target x86_64-unknown-linux-musl -- -D warnings
cargo test --workspace
```

Single steps: `fetch`, `kernel`, `userland`, `initramfs`, `uki`, `disk`,
`iso`, `manifest`; each takes `--test-image`. `-v` streams tool output;
logs are always under `target/logs/`.

Environment: `QSVS_TARGET_DIR`, `QSVS_JOBS`, `QSVS_SIGNING_KEY`,
`QSVS_TRUSTED_KEYS`, `QSVS_BUILD_ID`, `QSVS_RELEASE`, `QSVS_OVMF_CODE`,
`QSVS_OVMF_VARS`, `QSVS_QEMU`, `QSVS_NO_KVM`, `QSVS_BOOT_TIMEOUT`,
`QSVS_ALLOW_NO_KVM` (`docs/PHASE0.md` says what each does), and
`QSVS_QS400_IMAGE`, the QS/400 disk image the partition suites IPL
(`docs/PHASE3.md` says how to make it).
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
- `docs/QS400_GUEST.md`: what an unmodified QS/400 does in a partition
  without firmware, and what QS/400 would have to change.
- `docs/VMM.md`: the machine: memory map, devices, interrupts, ACPI,
  exit codes, the threat model for guest-controlled input.
- `docs/PROFILE.md`: a partition's profile, provisional.
- `docs/INSTALL.md`: the operator's guide to installing, attended and
  unattended.
- `docs/SERVICETOOLS.md`: every panel of the local console.
- `docs/REFCODES.md`: the host reference codes, their range and blocks.
- `docs/SEED.md`: `seed.toml`, the `[install]` table and the seed volume.
- `docs/LOCATION.md`: the location codes of the inventory.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- QS/400's `docs/BUILDKIT.md` (in its repository): the pipeline and the
  `Product` description.
