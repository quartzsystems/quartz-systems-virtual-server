// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS's installer. PID 1 of the media UKI runs it in place of the
//! service table. It shows its screens on the display and on the
//! serial port at once and takes keys from both. It verifies the media
//! before anything else; with an answers file on a seed volume it
//! installs without asking. It ends with status 0, and PID 1 restarts
//! the machine.

// The parts that need no kernel are built and tested on any host.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod media;
#[cfg(target_os = "linux")]
mod target;

#[cfg(target_os = "linux")]
mod on_host {
    use std::io::Write as _;
    use std::path::Path;

    use console::{Identity, Scale};
    use install::flow::{self, Env};
    use install::{Answers, DiskChoice, Outcome, Step, Target};
    use panels::forms::LinkChoice;
    use panels::screens::{Displays, Ended, Screens, SerialPort, run};
    use panels::{Runner, kit};

    use crate::media;
    use crate::target::QsVs;

    /// The serial port.
    const SERIAL: &str = "/dev/ttyS0";
    /// Where the seed volume is mounted while it is read.
    const SEED_MOUNT: &str = "/run/seed";

    /// The installer with no media it trusts: it has nothing to install.
    struct NoMedia;

    impl Target for NoMedia {
        fn product(&self) -> &'static str {
            release::NAME
        }
        fn banner(&self) -> String {
            release::banner()
        }
        fn what(&self) -> &'static [&'static str] {
            &[]
        }
        fn disks(&mut self) -> Vec<DiskChoice> {
            Vec::new()
        }
        fn min_disk_bytes(&self) -> u64 {
            0
        }
        fn links(&mut self) -> Vec<LinkChoice> {
            Vec::new()
        }
        fn default_host(&self) -> String {
            release::NAME.into()
        }
        fn now(&self) -> i64 {
            appliance::time::now_secs()
        }
        fn steps(&self) -> &'static [Step] {
            &[]
        }
        fn step(&mut self, _index: usize, _answers: &Answers) -> Result<(), String> {
            Err("the media was refused".into())
        }
        fn failed(&self) -> &'static str {
            refcode::INSTALL_FAILED.code
        }
        fn log(&mut self, line: &str) {
            say(line);
        }
    }

    /// One line for the installer's log; reference codes with their word.
    fn say(line: &str) {
        let code = line
            .split(' ')
            .next()
            .is_some_and(|w| refcode::find(w).is_some());
        if code {
            println!("{} {line}", refcode::WORD);
        } else {
            println!("{line}");
        }
    }

    /// The disk a block device is, or is on.
    fn disk_of(device: &Path) -> String {
        hostdisk::disks::disk_of(device).unwrap_or_else(|| {
            device
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
    }

    /// The answers file of a seed volume: its device and what it holds.
    fn seed() -> Option<(String, Result<seed::Seed, String>)> {
        let device = seed::volume::find(release::SEED_LABEL)?;
        let parsed = seed::volume::read(&device, Path::new(SEED_MOUNT))
            .and_then(|text| seed::format::parse(&text));
        Some((disk_of(&device), parsed))
    }

    fn end(serial: Option<&SerialPort>) -> ! {
        // The terminal's cursor below the grid, for what PID 1 prints.
        if let Some(mut out) = serial.and_then(|s| s.writer().ok()) {
            let _ = out.write_all(b"\x1b[0m\x1b[24;1H\r\n");
        }
        std::process::exit(0);
    }

    pub fn main() {
        let serial = SerialPort::open(Path::new(SERIAL)).ok();
        let line = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
        let screens = Screens::open(
            Identity {
                banner: release::banner(),
                ready_code: String::new(),
            },
            Displays {
                framebuffer: true,
                scale: Scale::from_cmdline(&line, &format!("{}console.scale", release::KEY_PREFIX)),
                serial: serial.as_ref().and_then(|s| s.writer().ok()),
            },
        );
        say(&format!("installer on {}", screens.describe()));
        let keys = serial.as_ref().and_then(|s| s.keys().ok());
        if let Some(keys) = &keys {
            keys.attach(&screens);
        }
        QsVs::links_up();
        screens.show(kit::page(
            &format!("Install {}", release::NAME),
            release::NAME,
            &kit::lines(&[&release::banner(), "", "Verifying the installation media."]),
            "",
        ));

        let found = media::open();
        let answers = seed();
        let outcome = match found {
            Err(reason) => {
                say(&format!(
                    "{} {}: {reason}",
                    refcode::INSTALL_MEDIA_REFUSED.code,
                    refcode::INSTALL_MEDIA_REFUSED.text
                ));
                let mut target = NoMedia;
                let mut env = Env::new(&mut target);
                run(&screens, &mut flow::refused(reason), &mut env, None)
            }
            Ok((device, payload)) => {
                say(&format!(
                    "{} {}: {} {} {}",
                    refcode::INSTALL_VERIFIED.code,
                    refcode::INSTALL_VERIFIED.text,
                    payload.manifest.product,
                    payload.manifest.release,
                    payload.manifest.build
                ));
                let mut skip = vec![disk_of(&device)];
                skip.extend(answers.iter().map(|(disk, _)| disk.clone()));
                let mut target = QsVs::new(payload, skip, Box::new(say));
                // Answers that are there and cannot be used stop the
                // installer; none at all make it ask.
                let given = match answers {
                    None => Ok(None),
                    Some((_, Err(reason))) => Err(reason),
                    Some((_, Ok(seed))) => install::answers::from_seed(&seed, &mut target)
                        .map_err(|e| format!("{}: [install]: {e}", seed::format::FILE)),
                };
                let mut runner: Runner<Env<'_>, Outcome> = match given {
                    Ok(None) => flow::attended(),
                    Ok(Some(answers)) => {
                        say("unattended: the answers of a seed volume are used");
                        flow::unattended(answers)
                    }
                    Err(reason) => {
                        say(&format!(
                            "{} {}: {reason}",
                            refcode::INSTALL_ANSWERS_INVALID.code,
                            refcode::INSTALL_ANSWERS_INVALID.text
                        ));
                        flow::stopped(reason)
                    }
                };
                let mut env = Env::new(&mut target);
                run(&screens, &mut runner, &mut env, None)
            }
        };
        match outcome {
            Ended::Outcome(Outcome::Installed) => say(&format!(
                "{} {}",
                refcode::INSTALL_COMPLETE.code,
                refcode::INSTALL_COMPLETE.text
            )),
            _ => say("the installer was left; nothing more is written"),
        }
        rustix::fs::sync();
        end(serial.as_ref());
    }
}

#[cfg(target_os = "linux")]
fn main() {
    on_host::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("qsvsinstall is the installer of a QS/VS host and runs on Linux only");
}
