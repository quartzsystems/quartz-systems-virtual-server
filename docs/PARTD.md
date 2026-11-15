# The partition manager

`partd` (`part/partd`, run as QS/VS by the binary `qsvspartd`) keeps
every partition's definition and profiles, gives out the host's
processors and memory at activation, and owns every `qsvmm` process.
It is a supervised service of PID 1's table, started after the storage
service, the log service and the network service and before Service
Tools, which lists what it has.

It is driven over a local socket. Nothing listens on the network: the
QSI host server of Phase 8 will sit on this socket, and its host
family maps onto the requests below one to one. In a release image
nothing sends it a request to act on a partition; Service Tools only
lists, and PID 1 asks for one thing, at the host's shutdown. In a test
image the test hooks carry the conformance harness's requests to it.

## A partition's state

| State | Means |
|---|---|
| Not activated | no VMM; the partition holds nothing |
| Starting | its VMM runs and its system has not reported its IPL complete |
| Running | its system reported its IPL complete; or it has no service channel and its kernel was entered |
| Shutting down | a shut down was asked for or the system is ending, and its VMM has not ended yet |
| Error | held with its memory after a terminal code or a guest fault: its VMM is kept, its processors stopped, and it holds what it held. Or its VMM failed or was killed: it holds nothing, is not started again by itself, and can be activated again |

These are the names QS/MC shows. The states follow the system's own on
its service channel (`docs/SERVICE.md`, "States"). A system that never
opens the service port is Running once its kernel was entered (`qsvmm`
reports `C2005090`) and the wait for a channel is over, and is marked
as having none. **Open firmware is never reported**: a partition has
no firmware. The VMM loads the kernel itself (`docs/ARCHITECTURE.md`,
"Partition IPL"), so there is no moment at which a partition sits in
firmware waiting.

Each partition has a **current reference code** and a **history** of
the last 256, oldest first, kept on disk across activations, partition
manager restarts and the host's IPLs. Each entry says whose it is: the
system's, from its service channel, or the host's about the partition
(`docs/REFCODES.md`): what its VMM reported (`C2005xxx`, `A2005xxx`),
how the VMM ended, and the partition manager's own (`C2006xxx`,
`C2009xxx` and `A2009xxx`). The current code is the system's while it
posts them, the host's before it has and after the VMM ends, and the
host's for a condition of its own (`docs/SERVICE.md`, "Reference
codes").

### What ends a partition, and what follows

| The VMM ends because | Code | The partition is then |
|---|---|---|
| the guest powered off | `C2005100` | Not activated |
| the guest asked for a restart | `C2005110` | activated again at once, with the same resources, from the side it asked for (`recorded`) |
| the partition manager shut it down | `A2005140` | Not activated; activated again with the profile it had, this activation's keylock and IPL type, when the request was a restart |
| it could not IPL the partition | `A2005100`, after the code that says why | Not activated, with `QVS2108` naming that code |
| the VMM failed, or it was killed | `A2005130`, `A2005150` | Error, with the code and how the VMM ended (`QVS2109`); not restarted |
| the activation failed before any VMM ran | `A2006010` | Not activated, with the message |

A VMM asked to end is given 10 seconds, then every process of its
control group is killed. A VMM that something other than the partition
manager ends with `SIGTERM` leaves its partition Not activated.

A guest fault does not end the VMM: the partition is held in Error
with `A2005170` (`docs/SERVICE.md`, "Held with its memory"). A
system's own restart (`PWRDWNSYS RESTART(*YES)`, or a delayed or
operating-system shut down with restart) is the guest's restart of the
table.

## The socket

`/run/qsvspartd.sock`, a Unix stream socket, root's alone. One request
and one answer per connection, each a frame of QS/400's `ipc` (a
length and the encoded value), as PID 1's control socket and the
storage service's. A request is a two-byte number and its arguments;
an answer is the request's own type. A request that is refused answers
with **every** message that applies (`docs/MESSAGES.md`), and changes
nothing. A request that changes a definition, a profile or the
settings has written its state file before it answers; the crate's
client waits up to 30 s for an answer, where PID 1's waits 2 s.

The types are the crates' own: `Definition` and `Profile`
(`part/model`, `docs/PROFILE.md`), `Current`, `HostResources`, `Pool`
and `Settings` (`part/resource`, `docs/RESOURCES.md`), `Status`,
`Posted` and `Event` (`part/partd`).

