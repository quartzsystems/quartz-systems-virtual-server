# Location codes

The format was accepted on 2026-11-12 (`docs/OPEN_QUESTIONS.md`
item 8). What real hardware has still to show is at the end.

Every disk, network interface and PCI device in the host inventory
(`inventory.toml`) has a location code: one string that says where the
device is. The format is `lic/hostinv/src/location/mod.rs`; nothing else
spells a code.

A code is upper case, has no spaces, and is the same from IPL to IPL as
long as the hardware and the firmware settings do not change.

```
SPCIE-SLOT-3            the card in the slot the firmware calls "PCIe Slot 3"
SPCIE-SLOT-3-F1         the second function of that card
SPCIE-SLOT-3-E0         its first network port
P0000-B00-D1F-F2        the PCI function 0000:00:1f.2, which is in no slot
P0000-B00-D1F-F2-T1.0.0.0   a disk on that adapter's second port
SM-2-SLOT-1-N1          namespace 1 of the drive in "M.2 Slot 1"
NOPCI-MMCBLK0           a disk no PCI function leads to
```

## The PCI function

A code starts with the PCI function the device is, or is attached to. It
takes one of two forms.

**From the slot.** The firmware describes its slots in SMBIOS (type 9
structures), each with a designation and, from SMBIOS 2.6, the PCI
address of what the slot holds. When a slot matches, the code is `S` and
the designation: upper case, `A-Z` and `0-9` kept, every run of other
characters one hyphen, none at either end. `PCIe Slot 3` is
`SPCIE-SLOT-3`.

A slot matches a function in one of two ways, by what the firmware named:

| The slot's address is | The slot covers | Code |
|---|---|---|
| a device that is not a bridge | every function of that device | `S<designation>` for the function named, `S<designation>-F<function>` for the others |
| a PCI-to-PCI bridge (a root port, say) | everything behind the bridge, but not the bridge | `S<designation>` for device 0 function 0 directly behind it, `S<designation>-F<function>` for the other functions of that device, `S<designation>-B<bus>-D<device>-F<function>` for anything else behind it |

A slot that names the device itself comes first; otherwise the bridges
above the function are tried from the nearest up, so a slot on a riser
wins over the riser's own. A slot whose function is not present, whose
address is all ones, or whose designation has no letter or digit, is not
used. Neither are slots that share a designation once it is spelled as a
code: their devices take the address form, so no two devices have one
code.

**From the address.** A function no slot covers is
`P<segment>-B<bus>-D<device>-F<function>`: the segment in four hex digits
(more if it needs them), the bus and the device in two, the function in
one. `0000:00:1f.2` is `P0000-B00-D1F-F2`.

## The unit

A PCI device's code is its function's and nothing more. A disk or a
network interface adds a suffix that tells several units behind one
function apart:

| Suffix | For | Meaning |
|---|---|---|
| `-T<host>.<channel>.<target>.<lun>` | a SCSI, SATA, SAS or USB disk | its SCSI address, in decimal. The host is not the kernel's number but the host's place, from 0, among the hosts of the same PCI function, so adding an adapter does not move it. |
| `-N<namespace>` | an NVMe disk | the namespace number |
| `-E<port>` | a network interface | the port of the function (`dev_port`), 0 when it has one port |
| none | a virtio disk, or any other disk | the function is the disk |

## No PCI function

A device with no PCI function in its sysfs path is
`NOPCI-<kernel name>`, the name spelled as a designation is:
`NOPCI-MMCBLK0`. This code is only as stable as the kernel's name.

## What a code depends on

- An `S` code is the firmware's designation text. A firmware update that
  renames a slot changes the code.
- The bus in a `P` code is the one the firmware assigned. It holds on
  unchanged hardware; adding a card with a bridge can renumber the buses
  behind it.
- A `NOPCI` code is the kernel's name for the device.

## Not seen on hardware

- NEEDS-CODY: QEMU's machine has no SMBIOS slot records, so every code a
  suite has seen is of the address form. The slot form, and what real
  firmware writes as designations, are unit-tested only.
