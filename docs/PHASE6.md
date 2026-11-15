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

## Running

```
cargo run -p build -- all
cargo run -p build -- all --test-image
cargo run -p build -- test --suite net-uplink
cargo run -p build -- fuzz --target netctrl
NL_LIVE=1 unshare -rn cargo test -p nl --test live
NL_LIVE=1 unshare -rn cargo test -p vswitchd --test live
```

The partition suites' cheap guest is the release image, so `all` comes
first. `net-qs400` takes `QSVS_QS400_ISO`, as `pool-install` does.

## The suites

The harness gives the host further NICs, uplink candidates at fixed PCI
places (`P0000-B00-D18-F0-E0` and on), on QEMU's `dgram` backend: their
far side is the harness's station (`tools/conformance/src/qemu/wire.rs`),
which sees every frame with its tags, answers DHCP, ARP and echo, and
pings. `net-qs400`'s uplink is on QEMU's user network instead, with a
port of the harness's host forwarded to QS/400's telnet port. Every
host has its own management NIC on QEMU's user network and its own
seed volume, first among its disks (below, "Found while testing"). The
switches are made as the host keeps them: `vswitch.toml` written
through the test hooks' channel and the switch service started again;
Service Tools makes and changes them in suites 1, 7 and 10. What the
kernel has is read through the channel: `/sys/class/net`, each tap's
counters, the bridge's `brforward`, the ports' record.

