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
