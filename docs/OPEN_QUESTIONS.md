# Open questions

What needs a decision, and what was decided. Numbers are kept so older
references still point somewhere. Phase 0's questions 1 to 5 were
answered with the Phase 1 handoff. Questions 7 to 15 were settled on
2026-11-12, when Cody had Claude's recommendation taken for each; the
entry says what that was and what changed. The Phase 2 handoff decided
9 and 13 again, and Phase 2 raised 16 to 22. The Phase 3 handoff
answered those: 16, 18 and 21 are closed below, 20 was decided and
built and waits for Cody to confirm the rule, 17 and 22 are decided in
Phase 8, and 19 stays open. Phase 3 raised 23 to 28.

## Open

### 6. The QEMU job in CI (NEEDS-CODY)

Unchanged from Phase 0: `.github/workflows/qemu.yml` has not run on
GitHub, because nothing is pushed. Whether the hosted runners offer
nested virtualization, or a self-hosted runner is needed, is to be seen
on the first push; the job fails with a message that says which when
`/dev/kvm` or nested virtualization is missing. The job builds both
images and runs every suite: two kernel builds, and about five minutes
for the power-pull suite. Nothing here can be decided before a push.

### 17. The host identity's key

The identity made at the first IPL is an Ed25519 key pair and a
self-signed X.509 certificate, written field by field in `base/identity`
(some eighty bytes of DER around the key and the signature), with
QS/400's own `ed25519-dalek` version: no further dependency, pure
Rust. Nothing uses it yet. If the TLS stack of QSI does not take
Ed25519 certificates, the identity has to be a P-256 one and hosts
installed before then make a new one. The Phase 3 handoff: decide in
Phase 8, with the TLS stack. Nothing changed.

### 19. Resynchronization after a power loss is the whole partition

A mirrored load source that was cut off writes its whole second unit
again from the first: there is no write-intent bitmap, so the service
cannot know which megabytes were in flight. The map of the copy
survives a further power loss, the host runs meanwhile, and the copy
gives way to the host's own I/O. On a load source of several hundred
gigabytes that is still a long time without protection. A bitmap in the
record partition would bound it to what was being written. The Phase 3
handoff: it stays open until a real machine shows how long it takes.

Storage pools (Phase 5) cannot wait for that answer: a pool is far
larger than a load source, and writing a whole unit of it again after
every power loss is not acceptable. They need a write-intent bitmap in
QS/400's `protect` before Phase 5, which is a change in QS/400's
repository and a new pin here; the load source then takes it too.

### 20. A pair with no third party (NEEDS-CODY, to confirm the rule)

If each disk of a mirrored pair is started alone in turn, each records
the other as missing and goes on. Until Phase 3 the record with the
higher sequence won when both were back, and the other disk was
written from it: what the loser alone had written was lost, silently.

The Phase 3 handoff decided: no winner is picked. The rule, as built
(`base/loadsource/src/set.rs`, `apart`): the copies of the set's label
are read from each disk by itself; when one disk's copy has its own
unit in step and another's not, and that other disk's copy says the
reverse, the units ran apart. The storage service then posts `A2003040`
and assembles nothing; PID 1 mounts no `/data` and starts no other
service; the display and the serial port show `Select Load-Source Unit
to Keep` with each unit's location code, serial number and the time
its disk was last written (`docs/SERVICETOOLS.md`). The operator's
choice is recorded by the storage service in the label on both disks
(`C2003050`), the IPL goes on from the unit kept, and the other is
written again from it. The host waits for the choice without limit.
The suite `mirror-split` shows all of it.

What Cody is asked to confirm:

- The rule itself, and that the host waits for ever: a host that was
  split and then loses power comes back to a screen, not to service,
  until someone is at the console (or at its serial port through a
  BMC).
- That the screen needs no sign-on. The users are on `/data`, and
  which `/data` is the question; being at the console is the
  authority, as for the installer.
- That a disk which ran alone while the other was merely away, and
  never started, is still taken as the newer one without a question
  (it is: only one side then says the other was missing).
- That the A/B record, which is read before the storage service
  starts, still follows the highest sequence of the two disks. It
  holds the default slot and the releases confirmed, not data.
- The label's version 2 (the time written). A release from before it
  does not read a version 2 label: a host that goes back to such a
  release in its other slot would find its set not assembled. No such
  release has shipped.

### 22. Who owns the service tools users

