# Open questions

What needs a decision, and what was decided. Numbers are kept so older
references still point somewhere. Phase 0's questions 1 to 5 were
answered with the Phase 1 handoff. Questions 7 to 15 were settled on
2026-11-12, when Cody had Claude's recommendation taken for each; the
entry says what that was and what changed. The Phase 2 handoff decided
9 and 13 again, and Phase 2 raised 16 to 22. The Phase 3 handoff
answered those: 16, 18 and 21 are closed below, 20 was decided and
built and waits for Cody to confirm the rule, 17 and 22 are decided in
Phase 8, and 19 stays open. Phase 3 raised 23 to 28, and 29 was
answered with them; the Phase 4 handoff answered 23 to 28, which are
closed below, and left 6, 17, 19 and 22 as they were. Phase 4 raised
30 to 33. Step 0 of Phase 5 closed 19 with QS/400's handoff 15; the
Phase 5 handoff confirmed 30 and 33 as built, left 31 as it is, put 32
with the kernel pin's next change, and left 6, 17 and 22 unchanged.
Phase 5 raised 34 to 36. On 2026-11-14, at the end of Phase 5, Cody
had Claude's recommendation taken for every question still open: 17,
22, 31 and 34 are closed below; 32, 35 and 36 are decided and wait on
a change in QS/400; 6 waits on the first push. The Phase 6 handoff
answered 35 and 36 (below, with what was decided before them) and
left 34, 6, 17, 22, 31 and 32 as they were; Phase 6 raised 37.

## Open

### 6. The QEMU job in CI (NEEDS-CODY)

`.github/workflows/qemu.yml` has not run on GitHub, because nothing is
pushed. Recommended, and done as far as it can be without a push: the
job as it is, on GitHub-hosted runners, with Phase 5's suites in two
shards of their own (`pools`, `media` in `.github/qemu-shards.txt`) and
QS/400's installation ISO built beside its guest image with
`tools/conformance/qs400/pool-install.rsp`, cached by the pin and the
response file. The partition suites run a partition three levels deep;
should the hosted runners not nest that far, the `vmm`, `qs400`,
`partd`, `pools` and `media` shards move to a self-hosted runner on
the Dell machine and the rest stay hosted. Nothing more can be seen
before the first push, which is Cody's.

### 37. Management on a VLAN of a switch (NEEDS-CODY)

The management interface is never a switch's uplink, and a switch's
uplink is never taken for management (`docs/NETWORK.md`), so a host
with one network port has internal switches only. The question is
whether management may later ride a VLAN of a switch: the host would
then have an address on a bridge VLAN of its own, beside partitions,
on a port it shares with them, which is the one thing Phase 6 rules out
(the host has no address on any partition network). Recommended: not
in V1; a host that needs outside networks for partitions has a second
port. If it is wanted later, as a VLAN sub-interface of the uplink's
bond that the bridge never sees, with its own trial as `netd` has
today, and never a VLAN a partition is on.

## Decided, waiting on QS/400

### 32. The kernel is built for 64 threads

`NR_CPUS=64` is what the pinned kernel configuration gives, here as in
QS/400. A host with more threads than that shows 64 of them, and its
partitions can be given no others. Decided: `NR_CPUS=512`, enough for a
two-socket host of 128 cores a socket with SMT, set in QS/400's kernel
fragment and in `kernel/config/qsvs.config` together, with the kernel
pin's next change (the Phase 5 handoff), each in a commit of its own.
QS/400 needs it too: a partition's guest kernel is QS/400's, and it
sees no more processors than its own `NR_CPUS`.

### 35. A copy of a virtual disk (answered by the Phase 6 handoff)

QS/400's `xfsfmt` makes a pool's xfs without reflink, so a copy of a
virtual disk would be a whole copy of its bytes. It is left out
(`docs/PHASE5.md`). Decided: `xfsfmt` gains the reflink feature, and
the reverse mapping it takes, in QS/400; QS/VS then copies a virtual
disk by `FICLONE`, a request of the pool service's, when QS/MC's
template partitions need it. A pool made before that has no reflink
and copies by bytes. The Phase 6 handoff: left out until QS/MC needs
template partitions; the reflink change to `xfsfmt` is a QS/400
handoff then.

