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

Three findings are QS/400's (T1, T2 and T5 there, items 9 to 11 of
its list for a QS/400 handoff): a slow request ends the storage service
and the system (T5, seen in the acceptance run); a blank volume is a
device error (T2); the messages sent are not the catalogue's (T1). Two
entries made on the way were withdrawn there: a save that saved nothing
was of an empty library, and a fault of the partition's kernel during
the check of a volume with ASCII labels, seen twice with Phase 8's
media, did not happen with the pin's and is one of the faults below.
All three tape suites pass. Question 36 waits on QS/400 for T1, T2 and
T5 (`docs/OPEN_QUESTIONS.md`).

### Faults of a partition's kernel

A partition three levels deep (WSL's kernel, QEMU with KVM, the
partition with KVM in the test host) has had its kernel fault now and
then since Phase 4, in unrelated places each time: `msi_domain_get_virq`
(Phase 4), `redirected_tty_write`, `clear_page_erms` (Phase 6),
`rb_erase` (Phase 7), `native_flush_tlb_one_user` (Phase 8), and in
Phase 10's runs `__rmqueue_pcplist`, `qdisc_alloc` and
`update_entity_lag` while QS/400 brought its network links up, and an
`rb_erase` during a tape check. Each was a suite that passed when run again. Faults in
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
| 2 | `pio-tape-save`, `pio-tape-span`, `pio-tape-volumes` | Section 1 (above): each passes, recording what QS/400 answered |
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

## Hashes

The acceptance run's builds at `4e234ac`, `QSVS_BUILD_ID=phase10`, the
signing key of the reproducibility runs; both clean builds gave every
one of these, and the 22 files with them, byte for byte:

| File | SHA-256 |
|---|---|
| release slot A UKI `qsvs-V1R0M0-a-4bd26f32586e.efi` | `4bd26f32586ed58b63fa46b0c53958b47d60bf4358bcf9860bf1bf4ac8d2b83e` |
| release slot B UKI `qsvs-V1R0M0-b-ad92ba6a8e61.efi` | `ad92ba6a8e613d200ccac38318f62b25ba673982d9905a7d879df3429b03c6b6` |
| release media UKI `qsvs-V1R0M0-d-3bf4f462c99d.efi` | `3bf4f462c99d0d6fb78e4ac2b7d9d5c2a5ea7903597a4627fe333e1bef3f9b33` |
| release disk image `qsvs-V1R0M0-4bd26f32586e.img` | `64ed8414bef57b2ed82c404aab9f195db0701c776a057ba91eae31d8189db3a9` |
| release ISO `QSVS-V1R0M0-3bf4f462c99d.iso` | `0bee6d8fc29b7aa3ded451e3095b5abbb716125469637d3a6be7d641e3302198` |
| release update image `QSVS-V1R0M0-update-cb51c507f72b.upd` | `cb51c507f72b1db5261646b06c2f82c92e32fb835e8df7399a36ad7b8da98ead` |
| release `manifest.json` | `89c9770a4b1cc1107234d34113c4c4e7faff4efdfc08a1763e39660321b5caff` |
| test image's slot A UKI `qsvs-V1R0M0-a-26ebe8c6c0c3.efi` | `26ebe8c6c0c3730208cc3f1f9240a341834eda7772dbf64b4e1b5db8c19747ae` |
| test image's disk image `qsvs-V1R0M0-26ebe8c6c0c3.img` | `1b33ef3799240e17f6bd471cd310931516786fd4cf0061f82819a2a8141dedd4` |
| test image's ISO `QSVS-V1R0M0-27d963f433fc.iso` | `5f00c8c67068e919d3a1f6746b5102002706ea27d8eeb09e0ca3114c8a672822` |
| test image's update image `QSVS-V1R0M0-update-01796115bc0f.upd` | `01796115bc0fc8d34c4de139e6df089785a2cc3bdb33d9bf67b6527b80e16869` |

The two builds must be made at one target path: the kernel's
configuration, which the manifest hashes, names its initramfs by its
path in the target (`CONFIG_INITRAMFS_SOURCE`). Built in two targets of
different names, every image was the same and the manifests differed in
that line alone.

## Acceptance

The acceptance run, in a fresh clone at `4e234ac` on ext4 in WSL (rustc
1.98.0, clang 21.1.8, QEMU 10.2.1, cargo-deny 0.20.2, WSL's
6.18.33.2 kernel, 12 processors), with QS/400's guest image and
installation ISO built at the pin (`2e950f9`), a `V1R0M1` test image's
update signed by the same key, and every host's emulated IOMMU on:

