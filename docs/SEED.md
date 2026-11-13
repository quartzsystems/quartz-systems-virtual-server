# The seed

How a host gets its first configuration before there is a console to
enter it on, and how a test gets any. The code is `base/seed`; the label
comes from the appliance (`QSVSSEED` for QS/VS, `lic/release`).

The same file, with one more table, is the answers file of an
unattended installation: `[install]`, from `schema_version = 2`. The
installer reads it from the same volume (`docs/INSTALL.md`).

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
nothing does.

An installed host is configured from its first IPL: the installer wrote
`host.toml`, from what the operator typed or from the answers file. A
seed volume attached to it later is not read. A host that was not
installed (the built disk image, which the suites boot) has no
`host.toml` until a seed or Service Tools writes one, so it can be given
a seed volume later: the next IPL reads it. A running host reads the
`[host]` and `[network]` tables of a seed and leaves `[install]` alone.
What the host keeps for itself is in other files (`identity.toml`, the
serial number it derives when the firmware gives none, and
`inventory.toml`), and they do not make it configured.

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

`schema_version` is required and is `1` or `2`. A file of a newer
version is refused, as a state file is. Version 2 is version 1 with the
`[install]` table allowed; a file that has that table says 2.

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

The serial number is the host's own (`identity.toml`) and no seed sets
it.

### `[network]`, which becomes `network.toml`

| Key | Value | Default |
|---|---|---|
| `device` | the management interface's device: a name (`eth0`), a MAC (`52:54:00:12:34:56`) or `auto` | `auto`: the first Ethernet link with carrier |
| `mode` | `dhcp` or `static` | `dhcp` |
| `address` | `static` only, and required there: the IPv4 address and prefix length, `192.0.2.10/24` | |
| `gateway` | `static` only: the default gateway, on the address's subnet | none |
| `vlan` | the 802.1Q VLAN ID, 1 to 4094; the interface is then `<device>.<id>` | none |
| `dns` | name servers, IPv4 or IPv6; with DHCP they come before the lease's | none |

IPv6 is link-local only and has no keys.

### `[install]`, the answers of an unattended installation

```toml
schema_version = 2

[install]
disks = ["S4EVNX0M702341", "P0000-B03-D00-F0-U1"]
protection = "mirrored"
user_id = "ADMIN"
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$..."

[host]
name = "vs-01"

[network]
mode = "dhcp"
```

| Key | Value | Default |
|---|---|---|
| `disks` | the load-source disks: one, or two for mirrored protection. Each is a disk's serial number or its location code (`docs/LOCATION.md`), whichever names exactly one disk of the machine | required |
| `protection` | `none` or `mirrored` | `none` for one disk, `mirrored` for two |
| `user_id` | the first service tools user ID: 1 to 10 letters and digits | required |
| `password_hash` | the hash of that user's password, an Argon2id PHC string. The password itself is never in the file | required |

The installer holds the table against the machine before it writes
anything: each disk must be there and large enough, two disks must
differ, the protection must fit the number of disks, the user ID must be
one and the hash must be an Argon2id hash. Anything else stops it on a
screen that says what is wrong (`docs/INSTALL.md`).

A password hash is made by any Argon2id tool that writes the PHC string
form, with a salt of its own; for example, with the `argon2` command of
the reference implementation:

```
printf '%s' 'the password' | argon2 "$(head -c 16 /dev/urandom | base64)" -id -e
```

The installer does not check the password's rules, since it never sees
the password: the rules of `docs/SERVICETOOLS.md` are the operator's to
keep here.

## Making a seed volume

Any FAT volume labelled `QSVSSEED` with `seed.toml` in its root: a USB
stick formatted on any system, or an image file given to a virtual
machine as a disk. The conformance harness makes one with QS/400's
`diskimg::fat` (`tools/conformance/src/qemu/image.rs`, `seed_volume`):
31 MiB of FAT16, the file under its long name. Its answers files, for
the unattended suites, are written by `answers` in
`tools/conformance/src/suite/install/mod.rs`.
