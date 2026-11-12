# QS/VS

Quartz Systems/Virtual Server: a hypervisor appliance that runs QS/400
instances as partitions. The Linux kernel is the only inherited
component; everything above it is Rust, static musl, x86_64 only.

QS/VS has no web UI, no shell and no remote management of its own. QS/MC
(Quartz Systems/Management Console) manages it over the QSI protocol, and
a Service Tools console on the local display and serial port covers host
basics, networking and disks.

## Status

Phase 0, the toolchain. A clean clone builds a reproducible, bootable
image whose PID 1 prints a banner and proves KVM is usable. Nothing else
exists yet. See `docs/PHASE0.md`.

## Building

On Linux (WSL Ubuntu works), with the prerequisites of `docs/PHASE0.md`:

```
cargo run -p build -- doctor
cargo run -p build -- all
cargo run -p build -- test
cargo run -p build -- test --media iso
```

The outputs are under `target/images/` (or `$QSVS_TARGET_DIR/images/`):
the three UKIs, the disk image, the installation ISO and the signed
manifest.

## Documents

- `docs/ARCHITECTURE.md`: the locked decisions.
- `docs/PHASE0.md`: what Phase 0 built, and the evidence.
- `docs/REFCODES.md`: host reference codes.
- `docs/OPEN_QUESTIONS.md`: what needs a decision.
- `CLAUDE.md`: ground rules and layout.

## License

MIT. See `LICENSE`.
