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

## Not done

- **NEEDS-CODY: the rule for units that ran apart** is built as the
  handoff has it and is to be confirmed (`docs/OPEN_QUESTIONS.md`
  item 20).
- `BLOCKED(phase3)`: the choice of the unit to keep was made at the serial port only; no suite types it at the display's keyboard.
- `BLOCKED(phase3)`: three disks that ran apart (a unit replaced while the other ran alone) are unit-tested in `set.rs` only.
- `BLOCKED(phase3)`: a disk of another host's pair, put in beside this host's, is not units that ran apart: the label with the higher sequence is still taken, as before.
- What Phase 2 left undone and this did not touch is still as
  `docs/PHASE2.md` lists it.
