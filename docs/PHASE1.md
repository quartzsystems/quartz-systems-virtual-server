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
  `host.toml` (host name, time zone, NTP servers, the derived serial),
  `network.toml` (the device by name or MAC, DHCP or a static address
  with prefix and gateway, the VLAN ID, DNS servers) and
  `inventory.toml`. An absent file is its defaults: the host name is
  `QSVS` and the last six hex digits of the first NIC's MAC, UTC, no NTP
  servers, DHCP on the first interface with carrier.
- `base/supervise`: the fixed service table (a binary, an order, the
  services it needs, a time to become ready); the restart policy (1, 2,
  4 ... 30 s; five failures in ten minutes end the restarts); the
  supervisor, which PID 1 drives from its own loop; the control
  socket's protocol over QS/400's `ipc` (status, power off, restart,
  and a service's `post` and `note`); the readiness signal (a service
  writes one byte to a pipe it inherits as its standard input, so no
  descriptor number crosses `exec`); signals read from a descriptor.
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
  when they can be.
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
- The orderly shutdown: `D2001000`, the services ended in reverse order
  (5 s each, then killed), sync, `/data` unmounted, `D2001090` or
  `D2001091`, then power off or restart. It is started by the control
  socket, by `SIGTERM` to PID 1, by the ACPI power button, and by
  Ctrl-Alt-Del (a restart; `docs/OPEN_QUESTIONS.md` item 15).
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
  location code (`docs/LOCATION.md`, **NEEDS-CODY**). The system serial
  number is DMI's when it has a real one; otherwise it is derived once
  from the first NIC's MAC (`M` and the twelve hex digits) and kept in
  `host.toml`.
- `lic/qsvshook`: the test hooks. In a test image only.

### The two images and the test hooks

`cargo run -p build -- all` builds the release image. `all --test-image`
builds the test image: the same sources with the test hooks, under
`<target>/test-image/`, a target root of its own.

The hooks are:

| Hook | Where | Switched on |
|---|---|---|
| the channel: the harness's way to the control socket's status and to files of the running host, on the second serial port | `qsvshook channel`, a service of the test image's table | always, in a test image |
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

All of these but the `RTC_HCTOSYS` line were already `=y` through
`x86_64_defconfig`; naming them makes the build fail if a kernel bump
drops one. Nothing `netd` or the xfs mount needed was missing:
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
| `static-vlan` | a seed with a static address and VLAN 100: the address is on `eth0.100`, which exists. Then `network.toml` is changed to DHCP and `netd` told to reload: the address changes and `eth0.100` goes, in the same process |
| `time` | an SNTP responder in the harness, three days ahead, moves the guest's clock |
| `inventory` | QEMU with 2 sockets of 2 cores, 2048 MiB, two disks with serials and one NIC with a fixed MAC: `inventory.toml` matches |
| `supervision` | `netd` killed through the channel is started again and ready; with `flaky-fails`, `flaky` is stopped after five failures with `A2001080` and the host runs on |
| `power-pull` | with the churn running, QEMU is killed at 50 random moments; after each the host IPLs and `host.toml`, `network.toml` and `inventory.toml` parse and are a complete old or new version |
| `slot-confirm` | slot B booted as pending confirms: default B in the record on the disk and `BootOrder` 0001,0000 in the firmware's store. With `refuse-confirm` it does not, and the next IPL is on slot A with `A2001035` |
| `shutdown` | QEMU's power button: `D2001000`, `D2001090`, QEMU ends within 30 s; the next mount of `/data` needs no xfs log recovery; the history has the shutdown |
| `sockets` | no TCP socket listens; no TCP or UDP socket is bound off loopback but the DHCP client's |
