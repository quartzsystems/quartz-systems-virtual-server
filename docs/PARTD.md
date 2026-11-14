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
| Starting | its VMM runs and has not yet entered the guest's kernel |
| Running | the guest runs |
| Shutting down | the partition manager asked its VMM to end, and it has not yet |
| Error | its VMM failed or was killed. The partition holds nothing, is not started again by itself, and can be activated again |

These are the names QS/MC shows. **Starting** lasts until the IPL
loader has entered the kernel, which is when `qsvmm` reports
`C2005090`; Phase 7 refines it with the guest's own reference codes,
which arrive on the service channel. **Open firmware is never
reported**: a partition has no firmware. The VMM loads the kernel
itself (`docs/ARCHITECTURE.md`, "Partition IPL"), so there is no
moment at which a partition sits in firmware waiting.

Each partition has a **current reference code**, the last it posted,
and a **history** of the last 64, oldest first. The codes are the
host's about the partition (`docs/REFCODES.md`): what its VMM reported
(`C2005xxx`, `A2005xxx`), how the VMM ended, and the partition
manager's own (`C2006010`, `A2006010`).

### What ends a partition, and what follows

| The VMM ends because | Code | The partition is then |
|---|---|---|
| the guest powered off | `C2005100` | Not activated |
| the guest asked for a restart | `C2005110` | activated again at once, with the same resources, from the side it asked for (`recorded`) |
| the partition manager shut it down | `A2005140` | Not activated; activated again with the profile it had when the request was a restart |
| it could not IPL the partition | `A2005100`, after the code that says why | Not activated, with `QVS2108` naming that code |
| the guest faulted, the VMM failed, or it was killed | `A2005120`, `A2005130`, `A2005150` | Error, with the code and how the VMM ended (`QVS2109`); not restarted |
| the activation failed before any VMM ran | `A2006010` | Not activated, with the message |

A VMM asked to end is given 10 seconds, then every process of its
control group is killed. A VMM that something other than the partition
manager ends with `SIGTERM` leaves its partition Not activated.

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
| 0 | list | | every partition in ID order: ID, name, state, current reference code and its text, current resources, the profile last activated with, how its VMM last ended, the message of a failure, the VMM's process | |
| 1 | create | name | the definition, with its ID and serial number assigned | the name is not one or is taken (`QVS1001`, `QVS1005`); the host has as many partitions as it is set to (`QVS2006`) |
| 2 | get definition | ID | the definition | no such partition (`QVS2001`) |
| 3 | change definition | definition | | activated (`QVS2002`); a fixed field differs (`QVS1006`); the definition's own errors; the name is taken; the default profile is not the partition's (`QVS1007`) |
| 4 | delete | ID | | activated. Its profiles go with it; its backing stores are left alone |
| 5 | list profiles | ID | their names | |
| 6 | get profile | ID, name | the profile | no such profile (`QVS2004`) |
| 7 | put profile | ID, profile | | every error of the profile (`QVS11xx`, `QVS12xx`). The first profile of a partition becomes its default. A partition may be running: its profile is the next activation's |
| 8 | delete profile | ID, name | | it is the default profile (`QVS2007`) |
| 9 | activate | ID, profile name (empty: the default), an IPL source or none, a keylock position or none | the resources it was given | activated already; no such profile, or no default (`QVS2005`); the profile's errors; a section a later phase brings (`QVS2106`); a backing store that is not there (`QVS2110`); a resource that is not available (`QVS2101` to `QVS2105`); the VMM could not be started (`QVS2107`); the host is shutting down (`QVS2010`) |
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

**Activate** answers when the VMM has been started: the partition is
then Starting, and a list or the events say how it goes on. The two
overrides are for that activation only: an IPL source in place of the
profile's, and a keylock position in place of the definition's. The
definition is not changed by them.

**Shut down** and **restart** are immediate: the VMM is stopped.
Delayed and operating-system shut down need the service channel of
Phase 7. Both answer when the VMM has been asked.

**End all** is PID 1's request before the host powers off or restarts:
the partitions are ended one at a time, in the reverse of the order
they were activated in, and nothing is activated afterwards. PID 1
waits until a list shows none activated, posts `D2001010`, and only
then stops the services.

## Events

Every state change, reference code and resource change is published to
the subscribers on the socket and written to the host's log.

| Event | With |
|---|---|
| state | the partition's ID and name, the state it is in now, the time |
| code | the ID and name, and the code posted: time, code, text |
| resources | the ID and name, what it holds now (nothing when it has ended), the time |
| defined | the ID and name, and whether the definition still exists: a partition was created, changed or deleted, or a profile of it was |

A subscriber that does not take an event within two seconds is
dropped.

## Activation, step by step

1. The profile is found and checked again, with every error.
2. The resources are resolved (`docs/RESOURCES.md`).
3. The control groups are set: the pools as they are with this
   partition holding its part, then the partition's own group.
4. `qsvmm` is started in that group, in a session of its own, with its
   standard error appended to `/run/partd/<ID>.report`. Its whole
   configuration is written to its standard input (`docs/VMM.md`): the
   resolved processors and their host threads, the memory, the IPL
   source, the keylock, IPL type and console mode, the units.
5. What the partition holds is recorded in `/run/partd/<ID>.record`.
6. The partition is Starting. Each line of the report that is a
   reference code of the table becomes its current code, is published,
   and is posted through PID 1; `C2005090` makes it Running.
7. When the VMM ends, what it held is given back, its group is
   removed, the end's code is posted, and the table above says what
   follows.

The report is the VMM's, which a guest may have got at. A line is read
to 512 characters, only a code the host's table has is taken from it,
and the history is bounded.

## Auto start

When the host's base is ready (every service has been ready), the
partitions flagged to start with it are activated with their default
profiles, in the order of their `auto_start.order` and then of their
IDs, each after its delay. `C2006020` says how many there are and
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
partition is taken up: it holds what its record says, the lines of its
report so far say where it is, `C2006010` is posted, and it is watched
again, by its control group this time (the VMM is another process's
child by now). When it ends, how is read from the report's last line,
`END <exit code>`, which `qsvmm` writes for exactly this; with no such
line it was killed. A record whose group is empty is a partition that
ended while nobody watched: it is ended then and there, the same way.

`/run` does not outlive an IPL, so a host that restarts starts with no
partition activated, and auto start brings back the ones flagged.

## Limits

The number of partitions is a setting (`max_partitions`, 64 by
default; 999 at most, the IDs there are). A partition has 64 profiles
at most. Each `qsvmm`'s own control group has a memory and a process
limit (`docs/RESOURCES.md`), so a VMM that misbehaves cannot starve
the host.

## Not here yet

- Delayed and operating-system shut down, console attach, panel
  functions, the guest's own reference codes: the service channel,
  Phase 7.
- Any change to a running partition's resources.
- `BLOCKED(phase4)`: a partition's history is in memory and in the host's log: a partition manager started again has the codes of the activations that still run, not the older ones.
- `BLOCKED(phase4)`: a VMM found running by a partition manager started again is watched by looking at its control group every 25 ms, not by an event.
