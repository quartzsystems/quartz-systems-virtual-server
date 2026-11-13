# Phase 1: PID 1 and the base system

What Phase 1 built, how to run it, and the evidence for the acceptance
table. Phase 0, the toolchain, is `docs/PHASE0.md`: the host
prerequisites, the pinned QS/400 commit and the environment variables
there still hold.

Phase 1 turns the banner-printing PID 1 into a base system: a supervised
service table, persistent state, logging, the management network, time,
the host inventory, the A/B record with slot confirmation, and an
orderly power off. There is no VMM, no installer and no Service Tools
console.

## What exists

### `base/`: shared with QS/MC, product-neutral

These crates were to live in the QS/MC repository, which is still empty.
They are written here and QS/MC will pin them from this repository
(`docs/ARCHITECTURE.md`, the base crates). Each takes an `Appliance`
from its caller; none names QS/VS or depends on `lic/`.

- `base/appliance`: the `Appliance` description (the product identifier,
  the name, the banner, the data and run directories, the host name
  prefix, the seed label, how an event is posted and a fact noted), the
  `Event`s the base crates report, civil time with the display zone, and
  the Ethernet links of sysfs.
- `base/state`: typed TOML files under `<data>/state/`, one a concern,
  each with a `schema_version`. A read refuses a version newer than the
  code knows and takes an older one through its registered migration
  steps. A write is atomic: a temporary file in the same directory,
  `fsync`, rename over the target, `fsync` of the directory. The files:
  `host.toml` (host name, time zone, NTP servers), `network.toml` (the
  device by name or MAC, DHCP or a static address with prefix and
  gateway, the VLAN ID, DNS servers), `identity.toml` (the serial number
  the host derived for itself) and `inventory.toml`. An absent file is its defaults: the host name is
  `QSVS` and the last six hex digits of the first NIC's MAC, UTC, no NTP
  servers, DHCP on the first interface with carrier.
- `base/supervise`: the fixed service table (a binary, an order, the
  services it needs, a time to become ready); the restart policy (1, 2,
  4 ... 30 s; five failures in ten minutes end the restarts); the
  supervisor, which PID 1 drives from its own loop; the control
  socket's protocol over QS/400's `ipc` (status, power off, restart,
  and a service's `post` and `note`); the readiness signal (a service
  writes one byte to a pipe it inherits as its standard input, so no
  descriptor number crosses `exec`); signals read from a descriptor. A
  service is started with no signal blocked: a child inherits its
  parent's mask, and PID 1 blocks the signals it reads from its
  descriptor, so without that `SIGTERM` could not end a service.
- `base/logd`: the log service. One NDJSON system log,
  `<data>/log/system.ndjson`, rotated at 4 MiB with all its files held
  under 64 MiB; each record has the time, the boot ID, the source, the
  severity and the message. The reference code history,
  `refcodes.ndjson`, is append-only and never rotated. The kernel log
  from `/dev/kmsg` is kept per boot, `kernel/<boot id>.log`, eight boots
  of 4 MiB at most. Records come in on a Unix datagram socket; the
  client queues them until the service takes them, so what PID 1 said
  before `/data` was mounted and the service was up arrives first and
  in order. Lines that cannot be written are held in memory and written
  when they can be. The records that arrive together are brought to the
  disk with one sync, at once when a code or an error is among them. On
  `SIGTERM` the service writes what its socket still holds and ends.
- `base/netd`: the management interface and time, over QS/400's
  `netlink`, `dhcp` and `ntp`. Link up, an optional 802.1Q
  sub-interface, DHCPv4 with renewal or a static address, the default
  route, and the name servers written to `/etc/resolv.conf` for the
  resolver. It follows carrier loss and return, and a changed
  `network.toml` on a reload request to its socket, without a restart.
  With NTP servers configured it keeps the clock in step (slewing up to
  128 ms, stepping past that) and writes the RTC back. IPv6 is
  link-local only: router advertisements are not accepted and nothing
  configures an IPv6 address. Nothing listens on the network.
