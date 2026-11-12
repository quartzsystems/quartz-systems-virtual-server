# Open questions

What needs a decision. Each says what was done meanwhile. Phase 0's
questions 1 to 5 were answered with the Phase 1 handoff and are closed;
their numbers are kept so older references still point somewhere.

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

## 6. The QEMU job in CI (NEEDS-CODY)

Unchanged from Phase 0: `.github/workflows/qemu.yml` has not run on
GitHub, because nothing is pushed. Whether the hosted runners offer
nested virtualization, or a self-hosted runner is needed, is to be seen
on the first push. The job now also builds the test image and runs the
ten base suites, which makes it longer: two kernel builds, and about
five minutes for the power-pull suite.

## 7. Secure Boot

Deferred to release engineering (Phase 11). The UKIs are not signed and
the manifest records no Secure Boot certificate.

## 8. The location code format (NEEDS-CODY)

`docs/LOCATION.md` defines one format: `S<slot designation>` where the
firmware's SMBIOS slot records cover the device, `P<segment>-B<bus>-
D<device>-F<function>` otherwise, with a unit suffix for a disk or an
interface. It is a proposal. Three choices in it want a look: the code
is derived from the designation text, which a firmware update can
change; a device with no PCI function is `NOPCI-<kernel name>`, which is
only as stable as the kernel's name; and an NVMe disk under native
multipath is placed by its subsystem's first controller.

## 9. Time zones are fixed offsets

`host.toml`'s `time_zone` is `UTC` or `UTC+hh:mm`. It affects display
only, as the handoff says, so nothing but the status screen reads it.
A named zone (`Europe/Berlin`) needs the zone database in the image and
its rules for daylight saving time; that is a few hundred kilobytes and
a dependency to choose. The Service Tools console of Phase 2 is where an
operator would pick one. Say whether named zones are wanted, and the
seed and state formats take them as a new value of the same key.

## 10. A seed applies once, and a host without a DMI serial is seeded on its first IPL only

The seed is read when `<data>/state/` holds no `host.toml`
(`docs/SEED.md`). The handoff also has the host keep a serial number
derived from the first NIC in `host.toml` when DMI gives none, which
writes that file on the first IPL. So on such a host (every QEMU guest;
no real server is expected to be one) a seed volume attached after the
first IPL is not read. Keeping the derived serial in a file of its own
would lift that; the handoff names `host.toml`, so that is what is done.

## 11. What `attempts` is for

The record's version 2 has "attempts left for the pending slot". With
the rule as given, a pending slot is booted once through `BootNext` and
either confirms or is cleared on the next IPL, so the field only ever
goes from what the update phase wrote to one less. PID 1 decrements it
on the trial IPL and shows it. If the update phase is meant to give a
slot several tries, the fall-back IPL would set `BootNext` again while
attempts are left instead of clearing the pending field; that is a
change to one function (`HostRecord::at_ipl`). To settle with the update
phase.

## 12. When the A/B record has no valid copy

A disk built by Phase 0 has a version 1 record, and a damaged disk may
have none. PID 1 then posts `A2001030` and writes a new record whose
default is the slot it booted from, so the host is usable and the next
IPL is quiet. The alternative is to leave the partition alone and post
the code at every IPL until Service Tools repairs it. Writing was
chosen because the partition has QS/VS's own GPT type and holds nothing
else.

## 13. Two things in QS/400's `netlink` that carry its name

`netd` uses QS/400's `netlink`, `dhcp` and `ntp` as they are
(`docs/PHASE1.md` has the check of each). Two names in `netlink` are
QS/400's own and are worked around rather than used: `Route::new` marks
a route with `RTPROT_QS400` (81), so `netd` sets the route's protocol
field itself (`RTPROT_STATIC`, `RTPROT_DHCP`); and the `nft` module's
table is called `qs400`, which `netd` does not touch (the QS/VS kernel
has no netfilter). A change in QS/400 that took the protocol number and
the table name from the caller would make the whole crate neutral. Not
needed for Phase 1; worth doing before QS/MC or the virtual Ethernet
phase lean on the crate.

## 14. The test image is a second kernel build

The test image is built under `<target>/test-image/`, a whole target
root of its own, so that no file of a test build is ever among the
release artifacts. The cost is a second kernel tree (about three
gigabytes and one more kernel build on a clean machine). `buildkit`
keeps the images directory under the target root and has no notion of a
second set of UKIs for one kernel build; a `Product` field for the
output directory would let the two share the kernel objects. Not asked
for; say if the build time matters.

## 15. Ctrl-Alt-Del

PID 1 takes Ctrl-Alt-Del from the kernel (`CAD_OFF`) so that it cannot
reset the machine under a mounted `/data`, and treats it as a request
for an orderly restart. The handoff names three shutdown triggers and
this is a fourth. The alternative is to ignore the key.
