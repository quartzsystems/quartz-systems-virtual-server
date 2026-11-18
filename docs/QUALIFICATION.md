# Qualification on real hardware

Everything of QS/VS has run under QEMU, at least two levels of
virtualization deep (WSL's Hyper-V, then QEMU; a partition is a third).
This is the list of what must be seen on a real machine before V1R0M0
is tagged: on the Dell PowerEdge test machine, by Cody (NEEDS-CODY,
every item). Each item says what to do and what passes; the document
named says the rest. The items come from each phase's "for a real
machine" list, gathered here in the order they can be done.

A release built with `build release` (`docs/RELEASING.md`) is used, not
a test image: what is qualified is what ships. A few items need a test
image, and say so.

## 1. The processor

- [ ] At the first IPL, no `A2010010` is posted and Service Tools option
      4 says nothing above its options: the processor has AVX2, ADX, AES
      and PCLMULQDQ, which QSI's TLS needs (`docs/OPEN_QUESTIONS.md`
      item 43). A host that lacks them cannot be managed by QS/MC.
- [ ] The KVM line of option 1 says KVM is usable, with no capability
      it lacks, and the host's processors are counted right with SMT on
      (both threads of a core are one processor, `docs/RESOURCES.md`).
- [ ] The IOMMU and interrupt remapping are on (`docs/PHYSICALIO.md`):
      option 10's devices can be released.

## 2. Firmware

- [ ] The Dell's firmware at the level qualified, written down: BIOS,
      iDRAC, the RAID controller in HBA mode or a plain HBA, each NIC.
- [ ] Both slots' boot entries per load-source disk are there after
      installing, and `BootOrder` follows a confirmed update
      (`docs/UPDATE.md`).
- [ ] Location codes: the firmware's SMBIOS slot records give slot-form
      codes for the PCIe slots, and no two devices share one
      (`docs/LOCATION.md`, never seen under QEMU).
- [ ] Secure Boot, custom mode, `qsvs-db.cer` enrolled in db with the
      vendor's keys kept: the installation media starts, the installed
      host starts from either slot, option 1 says `Secure Boot on`. A
      test image's unsigned UKI is refused by the firmware
      (`docs/INSTALL.md`).

## 3. Installation

- [ ] `docs/PHASE2.md`, "The first installation on real hardware": the
      ISO from the iDRAC's virtual media and from a USB stick, the
      display and the serial-over-LAN console both, attended and
      unattended, one disk and mirrored.

## 4. The load source and disks

- [ ] A mirrored load source with a unit pulled while the host runs:
      `A2003020`, its problem with the disk's location code, the host
      runs on; the disk put back and rebuilt at option 2, 2.
- [ ] The power pulled thirty times while the host writes (the iDRAC's
      power off, not an orderly one): every IPL after it mounts `/data`,
      resynchronizes by the write-intent map and posts nothing fatal
      (`docs/PHASE5.md`).
- [ ] A pool on the Dell's own disks, of each protection, a disk pulled
      from each (`docs/PHASE5.md`, `docs/PHASE9.md`).

## 5. Network

- [ ] An uplink to a switch port carrying tagged VLANs; an LACP bond
      against the real switch (`docs/PHASE6.md`).
- [ ] A 5250 session to a partition from a workstation.

## 6. Partitions

- [ ] QS/400 installed in a partition from its ISO through `qsictl`
      (`docs/PHASE8.md`), and from a real disc in the host's drive
      (`docs/PHASE5.md`).
- [ ] The faults of a partition's kernel (`docs/PHASE11.md`, Section 1):
      `build test --suite soak-faults` on a test image of this release,
      QS/400 and the cheap guest IPLed a few hundred times each, two
      levels deep; no fault, or each one read from its dump.
- [ ] A dump of a partition with tens of gigabytes of memory, timed
      (`docs/PHASE7.md`).

## 7. Physical I/O and tape

- [ ] `docs/PHASE10.md`, "For a real machine": the QLE2562 given whole
      and QS/400 saving to the VTL through it; the same drive passed
      through command by command; a network adapter given to a
      partition; the isolation groups as they are; the seconds a
      released device's host driver has it at each IPL.

## 8. Management and service

- [ ] Pairing at the Dell's console, `qsictl` from a Linux and a Windows
      workstation, every request, a console attached (`docs/PHASE8.md`).
- [ ] An update to V1R0M1 staged from `qsictl`, installed, activated and
      confirmed; another rolled back (`docs/PHASE9.md`).
- [ ] A host dump copied to a USB stick at option 8 and opened on a
      workstation.

## 9. A fortnight

- [ ] The release running fourteen days on the Dell under the day's
      script (`build test --suite soak-day` with `QSVS_SOAK_MINUTES`
      set to 20160 against a test image of the same commit, or by hand
      from `qsictl` on the release): no service restarted, no problem
      the script did not cause, nothing left behind (`docs/PHASE11.md`,
      Section 8).

## 10. Baselines

- [ ] `build test --suite baselines` on a test image of the release's
      commit on the Dell, two levels deep, and the numbers set beside
      the nested ones in `docs/PHASE11.md` (Section 9); and once more on
      the Dell with partitions dedicated real cores and pools on its
      own disks, by hand, timed from `qsictl`.

When every box is ticked, `docs/RELEASE_NOTES_V1R0M0.md` says the
hardware qualified, and the tag can be made (`docs/RELEASING.md`).
