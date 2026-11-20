# Phase 11: release engineering and hardening

The record of Phase 11. Phases 0 to 10 are `docs/PHASE0.md` to
`docs/PHASE10.md`; what they say still holds, except where this says
what changed.

Phase 11 adds no function. It moves the pin to QS/400's `main`, explains
the faults of a partition's kernel seen since Phase 3, confines the VMM
and narrows every service's rights, hardens the kernel and the file
systems, signs a release for update and for Secure Boot, builds a
release twice and compares, fuzzes every reader of guest bytes for four
hours, soaks a host, takes it to its limits, keeps baselines, and
writes what an operator and a qualifier read. The version stays 1.0.0
(`V1R0M0`); the tag is Cody's.

## Step 0

### The pin

QS/400 `423c65dea0b8c9fab3d2c3efaba6e9f27e679c99`, its `main` as the
phase began, with its handoff 21: the tape findings of Phase 10 (a blank
volume is one with no labels, the media and save messages as catalogue
entries, a slow request's fault point; `docs/QS400_GUEST.md` T1, T2,
T5). Every `rev =` moved to it in one commit of its own (`1d4236c`),
before anything else of the phase; nothing of QS/VS's had to change for
it. The kernel pin is QS/400's at that commit, unchanged
(`kernel/VERSION` 6.18.55), and `NR_CPUS` was already 512, so question
32 stays closed.

### Every earlier suite on the new pin

