# Phase 3: the VMM core

The record of Phase 3. Phases 0 to 2 are `docs/PHASE0.md`,
`docs/PHASE1.md` and `docs/PHASE2.md`; what they say of the host still
holds, except where the first section below says what changed.

Phase 3 writes the virtual machine monitor, `qsvmm`, and takes one
QS/400 system from a disk image to a completed IPL inside a partition.

## Before the VMM: the answers to Phase 2's questions

The Phase 3 handoff answered the questions Phase 2 left open
(`docs/OPEN_QUESTIONS.md`, 16 to 22). Three of the answers changed
code, and were applied before anything of the VMM was written.

### 18: the load source is every appliance's

QS/MC gets the same mirrored load source, so the layout and the
storage service's engine are in `base/` now, and product-neutral.

| Was | Is |
|---|---|
| `lic/hostdisk` (crate `hostdisk`) | `base/loadsource` (crate `loadsource`): `layout`, `gpt`, `disks`, `multi`, `record`, `set`, `boot`, `prepare`, `payload`, `sys` |
| `lic/qsvsstgd`, library half (the socket's requests) | `base/stgd` (crate `stgd`) |
| `lic/qsvsstgd/src/serve` (the engine) | `base/stgd/src/serve` |
| `lic/qsvsstgd`, the binary | `lic/qsvsstgd`: thirty lines that run `stgd::serve::run` as QS/VS |

What is an appliance's own comes from its `Appliance` description, in
a new part of it, `appliance::LoadSourceNames`: the GPT types of the
record and data partitions, the xfs UUID of a built image's data file
system, the GPT names of the three partitions (the data partition's is
its xfs label too), the vendor name (the ESP's directory, the
payload's directory on the media and the first word of its manifest),
slot B's path, the ESP's FAT label and volume id, the seed of a built
image's GUIDs, and four magics: the A/B record's, the set label's, the
resume map's record and the resume map's own. `Appliance` has one new
field, `load_source`, and one new method, `storage_file`, for the
storage service's files in the run directory.

QS/VS's values are `release::LOAD_SOURCE`, with `release::SLOT_B_PATH`.
They are the bytes they were: the release build's disk image and UKIs
do not change by this, and a disk installed before reads as before. A
unit test in `lic/release` holds every value.

Where a function of `hostdisk` took nothing and used a constant, the
function of `loadsource` takes the names:

| Was | Is |
|---|---|
| `layout::RECORD_TYPE_GUID`, `DATA_TYPE_GUID` | `layout::record_type(names)`, `data_type(names)` |
| `layout::parts()`, `install_parts()`, `min_disk_bytes()`, `fixed_guids()` | the same with `names` |
| `layout::VENDOR`, `ESP_LABEL`, `ESP_VOLUME_ID`, `SLOT_B_PATH`, the three names | fields of `LoadSourceNames`; `SLOT_A_PATH` is still a constant, the firmware's default path |
| `record::SPEC`, `MAGIC` | `record::spec(names)`, `names.record_magic` |
| `set::LABEL_SPEC`, `RESUME_SPEC`, `RESUME_MAGIC` | `set::label_spec(names)`, `resume_spec(names)`, `names.resume_map_magic` |
| `disks::load_source(disk)` and the functions over load sources | the same with `names` first |
| `prepare::esp_image`, `layout`, `write_table`, `format_data` | the same with `names` first |
| `boot::make_default(vars, name, esps, slot)` | `boot::make_default(vars, names, name, esps, slot)` |
| `payload::DIR`, `Manifest::render()`, `Manifest::parse(text)` | `payload::dir(names)`, `render(names)`, `parse(names, text)` |
| `qsvsstgd::SOCKET`, `DATA_DEVICE`, `Client::default()` | `stgd::socket(appliance)`, `data_device(appliance)`, `Client::of(appliance)` |

The engine names no reference code. It reports through
`appliance::Event::Storage`, with what happened (`appliance::Storage`)
and the detail, and `lic/refcode` says which code each is, as for the
other base services. It asks the appliance for one thing, the
machine's disks with their serial numbers and location codes
(`stgd::Host`), which QS/VS answers from `lic/hostinv`.

The neutrality check (`build doctor`, and the unit test
`host::neutral`) covers the two new crates with the rest: twelve
crates under `base/` name no product and depend on nothing of `lic/`.

### 20: a pair where each disk ran alone

No winner is picked silently any more. `docs/ARCHITECTURE.md` has the
rule ("Units that ran apart"), `docs/SERVICETOOLS.md` the screen, and
`docs/REFCODES.md` the two codes, `A2003040` and `C2003050`.

- `base/loadsource/src/set.rs`: the label is version 2 and holds the
  time it was written; a version 1 label reads with the time unknown.
  `apart` is the rule, in one place, and `SetLabel::keeping` the label
  once a unit is chosen. `disks::own_labels` reads each disk's own
  copy.
- `base/stgd/src/serve/apart.rs`: before the set is assembled the
  storage service looks for units that ran apart. If there are any it
  posts `A2003040`, makes the file `<run>/<id>stgd.choose`, answers
  status with the units (state `Apart`, each with the time its disk
  was last written) and waits for `keep`. It then writes the kept
  unit's label to every disk, clears the resume map, posts `C2003050`
  and goes on to assemble the set as at any IPL. The time is the
  RTC's: the service starts before PID 1 sets the system clock.
- `lic/qsvsinit/src/ipl/choose.rs`: while that file is there PID 1
  mounts nothing and starts nothing, keeps the storage service from
  being counted late (`Supervisor::patient`), and runs `qsvstools
  keep-unit` with the display and the serial port.
- `base/panels/src/keep.rs`: the panel and its confirmation.
  `svc/tools/src/keep.rs`: the console run for that one panel.

The rule waits for Cody to confirm it: `NEEDS-CODY`, in
`docs/OPEN_QUESTIONS.md` item 20, which lists what exactly, and at the
rule itself in `set.rs`.

### 16: what brings menu options 4 and 8

Option 4 says `comes with Phase 8` and option 8 `comes with Phase 9`,
on the menu, in the message line of each and in
`docs/SERVICETOOLS.md`. `CLAUDE.md` has the phase table, 0 to 11.

### The others

17 and 22 are decided in Phase 8. 19 stays open, with what storage
pools will need of QS/400's `protect` before Phase 5. 21 is closed as
it was. `docs/OPEN_QUESTIONS.md` has each.

### How it was checked

On 2026-11-12, in the working tree on the host of `docs/PHASE1.md`
(WSL Ubuntu, KVM nested), with the test image built by `build all
--test-image`. Every command ended with status 0: `cargo fmt --all
--check`; `cargo clippy --workspace --all-targets -- -D warnings`, the
same for the seven image binaries on the musl target, and for PID 1
and Service Tools with `QSVS_TEST_HOOKS=1`; `cargo test --workspace`
(264 tests); `build doctor` (178 `.rs` files start with the header, 12
crates of `base/` are neutral); `build deny` (196 crates, no `links`
key, no C build helper); `build notices --check`.

The suites run, one at a time with `build test --suite`:

| Suite | Time | Result |
|---|---|---|
| `status` | 2.5 s | pass |
| `install-serial` | 24.2 s | pass; the main menu matches its golden file with `Phase 8` and `Phase 9` |
| `install-unattended` | 24.8 s | pass |
| `mirror-degraded` | 53.7 s | pass |
| `mirror-replace` | 53.9 s | pass, three times (below) |
| `mirror-start` | 30.4 s | pass |
| `storage-kill` | 17.2 s | pass |
| `mirror-split` | 88.0 s | pass: each disk run without the other, then both; `REF A2003040 Load-source units ran apart` posted; for 8 s no `/data` mounted and no service started; the choice shown with both units (golden file `keep-unit`), left by a choice only; unit 2 kept (`keep-unit-confirm`), `REF C2003050` posted, the IPL went on from it on `/dev/ublkb0`, unit 1 written again and both Active; unit 1's disk alone then holds what unit 2 had |

The full `build test`, the release image and the two builds for the
hashes were not run for this: they are the phase's acceptance.

### Found while testing

- **A status asked for while the storage service was busy came back
  empty, and an empty status reads as a protected load source.** The
  socket's thread asked the service's loop and gave it two seconds;
  while the loop prepared a disk it answered nothing, and the answer
  sent instead was a status with no units, which `Work with Disk
  Units` showed as `Both units are in step`. `mirror-replace` took
  that screen for the rebuild's end and failed on its golden file, the
  first time it was run here; it passed three times after. The loop now
  says what the status is at every turn and when it accepts a request,
  and the socket answers with that at once (`serve::Shown`); a console
  takes a status with no protection in it as no answer. The code was
  Phase 2's.
- **A code posted while the console has the serial port is not on the
  serial port.** `C2003050` is posted while the choice's console still
  has it; the suite looks in the history, as `mirror-replace` does.

## What exists

`docs/VMM.md` has the machine and the threat model, `docs/PROFILE.md`
the profile, `docs/QS400_GUEST.md` what QS/400 does in a partition.

### The VMM (`vmm/`)

- `vmm/arch`: a partition's x86_64 boot state. The memory map with the
  32-bit hole; the page tables and the GDT; the boot processor's
  registers for the 64-bit Linux boot protocol; the zero page (the
  kernel's own setup header, the memory map, the ACPI root pointer);
  the CPUID a partition sees, its boot MSRs, the local APIC's pins.
  Nothing in it calls KVM.
- `vmm/acpi`: the tables in place of firmware, as one block: RSDP,
  XSDT, a hardware-reduced FADT with the reset and sleep registers,
  MADT, MCFG, and a DSDT with the PCI host bridge, the serial port,
  the clock and the power-off device.
- `vmm/devices`: the I/O port bus; the 16550 (`vm-superio`), the CMOS
  clock, the keyboard controller's reset line (`vm-superio`), the ACPI
  sleep register; the PCI bus with configuration space by port and by
  memory, and MSI-X; virtio over PCI (modern), with `ioeventfd` and
  `irqfd` asked of a `Platform` the VMM answers with KVM; virtio-scsi
  with disk LUNs over a direct-I/O backing store; virtio-rng.
- `vmm/ipl`: the IPL loader. Bounded readers of the guest's disk (GPT,
  FAT16 and FAT32); the LIC record read and its keylock position and
  console mode written through QS/400's `licpart`; the bzImage loaded
  with `linux-loader`. A step done is a `Step` and a load source that
  cannot be IPLed from a `Fault`; `qsvmm` posts each as a code.
- `vmm/qsvmm`: the binary. The profile; the machine on KVM; a thread a
  processor; the serial port's log and console socket; the privilege
  drop; the exit codes.

**Two readers were written here though QS/400's `diskimg` has one of
each.** `diskimg::read_table` and `diskimg::fat::read_file_at` read
back what a build or an installer has just written: they want a
writable device, read a file whole whatever its size, and follow the
table's counts and the FAT's chains as they find them. The loader's
disk is a guest's to write, so its readers check both GPT checksums
and every field, bound a directory and a chain, and refuse a file over
128 MiB before reading it (`vmm/ipl/src/gpt.rs`, `fat.rs`). The LIC
record is read and written by `licpart` and nothing else.

### Starting a partition by hand

There is no partition manager. In a test image, and only there, PID 1's
table has a third hook service, `qsvshook partition`
(`lic/qsvshook/src/partition/`). It looks for a FAT volume labelled
`QSVSPART` with `partition.toml` on it, starts `qsvmm` for that
profile, posts the codes `qsvmm` reports, relays the partition's
serial output to the host's serial port a line at a time
(`TESTLPAR| SRC C6001090  IPL complete`), and posts how the partition
ended with `qsvmm`'s exit code or the signal that killed it. The
channel has commands to start it again with another profile, to
signal it, to type at the partition's console, and to make of a host
device the requests no guest kernel sends (`scsiprobe`).

A release image has `/qsvs/bin/qsvmm` and nothing that starts it: a
unit test holds PID 1's table to that, and Phase 0's suite on the
release image checks that no partition code is posted.

### Reference codes

A new block, `C2005xxx` and `A2005xxx` (`docs/REFCODES.md`): twelve
codes for the steps of an activation and its orderly ends, sixteen for
what a partition lacked when it could not IPL and for the ends that
are not orderly.

### The fuzzer (`tools/fuzz`)

libFuzzer is C++, and the tree builds no C, so the loop is in Rust, as
QS/400's own fuzzer is (none of its source is here: it is not a
library, and it is QS/400's). `cargo run -p build -- fuzz` builds
`qsvsfuzz` with LLVM's `SanitizerCoverage` inline counters and runs
eight targets: `gpt`, `fat`, `lic`, `disk`, `bzimage`, `scsi`,
`virtqueue`, `devices`. A finding is a panic (arithmetic is checked in
the fuzz profile) or an input that runs for more than five seconds;
its bytes are written under `<target>/fuzz/crashes/`.

### Build

- `qsvmm` is a package of both images and is in a slot's initramfs.
- `build doctor` has a row for `QSVS_QS400_IMAGE`: a warning when it is
  unset or not a QS/400 disk image, since the pipeline does not need
  it and four suites do.
- `build fuzz`, and `build test` runs the fuzz targets after the
  suites.

## The crates added

Every one is from crates.io at the version named, pure Rust, with no
`links` key and no build script that compiles anything; `build deny`
passes with them (the count is in the acceptance table).

| Crate | Version | License | Used for | Build script |
|---|---|---|---|---|
| `kvm-ioctls` | 0.25.1 | Apache-2.0 OR MIT | KVM: the VM, the processors, `irqfd`, `ioeventfd`, interrupt routing | probes the compiler's version, compiles nothing |
| `kvm-bindings` | 0.14.2 | Apache-2.0 | KVM's structures, as Rust written ahead of time; `fam-wrappers` for the CPUID, MSR and routing tables | none |
| `vm-memory` | 0.18.0 | Apache-2.0 OR BSD-3-Clause | guest memory: every access bounds-checked against the regions | none |
| `virtio-queue` | 0.18.0 | Apache-2.0 AND BSD-3-Clause | the virtqueue: descriptor chains, the rings | none |
| `virtio-bindings` | 0.2.7 | BSD-3-Clause OR Apache-2.0 | `virtio-queue`'s constants | none |
| `linux-loader` | 0.14.0 | Apache-2.0 AND BSD-3-Clause | the bzImage loader, the zero page's structure (`bzimage` feature only) | none |
| `vm-superio` | 0.8.2 | Apache-2.0 OR BSD-3-Clause | the 16550 and the i8042 | none |
| `vmm-sys-util` | 0.15.0 | BSD-3-Clause | eventfd, epoll, the flexible-array wrappers | none |
| `acpi_tables` | 0.2.1 | Apache-2.0 | the ACPI table and AML writer | none |

They bring `zerocopy`, `zerocopy-derive`, `bitflags` (two versions)
and `log` with them; `zerocopy`'s build script too only probes the compiler. No C
header is read at build time: `kvm-bindings` and `virtio-bindings`
ship their bindings as source.

One QS/400 crate was added, at the pin of every other:
`licpart`, with what it depends on there (`abrec`, `diskimg`, both
already taken). `Cargo.toml` has fifteen `rev =` values, all
`03d6492913318da96bc707cb68d3b54a94ef9bff`. No QS/400 source is copied.
Two constants of QS/400's are restated in `vmm/ipl`, each a fact of
its disk format that no crate QS/VS may depend on exports
(`docs/OPEN_QUESTIONS.md` item 24).

## Kernel additions

None. The host's kernel has had KVM since Phase 0, and what a partition
is given needs nothing more of it: direct I/O on files and block
devices, eventfd, epoll. A QS/VS kernel as a guest already has
virtio-pci, virtio-scsi and the 8250 driver.

## The QS/400 guest image

The suites that IPL QS/400 need a QS/400 disk image built by QS/400's
own orchestrator at the pinned commit, named by `QSVS_QS400_IMAGE`:

```
git clone https://github.com/quartzsystems/quartz-systems-400 qs400-pin
cd qs400-pin && git checkout 03d6492913318da96bc707cb68d3b54a94ef9bff
QS400_TARGET_DIR=$HOME/qs400-pin-target cargo run -p build -- all
export QSVS_QS400_IMAGE=$HOME/qs400-pin-target/images/qs400-V1R1M0-<hash>.img
```

It is a 4 GiB sparse file, about 260 MiB on disk. Nothing of it is
changed: each suite works on a copy. `build doctor` says whether the
variable names one.

## Running

```
cargo run -p build -- all --test-image
cargo run -p build -- test --suite partition-boot
cargo run -p build -- test --suite fuzz
cargo run -p build -- fuzz --target scsi --secs 600
cargo run -p build -- test
```

By hand, on any Linux host with KVM, as an ordinary user who may open
`/dev/kvm` (the privilege drop is then skipped, and `qsvmm` says so):

```
cargo build -p qsvmm
target/debug/qsvmm --profile my.toml --data /tmp/p/data --run /tmp/p/run
```

The guest's serial output is in
`/tmp/p/data/log/partitions/<name>/serial.log`, and its console is the
socket `/tmp/p/run/partitions/<name>/console.sock`.

## The suites

`build test` runs every earlier suite, then these on the test image,
then the fuzz targets. QEMU stands in for the machine, the QS/VS test
image is the host in it, and the partition is a guest of that host:
three levels deep on a build host that is itself a virtual machine.
The QEMU machines that run a partition have their memory allocated
before they start ("Found while testing" says why).

| # | Suite | What it asserts |
|---|---|---|
| 1 | `partition-boot` | the release image's UKI IPLs in a partition with 1, 2 and 4 processors: `C2005060` names the count, the guest's PID 1 posts its own `C2001000`, its banner and `C2001090` on the partition's serial port, and the guest's inventory, read back by the host from the guest's disk, counts as many processors as it was given. A first start whose unit is not there yet gives `A2005010` and `A2005100` |
| 2 | `partition-scsi` | the guest mounts its data file system and writes its state: the unit counts reads, writes and flushes carried out on the host; the host reads the guest's `state/inventory.toml` from its own file, and in it the guest's disk is `QUARTZ VIRTUAL DISK` with the profile's serial number. Then, of a host block device through the virtio-scsi device itself: a read, a write and a verify past the end, a write across the end, a read at the largest address, an unknown command, a LUN that is not there, and four chains that are no request, are each refused, and the last block is unchanged |
| 3 | `partition-ipl-a` | QS/400 IPLs from side A: the activation's codes in order, then the sixteen codes QS/400's own conformance wants, in order, to `SRC C6001090`; the guest names slot A, its processors and the virtio SCSI controller; `pwrdwn` ends it with `C2005100` and exit 0 |
| 4 | `partition-ipl-b` | the same from side B, with four processors |
| 5 | `partition-attributes` | keylock manual and console serial in the profile: `C2005035`, the guest says `mode Manual from LIC override` and `Serial console (ttyS0)`, posts `C1001050`, stops at its IPL menu drawn on the serial port, completes when answered, and its record says the IPL was Manual. Then normal and local: `C2005030` shows them, the guest says `Local console`, posts `A6005008`, IPLs unattended |
| 6 | `partition-kill` | `qsvmm` killed (`SIGKILL`) after the guest located its units, and again after an IPL completed and an object was created: `A2005150` with signal 9 each time, the host's base ready after each; the IPL after each kill completes, and the object is there |
| 7 | `partition-power-pull` | QEMU killed at 20 random moments while the guest creates objects; at each of 21 IPLs the guest's object store recovers and the IPL completes, and every object the guest acknowledged before a cut is there |
| 8 | `partition-end` | guest power off: `C2005100`, exit 0; guest restart: `C2005110`, exit 10, and nothing starts it again; `SIGTERM`: `A2005140`, exit 50 |
| 9 | `partition-damaged` | seven load sources with one thing wrong each (no table, no ESP, an ESP that is no volume, no UKI for the side, a corrupt LIC record, a truncated UKI, a UKI that is no bzImage): each its own `A2005` code, `A2005100`, exit 30, the host's base ready after each |
| 10 | `partition-privilege` | `qsvmm` and every one of its threads run as user and group 23000 with no supplementary group, no capability in any set and `NoNewPrivs` set |
| 11 | `fuzz` | the eight fuzz targets build with coverage counters and each runs 60 s without a finding |
| 12 | | every earlier suite: `build test` runs them first |

Suites 3 to 8 need `QSVS_QS400_IMAGE`. None is blocked: QS/400 IPLs
(`docs/QS400_GUEST.md`).

**What suite 7 shows, and what it cannot.** Killing QEMU takes the
host's memory with it: anything `qsvmm` had taken from the guest and
not yet handed to the host's disk is gone, and a flush `qsvmm`
answered without carrying it out would show as an acknowledged object
that is missing. It does not show what a real power loss adds: QEMU
keeps every write it had taken, flushed or not, so a guest that
acknowledged before its own flush would pass here too. That part is
the first real machine's.

**The cheap guest is not on a block device.** The host takes any disk
with QS/VS's three partition types for a load-source disk of its own
(`docs/OPEN_QUESTIONS.md` item 26), so the harness gives it the QS/VS
image wrapped in a megabyte of zeros and the host copies it into a
file on `/data`; suites 1, 2 and 10 so run a partition on a file of
the host, and suites 3 to 9 on block devices.

## Found while testing

- **The clock was read 6 years wrong.** x86 Linux reads the CMOS
  clock's registers as BCD whatever the mode bit says. The first run
  had them in binary and QS/400 set its clock to 2020. The clock is
  BCD now (`vmm/devices/src/legacy/cmos.rs`).
- **A guest could have panicked the VMM by moving a BAR.** The offset
  of a memory access into a BAR was computed before the check that the
  address is not below it. The `devices` fuzz target found it in seven
  runs (`vmm/devices/src/pci/config.rs`, with a unit test).
- **A partition saw the host's VMX.** KVM's supported CPUID has it when
  the host allows nested virtualization, and the cheap guest's KVM
  probe said `KVM usable`. It is withheld now (`vmm/arch/src/cpu/cpuid.rs`).
- **The test image is no guest.** Its hooks' channel wants a second
  serial port; in a partition that service fails until PID 1 gives it
  up, and the guest's base is never ready. The cheap guest is the
  release image.
- **The partition hook's relay gave up.** It tried the console socket
  for five seconds; on a busy host `qsvmm` had not bound it by then and
  the partition ran with nothing relayed. It tries for as long as the
  run lasts.
- **A partition's memory arrived damaged, under the harness only.**
  About half of the partition starts in a QEMU host went wrong, each
  differently: a guest silent from its first instruction, a triple
  fault, a kernel fault one second in, a lock never released. `qsvmm`
  was given a report of its processors for it (`SIGUSR1`,
  `docs/VMM.md`), and the silent guests had stopped themselves in the
  kernel's decompressor with `XZ-compressed data is corrupt`, 4, 20
  and 46 MiB into the kernel. The kernel as `qsvmm` loaded it was
  whole each time; in one stopped guest four bytes of the copy the
  decompressor makes of it had changed. A few bytes in the wrong
  place is not what a device model's mistake looks like, and the same
  `qsvmm` straight on the build host's KVM started 36 guests of 36.
  The partition is three levels deep here (Hyper-V, WSL's kernel, the
  QEMU host, the partition), and WSL's kernel moves and reclaims the
  QEMU host's memory as it runs. With that memory allocated before
  the machine runs (`-mem-prealloc`, for the machines that run a
  partition: `Launch::prealloc`) six suites of six passed where four
  of six had just failed. It is taken for the outer hosts' nested
  paging and not `qsvmm`'s; that is an inference from those runs, not
  a proof. `NEEDS-CODY`: the partition suites on a Linux machine that
  is no virtual machine, and the first partition on real hardware.
- **A guest's restart never ended the partition.** `pwrdwn restart`
  printed `reboot: Restarting system` and `qsvmm` ran on. The
  processors' report had processor 0 in real mode, running through
  the GDT's bytes: the kernel had jumped to the reset vector, where
  a partition had nothing, and had written to neither the ACPI reset
  register nor the keyboard controller (`docs/QS400_GUEST.md` says
  why). The reset vector has seven bytes now that pulse the reset
  line (`vmm/arch/src/boot/reset.rs`, `docs/OPEN_QUESTIONS.md` item
  29). `docs/QS400_GUEST.md` had said a restart worked; it had not
  been read from a run of this build.
- **The suites looked for an answer at the start of a line.** QS/400's
  service console writes its prompt before an answer, so `QGPL/KILLED
  *DTAARA exists.` is mid-line. Suites 6 and 7 look for the text.
- **Phase 1's power-pull suite read a file before it was written.** On
  a disk the churn never ran on, its first `host.toml` is written as
  the hooks' channel becomes ready; the acceptance run's first attempt
  asked for it a moment sooner and stopped with `host.toml is gone`
  after 0 power cuts. The suite waits for that first write.

## Not done

- **NEEDS-CODY: the rule for units that ran apart** is built as the
  handoff has it and is to be confirmed (`docs/OPEN_QUESTIONS.md`
  item 20).
- `BLOCKED(phase3)`: the choice of the unit to keep was made at the serial port only; no suite types it at the display's keyboard.
- `BLOCKED(phase3)`: three disks that ran apart (a unit replaced while the other ran alone) are unit-tested in `set.rs` only.
- `BLOCKED(phase3)`: a disk of another host's pair, put in beside this host's, is not units that ran apart: the label with the higher sequence is still taken, as before.
- What Phase 2 left undone and this did not touch is still as
  `docs/PHASE2.md` lists it.

Of the VMM:

- **NEEDS-CODY: a QS/400 handoff.** A partition cannot be told the
  side of its next IPL: QS/400 asks firmware it does not have, and the
  LIC record has no field for it that `licpart` owns. Two constants of
  QS/400's are restated in `vmm/ipl` until QS/400 exports them.
  `docs/QS400_GUEST.md` ends with the list; `docs/OPEN_QUESTIONS.md`
  items 23 to 25.
- **NEEDS-CODY: the seven bytes at the reset vector**, which a guest's
  restart needs and "no guest firmware" did not foresee
  (`docs/OPEN_QUESTIONS.md` item 29).
- **NEEDS-CODY: the partition suites on a Linux machine that is no
  virtual machine.** Under WSL they pass only with the QEMU host's
  memory allocated first ("Found while testing").
- **NEEDS-CODY: the first partition on real hardware.** Everything in
  this record ran three levels deep, under QEMU, on a build host that
  is itself a virtual machine. On the Dell test machine a partition is
  one level deep: note how long an IPL of QS/400 takes there, and pull
  the machine's power under a guest that writes (below).
- **NEEDS-CODY: the QEMU job in CI** (`docs/OPEN_QUESTIONS.md` item 6)
  now also builds QS/400's image at the pin, and has still not run.
- `BLOCKED(phase3)`: a power loss that loses writes the host's disk had taken and not flushed cannot be made under QEMU; suite 7 shows what it can (above).
- `BLOCKED(phase3)`: what QS/400 does without firmware at a LIC PTF's IPL and at install registration was read in its source, not run: one needs a LIC PTF, the other an IPL from media (Phase 5).
- `BLOCKED(phase3)`: the system calls `qsvmm` may make are not filtered; the seccomp filter is Phase 11, and the place for it is after the privilege drop.
- `BLOCKED(phase3)`: virtio-scsi carries out one request at a time, with a synchronous read or write on the host; a controller's throughput is one thread's. Asynchronous I/O comes with storage pools (Phase 5).
- `BLOCKED(phase3)`: a guest that sets its clock keeps it for one activation; nothing of a partition is kept between activations before Phase 4.
- `BLOCKED(phase3)`: the loader reads the primary GPT only; a load source whose primary table is damaged is refused though its backup may be whole.
- `BLOCKED(phase3)`: a unit on a host device with 4096-byte logical sectors is given to the guest with that block size, and the loader reads its table at 512: it would be refused with `A2005020`. No such device was tried.
- `BLOCKED(phase3)`: the fuzzer keeps no corpus between runs: each run starts from the seeds built into its targets.
- Codes posted in no suite, only in unit tests or by reading the code:
  `A2005000` (a profile that is refused), `A2005040` (no LIC
  partition), `A2005060` (the machine not built), `A2005120` (a guest
  fault), `A2005130` (a VMM error).
- The virtio PCI configuration access capability
  (`VIRTIO_PCI_CAP_PCI_CFG`) is not there; a Linux guest does not use
  it (`docs/VMM.md`).

Not in Phase 3, and not started: the partition manager and real
profiles, processor pinning and shares, huge pages, storage pools and
virtual disks, optical and tape, virtio-net, the service channel and
console attach, QSI.

## The first partition on real hardware (NEEDS-CODY)

On the Dell test machine, with a test image installed and a QS/400
disk image written to a second disk.

- [ ] A profile on a `QSVSPART` stick naming that disk: `C2005000` to
      `C2005090` on the host's serial port, then the guest's
      `SRC C1001000` to `SRC C6001090` after `TESTLPAR| `. How long
      from `C2005000` to `C6001090`.
- [ ] `hw` at the guest's service console lists as many `PN` resources
      as the profile has processors.
- [ ] Service Tools option 1 on the host still shows every service
      ready while the partition runs.
- [ ] `pwrdwn` at the guest: `C2005100`. `pwrdwn restart`: `C2005110`.
- [ ] The machine's power pulled while the guest creates objects, a
      few times: at each IPL after, the guest's `Storage management
      recovery` and `IPL complete`, and every object it had
      acknowledged is there. This is the part suite 7 cannot show.
- [ ] The partition's `serial.log` under
      `/data/log/partitions/TESTLPAR/` is owned by user 23000 and
      rotates at 4 MiB.
- [ ] The cheap guest and QS/400 started twenty times each: every
      start reaches its first code. Under the harness, three levels
      deep, about half did not until the QEMU host's memory was
      allocated first ("Found while testing"); a real machine has no
      such layer, and this is where that is shown.