- `base/seed`: `seed.toml` on a FAT volume labelled by the appliance
  (`docs/SEED.md`).

### `lic/`: QS/VS's own

- `lic/qsvsinit`, PID 1. The IPL in order, each step posting a `C2`
  code: the kernel's file systems (with efivarfs), cgroup2, the A/B
  record, `QSVSDATA` at `/data` (found by GPT type through
  `lic/hostdisk`), the clock from the RTC, the seed, the state (with
  the inventory and the host name), the services, and "Base ready" when
  every service has said so. When `/data` cannot be mounted it posts
  `A2001040` and goes on with the state directory in memory (`/data` is
  then a directory of the initramfs), so the console still says what is
  wrong; it never reformats. Then one loop over the signals, the
  control socket (`/run/qsvsinit.sock`), the power button and the
  services' pipes: it reaps, restarts, passes the services' output to
  the log, heartbeats on the serial port and redraws the status screen.
  The status screen is plain text, on the display and the serial port,
  drawn again when it changes: the product and release, the slot with
  the record, the host name, the management address or the reason
  there is none, the date and time, the current reference code and a
  line a service. It is a placeholder for the Service Tools console.
- The orderly shutdown: `D2001000`, what PID 1 has queued for the log
  handed over, the services ended in reverse order (`SIGTERM`; one that
  has not ended after 5 s is killed, and the console says so), sync,
  `/data` unmounted, `D2001090` or `D2001091`, then power off or
  restart. The log service is the last to end. It is started by the control
  socket, by `SIGTERM` to PID 1 and by the ACPI power button.
  Ctrl-Alt-Del comes to PID 1 and is ignored, with a line on the console
  (`docs/OPEN_QUESTIONS.md` item 15).
- The slot rule (`lic/qsvsinit/src/slot/`, `docs/ARCHITECTURE.md`):
  the A/B record version 2 read at IPL; a pending slot confirmed at
  "Base ready", in the record and, through efivarfs, in `BootOrder`
  (a `Boot####` entry is made for a slot that has none); a pending slot
  that did not confirm cleared on the next IPL with `A2001035`.
- `lic/qsvslogd`, `lic/qsvsnetd`: `base/logd` and `base/netd` run as
  QS/VS. A dozen lines each.
- `lic/refcode`: the table (31 codes), the code of each base event, the
  poster: the console line and the record for the history. A service
  posts through PID 1.
- `lic/hostdisk`: the A/B record version 2 and its rule as pure
  functions (`at_ipl`, `confirm`), and the partitions found on a
  running host.
- `lic/hostinv`: the inventory, read at every IPL from sysfs, procfs
  and DMI and written to `inventory.toml`: processors (sockets, cores,
  threads, model, VMX or SVM), memory (total, 2 MB and 1 GB huge
  pages), disks (name, model, serial, size, rotational, transport),
  network interfaces (name, MAC, driver, speed, carrier) and PCI
  devices (address, vendor and device IDs, class, driver), each with a
  location code (`docs/LOCATION.md`). The system serial
  number is DMI's when it has a real one; otherwise it is derived once
  from the first NIC's MAC (`M` and the twelve hex digits) and kept in
  `identity.toml`, not in `host.toml`, which only a seed or an operator
  writes. A link has no carrier or speed until it is up, which is
  the network service's doing after the IPL has taken the inventory, so
  PID 1 reads and writes the inventory again whenever the management
  line changes.
- `lic/qsvshook`: the test hooks. In a test image only.

### The two images and the test hooks

`cargo run -p build -- all` builds the release image. `all --test-image`
builds the test image: the same sources with the test hooks, under
`<target>/test-image/`, a target root of its own.

The hooks are:

