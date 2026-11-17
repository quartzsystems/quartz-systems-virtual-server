# Phase 9: service, dumps and code update

The record of Phase 9. Phases 0 to 8 are `docs/PHASE0.md` to
`docs/PHASE8.md`; what they say still holds, except where this says
what changed.

Until Phase 9 a host posted reference codes and nothing kept track of
what a person had to do about them; a console could not take the
host's own service data away; and a new release went on only by
installing again. Phase 9 gives the host problems (`docs/PROBLEMS.md`),
its own dump (`docs/DUMPS.md`), and code update into the slot that does
not run, with its rollback (`docs/UPDATE.md`), all three over QSI's
`service`, `update` and `base` families and at Service Tools option 8.

## Step 0

### The pin

QS/400 `39105ca27a1c49553b3bc659615987e6c260ca65`, its handoff 19
(QSI's `service` and `update` families, a system's own dumps in the
`base` family, `EventLog::resume` in `qsisess`), on QS/400's `main`.
Every `rev =` moved to it in one commit of its own (`357958f`), before
anything else of the phase. The kernel pin was already QS/400's.

| QS/400 change | Here |
|---|---|
| `qsi::service`, `qsi::update` | the server's `service` and `update` families (below) |
| `base` gains `Dumps`, `TakeDump`, `DeleteDump`, `DownloadDump` | the pin's commit refuses them as not agreed, since the hello did not offer them; the host dump gave them their answers |
| `qsisess::EventLog::resume` | the server's event log taken up from disk (below) |
| nothing else | the pin broke only the server's exhaustive match over the base family's requests; format, both lint runs and every unit test passed at `357958f` |

### What Step 0 closes

| Commit | What |
|---|---|
| `6856e98` | the event log taken up with `EventLog::resume` at its last contiguous run: no first-event placeholders (question 42 closed) |
| `231f16a`, `2522b5d` | the server keeps what its log last said of every partition, pool, virtual disk, image, volume, switch, problem and host dump (`said`, a journal beside the log), and when it takes up a service's subscription it compares and logs each difference: a console away while the server was down gets every change (question 41 closed) |
| `a0b1ea8` | the network service tells the server of a change of its management addresses over the server's local socket (request 6, `management`); the 5 s look at PID 1's fact is gone |
| `b51e4b6` | a processor without AVX2, ADX, AES or PCLMULQDQ: PID 1 posts `A2010010` at the IPL, the server serves its local socket only and never touches the cryptography, and option 4 says why |

### Every earlier suite on the new pin

Every suite of Phases 0 to 8 ran at `b51e4b6`, the end of Step 0, in a
fresh clone with QS/400's guest image and installation ISO built at
Phase 8's pin: 114 of the 115 passed. `qsi-events` did not: its last
step subscribed again from event 1 on a session that had seen later
events, and with the log now taken up across the server's restart the
log still reached event 1, so the server replayed it and the console's
session closed on a number out of order. The step now uses a connection
of its own and pushes the log past its bounds first (`35ad55f`), and
passed at `26869db`. `qsictl`'s link now says why a session closed
(`b33e567`), which is how this was found.

## Answers to Phase 8's open questions

