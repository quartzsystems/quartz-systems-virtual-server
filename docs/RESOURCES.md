# Resources: processors and memory

What a host has for its partitions, how it is counted, what an
activation gets of it, and which of the kernel's controls each part
maps to. The arithmetic is the crate `part/resource`; the partition
manager (`docs/PARTD.md`) carries it out.

## What a processor is

**One host core.** With SMT a core has two threads, and a partition's
processor is both of them: the guest sees them as sibling threads of
one core, and two partitions never run on the two threads of one core
at the same time. Without SMT a processor is the core's one thread.

The host's cores are numbered in the order of their first thread, so
core 0 has `cpu0`. The inventory gives each thread's socket and core
(`/sys/devices/system/cpu/cpuN/topology`).

**Processing units** are cores' worth of time: 1.00 unit is one whole
core, which is its threads' worth of processor time.

## Installed, configurable, available

| | Processors | Memory |
|---|---|---|
| Installed | every core of the machine | the memory the kernel has (`MemTotal`) |
| Configurable | installed less the cores reserved for the host | installed less the memory reserved for the host, in whole memory blocks; or less, when the kernel gave fewer huge pages than that at the IPL |
| Available | configurable less what running partitions hold: a core for each dedicated processor, and each shared partition's units | configurable less what running partitions hold |

The same three are kept for each shared processor pool, in units: a
pool's maximum, what its running partitions are entitled to, and what
is left.

The **host reserve** is the difference between installed and
configurable. It is a setting of the host, in `partitions.toml`:

```toml
schema_version = 1
max_partitions = 64
memory_block_mib = 16
reserved_cores = 1
reserved_memory_mib = 2048

[[pool]]
name = "BATCH"
cores = [6, 7]
maximum_units = 1.5
```

| Key | Value | Left out |
|---|---|---|
| `max_partitions` | the most partitions the host may have defined: 1 to 999 | 64 |
| `memory_block_mib` | the memory block size: a power of two, 2 to 1024. A profile's memory is in multiples of it | 16 |
| `reserved_cores` | cores kept back for the host: the lowest-numbered ones. The host's services run on them and nowhere else | 1 |
| `reserved_memory_mib` | memory kept back for the host: 512 or more (a file with less, which the partition manager would not have written, is read as 512) | the larger of 2048 and 5% of the installed memory |
| `[[pool]]` | a shared processor pool: its `name`, its `cores`, and `maximum_units`, the most its partitions use together (its cores' worth when left out) | none: the default pool only |

The limit and the block size apply at once to what is checked from
then on. The reserve applies at the host's next IPL: memory is set
aside once, early, and cores a running partition holds are not taken
from it.

## Dedicated and shared

**Dedicated processors** are whole cores that no other partition and no
host service uses. Each of the partition's virtual CPUs is pinned to
one thread of its cores, one to one. They come out of the default pool,
from its highest-numbered cores, and go back to it when the partition
ends. A core the default pool's running partitions need (for their
units, or to have a core for each virtual processor) is not given.

**Shared processors.** A partition has virtual processors and
processing units. A virtual processor is one core's worth of virtual
CPUs (two with SMT), floating over the cores of its pool; the units
are its entitlement to those cores' time. A partition cannot have more
virtual processors than its pool has cores.

- **Capped**: the partition cannot use more than its units.
- **Uncapped**: it can use up to its virtual processors' worth when
  the pool has spare time, shared out by weight.

**Shared processor pools.** A named pool has a set of cores and an
optional maximum in units. The default pool, `DEFAULT`, has every core
that is not dedicated to a partition, reserved for the host or in a
named pool; it cannot be created, changed or deleted. A named pool is
changed or deleted while no partition runs in it, and may not take
cores a running partition holds or the default pool's partitions need.

## Memory

Partition memory is preallocated 2 MB huge pages. There is no
overcommit and no ballooning.

PID 1 reserves the configurable memory as 2 MB pages at the IPL, right
after the state is loaded and before the services after the storage
service are started (`/sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages`),
and posts `C2001072` with the pages and the megabytes. When the kernel
gives fewer pages than asked it posts `A2001072` with both numbers, and
the configurable memory is what it gave. The partition manager counts
in megabytes that are whole pages; `qsvmm` maps the guest's memory from
those pages and reserves them as it maps, so a partition whose pages
are not there fails at its activation (`A2005065`), not later.

## What an activation gets

For each of processors and memory: the **desired** amount when it is
available; else the most there is that is not below the **minimum**;
else the activation fails with the message that names the resource,
what was asked (the minimum) and what is available
(`QVS2101` to `QVS2104`). The **maximum** is what a partition may ever
be given; nothing changes on a running partition in this phase, so it
only bounds the desired value.

