# QS/400 as a guest: what happens without firmware

A partition has no firmware. `qsvmm` loads the side's UKI from the
partition's load-source disk, hands the kernel ACPI tables and starts
it at its 64-bit entry (`docs/VMM.md`). This is what an unmodified
QS/400 does there, point by point, and what QS/400 would have to
change: a list a QS/400 handoff can be written from.

**The short answer: it IPLs.** An unmodified QS/400 disk image reaches
`SRC C6001090 IPL complete` in a partition, from either side, with one
to four processors, finds its disks on virtio-scsi, takes its time from
the clock, powers off through ACPI, restarts by the reset vector, and
recovers its object store after the VMM is killed under it. Nothing in
QS/400 blocks a suite. Since the pin of Phase 4 (`e035bea`) it also
chooses the side of its next IPL without firmware, through its LIC
record; takes its serial number from the SMBIOS tables QS/VS gives a
partition; and names its hypervisor. The sections below say what was
seen at which pin.

## How it was run

- For Phase 3 the guest was `qs400-V1R1M0-f50c366ffb35.img`, built by
  QS/400's own orchestrator (`cargo run -p build -- all`,
  `QS400_BUILD_ID=qsvs-guest`) from a clone at the commit pinned then,
  `03d6492913318da96bc707cb68d3b54a94ef9bff`, with nothing changed.
  For Phase 4 it is `qs400-V1R1M0-150983bae8c2.img`, built the same
  way at `e035bea3998bd5732750f134eafcd89ba45d638f`. What is marked
  "at `e035bea`" below was seen with that one; `docs/PHASE4.md` says
  how to make it.
- It was run two ways: by `qsvmm` on the build host itself, against a
  copy of the image as a file, with the guest's service console driven
  through the console socket; and by the conformance suites, in a
  partition of a QS/VS host under QEMU, against a block device of that
  host (`docs/PHASE3.md`, suites 3 to 8). The lines quoted below are the
  guest's own, from its serial port.
- A profile of 2 processors and 2048 MiB unless said otherwise.

## The questions

### Does the kernel reach the initramfs with ACPI tables only, and does `slic` find its disks on virtio-scsi?

