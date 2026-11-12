# Open questions

What needs a decision, and what was decided. Numbers are kept so older
references still point somewhere. Phase 0's questions 1 to 5 were
answered with the Phase 1 handoff. Questions 7 to 15 were settled on
2026-11-12, when Cody had Claude's recommendation taken for each; the
entry says what that was and what changed.

## Open

### 6. The QEMU job in CI (NEEDS-CODY)

Unchanged from Phase 0: `.github/workflows/qemu.yml` has not run on
GitHub, because nothing is pushed. Whether the hosted runners offer
nested virtualization, or a self-hosted runner is needed, is to be seen
on the first push; the job fails with a message that says which when
`/dev/kvm` or nested virtualization is missing. The job builds both
images and runs every suite: two kernel builds, and about five minutes
for the power-pull suite. Nothing here can be decided before a push.

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
9. **Time zones.** Fixed offsets (`UTC`, `UTC+hh:mm`) in the base
   system. The zone affects display only, and nothing lets an operator
   pick one before the Service Tools console exists. Named zones
   (`Europe/Berlin`) are taken up with that console in Phase 2, as a new
   value of the same `time_zone` key, with the zone database then
   chosen. No change.
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
13. **Two names in QS/400's `netlink`.** `netd` goes on using the crate
    as it is and sets the route protocol itself; it does not use the
    `nft` module. The change that makes the crate neutral (the protocol
    number and the table name from the caller) is to be made in QS/400
    and the pin moved, before the virtual Ethernet phase or QS/MC lean
    on the crate. It is listed in `docs/PHASE1.md`; it cannot be made
    from this repository.
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
