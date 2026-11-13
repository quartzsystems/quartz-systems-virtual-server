# QS/VS

Quartz Systems/Virtual Server: a hypervisor appliance that runs QS/400
instances as partitions. The Linux kernel is the only inherited
component; everything above it is Rust, static musl, x86_64 only.

QS/VS has no web UI, no shell and no remote management of its own. QS/MC
(Quartz Systems/Management Console) manages it over the QSI protocol, and
a Service Tools console on the local display and serial port covers host
basics, networking and disks.

## Status

Phase 3: the VMM core. `qsvmm`, one process for one partition on KVM
with no guest firmware, takes an unmodified QS/400 disk image to a
completed IPL: it reads the side's UKI and the LIC record from the
partition's load-source disk, starts the kernel directly with ACPI
tables of its own, and gives it disks on virtio-scsi whose flushes are
the host's. There is no partition manager yet: a test image starts one
partition from a profile written by hand, and a release image starts
none. See `docs/PHASE3.md`, `docs/VMM.md` and `docs/QS400_GUEST.md`.

Before it, Phase 2: QS/VS installs on a blank machine and is configured by a
person. A clean clone builds a reproducible image and an installation
ISO. The ISO's installer, on the display and the serial port, writes a
load source on one disk or mirrors it over two; an answers file makes
it unattended. The installed host runs the IPL in order, supervises its
services (storage, logging, the management network and time, the
Service Tools console), keeps its state on `QSVSDATA`, starts from
either disk of a mirrored pair, and is configured from Service Tools.
There is no storage pool or virtual Ethernet yet. See
`docs/PHASE2.md`; the base system is `docs/PHASE1.md`, the toolchain
`docs/PHASE0.md`.

## Building

On Linux (WSL Ubuntu works), with the prerequisites of `docs/PHASE0.md`:

```
cargo run -p build -- doctor
cargo run -p build -- all
cargo run -p build -- all --test-image
cargo run -p build -- test
cargo run -p build -- test --media iso
cargo run -p build -- fuzz
```

The outputs are under `target/images/` (or `$QSVS_TARGET_DIR/images/`):
the three UKIs, the disk image, the installation ISO and the signed
manifest. `--test-image` builds the same with the test hooks the
suites need, under `target/test-image/`; it is never shipped.

## Documents

- `docs/ARCHITECTURE.md`: the locked decisions.
- `docs/PHASE0.md`: what Phase 0 built, and the evidence.
- `docs/PHASE1.md`: what Phase 1 built, and the evidence.
- `docs/PHASE2.md`: what Phase 2 built, and the evidence.
- `docs/PHASE3.md`: what Phase 3 built, the VMM core, and the evidence.
- `docs/VMM.md`: a partition's machine, and the threat model for what a
  guest controls.
- `docs/PROFILE.md`: a partition's profile, provisional.
- `docs/QS400_GUEST.md`: what QS/400 does in a partition without
  firmware.
- `docs/INSTALL.md`: installing a host, attended and unattended.
- `docs/SERVICETOOLS.md`: the Service Tools console, panel by panel.
- `docs/REFCODES.md`: host reference codes.
- `docs/SEED.md`: the first configuration of a host, `seed.toml`.
- `docs/LOCATION.md`: the location codes of the inventory.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- `CLAUDE.md`: ground rules and layout.

## License

MIT. See `LICENSE`.
