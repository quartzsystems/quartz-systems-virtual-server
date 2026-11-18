# Phase 10: physical I/O and tape pass-through

The record of Phase 10. Phases 0 to 9 are `docs/PHASE0.md` to
`docs/PHASE9.md`; what they say still holds, except where this says
what changed.

Until Phase 10 a partition had virtual devices only. Phase 10 lets a
host give a partition a whole PCI device with the rest of its IOMMU
group (`docs/PHYSICALIO.md`), pass a physical tape drive through to a
partition's tape device command by command (`docs/STORAGE.md`), and
change a running shared partition's entitlement (`docs/RESOURCES.md`).
Its first part, Section 1, was QS/400 against the virtual tape drive
of Phase 5 (`docs/QS400_GUEST.md`).

## Step 0

### The pin

QS/400 `2e950f9f641da2adf38feba0b75cd3a267bba657`, its handoff 20
(QSI's physical I/O: the host family's `Devices` and its events, vres's
`TapeDrives` and the `PhysicalTape` drive kind, a profile's
`physical_io` and a tape device's `@` drive, the partition family's
version 2 with `Entitlement`, the `PHYSICAL_IO` and `PARTITION_V2`
capabilities, the refusals `QSI1301` to `QSI1307` and `QSI1401` to
`QSI1405`), on QS/400's `main`. Every `rev =` moved to it in one commit
of its own (`5734221`), before anything else of the phase. The kernel
pin was already QS/400's.

| QS/400 change | Here |
|---|---|
| `qsi::host::host` gains `Devices` and five device events | the pin's commit refused it as not agreed, since the hello did not offer `PHYSICAL_IO`; `68409a6`'s server answers it from the partition manager |
| `qsi::host::vres` gains `TapeDrives`, `TapeDrive`, `DriveKind::PhysicalTape` | the same; answered from the partition manager |
| `qsi::host::partition` gains version 2's `Entitlement` and its event | the same; answered as the request's user |
| `qsi::host::model::Profile` gains `physical_io` | converted both ways (`qsi/server/src/families/convert.rs`) |
| nothing else | the pin broke only the server's three exhaustive matches over the host, vres and partition families; format, both lint runs and every unit test passed at `5734221` |

### Every earlier suite on the new pin

In a clone at `5734221` with its own target, from images built there,
one suite at a time: 123 of the 130 suites of Phases 0 to 9 passed in
the first run, and the other 7 when each ran again alone
(`partition-manager-restart`, `partition-identity`,
`pool-pulled-mirrored`, `net-internal`, `net-service-kill`,
`service-silent`, `qsi-two-consoles`). Each of the 7 had failed by time
while a build or another suite's QEMU ran beside it; two of them by
QS/400's own storage service ending in its partition (`B1001040`,
`B1001060`), which did not happen again. The harness then gave no host
an IOMMU; the acceptance run did (below).

The answers to the open questions given with the phase: 44 kept as
recommended (closed, `docs/OPEN_QUESTIONS.md`); 36 by Section 1
(below); 6, 37, 40 and 43 unchanged.

## What exists

### Physical I/O (`part/pio`, `lic/hostinv`)

- `hostinv` reads each PCI function's IOMMU group, its reset methods,
  its interrupt kinds and whether it is the boot display, and whether
  the host's IOMMU translates and remaps interrupts.
- `pio::device` says what the host uses each device for (a load
  source's or a pool's controller, the management interface, an
  uplink, the display, a bridge) and every reason one cannot be
  released, by the decisions table's rules; `pio::released` keeps
  `physio.toml`; `pio::assign` is what an activation is given of a
  profile's physical I/O; `pio::bind` is the kernel's PCI files.
- PID 1 binds the released devices to `vfio-pci` at the IPL, after the
  partitions' memory and before any service, or posts why not
  (`C2013020`, `A2013030`, `A2013035`), and posts `A2013010` when the
  IOMMU or interrupt remapping is off.

