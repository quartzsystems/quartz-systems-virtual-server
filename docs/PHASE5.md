# Phase 5: virtual storage, optical and tape

The record of Phase 5. Phases 0 to 4 are `docs/PHASE0.md` to
`docs/PHASE4.md`; what they say still holds, except where this says
what changed.

Phase 5 moves the pin of QS/400 to the commit that holds handoff 15,
gives the mirrored load source its write-intent bitmap, and then gives
the host its own storage: storage pools on protected sets of disks,
virtual disks in them, a media library with virtual optical and tape
devices, and IPL source D. There is still no virtual Ethernet, no
service channel and no QSI: the pool service is driven over a local
socket, as the partition manager is.

## Step 0: the pin of QS/400 and the load source's bitmap

### The prerequisite

QS/400's handoff 15 was on its `main` (`0fc8afa`, 2026-11-14): the
write-intent bitmap for `protect::set::MirrorSet` (`14db42c`,
`dbc7e52`, `5f18e44`) and the three answers in its
`docs/BUILDKIT.md` section 5 (`0fc8afa`): parity sets bound their
repair after a power loss to what was in flight; no set can grow; and
the limits of large sets.

### The pin

Every `rev =` in `Cargo.toml` is now

```
0fc8afa5055d1dee0a05cf35edb58760f10607ab
```