| # | Request | Arguments | Answer | Refused when |
|---|---|---|---|---|
| 0 | list | | every partition in ID order: ID, name, state, current reference code and its text, current resources, the profile last activated with, how its VMM last ended, the message of a failure, the VMM's process, and each virtual Ethernet adapter's slot, switch, MAC address and link | |
| 1 | create | name | the definition, with its ID and serial number assigned | the name is not one or is taken (`QVS1001`, `QVS1005`); the host has as many partitions as it is set to (`QVS2006`) |
| 2 | get definition | ID | the definition | no such partition (`QVS2001`) |
| 3 | change definition | definition | | activated (`QVS2002`); a fixed field differs (`QVS1006`); the definition's own errors; the name is taken; the default profile is not the partition's (`QVS1007`) |
| 4 | delete | ID, its disks too | | activated. Its profiles go with it. Its virtual disks are deleted first when asked, every one or none (the pool service's messages); otherwise they are left alone |
| 5 | list profiles | ID | their names | |
| 6 | get profile | ID, name | the profile | no such profile (`QVS2004`) |
| 7 | put profile | ID, profile | | every error of the profile (`QVS11xx`, `QVS12xx`). The first profile of a partition becomes its default. A partition may be running: its profile is the next activation's |
| 8 | delete profile | ID, name | | it is the default profile (`QVS2007`) |
| 9 | activate | ID, profile name (empty: the default), an IPL source or none, a keylock position or none | the resources it was given | activated already; no such profile, or no default (`QVS2005`); the profile's errors; source D with no optical device (`QVS1219`); a section a later phase brings (`QVS2106`); a host path in a release image (`QVS1218`), or one that is not there (`QVS2110`); a virtual disk or medium the pool service cannot give (its messages, `QVS2112` when it does not answer); a required Ethernet adapter's port the switch service cannot give (`QVS2117` and its messages, `QVS2116` when it does not answer); a resource that is not available (`QVS2101` to `QVS2105`); the VMM could not be started (`QVS2107`); the host is shutting down (`QVS2010`) |
| 10 | shut down | ID | | not activated (`QVS2003`); shutting down already (`QVS2009`) |
| 11 | restart | ID | | the same |
| 12 | host resources | | installed, configurable and available processors and memory, the reserved cores, each pool | |
| 13 | create pool | pool | | `QVS22xx` |
| 14 | change pool | pool | | partitions run in it (`QVS2208`); `QVS22xx` |
| 15 | delete pool | name | | the same |
| 16 | history | ID | the partition's reference codes, oldest first | |
| 17 | get settings | | `partitions.toml` | |
| 18 | change settings | settings | | a value out of range (`QVS2201`). The pools are not changed by this |
| 19 | end all | | how many partitions are being ended | |
| 20 | subscribe | | nothing; then every event, a frame each, for as long as the connection is open | |
| 21 | load media | ID, slot, name | | not running (`QVS2003`); no optical or tape device in the slot (`QVS2113`); the pool service's messages; the VMM refused (`QVS2115`) |
| 22 | unload media | ID, slot, forced | the medium's name | the same; the partition prevents its removal and it is not forced (`QVS2114`) |
| 23 | media | ID | each optical and tape device: slot, kind, the medium, whether the partition prevents its removal | not running |
| 24 | named | pool, virtual disk | the profiles that name it, each `PROD profile NORMAL slot 2` | |
| 25 | adapters | ID, or 0 for every partition | each virtual Ethernet adapter of the partition's profiles: the partition, its name, the profile, the slot, the switch, the port VLAN, the tagged VLANs, the MAC address, whether it is required | |
| 26 | shut down as | ID, kind (delayed, operating system, immediate), delay, restart | | not activated (`QVS2003`); shutting down already (`QVS2009`); a delay over 99999 (`QVS2302`); delayed or operating system with no service channel (`QVS2301`); refused by the system (`QVS2303` with its message) or not answered in 15 s (`QVS2304`) |
| 27 | panel | ID, function, its arguments (IPL source, IPL type, keylock, each optional) | the text it displays | `docs/PANEL.md`: `QVS2306` to `QVS2309`, the keylock, the system's refusal |
| 28 | take dump | ID | the dump's name; it is written in the background | not activated; one is being taken (`QVS2313`); no media repository (`QVS2310`); it could not begin (`QVS2311`) |
| 29 | dumps | ID | each dump, oldest first: name, time, size, the bytes it takes, the code, why it was taken | no such partition |
| 30 | delete dump | ID, name | | no such dump (`QVS2312`) |
| 31 | read dump | ID, name | its size; then its bytes on this connection, a frame of at most 128 KiB each, and an empty frame | no such dump |
| 32 | console | ID, take over | whether it is the serial port's; then the console on this connection, both ways | another viewer has it and this one does not take over (`QVS2314`) |
| 33 | screen | ID | the console's whole frame as it is, and its text | nothing shown yet (`QVS2315`) |
| 34 | put profile at | ID, the profile, the revision it was read at (none for a new one) | its new revision | as put profile; a name that is there with no revision (`QVS2012`); not at that revision, or deleted since (`QVS2013`) |
| 35 | save current | ID, a profile's name, the revision as 34 | its new revision | not activated (`QVS2015`); as 34 |
| 36 | create with | a name, the first profile | the definition, the profile its default | as create and as put profile; when the profile is refused nothing is kept |
| 37 | change at | the definition, the revision it was read at | its new revision | as change; not at that revision (`QVS2014`) |
| 38 | profile entries | ID | each profile's name and revision, and whether it is the default | no such partition |

**Revisions.** Every definition and profile has a revision, one more
at every write the partition manager makes of it, 0 in a file older
than revisions. What a caller gives in the field is not kept. The
requests that carry a revision (34, 35, 37) are refused when it is not
the one kept, so two clients cannot both write over what one of them
read: QSI's profile put and save current map onto them
(`docs/QSISERVER.md`). The requests without one (3, 7) write as before
and move the revision on. **Save current** (35) is the profile an
activated partition runs with, with what its activation gave it as the
desired processors and memory, its minimum and maximum widened to take
them. **Create with** (36) is a create and a put profile as one: a
profile refused deletes the partition it made.

**Activate** answers when the VMM has been started: the partition is
then Starting, and a list or the events say how it goes on. The two
overrides are for that activation only: an IPL source in place of the
profile's, and a keylock position in place of the definition's. The
definition is not changed by them.

**Shut down** (10) and **restart** (11) are immediate: the VMM is
stopped; they answer when the VMM has been asked. They work on a
partition held in Error, and on one whose system is shutting it down.
**Shut down as** (26) is all three kinds (`docs/SERVICE.md`, "Shut
down"); a delayed or operating-system one answers when the system has
accepted or refused it, or after 15 seconds. **Panel** (27) answers
with the function's text, function 21 when the system has accepted it.
These two are answered by the service when the system has, without
holding the partition manager meanwhile.

**Console** (32): after its answer the connection stays open. The
service sends `ConsoleOut` frames, a whole frame first and then each
update, or the serial port's bytes; the client sends `ConsoleIn`: a
key, a refresh, or bytes for the serial port. `Ended` and the
connection closed say it was taken over or the partition ended. The
forms are QS/400's (`qsipart::console`); nothing of them is
interpreted on the way. The client's `console` and `ConsoleSession`
are the crate's.

**End all** is PID 1's request before the host powers off or restarts:
the partitions are ended one at a time, in the reverse of the order
they were activated in, each through its own system when it has its
IPL complete and its channel up (an operating-system shut down given
`host_timeout_secs`, 300 s by default), immediately otherwise, and
nothing is activated afterwards. PID 1 waits that time and the VMM's
grace for each until a list shows none activated, posts `D2001010`, and
only then stops the services.

## Events

Every state change, reference code and resource change is published to
the subscribers on the socket and written to the host's log.

| Event | With |
|---|---|
| state | the partition's ID and name, the state it is in now, the time |
| code | the ID and name, and the code posted: time, code, text |
| resources | the ID and name, what it holds now (nothing when it has ended), the time |
| defined | the ID and name, and whether the definition still exists: a partition was created, changed or deleted, or a profile of it was |
| channel | the ID and name, and the service channel as the list has it: up, none, responding, held, the system's name, release, fix level and addresses, the version and capabilities: when it comes up or goes, stops or starts answering, or the system reports a change |
| dump | the ID and name, a dump's name, and why it was not written when it was not |
| profile | the ID and name, a profile's name, its revision now (none when it was deleted), and whether it is the default: a profile was put or deleted, or the default moved to or from it |

The list (request 0) has the same of the channel in each partition's
`service`.

A subscriber that does not take an event within two seconds is
dropped.

## Activation, step by step

1. The profile is found and checked again, with every error.
2. The resources are resolved (`docs/RESOURCES.md`).
3. The control groups are set: the pools as they are with this
   partition holding its part, then the partition's own group.
4. The pool service is asked to hold the profile's virtual disks and
   the media its optical and tape devices load, all at once
   (`docs/POOLD.md`); it hands each over open.
4a. The switch service is asked for a port for each of the profile's
   virtual Ethernet adapters, in slot order (`docs/VSWITCHD.md`); it
   hands each tap over open. A required adapter whose port is refused
   fails the activation, and what was held is given back; one that is
   not required comes up without link.
5. `qsvmm` is started in that group, in a session of its own, with its
   standard error appended to `/run/partd/<ID>.report`, and the held
   files as its descriptors 3 and on, in order, the taps after the pool
   service's files. Its whole
   configuration is written to its standard input (`docs/VMM.md`): the
   resolved processors and their host threads, the memory, the IPL
   source, the keylock, IPL type and console mode, the units and the
   optical and tape devices, the Ethernet adapters, a held file named
   by its `/proc/self/fd/N` path.
6. What the partition holds is recorded in `/run/partd/<ID>.record`,
   with its state, which is written again at each change.
7. The partition is Starting. Each line of the report that is a
   reference code of the table goes into its history, is published,
   and is posted through PID 1. The service channel's sockets
   (`/run/partd/<ID>.service.sock`, `.console.sock`), made before the
   VMM and handed to it as its descriptors after the taps, are
   connected to; the system's codes and states drive the partition
   from then on (`docs/SERVICE.md`).
8. When the VMM ends, what it held is given back (the pool service
   releases its disks and media, the switch service closes its ports),
   its group is removed, the end's code
   is posted, and the table above says what follows.

## Media while a partition runs

A medium is loaded into an optical or tape device, or unloaded, by
requests 21 and 22: the partition manager has the pool service hold
it, then hands it to the VMM over the VMM's control socket
(`docs/VMM.md`). What a guest does by itself, an eject or an unload,
the VMM reports as a line `MEDIA <slot> unloaded <name>`; the
partition manager gives the medium back. A guest's restart loads what
was loaded when it restarted.

## A pool that fails

The partition manager asks the pool service for the pools' states
every two seconds. When a pool is Failed, every active partition with
a virtual disk in it is ended (its control group killed), and is in
Error with `QVS2111` naming the pool. Partitions in other pools run on.

The report is the VMM's, which a guest may have got at. A line is read
to 512 characters, only a code the host's table has is taken from it,
and the history is bounded.

## Auto start

When the host's base is ready (every service has been ready), the
partitions flagged to start with it are activated with their default
profiles, in the order of their `auto_start.order` and then of their
IDs, each after its delay. One whose `auto_start.wait` is set is waited
for until its system reports its IPL complete, or its `wait_secs` have
passed, before the next. `C2006020` says how many there are and
`C2006030` how many were activated. One that fails posts `A2006020`
with its message and does not stop the rest. It is done once an IPL:
a partition manager started again does not do it over.

## When the partition manager itself is started again

A partition does not end because `partd` did. Each `qsvmm` is in a
session and a control group of its own and writes its report to a
file, so it neither receives its starter's signals nor loses its
standard error.

A partition manager that starts reads the records in `/run/partd`.
For each, when the partition's control group still has a process, the
partition is taken up: it holds what its record says and is in the
state recorded, its VMM is asked whether it is held, `C2006010` is
posted, its service channel is connected to again (the system says
hello and its current code again), and it is watched again, by its
control group this time (the VMM is another process's child by now).
Its history is the one kept on disk. When it ends, how is read from the report's last line,
`END <exit code>`, which `qsvmm` writes for exactly this; with no such
line it was killed. A record whose group is empty is a partition that
ended while nobody watched: it is ended then and there, the same way.
The switch service is then told which partitions run, and closes the
ports of any other. Then `C2006000` is posted with how many partitions
are defined and running, before the service says it is ready: at the host's IPL the
code so comes before `C2001090`, and the host's current reference code
is that one.

`/run` does not outlive an IPL, so a host that restarts starts with no
partition activated, and auto start brings back the ones flagged.

When a profile is saved, an Ethernet adapter that gives no MAC
address is given one, kept in the profile from then on
(`docs/NETWORK.md`), and its switch and VLANs are checked with the
switch service.

## Limits

The number of partitions is a setting (`max_partitions`, 64 by
default; 999 at most, the IDs there are). A partition has 64 profiles
at most. Each `qsvmm`'s own control group has a memory and a process
limit (`docs/RESOURCES.md`), so a VMM that misbehaves cannot starve
the host.

## Not here yet

- Any change to a running partition's resources.
- `BLOCKED(phase7)`: a VMM found running by a partition manager started again is watched by looking at its control group every 25 ms, not by an event.
