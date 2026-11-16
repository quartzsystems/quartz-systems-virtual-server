# Phase 8: the QSI host server

The record of Phase 8. Phases 0 to 7 are `docs/PHASE0.md` to
`docs/PHASE7.md`; what they say still holds, except where this says
what changed.

Until Phase 8 a host had no listener: everything was on local sockets.
Phase 8 gives it one, the QSI host server (`docs/QSISERVER.md`): TCP
2300 on the management addresses, TLS 1.3 both ways, the consoles
paired at Service Tools option 4. A paired console asks it what QSI's
families carry, and the server translates each request to the service
that owns the state. QSI itself, its messages, its session and its
pairing, is QS/400's crates (`qsi`, `qsisess`, `qsitrust`), pinned
with the others; nothing of them is restated here.

## Step 0

### The pin

QS/400 `30e18c2e5ff4896bbb56a754760c530dfa96c26c`, its handoff 18
(the QSI crates, `docs/QSI.md` with its mapping table, the service
channel TLV in the LIC record), on QS/400's `main`. Every `rev =` moved
to it in one commit of its own (`9592bd5`), before anything else of the
phase. The kernel pin was already QS/400's.

| QS/400 change | Here |
|---|---|
| `qsi`, `qsisess`, `qsitrust` | the QSI host server, `qsictl` and the harness; nothing restated |
| `licpart::ServiceChannel`, TLV `0x010C` | the IPL loader reads it (below) |
| graviola and `rustls` 0.23, `spake2` | the TLS of the server and `qsictl`; `cargo deny` passes, no C |
| nothing else | the pin broke nothing: format, both lint runs and every unit test passed at `9592bd5` |

### The channel TLV (closes question 38)

The IPL loader reads TLV `0x010C` of the partition's LIC record
(`04b94ba`) and `qsvmm` reports it (`CHANNEL has N`, `CHANNEL none`,
`CHANNEL unsaid`):

| The record says | The partition manager |
|---|---|
| it has a channel | waits for its hello with no 30 s guess: Starting until the system reports its IPL complete |
| it has none | Running once the kernel is entered |
| nothing (a record from before handoff 18) | the `channel_wait_secs` wait, as in Phase 7 |

QS/400's `docs/QSID.md` reads a missing TLV as "no channel"; the
handoff asked that it behave as Phase 7 did, because a system of
handoff 17 has a channel and writes no TLV. The handoff's rule is kept.

### Every earlier suite on the new pin

Every suite of Phases 0 to 7 ran in the acceptance run below, in a
fresh clone at `0177008` with QS/400's guest image and installation
ISO built at the pin: 90 of the 101 passed there, and the 11 that
did not pass on the working tree after the fixes under "Acceptance".
`net-tagged` and `service-manager-restart`, which Phase 7's run
left unexplained, passed in it.

## Answers to Phase 7's open questions

