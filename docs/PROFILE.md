# A partition's profile

**Provisional.** Phase 4's partition manager owns profiles: where they
are kept, who writes them, how a running partition's differs from its
next activation's. Until then a profile is a TOML file written by hand
and given to `qsvmm --profile <file>`, and only a test image starts
one (`docs/PHASE3.md`). The field names are meant to last; the file is
not.

```toml
name = "TESTLPAR"
processors = 2
memory_mib = 2048

[ipl]
side = "recorded"
keylock = "normal"
console = "serial"

[load_source]
backing = "/dev/sdb"
serial = "TESTLPAR-0"

[[disk]]
backing = "/dev/sdc"
```

A key the profile does not know is refused, with its name: a profile
that says more than this phase does is not half obeyed.

## The partition

| Key | Value |
|---|---|
| `name` | 1 to 10 characters of `A` to `Z` and `0` to `9`, a letter first. It names the partition in reference codes, in its log directory and in its console socket |
| `processors` | virtual processors, 1 to 240, and no more than KVM gives a VM on the host. One package of that many cores, one thread a core |
| `memory_mib` | memory in MiB: 128 to 1048576. Above 3072 the rest is placed beyond 4 GiB |

## `[ipl]`

The table may be left out; each key may.

| Key | Value | Left out |
|---|---|---|
| `side` | `a`, `b`, or `recorded`: the side of the last IPL that completed, by the guest's LIC record; side A for a disk that has never IPLed | `recorded` |
| `keylock` | `normal`, `manual`, `auto` or `secure`: written to the guest's LIC record before the IPL, as its `KEYLCKPOS` and as the mode of this IPL | what the record has |
| `console` | `serial` or `local`: written to the LIC record before the IPL. A partition has no display, so `serial` is the one that gives it a console | what the record has: `local` on a new disk |

`side = "recorded"`, a `keylock` or a `console` needs the LIC record,
so a load source with no LIC partition is refused with `A2005040`. With
`side` given and neither of the others, the loader does not need the
record and a disk without a LIC partition is loaded.

D, an IPL from optical media, is Phase 5.

## `[load_source]` and `[[disk]]`

The load source is LUN 0 of the partition's virtio-scsi controller;
each `[[disk]]` is the next LUN, in the order written. 64 units at
most.

| Key | Value | Left out |
|---|---|---|
| `backing` | an absolute path on the host: a file, or a block device. It is opened for direct I/O, and a block device exclusively | required |
| `serial` | the unit's serial number as the guest sees it: 1 to 20 letters, digits, `-` and `_`. It must stay the same for as long as the guest is to know the disk | the name, `-`, and the LUN |

Two units may not have one backing store or one serial number.

## What is not here yet

Phase 4: processor pinning and shares, huge pages, the partition's
state between activations, a serial number and identity for the
machine. Phase 5: virtual disks in storage pools, optical and tape.
Phase 6: virtual Ethernet adapters. Phase 7: the service channel and
the console.
