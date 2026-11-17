# Physical I/O

A partition can be given a PCI device of the host whole, with the rest
of its isolation group: an adapter, a controller, a network card. The
partition's own drivers run it; the host's do not touch it while the
partition holds it. This is Phase 10's (`docs/PHASE10.md`). A physical
tape drive can also be passed through command by command, with the
host keeping its controller; that is `docs/STORAGE.md`'s "Tape
pass-through".

The code: `part/pio` (the rules, `physio.toml`, assignment, the kernel's
PCI files), `lic/hostinv` (groups, resets, interrupts, the IOMMU),
`part/partd/src/manager/physio.rs` and `serve/machine.rs` (release,
assignment, reset, fencing), `vmm/devices/src/pci/passthrough` (what the
guest sees), `vmm/devices/src/sys/vfio.rs` and `vmm/qsvmm/src/physio.rs`
(VFIO), `lic/qsvsinit/src/ipl` (binding at the IPL), `svc/tools/src/physio.rs`
(Service Tools option 10).

## What can be given

A whole PCI function, with every other function of its IOMMU group. A
group is the set of devices the IOMMU cannot tell apart: whatever one
of them can reach by DMA, the others can too, so they go to one
partition together or not at all.

Never given, whatever is asked:

| The host's | Why |
|---|---|
| a load-source disk's controller | the host's own code and state are on it |
| a storage pool member's controller | every partition's virtual disks are on it |
| the management interface's adapter | QS/MC reaches the host through it |
| every uplink's adapter | the switches' traffic leaves through it |
| the console's display adapter | the local console is on it; no partition has a display in V1 |
| a bridge | the host's PCI plumbing |

A device whose group holds any of these cannot be released either: the
message names the device of the group the host uses.

Not in V1: virtual functions (SR-IOV), adding or taking away a device
from a running partition, a display adapter for a partition.

## The rules

A device is given only when all of these hold. No setting, profile
field, request or environment variable relaxes them:

