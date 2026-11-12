# Phase 0: toolchain and build system

What Phase 0 built, how to run it, and the evidence for the acceptance
table. QS/VS is x86_64 only.

## The pinned QS/400 commit

Every QS/400 crate is a git dependency on
`https://github.com/quartzsystems/quartz-systems-400` at one commit:

```
6d5532fa80e4328d57b410d4bf19e64baf4a40f6
```

It was `main` on 2026-11-11 and has `tools/buildkit`, `lic/vrm`,
`lic/abrec` and `docs/BUILDKIT.md` (QS/400 handoffs 13 and 13a). The
crates named in `Cargo.toml` are `buildkit`, `vrm`, `diskimg` and
`abrec`; `blockdev`, `xfsfmt` and `optical` come with them. No QS/400
source is in this repository. Three files are the same as QS/400's at
that commit, as the handoff asks: `rust-toolchain.toml`, `kernel/VERSION`
and `kernel/SHA256` (their git blob hashes match). `Cargo.lock` was
started from QS/400's lock file, so the crates the two share are at the
versions QS/400 tests, and cargo cut it down to this workspace's graph.

Moving the pin is one change to the four `rev =` values in `Cargo.toml`,
with the kernel pin if QS/400's moved.

## What exists

- `tools/build/`: the orchestrator, package `build`, binary `qsvs-build`:
  `cargo run -p build -- <command>`. `main.rs` is a clap enum over
  `buildkit`; `src/product/` is the QS/VS `Product`.
  - `fetch`, `kernel`, `userland`, `initramfs`, `uki`, `iso`, `manifest`,
    `deny`, `notices`: `buildkit`'s steps, unchanged.
  - `disk`: `buildkit` writes the GPT, the ESP (`EFI/BOOT/BOOTX64.EFI`
    slot A, `EFI/QSVS/B.EFI` slot B, volume label `QSVS ESP`, volume id
    `0x51535653`) and the empty xfs `QSVSDATA`. The step then writes the
    first A/B record through `abrec` (magic `QSVSREC` padded with a zero,
    QS/VS's own partition type) and reads the image back: the record,
    the ESP's label and volume id, and that the ESP holds exactly the
    two UKIs.
  - `all`: fetch, kernel, userland, initramfs, uki, disk, iso, manifest.
  - `test`: the conformance suite, `--media disk` (default) or `iso`.
  - `doctor`: `buildkit`'s rows, then KVM usable on the build host,
    nested virtualization on, OVMF present (code image and variable
    store), and the license header on every `.rs` file.
- `lic/qsvsinit/`: PID 1. Mounts proc, sysfs, devtmpfs, tmpfs on `/run`
  and cgroup2 on `/sys/fs/cgroup`; posts `REF C2001000` and prints the
  banner (`QS/VS V1R0M0 <build id>`), the slot from the command line and
  the KVM line on `/dev/console` and `/dev/ttyS0`; heartbeats
  `REF C2001005` on serial every 5 s; reaps children on `SIGCHLD`; a
  panic posts `REF B2001000` and parks. On the media UKI it says the
  installer arrives in Phase 2 and otherwise behaves the same. The KVM
  probe uses `kvm-ioctls` and `kvm-bindings`: `/dev/kvm` opens, the API
  version is 12, user memory regions and the in-kernel interrupt
  controller are there, and `KVM_CAP_MAX_VCPUS` gives a count.
- `lic/release/`: `V1R0M0` over QS/400's `vrm`, from `VERSION` and
  `git describe`, honouring `QSVS_RELEASE` and `QSVS_BUILD_ID`; the
  product text, the banner, the prefixes.
- `lic/refcode/`: the host reference codes (`docs/REFCODES.md`).
- `lic/hostdisk/`: the disk layout and the A/B record.
- `tools/conformance/`: the QEMU driver (`-machine q35`, OVMF by pflash,
  serial captured, KVM with `-cpu host` when `/dev/kvm` is usable) and
  the suite.
- `kernel/config/qsvs.config`, `.github/workflows/`, the documents.

### Disk layout of the image

| # | Name | GPT type | Size | Content |
|---|---|---|---|---|
| 1 | `QSVS ESP` | EFI System | 256 MiB | FAT32 with the two UKIs |
| 2 | `QSVS-AB` | `1f072256-9b60-4f79-a93e-f2af75b72775` | 1 MiB | the A/B record, two 64 KiB copies |
| 3 | `QSVSDATA` | `5a62035d-0a21-4e5c-8bd1-e3908ef0dfb9` | the rest (765 MiB of the default 1 GiB) | empty xfs |

Disk and partition GUIDs are fixed (`QSVS` prefix), as are the xfs UUID
and the FAT volume id.

### Kernel configuration

`qsvs.config` is merged over `x86_64_defconfig` after `buildkit`'s
baseline, as QS/400's fragment is. The independent check of the final
`.config` covers **110 symbols**: the baseline's 14 (11 required, 3
forbidden), QS/VS's 84 required and 11 forbidden
(`tools/build/src/product/kernel.rs`), and the command line. Every symbol
the handoff names is among the 84; a unit test
(`every_phase_0_symbol_is_checked`) holds that list. "`IRQ_BYPASS` where
applicable" is `IRQ_BYPASS_MANAGER` in 6.18, which KVM selects.
`docs/OPEN_QUESTIONS.md` item 4 lists what the fragment adds past the
handoff's list and what it turns off.

