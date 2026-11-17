# The VMM: a partition's machine

`qsvmm` is one process for one partition. It is given the partition's
configuration for one activation (below), builds the machine on KVM,
IPLs the guest
from its load source (`vmm/ipl`), runs it, and exits with a code that
says why the partition ended. It has no firmware to hand the guest,
no QEMU beside it, and nothing that listens on a network.

The code is in `vmm/`: `arch` (the boot state), `acpi` (the tables),
`devices`, `ipl` (the loader), `qsvmm` (the process).

## The configuration

The partition manager starts `qsvmm` and writes its whole
configuration to its standard input: a profile resolved
(`docs/PARTD.md`, `docs/RESOURCES.md`). `qsvmm` reads no profile and
decides nothing: how many virtual CPUs, on which host threads, how
much memory, which side, is all in what it is given. It is checked
again all the same, for what `qsvmm` itself relies on, and a key it
does not know is refused.

```toml
id = 3
name = "PROD"
serial = "8GQ2W03"
vcpus = 4
threads_per_core = 2
pin = [6, 14, 7, 15]
memory_mib = 2048
huge_pages = true
side = "recorded"
keylock = "normal"
ipl_type = "unattended"
console = "serial"

[[adapter]]
slot = 2

[[adapter.unit]]
lun = 0
backing = "/proc/self/fd/3"
serial = "PROD-LS"

[[optical]]
slot = 3
serial = "8GQ2W03-3-0"
media = "/proc/self/fd/4"
name = "QS400V1R1"

[[tape]]
slot = 4
serial = "8GQ2W03-4-0"
volume = "/proc/self/fd/5"
name = "SAVE1"
capacity_mib = 4096
protected = false

[[net]]
slot = 5
mac = "02:5a:3c:00:03:05"
tap = "/proc/self/fd/6"
mtu = 1500

[[port]]
name = "systems.quartz.qsvs.service.0"
socket = "/proc/self/fd/7"

[[port]]
name = "systems.quartz.qsvs.console.0"
socket = "/proc/self/fd/8"
```

| Key | Value |
|---|---|
| `id`, `name` | the partition's ID, 1 to 999, and its name. The ID names its log directory, its console socket and its user |
| `serial` | its system serial number, for the SMBIOS tables |
| `vcpus`, `threads_per_core` | the virtual CPUs, and how many of them are the sibling threads of one core: 1 or 2 |
| `pin` | dedicated processors: the host thread each virtual CPU's thread stays on, in order. Left out for shared processors |
| `memory_mib`, `huge_pages` | the memory, and whether it is the 2 MB huge pages the host reserved |
| `side` | `a`, `b`, `recorded`, or `d`: the installation media in the first optical device, which must then be there |
| `keylock`, `ipl_type`, `console` | written to the LIC record before the IPL; each may be left out, and is then left as the record has it. A load source with no LIC partition has nowhere to keep them: it loads when the side is named, with `C2005037`, and they are not written |
| `[[adapter]]` | a virtual SCSI adapter: its `slot`, and its units, each with its `lun`, `backing` and `serial` |
| `[[optical]]` | a virtual optical device: its `slot`, `serial`, and the image loaded at the start (`media`, with its library `name`) or none. Four at most |
| `[[tape]]` | a virtual tape device: its `slot`, `serial`, and the volume mounted at the start (`volume`, `name`, `capacity_mib`, `protected`) or none; or a physical drive's relay (`drive`, a socket's descriptor, and its `location`). Four at most |
| `[[net]]` | a virtual Ethernet adapter: its `slot`, its `mac`, its port's `tap` or none (no link), and the switch's `mtu`, 68 to 9000. Eight at most |
| `[[port]]` | a port of the virtio-console: its `name`, 1 to 64 printable characters, as the guest finds it, and its `socket`, a listening Unix socket the partition manager made. Four at most; none, and the partition has no virtio-console. The partition manager gives the service channel's two, named as QS/400's protocol crate names them (`docs/SERVICE.md`): `qsvmm` spells no name of its own |
| `console` | `serial`, `local` or `partition`, the console mode for the LIC record: `partition` is the console port |
| `vfio` | the VFIO container the partition manager opened, as a descriptor; only with devices |
| `[[device]]` | a host's PCI function given whole (`docs/PHYSICALIO.md`): its `location`, its host `address`, and its `group`, the descriptor of its isolation group, which every function of the group names |

A backing store, an image or a volume the partition manager holds is
given to `qsvmm` as one of its open descriptors and named by its
`/proc/self/fd/N` path, which `qsvmm` opens as it would any path: the
same file, so a rename or a removal of the name cannot put another
under it. A tap is the exception: opened again by its path it would be
a new, unattached tun, so `qsvmm` takes descriptor N itself, checked to
be open and a character device; so with a port's listening socket,
checked to be a socket. Every slot is a device's alone, whatever its
kind.

