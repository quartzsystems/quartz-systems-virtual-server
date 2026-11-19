# Virtual Ethernet

How partitions are networked: virtual switches on the host, a
partition's virtual Ethernet adapters, the uplinks that carry a switch
outside, and what a partition may and may not send. The switch service,
`vswitchd` (`net/vswitchd`, run as QS/VS by `lic/qsvsvswitchd`), keeps
all of it; its socket is `docs/VSWITCHD.md`. An adapter belongs to a
profile (`docs/PROFILE.md`); the device a guest sees is `docs/VMM.md`.

## A virtual switch

A switch is a named, VLAN-aware Linux bridge with VLAN filtering on and
spanning tree off. It has:

- a **name**, 1 to 10 of `A` to `Z`, `0` to `9` and `_`, a letter first;
- a **number**, given when it is made and never changed: the kernel's
  objects are named by it (`qvbr<n>` the bridge, `qvbo<n>` its bond), so
  a rename touches nothing in the kernel;
- the **VLANs** it carries, 1 to 4094;
- an **MTU**, 1280 to 9000, 1500 when made with none: its uplink's and
  every port's;
- at most one **uplink**. A switch with none is **internal**: its
  partitions reach each other and nothing else.

Switches are kept in `/data/state/vswitch.toml`, a state file of
`base/state`, with the limits. At the service's start every switch is
built from it; what the kernel has already is kept and brought in line,
so a service started again disturbs no traffic. A bridge or bond of a
number the file does not have is the service's own leftover and is
deleted; an interface the service does not own is never touched.

## An uplink

One physical interface, or a bond of several, each named by its
location code (`docs/LOCATION.md`), never by its kernel name. Bonds are
`active-backup` (needs nothing of the physical switch) or `802.3ad`
(LACP, fast rate, layer 3 and 4 hashing). An uplink is a trunk: every
VLAN of the switch is tagged on it, except one that may be chosen as its
**native** VLAN, carried untagged. An interface belongs to one switch.