### The partition manager (`part/partd`)

- Requests 39 to 43: devices, release, take back, entitlement, tape
  drives (`docs/PARTD.md`); events device, tape drive, entitlement.
- Activation gives each named group whole, reset, opened through VFIO
  as root; the end of a VMM resets each function and gives it back; a
  reset that fails fences the device until the host restarts
  (`A2013050`).
- A physical tape drive's relay (`serve/sg.rs`): `SG_IO` on the drive's
  generic device, the allow-list checked again, `A2013080` when the
  drive goes away.
- A running shared partition's entitlement through `cpu.max` and
  `cpu.weight` alone (`part/resource/src/entitle.rs`), `C2013040`.

### The VMM (`vmm/devices`, `vmm/qsvmm`)

- `devices::sys::vfio`: the container, groups and devices, their
  regions and interrupts, over `vfio-bindings` (MIT/Apache, no C).
- `devices::pci::passthrough`: a host's function on the partition's
  bus, its configuration space presented (`docs/PHYSICALIO.md`), its
  memory BARs placed top-down and mapped into the guest but for the
  MSI-X pages, MSI and MSI-X through irqfd, bus mastering held until
  the memory is mapped for DMA.
- `qsvmm` maps all of the partition's memory into the container, adds
  the groups to KVM's VFIO device, and places the functions after the
  virtio devices; it is handed every descriptor and opens nothing of
  VFIO itself.
- `devices::virtio::scsi::passed`: a physical drive's tape device: the
  allow-list, then a frame to the relay.

### Tape pass-through (`stg/tape/src/passthrough.rs`)

The allow-list, its refusals and their sense data, the relay's frames
and their bounds, the `Drive` trait and a double.

### Service Tools option 10 (`svc/tools/src/physio.rs`)

`Work with I/O for partitions`: every device and drive, release with
the location typed, take back, display; 14 rows a page. The other
options keep their numbers (`docs/SERVICETOOLS.md`).

### QSI and `qsictl`

The server offers `PHYSICAL_IO` and `PARTITION_V2`
(`docs/QSISERVER.md`); `qsictl io`, `qsictl tape` and `qsictl partition
entitlement` (`docs/QSICTL.md`).

### Messages and codes

`QVS2401` to `QVS2422` (devices and drives), `QVS2501` to `QVS2507`
(entitlement), `QVS3301` and `QVS3302` (a drive), `QVS5150` to
`QVS5154` (problems); `C2013000` to `C2013050`, `A2013010` to
`A2013080` (`docs/MESSAGES.md`, `docs/REFCODES.md`, `docs/PROBLEMS.md`).

## Kernel additions

`kernel/config/qsvs.config`: `INTEL_IOMMU_DEFAULT_ON`,
`IOMMU_DEFAULT_DMA_LAZY`, `X86_X2APIC`, `VFIO_CONTAINER`. Added to the
build's required list with them: `VFIO_IOMMU_TYPE1` and `KVM_VFIO`,
which the config selects. Already there since Phase 0: `VFIO`,
`VFIO_PCI`, `INTEL_IOMMU`, `AMD_IOMMU`, `IRQ_REMAP`. The baked command
line is unchanged.

The test image's kernel alone (`kernel/config/qsvs-test.config`, a
fragment after the release's, never in a release image): `SCSI_DEBUG`,
the tape double; `DEBUG_FS`, `IOMMU_DEBUGFS`, `INTEL_IOMMU_DEBUGFS`, the
IOMMU's own record of its mappings for the isolation suite. Its command
line adds `scsi_debug.add_host=0`, so the double makes no device until
the hooks ask for one. A unit test of the build holds both apart.

## Running

```
cargo run -p build -- all --test-image
export QSVS_QS400_IMAGE=... QSVS_QS400_ISO=...
cargo build -p qsictl
cargo run -p build -- test --suite pio-nvme
QSVS_GOLDEN=update cargo run -p build -- test --suite pio-tools
cargo run -p build -- fuzz --target passthrough --secs 60
cargo run -p build -- fuzz --target passed-tape --secs 60
```