A developer can name a file in place of standard input: `qsvmm
--config <file>`.

## The life of the process

1. The configuration is read. One that is refused ends here
   (`A2005000`).
2. **As root**: `/dev/kvm` is opened and a VM made with the interrupt
   controller and the PIT in the kernel; the partition's log directory
   and console socket are made; every disk unit is opened for direct
   I/O; `/dev/urandom` is opened; the guest's memory is mapped and
   given to KVM; the IPL loader reads the load source and loads the
   side's kernel; the ACPI and SMBIOS tables are written; the devices
   and the processors are made; the process gives itself a core
   scheduling cookie (`docs/RESOURCES.md`).
3. The process becomes the partition's own user and group, 23000 plus
   the partition's ID, for good: no supplementary
   group, every capability dropped from the bounding set, none
   permitted, effective, inheritable or ambient, and `no_new_privs`
   set. It then checks that it cannot become root again. This is done
   while the process has one thread, so no thread keeps what was
   dropped. Every partition has a user of its own, so one partition's
   process can neither signal another's nor read its files.
4. The threads are started: one for each virtual CPU (each pinned to
   its host thread when the processors are dedicated), one that writes
   the serial output, one that takes console connections, one that
   waits for a signal. A virtio device starts its own when the guest's
   driver is ready.
5. A processor, a device or a signal ends the partition. What the
   guest last said is written to the log, each unit's counts are
   reported, and the process exits.

After step 3 the process opens nothing but new generations of its
serial log, in a directory that is its own. The seccomp filter is
Phase 11; step 3 is where it will be installed, after the last open
and before the first thread.

`qsvmm` reports on standard error, which the partition manager gives
it as a file. A reference code is a line `REF <code> <text>`, the text
naming the partition; the partition manager posts it
(`docs/REFCODES.md`, the `C2005` and `A2005` blocks). `C2005090`,
"Partition started", is the report that the IPL loader has entered the
kernel. After the LIC record is read, a line `CHANNEL unsaid`,
`CHANNEL none` or `CHANNEL has <version>` says what the record says of
the system's service channel (TLV `0x010C`, QS/400's `docs/QSID.md`
section 11), which decides how long the partition manager waits for it
(`docs/SERVICE.md`, "States"). The report's last line is
`END <exit code>`, for a partition manager that was not there to wait
for the process. `qsvmm` cannot post by itself: PID 1's control socket
is root's.

## Exit codes

| Exit | Why | Posted by whoever started it |
|---|---|---|
| 0 | the guest powered off | `C2005100` |
| 10 | the guest asked for a restart. The process ends; it does not restart in place | `C2005110` |
| 20 | not used since Phase 7: a guest fault keeps the partition (below, "A partition kept for a dump") | `A2005120` |
| 30 | the partition did not IPL: the profile, a disk unit, or what is on the load source. The `A2005` code before it says which | `A2005100` |
| 40 | the VMM failed: a KVM error, a thread that could not be started | `A2005130` |
| 50 | the VMM was sent `SIGTERM`, `SIGINT` or `SIGHUP` | `A2005140` |
| a signal | the VMM was killed | `A2005150` |
| 2 | the command line was not one | |

