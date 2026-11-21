# The base crates

What an appliance other than QS/VS (QS/MC first) takes from `base/`, and
what it must give each crate of its own. Everything under `base/` is
product-neutral: no crate names QS/VS in its code outside its tests and
comments, and none depends on a crate of this repository outside
`base/` (`build doctor` and `tools/build/src/host/neutral.rs`'s test
fail otherwise). An appliance takes them as git dependencies on this
repository at one commit, with QS/400's crates at the commit this one
pins (`docs/PHASE0.md`). `build fixture` builds `QSDEMO`
(`tools/fixture`), an appliance of no product's, from them that way at
HEAD and runs each part.

## Each crate and what the product gives it

| Crate | What it does | What the product gives |
|---|---|---|
| `appliance` | the `Appliance` description, its load-source names, the events a base service posts, civil time, the Ethernet links | its identifier, name, banner, data and run directories, host-name prefix, seed label, interface roles, `LoadSourceNames`, how an event is posted as its code, how PID 1 is told a fact |
| `state` | typed TOML state files: schema versions, migrations, atomic writes; the host, identity, inventory and network documents | its own documents and their versions |
| `supervise` | the service table, restart policy, the runner, PID 1's control protocol, signals | its table of services |
| `logd` | the NDJSON system log, the reference code history, each boot's kernel log; `logd::panics`, what the kernel kept of a panic taken at the next IPL | the directory; the words and code of the panic it posts |
| `seed` | `seed.toml` on a labelled FAT volume, the `[install]` table | the label |
| `netd` | the network interfaces with their roles, DHCP or static, VLANs, the route, DNS and time each carries, a change on trial, NTP; a line noted for a role a change removes | the roles, the management interface's role, the notes' keys |
| `identity` | the key pair and self-signed certificate made at the first IPL | the name and serial |
| `users` | service tools user IDs, the password rules and the sign-on rule | the first user |
| `panels` | panels with help, the console on the display and the serial port, the common forms, the Service Tools panels every appliance has, the unit to keep; `panels::units`, the load source's units, protection started, a unit replaced and its rebuild watched; the power panel | its menu; for `units`, a `LoadSource` (the storage service, the machine's disks) and a `Product` (its own options, the help, the note on the other disks, its main menu); for `power`, `PowerWords` (what ends with the machine) |
| `install` | the installer's screens, the answers of an unattended installation; `install::write`, the eight steps that write a load source from verified media | its `Target` (its words, its disks and links, the smallest disk, each step's code); for `write`, a `Product` (its load-source names, its name for the boot entries, the codes it logs) |
| `loadsource` | the load-source disks: which are the appliance's own, the GPT layout, the A/B record, the set's label, units that ran apart, the boot entries; `loadsource::payload`, the media's payload, its search by volume identifier, its mount and its check against the trusted keys | its `LoadSourceNames`, the trusted keys' file, where the media is mounted, its release |
| `stgd` | the storage service: the mirrored load source assembled, served through `ublk`, resynchronized by its write-intent map, rebuilt; the disks looked for once a start | a `Host` (its inventory) |
| `fdpass` | open files handed between processes over a Unix socket | |
| `stream` | the release stream's signed index: entries with their dates and notes, signed, verified, never older than one seen | the stream's name, its keys |
| `processor` | the processor features QSI's cryptography needs, and which this one lacks | its words |
| `inventory` | the hardware inventory and its location codes | its own fields beside it |
| `qemu` | a conformance harness's QEMU driver | its variables and device places |
| `msg` | messages: a catalogue entry (`Message`), one issued with its values (`Issued`), a catalogue as a table | its catalogue, prefix and words |
| `problem` | the problem log: one record a condition, opened, counted and cleared from the posted history by the product's table, notes, severity, close, bounded; the service and its socket | `table::Rules` (its conditions, each condition's severity, the five messages it refuses with); a `serve::Host` (its identifier, directories, the code it posts when its data file system fills and how) |
| `servicedump` | the service dump: the ZIP archive, the store with its bounds, what every appliance gathers (logs, problems, state, the kernel's view), the rule that leaves secrets out and the test that holds the base crates' secret files to it | a `Kind` (its dumps' prefix, the manifest's title, its bounds, its own secret files) and `Sources` (its own entries) |
| `confine` | a process confined: namespaces and an empty, read-only root, a user with no capability, a seccomp filter of its calls; a learning mode that reports a refused call | each process's and thread kind's allowed calls (`bpf::Rule`) |
| `update` | code update: the image and its manifest, verification against trusted keys, staging, installing into the other slot on every load-source disk, the pending slot, activation, confirmation and roll back, the service and its socket; `update::state_copy`, the state kept across a trial and put back when it rolls back | a `Format` (its image's magic, its manifest's head, its fixes' prefix), its name, release and state schemas, `Messages`, how each `Code` is posted, the `Partitions` (or anything) stopped before the restart |
| `buildaid` | beside QS/400's `buildkit`: the license header check, the wording audit and the help audit, the release (keys, a development key refused, Secure Boot signing, `SHA256SUMS`, the stream's index, two builds compared), the update image builder, the catalogue (`quartz.catalogue/1`) and its audit, the fuzzer's run | its `buildkit::Product`, `release::Names` (its keys' prefix, the db certificate's subject, its stream), its update `Format` and schemas, its catalogue `Tables` (its codes, their document, its problem rules, its messages), its `Fuzzer` (package, binary, corpus) |

What stays QS/VS's: its menu and its host-specific panels (`svc/tools`),
its partitions, pools, switches and the control groups in a host dump
(`svc/hostdump`), its problem table and messages (`lic/qvsmsg`), its
update format, schemas, messages and the partitions it ends
(`lic/qsvsupdd`), its VMM's policy (`vmm/qsvmm/src/confine`), its
build's steps, tables, suites and names (`tools/build`,
`tools/conformance`).