| Step | Result |
|---|---|
| two clean builds, each with both update images | 1041 s and 1267 s; byte-identical, 22 files |
| `doctor`, `deny`, `notices --check` | pass |
| `cargo fmt`, both clippy runs | clean |
| unit tests | 732 passed, none failed |
| `qsictl` | built; in neither image |
| the suites of Phases 0 to 10 | 145: 129 passed in the first pass; the 16 that did not, and the 8 that install an update image, alone again: 140 of 145 after that, 143 after a third run of the 5 left, all 145 after a fourth of the 2 left |
| `test --media iso` | the installer's welcome in 2.6 s |
| the fuzz targets, 60 s each | 20 targets, from 37,271 to 127,535 runs each, no finding |

Every failure of the suites in the acceptance run was a time out or a
partition's own system failing, never a refusal or a wrong answer of
the host's, and each passed when run alone:

| Why | Suites |
|---|---|
| QS/400's storage service ended after a slow request (QS/400's T5) or its object store was unusable at the IPL | `partition-attributes`, `service-states`, `service-manager-restart`, `pio-tape-save`, `pio-tape-volumes` |
| a fault of the partition's kernel, three levels deep ("Faults of a partition's kernel") | `partition-ipl-a`, `service-hostile`, `service-auto-wait`, `service-system` |
| a host or a system later than a suite's time out, while another session's QEMU and benchmarks ran on the same machine | `pio-refused`, `pool-pulled-mirrored`, `qsi-end-to-end`, `svc-update-back`, `svc-update-power` |
| the harness: keys typed into QS/400's Command Entry lost while it drew, a line cut by a kernel message | `pio-tape-span`, `pio-tape-double` (both fixed in the harness, `085591b`, `01873f2`, `6c634bc`) |
| the acceptance script: the update image of `V1R0M0` taken for `V1R0M1`'s | `svc-tools` and the update suites, run again with `V1R0M1`'s |

The reruns were made with the clone taken to `4e234ac`'s successors
`23e8351` and `01873f2`, which changed nothing but the harness and the documents
since `4e234ac`; the images were not built again. Before the pin's
QS/400 media were built, the same run with Phase 8's media had passed
86 suites before those media were taken away by another session
mid-run; its results were set aside, not counted.

| # | Check | Evidence |
|---|---|---|
| 1 | Step 0 is committed on its own and every Phase 0 to 9 suite passes on the new pin with the IOMMU on | `5734221` alone; at it, all 130 (123, then 7 alone; no host IOMMU in the harness then); at `4e234ac` with every host's IOMMU on, every Phase 0 to 9 suite in the acceptance run (above) |
| 2 | Suites 3 to 16 pass | the conformance table above; in the acceptance run every Phase 10 suite, `pio-refused` and `pio-tape-double` on a second run, the tape suites as below |
| 3 | Suite 2 passes, or each part is blocked by a named QS/400 finding | `pio-tape-save`, `pio-tape-span`, `pio-tape-volumes` pass, each recording what QS/400 answered; QS/400's findings T1, T2 and T5 are recorded and block nothing (`docs/QS400_GUEST.md`) |
| 4 | Open questions 36 and 44 are closed or moved to "waiting on QS/400" with what is waited for | 44 closed; 36 under "Decided, waiting on QS/400", waiting for T1, T2 and T5 (`docs/OPEN_QUESTIONS.md`) |
| 5 | Two clean builds are byte-identical; `build deny` and `build doctor` pass | "Hashes" above; `deny` and `doctor` pass |
| 6 | Every QS/400 crate is pinned at the one new commit; no QSI message is restated | every `rev =` is `2e950f9`; the server's devices, drives and entitlement are `qsi`'s types filled from the partition manager's (`qsi/server/src/families/convert.rs`) |
| 7 | No setting relaxes a safety rule of the decisions table | the rules are checked in `pio::device::assess` and at assignment with no switch, profile field, request or variable over them; the one test hook (`/run/qsvshook.noreset`, a test image's alone) makes a reset fail, which only fences more |

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
- `NEEDS-CODY`: QS/400's findings T1, T2 and T5 for its handoff
  (`docs/QS400_GUEST.md`); whether the Dell, two levels deep, ever sees
  T5.
- `NEEDS-CODY`: the intermittent faults of a partition's kernel three
  levels deep (above, "Faults of a partition's kernel") looked for on
  the Dell, two levels deep.
- Carried from Phase 9: an update, a pool disk pulled by hand, a host
  dump copied out, pairing at the Dell and QS/400 IPLed many times over
  (`docs/PHASE9.md`); the joint gate with QS/MC is open
  (`docs/OPEN_QUESTIONS.md` item 40).

## Not done

- `BLOCKED(phase10)`, carried: a VMM found running by a partition
  manager started again is watched by looking at its control group every
  25 ms (`docs/PARTD.md`); an adapter's unit numbers have no gaps
  (`docs/PROFILE.md`); cores are given without regard to sockets or
  memory's place, and only 2 MB pages are used (`docs/RESOURCES.md`).
- Not in Phase 10, and not started: adding or taking away a device, a
  processor or memory on a running partition; virtual functions; media
  changers; a display adapter for a partition; release engineering.