1. The host's IOMMU translates DMA and remaps interrupts. PID 1 checks
   both at the IPL (`/sys/class/iommu`, the kernel's DMAR and IR state);
   without either it posts `A2013010`, a problem opens, nothing released
   is bound and nothing can be released, and everything else runs.
2. Every device of its group is released, and the group is given whole
   to one partition.
3. Every device of its group can be reset alone: function-level reset,
   or a reset of its bus when the group is all that is behind the
   bridge (`reset_method` in sysfs).
4. Every device of its group signals message interrupts (MSI or MSI-X).
   Line interrupts are never given.

The kernel the host runs is built with the Intel and AMD IOMMUs on by
default, lazy invalidation, x2APIC and VFIO's container and type 1
backend (`kernel/config/qsvs.config`); the build checks each symbol.

## Release and take back

Only at the host: Service Tools option 10, `Work with I/O for
partitions` (`docs/SERVICETOOLS.md`). A management console sees the
devices and names released ones in profiles; it cannot release one.

- **Release** (option 2): the operator types the device's location code
  in full. The device and every other device of its group are released
  together; the panel lists them first. The partition manager unbinds
  each from its host driver, gives it to `vfio-pci` (a group that does
  not bind whole goes back to the host's drivers), keeps it in
  `physio.toml` and posts `C2013010`. A released adapter is no network
  interface of the host from then on: PID 1 writes the inventory again
  when the code is posted, and again at each IPL once it has bound the
  released devices, so Service Tools, the switch service and a console
  never offer it.
- **Take back** (option 4): only while no partition holds it and no
  profile names it (`QVS2403`, `QVS2415`). The group goes back to the
  host's drivers together; `C2013015`.
- **Display** (option 5): a device whole: its group, how it resets, its
  interrupts, what the host uses it for, and why it cannot be released.

`physio.toml` (in `/data/state`) keeps each released device with its
location, PCI address, vendor and device IDs and its group as it was, so
a device that moved, was replaced or whose group changed is not bound
blindly at the next IPL.

## At the IPL

PID 1, after the partitions' memory and before any service, binds each
released device to `vfio-pci` (`driver_override`, unbind, probe) and
posts `C2013020` for it; then `C2013000` with the count. A released
device that is not there posts `A2013030`; one whose IDs or group
changed posts `A2013035` and is left with the host's driver. Each opens
a problem and blocks nothing else: a partition whose profile does not
require it activates without it.

The kernel's built-in drivers probe before PID 1 runs, so a released
device's host driver has it for the first seconds of every IPL, before
PID 1 takes it away. A kernel command-line list of devices for
`vfio-pci` would close that window, but the command line is baked into
the signed UKI and the same for every host. NEEDS-CODY: whether this
matters on the Dell for the QLE2562 (its host driver logs in to the
fabric for those seconds).

## Assignment

At activation only, and back at the partition's end only.

- A profile names devices by location code in `[[physical_io]]`, each
  `required` or not (`docs/PROFILE.md`). A put that names part of a
  group without the rest, a location twice, or a location the host has
  no device at is refused (`QVS2404`, `QVS2401`).
- At activation each named group must be released, not held, not fenced
  and present. A required device that is not fails the activation by
  name (`QVS2403` held, `QVS2402` not released, `QVS2412` fenced,
  `QVS2413` gone, `QVS2414` its group changed); a reset or open that
  fails is `QVS2405` or `QVS2422`. One not required is left out, with
  its group, and the partition activates without it; the activation's answer and the
  partition's event say what was left out and why (`QVS2418`).
- The partition manager resets each function, opens its group and
  device through VFIO as root and hands the descriptors to `qsvmm`,
  which runs as the partition's own unprivileged user with no
  capability. `C2013030` for each.
- At the partition's end, however it ends (a shut down, a kill, a
  fault), each function is reset again and kept for the next partition;
  `C2013035`.

### Reset and fencing

A function is reset before it is given and after it is given back. A
reset that fails fences the device: it is given to no partition until
the host restarts, its problem is open (`A2013050`), and Service Tools
shows it as `fenced: did not reset`. The fence is kept in the run
directory, not in state: a restart of the partition manager keeps it, a
restart of the host clears it.

## What the partition sees

`qsvmm` puts each function on the partition's PCI bus at the device
numbers after the virtio devices', in the profile's order
(`docs/VMM.md`). It presents the function's
configuration space; the device's own is behind it:

| Bytes | The guest reads | A guest write |
|---|---|---|
| IDs, class, subsystem, header | the device's, not multifunction | ignored |
| command | what it wrote | memory, bus master, parity and SERR reach the device; line interrupts stay off there, I/O never on |
| status | the device's | its write-one-to-clear bits reach the device |
| memory BARs | the VMM's places, sized as the device's | sizing is answered, an address is ignored |
| I/O BARs, the expansion ROM, the interrupt pin | zero | ignored |
| power management | the device's, with the state it wrote | kept by the VMM: the device stays in D0 |
| PCI Express | the device's | ignored |
| MSI, MSI-X | the VMM's | set the interrupts up |
| every other capability, device-specific bytes, extended space | zero | ignored |

Each memory BAR is placed in the partition's PCI memory window from the
top down and mapped straight into the guest, but for the pages of the
MSI-X table and pending bits, which the VMM answers. Each interrupt
vector is an eventfd the host's VFIO driver signals and KVM delivers
(irqfd), so an interrupt reaches the guest without leaving the kernel.
The VMM holds bus mastering off at the device until the partition's
memory is mapped for DMA.

All of the partition's memory (2 MB huge pages, pinned already) is
mapped for the device's DMA at its guest address, and nothing else. The
isolation suite (`pio-isolation`) reads the kernel's own record of the
mappings, the Intel IOMMU driver's debugfs page tables, and finds
exactly that.

Every configuration access and every BAR access the VMM answers is
guest input (`docs/VMM.md`); `tools/fuzz` has the target `passthrough`.

### Dumps

A partition held with its memory for a dump keeps its devices until it
ends. Their DMA stays inside the partition's memory, so a dump can show
memory a device wrote after the hold. A dump holds the partition's
memory and processors only: no device's BARs are read into it, since a
read of a device's registers can change the device.

## Codes and messages

| Code | When |
|---|---|
| `C2013000` | physical I/O checked at the IPL, with the count bound |
| `C2013010`, `C2013015` | released, taken back |
| `C2013020` | a released device bound at the IPL |
| `C2013030`, `C2013035` | given to a partition, given back |
| `A2013010` | IOMMU or interrupt remapping off |
| `A2013030` | a released device not found |
| `A2013035` | a released device's group or IDs changed |
| `A2013050` | a device did not reset and is fenced |

`QVS2401` to `QVS2422` are the requests' refusals (`docs/MESSAGES.md`);
each opens no problem. The attention codes open problems by the
catalogue's table (`docs/PROBLEMS.md`).

## Over QSI

`host` operation `0x0010`, `Devices`, lists every device as Service
Tools does; events `0x0010` to `0x0014` say a device was released,
taken back, given, given back or fenced (`docs/QSISERVER.md`). The
`PHYSICAL_IO` capability is offered. A profile's physical I/O goes over
the `profiles` family as any profile does.
