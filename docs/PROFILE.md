# Partitions: definitions and profiles

A partition has one **definition**, which says what the partition is,
and any number of **profiles**, each of which says what an activation
is to give it. Both belong to the partition manager (`docs/PARTD.md`):
it keeps them in state, checks them, and is the only thing that writes
them. They are created and changed by requests to its socket, which the
management console, QS/MC, will send through QSI (Phase 8). The fields
of a profile are the ones QS/MC's Create partition wizard collects.

The crate is `part/model`. Every error of a definition or a profile is
given at once, each as a message (`docs/MESSAGES.md`).

## Where they are kept

```
/data/state/partitions.toml                         the host's settings and pools (docs/RESOURCES.md)
/data/state/partitions/<ID>/definition.toml         the definition
/data/state/partitions/<ID>/profiles/<NAME>/profile.toml
```

Each is a state file of `base/state`: it starts with `schema_version`
(1 for all three), is written atomically, and a file written by a newer
release is refused, not half read. A key a file does not know is
refused with its name.

## Names

A partition's name, a profile's name and a shared processor pool's
name follow one rule: 1 to 10 characters of `A` to `Z`, `0` to `9` and
`_`, with a letter first. No two partitions of a host have one name; no
two profiles of a partition have one.

## The definition

```toml
schema_version = 1
id = 1
name = "PROD"
environment = "QS/400"
serial = "8GQ2W01"
keylock = "normal"
default_profile = "NORMAL"
created = 1794000000

[auto_start]
enabled = true
order = 10
delay_secs = 0
```

| Key | Value | Changed |
|---|---|---|
| `id` | 1 to 999. Assigned when the partition is created: the lowest that no partition has. It is no other partition's for as long as the definition exists; when the partition is deleted it is free again | never |
| `name` | by the naming rule | while not activated |
| `environment` | `QS/400`, always | never |
| `serial` | the partition's system serial number: seven characters, as QS/400's `QSRLNBR` holds. Made when the partition is created (below) | never |
| `keylock` | `normal`, `manual`, `auto` or `secure`. The keylock is the host's: it is written to the partition's LIC record at every activation, so a change made inside the guest lasts until the next activation | while not activated |
| `default_profile` | the profile an activation uses when it names none. The first profile a partition is given becomes its default | while not activated |
| `auto_start.enabled` | whether the partition is activated when the host has started | while not activated |
| `auto_start.order` | its place among the others: lower first; partitions with one place go by ID | while not activated |
| `auto_start.delay_secs` | seconds to wait before it, after the one before: 0 to 3600 | while not activated |
| `created` | when it was created, in seconds since the epoch | never |

### The serial number

Seven characters of the digits and the capital letters without `I` and
`O`. The first five come from the host's serial number (a hash of it,
so every partition of one host starts with the same five, and another
host's start with another five); the last two are the partition's ID
in that alphabet. No two partitions of a host have one serial number,
and a partition created again under an ID that was freed has the
number that ID had. It is in the definition and is given to the guest
in SMBIOS tables at every activation (`docs/VMM.md`), which is where
QS/400 takes `QSRLNBR` from (`docs/QS400_GUEST.md`).

### A partition's user

The partition's `qsvmm` runs as user and group 23000 plus the ID, with
no capability. Its serial log, `/data/log/partitions/<ID>/serial.log`,
and its console socket, `/run/partitions/<ID>/console.sock`, are in
directories that are that user's alone: one partition's process can
neither signal another's nor read its log.

## A profile

```toml
schema_version = 1
name = "NORMAL"

[processors]
mode = "shared"
minimum = 1
desired = 2
maximum = 4
minimum_units = 0.5
desired_units = 1.5
maximum_units = 4.0
sharing = "uncapped"
weight = 128
pool = "DEFAULT"

[memory]
minimum_mib = 1024
desired_mib = 2048
maximum_mib = 4096

[boot]
source = "recorded"
type = "unattended"
console = "serial"

[[scsi]]
slot = 2

[[scsi.lun]]
lun = 0
backing = "/dev/sdb"
serial = "PROD-LS"
```

### `[processors]`

A processor is one host core (`docs/RESOURCES.md`).

