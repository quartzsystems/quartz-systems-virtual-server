# Storage: pools, virtual disks, the media library

The host's own storage, beside its load source: storage pools on
protected sets of whole disks, virtual disks in them for the
partitions, and a media library of optical images and tape volumes in
the pool the operator designates. The pool service, `poold`
(`stg/poold`, run as QS/VS by `lic/qsvspoold`), keeps all of it and is
driven over a local socket (`docs/POOLD.md`). It is separate from the
load source's storage service (`base/stgd`), so a fault in pool code
cannot take `/data` away.

| Crate | What |
|---|---|
| `stg/pool` | the member label, a pool disk's layout, a pool to be made and its limits, what each disk of the host is |
| `stg/vdisk` | virtual disks: raw files in a pool |
| `stg/media` | the media library: optical images, tape volumes, imports |
| `stg/tape` | a tape volume's image (AWSTAPE) and a drive's view of it |
| `stg/poold` | the service and its socket |

## A storage pool

One protected set of whole disks with one xfs file system on it,
presented to the host through `ublk` and mounted at `/pools/<name>`.
The set is QS/400's `protect` at the pinned commit
(`docs/PHASE0.md`).

| Protection | Disks | Holds when |
|---|---|---|
| `none` | 1 | nothing fails |
| `mirrored` | 2 | one fails |
| `raid5` | 3 to 18 | one fails |
| `raid6` | 4 to 18 | two fail |

A pool's name follows the naming rule of partitions: 1 to 10 of `A`
to `Z`, `0` to `9` and `_`, a letter first. A pool has a UUID, made
when it is.

