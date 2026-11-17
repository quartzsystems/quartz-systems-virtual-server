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
wait = true              # the next waits for this one's IPL to complete
wait_secs = 600          # or this long
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
| `auto_start.wait` | whether the next partition is started only when this one's system has reported its IPL complete (`docs/SERVICE.md`), or `wait_secs` have passed; left out, `false` | while not activated |
| `auto_start.wait_secs` | the longest that wait: 1 to 3600; left out, 600 | while not activated |
| `created` | when it was created, in seconds since the epoch | never |
| `revision` | the partition manager's: one more at every write of the definition, left out (0) in a file older than revisions (`docs/PARTD.md`, "Revisions") | by the partition manager alone |

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
console = "partition"

[[scsi]]
slot = 2

[[scsi.lun]]
lun = 0
backing = "POOL1/PRODLS"
serial = "PROD-LS"

[[optical]]
slot = 3
media = "QS400V1R1"

[[tape]]
slot = 4

[[ethernet]]
slot = 5
switch = "PRODNET"
vlan = 10
tagged = [20]
mac = "02:5a:3c:00:01:05"
```

A profile also has `revision`, the partition manager's own: one more at
every write of it, left out (0) in a file older than revisions; what a
caller gives is not kept (`docs/PARTD.md`, "Revisions").

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
| `source` | the IPL source. `a` or `b`: that side. `recorded`: the side the partition itself asked for, in its LIC record: the side it asked for one IPL, which is then cleared, else its default side, else side A. `d`: the installation media in the partition's first optical device (the lowest slot); a profile with `d` and no optical device is refused (`QVS1219`) | `recorded` |
| `type` | `unattended`, or `attended`: the IPL stops at its menu. With the keylock in the manual position an IPL is attended whatever this says | `unattended` |
| `console` | `partition`: the console port of the service channel, the console as a full screen (`docs/SERVICE.md`); `serial`: the serial port, for a system older than QS/400's handoff 17; `local`: the display, which a partition does not have, so it is given the console port as `partition` is | `partition` |

All three are written to the partition's LIC record before the IPL,
with the definition's keylock: the console mode as the console port
(`licpart::ConsoleMode::Partition`) unless it is `serial`. A profile
kept before Phase 7 says `serial`, as the default then was, and keeps
it until it is changed.

### `[[scsi]]`: virtual SCSI adapters

Each adapter has a virtual `slot`, 2 to 255 (slots 0 and 1 are kept
for the partition's console and its service channel, which are a
device of their own after every slot's), and its units,
`[[scsi.lun]]`. Eight adapters at most, 64 units an adapter.

| Key | Value | Left out |
|---|---|---|
| `lun` | the unit number. An adapter's units are numbered from 0 without a gap | required |
| `backing` | what the unit is kept in: a virtual disk of a storage pool, `POOL/NAME` (`docs/STORAGE.md`). The pool service holds it for the partition at activation and hands it over open for direct I/O; a disk is held by one partition at a time. In a test image the absolute path of a file or a block device on the host is accepted too, for the suites of earlier phases; a release image refuses one at activation (`QVS1218`) | required |
| `serial` | the unit's serial number as the guest sees it: 1 to 20 letters, digits, `-` and `_`. It should stay the same for as long as the guest is to know the disk | the partition's serial number, the slot and the unit: `8GQ2W01-2-0` |

The load source is the lowest unit of the adapter in the lowest slot.
A profile with no unit is refused. Two units may not have one backing
store or one serial number. A backing store that is not there does not
make a profile invalid: it fails the activation, by name (a virtual
disk: `QVS3104`, a pool that cannot be used: `QVS3014`; a host path in
a test image: `QVS2110`).

### `[[optical]]` and `[[tape]]`: virtual optical and tape devices

Each device has a slot of its own and is a virtio-scsi function of its
own (`docs/VMM.md`). Four of each at most.

| Key | Value | Left out |
|---|---|---|
| `slot` | 2 to 255 | required |
| `media` | the medium loaded at activation: an optical image's name, or a tape volume's, in the media library (1 to 32 letters, digits, `-` and `_`, a letter first); for a tape device, or `@` and a physical tape drive's location code | an empty device |

The unit's serial number is the partition's, the slot and 0. A medium
is loaded and unloaded while the partition runs by the partition
manager's requests (`docs/PARTD.md`); a guest's restart loads what was
loaded when it restarted, not what the profile says.

A tape device whose medium is `@` and a location code is the host's
physical drive there, passed through command by command
(`docs/STORAGE.md`, "Tape pass-through"): the drive must be released at
the host, and one partition holds it at a time. Its cartridge is the
drive's: nothing is loaded or unloaded through the partition manager
(`QVS2421`).

```toml
[[tape]]
slot = 4
media = "@P0000-B00-D08-F0-T0.0.3.0"
```

### `[[ethernet]]`: virtual Ethernet adapters

Each adapter is a port of a virtual switch (`docs/NETWORK.md`) and a
virtio-net function of its own in its slot (`docs/VMM.md`). Eight at
most.

| Key | Value | Left out |
|---|---|---|
| `slot` | 2 to 255 | required |
| `switch` | the virtual switch it is on, by name. A profile without it is refused (`QVS1226`) | required |
| `vlan` | the port VLAN, 1 to 4094: the guest's frames on it are untagged | required |
| `tagged` | further VLANs passed to the guest with their tags: 1 to 4094 each, listed once, none the port VLAN, 32 at most | none |
| `mac` | `xx:xx:xx:xx:xx:xx`, no group address and no other partition's. Made when the adapter is first saved without one and written into the profile: the host's prefix, the partition's ID and the slot; it never changes afterwards | made |
| `required` | `false`: a missing or failed switch does not fail the activation, and the adapter comes up without link | `true` |

When a profile is saved, its adapters are checked against the switches
too: a switch that is not there (`QVS1223`), a VLAN it does not carry
(`QVS1224`, each), an address another partition has (`QVS1225`). When
the switch service does not answer, those are checked at activation.
An adapter's settings apply at the next activation.

### `[[physical_io]]`: devices of the host given whole

A PCI device of the host, given to the partition whole with the rest of
its isolation group (`docs/PHYSICALIO.md`).

| Key | Value | Left out |
|---|---|---|
| `location` | the device's location code (`docs/LOCATION.md`), as Service Tools option 10 shows it | required |
| `required` | `true`: an activation fails, naming the device, when it cannot be given; `false`: the partition activates without it | `false` |

A put is refused when a location is no device of the host
(`QVS2401`), a location is named twice, or a group is named in part
(`QVS2404`). Whether a device is released, free, present and not
fenced is the activation's.

```toml
[[physical_io]]
location = "P0000-B00-D06-F0"
required = true

[[physical_io]]
location = "P0000-B00-D07-F0"
```

An adapter of any kind has a slot of its own: a slot is used once in a
profile, whatever uses it.

## What an activation does with a profile

`docs/PARTD.md` has the request and `docs/RESOURCES.md` the accounts.
In short: the profile is checked again; the desired processors and
memory are given when they are available, else the most there is that
is not below the minimum, else the activation fails with the message
that names the resource, what was asked and what is available; and what
was given is the partition's current resources until it ends. A
running shared partition's entitlement can change without an IPL
(`docs/RESOURCES.md`); nothing else of a running partition does, and a
profile changed while its partition runs is the next activation's.

## Not here yet

- `BLOCKED(phase10)`: an adapter's unit numbers have no gaps: the virtio-scsi device numbers its units by their place (`docs/VMM.md`).