| Key | Value |
|---|---|
| `mode` | `dedicated`: whole cores no other partition and no host service uses. `shared`: virtual processors over a shared pool's cores |
| `minimum`, `desired`, `maximum` | processors when dedicated, virtual processors when shared: 1 to 240, the minimum not more than the desired, the desired not more than the maximum |
| `minimum_units`, `desired_units`, `maximum_units` | shared only, and then all three: processing units in steps of 0.01, in order. At each of the three levels the units are at least 0.05 and at most 1.00 for each virtual processor of that level |
| `sharing` | shared only: `capped` (the partition cannot use more than its units) or `uncapped`. Capped when left out |
| `weight` | shared only: the uncapped weight, 0 to 255; 128 when left out. A weight of 0 keeps the partition to its units, as if it were capped |
| `pool` | shared only: the shared processor pool. `DEFAULT` when left out |

A dedicated profile that has any of the shared keys is refused: the key
is not ignored.

### `[memory]`

`minimum_mib`, `desired_mib`, `maximum_mib`: in order, each 128 to
1048576, each a multiple of the host's memory block size (16 MB unless
the host's settings say otherwise).

### `[boot]`

The table may be left out; then it is as below.

| Key | Value | Left out |
|---|---|---|
| `source` | the IPL source. `a` or `b`: that side. `recorded`: the side the partition itself asked for, in its LIC record: the side it asked for one IPL, which is then cleared, else its default side, else side A. `d`, installation media, is accepted and refused at activation until Phase 5 | `recorded` |
| `type` | `unattended`, or `attended`: the IPL stops at its menu. With the keylock in the manual position an IPL is attended whatever this says | `unattended` |
| `console` | `serial` or `local`. A partition has no display, so `serial` is the one that gives it a console | `serial` |

All three are written to the partition's LIC record before the IPL,
with the definition's keylock.

### `[[scsi]]`: virtual SCSI adapters

Each adapter has a virtual `slot`, 2 to 255 (slots 0 and 1 are the
partition's console and its service channel), and its units,
`[[scsi.lun]]`. Eight adapters at most, 64 units an adapter.

| Key | Value | Left out |
|---|---|---|
| `lun` | the unit number. An adapter's units are numbered from 0 without a gap | required |
| `backing` | what the unit is kept in: the absolute path of a file or a block device on the host. It is opened for direct I/O, and a block device exclusively. Phase 5 puts a virtual disk of a storage pool here | required |
| `serial` | the unit's serial number as the guest sees it: 1 to 20 letters, digits, `-` and `_`. It should stay the same for as long as the guest is to know the disk | the partition's serial number, the slot and the unit: `8GQ2W01-2-0` |

The load source is the lowest unit of the adapter in the lowest slot.
A profile with no unit is refused. Two units may not have one backing
store or one serial number. A backing store that is not there does not
make a profile invalid: it fails the activation, by name
(`QVS2110`).

### The sections later phases bring

They are in the schema now and are checked for shape. A profile that
uses one is kept, and is refused at activation with `QVS2106`, which
names the phase that brings it.

| Section | Keys | Phase |
|---|---|---|
| `[[ethernet]]` | `slot`; `vlan`, 1 to 4094; `mac`, optional, six pairs of hexadecimal digits and no group address | 6 |
| `[[optical]]`, `[[tape]]` | `slot`; `media`, optional, an absolute path | 5 |
| `[[physical_io]]` | `location`, a location code (`docs/LOCATION.md`); `required`, optional | 10 |

An adapter of any kind has a slot of its own: a slot is used once in a
profile, whatever uses it.

## What an activation does with a profile

`docs/PARTD.md` has the request and `docs/RESOURCES.md` the accounts.
In short: the profile is checked again; the desired processors and
memory are given when they are available, else the most there is that
is not below the minimum, else the activation fails with the message
that names the resource, what was asked and what is available; and what
was given is the partition's current resources until it ends. Nothing
changes on a running partition in this phase: a profile changed while
its partition runs is the next activation's.

## Not here yet

- `BLOCKED(phase4)`: an adapter's unit numbers have no gaps: the virtio-scsi device numbers its units by their place (`docs/VMM.md`).
- Virtual disks in storage pools, optical and tape devices, IPL source
  D: Phase 5. Virtual Ethernet adapters: Phase 6. The service channel
  and console attach: Phase 7. Physical I/O: Phase 10.
