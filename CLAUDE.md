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
| **0 (current)** | the toolchain: a clean clone builds a reproducible, bootable image whose Rust PID 1 prints a banner and proves KVM is usable | `docs/PHASE0.md` |
| 1 | the service table, networking, logging and state | |
| 2 | the installer and Service Tools | |
| 3 | the VMM (`qsvmm`) | |

Work that is not done is listed in `docs/PHASE0.md`. Code and documents
mark it `BLOCKED(phase0): reason` (one line) or `NEEDS-CODY` (Cody's
hardware or sign-off). No other marker form.

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
- No IBM text, product names or marks anywhere. The names in use are
  QS/VS, QS/MC and QS/400.
- Anything not done is listed in `docs/PHASE0.md`.

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
  `QSVSDATA` (xfs). Storage pools on other disks.
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
    src/product/    QS/VS as buildkit's Product: names, kernel symbols, initramfs, media
    src/image/      the A/B record on the disk image, the ESP read back
    src/host/       doctor rows, the license header check, `test`
  conformance/    the QEMU harness
    src/qemu/       launch, OVMF, serial capture, the firmware boot entry for slot B
    src/suite/      the Phase 0 steps
lic/            PID 1, release, installer, base services
  qsvsinit/       PID 1
    src/ipl/        the IPL in order, mounts, the command line
    src/console/    /dev/console and /dev/ttyS0
    src/kvm/        the KVM probe and its report
    src/sys/        signals, the panic path
  release/        V1R0M0 over QS/400's vrm, the banner, the prefixes
  refcode/        host reference codes (docs/REFCODES.md)
  hostdisk/       the GPT layout and the A/B record over QS/400's abrec
docs/           ARCHITECTURE, PHASE0, REFCODES, OPEN_QUESTIONS
```

Category folders are created as phases need them. The full set: `tools/`
(host tools), `lic/` (PID 1, release, installer, base services), `vmm/`,
`stg/`, `net/`, `qsi/`, `svc/` (Service Tools). Phase 0 has only `tools/`
and `lic/`.

## Commands

The pipeline runs on Linux (WSL Ubuntu works); keep the output on ext4
with `QSVS_TARGET_DIR`.

```
cargo run -p build -- doctor          host prerequisites and the license headers
cargo run -p build -- all             fetch, kernel, userland, initramfs, uki, disk, iso, manifest
cargo run -p build -- test            boot the disk image under QEMU: slot A, then slot B
cargo run -p build -- test --media iso   boot the installation ISO
cargo run -p build -- deny            cargo-deny and the C-linkage check
cargo run -p build -- notices         rewrite lic/release/NOTICES.txt (--check in CI)
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p qsvsinit --all-targets --target x86_64-unknown-linux-musl -- -D warnings
cargo test --workspace
```

Single steps: `fetch`, `kernel`, `userland`, `initramfs`, `uki`, `disk`,
`iso`, `manifest`. `-v` streams tool output; logs are always under
`target/logs/`.

Environment: `QSVS_TARGET_DIR`, `QSVS_JOBS`, `QSVS_SIGNING_KEY`,
`QSVS_TRUSTED_KEYS`, `QSVS_BUILD_ID`, `QSVS_RELEASE`, `QSVS_OVMF_CODE`,
`QSVS_OVMF_VARS`, `QSVS_QEMU`, `QSVS_NO_KVM`, `QSVS_BOOT_TIMEOUT`,
`QSVS_ALLOW_NO_KVM` (`docs/PHASE0.md` says what each does).

## Documents

- `docs/ARCHITECTURE.md`: the locked decisions, a sentence each.
- `docs/PHASE0.md`: what exists, how to run it, host prerequisites, the
  pinned QS/400 commit, hashes, the acceptance table, what is not done.
- `docs/REFCODES.md`: the host reference codes and their range.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- QS/400's `docs/BUILDKIT.md` (in its repository): the pipeline and the
  `Product` description.
