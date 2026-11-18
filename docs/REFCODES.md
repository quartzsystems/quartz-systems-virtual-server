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
A posting may say what it is about (`refcode::about`): each resource's
kind, the name a person finds it by, its location code and its serial
number, and a partition's ID. The history keeps that as the line's
`about`, and the problem service reads it (`docs/PROBLEMS.md`).

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
| `C2005xxx`, `A2005xxx` | a partition: its activation and IPL in order, what it lacked when it could not IPL, and how it ended (`qsvmm`) |
| `C2006xxx`, `A2006xxx` | the partition manager (`partd`): its start, partitions found running, auto start, an activation that failed before a VMM ran |
| `C2007xxx`, `A2007xxx` | storage pools and the media library (`poold`, the pool service): assembly, creation, resynchronization and rebuild, hot spares, imports, how full a pool is |
| `C2008xxx`, `A2008xxx` | virtual switches (`vswitchd`, the switch service): its start, switches made, changed and deleted, an uplink's carrier lost and back, a switch not built |
| `C2009xxx`, `A2009xxx` | the partition manager's service channel to each system (`partd`, `docs/SERVICE.md`): the channel up, closed or silent, the shut down asked of a system and one forced, a partition held with its memory, dumps removed for the limits, the control panel functions, an adapter's uplink |

| `C2010xxx`, `A2010xxx` | the QSI host server (`qsvsqsid`, `docs/QSISERVER.md`): its start and addresses, pairing, a console removed |
| `C2011xxx`, `A2011xxx` | service: the host's dumps (`docs/DUMPS.md`) |
| `C2012xxx`, `A2012xxx` | code update (`docs/UPDATE.md`): staged, verified, installed, activated, confirmed or rolled back |
| `C2013xxx`, `A2013xxx` | physical I/O (`docs/PHYSICALIO.md`): the IOMMU at the IPL (PID 1), devices released, bound, given, given back and fenced (`partd`), physical tape drives, a running partition's entitlement |

Later phases take further blocks and record them here.

## A partition's codes

The codes of the `C2005` and `A2005` blocks are the host's, about a
partition: its own reference codes are QS/400's, on its own console.
`qsvmm` has no way to PID 1 once it has dropped its privileges, so it
reports a code as a line `REF <code> <text>` on its standard error,
and the partition manager, which started it, posts the line as it
stands (`docs/VMM.md`, `docs/PARTD.md`). What a posting adds to its
code's text starts with the partition's name: `REF C2005090 Partition
started: PROD, side A`. An IPL posts `C2005000` to `C2005090` in
order; one that cannot go on posts the `A2005` code that names what
was missing and then ends with `A2005100`. How the partition ended is
posted by the partition manager, from the exit code of `qsvmm` or the
signal that killed it, with the partition's name and that code.

The `C2006` and `A2006` blocks are the partition manager's own: that
it started and what it found running, auto start, an activation that
failed before any VMM ran. The `C2009` and `A2009` blocks are its too,
about each partition's service channel (`docs/SERVICE.md`): the
channel connected, closed or silent, a shut down asked of the system
and one forced, a partition held with its memory, a dump removed, a
panel function, an adapter's uplink. A `C2009` code of progress is a
note of the history and never the current code. So is a `C2011` code of
a host dump: a dump taken when a service stopped or a slot went back
leaves that code the host's current one.