| Hook | Where | Switched on |
|---|---|---|
| the channel: the harness's way to the control socket's status and to files of the running host, on the second serial port. It can also kill a service, send PID 1 `SIGTERM`, write a file and ask `netd` to reload | `qsvshook channel`, a service of the test image's table | always, in a test image |
| a service that fails on demand | `qsvshook flaky`, a service of the test image's table | `qsvs.hook=flaky-fails` |
| the state churn: `host.toml` and `network.toml` written over and over | a thread of the channel | `qsvs.hook=churn` |
| a slot that refuses to confirm | PID 1, under `cfg(test_hooks)` | `qsvs.hook=refuse-confirm` |

A switch is an SMBIOS OEM string the harness gives QEMU for one boot;
the image is the same every boot.

What keeps them out of a release image:

- PID 1's hooks are under `cfg(test_hooks)`, which its `build.rs` sets
  from `QSVS_TEST_HOOKS=1`. The userland step sets that variable itself
  for every build: `1` for a test image, `0` for a release image,
  whatever the caller's environment has.
- `qsvshook` is a package of the test image's product description only,
  so it is neither built for nor put into a release initramfs.
- After the build the userland step looks in PID 1 for the hooks'
  marker text and fails a release build that has it (and a test build
  that lacks it). The manifest step does the same again, and then reads
  the manifest back: a release manifest has `"product": "QS/VS"` and no
  `qsvshook` artifact; a test manifest has
  `"product": "QS/VS (test image)"` and lists it.

## Kernel additions

Added to `kernel/config/qsvs.config` and to the symbols the final
`.config` is checked for (`tools/build/src/product/kernel.rs`; the
check now covers 119 symbols):

| Symbol | Why |
|---|---|
| `RTC_CLASS`, `RTC_DRV_CMOS` | the clock is set from the RTC at IPL and written back after NTP |
| `RTC_INTF_SYSFS` | PID 1 reads the RTC as `/sys/class/rtc/rtc0/since_epoch` |
| `RTC_INTF_DEV` | the network service writes the RTC through `/dev/rtc0` |
| `RTC_HCTOSYS` off | one place sets the clock from the RTC: PID 1, which posts the code for it |
| `ACPI_BUTTON` | the power button as an input device; `INPUT_EVDEV` was Phase 0's |
| `DMI`, `DMIID` | the inventory: `/sys/class/dmi/id` and the raw SMBIOS table (slot designations, the system serial) |
| `SIGNALFD` | PID 1 and the log service take their signals from a descriptor |

Every one of these already had this value in Phase 0's `.config`,
through `x86_64_defconfig` and what it selects, so the kernel is
configured as it was; naming them makes the build fail if a kernel bump
changes one. Nothing `netd` or the xfs mount needed was missing:
`VLAN_8021Q`, `XFS_FS`, `VFAT_FS` (the seed volume), `EFIVAR_FS` and
`UNIX` were in Phase 0's fragment.

## The QS/400 crates reused

All at the one pinned commit (`docs/PHASE0.md`), eight `rev =` values in
`Cargo.toml`: `buildkit`, `vrm`, `diskimg`, `abrec` as before, and now
`ipc`, `netlink`, `dhcp` and `ntp`. Each of the three networking crates
was checked the way `BUILDKIT.md` section 6 checks the others: what it
depends on, and what in its code is QS/400's own.

| Crate | Depends on | As it is |
|---|---|---|
| `ipc` | `thiserror` | neutral (`BUILDKIT.md` says so): the wire codec, frames, `rpc!` |
| `netlink` | `thiserror`, `rustix` | the route socket, links, VLANs, addresses, routes and link events are neutral, and are all `netd` uses. Two things are QS/400's: `Route::new` marks routes `RTPROT_QS400`, so `netd` sets the protocol itself; and the `nft` module's table is named `qs400`, which is not used |
| `dhcp` | `thiserror`, `socket2` | neutral: the RFC 2131 client, the lease, the agent thread and its `Sink`. `qs400` appears in its comments and tests only |
| `ntp` | `thiserror`, `tracing` | neutral: the packet, the exchange, the adjustment policy (the caller gives the limits), the clock trait. The `*NTP` server half is not used on the host; the conformance harness uses it as its SNTP responder |

