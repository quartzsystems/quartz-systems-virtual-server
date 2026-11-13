// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The suites. Phase 0's are unchanged: `ipl` boots the release disk
//! image from slot A and then from slot B (by a firmware boot entry),
//! and the ISO suite boots the installation media; each checks what
//! PID 1 says on the serial console. Phase 1's ten ([`base`]) boot the
//! test image and check the serial console and the control socket's
//! status. Phase 2's ([`install`]) install from the ISO. Phase 3's
//! ([`partition`]) start a partition on the test image.

pub mod base;
pub mod install;
pub mod partition;
pub mod slotb;

use std::cell::RefCell;
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use crate::error::HarnessError;
use crate::qemu::session::Session;
use crate::qemu::{self, Launch, firmware};

/// Seconds a boot may take, when `QSVS_BOOT_TIMEOUT` does not say.
const BOOT_SECS: u64 = 60;
/// Environment override for the boot timeout, in seconds.
pub const TIMEOUT_ENV: &str = "QSVS_BOOT_TIMEOUT";
/// Set to `1` to accept a guest without KVM (a host without nested
/// virtualization), with a warning.
pub const ALLOW_NO_KVM_ENV: &str = "QSVS_ALLOW_NO_KVM";

/// What PID 1 says when KVM is usable, and when it is not.
const KVM_USABLE: &str = "KVM usable";
const KVM_NOT_AVAILABLE: &str = "KVM not available";

/// Phase 0's disk suite.
pub const IPL: &str = "ipl";

/// What to boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    /// The disk images: Phase 0's suite, then the base system's.
    Disk,
    /// The installation ISO, as a CD-ROM.
    Iso,
}

impl FromStr for Media {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "disk" => Ok(Media::Disk),
            "iso" => Ok(Media::Iso),
            other => Err(format!("unknown media `{other}` (disk, iso)")),
        }
    }
}

impl fmt::Display for Media {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Media::Disk => "disk",
            Media::Iso => "iso",
        })
    }
}

/// One step that passed.
#[derive(Debug, Clone)]
pub struct Report {
    /// Step name.
    pub name: &'static str,
    /// How long it took.
    pub elapsed: Duration,
    /// What was seen.
    pub detail: String,
    /// Something the reader should know though the step passed.
    pub warning: Option<String>,
}

/// The built images.
#[derive(Debug, Clone, Default)]
pub struct Images {
    /// The release disk image.
    pub release_disk: Option<PathBuf>,
    /// The release installation ISO.
    pub iso: Option<PathBuf>,
    /// The test disk image: the release image with the test hooks.
    pub test_disk: Option<PathBuf>,
    /// The test image's installation ISO.
    pub test_iso: Option<PathBuf>,
    /// The test image's slot A and slot B UKIs.
    pub test_ukis: Option<(PathBuf, PathBuf)>,
}

/// The suites over the built images.
pub struct Suite {
    images: Images,
    work: PathBuf,
    base: Launch,
    timeout: Duration,
    allow_no_kvm: bool,
    /// The serial logs of the suite that is running.
    log: RefCell<String>,
}

/// The line the banner starts with: the product and this repository's
/// release. The build ID follows it.
pub fn banner_prefix() -> String {
    format!("{} {} ", release::NAME, release::RELEASE)
}

/// Every disk suite, in the order `test` runs them.
pub fn names() -> Vec<&'static str> {
    let mut names = vec![IPL];
    names.extend(base::SUITES.iter().map(|s| s.0));
    names.extend(install::SUITES.iter().map(|s| s.0));
    names.extend(partition::SUITES.iter().map(|s| s.0));
    names
}

fn image<'a>(path: Option<&'a PathBuf>, what: &'static str) -> Result<&'a Path, HarnessError> {
    path.filter(|p| p.is_file())
        .map(PathBuf::as_path)
        .ok_or(HarnessError::ImageMissing { what })
}

