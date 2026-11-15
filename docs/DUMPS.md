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
