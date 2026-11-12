// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The Phase 0 suite: boot, and check what PID 1 says on the serial
//! console. The disk suite boots slot A and then slot B (by a firmware
//! boot entry); the ISO suite boots the installation media.

mod slotb;

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

/// What to boot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Media {
    /// The disk image: slot A, then slot B.
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

/// The suite for one media.
pub struct Suite {
    media: Media,
    image: PathBuf,
    work: PathBuf,
    base: Launch,
    timeout: Duration,
    allow_no_kvm: bool,
}

/// The line the banner starts with: the product and this repository's
/// release. The build ID follows it.
pub fn banner_prefix() -> String {
    format!("{} {} ", release::NAME, release::RELEASE)
}

impl Suite {
    /// A suite over `disk` or `iso`, whichever `media` asks for, with
    /// scratch files under `work`.
    pub fn new(
        disk: Option<&Path>,
        iso: Option<&Path>,
        media: Media,
        work: PathBuf,
    ) -> Result<Self, HarnessError> {
        let (image, what) = match media {
            Media::Disk => (disk, "disk image"),
            Media::Iso => (iso, "ISO"),
        };
        let image = image
            .filter(|p| p.is_file())
            .ok_or(HarnessError::ImageMissing { what })?
            .to_path_buf();
        std::fs::create_dir_all(&work)?;
        let timeout = env::var(TIMEOUT_ENV)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(BOOT_SECS);
        Ok(Self {
            media,
            image,
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
            },
            timeout: Duration::from_secs(timeout),
            allow_no_kvm: env::var(ALLOW_NO_KVM_ENV).is_ok_and(|v| v == "1"),
        })
    }

    /// How guests are launched: the binary, the firmware, KVM or not.
    pub fn launch(&self) -> &Launch {
        &self.base
    }

    /// Run every step of the media's suite.
    pub fn run_all(&self) -> Result<Vec<Report>, HarnessError> {
        match self.media {
            Media::Disk => {
                let a = self.ipl(
                    "IPL slot A",
                    &Launch {
                        disk: Some(self.image.clone()),
                        ..self.base.clone()
                    },
                    "IPL slot A (qsvs.slot=A)",
                    true,
                )?;
                let vars = slotb::vars(&self.image, &self.base.firmware, &self.work)?;
                let b = self.ipl(
                    "IPL slot B",
                    &Launch {
                        disk: Some(self.image.clone()),
                        vars: Some(vars),
                        ..self.base.clone()
                    },
                    "IPL slot B (qsvs.slot=B)",
                    false,
                )?;
                Ok(vec![a, b])
            }
            Media::Iso => Ok(vec![self.ipl(
                "IPL from media",
                &Launch {
                    cdrom: Some(self.image.clone()),
                    ..self.base.clone()
                },
                "IPL from installation media (qsvs.install=1)",
                true,
            )?]),
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
        let warning = match (usable, self.allow_no_kvm) {
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
        }
        Ok((detail, warning))
    }
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
}
