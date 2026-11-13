# Open questions

What needs a decision, and what was decided. Numbers are kept so older
references still point somewhere. Phase 0's questions 1 to 5 were
answered with the Phase 1 handoff. Questions 7 to 15 were settled on
2026-11-12, when Cody had Claude's recommendation taken for each; the
entry says what that was and what changed. The Phase 2 handoff decided
9 and 13 again, and Phase 2 raised 16 to 22.

## Open

### 6. The QEMU job in CI (NEEDS-CODY)

Unchanged from Phase 0: `.github/workflows/qemu.yml` has not run on
GitHub, because nothing is pushed. Whether the hosted runners offer
nested virtualization, or a self-hosted runner is needed, is to be seen
on the first push; the job fails with a message that says which when
`/dev/kvm` or nested virtualization is missing. The job builds both
images and runs every suite: two kernel builds, and about five minutes
for the power-pull suite. Nothing here can be decided before a push.

### 16. What brings menu options 4 and 8 (NEEDS-CODY)

The handoff has Service Tools show options 4 (management console
pairing) and 8 (service functions) as not yet available, "with the
phase that brings them". This repository numbers phases 0 to 3 only,
and neither is in those. The menu says what brings each instead:
`comes with QSI` and `comes with code update`. If the phases have
numbers, they go on the menu and in `docs/SERVICETOOLS.md`; and if
service functions come with something other than code update, the
wording is wrong.

### 17. The host identity's key

The identity made at the first IPL is an Ed25519 key pair and a
self-signed X.509 certificate, written field by field in `base/identity`
(some eighty bytes of DER around the key and the signature), with
QS/400's own `ed25519-dalek` version: no further dependency, pure
Rust. Nothing uses it yet. The QSI phase chooses a TLS stack; if that
stack does not take Ed25519 certificates, the identity has to be a
P-256 one and hosts installed before then make a new one. Recommended:
decide with the QSI phase, before any host is paired.

### 18. Whose the load source is

The handoff puts the load-source layout and its protection in
`lic/hostdisk` and `lic/qsvsstgd`, and they are QS/VS's: the partition
types, the record's magic, the set label `QSVSSET`. QS/MC is an
appliance with a load source too. If it is to have the same mirrored
load source, those two crates move to `base/` with their names taken
from the `Appliance` description, as the base services have them.
Related, and QS/400's question (`docs/BUILDKIT.md` section 10 there):
whether the appliances should share one on-disk member label written
by `protect`. QS/VS keeps its own, beside the A/B record.

### 19. Resynchronization after a power loss is the whole partition

A mirrored load source that was cut off writes its whole second unit
again from the first: there is no write-intent bitmap, so the service
cannot know which megabytes were in flight. The map of the copy
survives a further power loss, the host runs meanwhile, and the copy
gives way to the host's own I/O. On a load source of several hundred
gigabytes that is still a long time without protection. A bitmap in the
record partition would bound it to what was being written. Recommended:
decide when the first real machine shows how long it takes.

### 20. A pair with no third party

If each disk of a mirrored pair is started alone in turn, each records
the other as missing. When both are back, the record with the higher
sequence wins and the other disk is written from it: what the loser
alone had written is lost. Two disks cannot settle that between
themselves. The operator's rule is in `docs/ARCHITECTURE.md`: do not
start the host from a disk that was pulled while the other ran.
Whether to refuse the IPL instead, when both labels say the other is
missing, is a decision: refusing is safer and leaves a host that will
not start until someone chooses.

### 21. PID 1 does not supervise during its own IPL steps

The storage service is started first and the loop is turned until it
is ready. The steps that follow (`/data` mounted, the clock, the seed,
the state, the identity) run one after another without the loop: a
storage service that ended exactly then would not be started again
until they are done, and they wait on `/data`. The window is a second
at an IPL. Closing it means turning the loop from inside each step;
left as it is.

### 22. Who owns the service tools users

The handoff has the console ask the owning service for every change.
The users have no service but the console itself: nothing else signs
anyone on. So `svc/tools` keeps them, through the users' store of
`base/users` (QS/400's `svcusers` file store), and the two sessions
share one copy in memory. If QSI is to manage them from QS/MC, they
need an owner both can ask: PID 1, or a service of their own.

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