fifteen of them, all at once (`docs/PHASE0.md` has the pin's history),
in `833e270` with its lock file. The kernel pin and
`rust-toolchain.toml` did not change: they are QS/400's at that commit
as they were at the last.

**What the new pin broke: nothing.** Between the two pins QS/400 has
113 commits; of the fifteen crates QS/VS takes only `protect` changed
(a new `intent` module and `MirrorSet::open_with`, both additions) and
one of `buildkit`'s fixture files. The workspace built, linted on both
targets and passed its unit tests on the new pin with no change.

### The load source's bitmap

`1251e68`. The mirrored load source opens its set with `protect`'s
write-intent bitmap (`MirrorSet::open_with`). The map is kept where
`base/loadsource` could keep it without moving anything an installed
host has on disk: in the tails of the set label's two copies in the
record partition, which the label (at most 21.5 KiB of a 64 KiB slot)
never reaches. At 32 KiB into the first copy is the set's own map; at
32 KiB into the second, the map of a member out of service, so a unit
that was away comes back by what was written while it was
(`base/loadsource/src/intent.rs`). Each disk holds a copy and a write
goes to every disk; the copies are read together and their bits
joined, so a copy that missed a write only makes more regions dirty.
A region is 16 MiB, doubled for a data partition over 4 TiB until the
map fits its 32 KiB. The map has the magic `QSVSIMAP`
(`release::LOAD_SOURCE`, held by a test).

A host installed before this has zeros there, which read as no map: a
clean stop starts the map clear, and only an unclean one before the
first clean stop copies every region once, as before.

The storage service (`base/stgd`) no longer suspends the second unit
after an unclean stop: the set copies the regions its map has set from
the first unit in step to the other (`resync_step`), posts
`C2003020 Resynchronization started: units in step again, N of M MiB
to copy` (or `C2003025 ... 0 of M MiB` when nothing was dirty), clears
idle regions every turn of its loop, and empties the map when it stops
in order. Open question 19 is closed.

### The earlier suites on the new pin

Taken in a fresh clone at `1251e68`, empty targets, both images built,
`QSVS_QS400_IMAGE` the image QS/400's orchestrator built at the pin
(`qs400-V1R1M0-d845bbcfbcb7.img`), on 2026-11-14:

| Step | Result |
|---|---|
| `build doctor` | passed |
| `build all` | passed, 10 min 14 s from empty targets |
| `build all --test-image` | passed, 9 min 38 s |
| `build test --media iso` | `IPL from media` passed, 2.6 s |
| `build test`, Phases 0 to 3 and the first nine of Phase 4 | 37 passed, then `partition-recorded` failed (below) and the run stopped |
| the sixteen Phase 4 suites from `partition-recorded` on, one at a time | all passed but `partition-core-sched` (below); passed alone |

The times, in seconds, of the run (Phase 0, base, installer, VMM,
partition manager): `IPL slot A` 12.4, `IPL slot B` 2.9, `status`
2.7, `dhcp` 6.1, `static-vlan` 13.3, `time` 4.0, `inventory` 3.6,
`supervision` 31.9, `power-pull` 192.9, `slot-confirm` 16.7,
`shutdown` 13.1, `sockets` 2.9, `install-serial` 25.5,
`install-display` 26.3, `install-unattended` 22.4, `mirror-degraded`
74.7, `mirror-replace` 56.2, `install-power-pull` 179.6,
`mirror-power-pull` 125.0, `storage-kill` 17.6, `tools-network` 56.6,
`tools-sign-on` 25.5, `media-refused` 5.2, `answers-invalid` 9.3,
`installed-base` 320.8, `mirror-start` 13.0, `mirror-split` 90.2,
`partition-boot` 18.9, `partition-scsi` 13.3, `partition-ipl-a` 39.9,
`partition-ipl-b` 39.8, `partition-attributes` 89.2, `partition-kill`
64.8, `partition-power-pull` 583.4, `partition-end` 102.3,
`partition-damaged` 7.4, `partition-privilege` 10.4; and alone
`partition-recorded` 118, `partition-define` 11, `partition-three` 6,
`partition-minimum` 6, `partition-dedicated` 11, `partition-capped`
28, `partition-uncapped` 31, `partition-huge-pages` 15,
`partition-auto-start` 20, `partition-killed` 13,
`partition-manager-restart` 8, `partition-identity` 120,
`partition-isolation` 7, `partition-own-disks` 15, `partition-tools`
12, `partition-core-sched` 27, 26 and 25.

Two suites failed once and passed after:

- **`partition-recorded`** stopped at `C6001015`, QS/400's storage
  management started, waiting for `C6001030`; both virtual processors
  were halted. A `cargo clippy` of the working tree was running on the
  same build host at the time, at the lowest priority but on two of
  its threads. Alone it passed in 118 s. Nothing the new pin or the
  bitmap changed is under it: the partition's disk is a host file.
- **`partition-core-sched`** saw the two partitions on the core's two
  threads at once in 1 look of 3316, though the suite reads the two
  threads again at once when it sees that, to rule out a read across a
  switch. It passed three times after (3338, 3356, 3342 looks, never
  together). The threads are QEMU's virtual CPUs of a guest that is
  itself a virtual machine, where the interrupt that makes a sibling
  leave its core takes long enough to be seen twice. The suite is
  left as strict as it was; on real SMT it is still Phase 4's item for
  a real machine (`docs/PHASE4.md`).

`mirror-power-pull` now asserts that each cut's resynchronization is
less than the whole unit: on the working tree it passed with 1024 MiB
copied over twenty cuts of a 381 MiB unit, about 51 MiB a cut where
each was 381 before. `storage-kill`, which had asserted no
resynchronization at all after a server killed under load, now asserts
that none copies a whole unit: a server cut off may leave a write on
one unit only, and the regions the map has set are copied.

## Answers to Phase 4's open questions

| # | Question | What was done |
|---|---|---|
| 30 | what a profile's IPL type is | confirmed as built; closed |
| 31 | a VMM ended by a signal the partition manager did not send | left as it is |
| 32 | the kernel built for 64 threads | moves with the kernel pin's next change, in QS/400 and here together |
| 33 | what a processing unit is worth where a core has two threads | confirmed as built; `docs/RESOURCES.md` now says what a load that keeps one thread busy gets: half of a capped partition's units, at most, since its quota is counted in both threads' time |
| 6, 17, 22 | CI, the identity's key, the users' owner | unchanged |

Phase 5 raised 34 to 36 (`docs/OPEN_QUESTIONS.md`).

## What exists

`docs/STORAGE.md` has pools, virtual disks, the library and its
formats, every failure and the limits; `docs/POOLD.md` the pool
service's socket; `docs/PROFILE.md` the profile's disks and media;
`docs/VMM.md` the optical and tape units and the hold for space;
`docs/SERVICETOOLS.md` option 2's new panels.

### The storage crates (`stg/`)

- `stg/pool`: a pool's member label (two copies, sequence numbers,
  the units with their partition identifiers, serials and states, the
  owner's identity), a pool disk's GPT layout, a pool to be made and
  every reason it cannot be, and what each disk is: the load source,
  a unit or spare of this host's pools, not configured, or foreign.
  The pool disk's partition types and the label's magic are
  `release::POOL_DISK`, held by a test as `release::LOAD_SOURCE` is.
- `stg/vdisk`: virtual disks as raw files in a pool, thin (sparse)
  or thick (allocated with `fallocate`), with their names, sizes,
  growth, and what is allocated of each.
- `stg/media`: the library in the repository pool: optical images
  checked as ISO 9660 when they come in, tape volumes with their
  sidecar (volume serial, capacity, write protection), imports from a
  removable volume or the host's drive.
- `stg/tape`: the AWSTAPE image and a drive's view of it: blocks,
  filemarks, the end of the volume and its early warning, every
  error a damaged image gives, all bounded.
- `stg/poold` with `lic/qsvspoold`: the pool service. It finds the
  host's pool disks by their labels, assembles each pool's set
  (`protect`'s plain device, `MirrorSet` with its write-intent map,
  or `ParitySet`), serves it through `ublk` with recovery on, mounts
  its xfs at `/pools/<NAME>` and writes `pools.toml` from the labels.
  It rebuilds onto a hot spare or a replacement, giving way to the
  partitions' own transfers, warns at 80% and 90% full, opens virtual
  disks, images and volumes for the partition manager and hands them
  over open (`base/fdpass`), and keeps who holds what across its own
  restart. It is the fifth service of PID 1's table, before the
  partition manager.

### The VMM

- **Optical.** A virtio-scsi function of its own with an MMC unit:
  READ(10/12), READ CAPACITY, GET CONFIGURATION, READ TOC, the
  media events, PREVENT ALLOW, START STOP UNIT, a medium loaded and
  unloaded by the partition manager over the VMM's media socket.
- **Tape.** A function of its own with an SSC unit over an AWSTAPE
  volume: variable and fixed blocks, filemarks, SPACE, REWIND,
  LOCATE and READ POSITION, the early warning and the end of the
  medium as SSC gives them, write protection.
- **IPL source D.** The loader reads the UKI from the optical
  medium's ISO 9660 tree (`EFI/BOOT/BOOTX64.EFI`) through a bounded
  reader of its own (`vmm/ipl/src/iso.rs`).
- **A pool that fills.** A write to a thin virtual disk that finds no
  room stops the partition's processors and posts `A2005160`; every
  second the write is tried again, and when it goes the processors
  go on and `C2005160` is posted. The guest sees nothing but time.
- Three fuzz targets more: `iso`, `optical`, `tape`.

### The partition manager

A profile's disks are virtual disks (`POOL/NAME`), its optical drive
and tape drive name media of the library. At activation it asks the
pool service for every one, takes them open, and gives them to
`qsvmm` as inherited descriptors; at the end it gives them back. A
release image refuses any other backing store (`QVS1218`). The
requests 21 to 24 load and unload a running partition's media and say
what is loaded. A pool that fails under a running partition ends it
in Error.

### Service Tools

Option 2, Work with Disk Units, has four options more: 6, the disk
configuration; 7, work with storage pools (make, rename, delete,
replace a unit, add and remove spares, designate the media
repository, import an image); 8 and 9, the virtual disks and the
media library, read only. Every destructive request takes the pool's
name typed in full.

### Messages and reference codes

`QVS1218`, `QVS1219`, `QVS2111` to `QVS2115`, `QVS3001` to
`QVS3018` (pools), `QVS3101` to `QVS3108` (virtual disks),
`QVS3201` to `QVS3209` (the library). The pool service's codes are
the `C2007xxx` and `A2007xxx` block; the hold for space and IPL
source D's are in the partitions' `C2005xxx` and `A2005xxx`
(`docs/REFCODES.md`).

## Kernel additions

Added to `kernel/config/qsvs.config` and the build's required list:
`JOLIET` and `EXFAT_FS`, for imports into the library from a
removable volume. `BLK_DEV_UBLK`, `XFS_FS`, `BLK_DEV_SR`,
`ISO9660_FS` and `USB_STORAGE` were there already.