41 and 42 are closed by Step 0. 43 (the Dell's processor) stays
`NEEDS-CODY`, but the host now answers it at its first IPL. 40 (the
joint gate with QS/MC) stays open. 6, 36 and 37 stay as they were.
Phase 9 raised 44: a partition's creation time, which a capture cannot
put back (open, with a recommendation).

## What exists

### Problems (`svc/problem`, `lic/qsvsprobd`)

`docs/PROBLEMS.md` says it whole. In short:

- Every posting may say what it is about (`supervise::About`: a kind,
  an identifier, a name, a location code, a serial number); the
  history keeps it beside the code. The storage service, the pool
  service, the switch service, the partition manager and the server say
  it for the codes they post.
- The catalogue (`lic/qvsmsg`, `problems.rs`) holds one row for every
  attention and fatal code and the informational codes that clear
  them: what opens, what it is about, what clears it. A unit test holds
  it to the codes and to `docs/PROBLEMS.md`.
- The problem service, supervised at order 12, reads the history from
  where it got to, opens, counts and clears problems, and keeps its log
  and its place in one write, so a restart loses nothing and doubles
  nothing. It watches `/data`: 90% used posts `A2001045`, which opens a
  problem, and under 85% `C2001045` clears it.
- QSI's `service` family: list, show, close, the log; events as they
  change.

### The host's dump (`svc/hostdump`)

`docs/DUMPS.md`, "The host's dump". A ZIP archive with stored entries
and a `MANIFEST.txt` of every entry with its size and sum and of what
was left out and why: the logs, the reference code and problem
histories, the inventory, every state file but the host's key, the
users' hashes and the consoles' pins, the services' status, the
partitions, pools and switches, the control groups and the kernel's
log. PID 1 takes it on a thread into `/data/dumps/host` (root, group
22990), eight at most and 1 GiB in all, the oldest going first. It is
taken when a console or Service Tools asks, and by PID 1 itself when a
service stops after its retries or a pending slot does not confirm.
`hostdump`'s unit tests fail the build when one of those secret files
is not left out.

### Code update (`svc/update`, `lic/qsvsupdd`)

`docs/UPDATE.md` says it whole. In short:

- The update image (`QSVSUPDT`): both slots' UKIs and a manifest of
  them, the product, the release, its fixes and the state schemas it
  reads, signed by the build's key. `cargo run -p build -- update`
  makes it from the slots just built and lists it in the build's
  manifest.
- The update service, supervised at order 32, stages an image on
  `/data/update`, verifies it against the running image's trusted keys
  (nothing turns that off), refuses another product's, an older one
  without leave, one that cannot read the state and one that is
  damaged, then writes the other slot's UKI to every load-source disk
  and reads it back, and only then asks PID 1 to mark the slot pending
  (attempts 1). An activation (attempts 0, `BootNext`) shuts the
  partitions down through their systems first when asked to, and
  restarts.
- PID 1 alone writes the A/B record and the firmware's variables
  (control requests 10 to 13). Before the pending slot's first IPL
  reads state it keeps a copy; a slot confirmed posts `C2012060`; a
  trial not confirmed in 15 minutes is restarted, and the old slot puts
  the copy back (`C2012075`) and posts `A2001035`, which opens a problem,
  and `A2012070`; PID 1 takes a host dump.
- QSI's `update` family: status, stage (an upload stream), install,
  activate, cancel; events as it goes.

### Service Tools option 8 and the problem log (`svc/tools/src/service`)

Option 8 is live: work with problems (display, close), take a host
dump, work with dumps (copy to a removable volume, delete), install an
update from a removable volume, display the code levels, shut down all
partitions. Option 7 gains 3, the problem log. Every panel is in
`docs/SERVICETOOLS.md` and has a golden file.

### `qsictl` (`tools/qsictl/src/service.rs`)

`qsictl problems`, `qsictl update` and `qsictl hostdump`
(`docs/QSICTL.md`). It asks for `SERVICING` beside `HOST_V1`.

### Capture and put back (Section 5)

Nothing new was built: suite 13 (`svc-capture`) is the proof that
Phase 8's families are complete. It makes three partitions with their
profiles over a pool, three virtual disks, a tape volume, a switch and
a shared processor pool; reads the whole configuration over QSI into
one document (`host` for processor pools, `vres` for pools, disks,
volumes and switches, `partitions` and `profiles` for each partition);
installs the host again (a new load source and a blank pool disk),
makes the pool and the switch again by the same names at the host,
puts the document back over QSI alone, and reads it back the same,
definition by definition and profile by profile: the processor pool,
the virtual disks, the tape volume and the partitions are made again
from the document.

### Reference codes and messages

The new codes are in `docs/REFCODES.md` (`A2001045`, `C2001045`,
`A2007025`, `C2007055`, `A2009055`, `A2010010`, `A2010035`, the
host dump's `C2011xxx` and `A2011010`, the update's `C2012xxx` and
`A2012xxx`); the messages `QVS5001` to `QVS5004`, `QVS5101` to
`QVS5149`, `QVS5201`, `QVS5202` and `QVS5301` to `QVS5318` are in
`docs/MESSAGES.md`.

## Running

```
export QSVS_SIGNING_KEY=...                  one key for both builds
QSVS_RELEASE=V1R0M1 QSVS_FIXES=QV00001 cargo run -p build -- all --test-image
QSVS_RELEASE=V1R0M1 QSVS_FIXES=QV00001 cargo run -p build -- update --test-image
cp <target>/test-image/images/*-update-*.upd ~/next.upd
cargo run -p build -- all --test-image
cargo run -p build -- update --test-image
export QSVS_UPDATE_IMAGE=~/next.upd QSVS_QS400_IMAGE=...
cargo build -p qsictl
cargo run -p build -- test --suite svc-update
cargo run -p build -- fuzz --target update-image --secs 60
```

`QSVS_UPDATE_IMAGE` is the update image of another release built with
the same signing key: the update suites install it over the test
image, and `svc-update-back` installs the test image's own back over
it.

## Conformance

| # | Suite | What it shows |
|---|---|---|
| 1 | every earlier suite, `svc-away` | Step 0; a console away while the server was down gets the pools' and partitions' changes from the log |
| 2 | `svc-problem-pool` | a pulled pool disk opens one problem naming the unit by its location code and the pool; the spare's rebuild clears it; the spare pulled opens a second; the first closed over QSI with the console's user recorded |
| 3 | `svc-problems` | each condition the harness can cause opens its problem with the right code, description and resources |
| 4 | `svc-problem-kill` | the problem service killed while 60 codes are posted: none lost, none doubled |
| 5 | `svc-hostdump` | a host dump taken over QSI, fetched, opened by `python3 -m zipfile`; each entry in its manifest; no secret |
| 6 | `svc-update` | V1R0M1 staged and installed with a partition running and undisturbed, activated with the partitions shut down first; the host confirms the new release, its partitions and pairing as before |
| 7 | `svc-update-rollback` | a trial that never confirms is restarted by its deadline; the old slot with its state exactly as it was, rolled back said, the problem open, a host dump taken |
| 8 | `svc-update-refused` | a damaged image, a wrong signature, another product's, an older one without leave, one that cannot read the state; an activation with a partition running and no leave |
| 9 | `svc-update-power` | power cut at random moments while installing and during the pending slot's first IPL: always a good slot, state whole |
| 10 | `svc-update-mirror` | on a mirrored load source the update is on both disks, and with either removed the host IPLs the new release |
| 11 | `svc-update-back` | from the newer release, the older refused without leave and installed and confirmed with it |
| 12 | `svc-tools` | option 8's panels against their golden files at the serial port; a problem closed, a dump copied to a removable volume, an update installed from one; the problem log |
| 13 | `svc-capture` | three partitions captured over QSI, the host installed again, put back, read back the same |
| 14 | `svc-processor` | a processor without AVX2, ADX, AES and PCLMULQDQ: the host IPLs, the server is down with `A2010010` and its problem, Service Tools works and option 4 says why |
| 15 | `svc-qsictl` | `qsictl` runs the update's good path, lists a problem and closes it |
| 16 | `fuzz --target update-image`, `update-manifest` | 138,828 and 139,125 runs in 60 s, 338 and 289 edges, no finding |

Every suite of the table passed at `26869db`, from images built there
(the test image and a `V1R0M1` test image signed by the same key):
`svc-update-power` cut the power 30 times, each time the host up on a
good slot with its state whole; `svc-hostdump`'s archive held 23
entries; `svc-problem-kill` killed the problem service 4 times among 60
postings and found one problem counted 60 times. What the first runs
found, and what was done:

| Found | Done |
|---|---|
| a word from the network service with no address, sent on its own thread, overtook the one with the address, and the server stopped listening | the word is a cue: the server reads PID 1's fact again (`ea182a7`) |
| an installed image stayed staged, so the next could not be staged | an install takes the staged image (`a4053dc`) |
| the `hold-memory` hook aborted PID 1 when the host had less free than the reserve | it holds what is available, in pieces, never aborting (`9902d92`) |
| the problem of `A2001035` named no slot | PID 1 posts it about the slot that did not confirm (`7af279a`) |
| the harness's stick was attached read-only; option 4 left the serial port signed on; a pairing panel was left before its password was used; `/data` was filled with zeros the hook leaves as holes | the harness (`35ad55f`) |

`svc-capture` (Section 5) found one thing QSI cannot put back: a
partition's creation time, which the host sets when the partition is
made. Everything else of a definition and its profiles, serial numbers
and MAC addresses included, read back the same.

## Hashes

The two clean builds of `7af279a` (build ID `phase9`), each followed by
`build update`, the same to the byte, the update image and the
manifest that lists it included:

```
bfe1a24db83a4200ec3463d235f0dca6f528df066d620bbb92730d3a89d7b9ec  images/initramfs.cpio
babfa5f427e94ddbf3c28e8ae1842b86469482805ba2f67256c2d2696bf8659d  images/initramfs-media.cpio
64ec0232d53a3d83e9a173defbeac33cd1764807ce064dcdec1a52d5650214d0  images/qsvs-V1R0M0-a-64ec0232d53a.efi
5bb745133129f1bbb042b020409be71652925a8cf70ab19c37793dd531fa1bc9  images/qsvs-V1R0M0-b-5bb745133129.efi
8668fe9c018e243d7a5fb369dfac97eec6ceec6010cea38155c2b7def0ac664f  images/qsvs-V1R0M0-d-8668fe9c018e.efi
1a6f68f5887355ba504adb850eed88592fab3be114fc25b286cbc924e77dd359  images/qsvs-V1R0M0-64ec0232d53a.img
eca67548d9ceaf6ecf6fcae4a1b38efb34576cfafffe15318b29d82f5477b553  images/QSVS-V1R0M0-8668fe9c018e.iso
eb3e6810483d9d9037eb2c86ad8d311372d4f550594387eabd572403d868bbab  images/manifest.json
8d2d5e67481e2cdd87083c45ff8d8db40067d55c38f7022417ab5bd0df02cea8  kernel/build/.config
0935db3d04d478eced40b502603bfa133f38affc41f985ff3d5333f10340a614  x86_64-unknown-linux-musl/release/qsvsinit
9f5c4455b100b5ef6a89ef72f01686380fe667bb99c3d9ecf00e9be7d3611349  x86_64-unknown-linux-musl/release/qsvmm
dd7cff085375ced3de05b805affbd6e0354f45351e8d4c02ce41861c994e8eef  x86_64-unknown-linux-musl/release/qsvspartd
101a22f273c4c8a550b32397df7da8723bd44c37fe20e03aeb8be4579e99d63c  x86_64-unknown-linux-musl/release/qsvspoold
d33671195462fb017af8ef32d79e41f601612f9e5dda7e64cfeca9354ea34b26  x86_64-unknown-linux-musl/release/qsvsvswitchd
8a450f068eabaa0d05d4082978e155c8ddcd5f53685c82f5389a3c48a482153c  x86_64-unknown-linux-musl/release/qsvstools
68d5c63fda91aad165ab3aca5143d3218403ddb09157aa406970272905c2bad9  x86_64-unknown-linux-musl/release/qsvsqsid
4980b27ab769c2a620331f783c8644f525df598886bfa1d232e62bd80a184eb4  x86_64-unknown-linux-musl/release/qsvsprobd
2a8126492360b4151e36e3f58c7055e6d44caea8318a1bf5c4938ef232bffeab  x86_64-unknown-linux-musl/release/qsvsupdd
4dbf6f5a667ae46554f7b05d53828ca60fb1a21523817f0757ea93cf821732a1  images/QSVS-V1R0M0-update-4dbf6f5a667a.upd
```

`72fd85a` changed `qsvsinit` after them (below), so its image differs
from these.

## Acceptance

The acceptance run, at `7af279a` in a fresh clone on ext4 (rustc
1.98.0, clang 21.1.8, QEMU 10.2.1, cargo-deny 0.20.2, WSL's 6.18
kernel, 12 processors):

| Step | Result |
|---|---|
| two clean builds, each with its update image | 515 s and 520 s; every hash above the same |
| test image and its update image | built; the hooks in it, none in the release image |
| `doctor`, `deny`, `notices --check` | pass |
| `cargo fmt`, both clippy runs (`qsvsprobd` and `qsvsupdd` among the musl ones) | clean |
| unit tests | 678 passed, none failed |
| `qsictl` | built; in neither image |
| the suites of Phases 0 to 9 | 125 of 130 passed |
| `test --media iso` | the installer's welcome |
| the fuzz targets, 60 s each | 18 targets, no finding |

The 5 that did not pass, and what was done:

| Suite | Why | Fixed |
|---|---|---|
| `supervision`, `installed-base` | the host dump PID 1 takes when a service stops posted `C2011010` after `A2001080`, and it became the current code | `72fd85a`: a host dump's codes are notes of the history |
| `install-serial` | it took option 8 for one not there yet | `e84c99c`: it opens option 8 |
| `partition-tools` (and `install-serial`'s status screen) | the golden status screen had no `probd` or `updd` | `03b0388`: both status goldens written again, the two services the only change |
| `service-host-power-off` | QS/400 in partition 2 stayed at `C6001011 IFS created in ASP 1` and never reported its IPL complete, three levels of virtualization deep | nothing: it passed on a second run, both systems shut down through themselves; QS/400 IPLed many times over on the Dell is still `NEEDS-CODY` (below) |

Each of the 5 then passed with the test image and the release image
built at `ceb9428`, which has every fix above.

| # | Check | Evidence |
|---|---|---|
| 1 | Step 0 is committed on its own and every earlier suite passes on the new pin | `357958f` to `b51e4b6`; 114 of 115 at `b51e4b6`, `qsi-events` after `35ad55f`; every earlier suite in the acceptance run as above |
| 2 | Suites 2 to 16 pass | every Phase 9 suite passed at `26869db` and again in the acceptance run at `7af279a`; the two update fuzz targets found nothing |
| 3 | Open questions 41 and 42 are closed; the joint gate with QS/MC is still recorded as open | `docs/OPEN_QUESTIONS.md` 41, 42 closed, 40 open, 44 raised |
| 4 | Two clean builds are byte-identical, the update image included; `build deny` and `build doctor` pass | the hashes above; `deny` and `doctor` pass |
| 5 | Every QS/400 crate is pinned at the one new commit; QSI's service and update families are QS/400's crates and nothing of them is restated | every `rev =` is `39105ca`; the server's `service` and `update` code is translation to the problem and update services |
| 6 | A host dump holds no secret, and the build fails if a secret file is not left out | `svc-hostdump` read every entry; `hostdump`'s unit tests fail the build when the host's key, the users' file or the consoles' pins is not on `LEFT_OUT` (by the constants their own crates name them with), and when a dump made of a state directory holding them has any of them or a key's or hash's text |
| 7 | An update image that does not verify is never written; the running slot is never written; a slot that does not confirm rolls back with state as it was | `svc-update-refused`, `svc-update-power` (30 cuts), `svc-update-rollback` (every state file the same) |
| 8 | The release image listens on TCP 2300 on the management addresses and nowhere else, and holds neither the hook nor `qsictl` | `sockets`, `qsi-listen`; the release image's initramfs has no hook and no `qsictl` |

## Not done

- `BLOCKED(phase9)`, carried: a VMM found running by a partition manager started again is watched by looking at its control group every 25 ms, not by an event (`docs/PARTD.md`).
- `BLOCKED(phase9)`, carried: an adapter's unit numbers have no gaps (`docs/PROFILE.md`).
- `BLOCKED(phase9)`, carried: cores are given without regard to sockets or to memory's place, and only 2 MB pages are used (`docs/RESOURCES.md`).
- Not in Phase 9, and not started: an update server or any fetch of updates by the host; updating QS/400 inside a partition; physical I/O and tape pass-through (Phase 10); a dump of the host's kernel when it crashes; QS/MC itself.

## For a real machine (NEEDS-CODY)

- `NEEDS-CODY`: an update on the Dell: V1R0M1 staged from `qsictl`, installed, activated, confirmed; then a forced rollback (`fail-trial` is a test image's, so on a release image by cutting the power during the pending slot's first IPL, before it confirms) and the old release back with its state.
- `NEEDS-CODY`: a pool disk pulled by hand from the Dell, its problem read at `qsictl` and at Service Tools option 8 with the disk's location code, and closed once the disk is back.
- `NEEDS-CODY`: a host dump of the Dell copied to a USB stick at option 8 and opened on a workstation.
- `NEEDS-CODY`, carried from Phase 8: pairing at the Dell's console, `qsictl` from a workstation, an install of QS/400 through it, QS/400 IPLed in partitions many times over, and the Dell's processor flags (`docs/OPEN_QUESTIONS.md` item 43).
- The joint gate with QS/MC is open (`docs/OPEN_QUESTIONS.md` item 40).
