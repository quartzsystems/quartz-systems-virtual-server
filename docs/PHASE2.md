# Phase 2: the installer, the ISO and Service Tools

What Phase 2 built, how to run it, and the evidence for the acceptance
table. Phases 0 and 1 are `docs/PHASE0.md` and `docs/PHASE1.md`; the
host prerequisites and the environment variables there still hold.

Phase 2 makes QS/VS installable on a blank machine and configurable by
a person: a text-screen installer on the ISO, software RAID on the
load-source disks, and the Service Tools console in place of Phase 1's
status screen. There is no VMM, storage pool or virtual Ethernet code.

## The pin

Every QS/400 crate is at

```
03d6492913318da96bc707cb68d3b54a94ef9bff
```

which was `main` on 2026-11-12 and holds QS/400's handoff 14. The
prerequisite was checked before anything else: `docs/BUILDKIT.md`
there (section 5, "Beside the pipeline", and section 10) names
`console` without QS/400's identity, the screen kit `scrkit`, the
users `svcusers`, the protection crate `protect`, `ublk` serving any
block device with recovery, and `netlink` taking its route protocol
from the caller. All six are there.

The move was commit `b47a115`, alone: the `rev =` values, `Cargo.lock`,
the record in `docs/PHASE0.md`, and the one call that no longer
compiled (`netlink::route::Route::new` takes the protocol; `netd`
passed it instead of setting the field afterwards, which is the handoff's
own decision for `netlink`). Before anything else was written, clippy,
the unit tests, `doctor`, `deny`, both image builds and all twelve
steps of `build test` (Phase 0's two and Phase 1's ten) passed at the
new pin. `Cargo.lock` in that commit is one line ahead of its
`Cargo.toml` (a dependency of the commit after it), so that one commit
does not build with `--locked`; history is not rewritten here.

`Cargo.toml` now has fourteen `rev =` values, all that commit:
`buildkit`, `vrm`, `diskimg`, `abrec`, `ipc`, `netlink`, `dhcp`, `ntp`
as before, and `blockdev`, `protect`, `ublk`, `console`, `scrkit`,
`svcusers`. No QS/400 source is in this repository. One function is
bound to be the same call as QS/400 has in its own installer: the
`BLKRRPART` ioctl that has the kernel read a partition table again
(`lic/hostdisk/src/sys.rs`, six lines); no neutral crate offers it.

