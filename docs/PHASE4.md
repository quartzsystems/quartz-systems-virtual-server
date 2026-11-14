# Phase 4: the partition manager, profiles, processors and memory

The record of Phase 4. Phases 0 to 3 are `docs/PHASE0.md` to
`docs/PHASE3.md`; what they say still holds, except where this says
what changed.

Phase 4 moves the pin of QS/400 and then adds the partition manager,
`partd`: partition definitions, profiles with the fields QS/MC's Create
partition wizard collects, activation with resource accounting,
processor placement, huge-page memory, a state machine, and auto
start. There are still no storage pools, no virtual Ethernet, no
service channel and no QSI: `partd` is driven over a local socket.

## Step 0: the pin of QS/400

Every `rev =` in `Cargo.toml` is now

```
e035bea3998bd5732750f134eafcd89ba45d638f
```

fifteen of them, all at once (`docs/PHASE0.md` has the pin's history).
The kernel pin did not change: `kernel/VERSION` and `kernel/SHA256`
are QS/400's at that commit as they were at the last.

The commits, in order: `e1ae2de` moves the pin in `Cargo.toml`;
`355a801` has the lock file and the IPL loader's change; `00a83dc` the
suite. Three small ones follow that only the suite needed: `32f0546`
and `33af96b` correct what the suite expected of a disk's first IPL
and of a posted line, and `7ec7d5a` puts back two files that `32f0546`
removed by mistake (a deletion staged for the later work went into it;
history is not rewritten here, so it was undone by a commit). `e1ae2de`
does not build with `--locked` by itself: the lock file is in the
commit after it.

### What changed in QS/VS

- **The side of an IPL** (`vmm/ipl/src/lic.rs`). A profile's `recorded`
  source is what the partition itself asked for: `licpart::IplSource`,
  the TLV QS/400 writes when it has no firmware. `IplSource::take` is
  called once an activation: it gives the side asked for one IPL when
  there is one, and clears it in the record, which is then written;
  else the default side; a record without the TLV means side A. An
  activation that names side A or B loads that side and leaves the
  record's request alone. `C2005030` says `side B once` when the side
  is the once-only one.
- **Nothing of QS/400 is restated.** The console mode is
  `licpart::ConsoleMode`; the sides' paths on the ESP are
  `diskimg::SLOT_A_UKI` and `SLOT_B_UKI`. Open questions 23 and 24 are
  closed.

### What the new pin broke

Nothing in QS/VS's own build. Between the two pins QS/400 has 129
commits and a great deal of new code (an NFS server and client among
it), but of the fifteen crates QS/VS takes only `licpart` (two new
modules) and `diskimg` (two constants) differ, both by additions; the
lock file changed in the QS/400 source alone, and one line for the
loader's new dependency on `diskimg`. The workspace compiled, linted
and passed its unit tests on the new pin without a change beyond the
loader's.

What changed is the guest. QS/400 at `e035bea` in a partition:

- keeps `IPLSRC` and a LIC PTF's side in its LIC record
  (`slic: next IPL from slot B`), where at the old pin it said `IPL
  source not set: not booted through UEFI`;
- says `Hypervisor: QS/VS` and passes its firmware check as a
  partition of QS/VS, by the ACPI identifiers QS/VS has had since
  Phase 3;
- takes longer over the first IPL of a new disk: about 36 s from the
  activation to `C6001090` under the harness, where the image of the
  old pin took about 23 s (`docs/PHASE3.md`). Its storage service says
  what it does there: `2422 work management object(s) created; 54 user
  profile(s) created, 2503 object(s) re-owned`. Why it is longer was
  not looked into; nothing in a suite depends on it.

`docs/QS400_GUEST.md` has the guest's own lines.

### The earlier suites on the new pin

Taken in a fresh clone at `33af96b` (Step 0's last commit), both
images built from empty targets, `QSVS_BUILD_ID=phase4-step0`. `build
test` ran to its nineteenth step and stopped at `mirror-power-pull`;
the suites after it were then run one at a time. Every suite of
Phases 0 to 3 passed there, three of them not at the first run:

| Suites | Result |
|---|---|
| `IPL slot A`, `IPL slot B` (release image) | 12.1 s, 2.6 s |
| `status`, `dhcp`, `static-vlan`, `time`, `inventory`, `supervision`, `power-pull` (50 cuts), `slot-confirm`, `shutdown`, `sockets` | 2.5, 2.5, 10.3, 4.2, 6.1, 30.9, 196.1, 14.2, 11.0, 2.5 s |
| `install-serial`, `install-display`, `install-unattended`, `mirror-degraded`, `mirror-replace`, `install-power-pull` | 24.5, 26.4, 21.6, 72.8, 54.9, 276.7 s |
| `mirror-power-pull` | failed three times, then passed twice (320.8 s and 360.7 s): below |
| `storage-kill`, `tools-network`, `tools-sign-on`, `media-refused`, `answers-invalid`, `installed-base`, `mirror-start`, `mirror-split` | 22.5, 56.9, 24.5, 5.1, 9.1, 419.7, 19.4, 104.9 s |
| `partition-boot`, `partition-scsi`, `partition-ipl-a`, `partition-attributes`, `partition-kill`, `partition-power-pull` (20 cuts), `partition-end`, `partition-damaged`, `partition-privilege` | 24.8, 15.7, 44.0, 97.0, 109.3, 705.6, 81.8, 7.6, 12.1 s |
| `partition-ipl-b`, `partition-recorded` | each failed once, then passed (48.4 s, 222.8 s): below |
| the eight fuzz targets, 60 s each | no finding |

What did not pass at the first run, and what was found of it:

- **`mirror-power-pull`: a host IPL slower than the harness's 60 s.**
  Three runs stopped after 19, 12 and 12 cuts with `REF C2001080
  Services started` not seen in time; in the one looked at, the last
  line was `C2003010 Mirrored load source assembled` and the mount of
  `/data` had not been reported when the 60 s ended. With
  `QSVS_BOOT_TIMEOUT=180` it passed twice running, 20 cuts each, in
  the time Phase 2 measured, so the mount ends: it is late, not
  stuck. Between the pins none of `protect`, `ublk`, `blockdev`,
  `abrec` or `xfsfmt` changed, and Step 0 changed nothing of the host's
  own; the build host was running another QEMU test run all the
  while. How long the slow IPL took was not measured, and that the
  other load was the cause is the likely reading, not a shown one.
- **`partition-ipl-b` and `partition-recorded`: the guest's kernel
  failed, once each.** On its second processor, in the timer
  interrupt, 7 s and 15 s into QS/400's IPL: `BUG: kernel NULL pointer
  dereference` at `perf_event_task_tick+0x2` in one and at address 0
  from `__hrtimer_run_queues` in the other, an instruction pointer no
  call leads to. Both passed when run again, and so did every other
  QS/400 IPL of the run (twenty-one of them in
  `partition-power-pull`). QS/400's kernel is the same at both pins
  but for `CONFIG_FHANDLE`. Twelve IPLs of the same image under
  `qsvmm` on the build host itself, one level of virtual machine less,
  all completed. No log of Phase 3 has such a failure. It was seen
  again in Phase 4's runs, below ("The guest's kernel, again").

## Answers to Phase 3's open questions

| # | Question | What was done |
|---|---|---|
| 23, 24 | the side of the next IPL; what QS/VS restates | Step 0, above |
| 25 | a partition's identity | SMBIOS tables for each partition (`vmm/arch/src/boot/smbios.rs`): manufacturer `Quartz Systems`, product `QS/VS partition`, and a serial number made once, when the partition is created, from the host's serial number and the partition's ID (`part/model/src/definition.rs`, `serial_for`), in the definition and never changed. QS/400 at `e035bea` reads it as it is: `docs/QS400_GUEST.md` says what it reads |
| 26 | which disks the host takes | the disk it IPLed from and the members that disk's label names (`base/loadsource/src/own.rs`). The harness's wrapper around guest images is gone |
| 27 | whose the keylock is | the host's: in the definition, written to the LIC record at every activation |
| 28 | one user for every partition | each `qsvmm` is user and group 23000 plus the partition's ID; its log and its socket are in directories of that user alone |
| 6, 17, 19, 22 | CI, identity key, resync bitmap, users' owner | unchanged |

### 26, in a little more detail

A disk with QS/VS's three partitions was a load-source disk for that
alone. Now `loadsource::disks::load_sources` first finds every such
disk (`candidates`), then keeps the host's own (`own::own`):

1. **The disk of the IPL.** `BootCurrent` names the firmware boot
   entry that was started; its device path has the PCI functions down
   to the disk's controller, the GUID of the ESP, or both. Each thing
   the entry says narrows the candidates when a disk fits it and is
   passed over when none does. The entries QS/VS writes itself carry
   the ESP's GUID; the ones firmware makes for a disk carry its PCI
   functions. With no firmware to ask (a host that is itself a
   partition) or an entry that fits nothing, it is the first candidate
   in name order, as before.
2. **Its members.** The other candidates whose data partition that
   disk's label names as a member, unless such a disk holds a label of
   another set.

A candidate's own label is read without opening the disk for writing:
it may be a partition's.

## What exists

`docs/PARTD.md` has the partition manager, `docs/RESOURCES.md` the
accounts and the kernel's controls, `docs/PROFILE.md` definitions and
profiles, `docs/MESSAGES.md` the messages, `docs/VMM.md` what `qsvmm`
is given.

### The model (`part/model`)

A partition's `Definition` and its `Profile`s, as state files of
`base/state` and as values on the socket. Validation gives every error
at once, each a `QVS` message. A serial number is five characters of
the host's and two of the partition's ID. The sections later phases
bring (virtual Ethernet, optical, tape, physical I/O) are in the schema
and checked for shape; `validate::not_yet` names the phase that brings
the first one a profile uses.

### The messages (`lic/qvsmsg`)

A catalogue: an identifier, first-level text and second-level text for
each, with `&1` to `&9` for its values. `docs/MESSAGES.md` is written
from it and a unit test fails when they differ.

### Resources (`part/resource`)

Arithmetic only: the topology from the inventory's threads; installed,
configurable and available for processors, memory and each pool; what
an activation gets; the settings and pools of `partitions.toml`; and
the cgroup tree as a plan of directories and values, which the
partition manager carries out.

### The partition manager (`part/partd`, `lic/qsvspartd`)

- `proto`: the socket's requests, answers and events, with a client.
- `store`: the partitions in state.
- `manager`: the partitions as they are. Every request, every line a
  VMM reports and every end of a VMM is a method on it, and what it
  wants of the machine it asks of a `System`: its unit tests run all
  of it against a machine that only writes down what it was asked.
- `serve`: the machine itself: the cgroup file system, `qsvmm` started
  in its control group and a session of its own, its report followed
  and its end taken, the socket, auto start, and the partitions that
  already run taken up when the service starts.

It is the fourth service of PID 1's table, between the network service
and Service Tools.

### PID 1

- **The host's control group.** PID 1 joins `qsvs/host` right after
  cgroup2 is mounted, so every service is born in it; the partition
  manager keeps it to the reserved cores.
- **The memory step.** After the state is loaded: the huge pages the
  settings leave for partitions are asked of the kernel, and
  `C2001072`, or `A2001072` when it gave fewer, is posted.
- **Shutdown.** The partitions are ended before any service is
  stopped: another thread asks the partition manager and waits; the
  loop only looks whether it is done. `D2001010` when there were any;
  `A2006090` when they did not all end in time, and the shutdown goes
  on.

### The VMM

- The configuration comes on standard input from the partition
  manager (`vmm/qsvmm/src/config.rs`); the profile file is gone.
- Guest memory is 2 MB huge pages, reserved as it is mapped.
- A processor is a core: CPUID gives a core of two virtual CPUs its
  sibling threads; dedicated virtual CPUs are pinned one to one.
- The process gives itself a core scheduling cookie before any thread.
- SMBIOS tables (answer 25), with a range of their own in the memory
  map.
- Several virtual SCSI adapters, a virtio-scsi function each.
- The process's user is the partition's; its log and socket
  directories are named by the partition's ID.
- Every reference code it reports names the partition, and its report
  ends with `END <exit code>`.
- The IPL type (attended or not) is written to the LIC record with
  the keylock and the console mode.

### Service Tools

Option 1 counts the partitions, and F11 on it lists them: ID, name,
state, reference code, current processors and memory. No action is
offered (`docs/SERVICETOOLS.md`).

### The test image

The hook that started one partition from a volume is gone. The
channel's `partd` command carries a request to the partition manager's
socket, encoded as its own client encodes it, and brings back the
answer; the `partition` service relays what each activated partition
says on its serial port, and the lines of its VMM's report that are no
reference code, to the host's serial port after the partition's name.
Four more channel commands read the kernel's accounts for the suites:
`threads`, `tasks`, `curr` and `as`. PID 1 has one more hook, `hold-memory`:
at the IPL it holds the host's reserve of memory, written to, while
it asks for the huge pages, so the kernel gives fewer than asked and
`partition-huge-pages` sees `A2001072` on a host that has the memory.

A release image has no such path: nothing in it sends the partition
manager a request to act. Its userland has `qsvspartd` and `qsvmm` and
no `qsvshook`, which the userland step checks as before.

## Kernel additions

Added to `kernel/config/qsvs.config` and to the build's required list
(`tools/build/src/product/kernel.rs`): `SCHED_CORE`, `SCHED_SMT`,
`CGROUP_HUGETLB`. `SCHED_CORE` was off in the kernel as built before;
the other two were already on by the pinned configuration's defaults
and are now asked for by name. Everything else this phase uses was
there since Phase 0 (`docs/RESOURCES.md` lists it).

The suites read `/sys/kernel/debug/sched/debug` and
`/proc/<pid>/task/<tid>/stat`: debugfs and the scheduler's statistics
are in the pinned configuration's defaults (`DEBUG_FS`, `SCHED_INFO`,
`SCHEDSTATS`), in the release kernel as in the test image's. They are
not in the required list: the product does not use them.

## The QS/400 guest image

```
git clone https://github.com/quartzsystems/quartz-systems-400 qs400-pin
cd qs400-pin && git checkout e035bea3998bd5732750f134eafcd89ba45d638f
QS400_TARGET_DIR=$HOME/qs400-pin-target cargo run -p build -- all
export QSVS_QS400_IMAGE=$HOME/qs400-pin-target/images/qs400-V1R1M0-<hash>.img
```

At `e035bea` the image is `qs400-V1R1M0-150983bae8c2.img`. Building it
took 28 minutes on the build host. Nothing of it is changed: each
suite works on a copy.

## Running

```
cargo run -p build -- all --test-image
cargo run -p build -- test --suite partition-three
cargo run -p build -- test
QVS_MESSAGES=update cargo test -p qvsmsg
```

By hand, on any Linux host with KVM, as an ordinary user who may open
`/dev/kvm` (the privilege drop is then skipped, and `qsvmm` says so):

```
cargo build -p qsvmm
target/debug/qsvmm --config my.toml --data /tmp/p/data --run /tmp/p/run
```

with `huge_pages = false` in the configuration (`docs/VMM.md`). The
guest's serial output is in `/tmp/p/data/log/partitions/<ID>/serial.log`
and its console is the socket `/tmp/p/run/partitions/<ID>/console.sock`.

## The suites

`build test` runs every earlier suite, then these on the test image,
then the fuzz targets. As in Phase 3, QEMU stands in for the machine,
the QS/VS test image is the host in it, and a partition is a guest of
that host. Three guests are used: QS/400 (the image above); QS/VS's
own release image, the cheap guest; and a guest that does nothing but
run, `ipl::load::spinner`, whose boot processor jumps to itself for
ever: one busy processor from its first instruction, 128 MB, started
in a moment, so several run in a host under the harness.

The host of a partition suite has 5120 MB and six processors: one
processor and 2048 MB are the host's own, by the defaults.

| Handoff | Suite | What it asserts |
|---|---|---|
| 1 | `partition-recorded` | Step 0's: with nothing asked, side A; `PWRDWNSYS IPLSRC(B)` in the guest, then an activation with `recorded`: `C2005030` says `side B once`, side B is loaded, the guest names slot B; the IPL after it is side A again; asked again, an activation with side A loads A and leaves the request, which the next `recorded` activation takes |
| 2 | `partition-define` | three partitions with two profiles each, one changed in everything a definition may change; each definition and profile reads back the same after a host restart; a profile wrong in nine ways is refused with every message and nothing kept; a second partition of one name and a changed serial number are refused; a deleted partition's ID is given again with the serial number it had |
| 3 | `partition-three` | three partitions Running together, the accounts less by what they hold; a fourth whose minimum memory is not there refused with `QVS2104` naming what it asked and what is available; no control group, no record, no state of it left, the accounts as they were |
| 4 | `partition-minimum` | desired not available, minimum is: activated with the two cores and 256 MB there are, which the answer, the list and the VMM's `C2005060` all say |
| 5 | `partition-dedicated` | two dedicated partitions and a shared one, all busy: every thread of a dedicated VMM is allowed its partition's processors only and is seen there, each virtual CPU on one; no other process of the host but a kernel thread is allowed there; and what the scheduler says those processors run, looked at 40 times, is the partition's own thread, a kernel thread or nothing |
| 6 | `partition-capped` | one virtual processor capped at 0.50 units, a guest that only runs: its control group's processor time over 20 s is within the tolerance of half a core |
| 7 | `partition-uncapped` | a pool of one core, two uncapped partitions of equal weight entitled to 0.20 and 0.60, both only running: each gets at least its entitlement, and the spare 0.20 splits one to three |
| 8 | `partition-core-sched` | a host of three cores of two threads, a pool of one core, two shared partitions each with a busy virtual CPU: in every look at what the core's two threads run over 20 s the two partitions are never there at once, and together they use one thread's time, not two |
| 9 | `partition-huge-pages` | `C2001072` at the IPL and as many huge pages as the configurable memory; a partition of 256 MB has that much of 2 MB pages mapped and takes 128 pages; a reserve of 96 MB is refused with `QVS2201`; a host whose reserve is held by PID 1 while it asks for the pages (the hook `hold-memory`) posts `A2001072` and reports the lower configurable memory |
| 10 | `partition-auto-start` | three partitions flagged at places 20, 10 (after 2 s) and 5, the last with a backing store that is not there: after a host restart it fails alone with `A2006020` and `QVS2110`, the other two are activated in order and run, `C2006030` says 2 of 3; not done over when the partition manager is started again |
| 11 | `partition-end` | guest power off: Not activated, holding nothing; guest restart: activated again at once with what it held, Running; immediate shut down: `A2005140`, Not activated, and a second one refused; immediate restart: ended and activated again |
| 12 | `partition-killed`, `partition-kill` | of three running, one VMM killed: Error with `A2005150` and `QVS2109`, holding nothing, not started again by itself; the other two as before with the VMMs they had; activated again. With QS/400: killed during the IPL and while it ran, its disk IPLs after each and what it had acknowledged is there |
| 13 | `partition-manager-restart` | the partition manager killed with two partitions Running: started again, `C2006000` with `2 running`, both `C2006010`, Running with the VMMs and resources they had, their guests still running; one shut down and activated again under the new manager; then the host powers off: both ended first, the last activated first, `D2001010`, `D2001090` |
| 14 | `partition-identity` | two QS/400 partitions: each says its own `QSRLNBR`, the definition's, at two activations; `QSHWQUAL` names the machine `Quartz Systems QS/VS partition` and the hypervisor `QS/VS` |
| 15 | `partition-isolation`, `partition-privilege` | two VMMs are users 23001 and 23002 with no capability; each user may signal and read its own, and is refused the other's process and the other's serial log |
| 16 | `partition-own-disks` | a disk with QS/VS's three partitions and its identifiers, attached as it is: the host's load source is `vda` alone; after an IPL and a power off of the host the disk's table, ESP and record partition are byte for byte as they were; it then IPLs in a partition |
| 17 | `partition-tools` | Service Tools option 1 counts the partitions and F11 lists them; two golden screens; the list follows a shut down, and no key on it acts on a partition |
| 18 | | every earlier suite: `build test` runs them first. Phase 3's ten are the same suites, driven through the partition manager now |

Suites 1, 11, 14 and the QS/400 ones of Phase 3 need
`QSVS_QS400_IMAGE`.

The fuzzer has a ninth target, `report`: what a VMM writes on its
standard error, where text of the guest's can end up, read as the
partition manager reads it.

## Found while testing

- **A disk with no LIC partition stopped IPLing in a partition.** Since
  answer 27 every activation writes the keylock, and the IPL type and
  console mode with it, so the IPL loader took every activation as
  needing the LIC record and refused a load source without one
  (`A2005040`, exit 30). A QS/VS disk is such a disk, and
  `partition-own-disks` IPLs one. The loader now needs the record only
  for the recorded side; a disk without a LIC partition loads from a
  side that is named, posts `C2005037`, and the settings are written
  nowhere (`vmm/ipl/src/lib.rs`, `docs/VMM.md`, `docs/REFCODES.md`).
- **A host could be told to keep nearly nothing for itself.** The
  first form of `partition-huge-pages` set a reserve of 96 MB to make
  the kernel fall short; the kernel gave every page asked, and the host
  came up with under 100 MB of its own. The reserve now has a floor of
  512 MB (`QVS2201` below it), and the suite makes the kernel fall
  short with a hook of the test image instead (`hold-memory`).
- **The guest's kernel, again.** In the Phase 4 runs four more QS/400
  IPLs failed in the guest's kernel or its processes, where the rest
  (two runs of `partition-power-pull` alone are forty-two) completed: a page fault in the power-off system call
  (`partition-end`, which so restarted where it should have powered
  off), a NULL pointer in `redirected_tty_write` 4 s into an IPL
  (`partition-recorded`), the storage service ending at an IPL
  (`partition-identity`, `slic: starting the storage service: channel
  closed`), and a power down that stopped after `D1001005`. Each passed
  when run again. Every kernel fault has the kernel already tainted
  `W`, by a warning the `quiet` console does not print. What says the
  most: in the `redirected_tty_write` fault the instruction bytes the
  kernel dumped at the faulting address are data (`00 00 00 00 c4 44
  01 00 ...`), and the second dump, a moment later, has the function's
  right first instruction there (`f3 0f 1e fa`). The processor so ran
  bytes the guest's memory does not hold. A write into the guest's
  memory by `qsvmm` would have left them wrong for the second dump
  too; a mistaken translation of guest addresses would not. Here a
  partition runs four levels deep: Hyper-V, the WSL kernel's KVM, the
  QEMU host's KVM, the partition. The likely reading is a fault of
  nested virtualization below `qsvmm`; it is not shown. The VMM writes
  guest memory in its device threads alone, and they were looked at
  for this: see the next item. **NEEDS-CODY**: the partition suites on
  a Linux machine that is no virtual machine (two levels: the host,
  the partition), which `docs/PHASE3.md` already asks for, would say
  whether `qsvmm` has any part in it.
- **virtio-scsi answered an abort before the request had ended.** The
  control queue answered every task management function at once, on
  its own thread, while the request named might still be in progress
  on the request queue's thread, or not yet taken. A guest that
  aborts a request takes the answer to mean the buffers are free, and
  a read ended after it would write into memory that is no longer the
  request's. A task management function is now answered only once
  every request made before it has ended (`vmm/devices/src/virtio/scsi`,
  `docs/VMM.md`), and `qsvmm` reports how many there were. It is not
  the cause of the item above: in the runs since, no guest sent one.
- **A request to the partition manager timed out.** Its client waited
  2 s for an answer, as PID 1's does; a request that changes a profile
  writes a state file first, and with two partitions busy on a loaded
  host that took longer (`partition-core-sched`,
  `partition-isolation`, `EAGAIN`). The client now waits 30 s.
- **`partition-power-pull` flagged auto start after activating**, which
  the partition manager refuses while a partition is activated. It is
  now flagged first.
- **`partition-scsi` read the unit's counts before their line had
  ended.** It now waits for the whole line.
- **The QEMU workflow had never passed the first test-image suite.**
  The harness made its Unix sockets beside its scratch files, under
  the checkout's `target/`: on a CI runner that path is 113 to 132
  bytes, where a socket's has room for 107, and QEMU would not start.
  They are now under `/tmp`. The workflow's step that reads the pin
  had lost its backslashes and read nothing. It was rewritten: one job
  builds the images (the QS/400 guest image kept by pin), then the
  shards of `.github/qemu-shards.txt` run side by side.
- **The `report` fuzz target's check was wrong.** `END -05` is exit
  code -5, which the text does not spell; the check now says the code
  is what an `END` line parses to.
- **`partition-minimum` read the wrong partition's line.** It waited
  for the next `C2005060` and found the first partition's, still
  unread. It now waits for the line with the second one's name.

## Not done

- `BLOCKED(phase4)`: an adapter's unit numbers have no gaps: the virtio-scsi device numbers its units by their place.
- `BLOCKED(phase4)`: the kernel is built for 64 threads (`NR_CPUS=64`, the pinned configuration's); `docs/OPEN_QUESTIONS.md` item 32.
- `BLOCKED(phase4)`: cores and huge pages are given without regard to sockets or NUMA nodes.
- `BLOCKED(phase4)`: only 2 MB pages; 1 GiB pages are not used.
- `BLOCKED(phase4)`: a partition's history is in memory and the host's log: a partition manager started again has the codes of the activations that still run, not older ones.
- `BLOCKED(phase4)`: a VMM found running by a partition manager started again is watched by looking at its control group every 25 ms, not by an event.
- `BLOCKED(phase4)`: when two disks have the three partitions, the same ESP identifier and the firmware's entry names no PCI function, the disk of the IPL is the first in name order: a byte-for-byte copy of the host's own disk beside it, started from an entry QS/VS wrote, cannot be told apart.
- `BLOCKED(phase4)`: the device path's SATA, SCSI and NVMe nodes are not read: two disks behind one controller are told apart by the ESP's identifier alone.
- `BLOCKED(phase4)`: the system calls `qsvmm` may make are still not filtered (Phase 11).
- What a profile's IPL type means, what an outside `SIGTERM` leaves a
  partition in, and what a processing unit is worth where a core has
  two threads were decided here and want confirming:
  `docs/OPEN_QUESTIONS.md` items 30, 31 and 33.
- What Phase 3 left undone and this did not touch is still as
  `docs/PHASE3.md` lists it.

Not in Phase 4, and not started: storage pools and virtual disks,
optical and tape, IPL source D, virtual Ethernet, the service channel,
delayed and operating-system shut down, console attach, panel
functions, dumps, QSI, any change to a running partition's resources.

## For a real machine (NEEDS-CODY)

Everything here ran under QEMU on a build host that is itself a
virtual machine: a partition is three levels deep, the host's "cores"
are QEMU's virtual CPUs, and SMT is what QEMU was told to show.

- **NEEDS-CODY: suites 5 to 8 on a machine that is no virtual
  machine**, with SMT on:
  - [ ] `partition-dedicated`: the virtual CPUs of a dedicated
        partition on its cores' threads and nothing else there, on a
        machine whose cores have two real threads.
  - [ ] `partition-capped`: within 5% of half a core, not the 10%
        allowed here.
  - [ ] `partition-uncapped`: the shares under contention.
  - [ ] `partition-core-sched`: two partitions never on the two
        threads of one physical core at once. Here the "threads" were
        two virtual CPUs of QEMU; on real SMT it is the property the
        decision exists for.
- [ ] The huge pages at the IPL of a machine with real memory: how
      long the reservation takes, and whether the kernel gives all of
      them (`C2001072`, or `A2001072` with the numbers).
- [ ] The disk of the IPL on real firmware: `partition-own-disks`'s
      first half with a second disk that has QS/VS's partitions, on
      the Dell machine, started from QS/VS's own boot entry and from
      the firmware's.
- [ ] A host with more than one socket: which cores a dedicated
      partition is given.
- The first partition on real hardware, and the partition suites on a
  Linux machine that is no virtual machine, are still as
  `docs/PHASE3.md` lists them.