### The conformance suite

Within 60 s (`QSVS_BOOT_TIMEOUT`) each boot must show, in the order
PID 1 prints: `REF C2001000`, the banner with this repository's release,
where the IPL came from, the `KVM usable` line and, for the first boot
of each media, two heartbeats.

- `test` boots the disk image (slot A), then boots it again as slot B
  and requires `IPL slot B (qsvs.slot=B)`.
- `test --media iso` boots the ISO as a CD-ROM and requires
  `IPL from installation media (qsvs.install=1)`.
- Slot B is chosen by firmware boot entry, as QS/400 chooses its slot
  (`buildkit`'s fixture boots one slot only): the harness writes
  `Boot0000` for `EFI/QSVS/B.EFI` and `BootOrder` into a copy of OVMF's
  variable store (`tools/conformance/src/qemu/vars.rs`).
- With `QSVS_ALLOW_NO_KVM=1` a `KVM not available` line passes with a
  warning; without it, it fails. The serial log is printed on failure.
- Every drive is a QEMU snapshot, so a boot leaves the images as built.

## Host prerequisites (Linux; WSL Ubuntu works)

`cargo run -p build -- doctor` checks them.

| What | Why | Debian/Ubuntu package |
|---|---|---|
| rustup with `x86_64-unknown-linux-musl` | pinned by `rust-toolchain.toml` | `rustup target add x86_64-unknown-linux-musl` |
| clang, lld, llvm (LLVM 21) | `LLVM=1` kernel build; clang links the host tools | `clang lld llvm` |
| make, flex, bison, bc, perl | kernel build | `make flex bison bc perl` |
| libelf headers | objtool | `libelf-dev` |
| tar, xz, gzip | source extraction | `tar xz-utils gzip` |
| qemu-system-x86_64 | boot test | `qemu-system-x86` |
| OVMF, code image and variable store | test-only UEFI firmware | `ovmf` |
| `/dev/kvm` usable, nested virtualization on | the guest is a hypervisor and must see VMX or SVM | `sudo usermod -aG kvm $USER`; `kvm_intel` or `kvm_amd` `nested=1` |
| git | build ID, manifest | `git` |
| cargo-deny | `build deny` | `cargo install cargo-deny --locked` |

OVMF and QEMU are host test tools standing in for a real machine. They
never ship in the image.

On WSL keep the build output on ext4: `export QSVS_TARGET_DIR=$HOME/qsvs-target`.

## Running

```
cargo run -p build -- doctor
cargo run -p build -- all
cargo run -p build -- test
cargo run -p build -- test --media iso
cargo run -p build -- deny
cargo run -p build -- notices --check
```

| Variable | Use |
|---|---|
| `QSVS_TARGET_DIR` | the output root; else `CARGO_TARGET_DIR`, else `target/` |
| `QSVS_JOBS` | caps the kernel's `make -j` |
| `QSVS_SIGNING_KEY` | the ed25519 key (hex seed) the manifest is signed with; else a generated development key under `target/keys/` |
| `QSVS_TRUSTED_KEYS` | a file of further signers' public keys for `/qsvs/keys/system.pub` |
| `QSVS_BUILD_ID`, `QSVS_RELEASE` | the build ID instead of `git describe`; a release other than `VERSION`'s |
| `QSVS_OVMF_CODE`, `QSVS_OVMF_VARS` | the firmware, when not in a distribution path |
| `QSVS_QEMU` | the QEMU binary |
| `QSVS_NO_KVM` | run QEMU without KVM |
| `QSVS_BOOT_TIMEOUT` | seconds a boot may take (60) |
| `QSVS_ALLOW_NO_KVM` | `1` accepts `KVM not available` in `test` and makes the KVM rows of `doctor` warnings |

## Acceptance evidence

Measured on 2026-11-12 in WSL Ubuntu (kernel 6.18.33, 12 threads, Intel,
`kvm_intel` nested = Y), Rust 1.98.0, Ubuntu clang 21.1.8, QEMU 10.2.1,
cargo-deny 0.20.2, in a fresh `git clone` of this repository at commit
`5934f1d` on ext4.

| # | Check | Result |
| --- | --- | --- |
| 1 | From a clean clone, `cargo run -p build -- all` produces the UKIs, disk image, ISO and signed manifest | pass: no environment set, 6 m 34 s from nothing; three UKIs, the disk image, the ISO, `manifest.json` (9 artifacts) and `manifest.sig` |
| 2 | `build test` passes: banner with `V1R0M0`, slot, IPL reference code, heartbeats, `KVM usable` | pass: `REF C2001000` after 1.9 s, `QS/VS V1R0M0 5934f1d`, `IPL slot A (qsvs.slot=A)`, `KVM usable: VMX, API version 12, 1024 vCPUs at most`, two heartbeats; 11.9 s |
| 3 | `build test --media iso` passes with the media command line | pass: `IPL from installation media (qsvs.install=1)`, the same banner, code, KVM line and heartbeats; 12.0 s |
| 4 | Slot B boots and reports itself | pass: by the firmware boot entry, `IPL slot B (qsvs.slot=B)`; 2.3 s |
| 5 | Two clean builds are byte-identical | pass: hashes below |
| 6 | `build deny` passes with `kvm-ioctls` and `kvm-bindings` in the graph; no C anywhere | pass: `cargo deny check`, then 161 crates compiled for the host and for musl with no `links` key and no C build helper; `kvm-ioctls` 0.25.1 and `kvm-bindings` 0.14.2 are in `Cargo.lock` and in `qsvsinit` |
| 7 | `build doctor` passes, including the license-header check over every `.rs` file | pass: all rows ok; 36 `.rs` files start with the header |
| 8 | Every QS/400 crate is a git dependency at one recorded commit; no QS/400 source is copied into this repository | pass: four `rev =` values in `Cargo.toml`, all `6d5532fa`; `deny.toml` allows that one git source and no other |
| 9 | `qsvsinit` is static and stripped; the `.config` check covers every symbol of Section 2 | pass: `buildkit`'s userland check (no `PT_INTERP`, no `DT_NEEDED`, no `.symtab`); `file` says static-pie, stripped, 418040 bytes; 110 symbols checked |

Also checked: `cargo fmt --all --check`; `cargo clippy --workspace
--all-targets -- -D warnings`, and the same for `qsvsinit` on the musl
target; `cargo test --workspace` (33 tests); `build notices --check`.
The image files' hashes were the same after the three boots as before.

The failure path of `test` was seen once, by hand: a guest processor
without VMX or SVM (TCG with `-cpu qemu64,-svm`) prints `KVM not
available: /dev/kvm: No such file or directory (os error 2) (processor
reports neither VMX nor SVM)`; `test` fails and prints the serial log,
and passes with a warning under `QSVS_ALLOW_NO_KVM=1`. QEMU's default
TCG processor offers SVM, so `QSVS_NO_KVM=1` alone still gives `KVM
usable: SVM`.

### Reproducibility

Two builds, each from an empty target directory (nothing kept, the
kernel tarball fetched again), in the same checkout, with:

```
QSVS_BUILD_ID=phase0 QSVS_SIGNING_KEY=<the same key file> cargo run -p build -- all
```

The build ID is otherwise the commit, which every later commit changes,
and the development key is otherwise generated anew in each empty target
directory; its public half is in the initramfs. Every file below had the
same SHA-256 in both builds (`diff` of the two lists):

```
0fba9685d3da1395e0f3582313188e34a575d403a132cf6c4b4ff517436edde6  images/initramfs.cpio                  (419268 bytes)
0fba9685d3da1395e0f3582313188e34a575d403a132cf6c4b4ff517436edde6  images/initramfs-media.cpio            (419268 bytes)
caaec30258ff2dc5ae4c61f427a40a5e485c84da8a10fc0481277a05d32b583c  images/qsvs-V1R0M0-a-caaec30258ff.efi  (12383232 bytes)
b854dcd597097ed2332ab2d70d8a03def90f08928bd57d68e350731104d09609  images/qsvs-V1R0M0-b-b854dcd59709.efi  (12383232 bytes)
9c17d406621cb75732105222e8fd82ffd13f81cc1df8815ad05f4820df33e046  images/qsvs-V1R0M0-d-9c17d406621c.efi  (12383232 bytes)
77da389a24fd12fca8c1206858374bdca78342025dbac3a75139147dc00b7b3b  images/qsvs-V1R0M0-caaec30258ff.img    (1073741824 bytes)
4d6457a619754b1ae94741868e0d2953d220f1415eccd695c90463ab00797ba0  images/QSVS-V1R0M0-9c17d406621c.iso    (44972032 bytes)
068526dc3adc4c77eee994010a455991047754e923a36bed45bc80d9b7d755c4  x86_64-unknown-linux-musl/release/qsvsinit (418040 bytes)
1dbc25d63bbcd1eca45bb453258ce6b1e4951f663a60c732eaa718b85e19d766  kernel/build/.config
11115ec8469bdd57e77437776d097f0eb702cc1662eec34b8c75d4df9abe2358  images/manifest.json
20561c8c688e5aeafb91ad21470dd0befeb33f38e8b10aabb659295318d7642f  images/manifest.sig
```

What the hashes depend on, besides the sources:

- **The signing key.** The initramfs carries its public half, so the
  hashes above are this machine's development key's. With another key
  the UKIs, the disk and the ISO are other bytes, the same on every
  build with that key.
- **The checkout path.** Both builds ran at the same path. `qsvsinit`
  and the kernel's `.config` (the initramfs path) carry it; QS/400's
  pipeline has the same property (`docs/BUILDKIT.md` there, section 7).
  A build at another path was not compared.
- **The commit**, in `manifest.json` alone (`git_rev`), and in the
  banner when `QSVS_BUILD_ID` is not set.

The two initramfs files are the same file: the media UKI differs from
the slots' only in its command line until the installer exists.

Kernel pin: `linux-6.18.55.tar.xz`,
`f410638061a165c12f42ab871d2f3fcd525515359b5faeee80969cff84524df9`, as
QS/400's.

## Not done

This is the list as Phase 0 left it. Phase 1 closed four of its items:
the reference code range is confirmed, PID 1 reads the A/B record and
writes the firmware's boot order, the ISO's name is accepted, and Secure
Boot is deferred to release engineering. What is open now is in
`docs/PHASE1.md`.

- The host reference code range (`C2`, `A2`, `B2`, `D2`, the word
  `REF`) was a proposal. Confirmed with Phase 1. `docs/REFCODES.md`.
- **NEEDS-CODY: the QEMU job in CI** (`.github/workflows/qemu.yml`) is
  written to run on a GitHub-hosted runner and to fail with a clear
  message where the runner has no KVM or no nested virtualization, in
  which case it needs a self-hosted runner. Nothing has been pushed, so
  no workflow has run on GitHub yet: `ci.yml` and `qemu.yml` are
  untested there. `docs/OPEN_QUESTIONS.md` item 6.
- **NEEDS-CODY: real hardware.** Everything above ran under QEMU. The
  image has not been booted on the Dell test machine (PERC H330 in HBA
  mode); the `megaraid_sas` driver and the NIC drivers are configured
  and checked in `.config`, not exercised.
- `BLOCKED(phase0)`: PID 1 does not read the A/B record or write
  firmware boot entries; state is Phase 1. The test chooses slot B from
  the host side. `docs/OPEN_QUESTIONS.md` items 2 and 3.
- `BLOCKED(phase0)`: the ISO is named `QSVS-V1R0M0-<hash>.iso`, not
  `qsvs-...`; the name is `buildkit`'s. `docs/OPEN_QUESTIONS.md` item 5.
- `BLOCKED(phase0)`: the UKIs are not signed for Secure Boot; no decision
  on the key. `docs/OPEN_QUESTIONS.md` item 7.
- Reproducibility across checkout paths was not measured (above).
- The workspace was built and tested on Linux only. On Windows the host
  tools are meant to compile (`qsvsinit` builds a stub there), but that
  was not tried.

Not in Phase 0, and not started: the service table, networking, logging
and state (Phase 1); the installer and Service Tools (Phase 2); any VMM
code beyond the KVM probe (Phase 3).