None had to be written again here. The QS/400 change that would make
`netlink` neutral throughout is in `docs/OPEN_QUESTIONS.md` item 13.

## Running

```
cargo run -p build -- doctor
cargo run -p build -- all
cargo run -p build -- all --test-image
cargo run -p build -- test
cargo run -p build -- test --media iso
cargo run -p build -- test --suite list
cargo run -p build -- test --suite power-pull
```

`test` runs, on the release image, Phase 0's suite (`ipl`: slot A, then
slot B by a firmware boot entry) and, on the test image, the ten suites
of the base system. Each can run alone with `--suite`. On WSL set both
`QSVS_TARGET_DIR` and `CARGO_TARGET_DIR` to a directory on ext4.

| Suite | What it asserts |
|---|---|
| `status` | every IPL step's code in order; "Base ready" within 10 s of the kernel starting under KVM; the status screen; the control socket's status |
| `dhcp` | on QEMU's user network, `10.0.2.15/24` and gateway `10.0.2.2` on the status screen and in the status; the name server in `/etc/resolv.conf` |
| `static-vlan` | the host IPLs once with no seed volume and has no `host.toml`; on the next IPL a seed with a static address and VLAN 100 is applied: the address is on `eth0.100`, which exists. Then `network.toml` is changed to DHCP and `netd` told to reload: the address changes and `eth0.100` goes, in the same process |
| `time` | an SNTP responder in the harness, three days ahead, moves the guest's clock |
| `inventory` | QEMU with 2 sockets of 2 cores, 2048 MiB, two disks with serials and one NIC with a fixed MAC: `inventory.toml` matches, the NIC's carrier included once it has its address |
| `supervision` | `netd` killed through the channel is started again and ready; with `flaky-fails`, `flaky` is stopped after five failures with `A2001080` and the host runs on |
| `power-pull` | with the churn running, QEMU is killed at 50 random moments; after each the host IPLs and `host.toml`, `network.toml`, `identity.toml` and `inventory.toml` parse and are a complete old or new version |
| `slot-confirm` | slot B booted as pending confirms: default B in the record on the disk and `BootOrder` 0001,0000 in the firmware's store. With `refuse-confirm` it does not, and the next IPL is on slot A with `A2001035` |
| `shutdown` | Ctrl-Alt-Del from QEMU's keyboard is ignored. QEMU's power button: `D2001000`, `D2001090`, QEMU ends within 30 s and no service had to be killed; the next mount of `/data` needs no xfs log recovery; the history has the shutdown. Then the other triggers: a restart through the control socket (`D2001091`) and `SIGTERM` to PID 1; and the logs hold a kernel log for each of the three boots |
| `sockets` | no TCP socket listens; no TCP or UDP socket is bound off loopback but the DHCP client's |

## Acceptance evidence

Measured on 2026-11-12 in WSL Ubuntu (kernel 6.18.33, 12 threads, Intel,
`kvm_intel` nested = Y), Rust 1.98.0, Ubuntu clang 21.1.8, QEMU 10.2.1,
cargo-deny 0.20.2, in a fresh `git clone` of this repository at commit
`023478c` on ext4, after open questions 7 to 15 were settled. One
script ran everything below in order (two release builds, the test
image, then the checks and the suites). The same run at `f41a1df`,
before those changes, passed alike.