Since Phase 7 the system in a partition posts its own codes on its
service channel: they are QS/400's, in its own families, and go into
the partition's history marked as the system's beside the host's,
never through PID 1. A partition's current reference code is the
system's while it posts them, the host's before the system has posted
one and after the VMM ends, and the host's for a condition of the
host's: `docs/SERVICE.md` says which. The partition manager keeps a
history of the last 256 for each, on disk.

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
| `C2001045` | Data file system has room | `/data` is under 85% used again after it passed 90%, with how much; by the problem service |
| `C2001050` | Clock set from the RTC | with the date and time |
| `C2001060` | Seed checked | the seed step applied nothing: the host is configured, or there is no seed volume |
| `C2001061` | Seed applied | the state files were written from `seed.toml` |
| `C2001070` | State loaded | the state files are read and the inventory written, with the host name |
| `C2001072` | Partition memory reserved | the memory partitions can be given is set aside as 2 MB huge pages, with the pages and the MiB |
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
| `C2005000` | Partition activation started | `qsvmm` read its configuration, with the partition's name |
| `C2005010` | Partition disk units opened | every disk unit is open for direct I/O, with their number and the load source's size |
| `C2005020` | Partition load-source table read | the load source has a GPT whose checksums hold |
| `C2005030` | Partition LIC record read | with the side the partition asked for (`once` when for this IPL only), the keylock position, the console mode and the record's sequence |
| `C2005035` | Partition IPL attributes written | the keylock position, the IPL type or the console mode of the activation is in the LIC record, or the side asked for one IPL was taken out of it |
| `C2005037` | Partition has no LIC record yet | the LIC partition is blank and the guest writes the first record; or there is none and the IPL is from a side that was named: the keylock position, IPL type and console mode are then written nowhere |
| `C2005040` | Partition IPL image read | the side's UKI was read from the ESP, with the side and its size |
| `C2005045` | Partition installation media read | IPL source D: the media UKI was read from the ISO 9660 image in the first optical device, with the image's volume identifier and the UKI's size |
| `C2005050` | Partition kernel loaded | the kernel, the page tables, the GDT and the zero page are in guest memory |
| `C2005060` | Partition machine built | with the processors, the memory and the disk units |
| `C2005090` | Partition started | the processors are running, with the name and the side |
| `C2005100` | Partition powered off | the guest powered off: exit code 0 |
| `C2005110` | Partition ended for a restart | the guest asked for a restart: exit code 10. `qsvmm` does not restart in place: the partition manager activates the partition again |
| `C2005160` | Partition resumed | a partition held for its pool's space has room again: its processors run, and the write that found the pool full was done |
| `C2005180` | Partition dump started | the partition's processors are stopped and its memory and processor state are being written to a dump (`docs/DUMPS.md`) |
| `C2005181` | Partition dump in progress | how far the dump is, in percent of the partition's memory, every tenth |
| `C2005185` | Partition dump written | the dump is whole, with its size and the bytes it takes; a running partition's processors go on |
| `C2006000` | Partition manager started | with the partitions defined and those found running |
| `C2006010` | Partition found running | a partition that ran while the partition manager did not is watched again, with its name |
| `C2006020` | Partition auto start started | with the number of partitions flagged to start with the host |
| `C2006030` | Partition auto start complete | every flagged partition was tried, with how many were activated |
| `C2009000` | Partition service channel connected | the system said hello on its service port, with its release and the protocol version both sides speak |
| `C2009005` | Partition has no service channel | the partition's kernel was entered and its system did not open the service port in the time set: it is Running without one |
| `C2009020` | Partition responding again | a system that was not responding answers pings again |
| `C2009040` | Partition delayed shut down requested | the system accepted a controlled power down, with the delay |
| `C2009041` | Partition operating system shut down requested | the system accepted an immediate power down |
| `C2009050` | Partition held with its memory | the system posted a terminal reference code and halted: its processors are stopped and its memory kept for a dump; the terminal code stays its current one |
| `C2009060` | Partition dump removed | the oldest dump of the partition was removed to keep its dumps within their number and size |
| `C2009070` | Partition control panel function | a function that acts on the partition was carried out, with its number |
| `C2009080` | Partition adapter uplink back | the uplink of a switch one of the partition's adapters is on has carrier again |
| `C2010000` | QSI server started | the server listens on TCP 2300 of the management addresses, with them and the host's certificate fingerprint |
| `C2010005` | QSI server addresses changed | the management network's addresses changed and the server listens on the new ones, with them |
| `C2010020` | QSI pairing started | a pairing password was made at the host's console; it is good for one pairing, 15 minutes and five attempts |
| `C2010030` | QSI console paired | a console paired with the password, with its name and the start of its certificate's fingerprint |
| `C2010040` | QSI pairing ended | the password is gone unused: it expired, its attempts ran out, or pairing was ended at the host |
| `C2010050` | QSI console removed | a console's pin was removed at the host, with its name; it can connect no more |
| `C2011000` | Host dump started | by PID 1, with what asked for it: a console's user, Service Tools, a service stopped after its retries, or a slot that did not confirm |
| `C2011010` | Host dump written | with its name and size (`docs/DUMPS.md`) |
| `C2011020` | Host dump removed | deleted, or the oldest put out for the limits, with its name |
| `C2012000` | Update image staged | an update image was received whole, with its name and size, and is being verified |
| `C2012010` | Update image verified | its signature, hashes, product, release and state schemas hold, with its release and build |
| `C2012015` | Staged update discarded | the staged image was thrown away |
| `C2012020` | Update install started | with the slot written to |
| `C2012025` | Update slot written | the slot's image was written, synchronized and read back on one load-source disk, with the disk |
| `C2012040` | Update installed, slot pending | every disk has the image and the A/B record marks the slot pending; the firmware's next boot is set |
| `C2012045` | Pending update cancelled | the pending slot is pending no more; the next IPL is the default's |
| `C2012050` | Update activation started | the restart into the pending slot begins, with the partitions shut down first |
| `C2012055` | State kept for the pending slot | before the pending slot's first IPL reads state, PID 1 copied `/data/state` |
| `C2012060` | Update confirmed | the pending slot confirmed, with the release that runs now; the copy of state is dropped |
| `C2012075` | State restored after a rollback | the slot that came back put the copy of state back before any service read it |
| `C2013000` | Physical I/O checked | at the IPL, by PID 1: the IOMMU and interrupt remapping are on, with how many released devices were bound |
| `C2013010` | Device released for partitions | a device and the rest of its isolation group were released at Service Tools option 10, with their location codes |
| `C2013015` | Device taken back by the host | a device and its group were taken back at Service Tools option 10; the host's drivers have them again |
| `C2013020` | Released device bound | at the IPL, a released device was given to the pass-through driver before any service started |
| `C2013030` | Device given to a partition | at an activation, a device was reset and its descriptors handed to the partition's VMM, with the partition |
| `C2013035` | Device given back | at a partition's end, a device was reset and is free again, with the partition |
| `C2013040` | Partition entitlement changed | a running shared partition's processing units, capping or weight changed, with before and after and who asked |
| `C2013050` | Physical tape drive found | a physical tape drive that had gone away is on the host again |
| `C2007000` | Pool service started | with the pools found and how many were assembled |
| `C2007010` | Storage pool assembled | a pool's units were found by their labels, its set opened and its file system mounted, with its name, protection and state |
| `C2007015` | Storage pool created | with its name, protection, units and capacity |
| `C2007020` | Pool resynchronization started | a pool stopped uncleanly: a mirrored pool copies the regions its write-intent map has dirty, a parity pool computes the parity it had not, with the extent |
| `C2007025` | Pool resynchronization finished | the pool's units are in step |
| `C2007030` | Pool rebuild started | a unit is written again from the others, with the pool, the unit and its location code |
| `C2007035` | Pool rebuild finished | the unit is in step |
| `C2007040` | Hot spare taken into a pool | a hot spare took a failed unit's place, with both units' location codes; its rebuild follows |
| `C2007050` | Media image imported | an optical image is in the media library, with its name and volume identifier |
| `C2007055` | Storage pool under 80% allocated | a pool that passed 90% is under 80% again, with the pool |
| `C2007090` | Storage pools stopped | at the host's shutdown, after every partition ended: each pool's file system unmounted and its set stopped in order |
| `C2008000` | Switch service started | with the switches built from state and the partitions' ports found running |
| `C2008010` | Virtual switch created | with its name, VLANs and MTU |
| `C2008015` | Virtual switch changed | its VLANs, MTU, name or uplink, with what changed; nothing that runs on it is stopped |
| `C2008016` | Virtual switch deleted | with its name; no profile named it |
| `C2008020` | Uplink interface has carrier again | with the switch and the interface's location code; outside traffic resumes |
| `A2001005` | Host restarted after a kernel panic | the kernel panicked (an oops is a panic); its last messages, kept in the firmware's variables, were moved to `/data/log/panics` at this IPL and are in the next host dump, with why it panicked (`docs/SECURITY.md`) |
| `A2001030` | A/B record not valid | no copy of the record is valid; a new one is written for the slot booted |
| `A2001035` | Pending slot not confirmed | the pending slot did not confirm on its IPL; this IPL is on the default and the pending field is cleared |
| `A2001040` | Data file system not mounted | `/data` could not be mounted, with the reason; state is in memory only. Nothing is reformatted |
| `A2001045` | Data file system nearly full | `/data` is 90% used or more, with how much; by the problem service, which looks every minute |
| `A2001050` | Clock not set from the RTC | the RTC could not be read |
| `A2001060` | Seed not valid | a seed volume was found and refused, with the reason; nothing was written |
| `A2001070` | State file not valid | a state file was refused, with the file and the reason; its defaults are in use |
| `A2001072` | Partition memory not reserved in full | the kernel gave fewer huge pages than the settings ask for, with both numbers; the configurable memory is what it gave |
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
| `A2005000` | Partition profile not valid | the configuration `qsvmm` was given could not be read or a value in it is refused, with the value |
| `A2005010` | Partition disk unit cannot be read | a disk unit could not be opened for direct I/O or read, with the unit and the error |
| `A2005020` | Partition load source has no partition table | no GPT, or one whose checksum or a field does not hold, with which |
| `A2005021` | Partition load source has no EFI system partition | the table has no partition of the ESP's type |
| `A2005022` | Partition EFI system partition cannot be read | the ESP holds no FAT16 or FAT32 volume, or a damaged one |
| `A2005030` | Partition has no IPL image for the side | the ESP has no UKI at the side's path, with the side |
| `A2005031` | Partition IPL image cannot be read | the UKI's cluster chain is damaged or the file is larger than 128 MiB |
| `A2005032` | Partition has no installation media | IPL source D, and the partition's first optical device has no medium |
| `A2005033` | Partition installation media cannot be read | the medium is not an ISO 9660 image, or one whose structures do not hold, with what is wrong |
| `A2005034` | Partition installation media has no IPL image | the image has no file at `EFI/BOOT/BOOTX64.EFI` |
| `A2005040` | Partition load source has no LIC partition | the IPL is from the recorded side and the table has no LIC partition to read it from |
| `A2005041` | Partition LIC record not valid | neither copy of the LIC record is valid and the partition is not blank |
| `A2005050` | Partition IPL image is not a kernel | the UKI is not a bzImage with a 64-bit entry, with what is wrong |
| `A2005060` | Partition machine not built | KVM, memory, a device or the privilege drop failed, with which |
| `A2005065` | Partition memory not available | the huge pages of the partition's memory could not be reserved, with the amount |
| `A2005100` | Partition did not IPL | `qsvmm` ended before the partition ran: exit code 30. The code before it says why |
| `A2005120` | Partition ended with a guest fault | a triple fault, or KVM could not enter the guest: exit code 20 |
| `A2005130` | Partition ended with a VMM error | `qsvmm` failed while the partition ran: exit code 40, or one it does not use |
| `A2005140` | Partition ended by the host | `qsvmm` was sent `SIGTERM`: exit code 50 |
| `A2005150` | Partition VMM killed | `qsvmm` was killed by a signal, with the signal |
| `A2005160` | Partition held for storage pool space | a write to a thin virtual disk found its pool full: the partition's processors are stopped, it stays Running, nothing reaches the guest as an error, and it goes on by itself (`C2005160`) when the pool has room |
| `A2005170` | Partition held after a guest fault | a triple fault, a halt with nothing to wake it, an entry KVM refused or an exit the VMM does not handle: the processors are stopped and the memory kept, the partition is in Error until it is dumped, shut down or restarted |
| `A2005180` | Partition dump not written | a dump could not be written whole, with why: nothing of it is kept |
| `A2005190` | Partition VMM ended by its confinement | `qsvmm` made a system call its seccomp filter does not allow and the kernel ended it (`SIGSYS`); the partition is in Error, holding nothing (`docs/SECURITY.md`) |
| `A2006010` | Partition not activated | an activation failed before `qsvmm` ran, with the partition's name and the message |
| `A2006020` | Partition not started automatically | a partition flagged to start with the host could not be activated, with its name and the message; the others are still tried |
| `A2006030` | Partition state not valid | a definition, a profile or `partitions.toml` was refused, with the file and the reason |
| `A2006040` | Resource controls not set | a control group could not be made or a value written, with the file and the error |
| `A2006090` | Partitions not ended in time | at the host's shutdown the partition manager did not report every partition ended; the shutdown goes on |
| `A2009010` | Partition service channel closed | the system sent what is not the protocol: the channel is closed until the system says hello again, and the partition runs on |
| `A2009020` | Partition not responding | the system has not answered pings on its service port for three intervals; nothing is done to it (`C2009020` when it answers again) |
| `A2009030` | Partition identity differs | the system's hello names another serial number, side or keylock than the host gave it, with which |
| `A2009040` | Partition shut down forced | a delayed or operating-system shut down had not ended in its time and the partition was shut down immediately |
| `A2009055` | Partition held on a terminal code | after `C2009050`, when what held the partition was its system's terminal code, with the partition and the code: the problem it opens shows the system's code |
| `A2009080` | Partition adapter uplink lost | the uplink of a switch one of the partition's adapters is on lost carrier: its outside traffic stopped |
| `A2010000` | QSI server not listening | the server has no identity to present, or no management address to listen on, with which; nothing listens until it has |
| `A2010010` | Processor lacks what the service interface needs | at the IPL, by PID 1: the processor lacks AVX2, ADX, AES or PCLMULQDQ, which QSI's cryptography needs, with which; the server listens on nothing and touches no cryptography, and everything local works |
| `A2010030` | QSI pairing attempt failed | an attempt to pair did not confirm (a wrong password, or a relay), with the attempts left |
| `A2010035` | QSI pairing attempts failing | a pairing password's attempts were all spent, or an address was turned away for its failed handshakes and pairings |
| `A2011010` | Host dump not written | a host dump could not be written whole, with why; nothing of it is kept |
| `A2012010` | Update image refused | the staged image did not verify, with which check: signature, a hash, the product, an older release without leave, or state it cannot read |
| `A2012030` | Update not installed | an install failed, with the step and the disk; the slot is not pending and the running slot is as it was |
| `A2012070` | Update rolled back | the pending slot did not confirm and the host came back on the other slot, with the slot and why |
| `A2013010` | IOMMU or interrupt remapping off | at the IPL, by PID 1: the firmware has DMA remapping or interrupt remapping off, with which; nothing can be released and no device is given, and everything else works |
| `A2013030` | Released device not found | at the IPL, a released device is not on the host, with its location code; it is not bound and partitions that require it are not activated |
| `A2013035` | Released device group changed | at the IPL, a released device's isolation group is not what it was when it was released, with what changed; it is not bound |
| `A2013050` | Device did not reset | a device did not reset before a partition got it or after one ended, with the partition: it is fenced, given to no partition until the host restarts |
| `A2013080` | Physical tape drive went away | a physical tape drive a partition held is gone (cable, power): its commands end with an error until it comes back |
| `A2007010` | Storage pool not assembled | a pool's units were found but its set could not be opened or its file system mounted, with the pool and why; only the partitions that use it are affected |
| `A2007020` | Pool unit failed | a unit was taken out of service or not found, with the pool, the unit, its location code and serial number |
| `A2007025` | Storage pool degraded with no hot spare | a unit of a protected pool failed and no hot spare was there to take, with the pool: another failure may lose it |
| `A2007030` | Pool rebuild not finished | a rebuild or resynchronization stopped, with the pool and why |
| `A2007040` | Storage pool failed | more units are lost than the pool's protection covers: the pool is Failed, nothing more is written to its units, and its partitions are in Error |
| `A2007050` | Storage pool 80% allocated | a pool's allocated space passed 80% of its size, with the pool, what is allocated and what is provisioned |
| `A2007051` | Storage pool 90% allocated | the same at 90% |
| `A2007060` | Media image not imported | an import of an optical image failed, with the name and why; nothing of it is left |
| `A2008010` | Virtual switch not built | a switch of the state file could not be built at the service's start, with its name and why; its partitions' required adapters cannot be given ports |
| `A2008020` | Uplink lost carrier | every interface of a switch's uplink lost carrier, with the switch and the location codes: outside traffic stops, traffic between its partitions goes on |
| `A2008025` | Uplink bond member lost carrier | one interface of a bonded uplink lost carrier, with the switch and its location code; the others carry the traffic |
| `A2008030` | Uplink interface not found | an interface the state names as an uplink is not on the host, with the switch and the location code |
| `B2001000` | Host PID 1 failed | by PID 1's panic handler, with the panic message, before it parks |
| `B2003000` | Storage service stopped | the storage service of a mirrored load source failed five times in ten minutes; the host parks |
| `D2001000` | Host shutdown started | an orderly shutdown began, with what asked for it |
| `D2001010` | Partitions ended | every partition was ended, in the reverse of their start order; the services are stopped next |
| `D2001090` | Host powering off | services are stopped and `/data` is unmounted; power off follows |
| `D2001091` | Host restarting | services are stopped and `/data` is unmounted; a restart follows |
