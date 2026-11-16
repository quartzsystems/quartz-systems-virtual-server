# Code update

A host's code is in two slots, A and B, on every load-source disk
(`docs/ARCHITECTURE.md`, the slot rule). An update is installed into the
slot that is not running, marked pending, and activated by a restart
into it; the slot then confirms when the base services are ready, or the
host comes back on the other slot as it was. Nothing running is
disturbed before the activation.

## The image

One file, made by `cargo run -p build -- update` (and by `all`) from the
built slots: `images/QSVS-<release>-update-<hash>.upd`, listed among the
artifacts of the signed manifest (kind `update`). The same inputs make
the same bytes.

```text
"QSVSUPDT"            8 bytes
version               u32, little-endian: 1
manifest length       u32, at most 64 KiB
signature length      u32: 64
files                 u32, at most 256
manifest              its text
signature             ed25519, over the manifest's bytes
each file             name length u16, name, size u64, its bytes
```

The manifest (`svc/update/src/manifest.rs`):

```text
QSVS-UPDATE 1
product QS/VS
release V1R0M1
build phase9-1
fix QV00001
state definition.toml 3
state host.toml 1
...
file SLOTA.EFI 15647744 <sha256>
file SLOTB.EFI 15647744 <sha256>
```

`fix` lines name the fixes the release contains (`QVnnnnn`, from
`QSVS_FIXES` at build time, comma-separated); `state` lines each state
file the release reads with the newest schema version it reads
(`update::schemas::this_release`); `file` lines the two UKIs.

The image holds both slots' UKIs, where the handoff named one: a UKI
carries its slot in its baked command line (`qsvs.slot=A` or
`qsvs.slot=B`), so a host installs the UKI of the slot it does not run.

