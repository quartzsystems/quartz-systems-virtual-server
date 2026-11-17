# Dumps

Two kinds of dump are kept on a host: a partition's, its memory and
processor state for the system's supplier; and the host's own, its
service data for the release's supplier. Service Tools option 8, 3
lists both, copies them to a removable volume and deletes them.

# A partition's dumps

The host takes a partition's dump from outside: its processors
stopped, its memory and processor state written to a file. It needs
nothing of the system, so a hung system can be dumped, and one held
after a terminal code (`docs/SERVICE.md`). It is not the system's own
main storage dump, which is QS/400's.

## Taking one

| How | |
|---|---|
| function 22 | a running or held partition; its IPL restarts once the dump is written (`docs/PANEL.md`) |
| function 34 | a held partition, from the memory it kept; again as often as wanted |
| request 28 | a running or held partition, without restart: its processors stop for the time the dump takes and go on after (`docs/PARTD.md`) |

The partition manager makes the file in the partition's dumps
directory, as `<name>.partial`, and hands it to the VMM over its
control socket (`dump <time> <code> <reason> <text>`). The VMM stops
the processors (a held partition's are stopped already), takes the
registers each left when it stopped, writes the dump, and lets them go
on unless the partition is held. Its progress is in reference codes:
`C2005180` when it begins, `C2005181` at each tenth of the memory,
`C2005185` when it is whole, with its size and the bytes of memory
written, or `A2005180` and why when it is not. A whole dump is renamed
to `<name>`; one that is not (the pool full, the VMM gone) is removed,
and the event says why (`QVS2311`). Nothing of a partial dump is ever
left: one a partition manager left when it ended is removed before the
next dump of the partition.

One dump of a partition at a time (`QVS2313`).

A partition with a host's device given whole (`docs/PHYSICALIO.md`)
keeps the device while its processors are stopped and while it is
held: the device may go on writing the partition's memory by DMA, only
there, so a dump can show what it wrote after the stop. The dump holds
the partition's memory and processors only; no device's registers or
BARs are read into it, since reading a device's registers can change
it. The device is reset when the partition ends.

## Where they are

```
<the media repository's pool>/dumps/<ID>/<NAME>-<time>.core
```

In the pool designated as the media repository (`docs/STORAGE.md`);
with none designated a dump is refused by name (`QVS2310`). The name
is the partition's and the time it was asked for, in seconds since the
epoch.

Each partition keeps at most `max_dumps` (3) dumps and `max_dump_mib`
(256 GiB) of them together, by the bytes they take in the pool
(`[service]` of `partitions.toml`). When a dump is written, the oldest
others are removed until the rest are within both, each with
`C2009060`; the new one is never removed for the limits.

A dump is listed (request 29: name, time, size, bytes it takes, code,
reason), read (request 31: its size, then its bytes on the connection,
128 KiB a frame, an empty frame at the end) and deleted (request 30),
which QSI's `dumps` family carries in Phase 8.

## The format

An ELF core of x86-64 (`ET_CORE`, `EM_X86_64`, 64-bit,
little-endian), which every tool that reads cores takes (`readelf -h
-l -n` shows it; the suites hold it to that). The crate is `part/dump`.

| Part | What |
|---|---|
| the ELF header | 64 bytes; no section headers |
| program headers | one `PT_NOTE`, then a `PT_LOAD` for each region of guest memory: its guest-physical address as both its virtual and physical address, its size, read-write-execute, page-aligned |
| notes | for each processor a `CORE` note of type 1 (`NT_PRSTATUS`): `elf_prstatus` of x86-64, its process number the processor's number plus one, its registers in `user_regs_struct` order; and a `QSVS` note of type `0x51530002`: the processor's number, then `cr0`, `cr2`, `cr3`, `cr4`, `efer`, the GDT's base and the IDT's, eight bytes each. Last, a `QSVS` note of type `0x51530001`: the header record |
| memory | each region at its program header's offset. A page of zeroes is not written: the file has a hole there and takes no space for it |

The header record is lines of `key=value`: `partition` (its name),
`id`, `serial` (its serial number), `code` and `text` (its current
reference code when the dump was asked for), `time` (seconds since the
epoch), `release` (the host's, `V1R0M0`), `reason` (`function22`,
`function34`, `request`). A value holds no line break and is at most
256 characters.

## Reading one

```
readelf -h -l -n PROD-1794000000.core      # the header, the regions, the notes
```

The `QSVS` notes are not types a core reader takes for its own, so a
debugger reads the processors from the `CORE` notes alone. A guest
virtual address is translated with the processor's `cr3` from its
`QSVS` note: the regions are guest-physical. QS/MC will read the
header record to list dumps by partition and code.

The memory of a QS/400 partition of 4 GiB that had IPLed dumped in
11 s on the build host and took 568 MB: the rest was zero.

# The host's dump

The host's own dump is service data, not memory: what the release's
supplier needs to see why the host did what it did. It is taken by PID
1 (`svc/hostdump`), asked for:

| By | How |
|---|---|
| a management console | QSI base `TakeDump` of kind `ServiceData` (`docs/QSISERVER.md`) |
| Service Tools | option 8, 2 |
| PID 1 itself | when a supervised service is stopped after its retries (`A2001080`), and when a pending slot did not confirm and the host came back on the other (`A2001035`), once the services are ready |

One is taken at a time (`QSI1202` to a second ask), on a thread of PID
1's own, so its loop never waits on `/data`: `C2011000` when it begins,
`C2011010` with its name and size when it is written, `A2011010` with
why when it could not be (nothing of it is kept, and a problem opens).

## What it holds

One ZIP archive, `HOSTDUMP-YYYYMMDD-HHMMSS.zip` (UTC), every entry
stored with its CRC-32: Windows, macOS and Linux open it as it is. The
archive's comment names what asked for it.

| Entry | What |
|---|---|
| `log/` | the log service's directory: the system log and its rotations, the reference code history, each boot's kernel log |
| `problems/problems.json` | the problem log (`docs/PROBLEMS.md`) |
| `state/` | every state file under `/data/state`, the inventory and the partitions' definitions and profiles among them, but those left out below |
| `services/status.txt` | PID 1's facts and each service's state, process and failures |
| `services/partitions.txt` | each partition's status, definition, profiles and history; the host's resources and settings |
| `services/pools.txt` | the pools, disks, virtual disks, optical images and tape volumes |
| `services/switches.txt` | the switches, their ports and the host's interfaces |
| `host/cgroups.txt` | the control group tree: each group's processes, CPU, cpuset, memory and huge page controls |
| `host/` | `/proc`'s cpuinfo, meminfo, mounts, cmdline, uptime and version |
| `MANIFEST.txt` | when, by whom, the release and the host; each entry with its size; what was left out and why |

A service that does not answer is said so in its file. A file is taken
to its newest 64 MiB, and a dump to 512 MiB.

## No secret

These are left out by name, wherever they are, and the manifest says
so: `identity.key` (the host's private key), `users.db` (the service
tools users and their password hashes), `qsi-consoles.toml` (the
management consoles' pins). Any `.key` file, and any file that holds a
private key or a password hash whatever its name, is left out too. A
unit test (`hostdump::tests`) takes a dump of a host with each of them
and fails if one is in it, or if a crate's secret file is not on the
list by name.

## Where they are

`/data/dumps/host/`, root's and the QSI server's group's (the server
lists, reads and deletes them); nobody else's. At most 8 and 1 GiB
together: the oldest is put out first to make room (`C2011020`), and
deleting one is `C2011020` too.

Over QSI: base `Dumps` lists them oldest first (kind `ServiceData`,
`service data`), `DeleteDump` deletes one (`QSI1201` when there is no
such dump), `DownloadDump` opens a stream of its bytes, and the events
`DumpTaken` (written, or why not) and `DumpRemoved`. Base's capability
bit 4 (`caps::SYSTEM_DUMPS`) is in the host's hello.
