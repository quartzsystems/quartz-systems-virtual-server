# The partition service channel, on the host's side

A partition has a service channel to the host: the system in it posts
its reference codes and its state on it, the host asks it for a power
down and for control panel function 21, and its console is on it as a
full screen. The contract between the two sides is QS/400's
`docs/QSID.md`, and the protocol is QS/400's crate `qsipart`, pinned
with the others: nothing of it is restated here. This document is what
the host does with it.

Nothing listens on the network for it. The partition manager speaks
the protocol, and everything it learns is on its local socket
(`docs/PARTD.md`), which Phase 8's QSI host server sits on.

## Who does what

| | |
|---|---|
| `qsvmm` | gives the partition a virtio-console device with the two named ports, present from the kernel's first instruction (`docs/VMM.md`). It carries bytes between each port and a listening Unix socket the partition manager made and handed it as a descriptor, one peer at a time, and reads none of them |
| `partd` | binds the two sockets before it starts the VMM, connects to them, reads the system's frames with `qsipart`, and acts on them: the partition's state, its current code and history, the requests it asks of the system, the console's screen, the not-responding rule (`part/svcchan`, `part/console`, `part/partd/src/manager/service.rs`) |
| the system | `qsid` in QS/400 says hello, posts its codes and states, answers requests; its console draws on the console port (`docs/QSID.md`) |

The sockets are `/run/partd/<ID>.service.sock` and
`/run/partd/<ID>.console.sock`, root's alone. A partition manager
started again connects to them again: they are the VMM's, not its own.

## The ports

Each partition has a virtio-console device with two ports, named as
`qsipart::SERVICE_PORT` and `qsipart::CONSOLE_PORT` name them; the
partition manager writes the names into the VMM's configuration
(`[[port]]`), so the VMM spells nothing of the protocol. The emulated
serial port stays: the kernel's messages, the system's serial console,
and its log under `/data/log/partitions/<ID>` are as before.

The guest finds the ports by name in sysfs. The host's side of a port
is open while the partition manager is connected; the guest is told
when it opens and closes (`PORT_OPEN`), and sees `POLLHUP` while it is
closed. What the guest writes while nobody is connected is dropped,
and so is what the host sends while the guest has the port closed.

## States

| The partition is | When |
|---|---|
| Starting | from its activation until its system reports its IPL `complete`; again when the system reports `ipl` |
| Running | its system reported `complete`; or its kernel was entered and its system has no service channel: its LIC record says so, or says nothing and the system did not open the service port in the time set (`channel_wait_secs`, 30 s by default). It is marked as having **no service channel** and `C2009005` is in its history |
| Shutting down | a shut down asked of the system was accepted, or the system reported `ending`, or `down` without a terminal code, or the host asked its VMM to end, until the VMM ends |
| Error | held with its memory after a terminal code or a guest fault (below); or its VMM failed or was killed, holding nothing |
| Not activated | its VMM ended and nothing follows |

The LIC record says beforehand whether the system has a channel (TLV
`0x010C`, QS/400's `docs/QSID.md` section 11; the VMM reports it as
`CHANNEL`, `docs/VMM.md`):

