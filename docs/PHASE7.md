# Phase 7: the partition service interface

The record of Phase 7. Phases 0 to 6 are `docs/PHASE0.md` to
`docs/PHASE6.md`; what they say still holds, except where this says
what changed.

Until Phase 7 the host knew a partition only from outside: whether its
VMM ran. Phase 7 connects it to the system inside, over a service
channel: the system's reference codes as they are posted, its state, a
controlled shut down, the control panel functions, dumps taken from
outside, and its console as a full screen. There is still no QSI and no
network listener: everything is on the partition manager's local
socket, which Phase 8 puts QSI on. The contract with the system is
QS/400's `docs/QSID.md`, and the protocol is its crate `qsipart`,
pinned with the others.

## The pin

QS/400 `5ce33f14bd141c460fb5703ad62f70721547cd52`, its handoff 17
(`qsipart`, `qsid` in a partition, the console as a full screen over
the console port, `docs/QSID.md`), on QS/400's `main` when this phase
began. Every `rev =` moved to it in one commit of its own (`b950334`),
before anything else of the phase.

| QS/400 change | Here |
|---|---|
| `licpart::ConsoleMode::Partition`, the third console mode (byte 3 of TLV `0x0105`) | the one thing the pin broke: the IPL loader's match on the console modes, and `qsvmm`'s configuration, which now name it `partition` (`b950334`) |
| `qsipart`, the protocol crate | a dependency of `partd`, `part/svcchan`, `part/console`, the hooks and the harness; nothing of it restated |
| `qsid` in a partition; the console port as a display | the system's side of everything below |
| `console`, `svctools`, `hwqual` changes | nothing to change here |

The kernel pin and `NR_CPUS` did not move.

The guest media for the partition suites were built from a clone at the
pin (`~/qs400-5ce3`), as `docs/PHASE4.md` and `docs/PHASE5.md` say how:
the image `qs400-V1R1M0-626e2753f892.img` and the ISO with the
response file, `QS400-V1R1M0-91ce6f034d04.iso`.

## Answers to Phase 6's open questions

| # | Question | What was done |
|---|---|---|
| 37 | management on a VLAN of a switch | not in V1, as recommended; recorded as answered for V1, and open for Cody after it (`docs/OPEN_QUESTIONS.md`) |
| 6, 32, 35, 36 | CI, 64 threads, a virtual disk's copy, tape from QS/400 | unchanged |

Phase 7 raised 38 (how long a system is given to open its channel) and
39 (a held partition and the host's restart).

## What exists

`docs/SERVICE.md` is the channel on the host's side; `docs/PANEL.md`
the control panel functions; `docs/DUMPS.md` dumps and their format;
`docs/PARTD.md` the requests and events; `docs/VMM.md` the device, the
control socket's new requests and a partition kept after a fault;
`docs/PROFILE.md` the console mode and auto start's wait;
`docs/RESOURCES.md` the `[service]` settings.

### The device (`vmm/devices/src/virtio/console`)

virtio-console over the existing virtio-pci transport, with several
ports and the control queue. The two ports are announced by name on the
guest's `DEVICE_READY` and `PORT_READY`; each side's open and close is
tracked; the guest's size message is passed over. Each port is joined to
the host by a link of 64 KiB each way: the device takes the guest's
buffers only while the host has room, so a slow reader stalls the port
and not the VMM, and nothing is held without bound. The control queue is
read a header at a time, and what waits for the guest is one message of
a kind a port. A fuzz target (`console`) drives the control queue and a
port's queues from guest memory that is the input.

### `qsvmm`

- The configuration's `[[port]]`: each port's name, which the partition
  manager takes from `qsipart`, and its listening socket, an inherited
  descriptor taken as it is. One thread a port takes one peer at a time
  and carries bytes both ways; it reads none of them.
- A fault the VMM survives keeps the partition: the processor that met
  it reports `A2005170`, stops every other and parks; memory and state
  are kept. The pause has a reason for each hold (space, a dump, kept),
  and each processor leaves its registers when it parks.
- The control socket is served for every partition, with `hold`, `kept`
  and `dump`. A dump (`part/dump`) is an ELF core, written sparsely,
  with a header record; its progress in `C2005181`.

### The service channel (`part/svcchan`) and the console (`part/console`)

Both are state machines over `qsipart` with no I/O, tested alone. The
channel reads the system's frames as hostile (the protocol's reader, a
flood or junk or bodies that do not read close it), checks the hello's
version and capabilities, matches answers to requests and times them
out, pings, and says when the system stops answering. The console keeps
the latest whole frame from frames and updates, asks for a whole one
when an update does not fit, and has one viewer at a time.