In a clone at `1d4236c` with its own target, one suite at a time, 72
suites passed and 6 failed (`mirror-power-pull`, `partition-ipl-b`,
`partition-kill`, `partition-recorded`, `pool-power-pull`, `net-qs400`,
the last by its hour's timeout) before the run was stopped, on Cody's
word, so that the release could be cut; the other 69 were not run. Each
of the six had failed while another suite's QEMU or a build ran beside
it; none was run again alone.

## What exists

### Section 1: the faults of a partition's kernel

`soak-faults` (`tools/conformance/src/suite/soak`) starts partitions
over and over under load, three levels deep, for as long as it is given:
copies of the test image running the soak's load (`qsvshook soak`:
memory filled and checked, processes made and ended, a disk written
and read back, multicast on an internal switch) and copies of QS/400.
Every line of a partition's that names a fault, and every partition
held in Error, is a fault: the partition is dumped, its dump copied out
and read (`soak/core.rs`: the faulting RIP's bytes in the dump against
the kernel image's), and it is started again. The report is a line of
JSON a round, with the faults a thousand IPLs and an hour. The VMM's
test hooks give the controls: `smallpages` (ordinary pages),
`norng` and `noconsole` (a device removed), a profile without
adapters, one processor, `QSVS_NO_IOMMU`; and `holdrestart` holds a
guest that restarts so its memory can be dumped. The same load runs one
level less deep with `qsvmm` started straight in WSL.

<!-- results and the conclusion: below, "The faults explained" -->

### Section 2: the VMM confined

`vmm/qsvmm/src/confine` and `sys.rs` (`docs/SECURITY.md`, "The VMM
confined"): namespaces of its own, an empty read-only root with its
own log directory bound in it, its partition's user with no capability
and no new privileges, then a seccomp filter of every call, which each
kind of thread narrows as it starts. A test image's `confinelearn` hook
reports a refused call instead of ending the VMM; its `try` hook makes a
forbidden call on request. The partition manager posts `A2005190` for a
VMM ended by its filter, and its problem opens (`QVS5155`).

Each service of PID 1's table keeps only the capabilities its row
names, its bounding set dropped and no new privileges set before it is
started (`base/supervise/src/table/caps.rs`); the table and the reason
for each are in `docs/SECURITY.md`, "The services".

### Section 3: the host hardened

- The kernel's hardening, each option with its reason and its cost
  (`docs/SECURITY.md`, "Kernel"), required by the build
  (`tools/build/src/product/kernel.rs`). No modules, no `kexec`, no
  `/dev/mem`, no debug file system, no BPF system call, no user
  namespaces; `dmesg` restricted; an oops is a panic.
- The root is read-only once the services start; `/data` and the pools
  are mounted with no set-user-ID, no device and nothing executable.
- A host panic's last messages are kept by the firmware (`pstore`),
  moved to `/data/log/panics` at the next IPL, posted (`A2001005`,
  its problem `QVS5156`) and in the next host dump.
- QSI's limits, TLS, pairing and request log reviewed against
  `docs/QSISERVER.md` (`docs/SECURITY.md`, "The service interface").
- A partition's huge pages hold nothing of the one before
  (`memory-clean`).

### Section 4: signing

`docs/RELEASING.md`: a release signing key (ed25519) and a Secure Boot
db key; `build release` refuses a development key; an image signed with
one posts `A2001002` at every IPL and says so on the status screen and
in QSI's `identify`. The UKIs are signed with QS/400's `pesign`, the
certificate is at the media's root, and the installer's summary and
Service Tools' status screen say whether Secure Boot is on
(`docs/INSTALL.md`, "Secure Boot").

### Section 5: the release build and the stream

`build release` (`tools/build/src/image/release.rs`), `build keys`,
`build stream add|verify` and `base/stream`, the stream's signed index.
The host fetches nothing (`docs/ARCHITECTURE.md`, Outbound).

### Section 6: update across releases

`svc-update-migrate` (a release whose partition definitions are schema
version 2, updated to and rolled back from) and `svc-update-keys` (a
release trusting a second key, an image signed by it accepted, one
signed by a third refused with `QVS5301`), beside Phase 9's update
suites.

### Section 7: fuzzing

Every target four hours (`build fuzz --all`), the corpora kept in
`tools/fuzz/corpus` within a limit (`tools/fuzz/src/corpus.rs`: 64
files and 128 KiB a target), and four targets added: the seed file, a
pool's member label, the load source's set label, and a console's
pairing.

### Section 8: soaks and power

`soak-day` (a host worked by a script for a day: activations,
restarts, console views, uploads, dumps, entitlement changes,
pairings) and `power-writers` (the power pulled thirty times while
every writer the earlier pulls did not cover is busy).

### Section 9: limits and baselines

`limits` (each limit reached in one host, and one past it refused) and
`baselines` (named only: the numbers below).

### Section 10: what was carried

The watch of a found VMM's control group is an event now
(`part/partd/src/serve/watch.rs`). An adapter's unit numbers having no
gaps stays the rule (`docs/PROFILE.md`); NUMA placement and 1 GiB pages
are left for after V1 (`docs/RESOURCES.md`, the release notes).

### Section 11: documents and audits

`build doctor` checks that the catalogue, `docs/REFCODES.md` and the
problem table agree, that every panel has help, and that no other
vendor's product name is in the tree (`tools/build/src/host/catalogue.rs`,
`wording.rs`). The release carries `catalogue.json` with its schema
(`docs/catalogue.schema.json`). New: `docs/OPERATIONS.md`,
`docs/QUALIFICATION.md`, `docs/SECURITY.md`, `docs/RELEASING.md`,
`docs/RELEASE_NOTES_V1R0M0.md`.

### Section 12: CI

`.github/workflows/nightly.yml`: every suite through `qemu.yml`, the
release built twice with test keys and a refusal, the stream, the
update suites, every fuzz target and an hour's soak. Its runner is one
repository variable (`QSVS_NIGHTLY_RUNS_ON`).

## The release

V1R0M0 was cut on 2026-11-18 from `00848cc`, in a clean clone, with
the release keys Cody keeps (`docs/RELEASING.md`): the release signing
key `a277697cd9a6bade91fd3c7905998a76cfcb42721b149ac12def24822c6ca671`
and the Secure Boot db certificate of SHA-256
`74:22:4E:A1:CD:81:BF:45:91:2A:F1:C0:BF:2E:0F:73:19:B3:87:D5:D4:C0:05:67:1A:B7:3A:37:26:98:CE:34`.
`build release 1.0.0` built it twice from clean and found its 12 files
the same byte for byte (1505 s for both); `SHA256SUMS` checks, and the
stream's index verifies. Its ISO is `QSVS-V1R0M0-b7b2765f0e44.iso`, its
update image `QSVS-V1R0M0-update-824eb048f9b4.upd`. A tag `v1.0.0`
publishes it from `.github/workflows/release.yml` with the same keys as
the `release` environment's secrets.

Before it, with test keys at `82f1d19`: `build release` refused no
keys, no Secure Boot key and another version; built twice the same;
the stream refused an index signed by a key it did not trust and one
with a digit of a sum changed.

## Not done

The release was cut before the phase's long runs ended, on Cody's word.
What they would have shown is not shown:

- `BLOCKED(phase11)`: Section 1 has no conclusion. One depth-2 soak
  (`qsvmm` in WSL, ordinary pages, 69 IPLs) had one fault: three words
  of two processes' memory, each at offset 0xf80 of its page with bit
  20 set, as a write through a stale translation would leave; WSL had
  migrated 65 million pages by then. The control with huge pages and
  the depth-3 soak with its controls were stopped. QEMU at the same
  depth was not run: no host here has QEMU in it.
- `BLOCKED(phase11)`: the fuzz targets ran about 3.5 hours of 24 (four
  at a time), with no finding; the corpus was not saved.
- `BLOCKED(phase11)`: not run: `soak-day`, `baselines` (and the cost
  of the hardening), the Secure Boot suite on the release, and the
  acceptance run of every suite on HEAD from empty targets.
  `power-writers`, `limits`, `memory-clean`, `svc-update-migrate`,
  `svc-update-keys` and `vmm-confined` have passed since, and the
  golden screens have the Secure Boot row (below, 2026-11-19).
- `NEEDS-CODY`: the Dell's qualification (`docs/QUALIFICATION.md`),
  the fortnight's soak among it; the trademark review of the names; the
  first push of CI; the joint gate with QS/MC.

## 2026-11-19: the base follow-ups for QS/MC

After V1R0M0, not part of it: what QS/MC's `docs/OPEN_QUESTIONS.md`
asked of QS/VS's `base/` (its items 3, 4, 5, 11 and 12), at the same
QS/400 pin.

### What moved

- The processor check is `base/processor` (`9e35754`): `NEEDED`,
  `lacking`, `lacking_from`. PID 1, the QSI server and Service Tools
  keep their words; `qsi/local` re-exports it.
- The QEMU driver is `base/qemu` (`650a6b7`): launch, OVMF and the
  variable store, the serial port and its waits, the monitor, disks,
  NICs, removable volumes, power pulls. Every environment variable's
  name is its caller's; QS/VS's harness keeps the `QSVS_` ones in
  `tools/conformance/src/qemu`. An ISO is booted from a virtio-scsi
  CD-ROM, not q35's IDE one, which hung in OVMF on a share of starts;
  that is not known to be fixed for a real drive
  (`docs/QUALIFICATION.md`, section 3).
- The neutral part of the inventory is `base/inventory` (`267c0bd`):
  processors and their threads, memory, disks, interfaces, PCI
  devices' identities, DMI, the location codes. `lic/hostinv` keeps a
  hypervisor's fields: isolation groups, resets and interrupts, the
  IOMMU, huge pages, tape drives. The installer's disk choices and the
  console's link choices are made from its lists.
- `base/netd` manages a list of interfaces, each with a role the
  appliance names (`bd9a4ea`). `network.toml` is version 2 (an
  `[[interface]]` table each, with `carries_route`, `carries_dns` and
  `carries_time`); version 1 migrates to one interface with nothing
  else changed, and a seed's `[network]` is read in either form. QS/VS
  names one role, `management`. A reload or a change on trial remakes
  only the interfaces that changed. Nothing new listens.
- `base/netd` has a second way to apply a change, for a console
  reached over the network it changes (QS/MC's item 24):
  `apply_confirmed` keeps it only when `confirm` is called within its
  window, and takes it back when the window ends whatever carrier and
  address say. `apply`, which keeps a change once its interfaces have
  carrier and an address, is unchanged, and is what QS/VS's local
  console uses. A unit test drives a static change that works but is
  never confirmed to its window's end, where it is taken back and
  nothing is written, and one confirmed within it, which is written
  (`manage/tests.rs`).
- The neutrality check covers the new crates, and a `base/` crate may
  depend on no crate of this repository outside `base/` (it said
  `lic/` before). `build fixture` builds QSDEMO (`tools/fixture`), an
  appliance outside the workspace, from `processor`, `qemu`,
  `inventory`, `netd` and `state` as git dependencies at HEAD; at
  `992e100` it built with 7 base crates and none outside `base/`, and
  each of its parts held.

### debugfs

V1R0M0's kernel has debugfs: its `.config` has `CONFIG_DEBUG_FS=y`
though the fragment turns it off, because `x86_64_defconfig` turns on
`BLK_DEV_IO_TRACE`, which selects it, and the build's check did not
list it. Of the sixteen symbols that select `DEBUG_FS`, that was the
only one on. The fragment turns it off now (`20cf33d`), and the check
lists every symbol the fragment turns off, so one a default selects
back on fails the build. Against V1R0M0's `.config`, the symbols the
fragment turns off that were on: `DEBUG_FS` alone. In an older target
of the phase, `IPV6_SIT` too, which `9f11fd5` had already left out. A
test image's kernel keeps debugfs, by its own fragment
(`kernel.rs`, `TEST_ON`). The kernel built since has neither symbol.

### What running every suite found

Phase 11 cut V1R0M0 without running every suite on its HEAD. Running
them found these, each fixed here:

- The root is read-only, so the resolver's file could not be written:
  `/etc` is a tmpfs (`cb5b973`; `dhcp`, `installed-base`).
- The VMM's seccomp filter refused vhost-net's `VHOST_SET_OWNER` and
  `VHOST_SET_MEM_TABLE`, and a worker thread's kick: every partition
  with an adapter ended with `A2005190` (`fd2761e`; the network
  suites, `pool-full`).
- The golden screens lacked the Secure Boot row (`1e0727a`).
- Service Tools had lost `SYS_ADMIN`, so option 8 could not mount a
  removable volume, for an update image or a dump (`4154293`;
  `svc-tools`).
- A partition at its limits could not be activated, by three faults
  of `partd` and the pool service: a hold's files went on one byte,
  which carries 16 (`0f1f8a5`); a VMM given more than 61 files had
  its control group's file closed under it, so it started outside its
  group or not at all (`036568a`); and an adapter's virtual disks were
  kept one to a slot, so a profile with two pool disks on one adapter
  gave the VMM the wrong files (`4481b44`). `limits` passes.
- The suites: `power-writers` waited for "Base ready" twice
  (`602e5fb`); `qsi-events` took the last event from a link that had
  seen none (`c83338e`); the tape suites took QS/400's messages from
  before the pin, and its next-volume inquiries `CPA4088` and
  `CPA4089` were not answered from the reply list (`ed04605`,
  `e45d0c7`, `94d742e`).

All 154 suites of Phases 0 to 11 ran, each alone, on images of this
work. Every one has passed but `pio-tape-span` and `pio-tape-volumes`,
not on one build: those that failed were run again on the next.
`net-tagged`, `net-counters`, `service-console`, `memory-clean` and
`qsi-hostile` each failed once and passed when run again:
`net-tagged` and `service-console` by a partition's kernel panic
(Section 1), `memory-clean` by an activation slower than the hook
channel's 30 s, `net-counters` (a tap gone) and `qsi-hostile` (a TLS
connection ended) with no cause found. The acceptance run on
one build, from empty targets, is the next pin's.

### Not done here

- `BLOCKED(phase11)`: `pio-tape-span` (its next-volume reply, fixed
  in the suite, not yet run) and `pio-tape-volumes` (QS/400's console
  lost the session or a typed line in the middle of the suite).
- `BLOCKED(phase11)`: QS/MC's newer asks of QS/VS, after this work
  began: a frame capture of the user network's NIC in `base/qemu` and
  `cpu` honoured without KVM (its item 4), a write-intent map for a
  member absent at assembly (its item 13, QS/400's `protect` first),
  and `base/stgd` looking for the load-source disks once a start (its
  item 14).

## 2026-11-19: the pin at QS/400's handoff 22

### The pin

QS/400 `155aba213638b178cb730cff4a4113ef1d8fb525`, its `main` of
2026-11-19, with its handoff 22 (`2f9a3c3`): its web layer made
neutral, and `x509kit`, the certificates without the store. Every
`rev =` moved to it in one commit of its own (`982cc83`). Of the
crates QS/VS takes, `certs` changed (it takes `x509kit` now) and
`tls` under it; nothing else, and the kernel pin is the same
(`kernel/VERSION` 6.18.55). `x509kit` is pure Rust: `build deny`
passes.

What the pin broke here: `lic/release/NOTICES.txt` did not list
`x509kit` (`4629dbf`). And `build fixture` found this repository's
crates by the name of its folder, so it failed in a clone of another
name; it finds them by the repository's URL now (`2fe7583`). Nothing
had its cause in QS/400.

### From empty targets

In a clean clone at `2fe7583` on ext4 in WSL, every target empty, the
signing key of the reproducibility runs, QS/400's guest image and ISO
built at the same commit (`qs400-V1R1M0-434e2d0f2249.img`,
`QS400-V1R1M0-19dc6f9c6a83.iso` with `pool-install.rsp`, 35 minutes),
the update images of the next release built from the clone. Host:
rustc 1.98.0, clang 21.1.8, QEMU 10.2.1, cargo-deny 0.20.2, kernel
6.18.33.2-microsoft-standard-WSL2, 12 processors. QS/MC's suites ran
in the same WSL at the same time.

`build doctor`, `build deny`, `cargo fmt --check`, both clippy runs,
`cargo test --workspace`, `build notices --check` and `build fixture`
passed. Of the 154 suites of Phases 0 to 11, each run alone, 142
passed and 12 failed. Run again alone on images of `bb85ac5`, with
four network suites for `base/netd`'s change (`dhcp`, `static-vlan`,
`tools-network`, `net-management`), 15 of 16 passed:

| Suite | First run | Again |
|---|---|---|
| `installed-base` | the inventory read before the link had carrier | pass |
| `svc-update` | QS/400 held in storage management recovery | pass |
| `partition-attributes` | QS/400's storage service panicked after its IPL (`UserProfile has no field Magic`), its code the same at both pins | pass |
| `net-host-unreachable`, `net-vmm-kill`, `service-dumps`, `memory-clean` | a partition's kernel faulted (Section 1) | pass |
| `qsi-families` | QS/400 did not reach its IPL complete | pass |
| `qsi-power` | QS/400's storage service ended (`B1001060`) | pass |
| `qsi-qsictl` | `qsictl` was not built by the run | pass |
| `pio-tape-volumes` | QS/400's `CPF3770` on an ASCII-labelled volume, which the suite did not list (`bb85ac5`) | pass |
| `service-hostile` | the hostile system's kernel faulted | its PID 1 faulted (SIGSEGV) |

So every suite has passed at the pin but `service-hostile`, which
failed twice in the hostile system itself, by two faults unlike each
other: Section 1's, which these runs met far more often than the
morning's. `pio-tape-span` and `pio-tape-save` passed at the pin with
the suites' fixes of the morning.

### The release

`build release 1.0.0` in a clean clone at `bb85ac5`, with the test keys
(the release signing key of a test set, trusting its second key; its
Secure Boot db certificate of SHA-256
`D9:64:31:39:01:C7:A3:28:E7:1F:07:16:F6:A0:7E:48:DD:A3:1F:99:9C:C4:3D:B9:F6:A0:49:E3:08:7B:03:F3`),
built it twice from empty and found its 12 files the same byte for
byte (1498 s for both). `SHA256SUMS` checks. The kernel's `.config`
has neither `DEBUG_FS` nor `BLK_DEV_IO_TRACE`, and no other symbol the
fragment turns off.

| File | SHA-256 |
|---|---|
| `MANIFEST.json` | `f6b048e385fce9c0796c8161d2b964b929d4a49fdc1200e6d749208c53e03171` |
| `MANIFEST.sig` | `5252f8f0fbb0beba362dd7057a03764ca0f3657ec29649f1355e2ceafdc4b1de` |
| `NOTICES.txt` | `37334a6b79aac22a12dd700c7a04e4d2f5ae1958bab12c814a3732394604efaf` |
| `QSVS-V1R0M0-91ca538bb761.iso` | `819790f97d470bf0dc12fc103550e4f62194751dba203c20879dabaf524f4b65` |
| `QSVS-V1R0M0-update-527020370e51.upd` | `527020370e5191bd12c35fc2b8509fb9fb5ae24ea0f25b4737975d46972578ab` |
| `RELEASE_NOTES_V1R0M0.md` | `dc4f4195f4cf3152cb08016d4323a9444aab30b36629fc77554e4b932b305461` |
| `catalogue.json` | `19f70527ef6bdc8ac2fc1da781abe53eda0a8311b5771bd677bb48f9c73bb9bb` |
| `catalogue.schema.json` | `21584e8d75a2fdb60d264b08742726cc70e635b8a9d5e3e3fef817350c33c138` |
| `qsvs-db.cer` | `d964313901c7a328e71f0716f6a07e48dda31f999cc43db9f6a049e3087b03f3` |
| `stream/index.toml` | `0a3da8988c19247884654bc8dd09d19da9e7f76a6ef841078635f86e9175f28f` |

Why they are new against V1R0M0's: the code differs (the base
follow-ups of the morning, the kernel without debugfs, the fixes the
suites found, `base/netd`'s confirmed change, the pin and `x509kit` in
the notices), and these are signed with test keys, not the release
keys Cody keeps, so every signed file and the ISO differ from what
those would give. A release of this code is cut with them
(`docs/RELEASING.md`).

