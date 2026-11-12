# Host reference codes

A QS/VS host posts reference codes the way a QS/400 system posts its
own: eight characters, a two-character family and six digits. The table
is `lic/refcode/src/lib.rs`; nothing else spells a code, and a unit test
(`refcode::tests::the_document_lists_the_table`) holds this document to
it.

A code is posted as one line, `REF <code> <text>`, sometimes with what
this posting adds after a colon:

```
REF C2001000 Host IPL started
REF A2001080 Service stopped: netd
```

## The host range

QS/400's families (`lic/slic/src/src.rs` in its repository, at the
pinned commit) are `C1` (LIC initialization), `C6` (OS initialization),
`A6` (attention), `B1` (fatal) and `D1` (power down). A partition's
console and a host's serial log end up side by side in front of the same
operator, so a host code must never read as a partition's.

The host takes the families whose second character is `2`. No QS/400
family ends in `2`, and the first character keeps the meaning it has
there:

| Family | Meaning |
|---|---|
| `C2` | host IPL and progress |
| `A2` | attention; the host continues |
| `B2` | fatal; the host parks. Only a failure of PID 1 itself |
| `D2` | power down |

A unit test (`refcode::tests::no_code_is_in_a_qs400_family`) fails if a
host code lands in a QS/400 family. The word before the code is `REF`,
not QS/400's `SRC`, so a line also says which kind of system posted it.

## Posting

PID 1 owns the console and the order of the history. A posting is the
console line, on the display and the serial port, and a record to the
log service, which writes it to the system log and appends it to the
history, `<data>/log/refcodes.ndjson`: the code, its text, the time and
the boot ID, one JSON object a line, never rotated. Codes posted before
the log service is up wait in PID 1 and reach the history in order.

A service posts through PID 1's control socket
(`lic/refcode/src/post/remote.rs`). PID 1 posts only codes of the table.

Two postings are not in the history: the heartbeat, which is on the
serial port only, and the last code of a shutdown (`D2001090` or
`D2001091`), which is posted after `/data` is unmounted.

## The blocks

| Block | Whose |
|---|---|
| `C2001xxx`, `A2001xxx`, `B2001xxx`, `D2001xxx` | PID 1: the IPL in order, supervision, the slot, shutdown. An attention code has the number of the IPL step it belongs to |
| `C2002xxx`, `A2002xxx` | the management network and time (`netd`) |

Later phases take further blocks (storage, the VMM, QSI) and record them
here.

## The codes

| Code | Text | Posted |
|---|---|---|
| `C2001000` | Host IPL started | by PID 1 at IPL start |
| `C2001005` | Host running | every 5 seconds, on the serial port only: the heartbeat |
| `C2001010` | Kernel file systems mounted | proc, sysfs, devtmpfs, `/run` and efivarfs are mounted; a mount that failed is named |
| `C2001020` | Control groups mounted | cgroup2 is mounted |
| `C2001030` | A/B record read | with the default slot, the pending slot and the attempts left |
| `C2001040` | Data file system mounted | `QSVSDATA` is mounted at `/data`, with its device |
| `C2001050` | Clock set from the RTC | with the date and time |
| `C2001060` | Seed checked | the seed step applied nothing: the host is configured, or there is no seed volume |
| `C2001061` | Seed applied | the state files were written from `seed.toml` |
| `C2001070` | State loaded | the state files are read and the inventory written, with the host name |
| `C2001080` | Services started | the service table was started |
| `C2001090` | Base ready | every service reported ready, with the seconds since the kernel started |
| `C2001095` | Slot confirmed | the slot is the default in the A/B record and first in `BootOrder` |
| `C2002010` | Management interface up | the interface has carrier, with its name |
| `C2002020` | Management address acquired | with the address, the interface and the gateway |
| `C2002030` | Time synchronized | the clock is in step with a time server and the RTC written, with the server |
| `A2001030` | A/B record not valid | no copy of the record is valid; a new one is written for the slot booted |
| `A2001035` | Pending slot not confirmed | the pending slot did not confirm on its IPL; this IPL is on the default and the pending field is cleared |
| `A2001040` | Data file system not mounted | `/data` could not be mounted, with the reason; state is in memory only. Nothing is reformatted |
| `A2001050` | Clock not set from the RTC | the RTC could not be read |
| `A2001060` | Seed not valid | a seed volume was found and refused, with the reason; nothing was written |
| `A2001070` | State file not valid | a state file was refused, with the file and the reason; its defaults are in use |
| `A2001080` | Service stopped | a service failed five times in ten minutes and is not restarted, with its name |
| `A2001095` | Boot order not written | the slot is confirmed in the record, but `BootOrder` could not be written, with the reason |
| `A2002005` | No management interface | no link matches `network.toml` after 30 seconds |
| `A2002010` | No carrier on the management interface | no carrier for 30 seconds, with the interface |
| `A2002020` | No management address | carrier but no address for 30 seconds, with the interface |
| `B2001000` | Host PID 1 failed | by PID 1's panic handler, with the panic message, before it parks |
| `D2001000` | Host shutdown started | an orderly shutdown began, with what asked for it |
| `D2001090` | Host powering off | services are stopped and `/data` is unmounted; power off follows |
| `D2001091` | Host restarting | services are stopped and `/data` is unmounted; a restart follows |
