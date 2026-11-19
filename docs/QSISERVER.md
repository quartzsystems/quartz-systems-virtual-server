# The QSI host server

`qsvsqsid` (`qsi/server`, run by `lic/qsvsqsid`) is the one listener a
QS/VS host has. It speaks QSI (QS/400's `docs/QSI.md`) to a paired
management console and translates each request to the local request of
the service that owns the state. It owns no partition, pool or switch
state: only the consoles it paired, in `qsi-consoles.toml`, and its
event log, in `qsi-events.log`, both in `/data/qsi`.

QSI's messages, its session and its trust rules are QS/400's crates
(`qsi`, `qsisess`, `qsitrust`), pinned with the others. Nothing of them
is restated here.

## The process

PID 1 starts it last (order 35, after Service Tools) and stops it first.
As root it reads the host's identity (`identity.crt`, `identity.key` in
`/data/state`, made at the first IPL), makes `/data/qsi` its own, and
binds its local socket (`/run/qsvsqsid.sock`, root's alone). Then it
becomes user and group 22990 for good: no capability in any set, no
supplementary group but its own, `NO_NEW_PRIVS`. The sockets it asks
(PID 1's, the partition manager's, the pool service's, the switch
service's) are shared with that group (`release::share_with_server`):
`root:22990`, mode `0660`.

A host with no identity yet (no `/data` at its first IPL) posts
`A2010000` and looks again each minute; it listens on nothing until it
has one.

## The listener

TCP 2300 on each IPv4 management address, and nowhere else. The
addresses are PID 1's `management` fact when the server starts, and
then, each time the network service tells the server's local socket its
line changed, PID 1's fact read again (the network service notes it
there first, and two words close together may come in either order;
the line told is taken only when PID 1 does not answer): the listener
follows a DHCP renewal or a change made
at Service Tools at once (`C2010005`). An address it could not bind,
or PID 1 not answering at the start, is tried again every 2 seconds. No address, nothing listens (`A2010000`). Nothing listens
on a switch, a bridge, a tap or a partition's network, and nothing at
all before installation: the installer's media runs no service table.

| Limit | Value |
|---|---|
| Sessions at once | 6: two consoles, each reconnecting while its old session times out, and pairing |
| Handshakes under way at once | 4 |
| A handshake's time | 10 s, then the connection is dropped |
| Failed handshakes or pairings from one address | 10 in 60 s (more than one password's five attempts), then that address is turned away for 60 s |
| The session's own (`qsisess::Limits`) | as `docs/QSI.md` section 6: 1 MiB messages, 64 requests, 16 streams, 256 KiB windows, a log of 4,096 events or 4 MiB, keep-alive 15 s and timeout 45 s |

A connection turned away is closed at once and logged. Each admitted
connection is a thread: the TLS handshake (TLS 1.3 only, Ed25519, both
ends' certificates checked against the pins at every handshake,
nothing resumed), then the session's loop, which reads with a 20 ms
timeout so the session's clock, its events and its streams move while
the console says nothing.

## Who may connect

A console is pinned by its certificate's SHA-256. Two at most. A
handshake from a certificate that is not pinned is refused, unless a
pairing password is out and there is room: then it gets a session that
may only pair (`QSI0101` to anything else, for the session's whole life).
After pairing the console connects again.

A console removed at Service Tools can connect no more, and a session it
has is closed within a second (`C2010050`).

## Pairing

At Service Tools, option 4, then 2 (`docs/SERVICETOOLS.md`): the host
makes a password (`qsitrust::pair::one_time_password`, 16 Crockford
base32 characters, 80 bits; the handoff asked for 12, and QS/400's crate
is the one rule). It is shown on that panel and nowhere else: never
logged, never in a state file, kept in the server's memory only. It is
good for one pairing, 15 minutes and five attempts (`C2010020`). The
exchange is QS/400's SPAKE2 over the TLS connection's exporter: neither
end sends the password, and a relay that ends TLS on each leg fails
(`docs/QSI.md` section 3).

| What happens | Code |
|---|---|
| A password made | `C2010020` |
| A console paired: pinned, the password gone | `C2010030` |
| An attempt that did not confirm, with the attempts left | `A2010030` |
| The password gone unused: expired, out of attempts, or ended at the host (F12) | `C2010040` |
| A console removed | `C2010050` |

Each code goes through PID 1 into the host's history, so each is a base
`Refcode` event too.

## Sessions

The host's hello names it a host, with its name, release and serial
number and `caps::HOST_V1` (base, pair, and the nine host families) with
`caps::SERVICING`: `service`, its problems; `update`, its code update;
and base's system dumps, its own dumps; `caps::PHYSICAL_IO`: the host
family's `Devices` and its device events, vres's `TapeDrives` and its
event, a profile's physical I/O and a tape device's physical drive; and
`caps::PARTITION_V2`: the partition family's `Entitlement` and its
event. A request is checked by QSI's own bounds
(`QSI0002`), then translated (`qsi/server/src/families`, following
`docs/QSI.md` section 8):

| Family | To |
|---|---|
| base | PID 1 (state, power, a host dump taken), the log service's history file (`/data/log/refcodes.ndjson`, read only), the state files (host name, zone, inventory, read only), the host's dumps' directory (listed, read, deleted; `docs/DUMPS.md`) |
| host | the partition manager (resources, settings, devices), the inventory with the pool service's disks and the switch service's interfaces for each item's owner |
| partitions, partition, profiles, refcodes, panel, dumps, console | the partition manager; the partition family's `Entitlement` as the request's user |
| vres | the pool service (pools, virtual disks, images, volumes), the partition manager (drives, a partition's physical drive as `PhysicalTape`, physical tape drives, processor pools, where a disk is named, read from its profiles), the switch service (switches, ports, forwarding, VLANs, limits) |
| service | the problem service (`docs/PROBLEMS.md`): `List`, `Get`, and `Close` as the request's user; a service resource is QSI's `System` named `service <name>` |
| update | the update service (`docs/UPDATE.md`): `Status`, `Stage` as a stream, `Discard`, `Install`, `Activate` as the request's user, `Cancel` |

A refusal carries every message the service gave, the first first, with
its help (`refuse_with`). Where QSI names a conflict, its identifier
leads and QS/VS's follow: a stale profile put is `QSI0401` then
`QVS2013`, a new profile of a name there `QSI0402` then `QVS2012`, a
save of a partition not activated `QSI0403` then `QVS2015`; a problem
not there `QSI1002` then `QVS5001`, closed already `QSI1003` then
`QVS5002`, a close while its condition holds `QSI1001` then
`QVS5003`; an update service's refusal QSI names, `QSI11nn` then
`QVS53nn` (01 to 14); a device refusal QSI names, `QSI13nn` then
`QVS24nn` (`QSI1301` no such device, `1302` not released, `1303`
held, `1304` part of a group, `1305` not assignable now, `1306` no
such drive, `1307` a physical drive's slot loaded as a volume's), and an entitlement refusal `QSI14nn` then
`QVS25nn` (not running, dedicated, outside the profile, over the
virtual processors, short in the pool). A device is never released
over QSI: only at Service Tools option 10. A value of
an open enumeration the host does not have is `QSI0361`. A service that
does not answer (its socket missing, no answer in time) is `QSI0301`,
naming it; the session goes on and the other families answer.

What the host keeps to itself is left out: a VMM's process, a switch's
bridge, a port's tap, a pool's mount point.

What QSI v1 cannot carry: a partition created together with its first
profile (`partd`'s `CreateWith`) and a definition change at a revision
(`ChangeAt`). QSI's `Create` has a name only and its `Definition` no
revision; a console makes the partition, then puts its profile.

## Streams

| Stream | Far end | Credit |
|---|---|---|
| console `Attach` | the partition manager's console connection: its frames, updates and serial bytes out as `console::Out` pieces, the console's `In` pieces as keys, a refresh or serial bytes | a piece is sent when the console has credit for it; the manager's connection is read by a thread into a queue of eight pieces |
| dumps `Download` | the partition manager's dump read, its pieces as they come | as the console grants |
| base `DownloadDump` | the host dump's file, read by a thread in 16 KiB pieces | as the console grants |
| update `Stage` | the update service's stage: the bytes written to its connection as they come, exactly the size named | as the image upload's: the service's socket is the back-pressure |
| vres `ImageUpload` | the pool service's upload: the bytes written to its connection as they come, exactly the size named | each piece is granted back once the pool service took it: its socket is the back-pressure |

The upload's end is an event (vres `UploadEnded`): the image, or why
not. More bytes than named reset the stream (`QSI0701`); fewer, the pool
service sees its connection end and keeps nothing (`QSI0702`). A stream
whose console goes, or is reset, lets go of its far end at once: the
console is detached, the dump read stops, the upload's connection is
shut and the pool service removes what it had.

## Events

One log for every session (`qsisess::EventLog`), fed by:

| Source | How |
|---|---|
| the partition manager | its subscription; a `Resources` event also brings the host family's `Resources`; a device's change is the host family's `Released`, `TakenBack`, `Assigned`, `Returned` or `ResetFailed`, a drive's vres `TapeDrive`, an entitlement's the partition family's `Entitlement` |
| the switch service | its subscription |
| the pool service | it has none: the server looks every 2 seconds and says what changed (pools, virtual disks, images, volumes) |
| the problem service | its subscription: each problem opened, seen again, cleared or closed |
| the update service | its subscription: staged, verified, installing, installed, activating; `Confirmed` and `RolledBack` from PID 1's `C2012060` and `A2012070` in the history |
| the host | its history file looked at every 2 seconds (base `Refcode`; an `A2011010` also as base `DumpTaken` with its error), PID 1's state (base `State`), its dumps' directory (base `DumpTaken`, `DumpRemoved`) |
| uploads | their ends |

A source that goes away is subscribed to again every 2 seconds.

The log is kept on disk (`qsi-events.log`), bounded as the session's log
is, and synchronized every 2 seconds. A server started again takes it up
and numbers on, with `qsisess::EventLog::resume`: the last run of
events the file holds numbered one after another, and the next number
after them. A console whose last event is older than the first kept,
or newer than any, is told to read everything again. A line that does
not read is passed over, and a gap starts the run again: a damaged file
loses events, never the numbers.

What the services said while the server was down is in the log too.
Beside it the server keeps what the log last said of each thing
(`qsi-said.txt`): each partition's name, state and last code, each pool,
virtual disk, optical image, tape volume and switch, and the host's
place in its history. When it takes up a source, at its start or when
the source comes back, it compares the source with that and pushes what
differs before the source's own events:

| Source | Compared |
|---|---|
| the partition manager | after its subscription is answered: each partition made or deleted, each state, each code of a partition's history after the last the log said |
| the switch service | after its subscription is answered: each switch made, changed (its VLANs, MTU, uplink) or deleted |
| the problem service | after its subscription is answered: each problem the log never said (opened, and closed when it is), and each one cleared, closed or seen again since |
| the pool service | its first look: each pool, virtual disk, image and volume |
| the host | its history from the place the log last said |

An event that says only what the log said already (a state again, a code
the comparison pushed and the subscription then brings) is not pushed.
A source the server never compared before, as on its first start, has
its first look taken as said, with nothing pushed. Ports and uplink
carrier are the switch's status at the comparison, not each change.

## Logging

Each line to standard error, which PID 1 puts in the system log: a
session opened and closed, with the address and the console's name;
every request with its family and operation, the address and the user
the console passed (which grants nothing); a pairing and its outcome;
a connection turned away. Never a password, a key or a console's
screen.

## When something fails

| Failure | What happens |
|---|---|
| A local service is down | its requests are refused `QSI0301`; the others answer; it is asked again at the next request, and its events subscribed to again within 2 seconds |
| The server ends | PID 1 starts it again; sessions are dropped; consoles reconnect and resume their events |
| A console breaks the protocol | its session is closed and logged (`docs/QSI.md` section 4); nothing else is affected |
| A console stalls a stream | only that stream waits; requests and other streams go on |
| The management address changes | the network service tells the server and the listener moves (`C2010005`); a server that was away reads PID 1's fact when it starts |
| No identity | nothing listens (`A2010000`) until there is one |
| The processor lacks AVX2, ADX, AES or PCLMULQDQ | PID 1 posts `A2010010` at the IPL; the server touches no cryptography and listens on nothing, and its local socket answers with the reason (`qsilocal::processor`): no pairing, the consoles listed from their file; the check is `base/processor`'s, the words QS/VS's. Service Tools and the other services work |

## The local socket

`/run/qsvsqsid.sock` (`qsi/local`, crate `qsilocal`), root's alone, for
Service Tools option 4: the consoles, start pairing (the password, once),
the password out (without its text), end pairing, remove a console, and
what the server listens on; and for the network service, which tells it
the management interface's line when it changes. Nothing of QSI crosses
it.

## Proof

`cargo test -p qsiserver`: the families against fake services, the
connection's session, pairing and streams in memory, the event log's
file taken up again, and a console pairing over real TLS on loopback and
then answered pinned. `cargo run -p build -- fuzz --target qsi`: what a
pinned console sends, through the session and the translation. Phase
8's suites (`docs/PHASE8.md`) drive it from outside the host.