### Not done

- `service-hostile` had not passed at this pin, and the suites were
  not run on one build from empty: both are answered at the pin of
  handoff 23 (below).
- `BLOCKED(phase11)`: QS/MC's newer asks of QS/VS, its items 4, 13
  and 14 (above).

## 2026-11-20: the pin at QS/400's handoff 23

QS/400 `cb58b010a8b96ac48a66e06079e1ff4e4af371c7`, its `main` of
2026-11-20 with its handoff 23 (`cb58b01`), seven commits past
`155aba2`: `termlook` (a terminal's look, its screen and script) and
`srcdata` (the reference codes as data) made neutral, and its fixture
given a terminal page. The pin was at `3fba6cf` (`a3ab02d`) for the
morning, before the handoff's documents landed. QS/VS takes neither.
No crate QS/VS takes changed in its source; under `buildkit` only its own fixture and a
test did. The kernel pin is the same. Every `rev =` moved in one
commit each time; `Cargo.lock` changed in the revs only.

### From empty targets

In a clean clone at `3feacb0` (the pin) on ext4 in WSL, every target
empty, the signing key of the reproducibility runs, QS/400's guest
image and ISO built at `cb58b01` (`qs400-V1R1M0-57ac22978d4d.img`,
`QS400-V1R1M0-6097ac178e7b.iso` with `pool-install.rsp`, 36 minutes),
the update images of the next release built from the clone. Host:
rustc 1.98.0, clang 21.1.8, QEMU 10.2.1, cargo-deny 0.20.2, kernel
6.18.33.2-microsoft-standard-WSL2, 12 processors.

`build doctor`, `build deny`, `cargo fmt --check`, both clippy runs,
`cargo test --workspace`, `build notices --check` and `build fixture`
passed. Of the 154 suites, each run alone, 147 passed and 7 failed;
run again alone on the same build, 6 of the 7 passed:

| Suite | First run | Again |
|---|---|---|
| `partition-attributes` | QS/400's `IPL Options` never came | pass |
| `partition-recorded` | the partition's processor triple faulted (`A2005170`) | pass |
| `service-states` | a partition's kernel faulted (Section 1) | pass |
| `service-dumps` | QS/400's storage service ended (`B1001060`) | pass |
| `qsi-hostile` | TLS ended early while WSL refused new sessions under load | pass |
| `memory-clean` | `partd`'s channel answered `EAGAIN` under the same load | pass |
| `qsi-two-consoles` | the second console closed (`QSI0107`) | the same |

`qsi-two-consoles` waited on the first console for the partition to
run and never read the second, which the host closes when it hears
nothing for 45 s (`qsisess`'s `timeout_ms`, the same at both pins): a
QS/400 IPL longer than that failed it. It reads both now (`ee1b077`)
and passed three times of three on the same build. `service-hostile`,
which failed twice at the last pin, passed.

So every suite has passed at the pin, `qsi-two-consoles` with the
suite's fix.

### The release

`build release 1.0.0` in a clean clone at `3feacb0`, with the test
keys (the release signing key of a test set, trusting its second
key; its Secure Boot db certificate of SHA-256
`D9:64:31:39:01:C7:A3:28:E7:1F:07:16:F6:A0:7E:48:DD:A3:1F:99:9C:C4:3D:B9:F6:A0:49:E3:08:7B:03:F3`),
built it twice from empty and found its 12 files the same byte for
byte (1040 s for both). `SHA256SUMS` checks. The kernel's `.config`
has neither `DEBUG_FS` nor `BLK_DEV_IO_TRACE`, and no other symbol the
fragment turns off.

| File | SHA-256 |
|---|---|
| `MANIFEST.json` | `d09edc72d3e4d7f156cd56adde1274dbcb75ce141122fac46e603151cd5960c7` |
| `MANIFEST.sig` | `1818cdfb8fb2698931a411998ac55291fa3ac59b5a087f75a364ce3946ac618d` |
| `NOTICES.txt` | `37334a6b79aac22a12dd700c7a04e4d2f5ae1958bab12c814a3732394604efaf` |
| `QSVS-V1R0M0-26b2658a513e.iso` | `c9d5a0ef7dd7cb83c5a6c24360e6bbb16438634a3377487c57c0d2d2e4d1edb0` |
| `QSVS-V1R0M0-update-71dffc944984.upd` | `71dffc94498444694265c7ef96bdb64cf4c5fcecdcd0b5ea0a8a4380cfb533e4` |
| `RELEASE_NOTES_V1R0M0.md` | `dc4f4195f4cf3152cb08016d4323a9444aab30b36629fc77554e4b932b305461` |
| `catalogue.json` | `0ba154343884a7f10bb4c20008d33a64687a76acdb223bcd3212b75abf6dd794` |
| `catalogue.schema.json` | `21584e8d75a2fdb60d264b08742726cc70e635b8a9d5e3e3fef817350c33c138` |
| `qsvs-db.cer` | `d964313901c7a328e71f0716f6a07e48dda31f999cc43db9f6a049e3087b03f3` |
| `stream/index.toml` | `a5a1ee1b5860efb6959d144d0b71a9a3a70b348da3bda5b2af03448e489033b3` |

Against handoff 22's: the notices, the release notes, the schema and
the certificate are the same. The rest differ by the commit: the
build ID is in the catalogue, the manifest and the images, and the
code differs by the pin and the network change kept only when
confirmed (`ff181be`). `ee1b077` changes a suite only, nothing a
release holds.

### Not done

Nothing new at this pin: QS/MC's items 4, 13 and 14 stay open
(above).