### 36. Tape from QS/400 (answered by the Phase 6 handoff)

The virtual tape drive answers what Linux's `st` driver sends, and its
behaviour is shown at the device (`pool-tape`). A save of a library by
QS/400 to it and a restore to another partition were not run: QS/400's
tape code has never met a drive, real or virtual (its `docs/INSTALL.md`
says so), and its handling of the early warning (a block written again
after `ENOSPC`, then `EOV1` labels) needs `st`'s own behaviour past the
warning, which this drive gives as SSC has it. Decided: QS/400's
`INZTAP`, `SAVLIB` and `RSTLIB` against this drive are the first thing
of Phase 10, beside the pass-through to a real one; what QS/400's tape
code needs to change for it is QS/400's, found then. The Phase 6
handoff took it as recommended.

## Closed

1. **The host reference code range.** Confirmed: families `C2`, `A2`,
   `B2`, `D2`, posted as `REF <code> <text>` (`docs/REFCODES.md`).
2. **How a slot is chosen.** Confirmed: by firmware boot entry, as
   QS/400 does. The A/B record holds the default slot, the pending slot
   and whether it was confirmed (`docs/ARCHITECTURE.md`, the slot rule).
3. **The A/B record's payload.** Defined as version 2
   (`lic/hostdisk/src/record.rs`).
4. **Kernel symbols beyond the Phase 0 list.** Accepted as listed.
   Phase 1's additions are in `docs/PHASE1.md`.
5. **The ISO's file name.** Accepted as `buildkit` names it.
7. **Secure Boot.** Stays deferred to release engineering (Phase 11).
   The UKIs are not signed and the manifest records no Secure Boot
   certificate. Nothing in Phases 1 to 3 depends on the answer.
8. **The location code format.** Accepted as `docs/LOCATION.md` has it:
   `S<slot designation>` where the firmware's SMBIOS slot records cover
   the device, `P<segment>-B<bus>-D<device>-F<function>` otherwise, a
   unit suffix for a disk or an interface, `NOPCI-<kernel name>` for a
   device with no PCI function, and an NVMe disk under native multipath
   placed by its subsystem's first controller. One thing changed with
   the decision: slots that share a designation are not used, so their
   devices take the address form and no two devices can have one code.
   Still to be looked at on real hardware (NEEDS-CODY, in
   `docs/PHASE1.md`): QEMU has no slot records, so the slot form is
   unit-tested only.
9. **Time zones.** Fixed offsets (`UTC`, `UTC+hh:mm`) only, in V1. The
   Phase 2 handoff decided it: the zone affects display on a console
   few people look at, and no zone database is added. Service Tools and
   the installer take an offset (`+01:00`); there is no change for
   summer time. Named zones would be a new value of the same
   `time_zone` key, in a later version.
10. **When a seed applies.** A seed is read as long as the host has no
    `host.toml`, on every host alike. The serial number a host derives
    when DMI gives none moved from `host.toml` to a file of its own,
    `identity.toml`, so keeping it no longer marks the host as
    configured: a seed volume attached after the first IPL is read on
    the next one, under QEMU as on a real server. The handoff named
    `host.toml` for the serial; this departs from it, so that a test
    sees what hardware does. The `static-vlan` suite now seeds on the
    second IPL.
11. **What `attempts` is for.** A pending slot gets one trial IPL. The
    update phase writes `attempts = 1`; PID 1 decrements it on the trial
    and shows it. A slot that failed its trial is not tried again by
    itself: the failure is most likely to repeat, and a host that
    restarts into a trial nobody asked for is worse than one that waits
    for the update to be applied again. The byte stays in the record for
    a later rule. No change.
12. **When the A/B record has no valid copy.** PID 1 posts `A2001030`
    and writes a new record whose default is the slot it booted from.
    The partition has QS/VS's own GPT type and holds nothing else, the
    code is in the history, and a host that says the same thing at every
    IPL until a console exists to repair it helps nobody. No change.
13. **Two names in QS/400's `netlink`.** Closed with the pin of Phase 2:
    QS/400's handoff 14 made the crate neutral, and `Route::new` takes
    the route's protocol from its caller. `netd` passes `RTPROT_STATIC`
    or `RTPROT_DHCP` there and sets nothing by hand; it still does not
    use the `nft` module, whose table name is the caller's too now.
