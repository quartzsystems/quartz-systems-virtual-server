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
| `B2` | fatal; the host parks. A failure of PID 1 itself, or the loss of the storage service under a mirrored load source |
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

While a Service Tools session has the serial port, PID 1 prints nothing
there: a code posted meanwhile is on the display's console and in the
history, and the port's lines go on from the sign-off. A shutdown takes
the port back at once.

The installer's codes (`C2004xxx`, `A2004xxx`) are posted where there
is no log service and no `/data`: they are on the progress display, as
each step is done, and in the installer's own log in memory. The
installed host's history starts with its first IPL.

## The blocks

| Block | Whose |
|---|---|
| `C2001xxx`, `A2001xxx`, `B2001xxx`, `D2001xxx` | PID 1: the IPL in order, supervision, the slot, shutdown. An attention code has the number of the IPL step it belongs to |
| `C2002xxx`, `A2002xxx` | the management network and time (`netd`) |
| `C2003xxx`, `A2003xxx`, `B2003xxx` | the load source and its protection (`stgd`, the storage service) |
| `C2004xxx`, `A2004xxx` | the installer, numbered after its steps |

Later phases take further blocks (storage pools, the VMM, QSI) and
record them here.

## The storage service and `B2003000`

A mirrored load source is served to the kernel by the storage service,
through ublk. PID 1 restarts the service when it ends, by the back-off
rules of every service (1, 2, 4 ... 30 seconds; five failures in ten
minutes end the restarts). While there is no server the kernel holds
the requests to `/data`, and the next server takes them over: nothing
is lost and nothing above notices but the wait.

When the restarts end, `/data` has no server and never will have one
in this IPL. That is `B2003000`, the one fatal code that is not PID 1's
own failure: the host posts it on the console (the log is on `/data`)
and parks. It does not try to write or unmount `/data` again, and a
power off or restart asked for afterwards is done without either. The
partitions' storage is on other disks; the operator restarts the host.
An unprotected load source is a plain partition with no server, so the
code cannot occur there.

## Units that ran apart and `A2003040`

A mirrored pair has no third party. When each disk was started without
the other, each records the other as missing and goes on alone, and
when both are back nothing on them says which holds what the operator
wants. The host does not choose. The storage service posts `A2003040`
and assembles nothing; PID 1 mounts no `/data` and starts no other
service, and the display and the serial port show `Select Load-Source
Unit to Keep` (`docs/SERVICETOOLS.md`), with each unit's location
code, serial number and the time its disk was last written. When the
operator has chosen, `C2003050` is posted, the IPL goes on from the
unit kept, and the other is written again from it (`C2003020`,
`C2003025`). The host waits for the choice as long as it takes.

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
| `C2001075` | Host identity generated | the host's key pair and self-signed certificate were made, at the first IPL after installation |
| `C2003010` | Mirrored load source assembled | by the storage service at every IPL of a mirrored host, with the units and the device |
| `C2003020` | Resynchronization started | a unit is brought back in step: after a stop that was not in order, or when a unit that was away returns; with the unit |
| `C2003025` | Resynchronization finished | the unit is in step again |
| `C2003030` | Rebuild started | a replacement disk was put in a unit's place and is being written, with the unit |
| `C2003035` | Rebuild finished | the replacement is in step |
| `C2003040` | Mirrored protection started | a second disk was added to an unprotected load source; the set is assembled at the next IPL |
| `C2003050` | Load-source unit kept | the operator chose which of two units that ran apart to keep, with the unit; the other is written again from it |
| `C2004000` | Installation started | the installer writes from here on |
| `C2004010` | Installation media verified | the media's manifest is signed by a trusted key and the payload has its hashes |
| `C2004020` | Disks partitioned | each load-source disk has its table |
| `C2004030` | Protection set up | the mirrored set's label is on both disks; or the load source is unprotected |
| `C2004040` | Data file system made | `QSVSDATA` is formatted |
| `C2004050` | Slot A copied | slot A's UKI is on every load-source disk and reads back |
| `C2004055` | Slot B copied | the same for slot B |
| `C2004060` | A/B record written | on every load-source disk |
| `C2004070` | Boot entries written | a firmware entry for each slot of each disk, with `BootOrder` |
| `C2004080` | State written | the host name, the network, the time and the service tools user |
| `C2004090` | Installation complete | the host restarts into what was installed |
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
| `A2003010` | Mirrored load source not assembled | no unit that is in step was found, with the reason; `/data` is not mounted |
| `A2003020` | Load-source unit failed | a unit is missing or failed, with its number, location code and serial; at every IPL while it lasts, and when it happens |
| `A2003030` | Rebuild not finished | a resynchronization or a rebuild ended early, with the reason |
| `A2003040` | Load-source units ran apart | each unit of the mirrored pair ran without the other, with the units; `/data` is not mounted and nothing else starts until the operator chooses the unit to keep |
| `A2004010` | Installation media refused | the manifest's signature or a payload hash is wrong; nothing was written |
| `A2004015` | Installation answers not valid | an `[install]` table was found and refused, with the reason; nothing was written |
| `A2004090` | Installation failed | a step failed, with the step and the reason; the installer starts over |
| `A2001075` | Host identity not generated | the key pair or certificate could not be made or written |
| `A2004070` | Boot entries not written | the firmware's variables could not be written; the disks still start by their default path |
| `B2001000` | Host PID 1 failed | by PID 1's panic handler, with the panic message, before it parks |
| `B2003000` | Storage service stopped | the storage service of a mirrored load source failed five times in ten minutes; the host parks |
| `D2001000` | Host shutdown started | an orderly shutdown began, with what asked for it |
| `D2001090` | Host powering off | services are stopped and `/data` is unmounted; power off follows |
| `D2001091` | Host restarting | services are stopped and `/data` is unmounted; a restart follows |