Every test host now has QEMU's `intel-iommu` with interrupt remapping
(and `kernel-irqchip=split`), and the physical I/O suites give it an
`e1000e` at `00:1a.0`, an NVMe controller at `00:1b.0`, two NVMe
functions sharing a group at `00:1c`, and at `00:1d` a virtio disk with
an NVMe function beside it. `QSVS_NO_IOMMU=1` takes the IOMMU away from
every host but a suite's that asks for it. The partitions of the
physical I/O suites are copies of the test image, whose PID 1 says on
its serial port what it was given (`PIO GUEST`, `PIO DISK`, `PIO TAPE`
lines, then `PIO DONE`); the tape suites use `scsi_debug` in the host as
the physical drive.

Linux takes any Intel root-complex endpoint for isolated
(`pci_quirk_rciep_acs`), so two emulated Intel functions never share a
group: the shared groups are of QEMU's NVMe (`1b36`).

## The IOMMU's cost

Phase 6's baseline (`net-throughput`: ICMP echoes from the harness's
station through an uplink to a partition and back, rounds of 50, three
levels deep) at `1651bcd`, once with every host's emulated IOMMU and
interrupt remapping on and once with `QSVS_NO_IOMMU=1`, one after the
other on the same machine:

| Echo size | IOMMU on | IOMMU off | Phase 6 |
|---|---|---|---|
| 1472 bytes | 4,939 replies/s, 119.7 Mb/s | 4,171 replies/s, 101.0 Mb/s | 5,742 replies/s, 139.1 Mb/s |
| 56 bytes | 11,044 replies/s, 17.3 Mb/s | 10,913 replies/s, 17.1 Mb/s | 10,430 replies/s, 16.4 Mb/s |

No cost can be told from the noise between runs: the run without the
IOMMU was the slower. That is what the design gives. A partition's
frames go through vhost-net from its tap, in the host's memory: no
device of the host does DMA for them. The uplink here is a virtio
device, which QEMU does not put behind its IOMMU. A real host's uplink
NIC is translated, lazily (`IOMMU_DEFAULT_DMA_LAZY`); what that costs a
physical adapter is the Dell's to measure (`NEEDS-CODY` below).

## Section 1: QS/400 on the virtual tape drive

Done first, and reported in `docs/QS400_GUEST.md` ("Saving and
restoring on tape"). `INZTAP`, `SAVLIB`, `DSPTAP` and `RSTLIB` work
against the drive from one partition to another; a save and a restore
span two volumes past the early warning, the system unloading the first
and the host mounting the second; a protected volume and one with no
labels are refused as they should be. The drive needed no change.

Three findings are QS/400's (T1 to T3 there, items 10 to 12 of its
list for a QS/400 handoff): a blank volume is a device error (T2); a
save to another system's volume saves nothing without saying why (T3);
the messages sent are not the catalogue's (T1). The fourth, T4, is not
yet anyone's: during `INZTAP CHECK(*YES)` of a volume labelled in ASCII
the partition's kernel failed, twice, and whether the guest kernel's
tape path, the drive's answers or the nesting caused it is
`NEEDS-CODY`. `pio-tape-volumes` is blocked by T4; question 36 waits on
QS/400 for T1 to T3 and on T4 being placed (`docs/OPEN_QUESTIONS.md`).

### Faults of a partition's kernel