**Its size is fixed when it is made.** QS/400's handoff 15 answered
that neither kind of set can grow (`docs/BUILDKIT.md` section 5 in
QS/400's repository): a mirrored set's size is fixed when it opens,
and a parity set's layout is made once. More capacity is another pool.

**Hot spares** belong to a pool, up to 8, each at least as large as
the pool's units. A pool with no protection has none.

### A pool disk

Every disk of a pool, member or hot spare, is made a pool disk when it
is taken: a GPT with two partitions.

| Partition | Type (QS/VS's, `release::POOL_DISK`) | Name |
|---|---|---|
| record | `6c1e8a3d-57f2-4b9e-9a0c-3d4e51b702f8`, 8 MiB | `QSVSPOOL-R` |
| member | `2b9d4f61-0e8c-4a73-b5d2-8f17c6a9e430`, the rest | `QSVSPOOL` |

The record partition holds, from its start:

| At | What |
|---|---|
| 0 | the member label, copy A (`abrec`, magic `QSVSPOOL`); at 32 KiB the set's write-intent map |
| 64 KiB | the member label, copy B; at 32 KiB the map of a member out of service |
| 1 MiB | the resume map, copy A (magic `QSVSPRSM`), room for 27 TiB |
| 4.5 MiB | the resume map, copy B |

A parity set keeps its own records on its members, at their end: two
partial-parity log slots (`QSVSPPPL`) and a state page (`QSVSPSTA`).

### The member label

Every disk of a pool carries the whole label: the host's identity,
the pool's UUID and name, its protection, the set's size and each
member's, a parity set's layout and its synchronization mark, whether
the pool was stopped in order, when the copy was written, its
generation, and every unit (members in order, then hot spares) with
its state, its disk's serial number and location code. A unit that is
gone is still known by where it was.

**The labels are the truth.** At its start the service reads every
whole disk the host has; a disk whose record partition holds a label
of this host joins its pool, the copy with the highest generation is
the pool's, and `/data/state/pools.toml` is written out from the
labels. When that file is lost it is made again at the next start.

**Whose a disk is.** The host's identity is the first 16 bytes of its
Ed25519 public key (`base/identity`), the key the host made at its
first IPL and keeps in `/data/state`. The host takes a disk only when
its label is this host's. A disk with another host's label, a
partition table, a file system or another volume manager's label is
**foreign**: it is shown for what it is and taken only when the
request names it to confirm (in Service Tools, its name typed). A
disk with nothing the host can see is **not configured**. A host whose
identity was made again (its `/data` lost) sees its old pools as
foreign: reusing them erases them.

### States

| Pool | Means |
|---|---|
| Active | every member in step |
| Degraded | a member failed or is missing; the pool runs, protected or not |
| Rebuilding | a member is being written from the others |
| Resynchronizing | after an unclean stop, the members are being brought in step; or a new parity set's parity is being computed |
| Failed | more members are lost than the protection covers |

A unit's state is `protect`'s, with its location code, serial and
size: Active, Rebuilding, Resuming, Suspended, Failed, Missing, or
Hot spare.

### What happens on each failure

- **A unit fails** (its I/O fails, or it is not found at the IPL): it
  is taken out of service, `A2007020` is posted naming it by location
  code and serial, and the pool is Degraded. With a hot spare, the
  spare takes the unit's place at once (`C2007040`) and its rebuild
  starts (`C2007030`, `C2007035` when done). With none, the pool waits
  for the unit to be replaced from Service Tools.
- **A rebuild or a resynchronization** copies a megabyte at a time and
  stands back while the partitions' own transfers are under way; its
  progress is in the pool's status.
- **The power is lost.** A mirrored pool keeps a write-intent bitmap
  (16 MiB regions, more on a pool over 4 TiB) and copies only the
  regions it has set (`C2007020` with the extent). A parity pool's
  partial-parity log bounds the repair at open to the stripes in
  flight: at most members × 126 stripes, about 160 MiB read whatever
  the size of the pool (handoff 15's answer 1).
- **The pool service fails.** The `ublk` device outlives it and holds
  the partitions' transfers; the service started again takes the
  device over and they go on.
- **A pool is lost**: more units failed than its protection covers,
  or the set failed a transfer because no unit was left to serve it
  (a mirror keeps its last unit in service until then). It
  is Failed (`A2007040`), nothing more is written to its remaining
  units (every transfer answers an error), and the partitions that use
  it are ended and left in Error with `QVS2111`. Other pools and their
  partitions run on.
- **A pool cannot be assembled** at the IPL: `A2007010`, and only the
  partitions that use it fail to activate.

### Making, changing and stopping a pool

- **Create**: the disks chosen are checked, every problem at once (not
  the load source, not another pool's, large enough, one logical block
  size, the protection's number of disks, foreign disks confirmed);
  each is made a pool disk, the labels written, the set built (a
  mirrored set with its bitmap, a parity set with its log and its
  parity then computed), the `ublk` device started, xfs written by
  QS/400's `xfsfmt` with 4096-byte sectors, and the pool mounted.
  `C2007015`.
- **Replace a unit** that failed or is missing: the new disk is made a
  pool disk and rebuilt.
- **Add and remove a hot spare.** A spare taken away is not configured
  afterwards.
- **Rename** while no partition uses it: the mount moves.
- **Delete** only when it holds no virtual disk and no media: its disks
  are not configured afterwards.
- **At the host's shutdown**, after the partition manager has ended
  every partition, the service stops each pool in order: the file
  system unmounted, the device stopped, the set flushed and its bitmap
  cleared, the label marked clean (`C2007090`).

## Virtual disks

Raw files in a pool's `vdisks` directory, `<name>.img`. A name is 1
to 32 letters, digits, `-` and `_`, a letter first; a size is whole
megabytes, 1 MiB to 64 TiB.

- **Thin** is sparse: it takes space as the guest writes, and a guest's
  discard (UNMAP, WRITE SAME with UNMAP) gives it back.
- **Thick** is allocated whole when it is made (`fallocate`): a write
  to it never finds the pool full.

Which kind a disk is, is kept on its file (`user.qsvs.vdisk`).

A profile's unit names a disk as `POOL/NAME` (`docs/PROFILE.md`). At
activation the partition manager asks the pool service to hold it for
the partition: the service opens it for direct I/O, records the holder,
and hands the open file over; `qsvmm` is given it as one of its
descriptors. **A disk is held by one partition at a time.** It is
grown only while no partition holds it (upward only), and deleted only
when none holds it and no profile names it, unless the request says to
delete it all the same.

**Over-provisioning.** Thin disks may add up to more than the pool. A
pool reports its size, what is allocated, and what is provisioned, and
posts `A2007050` at 80% allocated and `A2007051` at 90%.

**A pool that fills.** A write that cannot be given space does not
reach the guest as an error. The partition is held: its processors
stop, it stays Running, and `A2005160` is posted. The write is tried
again each second; when it is done, `C2005160` and the partition goes
on. QS/MC sees a held partition as Running with that attention code
(`docs/OPEN_QUESTIONS.md` item 34, confirmed).

**No copy.** A copy of a virtual disk, for a template partition, would
be a reflink; QS/400's `xfsfmt` makes xfs with CRCs, ftype, finobt,
inobtcount and bigtime, and without reflink. So there is none
(`docs/PHASE5.md`).

## The media library

A directory, `media`, in the pool the operator designates as the
media repository (`media/REPOSITORY` marks it, so the designation
survives the state file). There is none until a pool is designated.

```
/pools/<pool>/media/optical/<name>.iso   an optical image
/pools/<pool>/media/tape/<name>.aws      a tape volume's image
/pools/<pool>/media/tape/<name>.toml     its identifier, capacity, protection
```

- **An optical image** is an ISO 9660 image with 2048-byte blocks.
  Imported by a streamed upload on the socket (what QSI carries in
  Phase 8), or from Service Tools from a removable volume or a disc in
  the host's own drive. Either way it is written under a name of its
  own, checked, and renamed when whole: an import cut off leaves
  nothing. An image that is not ISO 9660 is refused. Images may be
  loaded in several partitions at once (they are read only); one that
  is loaded is not deleted or renamed.
- **A tape volume** is an image, its identifier (1 to 6 of `A` to `Z`
  and `0` to `9`), its capacity (16 MiB to 16 TiB) and a
  write-protect flag. A volume is mounted in one drive at a time.

### The tape image format: AWSTAPE

A volume's image is in the AWSTAPE format, which other tools read
(Hercules' `tapecopy`, `awsutil` and others). Each block of data is
one chunk, or several when it is over 65,535 bytes, and each filemark
is a header alone. A chunk's header is six bytes: its length and the
length of the chunk before it, each two bytes little-endian, then a
flags byte (`0x80` the block begins, `0x20` it ends, `0x40` a
filemark) and a zero. The end of the file is the end of data. A
damaged image is read up to its last whole chunk.

A drive's early warning comes a hundredth of the capacity before its
end (1 MiB at least, 64 MiB at most). Writes reach the host's page
cache and are made durable when the guest writes a filemark, asks for
none (its flush), or the volume is unmounted.

## Limits

From QS/400's answers (handoff 15) and this phase's choices; each is
refused with a message.

| What | Limit | Why |
|---|---|---|
| a pool's size | 64 TiB | `xfsfmt` formats at most 64 TiB (`QVS3011`) |
| a parity pool's members | 18 | `protect`'s `MAX_MEMBERS` (`QVS3004`) |
| a mirrored pool's members | 2 | the decision of this phase |
| hot spares | 8 a pool | the label's room (`QVS3017`) |
| a pool disk | 8 MiB record and 256 MiB member at least | (`QVS3008`) |
| a virtual disk | 1 MiB to 64 TiB, whole megabytes | (`QVS3102`) |
| an image | 64 GiB, whole 2048-byte blocks | (`QVS3207`) |
| a tape volume | 16 MiB to 16 TiB | (`QVS3208`) |
| a tape block | 1 MiB | the drive's READ BLOCK LIMITS |

**Memory per TiB of a mirrored pool**: 8 KiB for the bitmap's bits
always, about 50 bytes for each region set, and 128 KiB while a
resume or a copy runs. A parity pool holds up to 16 MiB of pages
between flushes, whatever its size. A tape drive holds 8 bytes for
each block and filemark of its volume.