### The partition manager

- Per partition, the channel's sockets made before the VMM and handed to
  it after the taps; their connections kept by threads of the service,
  made again when they go, each tied to its activation.
- The states of the decisions table (`docs/SERVICE.md`, "States"); the
  current reference code and a history marked whose each code is, kept
  on disk (`/data/state/partitions/<ID>/history`, 256 entries). This
  removes the `BLOCKED` item about the history carried since Phase 4.
- The hello checked against the definition; the system's release, fix
  level and addresses in the list and in events.
- The three shut down kinds with restart, their timeouts as settings,
  forced when overdue; the host's own shut down through each system in
  reverse start order.
- Holding after a terminal code; dumps in the media repository's pool,
  with their limits, list, streamed read and delete.
- The panel functions; the console's viewer and screen capture; with a
  serial console mode, the serial port's bytes.
- Auto start's wait for an IPL to complete.
- Each partition's run record carries its state, so a manager started
  again shows each partition as it was until its system says otherwise.
- A fuzz target (`channel`) that drives the manager itself, over a
  machine that does nothing, with what a system sends on both ports.

### PID 1, Service Tools and the hooks

- PID 1's wait at the host's shut down gives each partition the
  `host_timeout_secs` of `partitions.toml` and the VMM's grace.
- Service Tools option 1's list (F11) shows each partition's current
  code live, the system's release and its state; still no action on a
  partition and no console there.