Any other code (a panic's 101) is read as a VMM error.

## Memory

An anonymous shared mapping for each region, of the 2 MB huge pages the
host reserved at its IPL (`docs/RESOURCES.md`). The pages are reserved
for the mapping as it is made, so a partition whose memory is not
there ends at its activation with `A2005065`, and never faults for a
page later. Run by hand without huge pages (`huge_pages = false`), the
mapping is of ordinary pages, not reserved ahead of use.

| From | To | What |
|---|---|---|
| `0` | `9FC00` | RAM. The boot structures: the GDT at `500`, the zero page at `7000`, six pages of page tables from `9000`, the (empty) command line at `20000` |
| `9FC00` | `A0000` | reserved |
| `E0000` | `F0000` | the ACPI tables, the root pointer first |
| `F0000` | `FF000` | reserved. The SMBIOS tables, the entry point first |
| `FF000` | `100000` | reserved. Seven bytes at the reset vector, `FFFF0`: `mov al, FE`, `out 64, al`, `hlt` and a jump back to it |
| `100000` | 3 GiB at most | RAM. The kernel is loaded at `1000000` |
| `C0000000` | `E8000000` | PCI memory space |
| `E8000000` | `E8100000` | PCI configuration space, memory mapped (bus 0) |
| `FEC00000` | | the I/O APIC |
| `FEE00000` | | the local APICs |
| `FFFBC000` | `FFFC0000` | KVM's own: the identity map page and the TSS pages |
| `100000000` | | RAM beyond 3 GiB |

## Processors

A partition's processor is a host core (`docs/RESOURCES.md`): one
virtual CPU where a core has one thread, two sibling ones where it has
two. Each virtual CPU is a thread of `qsvmm`. With dedicated
processors each thread is pinned to one host thread of the partition's
cores, one to one; with shared processors they float over the pool's.

Virtual CPU 0 is put where the 64-bit Linux boot
protocol wants it: long mode, the first four gigabytes mapped one to
one in 2 MiB pages, flat segments, interrupts off, `rsi` at the zero
page, `rip` 0x200 past where the kernel was loaded. The others wait,
as KVM leaves them, for the kernel to start them.

**CPUID** is what KVM supports on the host, with these changes:

- the topology leaves (1, 4, 0xB, 0x1F, and AMD's 0x80000008 and
  0x8000001E) say one package of as many cores as the partition has
  processors, each of one thread or of two sibling threads; a virtual
  CPU's APIC id is its number, so a core's threads are next to each
  other;
- the hypervisor bit, the TSC deadline timer and x2APIC are set;
- the hypervisor leaves are KVM's own (`KVMKVMKVM`), so the guest uses
  the paravirtual clock;
- hardware virtualization (VMX, SVM) is withheld: a partition does not
  run a hypervisor of its own.

**MSRs** set before the first run: the `SYSENTER` and `SYSCALL`
registers zero, fast strings on, and the MTRR default type write-back
with MTRRs enabled (without it the kernel takes all of memory for
uncacheable). The local APIC's LINT0 takes external interrupts and
LINT1 the NMI.

**Exits.** A port or memory access goes to the device there; an access
to nothing reads all ones and writes nowhere. A triple fault, a failed
entry or a crash the guest reports ends the partition as a guest fault;
KVM's own shutdown and reset events as a power off and a restart; a
halt cannot come out of KVM with the interrupt controller in the
kernel, and one that did would be a guest fault. A power off or a
restart the guest asks of the devices below ends the partition from
the device.

**The reset vector.** A Linux kernel that finds hardware-reduced ACPI
and has EFI support compiled in restarts by EFI's reset service. A
partition has none, and the kernel's next and last resort is a jump in
real mode to the reset vector; the ACPI reset register and the
keyboard controller are not tried. So the reset vector holds seven
bytes that pulse the reset line, in a page the memory map reserves.
They are the only bytes of the VMM's a guest ever runs, they run when
a guest jumps there and never before its kernel, and a guest that
writes over them has only undone its own restart. Kept as a decision
(`docs/OPEN_QUESTIONS.md` item 29).

## Devices

No firmware, no VGA, no keyboard, no floppy, no IDE.

| Device | Where | What it is |
|---|---|---|
| Interrupt controllers | in KVM | two 8259s, the I/O APIC, a local APIC a processor. The ACPI tables are hardware-reduced, so the kernel uses the APICs only |
| PIT | in KVM | made, and not used by a kernel that has the paravirtual clock |
| Serial port | ports `3F8` to `3FF`, interrupt 4 | a 16550 (`vm-superio`). The guest's kernel messages and, in serial console mode, its console |
| Clock | ports `70`, `71`, interrupt 8 (never raised) | a CMOS clock: the time in BCD, 24-hour, UTC. A guest that sets it moves it for as long as the partition runs |
| Reset line | ports `60` to `64` | `vm-superio`'s i8042, of which only the pulse on the reset line (`FE` to port `64`) does anything: the partition ends for a restart. The FADT names that port as the ACPI reset register, and the reset vector pulses it |
| Power off | ports `600`, `601` | the ACPI sleep control and status registers of a hardware-reduced machine. S5 with the enable bit ends the partition as powered off |
| PCI host bridge | ports `CF8` to `CFF`, and memory at `E8000000` | one bus. Device 0 is the bridge (`1b36:0008`); a virtio function follows at each device number |
| virtio-scsi | PCI `00:01.0` and on, `1af4:1048` | one function a device, in slot order: a SCSI adapter's disks at its LUNs, an optical drive or a tape drive at LUN 0 of a function of its own |
| virtio-net | among them by slot, `1af4:1041` | a virtual Ethernet adapter, its frames served by vhost-net from its port's tap |
| virtio-console | after them, `1af4:1043` | the service channel's two named ports, each joined to a socket of the partition manager's (below) |
| virtio-rng | after them, `1af4:1044` | the host's `/dev/urandom`, a page at most a buffer |
| a host's function | last, at the next device numbers | a PCI function of the host given whole (below) |

### virtio over PCI

Modern (version 1) only. Each function has two 32-bit memory BARs, in
a 64 KiB slice of PCI memory space: BAR 0 holds the common
configuration (offset `0000`), the interrupt status (`1000`), the
device's configuration (`2000`) and the notification area (`3000`,
four bytes a queue); BAR 1 the MSI-X table and its pending bits. A
guest may move a BAR; the notifications follow it.