Yes to both. The kernel is given no firmware services, an empty command
line (it ignores a loader's: `CMDLINE_OVERRIDE`), the memory map and the
address of the ACPI root pointer in the zero page. `slic` starts 0.4 s
after the kernel, and an IPL from an installed disk completes about
10 s after the kernel started:

```
SRC C1001000  IPL started
QS/400 V1R1M0 build qsvs-guest
slic: slot A, mode None, svc on
slic: LIC partition at /dev/sda2
SRC C1001010  LIC partition located
SRC C1001020  LIC record read
slic: hardware resource service 69: 7 resources
SRC C1001030  Hardware resources enumerated
slic: ASP 1 unit /dev/sda3 (3774 MiB)
SRC C1001070  ASP units located
slic: unit 0 is QUARTZ VIRTUAL DISK serial QS400A-0
slic: storage management recovery: 0 transaction(s) replayed, generation 1
SRC C1001080  Storage management recovery
```

The load source is LUN 0 of the virtio-scsi controller and comes up as
`sda`; the serial number is the one the profile gives the unit. The
hardware resource table (`hw` at the service console):

```
PN01      QP01-001  Operational   cpu0                        processor 0
PN02      QP01-001  Operational   cpu1                        processor 1
CMB01     QB01-001  Operational   0000:00:00.0                1b36:0008 host bridge
DC01      QD02-001  Operational   0000:00:01.0                1af4:1048 virtio SCSI
OTH01     QX00-000  Operational   0000:00:02.0                1af4:1044 PCI device
DD001     SSD       Operational   sda               DC01      sda 4096 MiB at 0:0:0:0 on DC01
CMN01     QS01-001  Operational   ttyS0                       serial port ttyS0
```

A disk image that has never IPLed has a blank LIC partition and a blank
ASP 1. `slic` initializes both at its first IPL in the partition
(`slic: LIC partition blank, initializing`, `slic: ASP 1 unit is blank;
formatting`, `SRC C6001007`, `SRC C6001011`), as it does on a machine.

### What does `slic` do when efivarfs is absent?

**At IPL.** Nothing fails. The side is the one baked into the UKI that
was loaded (`slic: slot A`, or `slot B`), which is the one the VMM
chose. `slic` asks the firmware for nothing during a normal IPL unless
a LIC PTF is pending: then `ptflic::at_ipl` opens the firmware, fails
with `not booted through UEFI`, and reports `boot order not changed`
with the IPL going on. That path was read in the source
(`lic/slic/src/cpp/ptflic.rs`), not run: it needs a LIC PTF.

**When `IPLSRC` or a PTF asks for the other side.** At `e035bea` the
request is kept in the LIC record and the hypervisor honours it:

```
QS400-SVC> pwrdwnsys OPTION(*IMMED) IPLSRC(B)
...
slic: next IPL from slot B
SRC D1001090  Power down complete
```

and at the next activation whose IPL source is `recorded`:

```
REF C2005030 Partition LIC record read: TESTLPAR, side B once keylock *NORMAL console serial sequence 9
REF C2005035 Partition IPL attributes written
REF C2005040 Partition IPL image read: TESTLPAR, side B 31589376 bytes
...
slic: slot B, mode None, svc on
```

`slic` writes the TLV `0x010B` when it did not come up through UEFI
and the processor has the hypervisor flag (`lic/slic/src/platform/partfw.rs`
there); the IPL loader reads it through `licpart::IplSource::take`,
once an activation: the side for one IPL is taken and cleared, and the
record written; with none it is the default side; with no TLV, side A.
An activation that names side A or B itself overrides the request and
leaves the record alone. The suite `partition-recorded` shows all of
it from the guest's side.

At the commit pinned for Phase 3 it was refused, in a line on the
serial port, and the power down went on:

```
QS400-SVC> pwrdwnsys OPTION(*IMMED) RESTART(*YES) IPLSRC(B)
...
slic: IPL source not set: not booted through UEFI
SRC D1001090  Power down complete
[   16.506382] reboot: Restarting system
```

The request is lost. `ptflic::arm` computes the slot
(`ptf::slot::next_slot`) and hands it to `EfiFirmware::boot_next`,
which is the only place it would have been kept.

**At install registration.** Read in the source, not run: an
installation is an IPL from media, type D, which is Phase 5.
`install.rs` tries `EfiFirmware::open_for(...).register_install()`,
logs `slic: install: boot entry not set (not booted through UEFI); the
firmware must boot EFI/BOOT/BOOTX64.EFI`, and completes. That is the
right outcome for a partition: side A's path is where the loader looks.
A LIC install over an installed system (`install.rs`, the slot-B path)
likewise logs `boot order not set` and writes slot A as well.

**Does the LIC record hold enough for the hypervisor to honour the
requested side at the next activation? At `e035bea`, yes** (above). At
the commit pinned for Phase 3 it did not:

- The record's header (`licpart::Header`) has `last_slot` and
  `last_good_slot`: where the last IPLs came from. It has no field for
  where the next one is to come from. On a machine that is the
  firmware's `BootNext` and `BootOrder`.
- The default slot and a pending LIC PTF's slot are in the record, in
  the TLV `TAG_LIC_SLOTS`, but its layout is the `ptf` crate's
  (`ptf::slot::SlotState`), which QS/VS does not depend on and should
  not.
- The slot `IPLSRC` names is not recorded anywhere.

So in Phase 3 a profile's recorded side meant the side of the last IPL
that completed (`last_good_slot`). Since Phase 4 it means what the
partition itself asked for; a disk that has never IPLed, or has asked
for nothing, starts from side A.

### Does the console come up on serial when the LIC record says serial, and what does the local-display path do with no framebuffer?

**Serial mode**: yes. With the console mode written as serial by the
host before the IPL:

```
slic: display: no display (Serial console (ttyS0))
slic: 0 keyboard device(s)
SRC C1001050  Console attached
```

Screens are drawn as text on the serial port and typed lines are the
keys. An attended IPL stops at its menu there:

```
                           IPL or Install the System
                                                                 System:  QS400
 Select one of the following:
      1. Perform an IPL
      2. Install the operating system
      3. Use Dedicated Service Tools (DST)
      4. Perform automatic installation of the operating system
      5. Save Licensed Internal Code
 Selection
 F3=Exit   F12=Cancel
```

and goes on to `IPL Options`, `Set Major System Options` and the IPL
when each is answered.

**Local mode** (the record's default): the console is looked for on a
display, there is none, and `slic` says so and goes on:

```
slic: display: no display (Local console (display and keyboard))
slic: 0 keyboard device(s)
slic: framebuffer: No such file or directory (os error 2)
SRC A6005008  Console not found, continuing on serial
```

The IPL completes, the service console is on the serial port, and the
sign-on screen exists and can be read with the service console's
`screen`. One more line follows `Service console ready`:
`slic: console: no session yet: No such file or directory (os error 2)`.
It has no effect that was seen.

### What does `QSHWQUAL` report for the partition's hardware?

`FAIL (5 passed, 6 warnings, 1 failed)`. Line by line:

| Result | Resource | What it says | Why, in a partition |
|---|---|---|---|
| | Machine | `not reported by the firmware` | a partition has no SMBIOS tables |
| | Hypervisor | `unidentified hypervisor` | the CPU's hypervisor flag is set and there is no DMI identity to name it by |
| **FAIL** | UEFI | `Legacy BIOS boot: QS/400 needs UEFI firmware. Switch the machine or VM to UEFI (OVMF, Gen2).` | there is no `/sys/firmware/efi`; the check takes that for a legacy BIOS |
| WARN | TPM | `No TPM was found` | none is given |
| PASS | PN01, PN02 | `processor 0: 06/97`, `processor 1: 06/97` | the host's processor model |
| WARN | MS01 | `Main storage 1966 MiB: enough to IPL; 2048 MiB or more for interactive and batch work.` | the profile gives 2048 MiB; the rest is the kernel's own |
| PASS | DC01 | `1af4:1048 virtio SCSI, driver virtio-pci, 1 disk units` | |
| PASS | DD001 | `sda 4096 MiB at 0:0:0:0 on DC01`, type `SSD` | the LUN says it does not rotate |
| WARN | DD001 | `One disk unit: mirrored protection needs a second unit` | one unit in the profile |
| WARN | CMN | `No Ethernet port: the system works only from its console.` | virtual Ethernet is Phase 6 |
| WARN | AGENT | `No guest agent channel: add a virtio-serial port named org.qemu.guest_agent.0` | no virtio-serial in this phase; the service channel is Phase 7 |
| PASS | CMN01 | `serial port ttyS0: the service console runs here.` | |
| WARN | FB0 | `No framebuffer: the firmware handed over no display, so the console runs on the serial port alone.` | a partition has no display |

### What clock, power-off and restart paths does `slic` take, and does each work?

- **Clock.** `slic` reads `/dev/rtc0` (`RTC_RD_TIME`) and sets the
  system clock: `slic: clock set to 1794537916 UTC from RTC`,
  `SRC C1001040`. It works. The kernel finds the clock as the ACPI
  device `PNP0B00` and reads its registers as BCD whatever the mode bit
  says; the first run here had them in binary and the guest came up in
  2020. That was the VMM's defect and is fixed.
- **Power off.** `reboot(PowerOff)` after `SRC D1001090`. The kernel
  writes the sleep type of S5 to the ACPI sleep control register,
  `qsvmm` ends with exit code 0, and the host posts `C2005100`. Works.
- **Restart.** `reboot(Restart)`. The kernel does not write the ACPI
  reset register: on hardware-reduced ACPI, with EFI support compiled
  in, it asks EFI's reset service, has none in a partition, and jumps
  in real mode to the reset vector. With nothing there the guest ran
  through empty memory and the partition never ended (the first runs
  were read wrongly as working). `qsvmm` now has seven bytes at the
  reset vector that pulse the reset line (`docs/VMM.md`): it ends
  with exit code 10 and the host posts `C2005110`. It is not restarted
  in place: whoever started it decides. A kernel command line cannot
  ask for another method: QS/400's is baked with `CMDLINE_OVERRIDE`.

### Anything else that fails or warns

- **The serial number.** `slic: 159 system values, 0 overridden,
  QSRLNBR 0000000`. With no DMI there is no machine serial number to
  make `QSRLNBR` from. Every partition so has the same one.
- **virtio-rng is an unknown resource**: `OTH01 QX00-000 ... 1af4:1044
  PCI device`. It works; it has no resource type of its own.
- **Processors that go away are remembered.** After an activation with
  four processors, one with two says `hwrsrc: PN03 not detected (cpu2)`
  and `PN04 not detected (cpu3)`. That is the resource service doing
  what it does on a machine; a partition's processors will change with
  its profile.
- **TCP/IP servers start with nothing to listen on** but the loopback
  interface (`QTVTELNET listening on port 23`): there is no Ethernet
  port yet.
- The kernel is quiet (`quiet` is baked in), so what it warned of is
  not on the serial port, and the image has no way to read its log from
  the service console. Nothing it printed above that level was seen but
  `reboot: Power down` and `reboot: Restarting system`.
- An abnormal end is noted as on a machine: after the VMM is killed,
  the next IPL posts `SRC C1002000 Previous IPL was abnormal`, replays
  its transactions (`storage management recovery: 158 transaction(s)
  replayed`) and completes.

## A partition's identity (at `e035bea`)

QS/VS gives each partition SMBIOS tables (`docs/VMM.md`): the
manufacturer `Quartz Systems`, the product `QS/VS partition`, and the
partition's system serial number, which is made once when the
partition is created and never changed (`docs/PROFILE.md`). An
unmodified QS/400 uses all three:

```
slic: 159 system values, 0 overridden, QSRLNBR K2M4101, QTIMZON Q0000UTC
```

```
QS400-SVC> qshwqual
 Machine  . . . . . . :   Quartz Systems QS/VS partition
 Hypervisor . . . . . :   QS/VS
 Firmware and security
  PASS   UEFI                  No firmware: a partition of QS/VS, which loads
```

**What QS/400 reads to use the serial number.** The SMBIOS system
information's serial number (type 1), as the kernel publishes it at
`/sys/class/dmi/id/product_serial`: `hwrsrc::dmi::Dmi::serial()`, which
`slic`'s `host_info()` hands to `sysval` as the machine's genuine
serial number. `sysval::store::fitting_serial` takes it as `QSRLNBR`
when it is letters and digits only and no longer than the value's
length, which a QS/VS serial number always is (seven characters of
`0` to `9` and `A` to `Z` without `I` and `O`). The UUID beside it
(type 1 too, derived from the serial number) is what QS/400 would hash
if the serial number did not fit; it is never needed. QS/400 knows a
partition of QS/VS by the product's name (`hwqual::checks::qsvs`), or
by the ACPI tables' OEM ids and the FADT's hypervisor vendor identity,
which were there since Phase 3.

So nothing is left for QS/400 to change for this: at `e035bea` it
already reads what QS/VS writes. The one thing a QS/400 handoff should
do is **keep it so**: the serial number of a partition is the SMBIOS
type 1 serial number, read from `product_serial`, and must not come to
depend on a firmware variable or on the product UUID.

## Installation media, optical and tape (Phase 5, at `0fc8afa`)

What QS/400 does with the devices of Phase 5, read from its source at
the pin; what a run showed is in `docs/PHASE5.md`.

**The installation media.** QS/400's build puts the media UKI at
`EFI/BOOT/BOOTX64.EFI` in the ISO 9660 tree (and the same in its El
Torito image); the tree has Joliet and Rock Ridge names. IPL source D
reads it from there and enters it as it would a side's UKI. The media
UKI's command line is built in (`CONFIG_CMDLINE_OVERRIDE`): nothing the
VMM passes is seen. Its `/init`, `instload`, looks for the media among
the block devices, `sr*` first, finds an ISO 9660 signature, and mounts
it with the kernel's iso9660; so the media is found in the partition's
optical device by name, and nothing of the device's identity is read.

**An unattended installation** needs `QS400/INSTALL.RSP` on the media
(a build with `QS400_INSTALL_RSP` set): with no OS installed and a
response that parses, the installation runs with no menu. The LIC
phase partitions the disk, writes both sides, records side A as the
default in the LIC record (`licpart::IplSource`, as a partition with no
firmware) and restarts; the partition manager activates it again from
the side recorded, and the OS phase reads the media again: **the
optical device keeps its media across the restart**, which it does
(`docs/PARTD.md`). The installation ends in the running system.

**Optical.** QS/400 sends no command of its own to an optical drive;
it reads `/dev/srN` through Linux's `sr` and `cdrom` drivers and the
iso9660 file system. It takes a device of peripheral type 5 as optical
(`OPTnn`) and reads its serial number from sysfs; vendor and product
are not read. Saving to and restoring from optical are its virtual
(image catalog) devices', not a drive's.

**Tape.** QS/400 drives a tape through Linux's `st` driver alone, on
`/dev/nstN`: variable blocks (`MTSETBLK 0`), 80-byte IBM standard
labels in EBCDIC, 64 KiB save blocks, a 256 KiB read buffer; it spaces
by filemarks and blocks, rewinds, writes filemarks, erases, unloads.
It takes the early warning as `ENOSPC` with `GMT_EOT` set, writes the
block again and ends the volume with `EOV1` labels. Its tape code has
never met a drive, real or virtual (QS/400's own `docs/INSTALL.md`):
a save to this drive and a restore from it were not run in Phase 5
(`docs/OPEN_QUESTIONS.md` item 36).

What QS/400 would have to change for any of it: nothing found. Two
things it could do: read its install response from somewhere other
than the media (the command line it cannot be given, a seed volume),
so one ISO serves attended and unattended installs alike; and say in
its tape code what it expects of a drive past the early warning.

## An Ethernet adapter (Phase 6, at `0fc8afa`)

What QS/400 does with a partition's virtio-net adapter, read from its
source at the pin; what a run showed is `net-qs400` in
`docs/PHASE6.md`.

**The resource.** QS/400's hardware resource service walks sysfs
(`lic/hwrsrc`). The adapter's PCI function, class `02`, is a `LINnn`
resource, `Network Adapter`, described as `1af4:1041 virtio network`
(`scan/pci.rs`). Its port is a `CMNnn` resource, `Ethernet Port`, of
type `QN01` model `001` (`Virtio Ethernet port`) because its driver is
`virtio_net` (`scan/nic.rs`); its description names the link, its MAC
address, and `link up` (a speed is added only when sysfs has one, which
`VIRTIO_NET_F_SPEED_DUPLEX` gives). Its serial is the MAC address
without colons, and the name it keeps across IPLs is keyed by its
permanent MAC address, so the profile's fixed address keeps it `CMN01`.

**What `QSHWQUAL` says.** It is QS/400's hardware qualification report,
not a system value (`lic/hwqual`). Its network check passes a port with
carrier and warns of one without: an adapter whose switch was missing
(not required, link down) warns. It knows QS/VS by the ACPI tables and
passes the firmware check under it (items 3 and 4 below). Nothing in it
is about virtio-net as such.

**Before it has an address.** Nothing is made at IPL: a new system has
the loopback only until an interface is configured (`lic/slic`). An
interface is made either on a line description (`CRTLINETH
RSRCNAME(CMN01)`, then `ADDTCPIFC ... LIND(...)`) or on the kernel link
directly (`LIND(ETH0)`, which counts as always varied on). `QSTRUP`
runs `STRTCP`, which starts every interface and route marked to start,
and the telnet server, whose autostart is on in a new system: no
`STRTCPSVR` is needed. DHCP is `INTNETADR(*IP4DHCP)`.

**Unattended.** Yes. The install response file (`QS400/INSTALL.RSP` on
the media) takes `ip` (`a.b.c.d/n` or `*IP4DHCP`) and `gateway`; the
installation makes an interface marked to start on the first line
description, or on `LIND(ETH0)` when there is none, and a default route.
That is how `net-qs400` configures it: a copy of the ISO whose response
file has both. A response file cannot make a line description, a VLAN
interface, a second interface or another route, or set DNS.

**Tagged VLANs.** Usable from QS/400 today, on the interface and not the
line: `ADDTCPIFC LIND(<line> <vlan>)` names the VLAN, and at the
interface's start QS/400 makes the `<link>.<vlan>` sub-interface through
its `netlink`. `CRTLINETH` has no VLAN parameter. An adapter with that
VLAN among its `tagged` ones carries it.

**Telnet and 5250.** `QTVTELNET` listens on `0.0.0.0:23` (IPv4 only)
and serves the `QDSIGNON` display on a `QPADEVnnnn` device made in
`QAUTOVRT`. `net-qs400` reaches it from the harness's network.

**What QS/400 would have to change**, as a list for a QS/400 handoff:

1. A response file key, or a partition setting, that makes a line
   description (with `RSRCNAME`) and, on it, an interface with a VLAN,
   so that an installation can be configured on a tagged VLAN and on a
   named port, not only on `ETH0`.
2. An `*IP4DHCP` interface on the first Ethernet port made by itself at
   the first IPL when the system knows it is a partition of QS/VS, so a
   partition is reachable with nothing configured.
3. An IPv6 listener for telnet beside the IPv4 one.
4. With two adapters, `LIND(ETH0)` and the response file's interface
   follow the kernel's probe order; naming the port by its resource
   (`CMN01`) would not.
5. Two documents of QS/400's that are out of date: `docs/TN5250.md`
   says `QSTRUP` runs `STRTCPSVR` (it runs `STRTCP`), and
   `docs/TCPIP.md` names `stgsvc::cmn::kernel::ensure_vlan`, which does
   not exist (the code is `tcpcfg::apply`).
6. `ADPTADR` and an aggregated line change the adapter's address: QS/VS
   records such a change and does not pass it on (`docs/NETWORK.md`),
   so a line with `ADPTADR` loses its traffic. QS/400 should be told,
   in its documentation for partitions, that the profile's address is
   the one a partition has.

NEEDS-CODY: whether any of 1 to 6 goes to QS/400 now, or waits for
QS/MC; none of it blocks `net-qs400`.

## The service channel (Phase 7, at `5ce33f1`)

QS/400's handoff 17 is the system's side of the channel: `qsid` in a
partition and the console as a display on the console port
(`docs/QSID.md` there). What runs against `qsvmm`'s virtio-console and
the partition manager showed, at the pin, with the guest image
`qs400-V1R1M0-626e2753f892.img`:

- `qsid` found both ports by their names in sysfs, said hello about
  four seconds after the VMM started on the build host (the serial
  number the host gave, slot A, keylock normal), then `C1001000` and
  every code of the IPL in order to `C6001090` and the state
  `complete`, at about twenty seconds. It answered every ping.
- The console port carried a whole frame of 24 by 80 when the viewer
  connected, the SRC display's codes during the IPL, then the sign-on
  display, which matches QS/400's own golden screen.
- An interface added and started on the serial service console
  (`ADDTCPIFC ... LIND(*VIRTUALIP)`, `STRTCPIFC`) came as a new state
  with its address, `10.9.9.9/32`, and a link-local address.
- `dbgkill JOB(*STG)` on the serial service console posted `B1001060`
  and then the state `down`, and `slic` halted without powering off:
  the host held the partition and dumped its memory (4 GiB in 11 s,
  568 MB written).
- The fix level was empty: the image has no cumulative package.

What QS/400 would have to change, found so far:

1. **Say in the LIC record that the system has a service channel.**
   The host cannot tell a system that has not opened the port yet from
   one that never will, so it waits 30 seconds for the second
   (`docs/OPEN_QUESTIONS.md` item 38). A TLV the IPL loader reads would
   end the wait. Done in QS/400's handoff 18 (TLV `0x010C`), read since
   Phase 8.
2. **The hello's `ipl_type`.** QS/400's `docs/QSID.md` gives it as `A`,
   `B` or `D`, which are IPL sources, and the hello sent `A` beside
   `slot = A` for an unattended IPL from side A. Whether the IPL was
   attended is not in it. The host keeps the field and reads nothing
   from it.
3. **The words of a code.** Functions 12 to 19 show `00000000`:
   QS/400's codes carry no words yet (`docs/QSID.md`, the `refcode`
   message). Nothing on the host's side changes when they do.
4. **A way to stop `qsid` alone.** The suite of a system that stops
   answering stops the whole VMM, because nothing of QS/400 stops its
   service daemon from outside (`dbgkill` takes the storage service's
   jobs only). A `dbgkill JOB(*QSID)` would let the suite stop the
   daemon alone.
5. **The console port for an installation from media.** `slic` takes
   its console mode from the LIC record alone
   (`lic/slic/src/ipl/display.rs`), and a blank disk has none, so its
   installer, IPLed from source D, uses the default, the local display.
   A partition has none: `A6005008`, and the installer's screens are
   on the serial port alone. The installed system, IPLed again from the
   load source, finds the mode the host wrote to the new record and is
   on the console port. With no record and the console port present,
   the installer should take the port; a kernel command-line key the
   IPL loader passes for source D would do as well.
   (`service-install-console` checks what there is.)
6. **Keylock after an automatic install.** After the installer's own
   re-IPL, the system said its keylock was normal though the host wrote
   manual to the record: `slic` logged `mode Normal from LIC override`,
   the install's override for its next IPL. The host says so
   (`A2009030`) and the partition runs on; the override should say the
   host's keylock or be cleared when the host writes one.
7. **Two faults met in the suites.** Under load on the build host, two
   QS/400 IPLs failed inside the guest: `memory allocation of
   3468972977995967440 bytes failed` once storage management had
   started, then a triple fault; and a page fault with reserved bits
   (`Bad pagetable: 000d`) after `C6001090`, then a panic in a standard
   channel and kernel oopses. Both were three levels of virtualization
   deep while another QEMU workload ran in the same WSL machine; every
   other IPL of the same image in the suites was clean. The host held
   the first as a guest fault (`A2005170`), as designed. Not known to
   be QS/400's.

## Saving and restoring on tape (Phase 10, at `2e950f9`)

Section 1 of Phase 10: QS/400 against the virtual tape drive of Phase 5
(`docs/VMM.md`, "The tape device"), every command typed at the system's
Command Entry and every volume mounted and unmounted from the host, as
an operator would (`pio-tape-save`, `pio-tape-span`, `pio-tape-volumes`
in `tools/conformance/src/suite/physio/tape.rs`). Two installed QS/400
partitions from QS/400's ISO at the pin, the volumes in the media
repository's pool, three levels deep (WSL, QEMU, the partitions), on
2026-11-17.

**What works.**

- `INZTAP`, `SAVLIB` of a library of six object types (`*DTAARA`,
  `*DTAQ`, `*JOBD`, `*MSGQ`, `*FILE`, `*SAVF`), `DSPTAP`; the volume
  unmounted by the host, mounted on the second partition's drive, and
  `RSTLIB` there: `CPC3722 6 objects saved from library TAPLIB`,
  `CPC3703 6 objects restored to TAPLIB, 0 not restored`, every object
  and the data area's value the same.
- A save past the end of a 16 MiB volume: QS/400 takes the early
  warning, ends the volume, unloads it (17 s into the save) and asks
  for the next (`CPA4075`, answered from the reply list); the host
  mounts the second volume and the save ends with `CPC3722`. `RSTLIB`
  with both volumes named reads across them the same way and ends with
  `CPC3703`.
- A write-protected volume: `SAVLIB` and `INZTAP` are refused with
  `CPF6768`; `DSPTAP` reads its labels.
- A volume with data and no labels: `DSPTAP` says `*NL`, `SAVLIB` is
  refused with `CPF6772`, `INZTAP CHECK(*YES)` initializes it.
- A volume labelled in ASCII by another system: `DSPTAP` reads its
  labels (`*SL`, `*ASCII`, its volume ID).

Nothing of the virtual drive needed changing for any of it.

**Findings** (T1 to T4, for a QS/400 handoff; each also in "What QS/400
would have to change" below):

- **T1. The messages sent are not the catalogue's.** `INZTAP` sends
  `CPC6773 Volume PRT001 initialized on device TAP01.`; the catalogue
  (`runtime/msgf/msgs/qcpfmsg.toml`) has `Volume &1 is initialized on
  device &2.`. `RSTLIB` sends `CPC3703 6 objects restored to TAPLIB, 0
  not restored`; the catalogue has `&1 objects restored to library
  &2.`. A protected volume gives `CPF6768 the volume is write
  protected`; the catalogue has its own words. `CRTLIB` answers with an
  object line, `QSYS/TAPLIB *LIB created as 0001-0001-…`, not `CPC2102
  Library &1 created.`. A program that monitors for the message's text
  or its data sees something other than the catalogue promises. The
  suites accept both.
- **T2. A blank volume is a device error.** On a volume never written,
  `DSPTAP`, `SAVLIB` and `INZTAP CHECK(*YES)` all end with `CPF6708
  device error: I/O error (os error 5)`. The drive answers the first
  read as SSC has it for a medium with no data, CHECK CONDITION, BLANK
  CHECK, end of data (`00/05`), which Linux's `st` gives as `EIO`; a
  real drive with a new cartridge answers the same way. `INZTAP
  CHECK(*NO)` initializes such a volume, so new cartridges can be used,
  but `CHECK(*YES)` is the default and the operator's first command.
  Expected: a blank volume is one with no labels (`*NL` at `DSPTAP`,
  `CPF6772` at a save, and `INZTAP CHECK(*YES)` initializes it).
- **T3. A save to a volume of another system's labels saves nothing.**
  `SAVLIB` to the ASCII-labelled volume ends with `CPF3770 0 objects
  saved from library SMALL`, not a refusal that names the volume's
  labels.
- **T4. `INZTAP CHECK(*YES)` of a volume labelled in ASCII ends the
  system.** QS/400's storage service is lost (`qsjob: storage service
  lost` from every job), its IFS's xfs on `ublkb0` shuts down, `init`
  exits and the kernel panics; the partition ends for a restart (exit
  10) and QS/400 IPLs again. After that IPL it said `0 media device(s)
  configured`, where it had 1 before, though the partition was the same
  machine with the volume still loaded in its tape device. Twice in two
  runs. `NEEDS-CODY`: QS/400's to find (its `tape::volume::inz`
  `identify` and `scan_files` on a volume whose labels are ASCII); a
  foreign volume should be refused, never end the system.

`pio-tape-volumes` checks the ASCII volume last, so every other
outcome is recorded first, and fails there by T4; `pio-tape-save` and
`pio-tape-span` pass.

## What QS/400 would have to change

For a QS/400 handoff, in the order of how much a partition needs each.
Items 1 to 6 were done at `e035bea`; what is said under each is how it
was at the commit pinned for Phase 3.

1. **Record the side of the next IPL in the LIC record when there is no
   firmware to tell.** `licpart` gains, in the header or in a TLV of
   its own, the slot to IPL from next, once (what `BootNext` is), and
   the default slot (what `BootOrder` is). `slic`'s `ptf::slot::Firmware`
   has a third implementation beside `EfiFirmware` and `NoFirmware`:
   one that writes those two through `licpart`, used when
   `/sys/firmware/efi` is absent and the CPU's hypervisor flag is set
   (or when QS/VS is identified, item 4). `IPLSRC`, a pending LIC PTF, a
   LIC install's `boot_default` and `register_install` then all work
   unchanged above it. The hypervisor reads the once-only slot, clears
   it, and falls back to the default: `vmm/ipl` would read both through
   `licpart` and `side = "recorded"` would mean what it says.
2. **Own the console mode in `licpart`.** The TLV tag `0x0105` and its
   two values (1 local, 2 serial) are in `svctools`
   (`dst/console_mode.rs`), which QS/VS may not depend on. QS/VS writes
   the mode through `licpart::Record::set_tlv` with the tag and the
   byte restated (`vmm/ipl/src/lic.rs`): the one place it knows
   something of QS/400 that it does not get from `licpart`. Moving the
   tag and the encoding into `licpart`, beside `ipla`, removes it.
3. **`QSHWQUAL` in a partition.** The UEFI check should pass when the
   system is a partition of QS/VS: there is no firmware by design and
   the hypervisor loads the UKI. The guest agent check should name the
   channel QS/VS will give (Phase 7) when the hypervisor is QS/VS.
4. **Identify QS/VS.** With no DMI, `hwqual::checks::hypervisor` says
   `unidentified hypervisor`. QS/VS can be known by its ACPI tables:
   the OEM id `QUARTZ` and OEM table id `QSVSPART` of every table, and
   `QUARTZVS` in the FADT's hypervisor vendor identity. Whether QS/VS
   should give a partition SMBIOS tables instead is a question for
   Phase 4 (`docs/OPEN_QUESTIONS.md`).
5. **A serial number for the partition.** `QSRLNBR` needs a source that
   is not DMI: the same answer as item 4, whichever it is.
6. **A resource type for virtio-rng**, so it is not `OTH01`.
7. **Nothing for the console.** Serial mode works as it is. A
   partition's profile should set it; the display path's `A6005008` is
   then never seen.
8. **Restart by the ACPI reset register in a partition**, if the seven
   bytes `qsvmm` keeps at the reset vector are not wanted
   (`docs/OPEN_QUESTIONS.md` item 29): `reboot=acpi` in the baked
   command line does it, and is harmless on a machine with firmware
   only if its reset register works.

Done at `e035bea`: 1 (`licpart::IplSource`, `partfw.rs`), 2
(`licpart::ConsoleMode`; QS/VS restates nothing, and takes the sides'
paths from `diskimg::SLOT_A_UKI` and `SLOT_B_UKI`), 3 and 4 (`QSHWQUAL`
names QS/VS and passes the firmware check), 5 (the serial number, from
the SMBIOS tables QS/VS writes since Phase 4), 6 (the entropy device
has a type of its own, `QX01`: `lic/hwrsrc/src/scan/pci.rs` there). 7 needs nothing. 8 stays as it is: the seven
bytes at the reset vector are kept (`docs/OPEN_QUESTIONS.md` item 29).

A guest agent for the service channel and a partition's reference
codes sent to the host came with QS/400's handoff 17 (above, "The
service channel"), which Phase 7 pinned.


For a QS/400 handoff after handoff 20, from Phase 10's Section 1
(above, "Saving and restoring on tape"):

9. **T4 first: a foreign volume must not end the system.** `INZTAP
   CHECK(*YES)` of a volume with ASCII labels loses the storage service
   and restarts the system. `NEEDS-CODY`.
10. **T2: a blank volume is a volume with no labels**, not a device
    error: `EIO` from `st` on the first read at the beginning of the
    volume, after `MTREW`, with the drive's sense BLANK CHECK, is the
    end of data.
11. **T3: a save to another system's labelled volume is refused** by a
    message that names the labels, before anything is written.
12. **T1: each message sent is its catalogue entry**, text and data:
    `CPC6773`, `CPC3703`, `CPF6768`, and `CRTLIB`'s `CPC2102`.
