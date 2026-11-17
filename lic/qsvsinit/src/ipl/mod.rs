// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The host IPL, in order. Each step posts its reference code: the
//! kernel's file systems, cgroup2 and the host's control group, the A/B
//! record, the storage service and `QSVSDATA` at `/data`, the clock
//! from the RTC, the seed, the state, the partitions' memory, the
//! services. "Base ready" is posted by the loop, when every
//! service has said so. On installation media the installer runs in
//! place of all that follows the clock.

#[cfg(target_os = "linux")]
mod choose;
#[cfg(target_os = "linux")]
mod clock;
pub mod cmdline;
#[cfg(target_os = "linux")]
mod install;
pub mod memory;
#[cfg(target_os = "linux")]
pub mod mounts;
#[cfg(target_os = "linux")]
mod state;
pub mod update;

#[cfg(target_os = "linux")]
pub use on_host::run;

#[cfg(target_os = "linux")]
mod on_host {
    use std::os::unix::net::UnixListener;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use abrec::Slot;
    use appliance::Appliance;
    use appliance::time::Zone;
    use loadsource::disks::{self, LoadSource};
    use rustix::system::{RebootCommand, reboot};
    use state::Store;
    use supervise::run::Supervisor;
    use supervise::signal::{CHLD, INT, Signals, TERM};

    use super::cmdline::{self, Source};
    use super::{choose, clock, install, memory, mounts, state as state_step, update};
    use crate::console::Console;
    use crate::host::{Host, table};
    use crate::{kvm, slot, sys};

    /// QS/VS as the base crates see it. They report to PID 1 through
    /// what they return, so nothing is posted or noted through this.
    static APPLIANCE: Appliance = release::appliance(|_| {}, |_, _| {});

    /// Where the seed volume is mounted while it is read.
    const SEED_MOUNT: &str = "/run/seed";
    /// How long the load-source disks may take to appear: some
    /// controllers are probed after PID 1 has started.
    const DISK_WAIT: Duration = Duration::from_secs(10);
    /// How long the storage service may take to settle.
    const STORAGE_WAIT: Duration = Duration::from_secs(60);

