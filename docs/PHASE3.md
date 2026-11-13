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

## Not done

- **NEEDS-CODY: the rule for units that ran apart** is built as the
  handoff has it and is to be confirmed (`docs/OPEN_QUESTIONS.md`
  item 20).
- `BLOCKED(phase3)`: the choice of the unit to keep was made at the serial port only; no suite types it at the display's keyboard.
- `BLOCKED(phase3)`: three disks that ran apart (a unit replaced while the other ran alone) are unit-tested in `set.rs` only.
- `BLOCKED(phase3)`: a disk of another host's pair, put in beside this host's, is not units that ran apart: the label with the higher sequence is still taken, as before.
- What Phase 2 left undone and this did not touch is still as
  `docs/PHASE2.md` lists it.
