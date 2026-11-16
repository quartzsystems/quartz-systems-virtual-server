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

BLOCKED(phase8): every suite of Phases 0 to 7 run at the current commit from empty targets; the results go in the acceptance table below.

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

BLOCKED(phase8): the two clean builds' manifests, from the acceptance run.

## Acceptance

BLOCKED(phase8): the acceptance run of Phase 8 at its last commit.

| # | Check | Evidence |
|---|---|---|
| 1 | Step 0 is committed on its own and every Phase 0 to 7 suite passes on the new pin | `9592bd5`, `04b94ba`; the suites: pending |
| 2 | Suites 2 to 15 pass | 15 passes; 2 to 14 pending |
| 3 | Open questions 38 and 39 are closed; the joint gate with QS/MC is recorded as open | `docs/OPEN_QUESTIONS.md` 38, 39 closed, 40 open |
| 4 | Two clean builds are byte-identical; `build deny` and `build doctor` pass | `deny` and `doctor` pass at `40360da`; the builds: pending |
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
- `NEEDS-CODY`: the Dell's processor has AVX2, ADX, AES and PCLMULQDQ, which graviola needs (`docs/OPEN_QUESTIONS.md` item 43).
- The joint gate with QS/MC is open (`docs/OPEN_QUESTIONS.md` item 40).