| Crate | Used for | From its caller |
|---|---|---|
| `console` (without `session`) | the 24x80 grid on the framebuffer and, through `AnsiDisplay`, on the serial port; the field editor; the keys | the banner; no ready code |
| `scrkit` | the screen builders and the panel stack (`Stack`, `Flow`). The panels are QS/VS's own trait over them, for help and for screens that move | every title, the system name, the keys |
| `svcusers` | the users, their Argon2id hashes, the file they are kept in | the users. Its three-failures rule is not used: `base/users` counts five and disables for five minutes |
| `protect` | `MirrorSet`: the mirrored load source as one block device, its resume | the members and their states (the set's label), the resume map's keeper |
| `ublk` | the set as `/dev/ublkbN`, with recovery | the target |
| `blockdev` | `LinuxBlockDev` over a data partition | |

## What exists

### The load source (`lic/hostdisk`, `lic/qsvsstgd`)

`docs/ARCHITECTURE.md` has the layout and the reasons.

- `lic/hostdisk`: the load-source disks. The layout as an installer
  writes it; a GPT reader; every load-source disk of a running host
  with its ESP, record and data partitions (`disks`); one record kept on
  every disk, the highest sequence winning and a write going to all
  with one sequence (`multi`, over QS/400's `abrec`); the A/B record on
  that; the mirrored set's label and the resume map beside it (`set`);
  the firmware's boot entries for each slot of each disk (`boot`, which
  was PID 1's and is now shared with the installer and the storage
  service); writing a disk (`prepare`); the media's payload manifest
  (`payload`).
- `lic/qsvsstgd`, the storage service, first in PID 1's table. With an
  unprotected load source it names the data partition and answers
  status. With a mirrored one it assembles the set from the label,
  serves it through `ublk` with recovery, and takes the device over
  when it is a restarted server (the device's number is in `/run`);
  keeps the label level with the members; resynchronizes a member that
  is stale, a megabyte at a time and giving way to the host's own I/O;
  prepares a disk that is added or put in a member's place (the table,
  a copy of the ESP, the records, the boot entries) and rebuilds it;
  marks the label clean when it is stopped in order. A transfer that
  changed a member's state does not complete until the label says so
  on the disks that are left (`serve/guard.rs`).
- PID 1 starts the table with everything after the storage service held
  back, turns its loop until that service is ready or will not be,
  mounts `/data` from what the service names, and then starts the rest
  (`base/supervise`: `hold_after`, `release`). At a shutdown the
  services above it are ended, `/data` is unmounted, and then the
  storage service is ended (`begin_stop_above`).
- New reference codes: `C2003010` set assembled, `C2003020` and
  `C2003025` resynchronization, `C2003030` and `C2003035` rebuild,
  `C2003040` protection started, `A2003010` set not assembled,
  `A2003020` unit failed (at every IPL while it lasts, and when it
  happens, with the unit's number, location code and serial),
  `A2003030` rebuild not finished, and `B2003000`
  (`docs/REFCODES.md`).

### The installer (`base/install`, `lic/qsvsinstall`)

`docs/INSTALL.md` is the operator's guide.

- `base/install`: the nine screens, the check of each field where it is
  typed, the answers of an unattended installation held against the
  machine, and the `Target` trait the product carries the steps out
  through. Product-neutral.
- `lic/qsvsinstall`: QS/VS's half. It finds the media among the block
  devices, mounts it, verifies the payload (`media`); lists the
  machine's disks and links from the inventory; and carries out the
  eight steps (`target`).
- PID 1 on the media UKI (`qsvs.install=1`) mounts the kernel's file
  systems, sets the clock, and runs the installer in place of the
  service table; when it ends, the machine restarts.
- `base/seed` reads the `[install]` table (`docs/SEED.md`).
- `base/identity`: the host's Ed25519 key pair and self-signed
  certificate, made by PID 1 at the first IPL that has a `/data`
  (`C2001075`), once.

### Service Tools (`base/panels`, `base/users`, `svc/tools`)

`docs/SERVICETOOLS.md` has every panel.

- `base/panels`: a panel with help (F1) and a tick over the screen
  kit's stack; the console on the framebuffer and on a serial terminal
  and the loop that runs a stack of panels on it, with the idle time;
  the forms the installer and the console share (network, time, user);
  and the panels every service console has (sign-on, network with its
  trial, users, date and time, logs, power).
- `base/users`: the password rules and the sign-on rule over
  `svcusers`.
- `svc/tools`, the service `tools`: QS/VS's main menu, system status and
  disk units, and the two sessions. The display shows the status screen
  until Enter; the serial port is PID 1's, as in Phase 1, until Enter
  is pressed there.
- `base/netd`: a network change is put on trial, kept when the
  interface has carrier and an address or the operator confirms, and
  taken back after 30 seconds otherwise (`trial`); the clock set by
  hand.
- PID 1's control socket: `set_host` writes `host.toml` and takes the
  name and the zone into use.

### Build and media

- `build iso`: the media UKI as the boot image, and beside it the
  payload: both slot UKIs, the manifest of them, its signature by the
  build's key (`tools/build/src/product/media.rs`). The ISO is
  reproducible and is listed in the build's artifact manifest.
- The media UKI's initramfs has PID 1, the installer and the trusted
  keys; a slot's has PID 1 and the services. So `initramfs.cpio` and
  `initramfs-media.cpio` differ now.
- The conformance harness writes answers files
  (`tools/conformance/src/suite/install/mod.rs`, `answers`).
- The test image is as before: `qsvshook`, PID 1 under
  `cfg(test_hooks)`, and now Service Tools under it too, for one hook:
  `qsvs.hook=short-idle` makes the idle sign-off fifteen seconds. The
  channel has three commands more: the storage service's status, a
  write load under `/data`, and a file's length and sum.

### Conformance

The harness drives a session and reads screens: it types at the serial
port (QEMU's standard input) and at the display's keyboard (`sendkey`
through the monitor), and keeps the serial port as a 24x80 grid
(`qemu/term.rs`). Golden files for every installer and Service Tools
screen are under `tools/conformance/golden/` (`qemu/golden.rs`): the
build ID, dates and times, counts of seconds and three rows whose
value is of the moment or of the machine are masked on both sides.
`QSVS_GOLDEN=update` writes them.

An installation takes a quarter of a minute, so the two that the later
suites start from (one disk; two mirrored and in step) are made once
for a build's ISO and kept under the harness's work directory; each
suite works on copies.

## Kernel additions

Added to `kernel/config/qsvs.config` and to the symbols the final
`.config` is checked for (123 now):

| Symbol | Why |
|---|---|
| `ISO9660_FS` | the installer mounts the media to read its payload |
| `BLK_DEV_SR` | the media as a CD-ROM: a real drive, QEMU's, or a BMC's virtual media |
| `USB_STORAGE` | the media on a USB stick, and a BMC's virtual media, which is a USB device |
| `USB_UAS` | USB storage that speaks UAS |

What else Phase 2 uses was Phase 0's already and is checked as before:
`BLK_DEV_UBLK` and `IO_URING` for the set's device; `FB`,
`DRM_SIMPLEDRM` and `DRM_FBDEV_EMULATION` for the console's
framebuffer; `INPUT_EVDEV`, `KEYBOARD_ATKBD` and `USB_HID` for its
keyboards.

## Running

```
cargo run -p build -- all
cargo run -p build -- all --test-image
cargo run -p build -- test
cargo run -p build -- test --suite list
cargo run -p build -- test --suite install-serial
QSVS_GOLDEN=update cargo run -p build -- test --suite install-serial
cargo run -p build -- test --media iso
```

`test` runs Phase 0's suite on the release image, Phase 1's ten on the
test image, and then these, which install from the test image's ISO
onto blank disks:

| # | Suite | What it asserts |
|---|---|---|
| 1 | `install-serial` | an attended installation on one blank disk, typed at the serial port; every screen of the installer matches its golden file, and one help screen; restart; base ready on the installed host with the name, the address and the slot asked for; sign-on; the panels of Service Tools the other suites do not visit match theirs; options 4 and 8 refuse and say what brings them; option 6 sets the display zone and the clock by hand; sign-off gives the port back; signed on again, option 9 powers the host off |
| 2 | `install-display` | the same answers typed at the display's keyboard through QEMU's monitor; judged by what was installed; then a sign-on typed at the display, and one at the serial port beside it: two sessions |
| 3 | `install-unattended` | no key pressed: an answers file, two blank disks, the ISO as a USB stick; from the host's side both disks hold both UKIs, a valid record and the set's label; restart; `/data` on the protected set; the identity made (`C2001075`, the key and the certificate); the second unit synchronized; power off; both labels clean |
| 4 | `mirror-degraded` | a mirrored host with unit 2's disk removed, then with unit 1's: it IPLs, runs, posts `A2003020` with the unit's location code, and Service Tools shows the unit Missing at that location |
| 5 | `mirror-replace` | unit 2 missing and a blank disk present: replaced from Service Tools option 2, the rebuild watched to its end; then the original disk removed and the host IPLs from the new one with what was on `/data` |
| 6 | `install-power-pull` | QEMU killed at 20 random moments of a mirrored installation; the installer starts over each time and never stops on what it finds; the installation then let finish is good |
| 7 | `mirror-power-pull` | QEMU killed at 20 random moments of a running mirrored host while the state files churn; at every IPL `/data` mounts from the set, every state file is whole, and a resynchronization starts; the last runs to its end |
| 8 | `storage-kill` | the storage service killed three times while files are written to `/data`: started again, the device taken over, the writes go on, the files read back whole, no resynchronization |
| 9 | `tools-network` | from Service Tools: DHCP to a static address and back, each kept and written in the same `netd` process; a change to a VLAN nothing answers on is taken back after 30 s and `network.toml` is untouched |
| 10 | `tools-sign-on` | a wrong password and an unknown user ID refused alike; a user added, one removed, the last one kept; the idle sign-off; five wrong passwords disable an ID and the right one is then refused; every refusal logged; no password in a log, the users' file or the serial output |
| 11 | `media-refused` | an ISO whose manifest was changed after it was signed is refused on a screen that says so; every byte of the disk is still zero |
| 12 | `answers-invalid` | answers that name a disk the machine lacks, and answers with an unknown key: the installer stops on a screen that says what is wrong; the disk is untouched |
| 13 | `installed-base` | Phase 0's suite on a disk the release ISO's installer wrote (slot A, and slot B by a boot entry), and Phase 1's ten on a disk the test ISO's installer wrote |
| | `mirror-start` | beyond the thirteen: mirrored protection started on an unprotected load source from Service Tools; prepared while the host runs; at the next IPL the set is assembled and the new unit written |

Two of Phase 1's suites seed an unconfigured host (`static-vlan`,
`time`). An installed host is configured and reads no seed
(`docs/OPEN_QUESTIONS.md` item 10), so in suite 13 the same
configuration is given to the installer as answers and the suite goes
on from there.

`test --media iso` boots the release ISO: PID 1's lines, then the
installer's first screen. In Phase 0 and 1 it ended at two heartbeats;
the media no longer runs the service table.

## Acceptance evidence

**The acceptance run did not complete.** It was started on 2026-11-12
in a fresh `git clone` on ext4 at commit `cf15e92`, as Phase 1's was:
two release builds each from an empty target, the test image, the
checks, `build test`, `build test --media iso`. The first release build
finished (497 s). During the second, the Windows volume that holds the
WSL virtual disk ran out of space and WSL failed with I/O errors. So
two rows of the table have no evidence yet, and say so. Everything
else below was measured the same day on the same host, suite by suite,
from the working tree, on the host of `docs/PHASE1.md` (WSL Ubuntu,
Intel, `kvm_intel` nested = Y; QEMU 10.2.1 and cargo-deny 0.20.2 by
`doctor`).

| # | Check | Result |
|---|---|---|
| 1 | Suites 1 to 13 pass under `build test` | **not shown in one run.** Each of the thirteen passed by itself with `build test --suite <name>` on the final code or within two commits of it (below); the whole of `build test` was last run at the pin move, before Phase 2's code |
| 2 | Two clean builds produce byte-identical UKIs, disk image and ISO | **not measured.** The second clean build did not finish. Nothing in the build is known to vary: the payload's manifest is text with the fixed build ID, and an ed25519 signature is a function of the key and the text |
| 3 | `build deny` and `build doctor` pass, including the license-header check | pass, at `746ef13`: `cargo deny check`, then 195 crates compiled for the host and for musl with no `links` key and no C build helper; `doctor` all rows ok, 172 `.rs` files start with the header |
| 4 | The `base/` neutrality test passes with the installer flow, common panels and users code under `base/` | pass: `doctor`'s row, 10 crates name no product and depend on nothing of `lic/` (`appliance`, `state`, `supervise`, `logd`, `netd`, `seed`, `users`, `panels`, `install`, `identity`), and the unit test `host::neutral` in the 255 |
| 5 | Every QS/400 crate is pinned at the one new commit; no QS/400 source is copied | pass: fourteen `rev =` values in `Cargo.toml`, all `03d64929`; `deny.toml` allows that one git source. One ioctl wrapper is the same call as QS/400's (above) |
| 6 | No listening socket exists on the installed system | pass: Phase 1's `sockets` suite on a disk the installer wrote, in suite 13: no TCP socket listens, and no socket is bound off loopback but the DHCP client's |
| 7 | No password appears in any log, screen capture or state file in clear | pass: suite 10 reads the system log, the reference code history, the users' file and the serial port's whole output for the four passwords it typed; suite 1 does the same for its serial output; none of the 39 golden files has one |

Also clean at the last code change (`cf15e92`): `cargo fmt --all`,
`cargo clippy --workspace --all-targets -- -D warnings`, the same for
the seven image binaries on the musl target, for PID 1 and Service
Tools with `QSVS_TEST_HOOKS=1` (at `746ef13`), and `cargo test
--workspace` (255 tests). `build notices` was written at `746ef13`.

### The suites, one at a time

Each line is the last run of that suite. "Final" is the code of
`cf15e92`; the storage service changed once after the runs marked
`746ef13` (it waits for a set member that is late to appear), and
suites 4 and 13 ran after that change.

| # | Suite | Code | Time | What was seen |
|---|---|---|---|---|
| 1 | `install-serial` | `66136b8` | 30.5 s | 10 screens of the installer and 11 panels of Service Tools matched their golden files; the installed host `vs-test` at `10.0.2.15/24` from slot A; option 6 set `2030-01-02 03:04 UTC+02:00`; option 9: `REF D2001000 Host shutdown started: power off, by the control socket` |
| 2 | `install-display` | `66136b8` | 31.3 s | 59 keys through the monitor installed the same host; `ADMIN` signed on at the display and, beside it, at the serial port |
| 3 | `install-unattended` | `66136b8` | 30.9 s | from the ISO as a USB stick; both disks hold both UKIs (13 390 848 bytes each), record sequence 1, the label; `/data` on `/dev/ublkb0`; the identity made; both units Active; powered off clean |
| 4 | `mirror-degraded` | final | 52.2 s | `REF A2003020 Load-source unit failed: unit 2 at P0000-B00-D11-F0`, and the same for unit 1 at `P0000-B00-D10-F0`; the unit Missing in Service Tools |
| 5 | `mirror-replace` | `746ef13` | 32.4 s | rebuilt to `Both units are in step`; `C2003030` and `C2003035` in the history; the host IPLs from the new disk alone and reads what was written before |
| 6 | `install-power-pull` | `b01daf1` | 191.8 s | 20 cuts (seed 388535741): 4 before the first step, 11 spread over 0 to 8 steps done, 5 after the last; the installer started over each time; the last installation good |
| 7 | `mirror-power-pull` | `746ef13` | 320.9 s | 20 cuts (seed 969018185): `/data` from the set at every IPL, a resynchronization started after each, the last to its end; `host.toml` generation 32 to 3108; both labels clean |
| 8 | `storage-kill` | `746ef13` | 36.3 s | killed three times (process 79 to 110, to 115, to 120) under 4 MiB writes; 38 files written, the last 3 read back whole; no resynchronization |
| 9 | `tools-network` | `746ef13` | 39.1 s | DHCP to static and back, kept at once, the same `netd` process; VLAN 100 taken back after 32 s, `network.toml` untouched, `eth0.100` gone |
| 10 | `tools-sign-on` | `746ef13` | 42.6 s | the rules of the suite's row above; signed off after 15 s (the test image's short idle) |
| 11 | `media-refused` | `746ef13` | 5.1 s | refused; every byte of the disk zero |
| 12 | `answers-invalid` | `746ef13` | 9.2 s | stopped on both kinds of answers; the disk untouched |
| 13 | `installed-base` | final | 366.0 s | on a release installation: `IPL slot A`, two heartbeats, and `IPL slot B` by a boot entry; on a test installation: `status`, `dhcp`, `static-vlan`, `time`, `inventory`, `supervision`, `power-pull`, `slot-confirm`, `shutdown`, `sockets` |
| | `mirror-start` | `746ef13` | 27.8 s | `REF C2003040 Mirrored protection started`; at the next IPL `/data` on `/dev/ublkb0`, unit 2 written, both Active |
| | `IPL from media` | final | 2.8 s | PID 1's lines, then the installer's welcome, on the release ISO |

The time of a suite includes making the kept installation it starts
from when the ISO had changed (some 20 s).

## Found while testing

Defects the suites found in this phase's own code or in Phase 1's,
fixed before the evidence below:

- **A static address equal to the lease left the host saying it had
  none** (`netd`, Phase 1's code). Changing from DHCP to a static
  address stops the DHCP client, whose last word, that its lease is
  off, arrived after the static address was on the interface and took
  it out of the service's own state. A stopped client's word is now
  ignored (`manage::unbound`, with a unit test). Suite 9 found it.
- **PID 1 waited on `/data` in its loop.** It wrote the inventory
  there whenever the management line changed. Behind a
  resynchronization that took seconds, and with the storage service
  ended it would have taken for ever: the loop that starts that
  service again was waiting for it. Those writes are another thread's
  now, and `set_host` waits a moment only. Suite 7 found it.
- **A resynchronization starved the host's own I/O.** It copied
  sixteen megabytes with the set held between each. It now copies one,
  and stands back while the host has a transfer under way.
- **A failed unit was recorded up to a tenth of a second late.** The
  label was written by the service's loop, after the transfer that
  noticed the failure had been answered. Had power been lost in
  between, the next IPL could have taken the failed disk for the good
  one. A transfer now waits for the label (`serve/guard.rs`, with a
  unit test).
- **The harness kept an installation for a build by the ISO's name.**
  The name is the boot image's hash; the payload beside it changes
  without that. The size and time are part of the key now.
- **A code posted during a session was looked for on the serial port.**
  While a session has the port PID 1 prints nothing there; such codes
  are on the display and in the history, where suite 5 now looks.

## Not done

- `BLOCKED(phase2)`: the acceptance run in a fresh clone did not complete: the build host's disk filled. Rows 1 and 2 of the table have no evidence; run `build all` twice from empty targets and `build test` once.
- **NEEDS-CODY: the first installation on real hardware.** The
  checklist is below. Everything in this record ran under QEMU.
- **NEEDS-CODY: the QEMU job in CI** has still not run on GitHub
  (`docs/OPEN_QUESTIONS.md` item 6). It now also installs: more disk
  and some eight minutes more.
- **NEEDS-CODY: what brings menu options 4 and 8**
  (`docs/OPEN_QUESTIONS.md` item 16). The menu says `comes with QSI`
  and `comes with code update`, not a phase number.
- `BLOCKED(phase2)`: a unit that fails I/O while the host runs is unit-tested in `protect` and in `serve/guard.rs`; no suite makes a virtual disk fail under a running host, only removes one between IPLs.
- `BLOCKED(phase2)`: `B2003000` was not seen under QEMU: no suite keeps the storage service from starting five times under a mounted `/data`.
- `BLOCKED(phase2)`: a resynchronization after a power loss copies the whole data partition; a write-intent bitmap would bound it (`docs/OPEN_QUESTIONS.md` item 19).
- `BLOCKED(phase2)`: PID 1 does not turn its loop during its own IPL steps after the storage service is ready (`docs/OPEN_QUESTIONS.md` item 21).
- `BLOCKED(phase2)`: nothing writes the other slot, sets a pending slot or `BootNext`; that is the update phase, as in Phase 1.
- `BLOCKED(phase2)`: the five minutes an ID is disabled for are counted in memory; a restart of the console service ends them.
- `BLOCKED(phase2)`: the identity's key is Ed25519; the QSI phase may need another (`docs/OPEN_QUESTIONS.md` item 17).
- The installer does not offer a disk for a storage pool, a spare, or
  parity: those are the storage phase's.
- A load source that was not written by the installer (the built disk
  image) cannot be given mirrored protection: its xfs has 512-byte
  sectors and the set's device has 4096. Service Tools says so.
- The display's session and the serial port's are each one at a time:
  there is no second serial port, and no session over the network.
- Disks with 4096-byte logical sectors are not handled: the table is
  read and written at 512, as QS/400's `diskimg` does.
- Attention codes not seen under QEMU, only in unit tests or by
  reading the code: `A2003010`, `A2003030`, `A2004070`, `A2004090`
  (a step that fails; the screen for it is unit-tested), `A2001075`.

Not in Phase 2, and not started: storage pools, parity sets, hot
spares for pools, virtual disks, the VMM, virtual Ethernet, QSI and
pairing, code update.

## The first installation on real hardware (NEEDS-CODY)

On the Dell test machine. Each line is one thing to see and note;
`docs/INSTALL.md` and `docs/SERVICETOOLS.md` say what is expected.

**The media, both ways**

- [ ] The ISO as the BMC's virtual media: the firmware starts it, PID 1's
      lines appear, then the installer's welcome.
- [ ] The same ISO written whole to a USB stick: the same.
- [ ] `Verifying the installation media` ends in the welcome, not in a
      refusal. How long it takes from virtual media.
- [ ] The media's own disk is not in the list of disks to choose from.

**The display and the serial port**

- [ ] The welcome is on the display, legible, centered, at a sensible
      scale (`qsvs.console.scale` is not on the command line: note what
      the automatic scale chose).
- [ ] The same screen is on the serial port (115200 8N1) through the
      BMC's serial-over-LAN or a cable, drawn right by the terminal used.
- [ ] Keys from the keyboard and from the terminal both work: Enter,
      Tab, F1, F3, F10, F12; on the terminal also Esc 1, Esc 0, Esc =
      and Ctrl+K.

**Location codes against the chassis**

- [ ] Every disk on the disk screen has a location code; note each with
      the bay it is physically in. Codes in the slot form (`S...`) name
      the slot the firmware does; codes in the address form
      (`P....-B..-D..-F.`) are listed with what the device is.
- [ ] Every interface on the network screen likewise, with the port's
      label on the chassis; `carrier` is shown for the ports with a
      cable and for no other.
- [ ] No two devices have one code.

**An installation on one disk**

- [ ] The eight steps run to `C2004080`; note how long each takes.
- [ ] `C2004070`: the firmware took the boot entries (the firmware's
      setup shows `QS/VS slot A` and `QS/VS slot B`), or `A2004070` was
      logged and the disk starts by its default path anyway.
- [ ] After the restart: `C2001090 Base ready`, the host name and the
      management address asked for, `C2001075` once.
- [ ] The RTC kept the time across the restart.
- [ ] Sign-on on the display and on the serial port, each its own
      session; the idle sign-off after ten minutes.
- [ ] Service Tools option 1 shows `KVM usable`.

**A mirrored installation with a disk pulled**

- [ ] Two disks chosen; after the restart `C2003010`, then `C2003020`
      and `C2003025`; note how long the first synchronization takes for
      the disks' size.
- [ ] Power off, pull one disk, power on: the firmware starts the other
      by itself; `A2003020` names the pulled disk's location code, and
      it is the bay the disk came out of.
- [ ] The same with the other disk pulled instead.
- [ ] The pulled disk put back: it is resynchronized by itself.
- [ ] A new disk in its place: Service Tools option 2, option 2; the
      rebuild runs to `Both units are in step`; note how long.
- [ ] Power pulled from the running host: at the next IPL `/data`
      mounts, `C2003020` starts, and the host is usable meanwhile.

**An unattended installation**

- [ ] A `QSVSSEED` stick with an answers file naming the disks by
      serial number: the installer asks nothing and restarts by itself.