impl Suite {
    /// The suites over `images`, with scratch files under `work`.
    pub fn new(images: Images, work: PathBuf) -> Result<Self, HarnessError> {
        std::fs::create_dir_all(&work)?;
        let timeout = env::var(TIMEOUT_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(BOOT_SECS);
        Ok(Self {
            images,
            work,
            base: Launch {
                qemu: qemu::locate()?,
                firmware: firmware::locate()?,
                vars: None,
                kvm: qemu::kvm_wanted(),
                disk: None,
                cdrom: None,
                memory_mb: 1024,
                cpus: 2,
                topology: None,
                extra: Vec::new(),
                snapshot: true,
                nic: None,
                hook: None,
                monitor: None,
                oem: Vec::new(),
                usb: None,
                media_first: false,
                keys: false,
                pinned: false,
                prealloc: false,
            },
            timeout: Duration::from_secs(timeout),
            allow_no_kvm: allow_no_kvm(),
            log: RefCell::new(String::new()),
        })
    }

    /// How guests are launched: the binary, the firmware, KVM or not.
    pub fn launch(&self) -> &Launch {
        &self.base
    }

    /// Run the suites of `media`: all of them, or the one named `only`.
    /// `each` is told of every step as it passes.
    pub fn run(
        &self,
        media: Media,
        only: Option<&str>,
        mut each: impl FnMut(&Report),
    ) -> Result<Vec<Report>, HarnessError> {
        let mut reports = Vec::new();
        let mut done = |more: Vec<Report>| {
            more.iter().for_each(&mut each);
            reports.extend(more);
        };
        if media == Media::Iso {
            let iso = image(self.images.iso.as_ref(), "ISO")?;
            done(vec![self.media(iso)?]);
            return Ok(reports);
        }
        if let Some(name) = only
            && !names().contains(&name)
        {
            return Err(HarnessError::Setup {
                step: "test",
                reason: format!("no suite `{name}`; the suites are {}", names().join(", ")),
            });
        }
        let wanted = |name: &str| only.is_none_or(|o| o == name);
        if wanted(IPL) {
            done(self.phase0()?);
        }
        let suites = base::SUITES
            .iter()
            .chain(install::SUITES)
            .chain(partition::SUITES);
        for (name, suite) in suites {
            if wanted(name) {
                done(vec![self.base_suite(name, *suite)?]);
            }
        }
        Ok(reports)
    }

    /// The installation media: the IPL as PID 1 says it, and then the
    /// installer's first screen in place of the service table.
    fn media(&self, iso: &Path) -> Result<Report, HarnessError> {
        let step = "IPL from media";
        let launch = Launch {
            cdrom: Some(iso.to_path_buf()),
            ..self.base.clone()
        };
        let mut session = Session::spawn(&launch)?;
        let deadline = Instant::now() + self.timeout;
        let source = "IPL from installation media (qsvs.install=1)";
        let outcome =
            self.check(&mut session, source, false, deadline)
                .and_then(|(detail, warning)| {
                    session
                        .screen("the installer's welcome", deadline, |grid| {
                            grid.row(1).trim() == "Install QS/VS"
                                && grid.contains("Press Enter to continue")
                        })
                        .map(|_| (format!("{detail}, then the installer's welcome"), warning))
                });
        let elapsed = session.elapsed();
        session.kill();
        match outcome {
            Ok((detail, warning)) => Ok(Report {
                name: step,
                elapsed,
                detail,
                warning,
            }),
            Err(reason) => Err(HarnessError::Failed {
                step,
                reason,
                log: std::mem::take(&mut session.log),
            }),
        }
    }

    /// Phase 0's disk suite, on the release image: slot A, then slot B.
    fn phase0(&self) -> Result<Vec<Report>, HarnessError> {
        let disk = image(self.images.release_disk.as_ref(), "disk image")?;
        let a = self.ipl(
            "IPL slot A",
            &Launch {
                disk: Some(disk.to_path_buf()),
                ..self.base.clone()
            },
            "IPL slot A (qsvs.slot=A)",
            true,
        )?;
        let vars = slotb::vars(disk, &self.base.firmware, &self.work)?;
        let b = self.ipl(
            "IPL slot B",
            &Launch {
                disk: Some(disk.to_path_buf()),
                vars: Some(vars),
                ..self.base.clone()
            },
            "IPL slot B (qsvs.slot=B)",
            false,
        )?;
        Ok(vec![a, b])
    }

    /// One of the base system's suites, on the test image.
    fn base_suite(&self, name: &'static str, suite: base::SuiteFn) -> Result<Report, HarnessError> {
        let disk = image(self.images.test_disk.as_ref(), "test disk image")?;
        self.log.borrow_mut().clear();
        let started = Instant::now();
        let run = base::Run {
            name,
            disk,
            work: &self.work,
            base: &self.base,
            images: &self.images,
            release_disk: self.images.release_disk.as_deref(),
            installed: false,
            timeout: self.timeout,
            log: &self.log,
        };
        match suite(&run) {
            Ok(detail) => Ok(Report {
                name,
                elapsed: started.elapsed(),
                detail,
                warning: None,
            }),
            Err(reason) => Err(HarnessError::Failed {
                step: name,
                reason,
                log: self.log.take(),
            }),
        }
    }

    /// Boot and check, in the order PID 1 prints: the IPL reference
    /// code, the banner with this release, where the IPL came from, the
    /// KVM line and, when `heartbeats`, two heartbeats.
    fn ipl(
        &self,
        step: &'static str,
        launch: &Launch,
        source: &str,
        heartbeats: bool,
    ) -> Result<Report, HarnessError> {
        let mut session = Session::spawn(launch)?;
        let deadline = Instant::now() + self.timeout;
        let outcome = self.check(&mut session, source, heartbeats, deadline);
        let elapsed = session.elapsed();
        session.kill();
        match outcome {
            Ok((detail, warning)) => Ok(Report {
                name: step,
                elapsed,
                detail,
                warning,
            }),
            Err(reason) => Err(HarnessError::Failed {
                step,
                reason,
                log: std::mem::take(&mut session.log),
            }),
        }
    }

    fn check(
        &self,
        s: &mut Session,
        source: &str,
        heartbeats: bool,
        deadline: Instant,
    ) -> Result<(String, Option<String>), String> {
        ipl_check(s, source, heartbeats, deadline, self.allow_no_kvm)
    }
}

/// Whether a guest without KVM is accepted, by the environment.
pub fn allow_no_kvm() -> bool {
    env::var(ALLOW_NO_KVM_ENV).is_ok_and(|v| v == "1")
}

/// Phase 0's check of one IPL: the IPL reference code, the banner with
/// this release, where the IPL came from, the KVM line and, when
/// `heartbeats`, two heartbeats. What was seen, and a warning.
pub fn ipl_check(
    s: &mut Session,
    source: &str,
    heartbeats: bool,
    deadline: Instant,
    allow_no_kvm: bool,
) -> Result<(String, Option<String>), String> {
    s.expect(&refcode::IPL_STARTED.tag(), deadline)?;
    let ipl_at = s.elapsed();
    s.expect(&banner_prefix(), deadline)?;
    let build_id = s.rest_of_line(deadline)?;
    s.expect(source, deadline)?;
    let usable = s.expect_any(&[KVM_USABLE, KVM_NOT_AVAILABLE], deadline)? == 0;
    let rest = s.rest_of_line(deadline)?;
    let kvm = format!(
        "{}{rest}",
        if usable {
            KVM_USABLE
        } else {
            KVM_NOT_AVAILABLE
        }
    );
    let warning = match (usable, allow_no_kvm) {
        (true, _) => None,
        (false, true) => Some(format!("{ALLOW_NO_KVM_ENV}=1 accepted `{kvm}`")),
        (false, false) => {
            return Err(format!(
                "`{kvm}`; a host without nested virtualization may set {ALLOW_NO_KVM_ENV}=1"
            ));
        }
    };
    let mut detail = format!(
        "{} after {:.1}s, {}{build_id}, {source}, {kvm}",
        refcode::IPL_STARTED.tag(),
        ipl_at.as_secs_f64(),
        banner_prefix(),
    );
    if heartbeats {
        let tag = refcode::HEARTBEAT.tag();
        s.expect(&tag, deadline)?;
        s.expect(&tag, deadline)?;
        detail.push_str(", two heartbeats");
        // Nothing of an image starts a partition by itself: no code of
        // the partition block is posted.
        let partition = &refcode::PARTITION_ACTIVATING.code[..5];
        if s.said(&format!("{} {partition}", refcode::WORD))
            || s.said(&format!("{} A{}", refcode::WORD, &partition[1..]))
        {
            return Err("a partition code was posted: something started a partition".into());
        }
        detail.push_str(", no partition started");
    }
    Ok((detail, warning))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_names_parse_and_print() {
        for m in [Media::Disk, Media::Iso] {
            assert_eq!(m.to_string().parse::<Media>(), Ok(m));
        }
        assert!("usb".parse::<Media>().is_err());
    }

    #[test]
    fn the_banner_prefix_is_this_release() {
        assert!(release::banner().starts_with(&banner_prefix()));
    }

    #[test]
    fn there_are_phase_0_s_suite_and_those_of_each_phase_since() {
        // Phase 0's, Phase 1's ten, Phase 2's thirteen and the two
        // beside them, Phase 3's ten (its eleventh, the fuzz targets,
        // is the build tool's), and Phase 4's sixteen.
        assert_eq!(names().len(), 1 + 10 + 13 + 2 + 10 + 16);
        assert_eq!(
            names()[26..],
            [
                "partition-boot",
                "partition-scsi",
                "partition-ipl-a",
                "partition-ipl-b",
                "partition-attributes",
                "partition-kill",
                "partition-power-pull",
                "partition-end",
                "partition-damaged",
                "partition-privilege",
                "partition-recorded",
                "partition-define",
                "partition-three",
                "partition-minimum",
                "partition-dedicated",
                "partition-capped",
                "partition-uncapped",
                "partition-core-sched",
                "partition-huge-pages",
                "partition-auto-start",
                "partition-killed",
                "partition-manager-restart",
                "partition-identity",
                "partition-isolation",
                "partition-own-disks",
                "partition-tools"
            ]
        );
        assert_eq!(
            names()[11..26],
            [
                "install-serial",
                "install-display",
                "install-unattended",
                "mirror-degraded",
                "mirror-replace",
                "install-power-pull",
                "mirror-power-pull",
                "storage-kill",
                "tools-network",
                "tools-sign-on",
                "media-refused",
                "answers-invalid",
                "installed-base",
                "mirror-start",
                "mirror-split"
            ]
        );
        assert_eq!(
            names()[..11],
            [
                "ipl",
                "status",
                "dhcp",
                "static-vlan",
                "time",
                "inventory",
                "supervision",
                "power-pull",
                "slot-confirm",
                "shutdown",
                "sockets"
            ]
        );
    }
}
