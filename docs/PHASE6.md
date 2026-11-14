# Phase 6: virtual Ethernet

The record of Phase 6. Phases 0 to 5 are `docs/PHASE0.md` to
`docs/PHASE5.md`; what they say still holds, except where this says
what changed.

Phase 6 gives partitions a network: virtual switches on the host,
virtual Ethernet adapters in profiles, a virtio-net device in the VMM
over vhost-net, and uplinks to the physical network. There is still no
service channel, console attach or QSI: the switch service is driven
over a local socket, as the partition manager and the pool service are.
No re-pin was needed: QS/400 stays at `0fc8afa`.

## Answers to Phase 5's open questions

| # | Question | What was done |
|---|---|---|
| 35 | a copy of a virtual disk | left out until QS/MC needs template partitions; the reflink change to `xfsfmt` is a QS/400 handoff then |
| 36 | tape from QS/400 | as recommended: `INZTAP`, `SAVLIB` and `RSTLIB` against the virtual drive are the first thing of Phase 10 |
| 34 | a held partition's state for QS/MC | still Cody's to confirm; nothing changed |
| 6, 17, 22, 31, 32 | | unchanged |

Phase 6 raised 37, management on a VLAN of a switch
(`docs/OPEN_QUESTIONS.md`).

## What exists

`docs/NETWORK.md` has switches, uplinks, adapters, MAC addresses, what
a partition may send, what the host exposes, the limits and every
failure; `docs/VSWITCHD.md` the switch service's socket;
`docs/PROFILE.md` the `[[ethernet]]` section; `docs/VMM.md` the
virtio-net device; `docs/SERVICETOOLS.md` option 3's new panels.

### The netlink pieces (`net/nl`)

QS/400's `netlink` at the pin already creates and deletes links, bonds
and VLAN sub-interfaces, sets a link's master, MTU and state, reads
links with their statistics, and subscribes to link events: all of it
is used as it is (`netlink::route::Rtnl`). What it lacks is written
over its public `codec` and `Socket`:

- a bridge with VLAN filtering on, spanning tree off and no default
  port VLAN (`IFLA_BR_*`);
- a port's bridge attributes: learning, locked, BPDU guard, unicast,
  multicast and broadcast flood (`IFLA_PROTINFO`, `IFLA_BRPORT_*`);
- per-port VLAN membership with the PVID and untagged flags, on a port
  and on the bridge device itself, and reading every port's VLANs back
  (`IFLA_AF_SPEC`, `IFLA_BRIDGE_VLAN_INFO`, `RTEXT_FILTER_BRVLAN`);
- static forwarding entries and the forwarding table (`AF_BRIDGE`
  neighbours, `NTF_MASTER`, `NDA_VLAN`, `NDA_MASTER`);
- a link's largest MTU (`IFLA_MAX_MTU`, which `netlink::route::Link`
  does not parse);
- a clsact qdisc and flower filters with gact actions on a link's
  ingress (`RTM_NEWQDISC`, `RTM_NEWTFILTER`, `TCA_FLOWER_*`);
- taps through `/dev/net/tun` with virtio-net headers;
- IPv6 turned off and on per link (through its `sysctl`).

**For a later QS/400 handoff**, each of these belongs in QS/400's
`netlink`, beside its bonds and VLANs: the bridge, the port attributes,
the port VLANs and their dump, the static forwarding entries and the
table's dump, `IFLA_MAX_MTU` in `Link`, and the tc qdisc and flower
filter requests. The tap belongs in QS/400 too, but in a crate of its
own (it is an ioctl on a character device, not netlink). Nothing in
QS/400 needs them today; when they move, `net/nl` keeps only the port
rules that are QS/VS's own (`PortRules::PARTITION`,
`filter::partition_rules`).

