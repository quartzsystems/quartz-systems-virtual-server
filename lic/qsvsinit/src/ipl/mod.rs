// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host IPL, in order. Each step posts its reference code: the
//! kernel's file systems, cgroup2, the A/B record, `QSVSDATA` at
//! `/data`, the clock from the RTC, the seed, the state, the services.
//! "Base ready" is posted by the loop, when every service has said so.

#[cfg(target_os = "linux")]
mod clock;
pub mod cmdline;
#[cfg(target_os = "linux")]
pub mod mounts;
#[cfg(target_os = "linux")]
mod state;

#[cfg(target_os = "linux")]
pub use on_host::run;

#[cfg(target_os = "linux")]
mod on_host {
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::time::Instant;

    use abrec::Slot;
    use appliance::Appliance;
    use appliance::time::Zone;
    use rustix::system::{RebootCommand, reboot};
    use state::Store;
    use supervise::run::Supervisor;
    use supervise::signal::{CHLD, INT, Signals, TERM};

    use super::cmdline::{self, Source};
    use super::{clock, mounts, state as state_step};
    use crate::console::Console;
    use crate::host::{Host, table};
    use crate::{kvm, slot, sys};

    /// QS/VS as the base crates see it. They report to PID 1 through
    /// what they return, so nothing is posted or noted through this.
    static APPLIANCE: Appliance = release::appliance(|_| {}, |_, _| {});

    /// Where the seed volume is mounted while it is read.
    const SEED_MOUNT: &str = "/run/seed";

    fn join(failed: &[(&'static str, String)]) -> String {
        let all: Vec<String> = failed
            .iter()
            .map(|(at, why)| format!("{at} not mounted, {why}"))
            .collect();
        all.join("; ")
    }

    /// The record step. On installation media there is no slot and no
    /// record.
    fn record(host: &mut Host, source: Source) {
        let booted = match source {
            Source::SlotA => Slot::A,
            Source::SlotB => Slot::B,
            Source::Install | Source::Unknown => {
                host.line("A/B record not read: this IPL is from no slot");
                return;
            }
        };
        let mut posted = Vec::new();
        let slots = slot::at_ipl(booted, |code, detail| {
            posted.push((code.code, detail.to_string()));
        });
        for (code, detail) in posted {
            if let Some(code) = refcode::find(code) {
                host.post(code, &detail);
            }
        }
        host.slots = Some(slots);
    }

    /// The data step: `QSVSDATA` at `/data`, or state in memory. It is
    /// never reformatted.
    fn data(host: &mut Host, source: Source) {
        if source == Source::Install {
            host.line("Data file system not mounted on installation media: state is in memory");
            return;
        }
        let mounted = hostdisk::locate::data().and_then(|device| {
            mounts::data(&device, release::DATA_DIR)
                .map(|()| device.display().to_string())
                .map_err(|e| format!("{}: {e}", device.display()))
        });
        match mounted {
            Ok(device) => {
                host.post(&refcode::DATA_MOUNTED, &device);
                host.facts.data = device;
                host.data_mounted = true;
            }
            Err(e) => host.post(
                &refcode::DATA_NOT_MOUNTED,
                &format!("{e}; state is in memory only"),
            ),
        }
    }

    fn clock(host: &mut Host) {
        match clock::set_from_rtc() {
            Ok(secs) => host.post(&refcode::CLOCK_SET, &Zone::UTC.display(secs)),
            Err(e) => host.post(&refcode::CLOCK_NOT_SET, &e),
        }
    }

    fn seed(host: &mut Host, store: &Store) {
        use seed::Outcome;
        match seed::run(APPLIANCE.seed_label, Path::new(SEED_MOUNT), store) {
            Outcome::NotNeeded => host.post(&refcode::SEED_CHECKED, "the host is configured"),
            Outcome::Absent => host.post(&refcode::SEED_CHECKED, "no seed volume"),
            Outcome::Applied => host.post(&refcode::SEED_APPLIED, ""),
            Outcome::Invalid(reason) => host.post(&refcode::SEED_INVALID, &reason),
        }
    }

    fn state(host: &mut Host, store: &Store) {
        let loaded = state_step::load(store, APPLIANCE.host_prefix);
        for refused in &loaded.refused {
            host.post(&refcode::STATE_INVALID, refused);
        }
        host.post(
            &refcode::STATE_LOADED,
            &format!("host {}", loaded.host_name),
        );
        host.facts.host = loaded.host_name;
        host.facts.zone = loaded.zone;
        host.facts.serial = loaded.serial;
    }

    fn services(host: &mut Host) {
        let control = APPLIANCE.control_socket();
        let _ = std::fs::remove_file(&control);
        let listener = UnixListener::bind(&control)
            .and_then(|l| l.set_nonblocking(true).map(|()| l))
            .map_err(|e| host.line(&format!("{}: {e}", control.display())))
            .ok();
        // A table that is not one is a failure of PID 1 itself.
        let sup = Supervisor::new(table::TABLE, Instant::now())
            .unwrap_or_else(|e| panic!("the service table: {e}"));
        host.start(sup, listener);
        let names: Vec<&str> = table::TABLE.iter().map(|s| s.name).collect();
        host.post(&refcode::SERVICES_STARTED, &names.join(", "));
    }

    /// Run the IPL, then the host, and never return: PID 1 must not
    /// exit.
    pub fn run() -> ! {
        sys::panic::install();
        // Before anything is started: children are reaped and a
        // shutdown is asked for through these.
        let signals = Signals::take(&[CHLD, TERM, INT])
            .unwrap_or_else(|e| panic!("the signal descriptor: {e}"));
        // Ctrl-Alt-Del asks PID 1 instead of resetting the machine.
        let _ = reboot(RebootCommand::CadOff);
        let failed = mounts::kernel();
        let mut host = Host::new(Console::open(), signals, &APPLIANCE.log_socket());

        host.post(&refcode::IPL_STARTED, "");
        host.line(&release::banner());
        #[cfg(test_hooks)]
        host.line("TEST IMAGE: the test hooks are in this image; it is not a release");
        let line = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
        let source = cmdline::parse(&line);
        host.line(&source.line());
        host.facts.slot = source.name().to_string();
        host.facts.kvm = kvm::probe::probe().to_string();
        let kvm = host.facts.kvm.clone();
        host.line(&kvm);

        host.post(&refcode::MOUNTED, &join(&failed));
        host.post(&refcode::CGROUPS, &join(&mounts::cgroups()));
        record(&mut host, source);
        data(&mut host, source);
        clock(&mut host);
        let store = Store::at(APPLIANCE.state_dir());
        seed(&mut host, &store);
        state(&mut host, &store);
        services(&mut host);
        host.run()
    }
}
