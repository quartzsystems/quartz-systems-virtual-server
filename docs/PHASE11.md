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
- `BLOCKED(phase11)`: not run: `soak-day`, `power-writers`, `limits`,
  `baselines` (and the cost of the hardening), `memory-clean`,
  `svc-update-migrate`, `svc-update-keys`, the Secure Boot suite on the
  release, and the acceptance run of every suite on HEAD from empty
  targets. `vmm-confined` last failed on `sit0`, which `9f11fd5` leaves
  out of the kernel; it has not run since.
- `BLOCKED(phase11)`: the golden screens of the status panel and the
  installer's summary were not written again for the Secure Boot row.
- `NEEDS-CODY`: the Dell's qualification (`docs/QUALIFICATION.md`),
  the fortnight's soak among it; the trademark review of the names; the
  first push of CI; the joint gate with QS/MC.