`net/nl/tests/live.rs` runs the requests against a live kernel in a
network namespace of its own (`NL_LIVE=1 unshare -rn cargo test -p nl
--test live`, in WSL as an ordinary user): a bridge, three taps, and
frames written into them and read out of the others. It shows every
rule of the port: a foreign source address dropped, the VLANs kept
apart, a tagged frame from an access port dropped (its own VLAN's too),
a listed tagged VLAN passed with its tag, a BPDU dropped, unknown
unicast not flooded to partitions, the static entries in the table, the
counters, and a tap gone with its last file.

### The switch service (`net/vswitch`, `net/vswitchd`, `lic/qsvsvswitchd`)

`net/vswitch` is the model: `vswitch.toml` with the switches and the
limits, every error of each, a port's VLANs, and the MAC addresses.
`net/vswitchd` is the service: a core that asks a `Kernel` trait and an
`Env` trait for everything (unit-tested against a model of the kernel's
links, twelve tests), its Linux kernel over `nl`, and its socket. It is
the fifth service of PID 1's table (order 23), after the network
service and the pool service and before the partition manager.
`net/vswitchd/tests/live.rs` runs the real core over the live kernel
with dummy links as the host's interfaces: a bonded trunk, a port with
its static entries, a live MTU change, the tap gone with its file, the
interfaces given back.

`base/netd` takes for management no link that has a master (a bridge or
bond port) or that its caller reserves; `qsvsnetd` reserves the
interfaces the switch state names as uplinks, so the management
interface cannot be taken by a switch and a future uplink cannot be
taken for management at an IPL before the switch service has run.
`netd`'s status now names the device under a VLAN interface.

### The device (`vmm/devices/src/virtio/net`)

virtio-net over the existing transport, its receive and transmit queues
vhost-net's: the tap from the partition manager, `/dev/vhost-net`
opened before the privilege drop, the guest's memory given to it, the
queues wired by the transport's ioeventfds and the vectors' irqfds. The
control queue is the VMM's, bounded and fuzzed (`netctrl`). The ioctls
are in `vmm/devices/src/sys/vhost.rs`, the crate's one place for
`unsafe`. `qsvmm` takes each tap by its inherited descriptor
(`sys::inherited`) and opens no network device by name.

### The partition manager and profiles

`[[ethernet]]` is no longer a later phase's: `switch`, `vlan`,
`tagged`, `mac`, `required`, validated with every error. MAC addresses
are made from a prefix kept in `partitions.toml`. Activation asks the
switch service for each port and gives the taps to `qsvmm` after the
pool service's files; the end closes them; a partition manager that
starts has the switch service close the ports of partitions that do not
run. Request 25 lists the adapters; the partition list reports each
adapter's switch, MAC address and link.

### Service Tools

Option 3 is now `Work with network`: the management network as before,
the network hardware, the virtual switches, and the partitions'
adapters read only (`docs/SERVICETOOLS.md`).

### Messages and reference codes

`QVS1220` to `QVS1226` (adapters in a profile), `QVS2116` and `QVS2117`
(activation), `QVS4001` to `QVS4015` (switches), `QVS4101` to `QVS4107`
(uplinks and interfaces), `QVS4201` and `QVS4202` (ports). The switch
service's codes are the `C2008xxx` and `A2008xxx` block.

## Kernel additions

Added to `kernel/config/qsvs.config` and the build's required list:
`NET_SCH_INGRESS`, `NET_CLS_FLOWER` and `NET_ACT_GACT`, with
`NET_SCHED` and `NET_CLS_ACT` (which the baseline had on already) named
too: a partition port's ingress filter, for what the bridge cannot
refuse by itself with spanning tree off (a BPDU) and cannot tell apart
(a frame tagged with the port's own VLAN). `NETFILTER` stays off, and a
unit test of the build says so. The bridge, VLAN filtering, bonding,
tun and vhost-net were Phase 0's.

## Found while building

- **A dummy link says its largest MTU is 0**, meaning no limit of its
  driver's; read as a limit, every switch MTU was refused on it. 0 is
  read as "the driver says nothing" (`net/vswitchd/src/kernel.rs`).
- **Flower takes a VLAN key only with an Ethernet type key.** Without
  `TCA_FLOWER_KEY_ETH_TYPE` the kernel accepted the rule and ignored its
  VLAN ID; `tc` adds the type for you, a netlink client must.
- **With spanning tree off a bridge forwards BPDUs**, and BPDU guard
  acts only with it on: hence the filter's first rule.

## Not done

- `BLOCKED(phase6)`: the conformance harness has no network for uplinks yet, and suites 1 to 15 and 17 have not run.
- `BLOCKED(phase6)`: the vhost-net data path has not run: WSL gives an ordinary user no `/dev/vhost-net`, so its first run is under QEMU.
- `BLOCKED(phase6)`: Section 6 (`docs/QS400_GUEST.md`) is answered from QS/400's source at the pin, not yet from a run.
- `BLOCKED(phase6)`: the throughput baseline is not measured.
- `BLOCKED(phase6)`: golden screens of the new network panels are not written.
- `BLOCKED(phase6)`: a VLAN of a running partition's port is not added to or taken from it live; a tagged VLAN added to an adapter applies at its next activation.
- What Phase 5 left undone and this did not touch is still as `docs/PHASE5.md`, `docs/PARTD.md`, `docs/PROFILE.md` and `docs/RESOURCES.md` list it.

## For a real machine (NEEDS-CODY)

- [ ] NEEDS-CODY: an uplink on the Dell machine to a switch port carrying tagged VLANs.
- [ ] NEEDS-CODY: an LACP bond against a real switch.
- [ ] NEEDS-CODY: a 5250 session to a partition from a workstation.
- Phase 5's list for a real machine (`docs/PHASE5.md`) still stands.