A partition three levels deep (WSL's kernel, QEMU with KVM, the
partition with KVM in the test host) has had its kernel fault now and
then since Phase 4, in unrelated places each time: `msi_domain_get_virq`
(Phase 4), `redirected_tty_write`, `clear_page_erms` (Phase 6),
`rb_erase` (Phase 7), `native_flush_tlb_one_user` (Phase 8), and in
Phase 10's runs `__rmqueue_pcplist`, `qdisc_alloc` and
`update_entity_lag` while QS/400 brought its network links up, and the
`rb_erase` of T4. Each was a suite that passed when run again. Faults in
memory-management and scheduler structures at random places are what
memory or TLB corruption looks like, and nothing of Phase 10 runs in a
partition without physical I/O. The suspects are nested virtualization
on WSL's Hyper-V and the VMM. `NEEDS-CODY`: QS/400 partitions IPLed
many times over on the Dell, two levels deep, show which (carried from
Phase 9).

## Conformance

| # | Suite | What it shows |
|---|---|---|
| 1 | every earlier suite | Step 0, and in the acceptance run with every host's IOMMU on (below) |
| 2 | `pio-tape-save`, `pio-tape-span`, `pio-tape-volumes` | Section 1 (above): the first two pass; the third records every outcome and is blocked by QS/400's T4 |
| 3 | `pio-tools` | option 10's screens against `tools-io`, `tools-io-display`, `tools-io-release`, `tools-io-released`; the adapter released with its location typed is no interface of the host nor in its inventory; a partition whose profile names it took `10.0.2.15` through it |
| 4 | `pio-nvme` | the NVMe controller given to FIRST, whose system found the disk blank, wrote and read back; reset and given back (`C2013035`); SECOND found FIRST's marker and wrote its own |
| 5 | `pio-refused` | the load source's controller, a pool member's, the management interface's and an uplink's, each refused with `QVS2406` and its use; the function beside the pool member's in its group with `QVS2407` naming it |
| 6 | `pio-group` | releasing one function of the shared group released both; a profile of one alone refused with `QVS2404` naming the other; both given whole |
| 7 | `pio-iommu-off` | `A2013010` and its problem; a release refused with `QVS2410`; a partition with no physical I/O ran |
| 8 | `pio-held` | required and held: `QVS2403` naming FIRST; not required: SECOND activated without it, FIRST still holding it |
| 9 | `pio-reset` | a killed `qsvmm`: the controller reset, given back and given again; a reset made to fail: `A2013050`, its problem, the device fenced even without the hook, bound and given after a restart |
| 10 | `pio-restart` | the released controller bound to `vfio-pci` at the IPL before the services started; the released adapter taken out of the machine: `A2013030` and its problem, and a partition that did not require it ran |
| 11 | `pio-isolation` | the Intel IOMMU's debugfs record of the controller's page tables: `0x0+0x20000000`, the partition's 512 MiB at guest address 0, and nothing else; nothing after the partition ended |
| 12 | `pio-hostile`, `fuzz --target passthrough` | a guest's 9,210 configuration space writes (42 by name, 9,168 at random, every width): the other partition ran on in its VMM, the manager answered within 20 ms, every service ready; the controller reset and used by the next partition; the fuzz target 119,962 runs in 60 s, no finding |
| 13 | `pio-tape-double`, `fuzz --target passed-tape` | `scsi_debug` as a tape drive released and given: INQUIRY and data through `st` as the drive gave them; WRITE BUFFER refused in front of the drive (`5/20`) and never reached it (no microcode-changed attention after); a second holder refused (`QVS2403`); the drive deleted: `A2013080`, its problem, the partition's next command a transport failure; the fuzz target 122,305 runs in 60 s, no finding |
| 14 | `pio-entitlement` | capped at 0.50 units the partition used 0.500 of a core; raised to 1.00 with the same VMM, 1.000; 1.20 refused (`QVS2503`), 0.80 beside another's 0.30 in a pool of one core (`QVS2505`), a dedicated partition (`QVS2502`); the next activation's `cpu.max` the profile's |
| 15 | `pio-qsi` | over QSI: the devices as the host has them, before and after a release; a partition made, given the controller and the drive and activated; a second refused `QSI1303`; units changed, 1.20 refused `QSI1403`; device, drive and entitlement events; `qsictl io`, `tape` and `partition entitlement` |
| 16 | every earlier suite, `pio-iommu-off` | in the acceptance run with the IOMMU on; a partition with no physical I/O as before |