    fn join(failed: &[(&'static str, String)]) -> String {
        let all: Vec<String> = failed
            .iter()
            .map(|(at, why)| format!("{at} not mounted, {why}"))
            .collect();
        all.join("; ")
    }

    /// The record step. On installation media there is no slot and no
    /// record.
    fn record(host: &mut Host, source: Source, sources: &[LoadSource]) {
        let booted = match source {
            Source::SlotA => Slot::A,
            Source::SlotB => Slot::B,
            Source::Install | Source::Unknown => {
                host.line("A/B record not read: this IPL is from no slot");
                return;
            }
        };
        let mut posted = Vec::new();
        let slots = slot::at_ipl(booted, sources, |code, detail| {
            posted.push((code.code, detail.to_string()));
        });
        // The slot that did not confirm is what its problem is about.
        let fell_back = match slots.ipl {
            loadsource::record::Ipl::FellBack { pending } => {
                Some(loadsource::record::letter(pending).to_string())
            }
            _ => None,
        };
        for (code, detail) in posted {
            let Some(code) = refcode::find(code) else {
                continue;
            };
            match &fell_back {
                Some(slot) if code.code == refcode::SLOT_FELL_BACK.code => {
                    host.post_about(code, &detail, &[refcode::about::slot(slot)]);
                }
                _ => host.post(code, &detail),
            }
        }
        host.slots = Some(slots);
    }

    /// The storage step: the control socket, then the service table
    /// with everything after the storage service held back, and the
    /// loop turned until that service is ready or will not be. A
    /// mirrored pair whose units ran apart is not assembled until the
    /// operator has chosen one: the choice is shown meanwhile, for as
    /// long as it takes, and nothing else is started.
    fn storage(host: &mut Host) {
        let control = APPLIANCE.control_socket();
        let _ = std::fs::remove_file(&control);
        let listener = UnixListener::bind(&control)
            .and_then(|l| l.set_nonblocking(true).map(|()| l))
            // The QSI host server asks for the host's state and power.
            .and_then(|l| release::share_with_server(&control).map(|()| l))
            .map_err(|e| host.line(&format!("{}: {e}", control.display())))
            .ok();
        // A table that is not one is a failure of PID 1 itself.
        let mut sup = Supervisor::new(table::TABLE, Instant::now())
            .unwrap_or_else(|e| panic!("the service table: {e}"));
        sup.hold_after(table::STGD.order);
        host.quiet = true;
        host.start(sup, listener);
        let mut give_up = Instant::now() + STORAGE_WAIT;
        let mut chooser = choose::Chooser::new(stgd::choose_marker(&APPLIANCE));
        while !host.storage_settled() {
            if chooser.wanted(host) {
                host.storage_waits();
                give_up = Instant::now() + STORAGE_WAIT;
            } else if Instant::now() >= give_up {
                break;
            }
            host.pump();
        }
        chooser.end(host);
    }

    /// What `/data` is mounted from: what the storage service named,
    /// or, when it named nothing and the load source is not mirrored,
    /// the data partition itself.
    fn data_device(sources: &[LoadSource]) -> Result<PathBuf, String> {
        let named = std::fs::read_to_string(stgd::data_device(&APPLIANCE))
            .ok()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        if let Some(device) = named {
            return Ok(PathBuf::from(device));
        }
        if disks::label(APPLIANCE.load_source, sources).is_some() {
            return Err("the storage service does not serve the mirrored load source".into());
        }
        sources
            .first()
            .map(|s| s.data.clone())
            .ok_or_else(|| "no load-source disk was found".into())
    }

    /// The data step: `QSVSDATA` at `/data`, or state in memory. It is
    /// never reformatted.
    fn data(host: &mut Host, sources: &[LoadSource]) {
        let mounted = data_device(sources).and_then(|device| {
            mounts::data(&device, release::DATA_DIR)
                .map(|()| device.display().to_string())
                .map_err(|e| format!("{}: {e}", device.display()))
        });
        match mounted {
            Ok(device) => {
                host.post(&refcode::DATA_MOUNTED, &device);
                host.served = device.starts_with("/dev/ublkb");
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

    /// State across an update: the pending slot's first IPL copies it,
    /// the slot that comes back after it did not confirm restores it.
    fn update_state(host: &mut Host) {
        if !host.data_mounted {
            return;
        }
        let Some(slots) = host.slots.as_ref() else {
            return;
        };
        let (ipl, fell) = match slots.ipl {
            loadsource::record::Ipl::Trial => (update::Ipl::Trial, None),
            loadsource::record::Ipl::FellBack { pending } => (
                update::Ipl::FellBack,
                Some(loadsource::record::letter(pending)),
            ),
            loadsource::record::Ipl::Normal => (update::Ipl::Normal, None),
        };
        match update::before_state(Path::new(release::DATA_DIR), ipl) {
            Ok(update::Done::Copied) => {
                host.post(&refcode::UPDATE_STATE_KEPT, update::COPY);
            }
            Ok(update::Done::Restored) => {
                host.post(
                    &refcode::UPDATE_STATE_RESTORED,
                    "/data/state as it was before the update",
                );
            }
            Ok(update::Done::Kept | update::Done::Dropped | update::Done::Nothing) => {}
            Err(e) => host.line(&format!("the state across the update: {e}")),
        }
        if let Some(slot) = fell {
            host.post_about(
                &refcode::UPDATE_ROLLED_BACK,
                &format!(
                    "slot {slot} did not confirm; this IPL is on slot {}",
                    host.facts.slot
                ),
                &[refcode::about::slot(&slot.to_string())],
            );
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

    /// The memory step: what partitions can be given is set aside as
    /// huge pages, before anything has fragmented memory.
    fn partition_memory(host: &mut Host, store: &Store) {
        let installed = hostinv::memory_mib(&hostinv::Roots::default());
        // A test image can be told to have too little free for them.
        #[cfg(test_hooks)]
        let held = crate::hooks::has(crate::hooks::HOLD_MEMORY).then(|| {
            let settings = store.load::<resource::Settings>().ok().flatten();
            memory::hold(settings.unwrap_or_default().reserve_mib(installed))
        });
        let reserved = memory::reserve(store, installed, Path::new(memory::NR_HUGEPAGES));
        #[cfg(test_hooks)]
        drop(held);
        let code = if reserved.given < reserved.wanted {
            &refcode::MEMORY_SHORT
        } else {
            &refcode::MEMORY_RESERVED
        };
        host.post(code, &reserved.detail());
    }

    /// The physical I/O step (`docs/PHYSICALIO.md`): the IOMMU and
    /// interrupt remapping checked, and each released device given to
    /// the pass-through driver before any service starts. One that is
    /// gone, or whose isolation group changed, is not.
    fn physio(host: &mut Host, store: &Store) {
        let roots = hostinv::Roots::default();
        let iommu = hostinv::iommu(&roots);
        if !iommu.translation || !iommu.remapping {
            let which = match (iommu.translation, iommu.remapping) {
                (false, false) => "DMA remapping and interrupt remapping are off",
                (false, true) => "DMA remapping is off",
                _ => "interrupt remapping is off",
            };
            host.post(&refcode::PHYSIO_OFF, which);
            return;
        }
        let released = match store.load::<pio::Released>() {
            Ok(r) => r.unwrap_or_default(),
            Err(e) => {
                host.post(&refcode::STATE_INVALID, &e.to_string());
                pio::Released::default()
            }
        };
        let pci = hostinv::pci(&roots);
        let mut bound = 0;
        for at in released.at_ipl(&pci) {
            match at {
                pio::released::AtIpl::Bind { location, address } => {
                    let about = [refcode::about::device(&location)];
                    match pio::bind::to_passthrough(&roots.sys, &address) {
                        Ok(()) => {
                            host.post_about(&refcode::DEVICE_BOUND, &location, &about);
                            bound += 1;
                        }
                        Err(e) => host.post_about(
                            &refcode::DEVICE_MISSING,
                            &format!("{location}: not bound: {e}"),
                            &about,
                        ),
                    }
                }
                pio::released::AtIpl::Gone { location } => host.post_about(
                    &refcode::DEVICE_MISSING,
                    &location,
                    &[refcode::about::device(&location)],
                ),
                pio::released::AtIpl::Changed { location, what } => host.post_about(
                    &refcode::DEVICE_GROUP_CHANGED,
                    &format!("{location}: {what}"),
                    &[refcode::about::device(&location)],
                ),
            }
        }
        // The inventory was read before: a released adapter was still
        // the host's network interface then.
        if bound > 0
            && let Ok(Some(mut inventory)) = store.load::<state::Inventory>()
        {
            inventory.interfaces = hostinv::interfaces(&roots);
            if let Err(e) = store.save(&inventory) {
                host.line(&format!("the inventory was not written again: {e}"));
            }
        }
        host.post(
            &refcode::PHYSIO_CHECKED,
            &format!("{bound} released device(s) bound"),
        );
    }

    /// The host's identity: made at the first IPL that has a data
    /// file system to keep it on, and never again.
    fn identity(host: &mut Host) {
        if !host.data_mounted {
            return;
        }
        let made = identity::ensure(
            &APPLIANCE.state_dir(),
            &host.facts.host,
            &host.facts.serial,
            appliance::time::now_secs(),
        );
        match made {
            Ok(true) => host.post(&refcode::IDENTITY_GENERATED, identity::CERT_FILE),
            Ok(false) => {}
            Err(e) => host.post(&refcode::IDENTITY_NOT_GENERATED, &e.to_string()),
        }
    }

    /// The services step: what waited for `/data` is started.
    fn services(host: &mut Host) {
        host.release_services();
        host.quiet = false;
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
        // Ctrl-Alt-Del comes to PID 1, which ignores it, instead of
        // resetting the machine.
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
        // What the service interface's cryptography needs.
        let lacking = qsilocal::processor::lacking();
        if !lacking.is_empty() {
            host.post(&refcode::QSI_PROCESSOR, &lacking.join(", "));
        }

        host.post(&refcode::MOUNTED, &join(&failed));
        host.post(&refcode::CGROUPS, &join(&mounts::cgroups()));
        // Before any service exists, so each is born in the group.
        if let Err(e) = mounts::host_group() {
            host.line(&format!("the host's control group: {e}"));
        }
        if source == Source::Install {
            clock(&mut host);
            install::run(host);
        }
        let sources = disks::load_sources_within(APPLIANCE.load_source, DISK_WAIT);
        record(&mut host, source, &sources);
        storage(&mut host);
        data(&mut host, &sources);
        clock(&mut host);
        update_state(&mut host);
        let store = Store::at(APPLIANCE.state_dir());
        seed(&mut host, &store);
        state(&mut host, &store);
        partition_memory(&mut host, &store);
        physio(&mut host, &store);
        // A test image IPLed as a partition says what it was given.
        #[cfg(test_hooks)]
        crate::hooks::guest_probe(&mut host);
        identity(&mut host);
        services(&mut host);
        host.run()
    }
}