## Handoff of QS/400's handoff 24: what each item became

Every item of the prompt "Everything QS/MC's remaining phases need from
QS/VS", with the commit that did it.

| Item | What | Commit |
|---|---|---|
| Step 0 | every `rev =` at QS/400 `cf8f014`; `qsiserver`'s matches over the new operations | `75551df` |
| A1 | switches made, changed, renamed, deleted, given and cleared an uplink over QSI, the management interface refused | `49b117c` |
| A2 | storage pools over QSI; the pool service's candidates (request 26); extend refused `QSI1616` and `QVS3019`; a foreign disk confirmed by the console's flag | `49b117c` |
| A3 | a partition created at a chosen ID (`partd` 44; `QVS2016`, `QVS2017`) | `49b117c` |
| A4, A5 | a definition's settings, schema version 2 (`partd` 45, 46): description, panel functions allowed from a console (`QVS2318`), how it ends with the host and its delay; the host's end of each by its own settings | `49b117c` |
| A6 | running since, a profile's last activation (`partd` 47, 48; `activations.toml`) | `49b117c` |
| A7 | start partitions with the host, the default power off and its wait (`partitions.toml` version 2, `partd` 52, PID 1, Service Tools option 9, 3) | `49b117c` |
| A8 | the panel functions from the table `docs/PANEL.md` is held to (`partd` 49) | `49b117c` |
| A9 | a note added to a problem, severity from the table (problem service 4 to 6, `QVS5005`) | `49b117c` |
| A10 | dump space: the host's dumps, each partition's (`partd` 51) | `49b117c` |
| A11 | a release's date and notes in the stream's index (`base/stream`, `build release`) | `49b117c` |
| Part 1's proof | `qsi-additions`, every addition over QSI against the services, each refusal, a restart | see `docs/PHASE11.md` |
| P1 | `base/update` with `update::state_copy`; QS/VS's format, schemas, messages and partitions in `lic/qsvsupdd` | `7ba9dca` |
| P2 | `base/servicedump`; `svc/hostdump` QS/VS's entries and the pins left out | `49b117c` |
| P3 | `base/problem` over `base/msg`; QS/VS's table `qvsmsg::problems::RULES` | `49b117c` |
| P4 | `base/confine`; the VMM's policy stays in `qsvmm` | `49b117c` |
| P5 | `logd::panics` | `49b117c` |
| P6 | `base/buildaid`: header, wording and help audits, release, stream, signing, update image, catalogue, fuzz runner | `3a87c5b` |
| P7 | `loadsource::payload`: the media's search, mount and check (QS/MC item 15) | `49b117c` |
| P8 | `panels::units` behind `LoadSource` and `Product` (QS/MC item 16) | `49b117c` |
| P9 | `install::write` (QS/MC item 18) | `49b117c` |
| P10 | `panels::tools::PowerWords` (QS/MC item 17) | `49b117c` |
| Fixture | QSDEMO stages, verifies and refuses an image of itself, rolls a trial back and confirms one, takes a dump with secrets left out, opens, notes and closes a problem, runs a confined child, builds a release directory with its signed index, takes a panic's records, checks its media | `1c2f4fb` |
| Part 3, item 14 | the load-source disks looked for once a start | `ced5a19` |
| Part 3, item 13 | `protect` starts the map of a member absent at a clean open (QS/400 `7e65f618e`, at the pin); `base/stgd` gives it every member's state and away store, so nothing changed here; `mirror-degraded` puts the disk back and requires it back by its map | `4053b26` |
| Part 3, item 28 | a line noted for each role a change removes | `b12c2b4` |
| Part 3, other QS/VS items | items 30 and 33 are A7 and A8; item 34 is the outside harness (Part 4); items 6, 8, 9 (CI, the ISO's name, Secure Boot) are QS/MC's or Cody's, not QS/VS's to change | |
| Part 4 | `docs/CONFORMANCE.md`; `docs/catalogue.json`, the catalogue as data at the pinned commit, held to the tables | `0407404` |

### Not done

- **The limits and soak helpers** (P6's last words) stay in
  `tools/conformance` (`suite/limits`, `suite/soak`): each is a QEMU
  suite of a host's partitions, pools and switches, nothing an
  appliance without them would run as it is. The driver they stand on
  is `base/qemu`'s already.
- **The fixture's update** is staged, verified, rolled back and
  confirmed without a machine: installing into a slot and the restart
  need PID 1's slots and a load source, which `QSDEMO` is not. QS/VS's
  update suites (`servicing/`) show them over `base/update`.
