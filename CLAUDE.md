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

## Phases

| Phase | What it builds | Record |
|---|---|---|
| 0 | the toolchain: a clean clone builds a reproducible, bootable image whose Rust PID 1 prints a banner and proves KVM is usable | `docs/PHASE0.md` |
| **1 (current)** | PID 1 and the base system: the supervised service table, state, logging, the management network and time, the host inventory, the A/B record with slot confirmation, the orderly shutdown | `docs/PHASE1.md` |
| 2 | the installer and Service Tools | |
| 3 | the VMM (`qsvmm`) | |

Work that is not done is listed in the current phase's record,
`docs/PHASE1.md`. Code and documents mark it `BLOCKED(phase1): reason`
(one line) or `NEEDS-CODY` (Cody's hardware or sign-off). No other
marker form.

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
  path dependencies, never copies. Do not depend on QS/400's `console`,
  `svctools` or `licpart` crates: they are QS/400's own.
- Everything under `base/` is product-neutral: QS/MC will pin those
  crates from this repository. A `base/` crate takes an `Appliance`
  description from its caller, has no `qsvs` or `QSVS` in its code
  outside tests and comments, and depends on no crate of `lic/`.
  `build doctor` and a unit test fail otherwise.
- Test hooks are compiled only into a test image (`--test-image`,
  `cfg(test_hooks)`, the `qsvshook` binary). Nothing of them may reach a
  release artifact; the userland and manifest steps check.
- No IBM text, product names or marks anywhere. The names in use are
  QS/VS, QS/MC and QS/400.
- Anything not done is listed in `docs/PHASE1.md`.

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
  from its virtual disk directly.
- **Virtual devices.** virtio over PCI only: scsi, net (vhost-net),
  console, rng.
- **Storage.** Storage pools are xfs file systems; virtual disks are raw
  files; software RAID as on QS/400, presented through a ublk block device.
- **Virtual Ethernet.** VLAN-aware kernel bridges, tap devices with
  vhost-net.
- **Management.** QSI server on TCP 2300 with mutual TLS; the Service
  Tools console locally; nothing else listens.
- **Disk layout.** GPT: ESP (slot A and B UKIs), an A/B record partition,
  `QSVSDATA` (xfs, mounted at `/data`). Storage pools on other disks.
- **The slot rule.** A slot is chosen by firmware boot entry. The A/B
  record (version 2) holds the default slot, the pending slot, the
  attempts left and the release last confirmed in each slot. A pending
  slot confirms when the base services are all ready: the record's
  default and the firmware's `BootOrder` become it.
- **Base crates.** `base/` is product-neutral and shared with QS/MC.
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
    src/host/       doctor rows, the license header check, the neutrality check of base/, `test`
  conformance/    the QEMU harness
    src/qemu/       launch, OVMF, serial capture, firmware variables, image copies, the test hooks' channel
    src/suite/      Phase 0's steps; base/ has the ten suites of Phase 1
base/           what every Quartz appliance needs; product-neutral
  appliance/      the Appliance description, events, civil time, the Ethernet links
  state/          typed TOML state files: schema versions, migration, atomic writes
  supervise/      the service table, restart policy, the runner, the control protocol, signals
  logd/           the NDJSON system log, the reference code history, the kernel log per boot
  netd/           the management interface (DHCP or static, VLAN, carrier, reload) and NTP
  seed/           seed.toml on a labelled FAT volume
lic/            PID 1, release, installer, base services
  qsvsinit/       PID 1
    src/ipl/        the IPL in order: mounts, the command line, the clock, the state step
    src/host/       the running host: the loop, the service table, the control socket, shutdown
    src/slot/       the slot rule: the A/B record at IPL, confirmation, the firmware boot entries
    src/console/    /dev/console and /dev/ttyS0, the status screen
    src/kvm/        the KVM probe and its report
    src/sys/        reaping, the power button, the panic path
    src/hooks.rs    PID 1's test hooks, cfg(test_hooks) only
  qsvslogd/       base/logd run as QS/VS
  qsvsnetd/       base/netd run as QS/VS
  qsvshook/       the test hooks' binary: the harness's channel, a failing service, the state churn
  release/        V1R0M0 over QS/400's vrm, the banner, the prefixes, the Appliance description
  refcode/        host reference codes (docs/REFCODES.md), the code of each base event, the poster
  hostdisk/       the GPT layout, the A/B record over QS/400's abrec, finding the partitions
  hostinv/        the host inventory and the location codes (docs/LOCATION.md)
docs/           ARCHITECTURE, PHASE0, PHASE1, REFCODES, SEED, LOCATION, OPEN_QUESTIONS
```

Category folders are created as phases need them. The full set: `tools/`
(host tools), `base/` (shared with QS/MC), `lic/` (PID 1, release,
installer, base services), `vmm/`, `stg/`, `net/`, `qsi/`, `svc/`
(Service Tools). Phase 1 has `tools/`, `base/` and `lic/`.

## Commands

The pipeline runs on Linux (WSL Ubuntu works); keep the output on ext4
with `QSVS_TARGET_DIR`.

```
cargo run -p build -- doctor          host prerequisites, the license headers, base/ neutral
cargo run -p build -- all             fetch, kernel, userland, initramfs, uki, disk, iso, manifest
cargo run -p build -- all --test-image   the same with the test hooks, under <target>/test-image/
cargo run -p build -- test            Phase 0's suite on the release image, the ten base suites on the test image
cargo run -p build -- test --suite dhcp  one suite alone (--suite list names them)
cargo run -p build -- test --media iso   boot the installation ISO
cargo run -p build -- deny            cargo-deny and the C-linkage check
cargo run -p build -- notices         rewrite lic/release/NOTICES.txt (--check in CI)
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p qsvsinit -p qsvslogd -p qsvsnetd -p qsvshook --all-targets --target x86_64-unknown-linux-musl -- -D warnings
cargo test --workspace
```

Single steps: `fetch`, `kernel`, `userland`, `initramfs`, `uki`, `disk`,
`iso`, `manifest`; each takes `--test-image`. `-v` streams tool output;
logs are always under `target/logs/`.

Environment: `QSVS_TARGET_DIR`, `QSVS_JOBS`, `QSVS_SIGNING_KEY`,
`QSVS_TRUSTED_KEYS`, `QSVS_BUILD_ID`, `QSVS_RELEASE`, `QSVS_OVMF_CODE`,
`QSVS_OVMF_VARS`, `QSVS_QEMU`, `QSVS_NO_KVM`, `QSVS_BOOT_TIMEOUT`,
`QSVS_ALLOW_NO_KVM` (`docs/PHASE0.md` says what each does).
`QSVS_TEST_HOOKS` is the build's own: the userland step sets it, on for
a test image and off for a release image, whatever the caller has.

## Documents

- `docs/ARCHITECTURE.md`: the locked decisions, a sentence each.
- `docs/PHASE0.md`: the toolchain: host prerequisites, the pinned QS/400
  commit, the environment, its acceptance table.
- `docs/PHASE1.md`: the base system: what exists, how to run it, kernel
  additions, the QS/400 crates reused, hashes, the acceptance table,
  what is not done.
- `docs/REFCODES.md`: the host reference codes, their range and blocks.
- `docs/SEED.md`: `seed.toml` and the seed volume.
- `docs/LOCATION.md`: the location codes of the inventory.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- QS/400's `docs/BUILDKIT.md` (in its repository): the pipeline and the
  `Product` description.