For shared processors: the virtual processors are the desired number
or as many as the pool has cores; the units are the desired units or
what the pool has left, and not more than 1.00 for each virtual
processor; and when the units are fewer than 0.05 for each virtual
processor, the virtual processors come down to what the units allow.

What was given is the partition's **current resources**, shown with it
until it ends. A guest's own restart keeps them: the partition is
activated again at once with the same cores and memory.

## The kernel's controls

Everything is in one cgroup v2 tree.

```
/sys/fs/cgroup/qsvs/
  host/                 PID 1 and the services
  dedicated/
    p<ID>/              a dedicated partition
  pools/
    <POOL>/             a shared pool
      p<ID>/            a shared partition
```

| Group | Control | Value |
|---|---|---|
| `host` | `cpuset.cpus` | the threads of the reserved cores. PID 1 joins the group before it starts any service, so every service is born in it |
| `dedicated/p<ID>` | `cpuset.cpus` | the threads of the partition's cores; no other group has them |
| `pools/<POOL>` | `cpuset.cpus` | the threads of the pool's cores. The default pool's shrink and grow as dedicated partitions come and go |
| `pools/<POOL>` | `cpu.max` | the pool's maximum, as a quota; `max` for the default pool |
| `pools/<POOL>/p<ID>` | `cpu.max` | capped: the units. Uncapped: the virtual processors' worth |
| `pools/<POOL>/p<ID>` | `cpu.weight` | the entitlement scaled by the weight (below) |
| every `p<ID>` | `hugetlb.2MB.max` | the partition's memory: the VMM maps no more huge pages than that |
| every `p<ID>` | `memory.max` | 512 MiB: what a `qsvmm` may use for itself, beside the guest's memory |
| every `p<ID>` | `pids.max` | 64 and one for each virtual CPU |

A quota is in thread time: `cpu.max` of `U` units on a host with `T`
threads a core is `U x T x 100000` microseconds in each period of
100000. So 0.50 units is half a thread's time where a core has one
thread, and one thread's time where it has two: half the core either
way.

**Core scheduling.** Each `qsvmm` gives itself a core scheduling cookie
before it starts any thread (`prctl(PR_SCHED_CORE)`), so all its
threads share one and no other process has it. The kernel then runs
threads of one cookie only on the sibling threads of a core at a time:
while a partition's virtual CPU runs on one thread of a core, the
other thread runs that partition or nothing. On a host without SMT
there is nothing to keep apart and `qsvmm` says so.

### Weight

The kernel shares a pool's contended time among its groups in
proportion to `cpu.weight`. A shared partition's is

```
cpu.weight = units x 100 x weight / 128        (at least 1, at most 10000)
```

so 1.00 unit at the default weight of 128 is the kernel's own default,
100, and a capped partition competes as one of weight 128.

**Equal weights give every partition at least its entitlement.** With
every partition of a pool at one weight, the shares are in proportion
to the units; the units of a pool's running partitions never add up to
more than the pool has; so under full contention each gets at least
its units, and the time nobody is entitled to is split in the same
proportion.

**Unequal weights do not.** The kernel has no control that guarantees
a group a minimum of processor time: the weight is all there is. Two
uncapped partitions of 1.00 unit each on a pool of two cores, one of
weight 255 and one of weight 1, have the kernel weights 199 and 1
(0.78 rounded down to the least there is): when both want all they
can have, the first gets 199 parts in 200 of what they contend for and
the second one part, far below the 1.00 unit it is entitled to. A low
weight beside a high one gives up entitlement under contention, not
only spare time. To keep entitlements whole, give the partitions of a
pool one weight, or cap the ones that should not take more.

**Weight 0** means capped at entitlement: the partition's quota is its
units, and it competes as a capped partition does.

## Limits on a VMM

A `qsvmm` that misbehaves cannot starve the host: its group has the
memory and process limits above, its processors are its partition's
cores or its pool's, and the host's services have cores of their own.
The system calls it may make are not filtered yet (Phase 11).

## The kernel symbols

Added to `kernel/config/qsvs.config` and to the build's required list
with this phase: `SCHED_CORE`, `SCHED_SMT`, `CGROUP_HUGETLB`. Already
there since Phase 0: `CPUSETS`, `CGROUP_SCHED`, `FAIR_GROUP_SCHED`,
`CFS_BANDWIDTH`, `MEMCG`, `CGROUP_PIDS`, `HUGETLBFS`, `HUGETLB_PAGE`.

## Not here yet

- `BLOCKED(phase4)`: the kernel is built for 64 threads (`NR_CPUS=64`, the pinned configuration's); a host with more shows 64.
- `BLOCKED(phase4)`: cores are given without regard to sockets or to memory's place: a partition's cores and its huge pages may be on different NUMA nodes.
- `BLOCKED(phase4)`: only 2 MB pages; 1 GiB pages are not used.
- Any change to a running partition's resources: not in this phase.
