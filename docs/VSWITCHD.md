# The switch service

`vswitchd` (`net/vswitchd`, run as QS/VS by `lic/qsvsvswitchd`) keeps
the host's virtual switches and the partitions' ports on them
(`docs/NETWORK.md`). It is a supervised service of PID 1's table, apart
from the network service so that a fault in switch code cannot take the
management network away: started after the network service and the pool
service and before the partition manager, which asks it for a
partition's ports; stopped after the partition manager, leaving the
kernel's objects as they are.

It is driven over a local socket. Nothing listens on the network: the
QSI host server of Phase 8 will sit on this socket, and the virtual
Ethernet part of its `vres` family maps onto the requests below one to
one. In a release image Service Tools asks it for the switches and the
interfaces and makes, changes and deletes switches; the partition
manager asks for ports; nothing else sends it a request.

## The socket

`/run/qsvsvswitchd.sock`, a Unix stream socket, root's alone. One
request and one answer per connection, each a frame of QS/400's `ipc` (a
length and the encoded value), as the partition manager's and the pool
service's. A request is a two-byte number and its arguments; an answer
is the request's own type. A request that is refused answers with
**every** message that applies (`docs/MESSAGES.md`, `QVS40xx` to
`QVS42xx` and the profile's `QVS122x`) and changes nothing; one that
changes a switch has done it in the kernel and written `vswitch.toml`
before it answers, and a step the kernel refuses puts back what was.
The crate's client waits up to 30 s.

Two requests carry more than frames:

- **open port** (11): after an answer that gives the port, the tap's
  open file follows on one byte of its own (`SCM_RIGHTS`, `base/fdpass`).
  The service keeps no copy of it.
- **subscribe** (15): after its answer, every event follows, a frame
  each, for as long as the connection is open.

| # | Request | Arguments | Answer | Refused when |
|---|---|---|---|---|
| 0 | switches | | every switch: as kept, its bridge, its uplink's state (`None`, `Up`, `Degraded`, `Down`, `Missing`) and each interface's carrier, how many ports, why it was not built | |
| 1 | create | name, VLANs, MTU (0: 1500) | the switch | the name (`QVS4001`, `QVS4002`); a VLAN (`QVS4004`, `QVS4005`); the MTU (`QVS4006`); too many switches (`QVS4007`); the kernel (`QVS4012`) |
| 2 | change | name, VLANs, MTU | | not there (`QVS4003`); the VLANs and MTU as for create; a VLAN taken away that a profile uses (`QVS4009`, each) or a running port is on (`QVS4015`); an MTU an uplink interface cannot carry (`QVS4105`) |
| 3 | delete | name, forced | | a running partition's port is on it (`QVS4015`, each); a profile names it and it is not forced (`QVS4008`, each); the partition manager does not answer (`QVS4012`) |
| 4 | rename | name, new name | | the new name (`QVS4001`, `QVS4002`); a profile names it (`QVS4008`) |
| 5 | set uplink | name, interfaces by location code, bond mode, native VLAN | | an interface not on the host (`QVS4101`), the management interface (`QVS4102`), another switch's (`QVS4103`), named twice (`QVS4107`); several and no bond mode (`QVS4104`); none or more than 8 (`QVS4106`); a native VLAN not carried (`QVS4010`); an MTU not carried (`QVS4105`) |
| 6 | clear uplink | name | | not there |
| 7 | interfaces | | every network interface of the host: name, location code, MAC address, driver, speed, carrier, largest MTU, what it is (`Management`, `Uplink of S`, `Bond member of S`, `Not used`) | |
| 8 | ports | switch, or empty for every one | each port: partition, slot, switch, VLANs, MAC address, its tap, link, and counters (bytes, frames and drops the partition sent and was sent) | |
| 9 | forwarding table | switch | each entry: MAC address, VLAN, where (a partition and slot, `uplink`, `switch`), how (`static`, `learned`, `local`) | not there |
| 10 | VLANs | | for each switch and VLAN: whether native on the uplink, and the partitions and slots on it, tagged or not | |
| 11 | open port | partition, slot, switch, port VLAN, tagged VLANs, MAC address | the port, its tap, the switch's MTU; then the tap's file | the switch is not there (`QVS1223`) or not built (`QVS4201`); a VLAN it does not carry (`QVS1224`, each); a group address (`QVS1213`); another port's address (`QVS4202`); too many ports on the switch (`QVS4013`) or of the partition (`QVS1222`); a kernel step (`QVS4201`, and nothing left) |
| 12 | close port | partition, slot (0: every one of its) | how many were | |
| 13 | limits | | the limits | |
| 14 | change limits | limits | | a limit out of range (`QVS4011`); fewer switches than there are (`QVS4007`) |
| 15 | subscribe | | nothing; then every event | |
| 16 | reconcile | the partitions that run | how many ports were closed | |

A slot opened again is a new port: the old one is closed first.

The ports' specifications are kept in `/run/vswitchd/ports`, so a
service started again knows the tagged VLANs of each; the kernel is the
truth of which ports there are (the taps on its bridges).

## Events

| Event | With |
|---|---|
| switch | its name, its status or none when it was deleted, the time |
| uplink | the switch, the interface's location code, whether it has carrier, the uplink's state, the time |
| port | the partition, the slot, the switch, whether it is open now, the time |

A subscriber that has gone is dropped at the next event. Every event is
written to the host's log too.

## Reference codes

The `C2008xxx` and `A2008xxx` block (`docs/REFCODES.md`): the service
started with how many switches it built and ports it found, a switch
made, changed and deleted, an uplink's interface back; a switch not
built, an uplink with no carrier, a bond member without it, an uplink's
interface not on the host.

## What the partition manager does with it

At activation it asks for a port for each of the profile's adapters, in
slot order, after the pool service has given the virtual disks and
media, and gives the taps to `qsvmm` as the descriptors after the pool
service's files (`docs/PARTD.md`). When the VMM ends it closes them. A
required adapter whose port is refused fails the activation; one that
is not required comes up without link. When it starts it sends
**reconcile** with the partitions it found running. When it saves a
profile it asks for the switches, and refuses an adapter on a switch or
VLAN that is not there; when the switch service does not answer, that
is checked at activation instead.

The switch service, for its part, asks the partition manager (its
request 25) which profiles' adapters name a switch or a VLAN before it
deletes, renames or takes away either; and, when it starts, which
partitions run.