Interrupts are MSI-X only: a function has no interrupt pin. Each
vector has an interrupt number of its own from 24 up, an eventfd KVM
raises it from (`irqfd`), and a route written into KVM's table when
the guest unmasks it; a vector signalled while masked stays pending. A
notification is an eventfd KVM writes without leaving the kernel
(`ioeventfd`). The features offered are `VERSION_1` and
`RING_INDIRECT_DESC`.

A device's queues are served on threads of its own, never on a
processor's thread: virtio-scsi has one for requests and one for the
control queue.

Not there, and not missed by a Linux guest: the PCI configuration
access capability (`VIRTIO_PCI_CAP_PCI_CFG`), legacy and transitional
interfaces, `INTx`, `EVENT_IDX`, packed queues.

### virtio-scsi

One request queue of 256, the control queue, the event queue (never
used: nothing is plugged or resized under a running partition). Each
virtual SCSI adapter of the configuration is a virtio-scsi function of
its own, in slot order; an adapter's units are the LUNs of its target
0, numbered from 0 without a gap, and the load source is LUN 0 of the
first. An inquiry
gives `QUARTZ` / `VIRTUAL DISK` / `0100` and the unit's serial
number in the unit serial number and device identification pages.

The commands a disk takes: TEST UNIT READY, REQUEST SENSE, INQUIRY and
its vital product data pages (supported pages, serial number, device
identification, block limits, block device characteristics, logical
block provisioning), REPORT LUNS, READ CAPACITY (10) and (16), READ and
WRITE (6), (10), (12) and (16), VERIFY without comparison, SYNCHRONIZE
CACHE, MODE SENSE (6) and (10) for the caching page, UNMAP, WRITE SAME
(10) and (16), START STOP UNIT and PREVENT ALLOW MEDIUM REMOVAL (which
do nothing). Everything else ends with CHECK CONDITION and sense
data: illegal request, invalid command operation code. Every task
management function (an abort or a reset, which a guest sends when a
request has taken too long) is answered complete, but only once every
request the guest made before it has ended: the control queue's thread
waits for the request in progress and serves those still waiting
itself. The guest takes the answer to mean the device has done with
those requests' buffers and reuses them, so a request that ended after
it would write into memory that is no longer its. A request stuck in
the host's I/O so keeps the answer back, and the guest's error handling
goes on to its next step. At its end `qsvmm` reports how many there
were, an adapter at a time, when there were any:
`qsvmm: adapter 0 task management 3`.