| # | Suite | What it shows (working tree, `8b29fd9`) |
|---|---|---|
| 1 | `net-switch-create` | Service Tools option 3, 3, F6 made PRODNET with VLANs 10 and 20; there after a host restart and after a power pull; six golden screens |
| 2 | `net-internal` | on an internal switch, A and B on VLAN 10 sent 19 frames each and were sent 19: each was sent no more than the other sent; C on VLAN 20 sent 11 and was sent 0 |
| 3 | `net-uplink` | a partition got its address by DHCP from the station through the uplink, its frames tagged VLAN 10, 10 of 10 echoes; with VLAN 10 native, 10 of 10 untagged and no frame tagged |
| 4 | `net-tagged` | A, at a static address on tagged VLAN 20, asked the station for the time with its tag and answered 10 of 10 echoes on it; B's DHCP on VLAN 30, not listed, never left its port, and 20 broadcasts on VLAN 30 from outside reached it 0 times |
| 5 | `net-may-not` | access adapters on VLAN 10: A's DHCP tagged VLAN 20 and B's tagged with its own VLAN 10 dropped at their ports (13 frames each sent, no IPv4 of either on the uplink). The other two checks are below |
| 6 | `net-host-unreachable` | A on an internal switch asked the host's management address, B on another switch asked A: no address or route of the host's on any bridge or tap, IPv6 off on each, forwarding off, neither partition sent a single frame back (nothing of the host's answered), none of A's left by the other switch |
| 7 | `net-management` | the management interface refused as an uplink from Service Tools (`QVS4102`, golden screen); another taken; the management line `10.0.2.15/24 on eth0, gateway 10.0.2.2` the same after |
| 8 | `net-uplink-loss` | a two-member bond: one member's link down, `A2008025` naming `P0000-B00-D18-F0-E0` and 5 of 5 echoes through the other; both down, `A2008020` naming `...-D19-...`; one back, `C2008020` naming it and 5 of 5 echoes again |
| 9 | `net-qs400` | QS/400 installed in 59 s from a copy of its ISO whose response file has `ip` and `gateway`; its telnet port answered from the harness's side, and a 5250 session negotiated as IBM-3179-2 read the sign-on display |
| 10 | `net-live` | MTU 9000: 10 of 10 echoes of 8972 bytes end to end; VLAN 30 added from Service Tools under traffic, 10 of 10 echoes; the uplink moved to another interface, 10 of 10 there and 0 on the first; an interface of 1500 refused for it (`QVS4105`, golden screen) |
| 11 | `net-service-kill` | the switch service killed three times under 120 echoes: 0 lost, its port found again each time; a partition activated after got its port and an address |
| 12 | `net-vmm-kill` | a `qsvmm` killed: its tap and port gone within seconds, the other's untouched; the partition manager killed and started again: the other's port the only one left, its counters going on |
| 13 | `net-not-required` | switch GONE taken from the host's state under a profile that names it: the required adapter refused with `QVS2117 ... slot 5 has no port: ... switch GONE, which does not exist.`; not required, the partition runs with its adapter's link down and no port |
| 14 | `net-counters` | the counters of suite 2's traffic (A and B each sent 19 and were sent 19, C 11 and 0) and the bridge's table: each partition's address static on its port |
| 15 | `net-throughput` | the baseline below |
| 16 | the `netctrl` fuzz target | 233,046 runs in 60 s, no finding |
| 17 | every earlier suite | below, "Acceptance" |

**Suite 5's other two checks.** A frame from another source address,
and a guest that sets its own address through the control queue, need
a guest that does either, and neither the cheap guest nor QS/400 does
by itself (QS/400's `ADPTADR` needs a line description made at its
console). They are shown where they can be made to happen: `net/nl`'s
live test writes a frame of another source into a partition's tap and
the locked port drops it, and nobody else's traffic changes; the
device's unit tests set an address through the control queue and see
it recorded and answered and not passed on, and the port still passes
only the profile's address (the same lock). `BLOCKED(phase6)` below
says what is left for a QEMU run.

## The throughput baseline

From the harness's station through the uplink to a partition on
PRODNET and back: ICMP echoes in rounds of 50, each waited out, three
levels deep (WSL, QEMU, the partition), on 2026-11-15:

| Echo size | Through | Rate | Both ways |
|---|---|---|---|
| 1472 bytes (full frames) | 2000 of 2000 in 0.35 s | 5,742 replies/s | 139.1 Mb/s |
| 56 bytes | 2000 of 2000 in 0.19 s | 10,430 replies/s | 16.4 Mb/s |

Sent all at once, a third of the 2000 full frames were lost in the
queues, and a burst of ten 9000-byte frames lost one or two; one at a
time none were. No threshold: it is a baseline. Between two partitions
on one switch there is nothing to measure yet: the cheap guest makes
no traffic on demand, and QS/400 has no load generator (`BLOCKED`
below).

## Found while testing

- **A host took a guest's seed.** A suite's host disk is not
  installed, and at its IPL it applies the first seed volume it finds:
  a guest's, attached beside it as a further disk. One with a VLAN gave
  the host a VLAN interface on its only NIC, the uplink candidate,
  which took every frame of that VLAN before the bridge saw it. The
  host had also taken that NIC for management, being its only one.
  Every network suite's host now has its own management NIC and its
  own seed volume, first among its disks. Nothing in the product: a
  real host is installed, and then reads no seed.
- **The switch service took the management interface from state.** A
  `vswitch.toml` naming the management interface as an uplink was
  built: the check was the request's alone. It is now the build's too
  (`ca2f6c7`, with a unit test).
- **A bridge counts a frame before it drops it for its VLAN.** The
  bridge device's own receive counter goes up for frames it then drops
  because the bridge itself is in no VLAN, so it says nothing of what
  reached the host. `net-host-unreachable` reads the partitions' ports
  instead: a partition with no other neighbour that is sent nothing
  had no answer from the host.
- **`QVS2117` quoted a message with its full stop**, and ended in two.
- **Service Tools on the serial port and a partition's output.** The
  test hooks relay partitions' serial lines to the host's serial port,
  over the console's screen. `net-live` reads its screens before the
  partition runs and makes its live changes without reading them,
  checking the state file and the traffic.

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

- `BLOCKED(phase6)`: suite 5's frame of another source and an address set through the control queue are shown in `net/nl`'s live test and the device's unit tests, not under QEMU: no guest of the suites does either by itself.
- `BLOCKED(phase6)`: no throughput figure between two partitions on one switch: the cheap guest makes no traffic on demand and QS/400 has no load generator; the uplink's is above.
- `BLOCKED(phase6)`: a VLAN of a running partition's port is not added to or taken from it live; a tagged VLAN added to an adapter applies at its next activation.
- What Phase 5 left undone and this did not touch is still as `docs/PHASE5.md`, `docs/PARTD.md`, `docs/PROFILE.md` and `docs/RESOURCES.md` list it.

## For a real machine (NEEDS-CODY)

- [ ] NEEDS-CODY: an uplink on the Dell machine to a switch port carrying tagged VLANs.
- [ ] NEEDS-CODY: an LACP bond against a real switch.
- [ ] NEEDS-CODY: a 5250 session to a partition from a workstation.
- Phase 5's list for a real machine (`docs/PHASE5.md`) still stands.