38 is closed by Step 0; 39 is closed as recommended (a held partition
is ended at the host's restart, as built). 6, 36 and 37 stay as they
were. Phase 8 raised 40 to 43 (`docs/OPEN_QUESTIONS.md`): the joint gate
with QS/MC (open), what the services say while the server is down, an
event log that begins at a number (decided, waiting on QS/400), and the
Dell's processor for graviola (`NEEDS-CODY`).

## What exists

### The server (`qsi/server`, `lic/qsvsqsid`)

`docs/QSISERVER.md` says it whole. In short:

- `trust/`: the paired consoles in `qsi-consoles.toml` (through
  `base/state`), the pins QS/400's TLS asks for, and the one pairing
  password out, in memory only: one pairing, 15 minutes, five attempts.
- `families/`: each request of the base family and the nine host
  families translated to PID 1, the partition manager, the pool service
  or the switch service, and its answer or every message of its refusal
  back; QSI's own identifiers lead a conflict QSI names.
- `events/`: the services' events as QSI's, the pool service looked at
  every 2 seconds for what changed, and the one log on disk, taken up
  again by a restarted server.
- `conn/`: one console's connection after TLS, sans I/O: the session,
  pairing, requests, streams and events.
- `serve/` (Linux): the listener on the management addresses with its
  limits, a thread for each connection, the streams' far ends, the
  pumps, the local socket.
- `fuzzing`: the target of what a pinned console sends.
- `lic/qsvsqsid`: reads the identity as root, makes `/data/qsi` its own,
  binds the local socket, becomes user 22990 for good; PID 1's service
  `qsid`, order 35.

### What the services gain

- `partd` (`ca936ec`): a revision on every definition and profile, a
  put or change at a revision refused when it is stale (`QVS2013`,
  `QVS2014`), a new profile of a name there refused (`QVS2012`), the
  current configuration saved as a profile (`QVS2015` when not
  activated), a partition created with its first profile in one request
  and undone whole, profile events; `docs/PARTD.md` requests 34 to 38.
- PID 1 (`df3a424`): the host's state (`State`) and a power down with a
  delay and an immediate kind (`PowerDown`) on its control socket.
- The services' sockets shared with the server's group (`51300a3`).
- `partd` keeps a system's lack of a channel in its run record
  (`149761c`), found when Phase 7's acceptance ran.

### Service Tools option 4 (`svc/tools/src/consoles.rs`)

`docs/SERVICETOOLS.md`: the consoles paired, each shown and removed by
its typed name; start pairing, the password shown once with its expiry,
its attempts and the host's fingerprint; the management interface.
Over the server's local socket (`qsi/local`).

### `qsictl` (`tools/qsictl`)

`docs/QSICTL.md`: pairs, asks every request, follows events, uploads,
downloads and attaches a console. Built and run on Linux and on Windows.

### Reference codes

`C2010000` to `C2010050` and `A2010000`, `A2010030`
(`docs/REFCODES.md`).

## What QSI v1 does not carry

A partition created with its first profile in one request, and a
definition changed at a revision: `partd` has both, QSI's `Create` has
a name only and its `Definition` no revision. The pairing password is
16 characters, QS/400's crate's, where the handoff asked for 12.

## Running

```
cargo run -p build -- all --test-image
export QSVS_QS400_IMAGE=...   QSVS_QS400_ISO=...
cargo build -p qsictl
cargo run -p build -- test --suite qsi-pairing
cargo run -p build -- test
cargo run -p build -- fuzz --target qsi --secs 60
```

The QSI suites reach the host's server through QEMU's user network, a
port of the harness's forwarded to 10.0.2.15:2300, from outside the
host. `qsi-qsictl` runs the `qsictl` built beside the harness.

## Conformance

| # | Suite | What it shows |
|---|---|---|
| 1 | every earlier suite | Step 0 |
| 2 | `qsi-pairing` | the password from Service Tools pairs once; a second use, five wrong passwords and an expired one refused, with the codes; golden screens `tools-qsi`, `tools-qsi-pair` |
| 3 | `qsi-trust` | an unpinned peer refused with no pairing out; pinned reconnects; a removed console refused and its session closed; a third pairing refused; a relayed pairing fails |
| 4 | `qsi-end-to-end` | over QSI only (the pool made at the host): the install ISO uploaded, a thin disk, a partition and its profile, source D, the codes as events, the installer to Sign On on the console stream, the recorded side, a sign-on, a shut down through the system, the partition and its disk deleted |
| 5 | `qsi-families` | every base, host and vres request and a partition's every request once, each against what its service says locally |
| 6 | `qsi-events` | a disconnect during an IPL and a reconnect, every code once and in order; the server killed and restarted numbers on; past the log's reach, read again |
| 7 | `qsi-two-consoles` | the same events; a stale put refused; a console taken over closes the first's |
| 8 | `qsi-streams` | an upload cut at a random place leaves nothing; requests beside an idle stream; a dump fetched whole, the sum of its file |
| 9 | `qsi-power` | a restart over QSI: the partitions ended first, the session ended cleanly, the console back and the auto start partition with it |
| 10 | `qsi-service-kill` | the partition manager killed: its requests refused `QSI0301`, the others answered, served again when it is back |
| 11 | `qsi-hostile` | floods, a silent handshake, nonsense before and after the handshake, streams past the limit, credit never given |
| 12 | `qsi-listen`, `sockets` | one listening socket: 10.0.2.15:2300 |
| 13 | `qsi-qsictl` | `qsictl` pairs, makes a partition, activates it, attaches its console |
| 14 | `qsi-log` | every request logged with the console and its user; no password, key or screen |
| 15 | `fuzz --target qsi` | 152,490 runs in 60 s, 1,971 edges, no finding |

## Hashes

The two clean builds of `0177008` (build ID `phase8`), the same to the
byte:

```
385998e06f0b579c5bee688c3f96aeba1939c194891bba7c47656a5c9ecd9bd9  images/initramfs.cpio
fdcfcc61e07823a13bbfc958f44b733717617f028fc02bc1aac669b3392d3f07  images/initramfs-media.cpio
814e21ba5efa1975ffbcb67c267cd1f6233b05b9044a0bea699d7dc48fe863bd  images/qsvs-V1R0M0-a-814e21ba5efa.efi
d1d640bf3f4166cd0aa5620e2ac134924e981d8a448ad48fe566a70d0285e9f4  images/qsvs-V1R0M0-b-d1d640bf3f41.efi
0d96fed476d2eadffaa42176e003b9dec5d2c7aaef97bda30ef94b0b43734fd0  images/qsvs-V1R0M0-d-0d96fed476d2.efi
5322085c6125f4c57ca3a03ab61f5fb723ddb4a887a2d75c97fcdff58a93b121  images/qsvs-V1R0M0-814e21ba5efa.img
f4cc81b524ed50e7f2bd52b48bcd3a0869fa332384ed685f66c1a1f9664c590d  images/QSVS-V1R0M0-0d96fed476d2.iso
edd5f5ffac9dbf82bfc2919054a9d4bc493711eaf43a3debb72fda193d4c80d2  images/manifest.json
c63bb7fe966d2d741e1cf7155cbeca8020684490192df5003c3fce8814033b04  kernel/build/.config
6bc8d23f27be8f69570a6dd6b9b9a34948e031d75edf8f25524d2283c2f2a591  x86_64-unknown-linux-musl/release/qsvsinit
dc1eb7d0fd8f6334e593683f4c5a92b49f9e44b374df7476854612d2795608f1  x86_64-unknown-linux-musl/release/qsvmm
ab74bfb10ec9f2c003955c44c6480b49479244618f1e4afc4480a5bbda0507ac  x86_64-unknown-linux-musl/release/qsvspartd
d453440c9feb21ba29e0c4dd18a91e33e6a7a10f82dee33f6b6d2d647bd7cd62  x86_64-unknown-linux-musl/release/qsvspoold
c1ca17baf2cf3bc43236f928734f66edff9d64741d050d7045eef56d248ea676  x86_64-unknown-linux-musl/release/qsvsvswitchd
a16a7a23b950d0ad1f1f1477384ac971f84b43376ac7ac3fd063dea3bec8aa12  x86_64-unknown-linux-musl/release/qsvstools
9bfe1528c0ad7506bf4d9a2e9acaf42a0aa1579c6383e69c54829561ce9b219e  x86_64-unknown-linux-musl/release/qsvsqsid
```

`80b3aff` changed `qsvstools` after them (the status screen below),
so its image differs from these.

## Acceptance

The acceptance run, at `0177008` in a fresh clone on ext4 (rustc
1.98.0, clang 21.1.8, QEMU 10.2.1, WSL's 6.18 kernel, 12 processors):

| Step | Result |
|---|---|
| two clean builds | 609 s and 505 s; every hash above the same |
| test image | built; the hooks in it, none in the release image |
| `doctor`, `deny`, `notices --check` | pass; 241 crates, no `links`, no C build helper |
| `cargo fmt`, both clippy runs | clean |
| unit tests | 629 passed, none failed |
| `qsictl` | built; in neither image |
| the suites of Phases 0 to 8 | 103 of 114 passed |
| `test --media iso` | the installer's welcome |
| the fuzz targets, 60 s each | 16 targets, no finding; `qsi` 157,126 runs, 2,008 edges |

The 11 that did not pass, and what was done:

| Suite | Why | Fixed |
|---|---|---|
| `install-serial` | it took option 4 for one not there yet; the golden status screen had no `qsid` | `72f34d2`, `80b3aff` |
| `installed-base` | its host has no Ethernet carrier, and the current code was `A2010000` (the server has no address), which is right; the status suite now accepts it when the management line says none | `72f34d2` |
| `partition-tools` | the golden status screen had no `qsid`; with it, eleven services in a test image ran past the lines a status screen shows and the last was dropped unsaid. The status screen now goes three to a line when two would not fit | `80b3aff` |
| `service-tools` | its golden screen had never been written: Phase 7's run failed before it | `3bc49e1` |
| `partition-define` | it compared a changed definition without the revision the change gave it | `3bc49e1` |
| `partition-manager-restart` | it wanted `C2006010` as the current code, which has been a note of the history since `1481aca` | `3bc49e1` |
| `pool-pulled-raid5`, `pool-pulled-raid6` | a 32 MiB copy onto a parity pool, three levels deep, had 10 s to answer | `3bc49e1`: two minutes |
| `partition-ipl-a`, `partition-identity` | QS/400's own storage service lost its channel during the guest's first IPL (in `partition-identity` with write errors on the guest's `ublkb0`), and the IPL halted | nothing: both passed on a second run |
| `partition-end` | the guest kernel, IPLing after its restart, took a double fault in its TLB flush (`INVPCID`) with its stack page missing, and panicked; the panic's restart made a second activation | nothing: passed on a second run |

Each of the 11 then passed on the working tree, with the test image
and the release image built from it. The three guest failures were in
QS/400's guest, three levels of virtualization deep; the other suites
that IPL QS/400 passed, and none of the three came back. Whether a
guest ever does so on the Dell is below.

| # | Check | Evidence |
|---|---|---|
| 1 | Step 0 is committed on its own and every Phase 0 to 7 suite passes on the new pin | `9592bd5`, `04b94ba`; every suite of Phases 0 to 7 passed, 90 at `0177008` and the other 11 after the fixes above |
| 2 | Suites 2 to 15 pass | the thirteen QSI suites and `sockets` passed at `0177008`; the `qsi` fuzz target found nothing |
| 3 | Open questions 38 and 39 are closed; the joint gate with QS/MC is recorded as open | `docs/OPEN_QUESTIONS.md` 38, 39 closed, 40 open |
| 4 | Two clean builds are byte-identical; `build deny` and `build doctor` pass | the hashes above, from two clean builds of `0177008`; `deny` and `doctor` pass |
| 5 | Every QS/400 crate is pinned at the one new commit; QSI's messages, session and pairing are QS/400's crates and nothing of them is restated | every `rev =` is `30e18c2`; `qsi`, `qsisess`, `qsitrust` are dependencies, and the server's code is translation, pins and logs |
| 6 | The release image listens on TCP 2300 on the management addresses and nowhere else, and holds neither the hook nor `qsictl` | `sockets`, `qsi-listen`; the build's package list test (`qsictl` in no image, the hook in the test image only) |

## Not done

- `BLOCKED(phase8)`: what the partition manager and the switch service say while the server is down is not in the event log (`docs/OPEN_QUESTIONS.md` item 41).
- `BLOCKED(phase8)`: the server learns of a change of the management addresses by looking at PID 1's fact every 5 seconds, not by word from `netd`.
- `BLOCKED(phase8)`, carried: a VMM found running by a partition manager started again is watched by looking at its control group every 25 ms, not by an event (`docs/PARTD.md`).
- `BLOCKED(phase8)`, carried: an adapter's unit numbers have no gaps (`docs/PROFILE.md`).
- `BLOCKED(phase8)`, carried: cores are given without regard to sockets or to memory's place, and only 2 MB pages are used (`docs/RESOURCES.md`).
- Not in Phase 8, and not started: code update, serviceable events and the host's own dumps (Phase 9); physical I/O (Phase 10); QS/MC itself; managing service tools users over QSI; a listener in QS/400; any second listener or web interface.

## For a real machine (NEEDS-CODY)

- `NEEDS-CODY`: pairing at the Dell's console: Service Tools option 4 on its display, the password typed at `qsictl` on a workstation on the LAN.
- `NEEDS-CODY`: `qsictl` from a workstation on the LAN, Linux and Windows: every request, a console attached.
- `NEEDS-CODY`: an install of QS/400 into a partition through `qsictl`: the ISO uploaded, the partition made, source D, its console.
- `NEEDS-CODY`: QS/400 IPLed in partitions on the Dell many times over, to see whether its storage service losing its channel or its kernel faulting in a TLB flush (both seen once each three levels deep in the acceptance run) happens with one level of virtualization.
- `NEEDS-CODY`: the Dell's processor has AVX2, ADX, AES and PCLMULQDQ, which graviola needs (`docs/OPEN_QUESTIONS.md` item 43).
- The joint gate with QS/MC is open (`docs/OPEN_QUESTIONS.md` item 40).
