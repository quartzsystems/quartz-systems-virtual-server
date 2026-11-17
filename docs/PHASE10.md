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
