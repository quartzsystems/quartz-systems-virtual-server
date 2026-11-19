# The pool service

`poold` (`stg/poold`, run as QS/VS by `lic/qsvspoold`) keeps the
host's storage pools, their virtual disks and the media library
(`docs/STORAGE.md`). It is a supervised service of PID 1's table,
started after the network service and before the partition manager,
which asks it for a partition's disks and media; it is stopped after
the partition manager, so every partition has ended before a pool is.

It is driven over a local socket. Nothing listens on the network: the
QSI host server of Phase 8 will sit on this socket, and its storage
family maps onto the requests below one to one. In a release image
Service Tools asks it for disks and pools, and the partition manager
for what a partition holds; nothing else sends it a request. In a test
image the test hooks carry the conformance harness's requests to it.

## The socket

`/run/qsvspoold.sock`, a Unix stream socket, root's alone. One request
and one answer per connection, each a frame of QS/400's `ipc` (a
length and the encoded value), as the partition manager's. A request
is a two-byte number and its arguments; an answer is the request's own
type. A request that is refused answers with **every** message that
applies (`docs/MESSAGES.md`, `QVS30xx` to `QVS32xx`) and changes
nothing. The crate's client waits up to 120 s for an answer: making a
pool formats its file system.

Two requests carry more than frames:

- **hold** (22): after an answer that gives backing stores, their open
  files follow on bytes of their own, sixteen a byte (`SCM_RIGHTS`,
  `base/fdpass`), in the answer's order.
- **upload** (14): after an `Ok` answer the client sends the image's
  bytes, exactly as many as it said, and the service answers a second
  time when they are checked and named. A connection that ends early
  leaves nothing.

| # | Request | Arguments | Answer | Refused when |
|---|---|---|---|---|
| 0 | disks | | every whole disk of the host: name, serial, location, size, logical block size, what it is, its pool, whether it is free, whether it is foreign | |
| 1 | pools | | every pool: name, UUID, protection, state, the work and its unit and percent, size, allocated, provisioned, mount point, whether it is the repository, its units, why it failed | |
| 2 | create pool | name, protection, members, hot spares, confirmed | the pool | the name (`QVS3001`, `QVS3002`); a disk not there, taken, foreign and not confirmed, too small (`QVS3005` to `QVS3008`); block sizes (`QVS3009`); the protection's number of disks (`QVS3004`); a spare (`QVS3010`, `QVS3017`); the size (`QVS3011`); a step failed (`QVS3016`) |
| 3 | delete pool | name | | not there (`QVS3003`); it holds a virtual disk or media (`QVS3012`) |
| 4 | rename pool | name, new name | | the new name; a partition holds something in it (`QVS3013`) |
| 5 | replace unit | pool, unit, disk, confirmed | | the unit is not failed or missing (`QVS3015`); the disk |
| 6 | add hot spare | pool, disk, confirmed | | no protection, eight already (`QVS3017`); the disk |
| 7 | remove hot spare | pool, disk | | not one of its spares (`QVS3017`) |
| 8 | designate | pool, or empty for none | | the pool is not usable (`QVS3014`); the library's media are in use (`QVS3205`) |
| 9 | virtual disks | pool, or empty for every pool | each: pool, name, size, allocated, thick, holder (partition and slot) | |
| 10 | create virtual disk | pool, name, MiB, thick | the disk | the pool (`QVS3003`, `QVS3014`); the name, the size, one there (`QVS3101` to `QVS3103`); no room for a thick disk (`QVS3107`) |
| 11 | resize virtual disk | pool, name, MiB | the disk | held (`QVS3105`); smaller (`QVS3106`); no room |
| 12 | delete virtual disk | pool, name, forced | | held (`QVS3105`); a profile names it and it is not forced (`QVS3108`, one for each) |
| 13 | images | | each: name, size, volume identifier, holder | |
| 14 | upload | name, bytes | first `()`, then the image | no repository (`QVS3201`); the name, one there (`QVS3202`, `QVS3203`); the size (`QVS3207`); not ISO 9660 (`QVS3206`) |
| 15 | rename image | name, new name | | loaded (`QVS3205`); the names |
| 16 | delete image | name | | loaded (`QVS3205`) |
| 17 | import image | device, path on it, name | | an import under way (`QVS3209`); no repository; the name. The import runs on; `C2007050` or `A2007060` says how it ended |
| 18 | tape volumes | | each: name, identifier, capacity, used, protected, holder | |
| 19 | create tape volume | name, identifier, MiB | the volume | the name; the identifier or capacity (`QVS3208`) |
| 20 | delete tape volume | name | | mounted (`QVS3205`) |
| 21 | protect tape volume | name, on | | mounted |
| 22 | hold | partition, wanted | each given, and the open files after | every one that cannot be given: a pool not usable, a disk not there or another partition's, no repository, a medium not there, a volume mounted elsewhere. Every one is given or none |
| 23 | release | partition, wanted (empty: all) | | |
| 24 | holders | | everything held | |
| 25 | stop all | | how many pools were stopped | |

A **wanted** item is its kind (a virtual disk, an image, a volume),
its pool (for a disk), its name and the slot it is for. A partition
holding a kind in a slot again replaces what it held there. Images are
read only and may be held by several partitions; a disk and a volume
by one at a time. The holders are kept in `/run/poold/holders`, so a
service started again knows them; `/run` does not outlive an IPL,
and neither does a partition.

## Reference codes

The `C2007xxx` and `A2007xxx` block (`docs/REFCODES.md`): the service
started, a pool assembled or made, resynchronization and rebuild
started and finished, a hot spare taken, an image imported, the pools
stopped; a pool not assembled, a unit failed, a rebuild not finished,
a pool failed, a pool 80% and 90% allocated, an import that failed.

## What the partition manager does with it

At activation the partition manager asks it to hold the profile's
virtual disks (`POOL/NAME`) and the media its optical and tape devices
load, all at once, and gives the files to `qsvmm` as its descriptors 3
and on (`docs/PARTD.md`). When the VMM ends, everything the partition
held is released. A medium loaded or unloaded while the partition runs
is held or released by itself. The partition manager asks for the
pools' states every two seconds; a pool that is Failed has every
running partition with a disk in it ended and left in Error.