14. **The test image is a second kernel build.** Kept. That no file of
    a test build can be among the release artifacts is worth more than
    the three gigabytes and the one kernel build it costs on a clean
    machine. A shared kernel tree would need a `Product` field in
    `buildkit` for the output directory; to be asked for only if the
    build time becomes a burden.
15. **Ctrl-Alt-Del.** Ignored. PID 1 still takes the key from the kernel
    (`CAD_OFF`), so it cannot reset the machine under a mounted `/data`,
    and says `Ctrl-Alt-Del ignored` on the console and in the log. It
    was an orderly restart before; on a host that runs partitions, three
    keys pressed at the wrong keyboard should not end them all. The
    shutdown triggers are the handoff's three: the control socket,
    `SIGTERM` to PID 1 and the power button.

16. **What brings menu options 4 and 8.** The phases have numbers now
    (`CLAUDE.md` has the table, 0 to 11). Option 4, management console
    pairing, comes with Phase 8, the QSI host server; option 8, service
    functions, with Phase 9, service and code update. The menu and its
    message lines say `Phase 8` and `Phase 9`, and so does
    `docs/SERVICETOOLS.md`.
18. **Whose the load source is.** Every appliance's: QS/MC gets the
    same mirrored load source. The layout moved from `lic/hostdisk` to
    `base/loadsource` and the storage service's engine from
    `lic/qsvsstgd` to `base/stgd`. What is an appliance's own (the GPT
    types and names of the record and data partitions, the ESP's vendor
    directory, label and volume id, the magics of the A/B record, of
    the set's label and of the resume map, the seed of a built image's
    GUIDs) is `appliance::LoadSourceNames`, part of the `Appliance`
    description; QS/VS's values are `release::LOAD_SOURCE`, the bytes
    they were, and a test holds them there. `lic/qsvsstgd` is the
    binary that runs the engine as QS/VS, with QS/VS's inventory for
    the disks' serial numbers and location codes; `lic/hostdisk` is
    gone. The storage service's reference codes go through
    `appliance::Event` like the other base services'. Each appliance
    keeps its own member label, under its own magic: there is no
    shared on-disk label written by `protect`.
20. **A pair where each disk ran alone.** Confirmed as built on
    2026-11-13: no winner is picked (`base/loadsource/src/set.rs`,
    `apart`). The storage service posts `A2003040` and assembles
    nothing, PID 1 mounts no `/data` and starts no other service, and
    the console asks which unit to keep, without limit and without a
    sign-on: being at the console is the authority, as for the
    installer. A disk that ran alone while the other was merely away
    is still taken as the newer one without a question. The A/B
    record still follows the higher sequence of the two disks. The
    label is version 2, with the time written; no release from before
    it has shipped.
21. **PID 1 does not supervise during its own IPL steps.** Left as it
    is: the steps between the storage service being ready and the
    services being started (`/data` mounted, the clock, the seed, the
    state, the identity) run without the loop, for about a second at
    an IPL. A storage service that ended exactly then is started again
    when they are done.
29. **Seven bytes at the reset vector.** Kept, on 2026-11-13. An
    unmodified QS/400 restarts by a jump to the reset vector
    (`docs/QS400_GUEST.md`), and `qsvmm` keeps seven bytes there that
    pulse the reset line it already has (`docs/VMM.md`). "No guest
    firmware" stands as it was meant: nothing of the VMM's runs before
    the guest's kernel, and the IPL is the VMM's. A QS/400 that
    restarts by the ACPI reset register in a partition would make them
    unused, not wrong; ACPI that is not hardware-reduced was not taken.
23. **The side of a partition's next IPL.** Closed with the pin of
    Phase 4 (`e035bea`): QS/400 records the side for one IPL and the
    default side in its LIC record when it has no firmware
    (`licpart::IplSource`), and the IPL loader takes it, once an
    activation (`vmm/ipl/src/lic.rs`). A profile's `recorded` source
    is what the partition itself asked for; side A or B in an
    activation overrides it and leaves the record alone. The suite
    `partition-recorded` shows it from the guest's side.
