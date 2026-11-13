// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's partition manager: `part/partd` run as QS/VS, with the
//! host's cores, memory and serial number from QS/VS's inventory. PID 1
//! starts it after the storage service; its partitions go on when it
//! is started again.

#[cfg(target_os = "linux")]
fn main() {
    use std::path::{Path, PathBuf};

    use hostinv::Roots;
    use partd::serve::{Service, run};
    use resource::Thread;
    use supervise::control::{Calls as _, Client};

    let roots = Roots::default();
    let threads = hostinv::threads(&roots)
        .into_iter()
        .map(|t| Thread {
            cpu: t.cpu,
            socket: t.socket,
            core: t.core,
        })
        .collect();
    // The serial number PID 1 has: DMI's, or the one the host derived
    // and keeps.
    let serial = Client::new(release::CONTROL_SOCKET)
        .status()
        .ok()
        .and_then(|status| status.fact(release::fact::SERIAL).map(str::to_string))
        .filter(|serial| !serial.is_empty())
        .or_else(|| hostinv::dmi_serial(&roots))
        .unwrap_or_default();
    let service = Service {
        state_dir: Path::new(release::DATA_DIR).join("state"),
        run_dir: PathBuf::from(release::RUN_DIR),
        control_socket: PathBuf::from(release::CONTROL_SOCKET),
        cgroups: PathBuf::from("/sys/fs/cgroup"),
        huge_pages: PathBuf::from("/sys/kernel/mm/hugepages/hugepages-2048kB/nr_hugepages"),
        vmm: Path::new(release::BIN_DIR).join("qsvmm"),
        threads,
        installed_mib: hostinv::memory_mib(&roots),
        serial,
    };
    // It returns only when it could not start.
    let e = run(&service);
    eprintln!("the partition manager did not start: {e}");
    std::process::exit(1);
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvspartd is a service of a QS/VS host and runs on Linux only");
}
