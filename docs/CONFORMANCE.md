# A QS/VS host for an outside harness

What a harness outside this repository (QS/MC's, first) needs to start
a QS/VS host with one QS/400 partition and drive it over QSI: the
images to build, how to start them, how to pair, and what the build
machine must be able to nest. Everything named here is at the commit
the harness pins.

## The images

| What | How | Where |
|---|---|---|
| The host's test image | `cargo run -p build -- all --test-image` | `<target>/test-image/images/qsvs-*.img` (a load-source disk already installed, one disk, no protection) and its ISO. A test image carries the test hooks (`qsvshook`, below); a release image never does |
| QS/400's guest image | in a clone of QS/400 at the commit QS/VS pins (`docs/PHASE0.md`): `QS400_TARGET_DIR=<dir> cargo run -p build -- all` | `<dir>/images/qs400-V1R1M0-<hash>.img` (about 35 minutes beside another build). `docs/PHASE4.md` has the steps |
| The catalogue | nothing to build: `docs/catalogue.json` (form `docs/catalogue.schema.json`, `quartz.catalogue/1`), the catalogue of the tree's tables with the build named `source`; a unit test holds it to them (`QVS_CATALOGUE=update cargo test -p build` writes it) | a console builds it into its image as it is. A release carries the same file with its own build ID (`build catalogue FILE` writes it alone) |

`QSVS_TARGET_DIR` and `CARGO_TARGET_DIR` on ext4 (`docs/PHASE0.md`).

## Starting the host

QS/VS's harness does it with `base/qemu` (`qemu::Launch`), which any
harness can take as a git dependency (`docs/ARCHITECTURE.md`, the base
crates):

- **The machine.** q35 with OVMF and a copy of the variable store
  (`QSVS_OVMF_CODE`, `QSVS_OVMF_VARS`), `-enable-kvm -cpu host`, QEMU's
  `intel-iommu` (`QSVS_NO_IOMMU=1` leaves it out). For a partition: 5120
  MiB preallocated and 6 processors (one is the host's own; a QS/400
  partition is given 2048 MiB).
- **Its disks.** The test image's disk, copied, as the first; the QS/400
  image, sparsely copied, as the next (serial `QSVSUNIT0`, the host's
  `/dev/vdb`); a pool's blank disks after, if any.
- **Its network.** One NIC on QEMU's user network with the management
  MAC address the suites use (`52:54:00:12:34:56`), DHCP from QEMU, and a
  host port forwarded to the guest's TCP 2300 (`release::QSI_PORT`).
  A console reaches the host's QSI server at `127.0.0.1:<port>`.
- **Its serial port** is the boot's log and Service Tools' second
  session; the test hooks' channel is a second serial port
  (`qsvshook::proto::PORT`, `/dev/ttyS1` in the guest).
- **Ready** when the history has `C2010000` (the QSI server started:
  `refcode::QSI_STARTED`) and PID 1's status says every service ready.

The partition is made over QSI like any other: partition `Create` or
`CreateWith`, then a profile put whose LUN 0 is the QS/400 disk. On a
test image a profile may name a host path as its backing store
(`/dev/vdb`); a release image accepts only a storage pool's virtual disk
(`POOL/NAME`). `Activate` and the partitions family's events follow it
to Running (QS/400 reports its IPL complete about 20 s after it starts
in WSL with KVM).

## Pairing

A console pairs once, with a password shown at Service Tools: sign on
at the serial port or the display as the installed user (the suites'
installation makes `ADMIN` with the password `service tools 1`), option
4, Management
console pairing, then 2, Start pairing; the screen shows the password
and the host's certificate's fingerprint. QS/VS's harness reads both
from the screen (`suite::qsi::start_pairing`). The password is kept in
memory only, for 15 minutes or five attempts; then the console pairs
with QSI's `pair` family (`qsitrust`) and is pinned. The serial port
shows Service Tools as text, so a harness reads the password there.

Two consoles at most are pinned; a QSI session closes after 45 s with
nothing heard (`qsisess`), so a harness that waits long keeps it busy.

## What the build machine must nest

The host is a KVM guest, and its partitions are KVM guests in it: the
machine the harness runs on must give its own guests virtualization.
Under WSL 2 that is the nested virtualization WSL gives (on by default
on recent Windows), and `/dev/kvm` must open for the harness's user.
A console running in a second QEMU beside the host adds no level; a
QS/400 partition is the third level from Windows (Windows, WSL, the
host, the partition), which works but runs slowly under load: give the
host the 5 GiB it asks and run one such host at a time
(`flock /tmp/qsvs-fu-qemu.lock`, which QS/VS's and QS/MC's suites share).
`QSVS_BOOT_TIMEOUT` raises the boot's limit on a loaded machine.

## What QS/VS's own suites show of it

`qsi-*` (`tools/conformance/src/suite/qsi`): pairing and trust, every
family against the services, the events and their resumption, two
consoles, the streams, a hostile console, and `qsi-additions`, QS/400's
handoff 24 from a console (bits 19 to 26) with a restart after it.