24. **What QS/VS restates of QS/400.** Nothing any more: the console
    mode is `licpart::ConsoleMode`, and the sides' paths are
    `diskimg::SLOT_A_UKI` and `SLOT_B_UKI`.
25. **A partition's identity.** QS/VS writes SMBIOS tables for each
    partition: the manufacturer `Quartz Systems`, the product `QS/VS
    partition`, and a system serial number made once, when the
    partition is created, from the host's serial number and the
    partition's ID, in the format `QSRLNBR` holds; it is in the
    definition and never changes (`docs/PROFILE.md`, `docs/VMM.md`).
    QS/400 at `e035bea` reads it as it is (`docs/QS400_GUEST.md`).
26. **Which disks the host takes.** The disk it IPLed from and the
    members that disk's label names, and no other
    (`base/loadsource/src/own.rs`). The disk of the IPL is the one the
    firmware's `BootCurrent` entry names, by the PCI functions of its
    device path or the GUID of its ESP; with no firmware to ask it is
    the first disk in name order that has the three partitions. The
    harness's wrapper around guest images is gone, and the suite
    `partition-own-disks` shows a guest's disk with QS/VS's own
    partitions and identifiers left as it was.
27. **Whose the keylock is.** The host's. The definition holds it, it
    is written to the LIC record at every activation, and what the
    guest sets for itself lasts until the next one
    (`partition-attributes`).
28. **One user for every partition.** Each partition's `qsvmm` runs as
    its own user and group, 23000 plus the partition's ID; its serial
    log and its console socket are in directories of that user alone
    (`partition-isolation`).
19. **Resynchronization after a power loss is the whole partition.**
    Closed with the pin of Phase 5 (`0fc8afa`): QS/400's handoff 15
    gives `protect`'s mirrored set a write-intent bitmap, and the
    mirrored load source keeps it in the tails of the set label's two
    copies, which the label never reaches: the set's map in the first,
    the map of a member out of service in the second
    (`base/loadsource/src/intent.rs`). Nothing an installed host has on
    disk moved; a host installed before reads zeros there, takes every
    region as dirty once, and keeps the map from then. After a power
    loss only the regions the map has set are copied, from the first
    unit in step to the other, and a stale unit comes back by what was
    written while it was away. The suite `mirror-power-pull` shows the
    copy bounded after each of its twenty cuts.

30. **What a profile's IPL type is.** Confirmed as built (Phase 5
    handoff): `attended` or `unattended`, with the keylock in manual
    making an IPL attended whatever the profile says
    (`docs/PROFILE.md`).
33. **What a processing unit is worth where a core has two threads.**
    Confirmed as built (Phase 5 handoff): a capped partition's quota is
    its units times two in thread time, exact under full load.
    `docs/RESOURCES.md` says what a load that keeps one thread busy
    gets.
17. **The host identity's key.** Decided on 2026-11-14: it stays
    Ed25519. QSI's TLS stack (Phase 8) is to take Ed25519 certificates:
    TLS 1.3 has the scheme, a pure-Rust provider has it, and QS/MC, the
    only peer, is Quartz's own. A host installed now keeps the identity
    it made. Were the stack chosen in Phase 8 not to take it after all,
    the fallback is as before: a P-256 identity, made again at the next
    IPL of every host.
22. **Who owns the service tools users.** Decided on 2026-11-14: PID 1,
    as it owns the host's settings. The console and, in Phase 8, the
    QSI host server ask it to add, change and remove a user and to
    check a sign-on; nothing else opens the users' store, and the rule
    that the local console edits no state file then holds for the users
    too. Built with the QSI host server in Phase 8, which is the first
    second client; until then `svc/tools` keeps them as now.
31. **A VMM ended by a signal the partition manager did not send.** Left
    as it is (Phase 5 handoff): Not activated, as for its own request.
    Nothing in a release image sends that signal.
34. **A held partition's state for QS/MC.** Confirmed on 2026-11-14:
    Running, with `A2005160` as its current reference code until it goes
    on by itself, as built (`docs/STORAGE.md`). QSI carries the state
    and the code; QS/MC shows the code beside the state.