| # | Check | Result |
|---|---|---|
| 1 | Suites 1 to 10 pass under `build test` | pass: `test` ran 12 steps in 4 m 32 s, Phase 0's two and the ten below |
| 2 | Two clean builds of the release image are byte-identical; test hooks are absent from it | pass: 13 files with the same SHA-256 in both builds (below); the hooks' marker, the `qsvshook` name and the `qsvs.hook=` switch occur nowhere in the release initramfs |
| 3 | `build deny` and `build doctor` pass, including the license-header check | pass: `cargo deny check`, then 176 crates compiled for the host and for musl with no `links` key and no C build helper; `doctor` all rows ok, 130 `.rs` files start with the header |
| 4 | The `base/` neutrality test passes | pass: `host::neutral::tests::base_is_product_neutral`, and the `doctor` row: 6 crates name no product and depend on nothing of `lic/` |
| 5 | Every QS/400 crate, including any networking crate reused, is pinned at one commit | pass: eight `rev =` values in `Cargo.toml`, all `6d5532fa`; `deny.toml` allows that one git source |
| 6 | Phase 0's suites still pass unchanged | pass: `IPL slot A`, `IPL slot B` and `IPL from media` on the release image, the same checks as in Phase 0 |

Also clean: `cargo fmt --all --check`; `cargo clippy --workspace
--all-targets -- -D warnings`; the same for the four image binaries on
the musl target, and for PID 1 with `QSVS_TEST_HOOKS=1`; `cargo test
--workspace` (190 tests); `build notices --check`.

### The suites

