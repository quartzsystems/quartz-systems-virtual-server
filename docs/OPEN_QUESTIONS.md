# Open questions

What Phase 0 left for a decision. Each says what was done meanwhile.

## 1. The host reference code range (NEEDS-CODY)

The host posts codes in the families `C2`, `A2`, `B2` and `D2`, with the
word `REF` before the code (`docs/REFCODES.md`). No QS/400 family ends in
`2`. This is a proposal: confirm it, or name another range, before
Phase 1 adds codes.

## 2. How slot B is chosen

The handoff asks for slot B to be booted "by the A/B record or the
firmware boot entry, whichever `buildkit`'s fixture uses". The fixture
boots one slot only and uses neither. QS/400 itself chooses its slot by
firmware boot entry (`BootNext`/`BootOrder` to a `Boot####` option for
`EFI/QS400/B.EFI`), with the A/B record holding the state around it.

Phase 0 follows QS/400: the test writes a `Boot0000` entry for
`EFI/QSVS/B.EFI` into a copy of OVMF's variable store, from the host
side, and boots with it. PID 1 does not yet read the A/B record or write
firmware variables; `EFIVAR_FS` is in the kernel for when it does. To
confirm for Phase 1: the slot is chosen by firmware boot entry, and the
record says which slot is the default and whether the last switch was
confirmed.

## 3. The A/B record's payload

The record is on disk now (`QSVSREC`, version 1, `lic/hostdisk`): four
bytes, the default slot and three reserved zeros. It is the least that
makes the partition a valid record. Phase 1 defines the real payload
(pending slot, IPL attempts, confirmation) under version 2; nothing
reads version 1 yet, so nothing has to migrate.

## 4. Kernel symbols added beyond the Phase 0 list

`kernel/config/qsvs.config` enables, past the list in the handoff:

- `VFAT_FS`, `NLS_CODEPAGE_437`, `NLS_ISO8859_1` and `EFIVAR_FS`: the
  ESP an update writes the other slot to, and the firmware boot entries.
- The console set (`VT`, `FRAMEBUFFER_CONSOLE`, `DRM_SIMPLEDRM`,
  `SERIAL_8250_CONSOLE`, keyboard and USB HID): the baked command line
  names `tty0` and `ttyS0`, and the Service Tools console is on both.
- `E1000` and `IGC`, which QS/400's fragment has with the other Intel
  drivers.
- The networking core (`INET`, `IPV6`, `UNIX`, `PACKET`).

It turns off `NETFILTER` and `EXT4_FS`, which `x86_64_defconfig` turns
on. Say if any of these should go.

Not enabled, though QS/400 has them: `SCSI_LPFC` (Emulex Fibre Channel;
the handoff names QLogic only), the Realtek and VMware NICs, the TPM,
and ISO 9660 and USB storage (the installer's, Phase 2).

## 5. The ISO's file name

The handoff gives `qsvs-V1R0M0-<hash>.iso`. `buildkit` names the ISO
after the product's vendor directory, so the file is
`QSVS-V1R0M0-<hash>.iso` (as QS/400's is `QS400-...`), and the UKIs carry
their kind: `qsvs-V1R0M0-a-<hash>.efi`, `-b-` and `-d-`. The disk image
is `qsvs-V1R0M0-<hash>.img` as asked. Changing the ISO's case is a change
to `buildkit` in QS/400, not something to work around here.

## 6. The QEMU job in CI (NEEDS-CODY)

`.github/workflows/qemu.yml` runs `build all` and both `build test`
suites on a GitHub-hosted runner, as QS/400's CI does. The suite needs
more than QS/400's: the guest itself must see VMX or SVM, so the runner
must offer nested virtualization to its guests. The job checks for that
first and fails with a message saying a self-hosted runner is needed
when it is missing. It has not run on GitHub yet (nothing is pushed);
whether the hosted runners pass, or a self-hosted one is needed, is to
be seen on the first push.

## 7. Secure Boot

The UKIs are not signed, and the manifest records no Secure Boot
certificate (`secure_boot_db` is `None`). QS/400 signs its own with
`pesign`, which is its tool; whether QS/VS shares the key and the tool
or has its own is a decision for a later phase.