The image is read as hostile: every length bounded before it is used,
nothing allocated by a size it names, a file's range inside the image,
the manifest's every line known. `cargo run -p build -- fuzz --target
update-image` and `--target update-manifest` run them.

## Trust

An image is installed only if its manifest is signed by a key in the
running image's `/qsvs/keys/system.pub` (the build's own key, and
`QSVS_TRUSTED_KEYS`), the same keys the installer trusts its media by.
No setting turns that off. A test image is updated only by a test
image's update: the product is `QS/VS (test image)` in both.

## The checks, in order

| Check | Refused | QSI |
|---|---|---|
| it reads as an update image | `QVS5302` damaged | `QSI1102` |
| its manifest is signed by a trusted key | `QVS5301` | `QSI1101` |
| each UKI is the size and SHA-256 its manifest names | `QVS5302` damaged | `QSI1102` |
| its product is this host's | `QVS5303` | `QSI1103` |
| its release is not older than what runs, unless the stage allowed older | `QVS5304` | `QSI1104` |
| for each state file on the host, the image's release reads its version | `QVS5305`, whatever the stage allowed | `QSI1105` |

The same release again (a rebuild) is not older and is installed.

## The steps

| Step | What happens | Codes |
|---|---|---|
| Stage | the image is written to `/data/update/staged.part` from a console's stream (QSI `Stage`) or a file of a removable volume (Service Tools option 8, 4), exactly the bytes named; then verified. A verified image is kept as `staged.img`; a refused one is not kept. One at a time | `C2012000`, then `C2012010` or `A2012010` |
| Discard | what is staged is thrown away | `C2012015` |
| Install | refused while a slot is pending. The image is read and verified again; the UKI of the slot that does not run is written to the ESP of every load-source disk (QS/400's `fat::write_file`: a new file beside the old one, renamed over it, so the running slot's file is never written), synchronized, read back and compared; then PID 1 marks the slot pending in the A/B record on every disk, with one attempt (installed, not activated). The slot has the image then, and nothing is staged | `C2012020`, `C2012025` for each disk, `C2012040`; `A2012030` when a step failed, with the step and the disk |
| Activate | refused when no slot is pending, and while partitions run unless the request says to shut them down: then each is shut down through its operating system, given the request's timeout, and ended at once after it. PID 1 then spends the slot's attempt in the record, sets the firmware's `BootNext` to the slot's entry, and restarts | `C2012050`, then the shutdown's |
| Cancel | the pending slot is pending no more, and `BootNext` is gone | `C2012045` |
| Confirm | at the trial IPL, when every base service is ready: the record's default and `BootOrder` become the slot (Phase 1's rule), the copy of state is dropped | `C2001095`, `C2012060` with the release now running |
| Roll back | the pending slot never confirmed: it did not start, or its services were not all ready within 15 minutes of its IPL, when PID 1 restarts it; the firmware then boots the default, and that slot clears the pending field, puts the copy of state back, opens the problem and takes a host dump | `A2001035`, `A2012070`, `C2012075`; `C2011000` |

## State across an update

PID 1 does it before the seed or any service reads state
(`lic/qsvsinit/src/ipl/update.rs`):

- The pending slot's first IPL copies `/data/state` to
  `/data/update/state-copy` (written as `state-copy.part`, then renamed).
  A second IPL of the same trial keeps the first copy.
- The slot that confirms drops the copy.
- The slot that comes back after the pending one did not confirm puts it
  back: `state` is renamed aside, the copy renamed to `state`, the aside
  removed. A rolled back host is exactly as it was before the restart,
  whatever the trial's release wrote.
- A copy found by any other IPL is left from an update that ended, and
  dropped.

A release reads state of an older schema by migrating it in memory
(`base/state`); loading never writes. The newer form is written when a
service next saves the file. A file a service saves during the trial is
in the newer form until the trial ends; if the slot does not confirm,
the copy puts the older form back, so the old release never reads what
it cannot.

## What each failure leaves

A power loss at any point leaves the running slot bootable, and the
other slot either complete and pending, or not pending at all.

| Cut | Left |
|---|---|
| while staging | `staged.part`, removed when the service starts; nothing staged |
| while writing the UKI to a disk | the slot's old file, or the new one beside it not yet renamed; the slot is not pending; the running slot's file and directory are not written |
| between disks, or before the record | some disks have the new UKI, none pending: an install that did not finish, done again |
| while the record is written | `abrec` writes the older copy of each disk: the newest valid copy is either the old record or the pending one |
| installed, before activation | the slot stays pending with its attempt unspent: an IPL of the running slot is no rollback |
| after activation, during the trial | the firmware's `BootNext` was spent: the default slot comes up, sees the attempt spent, and rolls back |
| while the state is copied or put back | the copy and the state are each renamed whole; the next IPL finishes what was begun |

## The update service

`svc/update`, run as `lic/qsvsupdd`, a supervised service after the
partition manager. Its directory is `/data/update`: what is staged
(`staged.img`, `staged.json`) and what each slot holds (`slots.json`:
release, build, fixes, how its last activation ended and why). PID 1
owns the A/B record and the firmware's variables; the service asks it
over the control socket to mark a slot pending, cancel it, or activate
it.

Its socket, `/run/qsvsupdd.sock`, root's and the QSI server's group's:
status, stage (the bytes follow its first answer on the connection, and
a second answer says how it ended), stage from a file, discard, install,
activate, cancel, and a subscription to its events (staged, verified,
installing with its progress, installed, activating).

## Over QSI

The `update` family (`docs/QSISERVER.md`): `Status`, `Stage` (a stream,
console to host, of exactly the bytes named: more resets the stream with
`QSI1109`, fewer keeps nothing, `QSI1110`), `Discard`, `Install`
(answered with the slot; progress follows as events), `Activate`
(partitions `Refuse` or `ShutDown`, with a timeout; the activation is the
request's user's), `Cancel`. Its events are the service's, and
`Confirmed` and `RolledBack` after the restart, which the server reads
from PID 1's codes in the history, so a console that reconnects sees
them. Every refusal leads with QSI's identifier (`QSI1101` to
`QSI1114`), then the host's message (`QVS5301` to `QVS5314`).

## At the host

Service Tools option 8, 4 installs an image from a removable volume:
choose the file, see what it is beside what runs, install; then restart
now or later, with the partitions that would be shut down listed and a
typed confirmation. Option 8, 5 displays the code levels.