**The backing store** is a file or a block device of the host, opened
with `O_DIRECT`, and a block device with `O_EXCL` too, so one the host
has mounted or another partition has is refused. Nothing a guest wrote
waits in the host's page cache. The caching mode page says the write
cache is on, so the guest sends its flushes; SYNCHRONIZE CACHE, and a
write with FUA, is `fdatasync` on the host before the command ends.
The guest's own crash consistency rests on those two things. UNMAP and
WRITE SAME with UNMAP are offered where the store takes a discard (a
file always; a block device by its queue's limits), and a discarded
range is said to read as zeros only for a file.

Each unit counts its reads, writes, flushes and refused commands;
`qsvmm` reports them at its end.

### The optical device

A read-only CD/DVD drive of 2048-byte blocks (peripheral type 5,
removable), `QUARTZ` / `VIRTUAL OPTICAL`, with an image of the media
library behind it, read through the page cache. It answers what Linux's
`sr` and `cdrom` drivers send, which is all QS/400 sends to an optical
drive (`docs/QS400_GUEST.md`): TEST UNIT READY, REQUEST SENSE, INQUIRY
and the serial number and identification pages, READ CAPACITY, READ
(10) and (12), READ TOC (the tracks and the first session, by address
or time), GET CONFIGURATION (the profile list, core, morphing,
removable medium, random readable and the CD or DVD read feature; DVD
for an image over 900 MiB), GET EVENT STATUS NOTIFICATION (polled, the
media class), MODE SENSE (6) and (10) for the capabilities page, READ
DISC INFORMATION, START STOP UNIT and its eject, PREVENT ALLOW MEDIUM
REMOVAL, SEEK, VERIFY, REPORT LUNS. A write is DATA PROTECT; everything
else ILLEGAL REQUEST, invalid operation code.

The first command after the device starts, and the first after a
medium is loaded or unloaded, is answered UNIT ATTENTION (29/00, then
28/00), except INQUIRY, REQUEST SENSE, GET CONFIGURATION, GET EVENT
STATUS NOTIFICATION and REPORT LUNS, which neither report it nor clear
it; the next event poll says new media or media removal. The guest's
eject unloads the medium and `qsvmm` reports it (`MEDIA <slot>
unloaded <name>`). A guest that prevents removal makes the host's
unload fail unless forced, and its own eject fail.

### The tape device

A sequential-access drive (peripheral type 1, removable), `QUARTZ` /
`VIRTUAL TAPE`, with a volume of the media library behind it in the
AWSTAPE format (`docs/STORAGE.md`). It answers what Linux's `st`
driver sends, which is all QS/400 sends to a tape drive: TEST UNIT
READY, REQUEST SENSE, INQUIRY, READ BLOCK LIMITS (1 byte to 1 MiB),
MODE SENSE and MODE SELECT (6) and (10) with the block descriptor
(variable blocks to begin with, fixed by a descriptor's length) and
the data compression and device configuration pages, READ (6) and
WRITE (6) in variable and fixed blocks, WRITE FILEMARKS (6), SPACE (6)
by blocks, filemarks and to the end of data, REWIND, LOCATE (10), READ
POSITION (short form), LOAD UNLOAD, ERASE (6), PREVENT ALLOW MEDIUM
REMOVAL, REPORT DENSITY SUPPORT, REPORT LUNS.

The conditions are reported as SSC has them, with the information
field the residue:

| Condition | Status, sense key, ASC/ASCQ, bits |
|---|---|
| a filemark read or spaced to | CHECK CONDITION, NO SENSE, 00/01, FILEMARK; the residue the count not done |
| the end of data | BLANK CHECK, 00/05 |
| the beginning reached spacing back | NO SENSE, 00/04, EOM |
| a block of another length | NO SENSE, ILI; the data given; the residue negative for a longer block. SILI suppresses it for a shorter one in variable mode |
| the early warning | NO SENSE, 00/02, EOM; the block written |
| the end of the volume | VOLUME OVERFLOW, 00/02, EOM; nothing written |
| a write-protected volume | DATA PROTECT, 27/00 |

A write reaches the page cache; WRITE FILEMARKS (of none as well, the
guest's flush) makes everything durable, and so does an unload and the
partition's end.

A physical drive's tape device (`drive` in its `[[tape]]`) answers
nothing itself: each command the guest sends is checked against the
allow-list (`docs/STORAGE.md`, "Tape pass-through"), refused with
ILLEGAL REQUEST, INVALID COMMAND OPERATION CODE when it is not on it,
and otherwise sent over the relay's socket as a bounded frame, its
status, sense and data given back to the guest as the drive gave them.
A relay that fails ends the command with a transport failure.

### A host's function

A function given whole is reached through VFIO. The partition manager
opens the container, each group and each device as root and hands
them over; `qsvmm` checks the device's regions and interrupts, then:

- maps the partition's memory into the container at its guest
  addresses, all of it and nothing else, before the guest runs (the
  memory is huge pages, pinned already);
- adds the container's groups to KVM's VFIO device, so the guest's
  cache-coherency matches the device's;
- places each memory BAR in the PCI memory window from the top down,
  aligned to its size, and maps it into the guest from KVM memory slot
  32 on, but for the pages of the MSI-X table and pending bits, which
  it answers itself;
- presents the configuration space as `docs/PHYSICALIO.md` has it;
- gives each MSI or MSI-X vector the guest enables an interrupt number
  of its own and an eventfd that VFIO signals and KVM delivers;
- keeps bus mastering off at the device until the memory is mapped.

I/O BARs, the expansion ROM and line interrupts are never given.

### virtio-net

A virtual Ethernet adapter is a function of its own in its slot, PCI
class Ethernet. Its frames do not pass through `qsvmm`: its receive and
transmit queues (256 entries each) are served in the kernel by
vhost-net, from the tap of the adapter's port, which the switch service
made and the partition manager gave (`docs/NETWORK.md`). `/dev/vhost-net`
is opened as root before the privilege drop; at the first driver's start
`qsvmm` makes itself its owner and gives it the guest's memory; at each
start it sets the features vhost-net takes (`MRG_RXBUF`, `VERSION_1`,
`RING_INDIRECT_DESC`), tells the tap which offloads the guest took, and
gives each queue its size, base, rings at `qsvmm`'s own addresses, the
transport's notification eventfd (the ioeventfd) as its kick and the
queue's MSI-X vector's eventfd (the irqfd) as its call, and last the
tap. A vector the guest masks after it was routed is still raised
through it; a Linux guest does not mask them in use.

| Feature | Offered |
|---|---|
| `MAC` | the profile's address |
| `STATUS` | link up with a tap; down without one, an adapter not required whose switch was missing. It does not follow the uplink |
| `MTU` | the switch's: a Linux guest takes it as its MTU and its largest |
| `MRG_RXBUF` | yes |
| `CSUM`, `GUEST_CSUM`, `HOST_TSO4`, `HOST_TSO6`, `HOST_ECN`, `GUEST_TSO4`, `GUEST_TSO6`, `GUEST_ECN` | with a tap; the guest's choice is the tap's `TUNSETOFFLOAD` |
| `CTRL_VQ`, `CTRL_RX`, `CTRL_MAC_ADDR` | the control queue (64 entries), served by `qsvmm` on a thread of its own |
| `SPEED_DUPLEX` | 10000 Mb/s, full duplex; unknown without link |

Not offered: more queue pairs, `CTRL_VLAN` (the port filters VLANs),
`CTRL_GUEST_OFFLOADS`, `GUEST_ANNOUNCE`, `EVENT_IDX`.

The control queue's requests are read to 4 KiB at most and an address
table to 64 entries; anything that does not hold is answered with an
error. Receive modes are recorded and answered: the port decides what
reaches the adapter. An address the guest sets for itself is recorded,
written to the report as `NET <slot> the guest set its address to
<address>` (the partition manager logs it), answered, and not passed
on. Without a tap, what the guest sends is taken and goes nowhere.

A reset by the guest stops vhost-net's queues (each back end taken away
and its base read back) and the control queue's thread; the next
driver's start sets them again. A guest's restart and the partition's
end close the tap and vhost-net with the process.

### virtio-console

The service channel's device (`docs/SERVICE.md`): `VIRTIO_CONSOLE_F_MULTIPORT`
and the control queue, the configuration saying the most ports, no
size and no emergency write. Its ports have IDs from 1, in the order of
the configuration's `[[port]]`s; port 0, which a guest takes for a
console of its own, is never added. The queues are port 0's two, the
control queue's two, then each port's two, to the guest and from it.

On the guest's `DEVICE_READY` each port is announced (`DEVICE_ADD`);
on its `PORT_READY` the port's name (`PORT_NAME`) and whether the
host's side is open (`PORT_OPEN`). The guest's own `PORT_OPEN` is kept
and counted; its size message, and any event the device does not take,
are passed over. At most one message of each kind waits for a port,
however often a guest asks: what waits is bounded.

Each port is joined to the host by a link: 64 KiB each way. The device
works its end on its worker's thread and never waits. Toward the host,
it takes the guest's buffers only while the link has room, and reads a
buffer only to 64 KiB, so a slow reader stalls the port and the guest's
writes wait in its own queue; toward the guest, the host's writer waits
while the link is full. The host's end is `qsvmm`'s threads: one a port
takes one peer at a time on the listening socket it was given and
carries bytes both ways, reading none of them. A peer that goes away
(the partition manager restarting) is told to the guest as the port's
host side closed, and opened again when the next connects. What the
guest writes with no peer, and what comes for it with its port closed,
is dropped.

The device is there from the kernel's first instruction. A guest's
restart ends the process, and the next VMM has the device again.

### The control socket

`<run>/partitions/<ID>/control.sock`, made by `qsvmm` as root before
it drops its privileges and root's alone (mode 600): the partition
manager reaches it, no partition does. It is served whatever devices
the partition has. One request a connection: a line, after one byte
that carries an open file when there is one (`SCM_RIGHTS`):

| Request | Answer |
|---|---|
| `load <slot> <name> <capacity MiB> <protected>`, the medium's file with it | `ok <name>` or `refused <why>` |
| `unload <slot> <forced>` | the same |
| `media` | a line a device, then `end` |
| `hold` | `ok kept`: every processor stopped and the partition kept (below) |
| `kept` | `ok true` or `ok false`: whether it is kept |
| `dump <time> <code> <reason> <text>`, the dump's file with it | `ok <bytes> <bytes of memory written>` once the dump is whole, or `refused <why>` (`docs/DUMPS.md`) |

## A partition kept for a dump

A fault the VMM survives does not end the partition: a triple fault, a
halt with nothing to wake it, an entry KVM refused, a crash the guest
reported, an exit the VMM does not handle. The processor that met it
reports `A2005170` with which, stops every other, and parks itself;
the memory and every processor's state are kept. The partition
manager's `hold` does the same for a system that posted a terminal
code. A kept partition is not resumed: a dump, an immediate shut down
or a restart is what follows (`docs/SERVICE.md`). Each processor leaves
its registers when it parks, which a dump writes.

A dump of a partition that runs stops its processors the same way for
as long as it takes, and lets them go on. The stops are apart: a
partition kept for a dump stays stopped when its pool has room again.

## A partition held for its pool's space

A write to a virtual disk that the host's file system answers with no
space (a thin disk in a full pool) does not reach the guest. The
device thread that made it waits: the first such write stops every
processor of the partition (each is brought out of the guest and
parks before it enters it again) and `qsvmm` reports `A2005160`. The
write is tried again each second. When the last write that waited is
done, `C2005160` and the processors go on, each telling KVM it was
stopped so the guest's clock does not take the time for a lockup. The
partition is Running throughout (`docs/PARTD.md`).

## Interrupts

KVM's routing table has the first 24 interrupt numbers on the
controllers' pins, as KVM sets them up itself, and a message route for
each MSI-X vector the guest has unmasked. The serial port's interrupt
is an eventfd on number 4.

## ACPI

One block at `E0000`: the RSDP, then the DSDT, FADT, MADT, MCFG and
XSDT. OEM id `QUARTZ`, OEM table id `QSVSPART`.

| Table | What it says |
|---|---|
| FADT | hardware-reduced; no 8042 keyboard, no VGA; the reset register is I/O port `64`, value `FE`; the sleep control and status registers are I/O ports `600` and `601`; hypervisor vendor identity `QUARTZVS` |
| MADT | a local APIC for each processor, one I/O APIC |
| MCFG | configuration space of segment 0, bus 0, at `E8000000` |
| DSDT | `\_SB.PCI0` (`PNP0A08`, `PNP0A03`: bus 0, the configuration ports, PCI memory space); `\_SB.MBRD` (`PNP0C02`: the memory-mapped configuration space, so the kernel trusts the MCFG); `\_SB.COM1` (`PNP0501`); `\_SB.RTC0` (`PNP0B00`); `\_SB.PWRD` (`PNP0C02`: the sleep registers); `\_S5` |

## SMBIOS

A partition has no firmware to say what machine it is, so `qsvmm`
writes SMBIOS tables where a kernel without firmware looks for them: a
64-bit entry point (`_SM3_`) at `F0000`, the structures after it.

| Type | What | Says |
|---|---|---|
| 0 | BIOS | the vendor `Quartz Systems`, the version `QS/VS` and the host's release; a virtual machine |
| 1 | System | the manufacturer `Quartz Systems`, the product `QS/VS partition`, the release, the partition's serial number, a UUID made from the serial number, the family `QS/VS` |
| 3 | Chassis | the manufacturer and the serial number again |
| 127 | End | |

The serial number is the partition's own, from its definition, the
same at every activation (`docs/PROFILE.md`). The manufacturer and the
product are what QS/400 knows a partition of QS/VS by
(`docs/QS400_GUEST.md`). Every string is the host's; nothing in the
tables comes from the guest.

## The serial port's far side

What the guest transmits goes, through a bounded queue and one thread,
to `<data>/log/partitions/<ID>/serial.log` and to whoever is
connected to `<run>/partitions/<ID>/console.sock`. Both directories
are the partition's user's alone. The log is rotated
at 4 MiB with three generations behind it, so a guest that never stops
talking fills a bounded space. The socket is the partition's user's
(mode 600), so root and that user reach it and no other partition
does; a
client reads the guest's output from the moment it connects and writes
the guest's input. A client that does not take the output is dropped;
when the queue is full the guest waits, as on a slow line. A later
phase attaches a console there.

## Asking a partition what it is doing

`SIGUSR1` has `qsvmm` write one line for each processor on its
standard error: where it is (`rip`), whether it takes interrupts,
whether it is halted or was never started, what its local APIC's
timer is armed for beside what the TSC reads now, and the eight bytes
it is about to run. A partition that has gone quiet is either running
somewhere or waiting for an interrupt that does not come, and this
says which. The process goes on.

For a processor that takes no interrupts the line has more: `cr2`,
the general registers, the first 48 words of its stack, and any text
those point at near the code. A kernel that stops itself before it
has a console, as the decompressor does, has its reason nowhere else.
What is read is the guest's and is only shown: each read is bounded
and goes through the guest's own page tables by KVM.

## The threat model for guest-controlled input

The guest is hostile. Everything it can write is taken for an attempt:
on the VMM first, and through it on the host and on other partitions.

**What the guest controls.** Every register access to a port or a
memory address outside its RAM (the serial port, the clock, the reset
and power registers, PCI configuration space, the virtio BARs and
MSI-X tables, a passed function's configuration space and its MSI-X
table); a passed function's own registers and its DMA, within what the
IOMMU lets it reach; every command to a passed tape drive; all of its own memory, and so every byte of every
virtqueue (descriptor tables, rings, the buffers they name); every
SCSI command and its data; every byte of its disks, which the IPL
loader reads at the next activation: the partition table, the ESP's
FAT volume, the UKI, the LIC record.

**What it must not be able to do.** Make `qsvmm` panic, read or write
outside what it was given, allocate without bound, loop without bound,
or reach a file descriptor or a byte of a disk that is not its own.

**How.**

- No device panics on a value: registers are read and written through
  masks and bounds-checked offsets; an access of an unexpected width,
  or beyond a structure, reads ones and writes nothing.
- Guest memory is reached only through `vm-memory`, which checks every
  range against the guest's regions. A descriptor that points outside
  them makes its chain unusable, not a fault.
- A descriptor chain is followed for as many descriptors as the queue
  has and no further (`virtio-queue`); a chain that loops ends there.
  A chain that is not a request is returned with nothing written.
- A SCSI request's block range is checked against the LUN before the
  host is asked for anything: an address or a count that overflows, or
  that ends beyond the unit, is refused with sense data and never
  becomes a host offset. UNMAP checks every range of its list before
  it discards one.
- Data moves through one buffer of a megabyte however much a command
  names; a transfer, an UNMAP list and a WRITE SAME have fixed upper
  bounds, which the device also tells the guest.
- The loader reads the guest's disk as a hostile one: both GPT
  checksums must hold and every count, size and address in the table
  is checked; the FAT reader follows a cluster chain for as many
  clusters as the file's size needs, reads a directory up to a
  megabyte, and refuses a UKI over 128 MiB before reading a byte of
  it; the bzImage's header is checked before the loader sees it; the
  LIC record is decoded by `licpart`, which checks its CRC and its
  lengths. A disk that fails any of it gives an `A2005` code and exit
  code 30.
- The process holds no privilege while the guest runs, and the only
  files it has open are its own units, its media, its log, its sockets,
  the random source, its adapters' taps and `/dev/vhost-net`, the
  service channel's listening sockets. It opens no network device by
  name, and reads none of the bytes its console ports carry.
- The virtio-console's control queue is read a header at a time: a
  message shorter than one, an event or a port the device does not
  have, is passed over; what waits for the guest is one message of a
  kind a port. A port's buffers are read to 64 KiB and taken only while
  the host has room.
- A passed function's configuration space is the VMM's copy: a write
  of any width at any offset reaches the device only through the bits
  the table in `docs/PHYSICALIO.md` names, and its capability list is
  followed a bounded number of times inside the header's 256 bytes.
  Its DMA reaches the partition's memory and nothing else (the IOMMU),
  and its interrupts are remapped. The fuzz target `passthrough` and
  the suite `pio-hostile` hold this.
- A command to a passed tape drive is checked twice, in the VMM and in
  the partition manager's relay, against the allow-list; a frame is
  bounded before it is read.
- A dump is written to the file the partition manager hands over, from
  the guest's memory through `vm-memory`'s bounds; nothing of the
  guest's memory is interpreted.
- A virtio-net adapter's frames are vhost-net's in the kernel, which
  checks the rings against the memory table it was given; the control
  queue's requests are the VMM's, bounded as above.

**How it is checked.** Unit tests for each of the above; the
conformance suites (`partition-scsi` makes the requests no guest
kernel sends, of a real host device; `partition-damaged` gives the
loader seven load sources with one thing wrong each); and the fuzzer
(`tools/fuzz`, `cargo run -p build -- fuzz`), which runs eight targets
on mutated input with LLVM's coverage counters as its guide: the GPT,
the FAT volume, the LIC record, a whole load source, the bzImage, a
SCSI command with its data, guest memory holding the virtio-scsi and
virtio-rng queues, and accesses to the ports, PCI configuration space
and the virtio BARs; since Phase 5 the ISO 9660 volume and the optical
and tape drives' commands, since Phase 6 the virtio-net control queue
(`netctrl`), and since Phase 7 the virtio-console's control queue and a
port's queues (`console`) and what a system sends on its service
channel, as the partition manager takes it (`channel`). The targets that sit behind a checksum are also
run on input whose checksum was made to hold.

**What is not defended yet.** The system calls `qsvmm` can make are
not filtered (Phase 11). KVM itself and the host kernel's block layer
are trusted. A guest can make its own processor threads and its
device threads busy: processor and I/O limits are Phase 4. Timing
between partitions is not considered.