| The record says | The partition manager |
|---|---|
| it has one | waits for it with no limit: Starting until the system reports its IPL complete |
| it has none | does not wait: Running once the kernel is entered, marked as having none |
| nothing (a system older than QS/400's handoff 18, a blank disk, an IPL from media) | waits `channel_wait_secs` |

A system that opens the channel after all clears the mark, and its
state is its system's from then on.

A partition manager started again takes each partition's state from
its run record, which it rewrites at every change, so a partition is
in the state it was in until its system says otherwise.

## Reference codes

Each history entry says whose it is: the **system's**, posted on the
channel, or the **host's** about the partition (`docs/REFCODES.md`).
The history is kept on disk, `/data/state/partitions/<ID>/history`, the
last 256, across activations, partition manager restarts and the
host's IPLs; it goes with the partition when the partition is deleted.

The **current reference code** is:

| | |
|---|---|
| the system's latest | while it posts them |
| the host's latest | before the system has posted one in this activation, and after the VMM ends |
| the host's attention code | for a condition of the host's: held for its pool's space (`A2005160`), its adapter's uplink lost (`A2009080`), not responding (`A2009020`), channel closed (`A2009010`), held after a guest fault (`A2005170`); the next host code of progress gives the current code back to the system's |

The `C2009` codes of progress (the channel connected, no channel, a
shut down accepted, a dump removed, a panel function) and `C2006010`
(found running by a partition manager started again) are notes of the
history and never the current code. A code the system says is current
when the channel comes up again, and the history has already, is not
entered a second time.

The system's codes are not posted through PID 1: the host's history
and console are the host's own. They are in the host's log, as every
event of the partition manager is.

## The hello

The system's hello is checked against what the host gave the
partition: its serial number, and its keylock (normal or manual) where
the system read one. A difference is said once an activation: a
message (`QVS2316`) in the host's log and `A2009030`; the partition
runs on. The system's name, release, and the version and capabilities
both sides have are kept; its fix level and addresses come with its
state. All of it is in the partition manager's list (`Status.service`)
and its events. The host answers every hello with its own, and a time
request with its clock.

## What the system sends is hostile

Frames are read by `qsipart`'s reader, which never holds more than four
frames' worth and passes over junk a byte at a time. Every field kept is
bounded (a code 16 characters, a text 256, 32 addresses, the hello's
fields 16). The channel is closed when the system sends more than
2000 frames in a second, more than 16 bodies of a known kind that do
not read, or more than 1 MiB that is no frame: `A2009010` and
`QVS2317` in the log. The partition runs on; the channel is connected
again after 30 seconds, and the system says hello again. A kind of a
later version is passed over, never nonsense.

## Not responding

The host pings every 10 seconds while the channel is up. A system that
has answered nothing for three intervals is **not responding**:
`A2009020`, an event, and its current code; nothing is done to it.
While the host has its processors stopped (held, a dump being taken,
held for its pool's space) its silence is not counted; the three
intervals start again when they run.
When it answers again: `C2009020`, an event, and the current code is
the system's again.

## Shut down

| Kind | What | Forced |
|---|---|---|
| Delayed | the system's controlled power down (`PWRDWNSYS OPTION(*CNTRLD)`) with the delay given, 0 to 99999 seconds | when the delay and `delayed_grace_secs` (120 s) have passed |
| Operating system | the system's immediate power down (`OPTION(*IMMED)`) | after `system_timeout_secs` (300 s) |
| Immediate | the VMM is stopped, as before Phase 7 | the VMM is killed 10 s after it was asked |

Each with or without restart. The first two need the channel up with
the power-down capability: a partition with no channel refuses them by
name (`QVS2301`). The request answers when the system has accepted
(`C2009040` or `C2009041`) or refused (`QVS2303`, with the system's own
message identifier, `QSI0010` before its IPL is complete for one), or
after 15 seconds without an answer (`QVS2304`). Accepted, the partition
is Shutting down; when its time has passed and its VMM still runs, it
is shut down immediately: `A2009040`, and `QVS2305` as its message.

A restart asked of the system (`RESTART(*YES)`, from inside or with a
shut down from the host) ends the VMM with a restart, and the
partition is activated again with the same resources from the side it
recorded, as Phase 4 has it. An immediate restart, function 03 and
function 22 activate it again with the profile it had and this
activation's keylock and IPL type.

The settings are `[service]` of `partitions.toml` (`docs/RESOURCES.md`).

## The host's own shut down

PID 1's request (`end all`) ends the partitions one at a time, the last
activated first. One whose system has the channel up and its IPL
complete is asked for an operating-system shut down and given
`host_timeout_secs` (300 s); one that refuses, does not answer, or has
no channel is shut down immediately. PID 1 waits that time and the
VMM's grace for each, posts `D2001010`, and only then stops the
services.

## Held with its memory

A partition whose system posts a terminal code (a `B1` code) and then
`down`, or a `B1` code and nothing for 10 seconds, is **held**: the
partition manager has its VMM stop the processors and keep everything
(`hold` on its control socket). It is in Error with the terminal code
as its current code and `C2009050` in its history; it holds its
processors and memory.

A guest fault the VMM survives is held the same way by the VMM itself:
a triple fault, a halt with nothing to wake it, an entry KVM refused,
an exit the VMM does not handle. `A2005170` says which and is the
current code. A VMM that itself fails still ends the partition, in
Error, holding nothing.

From held: a dump (function 34, or a dump request), an immediate shut
down, or a restart. It is not activated or resumed otherwise.

## Dumps

`docs/DUMPS.md`.

## The console

The partition manager keeps the latest whole frame of the console
port, applying the system's updates as they come whether or not anyone
watches (`part/console`, with `qsipart::console::Grid`). Frames and
updates are checked (the size, a cell for every place, every run on the
grid, the cursor on it); one that fails is dropped and a whole frame
asked for, three times at most until one comes. No attribute is read.

One viewer at a time. A viewer attaches on the partition manager's
socket (`docs/PARTD.md`, request 32) and gets the whole frame at once,
then every update; its keys go to the system. A second viewer is
refused (`QVS2314`) unless it takes the console over; the one it
displaces is told and detached. A viewer that does not take what it is
sent is detached. A screen capture (request 33) is the whole frame
alone, for the suites and for QS/MC's reference code pages.

With the profile's console mode serial, a viewer has the serial port's
bytes instead (`ConsoleOut::Serial`), from the VMM's console socket,
and its `Bytes` go to it; the attach answers that it is serial.

Every activation writes the console mode to the LIC record: the
console port (`licpart::ConsoleMode::Partition`) unless the profile
asks for serial. Serial stays valid for a system older than QS/400's
handoff 17.

## When either side restarts

| | What happens |
|---|---|
| the partition manager restarted | the partitions run on; each state is the run record's; the new manager connects to the sockets again, the system says hello and its current code again, which is not a second history entry; the console's next viewer gets a whole frame |
| the system's daemon restarted | the system says hello again and its code and state; the waiting requests of the old connection are answered `QVS2301` |
| the VMM ended | the channel goes with it; a viewer is told the partition ended; the waiting requests are answered `QVS2009` |
| a connection closed for nonsense | made again after 30 s |

## Auto start's wait

A definition's `auto_start.wait` has auto start wait, after activating
the partition, until its system has reported its IPL complete, or
`wait_secs` (600 s by default) have passed, before the next
(`docs/PROFILE.md`).