The handoff has the console ask the owning service for every change.
The users have no service but the console itself: nothing else signs
anyone on. So `svc/tools` keeps them, through the users' store of
`base/users` (QS/400's `svcusers` file store), and the two sessions
share one copy in memory. If QSI is to manage them from QS/MC, they
need an owner both can ask: PID 1, or a service of their own. The
Phase 3 handoff: decide in Phase 8, when QSI may need to manage them.
Nothing changed.

### 23. The side of a partition's next IPL (NEEDS-CODY: a QS/400 handoff)

QS/400 chooses the side of its next IPL through firmware variables
(`BootNext`, `BootOrder`). A partition has none, and the request is
lost: `IPLSRC(B)` at a power down says `IPL source not set: not booted
through UEFI` (`docs/QS400_GUEST.md`). The LIC record has no field for
it that `licpart` owns: the default and pending slots are in a TLV of
QS/400's `ptf` crate. So `side = "recorded"` in a profile means the
side of the last IPL that completed. Recommended: QS/400 records the
once-only slot and the default slot through `licpart` when it has no
firmware, and `vmm/ipl` honours them. Needed before a partition can
apply a LIC PTF, so before Phase 4 calls the IPL source the
partition's own.

### 24. What QS/VS restates of QS/400 (a QS/400 handoff)

Two things the IPL loader knows of QS/400 that it does not get from
`licpart`: the console mode's TLV tag and its two values, which are in
QS/400's `svctools` (`vmm/ipl/src/lic.rs`), and the two paths of the
sides' UKIs on the ESP, which are in `slic` (`vmm/ipl/src/lib.rs`).
Each is a constant, not copied source. Recommended: QS/400 moves the
console mode into `licpart` and exports the paths from `diskimg`, and
QS/VS takes them from there at the next pin.

### 25. A partition's identity

A partition has no SMBIOS tables. QS/400 so has no machine serial
number (`QSRLNBR 0000000` in every partition) and cannot name its
hypervisor (`unidentified hypervisor` in `QSHWQUAL`). Two ways: QS/VS
writes SMBIOS tables for a partition, with a serial number from its
profile; or QS/400 learns a partition from QS/VS's ACPI tables and the
service channel of Phase 7. Recommended: decide with Phase 4's
profiles, which is where a partition's serial number would be kept.

### 26. Which disks the host takes for its own load source

The host takes every disk whose table has its three partition types
for a load-source disk: it writes its A/B record to each. That was
right when every disk was the host's. A partition's disk can now look
like one (a QS/VS image as a guest's disk, which only the conformance
harness has; but also a disk moved from another host). The harness
wraps its guest image so the host does not see it
(`tools/conformance/src/suite/partition/devices.rs`). Recommended,
before a partition is given a whole disk (Phases 5 and 10): the host
takes the disk it IPLed from and the members its set's label names,
and no other.

### 27. Whose the keylock is

The profile's `keylock`, when it has one, is written to the guest's
LIC record at every activation; the guest's own `mode manual` at its
service console writes the same field for its next IPL. Whoever wrote
last wins. On a machine the keylock is a physical key. Recommended:
decide with Phase 4's panel functions whether a partition's keylock is
the host's alone.

### 28. One user for every partition

`qsvmm` runs as user and group 23000 once it has opened its devices:
the same for every partition, so one partition's process could signal
another's or read its serial log. With one partition that is nothing.
Phase 4 gives each partition its own.

### 29. Seven bytes at the reset vector (NEEDS-CODY)

An unmodified QS/400 restarts by a jump to the reset vector
(`docs/QS400_GUEST.md`), and a partition has no firmware there. `qsvmm`
puts seven bytes there that pulse the reset line it already has, so
that `pwrdwn restart` ends the partition with `C2005110`. Nothing runs
before the kernel and the IPL is still the VMM's, but it is guest code
the VMM supplies, which "no guest firmware" did not foresee. The
other ways: QS/400 restarts by the ACPI reset register when it is a
partition (`reboot=acpi` in its baked command line, or the call in
`slic`), a QS/400 change; or ACPI that is not hardware-reduced, with
the PM1 blocks and the SCI that come with it. To confirm, or to say
which.

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
21. **PID 1 does not supervise during its own IPL steps.** Left as it
    is: the steps between the storage service being ready and the
    services being started (`/data` mounted, the clock, the seed, the
    state, the identity) run without the loop, for about a second at
    an IPL. A storage service that ended exactly then is started again
    when they are done.
