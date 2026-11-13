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
recovers its object store after the VMM is killed under it. Nothing in QS/400 blocks
a suite of Phase 3. What does not work is everything QS/400 does
through firmware variables: choosing the side of the next IPL.

## How it was run

- The guest is `qs400-V1R1M0-f50c366ffb35.img`, built by QS/400's own
  orchestrator (`cargo run -p build -- all`, `QS400_BUILD_ID=qsvs-guest`)
  from a clone at the pinned commit `03d6492913318da96bc707cb68d3b54a94ef9bff`,
  with nothing changed. `docs/PHASE3.md` says how to make it.
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

**When `IPLSRC` or a PTF asks for the other side.** It is refused, in a
line on the serial port, and the power down goes on:

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

**Does the LIC record already hold enough for the hypervisor to honour
the requested side at the next activation? No.**

- The record's header (`licpart::Header`) has `last_slot` and
  `last_good_slot`: where the last IPLs came from. It has no field for
  where the next one is to come from. On a machine that is the
  firmware's `BootNext` and `BootOrder`.
- The default slot and a pending LIC PTF's slot are in the record, in
  the TLV `TAG_LIC_SLOTS`, but its layout is the `ptf` crate's
  (`ptf::slot::SlotState`), which QS/VS does not depend on and should
  not.
- The slot `IPLSRC` names is not recorded anywhere.

So `side = "recorded"` in a profile means, for now, the side of the
last IPL that completed (`last_good_slot`); a disk that has never IPLed
starts from side A.

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

## What QS/400 would have to change

For a QS/400 handoff, in the order of how much a partition needs each.

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

None of these is needed for Phase 3. Item 1 is needed before a
partition can apply a LIC PTF or be told `IPLSRC`, which is before
Phase 4 can call a partition's IPL source its own.