What the runs found and what was done, beside Section 1's:

| Found | Done |
|---|---|
| `partd`'s `SG_IO` request was typed for glibc and did not build for musl | `libc::Ioctl` (`f02d595`) |
| emulated Intel functions never share an IOMMU group: Linux takes an Intel root-complex endpoint for isolated (`pci_quirk_rciep_acs`) | the harness's shared groups are QEMU's NVMe (`13f84e2`) |
| `scsi_debug`, built in, made a disk at every IPL of the test image | the test image's command line has `scsi_debug.add_host=0` (`9d99d6a`) |
| option 10's list cut its descriptions and uses mid-word, and showed 15 rows of a host's dozens | a page of 14 with F7 and F8, kinds and short uses (`d3bee17`, `fd3f6f9`) |
| a released adapter stayed in the inventory until the next IPL, and after it | PID 1 writes the inventory again when a release is posted and after binding at the IPL (`3664f31`) |
| `qsictl` did not offer `PHYSICAL_IO` and `PARTITION_V2`, so its new commands were refused | it offers both (`20e50fd`) |
| the relay logged a refused command in its own words | `QVS3302` (`ef64449`) |
| the harness's own mistakes: the problem book's path, a code posted during the IPL looked for after it, two partitions on one unit, the guest probe writing to the display's console, a QSH line whose digits named a descriptor | each in its suite |

## For a real machine (NEEDS-CODY)

- `NEEDS-CODY`: the QLE2562 given whole to a partition on the Dell, and
  QS/400 saving to the VTL through it (`SAVLIB`, `RSTLIB`, a save across
  two cartridges), the HBA reset between partitions.
- `NEEDS-CODY`: the same VTL drive passed through from a controller the
  host keeps (`@` and its location in a tape device), the allow-list's
  log read for anything QS/400's driver sends that it refuses.
- `NEEDS-CODY`: a network adapter of the Dell given to a partition,
  which takes an address through it; the IOMMU's cost on the Dell's
  uplink throughput.
- `NEEDS-CODY`: the Dell's isolation groups as they are
  (`/sys/kernel/iommu_groups`), with Service Tools option 10's list and
  Display of each, and which devices come out never releasable.
- `NEEDS-CODY`: the seconds a released device's built-in host driver has
  it at each IPL before PID 1 binds it to `vfio-pci`
  (`docs/PHYSICALIO.md`, "At the IPL"); whether the QLE2562's driver
  logging in to the fabric for them matters.
- `NEEDS-CODY`: T4 (`docs/QS400_GUEST.md`) run two levels deep on the
  Dell and against a real drive passed through, to place it; QS/400's
  findings T1 to T3 for its handoff.
- `NEEDS-CODY`: the intermittent faults of a partition's kernel three
  levels deep (above, "Faults of a partition's kernel") looked for on
  the Dell, two levels deep.
- Carried from Phase 9: an update, a pool disk pulled by hand, a host
  dump copied out, pairing at the Dell and QS/400 IPLed many times over
  (`docs/PHASE9.md`); the joint gate with QS/MC is open
  (`docs/OPEN_QUESTIONS.md` item 40).

## Not done

- `BLOCKED(phase10)`: `pio-tape-volumes` fails at its last step by T4
  (`docs/QS400_GUEST.md`), not yet placed.
- `BLOCKED(phase10)`, carried: a VMM found running by a partition
  manager started again is watched by looking at its control group every
  25 ms (`docs/PARTD.md`); an adapter's unit numbers have no gaps
  (`docs/PROFILE.md`); cores are given without regard to sockets or
  memory's place, and only 2 MB pages are used (`docs/RESOURCES.md`).
- Not in Phase 10, and not started: adding or taking away a device, a
  processor or memory on a running partition; virtual functions; media
  changers; a display adapter for a partition; release engineering.
