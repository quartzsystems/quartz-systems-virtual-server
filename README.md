# QS/VS

Quartz Systems/Virtual Server: a hypervisor appliance that runs QS/400
instances as partitions. The Linux kernel is the only inherited
component; everything above it is Rust, static musl, x86_64 only.

QS/VS has no web UI, no shell and no remote management of its own. QS/MC
(Quartz Systems/Management Console) manages it over the QSI protocol, and
a Service Tools console on the local display and serial port covers host
basics, networking and disks.

## Status

Phase 2: QS/VS installs on a blank machine and is configured by a
person. A clean clone builds a reproducible image and an installation
ISO. The ISO's installer, on the display and the serial port, writes a
load source on one disk or mirrors it over two; an answers file makes
it unattended. The installed host runs the IPL in order, supervises its
services (storage, logging, the management network and time, the
Service Tools console), keeps its state on `QSVSDATA`, starts from
either disk of a mirrored pair, and is configured from Service Tools.
There is no VMM, storage pool or virtual Ethernet yet. See
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
- `docs/PHASE3.md`: Phase 3, the VMM core, as far as it is.
- `docs/INSTALL.md`: installing a host, attended and unattended.
- `docs/SERVICETOOLS.md`: the Service Tools console, panel by panel.
- `docs/REFCODES.md`: host reference codes.
- `docs/SEED.md`: the first configuration of a host, `seed.toml`.
- `docs/LOCATION.md`: the location codes of the inventory.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- `CLAUDE.md`: ground rules and layout.

## License

MIT. See `LICENSE`.