The uplink's MTU is raised to the switch's. A switch MTU an interface
cannot carry (its driver's largest) is refused (`QVS4105`), when the
uplink is set and when the MTU is changed. An MTU raised goes to the
uplink first, then the ports, then the bridge; one lowered, the other
way.

**The management interface is never an uplink.** The device `netd`
manages the host through is refused as an uplink (`QVS4102`), and `netd`
never takes for management a link that is on a bridge or bond, or that a
switch's state names as an uplink (`qsvsnetd` reserves those), even at
an IPL before the switch service has started. A host with one network
port therefore has internal switches only. Whether management may later
ride a VLAN of a switch is open (`docs/OPEN_QUESTIONS.md` item 37).

## A partition's adapter

An `[[ethernet]]` section of a profile: its `slot`, its `switch`, its
port VLAN (`vlan`, untagged to the guest), further `tagged` VLANs passed
with their tags, its `mac`, and whether it is `required`
(`docs/PROFILE.md`). Eight at most a partition.

At activation the partition manager asks the switch service to open a
**port** for each adapter. The service makes a tap (`qvt<partition>s<slot>`)
with virtio-net headers, turns IPv6 off on it, gives it the switch's MTU,
puts it on the bridge with the port rules below, its VLANs and its
address's static forwarding entries, and hands its open file back
(`base/fdpass`). The partition manager gives it to `qsvmm` as one of its
descriptors, after the pool service's files; `qsvmm` takes it as it is
(a tap's file opened again by its path would be another tap) and gives
it to vhost-net. Packets do not pass through `qsvmm`.

The tap is not persistent: it goes when the last open file of it does,
so a partition that ends takes its port with it, however it ends. The
service forgets a port whose tap is gone, every second; and when either
it or the partition manager starts, the ports of partitions that do not
run are closed.

An adapter that is **not required** whose switch is missing, not built
or refuses the port comes up without link: the guest has the device, its
link status is down, and what it sends goes nowhere. A **required** one
fails the activation by name (`QVS2117` with the switch service's
message).

An adapter's settings apply at the next activation. A switch's VLANs,
MTU and uplink change while partitions run on it; a running guest is not
told a new MTU.

## MAC addresses

A profile that gives an adapter no address has one made when the
adapter is first saved, and kept in the profile: locally administered
and unicast, three bytes of a prefix the host derives once from its
serial number and keeps in `partitions.toml` (`mac_prefix`), then the
partition's ID in two bytes and the slot in one. It never changes
afterwards. A group address is refused (`QVS1213`); an address another
partition has is refused (`QVS1225`); deleting a partition frees its
addresses, and only that does.

## What a partition may send

Only frames from its adapter's own address, and only on its port VLAN
(untagged) or one of its listed tagged VLANs. Everything else is dropped
at the port. These are the bridge's own controls:

| Rule | How |
|---|---|
| Its own source address only | the port is **locked** with learning off; a static forwarding entry for the adapter's address in each of its VLANs is the only one the port has |
| Its VLANs only | per-port VLAN membership with VLAN filtering: the port VLAN as PVID and untagged, the tagged ones tagged |
| No traffic for others | unknown unicast is not flooded to a partition's port |

Two things the pinned kernel's bridge cannot refuse by itself: with
spanning tree off it forwards a BPDU rather than take it (BPDU guard
acts only with spanning tree on), and a frame tagged with the port's own
VLAN is taken into it like an untagged one. For those each partition's
tap has an ingress filter, a clsact qdisc with flower rules and gact
actions (`net/nl/src/filter.rs`): a frame to a reserved link-local group
(`01:80:C2:00:00:0X`, BPDUs among them) is dropped; a frame tagged with
one of the adapter's tagged VLANs, or priority-tagged (VLAN 0), passes;
any other 802.1Q or 802.1ad frame is dropped. That is tc, not
netfilter: `NETFILTER` stays off. The kernel additions are
`NET_SCH_INGRESS`, `NET_CLS_FLOWER` and `NET_ACT_GACT`
(`NET_SCHED` and `NET_CLS_ACT` were on already).

A guest that sets another address through the control queue has it
recorded (`NET <slot> the guest set its address to ...` in the host's
log) and not passed on: the port still passes only the profile's
address, so the guest loses its own traffic and nobody else's is
affected.

## What the host exposes

Nothing. The host has no address on any switch, bridge, bond, uplink or
tap, IPv4 or IPv6: IPv6 is turned off on each before it is brought up,
and the bridge itself is in no VLAN (its default port VLAN is 0), so no
frame a partition sends reaches the host's stack. IPv4 and IPv6
forwarding are off: nothing is forwarded between switches or between a
switch and the management network. A partition cannot reach the host
through its adapters.

## The host's own interfaces

`network.toml` (version 2, `base/state`) lists the host's interfaces,
each with the role the appliance names, and `base/netd` runs each on
its own: its device (a name, a MAC, or the first Ethernet link with
carrier that no other interface has), DHCP or a static address, an
optional VLAN, and whether it carries the default route (one at most),
the name servers and the time. A reload or a change on trial undoes
and remakes only the interfaces that changed, and a change is judged
by those alone. QS/VS names one role, `management`, and has one
interface, so its file is version 1's interface as the first of the
list: an interface with no role is the first role's, and one written
by version 1 migrates to that with nothing else changed. Nothing
listens on any of them but the QSI host server (`docs/QSISERVER.md`).

## Limits

Settings of `vswitch.toml`'s `[limits]`, with their defaults:

| Limit | Default | Range |
|---|---|---|
| `switches` | 16 | 1 to 64 |
| `vlans_per_switch` | 256 | 1 to 4094 |
| `adapters_per_partition` | 8 | 1 to 8 |
| `ports_per_switch` | 256 | 1 to 1024 |

An adapter has 32 tagged VLANs at most; an uplink 8 interfaces.

## What happens on each failure

| Failure | What follows |
|---|---|
| An uplink interface loses carrier, another of its bond has it | `A2008025` naming the interface by location code, an event; the bond carries on |
| Every interface of an uplink loses carrier | `A2008020`, an event; outside traffic stops, traffic between the switch's partitions goes on |
| An interface gets carrier back | `C2008020`, an event; outside traffic resumes |
| An uplink's interface is not on the host at the start | `A2008030`; the switch is built without it |
| A switch cannot be built at the start | `A2008010`; its required adapters get no port, the others come up without link |
| `vswitch.toml` is refused | `A2008010` naming the file; nothing in the kernel is taken away |
| The switch service ends | the bridges, ports and taps are the kernel's and stay; traffic goes on; PID 1 starts it again and it finds them |
| A VMM ends, however | its taps close with it, its ports go, the service forgets them |
| The partition manager ends | the partitions run on with their ports; when it starts again it has the switch service close the ports of partitions that do not run |
| The host shuts down | PID 1 ends every partition, then stops the services; the switch service leaves the kernel's objects as they are |

## Not here

Multiple queues, rate limits and traffic shaping, port mirroring,
management on a switch's VLAN, a physical adapter given to a partition
(Phase 10), and any address or service of the host on a partition
network.
