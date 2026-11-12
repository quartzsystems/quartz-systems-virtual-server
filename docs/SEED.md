# The seed

How a host gets its first configuration before there is a console to
enter it on, and how a test gets any. The code is `base/seed`; the label
comes from the appliance (`QSVSSEED` for QS/VS, `lic/release`).

This format is the start of the unattended-install answers file of
Phase 2: an installer's answers will be further tables of a later
`schema_version`, beside the two here.

## When a seed is read

At every IPL PID 1 runs the seed step, after `/data` is mounted and the
clock is set, before the state files are read:

1. If `<data>/state/host.toml` exists, the host is configured. Nothing
   is looked for. `REF C2001060 Seed checked: the host is configured`.
2. Otherwise the block devices are searched, in name order, for a FAT
   volume whose label is `QSVSSEED`: a whole disk or a partition, FAT12,
   FAT16 or FAT32. None: `REF C2001060 Seed checked: no seed volume`,
   and the host comes up on its defaults.
3. The volume is mounted read-only, `seed.toml` is read from its root
   (256 KiB at most) and the volume is unmounted.
4. The file is checked as a whole. If anything in it is wrong, nothing
   is written: `REF A2001060 Seed not valid: <reason>`, and the host
   comes up on its defaults. The next IPL looks again.
5. Otherwise `network.toml` and then `host.toml` are written, each
   atomically, and `REF C2001061 Seed applied` is posted. `host.toml`
   is last because it is what says the host has been seeded.

A seed is applied once. The volume can stay attached: step 1 stops the
search from then on. To seed a host again, `host.toml` has to go, which
nothing in Phase 1 does.

A host whose firmware gives no serial number writes `host.toml` on its
first IPL, to keep the serial it derives from the first NIC
(`docs/PHASE1.md`). Such a host is seeded on its first IPL or not at
all. That is every QEMU guest, so a test attaches the seed volume to the
first boot.

When `/data` could not be mounted the state directory is in memory, and
a seed is applied to that: the host is configured until it powers off.

## `seed.toml`

```toml
schema_version = 1

[host]
name = "vs-01"
time_zone = "UTC+01:00"
ntp_servers = ["time1.example.net", "192.0.2.123:123"]

[network]
device = "52:54:00:12:34:56"
mode = "static"
address = "192.0.2.10/24"
gateway = "192.0.2.1"
vlan = 100
dns = ["192.0.2.53"]
```

`schema_version` is required and is `1`. A file of a newer version is
refused, as a state file is.

Each table has exactly the keys of the state file it becomes, and is
checked by that file's own rules (`base/state/src/doc/`). A table left
out gives that file's defaults; so does a key left out. A key or a table
this version does not know is an error, not ignored: a seed with a typing
mistake must not half apply.

### `[host]`, which becomes `host.toml`

| Key | Value | Default |
|---|---|---|
| `name` | the host name: one label of letters, digits and hyphens, 63 characters at most, no hyphen first or last | `QSVS` and the last six hex digits of the first NIC's MAC |
| `time_zone` | `UTC`, or a fixed offset `UTC+hh:mm` or `UTC-hh:mm`. It changes what is displayed; the system clock is UTC | `UTC` |
| `ntp_servers` | time servers, each `host` or `host:port`, tried in order | none: the clock is the RTC's |

`serial` is a key of `host.toml` (the serial number the host derives for
itself) and is not accepted in a seed.

### `[network]`, which becomes `network.toml`

| Key | Value | Default |
|---|---|---|
| `device` | the management interface's device: a name (`eth0`), a MAC (`52:54:00:12:34:56`) or `auto` | `auto`: the first Ethernet link with carrier |
| `mode` | `dhcp` or `static` | `dhcp` |
| `address` | `static` only, and required there: the IPv4 address and prefix length, `192.0.2.10/24` | |
| `gateway` | `static` only: the default gateway, on the address's subnet | none |
| `vlan` | the 802.1Q VLAN ID, 1 to 4094; the interface is then `<device>.<id>` | none |
| `dns` | name servers, IPv4 or IPv6; with DHCP they come before the lease's | none |

IPv6 is link-local only in Phase 1 and has no keys.

## Making a seed volume

Any FAT volume labelled `QSVSSEED` with `seed.toml` in its root: a USB
stick formatted on any system, or an image file given to a virtual
machine as a disk. The conformance harness makes one with QS/400's
`diskimg::fat` (`tools/conformance/src/qemu/image.rs`, `seed_volume`):
31 MiB of FAT16, the file under its long name.
