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
backing = "/dev/sdb"
serial = "PROD-LS"
```

| Key | Value |
|---|---|
| `id`, `name` | the partition's ID, 1 to 999, and its name. The ID names its log directory, its console socket and its user |
| `serial` | its system serial number, for the SMBIOS tables |
| `vcpus`, `threads_per_core` | the virtual CPUs, and how many of them are the sibling threads of one core: 1 or 2 |
| `pin` | dedicated processors: the host thread each virtual CPU's thread stays on, in order. Left out for shared processors |
| `memory_mib`, `huge_pages` | the memory, and whether it is the 2 MB huge pages the host reserved |
| `side` | `a`, `b` or `recorded` |
| `keylock`, `ipl_type`, `console` | written to the LIC record before the IPL; each may be left out, and is then left as the record has it. A load source with no LIC partition has nowhere to keep them: it loads when the side is named, with `C2005037`, and they are not written |
| `[[adapter]]` | a virtual SCSI adapter: its `slot`, and its units, each with its `lun`, `backing` and `serial` |

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
kernel: the partition is Running from then. The report's last line is
`END <exit code>`, for a partition manager that was not there to wait
for the process. `qsvmm` cannot post by itself: PID 1's control socket
is root's.

## Exit codes

| Exit | Why | Posted by whoever started it |
|---|---|---|
| 0 | the guest powered off | `C2005100` |
| 10 | the guest asked for a restart. The process ends; it does not restart in place | `C2005110` |
| 20 | the guest faulted: a triple fault, or KVM could not enter it | `A2005120` |
| 30 | the partition did not IPL: the profile, a disk unit, or what is on the load source. The `A2005` code before it says which | `A2005100` |
| 40 | the VMM failed: a KVM error, an exit it does not handle | `A2005130` |
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
| virtio-scsi | PCI `00:01.0`, `1af4:1048` | one target, a disk at each LUN |
| virtio-rng | PCI `00:02.0`, `1af4:1044` | the host's `/dev/urandom`, a page at most a buffer |

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
MSI-X tables); all of its own memory, and so every byte of every
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
  files it has open are its own units, its log, its console socket and
  the random source.

**How it is checked.** Unit tests for each of the above; the
conformance suites (`partition-scsi` makes the requests no guest
kernel sends, of a real host device; `partition-damaged` gives the
loader seven load sources with one thing wrong each); and the fuzzer
(`tools/fuzz`, `cargo run -p build -- fuzz`), which runs eight targets
on mutated input with LLVM's coverage counters as its guide: the GPT,
the FAT volume, the LIC record, a whole load source, the bzImage, a
SCSI command with its data, guest memory holding the virtio-scsi and
virtio-rng queues, and accesses to the ports, PCI configuration space
and the virtio BARs. The targets that sit behind a checksum are also
run on input whose checksum was made to hold.

**What is not defended yet.** The system calls `qsvmm` can make are
not filtered (Phase 11). KVM itself and the host kernel's block layer
are trusted. A guest can make its own processor threads and its
device threads busy: processor and I/O limits are Phase 4. Timing
between partitions is not considered.
