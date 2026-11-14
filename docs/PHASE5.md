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

## The QS/400 guest media

`pool-install` installs QS/400 from its installation ISO, which has to
carry a response file to install unattended. At the pin, in a clone at
`0fc8afa5055d1dee0a05cf35edb58760f10607ab`, with `pool-install.rsp`:

```
QS400-INSTALL-RSP 1
lic-option = 2
load-source = *FIRST
language = 2924
system-name = QSPOOL
qsecofr-password = Quartz1
licpgm = *NONE
```

then

```
QS400_TARGET_DIR=$HOME/qs400-0fc8-target QS400_INSTALL_RSP=$PWD/pool-install.rsp \
  cargo run -p build -- iso
export QSVS_QS400_ISO=$HOME/qs400-0fc8-target/images/QS400-V1R1M0-c1c9ff9f691a.iso
```

The ISO keeps its name (the hash is of the release's payload) and is
2 KiB larger with `QS400/INSTALL.RSP`; `build doctor` says whether
the ISO it is given has the media UKI and a response file. The other
partition suites still take `QSVS_QS400_IMAGE`, the disk image of the
same commit (`qs400-V1R1M0-d845bbcfbcb7.img`).

## Running

```
cargo run -p build -- all --test-image
cargo run -p build -- test --suite pool-create
cargo run -p build -- test
cargo run -p build -- fuzz --target tape
```

By hand, a pool service of a test image is asked through the hooks'
channel (`poold <hex>`, the request encoded as `docs/POOLD.md` has
it); the conformance client (`tools/conformance/src/suite/storage`)
does that for every request.

## The suites

`build test` runs every earlier suite, then these on the test image,
then the fuzz targets. A pool disk is a further QEMU drive of 2 GiB
with the serial `QSVSUNIT<n>`; a disk is pulled from under the host
through QEMU's monitor (`drive_del`). The guests are as in Phase 4:
QS/400, the release image (the cheap guest, from side A), and the
spinner.

| # | Suite | What it shows |
|---|---|---|
| 1 | `pool-create` | a mirrored pool of two disks made, Active and mounted at `/pools/POOL1`; assembled from its labels at the next IPL (`C2007010`); its state file deleted and made again from the labels at the IPL after |
| 2 | `pool-install` | the media repository designated, QS/400's ISO uploaded, a thin virtual disk of 8 GiB made, a partition with it and an optical device activated from IPL source D: the media UKI read (`C2005045`), the LIC installed, the partition restarted into side A and the operating system installed from the media to `C6001090`; activated again with `recorded`, side A to `C6001090` |
| 3 | `pool-thin-thick` | a thin disk takes space as it is written and gives it back on discard; a thick disk has its space when made and writes with the pool otherwise full |
| 4 | `pool-full` | the cheap guest on a thin disk in a pool filled by a file: its first write that needs room holds it (`A2005160`), still Running, no error in the guest; room made, it goes on by itself (`C2005160`) |
| 5 | `pool-pulled-mirrored` | unit 1 of a mirrored pool pulled while it runs: `A2007020` names the unit, the hot spare taken and rebuilt, Active, 32 MiB written before read back the same |
| 6 | `pool-pulled-raid5` | the same for a RAID 5 pool of three with a spare, unit 2 pulled |
| 7 | `pool-pulled-raid6` | a RAID 6 pool of four with a spare, two units pulled: the spare takes one, the pool Degraded and the data whole |
| 8 | `pool-lost` | both units of a mirrored pool pulled: `A2007040`, the pool Failed, nothing more written; another pool Active |
| 9 | `pool-power-pull` | twenty power cuts while a mirrored pool is written: assembled every time, each resynchronization only what the map had set (32 to 64 MiB of 2038), Active at the end |
| 10 | `pool-kill` | the pool service killed three times under a mounted pool: started again, the `ublk` device taken over, a file written before read back the same |
| 11 | `pool-vdisk-rules` | a disk held by one partition at a time (`QVS3105`); grown only while none holds it; not deleted while two profiles name it (`QVS3108` for each) unless forced; a partition deleted with its disks takes them |
| 12 | `pool-optical` | through the device, over two images of the library: 166 blocks read the same as the image, a write refused, malformed commands refused; while removal is prevented the host's unload and the guest's eject are refused, and once allowed the eject unloads; another image loaded is told by a unit attention |
| 13 | `pool-tape` | through the device, over a volume of the library: a label, a filemark, a 64 KiB block and two filemarks written and read back, the filemark and the end of data reported; written to the end, the early warning first and then the end with nothing written; a write-protected volume refuses a write |
| 14 | `pool-media-damaged` | IPL source D from an empty drive, a damaged image and an image with no UKI: `A2005032`, `A2005033`, `A2005034`, each Not activated |
| 15 | `pool-upload` | an upload cut off leaves nothing; a file that is not ISO 9660 refused (`QVS3206`) and nothing left; a whole one kept and listed by its volume identifier |
| 16 | `pool-foreign` | a disk with a partition table and one with a file system shown as foreign, a pool of them refused with `QVS3007` for each, nothing written; made when the two were confirmed |
| 17 | `pool-tools` | Service Tools option 2: the other disks counted, option 6 shows what each is, option 7 and F6 make a mirrored pool once its name is typed and not on a wrong word, option 5 shows its units; eight golden screens |

The three fuzz targets of this phase, 60 s each on the working tree
before the acceptance run: `iso` 292926 runs, `optical` 302013, `tape`
28420, no finding.

## Acceptance

Taken in a fresh clone at `863d9c1` on ext4 in WSL, every target
empty, `QSVS_BUILD_ID=phase5`, the signing key of the reproducibility
runs, `QSVS_QS400_IMAGE` and `QSVS_QS400_ISO` as above, on
2026-11-14. Host: rustc 1.98.0, clang 21.1.8, QEMU 10.2.1, cargo-deny
0.20.2, kernel 6.18.33.2-microsoft-standard-WSL2, 12 processors.

The run stopped at `partition-auto-start` after 46 suites had passed.
The suite was wrong, not the manager: it took the order of the VMMs'
first lines for the order they were started in, and two VMMs started
a moment apart say them in either order. It checks their process IDs
now (`c585c28`). The clone was moved to that commit, the images kept,
and every suite from there on run one at a time. Of those,
`partition-tools` failed on its golden status screen, which had no
pool service on it, and the two RAID pulls on a copy of the test
hooks made slow by the 4 KiB holes of `pool-full` (a parity
read-modify-write each page). Both fixed in `f95df94`; the clone was
moved again, its test image built again, and every storage suite and
`partition-tools` run again. The release build at `f95df94` is the
same file for file as at `863d9c1` but for the manifest, which names
the commit (`git_rev`).

| # | Check | Result |
|---|---|---|
| 1 | Step 0 on its own commits, the earlier phases passing on the new pin | `833e270` and `1251e68`, before any storage; every earlier suite passed on them ("Step 0", above) and again here |
| 2 | Suites 1 to 17 | all passed: Phases 0 to 4 at `863d9c1` and `c585c28`, Phase 5's at `f95df94` (below); the twelve fuzz targets, 60 s each, no finding; `build test --media iso` passed |
| 3 | Phase 4's questions | 30 and 33 closed, 31 left, 32 with the next kernel pin (`docs/OPEN_QUESTIONS.md`) |
| 4 | Two clean builds byte for byte; `deny`, `doctor` | identical (below); `doctor` (with the `QSVS_QS400_ISO` row), `deny`, `notices --check` passed; `cargo fmt`, both clippy runs and `cargo test --workspace` passed on the working tree at each commit |
| 5 | One pin, nothing of QS/400 copied | one `rev` in `Cargo.toml` for all fifteen crates, `0fc8afa` |
| 6 | The release image listens nowhere and cannot act on a partition or a virtual disk | its initramfs has no `TEST IMAGE:`, no `qsvshook` and no `qsvs.hook=` (0, 0, 0; the test image's: 1, 6, 0); `sockets` passed; its partition manager refuses a host path (`QVS1218`, the unit test `a_release_image_takes_no_host_path_as_a_backing_store`); Service Tools' lists of virtual disks and media have no option that acts |

The suites of Phase 5, at `f95df94`, in seconds: `pool-create` 15,
`pool-install` 91 (QS/400 installed in 58 s of it), `pool-thin-thick`
20, `pool-full` 18, `pool-pulled-mirrored` 34, `pool-pulled-raid5`
26, `pool-pulled-raid6` 131, `pool-lost` 8, `pool-power-pull` 74,
`pool-kill` 15, `pool-vdisk-rules` 10, `pool-optical` 7, `pool-tape`
6, `pool-media-damaged` 7, `pool-upload` 8, `pool-foreign` 8,
`pool-tools` 29; and `partition-tools` 13.

The earlier ones, at `863d9c1`: `IPL slot A` 12.7, `IPL slot B` 2.8,
`status` 2.7, `dhcp` 2.7, `static-vlan` 13.6, `time` 3.9, `inventory`
2.9, `supervision` 31.3, `power-pull` 189.0, `slot-confirm` 16.1,
`shutdown` 12.7, `sockets` 2.7, `install-serial` 24.7,
`install-display` 25.3, `install-unattended` 23.1, `mirror-degraded`
74.1, `mirror-replace` 55.7, `install-power-pull` 185.9,
`mirror-power-pull` 133.1, `storage-kill` 17.4, `tools-network` 57.3,
`tools-sign-on` 25.5, `media-refused` 4.9, `answers-invalid` 9.3,
`installed-base` 344.5, `mirror-start` 15.6, `mirror-split` 91.1,
`partition-boot` 19.8, `partition-scsi` 14.9, `partition-ipl-a` 42.6,
`partition-ipl-b` 47.7, `partition-attributes` 88.4, `partition-kill`
66.2, `partition-power-pull` 545.4, `partition-end` 97.2,
`partition-damaged` 7.1, `partition-privilege` 10.2,
`partition-recorded` 114.6, `partition-define` 9.4, `partition-three`
6.3, `partition-minimum` 5.9, `partition-dedicated` 10.7,
`partition-capped` 25.7, `partition-uncapped` 25.9,
`partition-core-sched` 26.1, `partition-huge-pages` 11.4; at
`c585c28`: `partition-auto-start` 34, `partition-killed` 31,
`partition-manager-restart` 11, `partition-identity` 121,
`partition-isolation` 6, `partition-own-disks` 14.

### Hashes

The release build at `863d9c1`, the same in two clean builds one after
the other (524 s and 566 s):

| File | SHA-256 |
|---|---|
| `images/qsvs-V1R0M0-a-986e1c97c6d8.efi` | `986e1c97c6d8782d293cf0d8eceb3a01e5b8109f4e386d01ea87e183ffbf21e2` |
| `images/qsvs-V1R0M0-b-d52fd39d6760.efi` | `d52fd39d67605fe33d58da70253a958cb93a3eb986b8cdcf6cae81159c803426` |
| `images/qsvs-V1R0M0-d-86f4bcb7e122.efi` | `86f4bcb7e122ece6d768019f6fedf0cf906709100612f89544c6902fda91137f` |
| `images/qsvs-V1R0M0-986e1c97c6d8.img` | `7849eb20ae3ee9d53ddb9e2da654113f80ce4f543ed487db7ef4234427fea7e8` |
| `images/QSVS-V1R0M0-86f4bcb7e122.iso` | `68e583437630a1460d32e5ba84d3c9bbb03f60fbcbd156ca36c6be29d6e628d1` |
| `images/initramfs.cpio` | `8f713536032867602ff8dedd178bd010f16d90a23cc8a3a769f6220f9eb67dc5` |
| `images/initramfs-media.cpio` | `c5713a7df8008e11a39d57cf0b7223d28676ed1c0a7ebc6eda5820f9158ef330` |
| `images/manifest.json` | `f4a6936428ee7ac0d6603e6f919f4288a139d3452cf6c262a72b49ea14f2ea9b` |
| `kernel/build/.config` | `491b9f923cdeaa6af84e37115ba40a5dc8d4a77d91085624fdc78d2a6323acbd` |
| `qsvspartd` | `632b23a0eaf7a346fee674b2a94e628eaf143ded9f75803d7cd319c7a2cb06a4` |
| `qsvmm` | `735f0e691648c55be63a433a028ecf9ed0657919b8cdf7139e58de26eb29dc0f` |
| `qsvspoold` | `4dce27b7ff382146ca5a31ca2636fb30c45ba25c53621e27a777bfebc508b59a` |

At `f95df94` the manifest is
`c2faa9c320b0417c742f607eb16128b0bddcc694dba9b19f249869461a68651d`
and every other file the same. The manifest's signature depends on
the key and is left out.

## Found while testing

- **A virtual disk on a pool failed a guest's 512-byte reads.** The
  VMM opens a backing file for direct I/O and took a file's alignment
  to be 512 bytes. A file on a pool is on xfs over the pool's `ublk`
  device, whose blocks are 4096 bytes, and direct I/O there wants
  4096: the release image's data file system, read in 512-byte
  sectors, failed with a medium error and was not mounted. QS/400's
  kernel reads 4096 at a time and installed and ran. A file's store
  now takes its alignment from `statx` and reads and writes what is
  not on it through an aligned buffer, a write of part of a block by
  reading it, changing it and writing it, under a lock
  (`vmm/devices/src/virtio/scsi/backing.rs`, with a unit test).
- **A mirrored pool with both units gone was not Failed.** A mirror
  keeps its last unit in service whatever it does, so the label never
  had two units out of step; every transfer already failed. A
  transfer the set itself fails now fails the pool (`A2007040`).
- **A suite's host disk outlived its run.** The partition suites' host
  disk is copied from the test image once and kept; a storage suite
  run again after a rebuild booted the image before it, with the last
  run's pools in its state. Every storage suite now starts from a
  fresh copy.
- **The test hooks' probe laid a 64 KiB write over its own response.**
  Data-out was at `0x9000`, the response at `0xA000`: the block was
  changed before the drive had it. The buffers are apart now.
- **The test hooks' copy skipped zeros by the mebibyte.** A pool
  "filled" from `/dev/zero` was holes, and a guest's writes found
  chunks already allocated. The copy leaves each page of zeros a hole
  and the suites fill with random bytes.
- **`partition-auto-start` read the order of starting from the serial
  port.** Its two partitions' first lines are their VMMs', which two
  VMMs started a moment apart say in either order; it passed three
  times on that before it failed. It reads the VMMs' process IDs now.
- **The copy's 4 KiB holes made the RAID pulls time out.** Each page
  was a write of its own, and on a parity pool a read, a change and a
  write: the 32 MiB pattern outlasted the channel. The pages between
  holes go in one write now.
- **`partition-recorded` and `partition-core-sched` each failed once
  in Step 0's run** and passed after (Step 0, above).

## Not done

- `BLOCKED(phase5)`: a save by QS/400 to the virtual tape drive and a restore from it to another partition were not run: QS/400's tape code has never met a drive (`docs/OPEN_QUESTIONS.md` item 36). Moved to Phase 10, with the pass-through to a real drive.
- `BLOCKED(phase5)`: a virtual disk cannot be copied: the pool's xfs has no reflink (`docs/OPEN_QUESTIONS.md` item 35).
- `BLOCKED(phase5)`: a pool's size is fixed when it is made: no set of QS/400's can grow (handoff 15).
- `BLOCKED(phase5)`: an import into the library from the host's own optical drive or a USB volume was shown in unit tests, not under QEMU with a real disc.
- What Phase 4 left undone and this did not touch is still as `docs/PARTD.md`, `docs/PROFILE.md` and `docs/RESOURCES.md` list it.

Not in Phase 5, and not started: virtual Ethernet, the service
channel, QSI, physical I/O, any change to a running partition's
resources.

## For a real machine (NEEDS-CODY)

- [ ] NEEDS-CODY: a pool on real disks of the Dell machine: made from
      Service Tools, a disk pulled from a running RAID 5 pool and the
      rebuild onto a hot spare timed, and a power cut while it
      rebuilds.
- [ ] NEEDS-CODY: the load source's bitmap after a real power loss:
      `C2003020` with the copy bounded, on two real disks.
- [ ] NEEDS-CODY: QS/400 installed from a real disc in the host's
      drive imported into the library (option 7, F7), then from IPL
      source D.
- [ ] NEEDS-CODY: `docs/OPEN_QUESTIONS.md` item 34: whether QS/MC
      should see a held partition as Running with `A2005160`.
- Phase 4's list for a real machine (`docs/PHASE4.md`) still stands.