| Suite | Time | What was seen |
|---|---|---|
| `IPL slot A` (Phase 0) | 12.2 s | `REF C2001000` after 2.2 s, `QS/VS V1R0M0 phase1`, `IPL slot A (qsvs.slot=A)`, `KVM usable: VMX, API version 12, 1024 vCPUs at most`, two heartbeats |
| `IPL slot B` (Phase 0) | 2.5 s | the same with `IPL slot B (qsvs.slot=B)`, by a firmware boot entry |
| `IPL from media` (Phase 0) | 12.1 s | the same with `IPL from installation media (qsvs.install=1)` |
| 1 `status` | 2.2 s | base ready 0.69 s after the kernel started, under KVM; four services ready (the two base services and the test image's two) |
| 2 `dhcp` | 2.3 s | `10.0.2.15/24 on eth0, gateway 10.0.2.2`; name server `10.0.2.3` |
| 3 `static-vlan` | 7.3 s | an IPL with no seed volume left no `host.toml`; seeded on the second IPL: `192.0.2.10/24 on eth0.100, gateway 192.0.2.1`, `eth0.100` exists; reloaded: `10.0.2.15/24 on eth0`, `eth0.100` gone, the same `netd` process |
| 4 `time` | 3.4 s | a responder 259 200 s ahead moved the guest's clock 259 202 s |
| 5 `inventory` | 2.5 s | 2 sockets, 4 cores, 4 threads, VMX; 1962 MiB; disks `QSVSHD0` and `QSVSHD1`; `eth0 52:54:00:12:34:56` at `P0000-B00-D04-F0-E0`; 8 PCI devices; serial `M525400123456` (derived) |
| 6 `supervision` | 30.2 s | `netd` killed (process 87) and ready again (process 93); `flaky` stopped after 5 failures with `REF A2001080 Service stopped: flaky`; the host runs on |
| 7 `power-pull` | 184.6 s | 50 power cuts, 51 IPLs, every state file whole each time; `host.toml` went from generation 0 to 8926 |
| 8 `slot-confirm` | 12.7 s | `REF C2001095 Slot confirmed: slot B is the default; BootOrder 0001,0000`, default B and V1R0M0 in the record on the disk; refused: the next IPL on slot A with `REF A2001035 Pending slot not confirmed: slot B; this IPL is on slot A` |
| 9 `shutdown` | 9.7 s | `Ctrl-Alt-Del ignored` and no shutdown; `REF D2001000 Host shutdown started: power off, by the power button`, `REF D2001090`, powered off 0.2 s after the button; the next mount clean; 21 codes in the history; then a restart by the control socket and a power off by `SIGTERM` |
| 10 `sockets` | 2.2 s | no TCP socket at all; one UDP socket, the DHCP client's on port 68 |

### Seen by hand

Four paths the suites do not reach were booted once each under QEMU
with the test image (at `f41a1df`; not repeated since), and the serial
console read:

- `/data` that cannot be mounted (the partition's first MiB zeroed):
  `REF A2001040 Data file system not mounted: /dev/vda3: Invalid
  argument (os error 22); state is in memory only`, then the IPL goes on
  to `REF C2001080 Services started`.
- A seed volume whose `seed.toml` has `vlan = 0`: `REF A2001060 Seed
  not valid: seed.toml: [network]: 0 is not a VLAN ID (1 to 4094)`, and
  the host comes up on its defaults.
- No NIC: `REF A2002005 No management interface: no Ethernet link with
  carrier`, 30 s after the network service started.
- The NIC's link taken away and given back (`set_link` in QEMU's
  monitor): `Management none: no carrier on eth0`, `REF A2002010 No
  carrier on the management interface: eth0` after 30 s, then on its
  return `REF C2002010` and `REF C2002020 Management address acquired`
  again, with `netd` not restarted.

### Reproducibility

Two builds, each from an empty target directory (nothing kept, the
kernel tarball fetched again), in the same checkout, with:

```
QSVS_BUILD_ID=phase1 QSVS_SIGNING_KEY=<the same key file> cargo run -p build -- all
```

7 m 4 s and 6 m 57 s. Every file below had the same SHA-256 in both
builds (`diff` of the two lists is empty). What the hashes depend on
besides the sources is as in Phase 0: the signing key, the checkout
path, and the commit (in `manifest.json` alone).

```
65ee19703e9fe1e757ae70406d51be427e12072530aceeace96c4ccadb1059f9  images/initramfs.cpio                  (2498948 bytes)
65ee19703e9fe1e757ae70406d51be427e12072530aceeace96c4ccadb1059f9  images/initramfs-media.cpio            (2498948 bytes)
7824c4c3114cfc677182f05e95ca9c861863a7bbf1a556564ae8ed1e3d688e6e  images/qsvs-V1R0M0-a-7824c4c3114c.efi  (12837888 bytes)
531a6a4ffc5639119f4ea7c9644a857424b2a01f23b85d099f38aac37035f8ad  images/qsvs-V1R0M0-b-531a6a4ffc56.efi  (12837888 bytes)
269bd9f307e4c05f2658174b57cec1fc40292b6511bd9d823b20464f764cda4d  images/qsvs-V1R0M0-d-269bd9f307e4.efi  (12841984 bytes)
3c4f9f1667de9c38f576a5960d369b4694a5337becac52a7203a3b3e6e478afe  images/qsvs-V1R0M0-7824c4c3114c.img    (1073741824 bytes)
1c6a926b90ff2ac1c1dec79da73e54e1f26c18f7b060ab72f20b0eabcda96006  images/QSVS-V1R0M0-269bd9f307e4.iso    (45430784 bytes)
8ad754d58644b600d98bddc2c3f2db8a12eb9b017a19e7a5a756a36c4f0499e2  x86_64-unknown-linux-musl/release/qsvsinit (1109720 bytes)
5f09dd00e55707e9c987079dce0dca86ae0a9e034a4f481c3deb4573be80f6c4  x86_64-unknown-linux-musl/release/qsvslogd (445528 bytes)
bdb26d153fd5244e971fccae172756b3c8dd7b98129007fc3866bc5c3a070ef4  x86_64-unknown-linux-musl/release/qsvsnetd (941864 bytes)
a6a7eebcfea08b40d3f03cb8a2be2e60683e3f3c1e42e1940c1d53a4d2051150  kernel/build/.config
af9d91bb0eb9dfe29320834754a9c0e001e7d1e42334f57b920ab68f7072a9be  images/manifest.json
c0cdfd378bfdcbfa3ffb44eb20bd434742a3240828078b1648b99bf438718a16  images/manifest.sig
```

`file` says of each of the three binaries: static-pie linked, stripped.

The test image was then built in the same clone (6 m 36 s). Its
manifest has `"product": "QS/VS (test image)"` and 12 artifacts,
`qsvshook` among them; the release manifest has `"product": "QS/VS"`
and 11, without it. The test image was not built twice and its
reproducibility was not measured.

## Found while testing

Three defects the suites found in this phase's own code, fixed before
the evidence above:

- **Services could not be ended by `SIGTERM`.** A child inherits its
  parent's signal mask, and PID 1 blocks the signals it reads from a
  descriptor, so every service but the log service ran its 5 s grace
  out and was killed: a power off took 15 s. Services are now started
  with an empty mask (`supervise::signal::unblocked`, with a unit
  test), PID 1 says on the console when it has to kill one, and the
  shutdown suite fails on that line.
- **The last records before a shutdown could be lost.** With the log
  service behind (it synced twice a record at an IPL), PID 1's newest
  records were still in its own queue when it stopped the service. PID 1
  now hands its queue over before it stops anything, and the log
  service syncs once for the records that arrive together.
- **A virtio disk's model was its vendor ID** (`0x1af4`). It is now
  `virtio-blk`.

## Not done

- **NEEDS-CODY: the QEMU job in CI** has still not run on GitHub:
  nothing is pushed. It now builds both images and runs every suite
  (`docs/OPEN_QUESTIONS.md` item 6).
- **NEEDS-CODY: real hardware.** Everything above ran under QEMU. Not
  exercised on a machine: the RTC write, the ACPI power button of real
  firmware, efivarfs on real firmware, a NIC that takes seconds to get
  carrier, and the DMI slot records the location codes are derived from
  (QEMU's machine has none, so every code seen was of the PCI address
  form; the slot form is unit-tested only).
- `BLOCKED(phase1)`: named time zones wait for the Service Tools console of Phase 2; fixed offsets until then (`docs/OPEN_QUESTIONS.md` item 9). Decided in Phase 2: fixed offsets only, in V1.
- `BLOCKED(phase1)`: QS/400's `netlink` still carries its own route protocol number and table name; to change there, then move the pin (`docs/OPEN_QUESTIONS.md` item 13). Done in Phase 2, with the pin (`docs/PHASE2.md`).
- `BLOCKED(phase1)`: nothing writes the other slot, sets a pending slot
  or `BootNext`; the suite does that from the host side. That is the
  update phase.
- Attention codes not seen under QEMU, only in unit tests or by reading
  the code: `A2001030` (no valid A/B record), `A2001050` (no RTC),
  `A2001070` (a state file refused), `A2001095` (`BootOrder` not
  written) and `A2002020` (carrier but no address; the machine that
  posts it is unit-tested).
- Log rotation, the total cap and the per-boot kernel log's pruning are
  unit-tested; no suite fills the disk far enough to see them on a host.
- A DHCP lease is not kept across IPLs: each IPL starts with a
  `DISCOVER`. Within an IPL the client goes on from the lease it had
  when carrier returns.
- IPv6 is link-local only, as the handoff asks.
- The host tools no longer compile on Windows: the conformance harness
  uses Unix sockets. The pipeline needs Linux in any case, and Phase 0
  had not tried Windows either.
- Reproducibility across checkout paths was not measured, as in
  Phase 0.

Not in Phase 1, and not started: the installer and the Service Tools
console (Phase 2); software RAID; any VMM, storage pool or virtual
Ethernet code; any network listener.

Phase 2 changed three things this record describes. The status screen
is the idle screen of the Service Tools console: on the serial port it
is as here, with a last line that says how to sign on; the display
shows it through the console's own renderer. The service table has two
more services (`stgd` first, `tools` after `netd`), so the suites count
six ready services where this record says four. And the installation
media no longer runs the service table: `test --media iso` ends at the
installer's first screen, not at two heartbeats (`docs/PHASE2.md`).