- The test hooks carry the new requests as they carry the others, and
  have `console` (a viewer through the partition manager's socket),
  `dumpread` (a dump's streamed read), `partition kill <id> STOP|CONT`,
  and a hostile system: a test image as a partition that floods its
  service port when one of its disks begins with a marker. None of it is
  in a release image.

### Messages and reference codes

`QVS2301` to `QVS2317` (`docs/MESSAGES.md`), and the `C2009` and
`A2009` blocks with `A2005170`, `A2005180` and `C2005180` to
`C2005185` (`docs/REFCODES.md`).

## Kernel additions

None: `CONFIG_VIRTIO_CONSOLE` was already in the kernel fragment, and
the guests' kernels are their own.

## Running

```
cargo run -p build -- all --test-image
export QSVS_QS400_IMAGE=$HOME/qs400-5ce3-target/images/qs400-V1R1M0-626e2753f892.img
export QSVS_QS400_ISO=$HOME/qs400-5ce3-target/images/QS400-V1R1M0-91ce6f034d04.iso
cargo run -p build -- test --suite service-states
cargo run -p build -- test
cargo run -p build -- fuzz --target channel --secs 60
```

`service-terminal` copies a dump out through a scratch disk and runs
`readelf` on it, from binutils, on the machine that runs the harness.

## Acceptance

Taken in a fresh clone at `e1373ce` (Phase 7's last commit) on ext4 in
WSL, every target empty, `QSVS_BUILD_ID=phase7`, the signing key of
the reproducibility runs, the guest image and ISO of the pin, on
2026-11-15. Host: rustc 1.98.0, clang 21.1.8, QEMU 10.2.1, cargo-deny
0.20.2, kernel 6.18.33.2-microsoft-standard-WSL2, 12 processors. No
other QEMU ran beside any suite.

101 of 114 steps passed. What failed, and what came of it:

| Step | Why | Since |
|---|---|---|
| `notices --check` | `NOTICES.txt` was not written again after the last dependency change | current from `0cb324c` on |
| `partition-manager-restart`, `service-manager-restart` | a partition manager started again forgot that a system had no channel (the first); the channels of both partitions did not come back after the restart (the second) | the first fixed in `149761c`; the second run again in Phase 8's acceptance |
| `partition-attributes` | the suite still expected `console local` in the record, which Phase 7 gives as `partition` | the suite, `d3285fc` |
| `partition-recorded`, `pool-install`, `partition-identity` | the guest's own daemon wrote into the middle of the line an IPL code is on, and the harness looked for the code at the start of a line | the harness matches a code anywhere on the guest's line, `d3285fc` |
| `partition-kill` | `pwrdwn`, sent while a CL command was ending, was taken as CL (`CPD0030`) | the suite powers down with `PWRDWNSYS`, `d3285fc` |
| `pool-full`, `pool-pulled-raid5`, `pool-pulled-raid6` | the hooks' channel did not answer in time (`EAGAIN`) under a guest three levels deep | run again in Phase 8's acceptance |
| `net-tagged` | a tap's statistics file was gone when read | run again in Phase 8's acceptance |
| `service-tools` | the partition was not responding (`A2009020`) during its IPL while a QS/400 build ran beside it | run again in Phase 8's acceptance |

| # | Check | Result |
|---|---|---|
| 1 | The pin's own commit | `b950334` |
| 2 | Phase 7's suites | 14 of 17 passed at `e1373ce`; `service-manager-restart` and `service-tools` failed as above; 15 fuzz targets 60 s each, no finding (`channel` 187,691 runs, 2,003 edges); `test --media iso` passed |
| 3 | The history's `BLOCKED` item gone, question 37 answered | `docs/OPEN_QUESTIONS.md` 37; 38 and 39 since closed by Phase 8 |
| 4 | Two clean builds byte for byte; `deny`, `doctor` | identical (below); `deny` and `doctor` passed; `cargo fmt`, both clippy runs and `cargo test --workspace` passed |
| 5 | One pin; the protocol is QS/400's crate | one `rev` for every crate; `qsipart` a dependency, nothing of it restated |
| 6 | The release image has no listener and no action on a partition | its initramfs has no `TEST IMAGE:`, no `qsvshook`, no `qsvs.hook=` (0, 0, 0; the test image's 1, 9, 0); `sockets` passed |

Phase 7 was accepted as far as these go; what failed is run again,
with every earlier suite, in Phase 8's acceptance (`docs/PHASE8.md`).

### Hashes

```
e53ba72234a5fd469a18fad5acd082c9e9b4ebe228a48a4c286e5b5a0d2e381b  images/initramfs.cpio
b860e00113b0a86862644741cd3083b0f5d8e76205adbeadd2dbf9483544b7f6  images/initramfs-media.cpio
c05a4180343355da428a1fd3f7d056850ca41728798d1dd7c61e11402961186d  images/qsvs-V1R0M0-a-c05a41803433.efi
b9846e61cc4cedc65c85b2e0da288c3b818d76574f20395f1d197df52ecc68c4  images/qsvs-V1R0M0-b-b9846e61cc4c.efi
784011d6119f054730d2ce269a3e1cb99f5be5f54950ead5722cff3f8a8a0ce8  images/qsvs-V1R0M0-d-784011d6119f.efi
89f2b98c9573471793f1df172b79c54022ecc2df688e53698381e9ce446bf9d1  images/qsvs-V1R0M0-c05a41803433.img
c70f2ff68075528a5847b99f565aedee8ecf9a88526e13a65e4da0fa13d958c4  images/QSVS-V1R0M0-784011d6119f.iso
5d58e5e0b6f6a7939fd01b64fce4b4a98e0b0e847c5a1a4d6888213f9a4d1d02  images/manifest.json
00a43ed6943762956e6fa348077a47863c8aa386e7f37c7337583cad49350da6  kernel/build/.config
```

## Not done

- `BLOCKED(phase7)`: a VMM found running by a partition manager started again is watched by looking at its control group every 25 ms, not by an event (carried from Phase 4, `docs/PARTD.md`).
- `BLOCKED(phase7)`: the host cannot tell beforehand whether a system has a service channel, and waits `channel_wait_secs` for it (`docs/OPEN_QUESTIONS.md` item 38); QS/400 would say so in its LIC record.
- `BLOCKED(phase7)`: QS/400's installer IPLed from source D onto a blank disk has no LIC record and so no console mode, and draws on the serial port alone (`A6005008`); the console port shows the system once it has IPLed from its load source (`docs/QS400_GUEST.md`, the service channel, item 5).
- Not in Phase 7, and not started: QSI, pairing and any network listener; a console or any action on a partition from Service Tools; the system's own main storage dump; serviceable events; a change to a running partition's resources.

## For a real machine (NEEDS-CODY)

- `NEEDS-CODY`: a dump of a partition with tens of gigabytes of memory, timed on the Dell: the time, the bytes written and the bytes it takes in the pool, and the partition's processors stopped for that long. The build host dumped 4 GiB of a QS/400 partition in 11 s.
- `NEEDS-CODY`: a console session watched by a person: QS/MC is not there yet, so a client of the partition manager's socket (the hooks' `console` in a test image) is all there is; whether the screen as the protocol carries it is what a person at QS/400's own console would see, and whether typing keeps up.
