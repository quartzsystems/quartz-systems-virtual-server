// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The console as PID 1 runs it when the units of the mirrored load
//! source ran apart: one panel, on the display and the serial port at
//! once, that asks which unit to keep. There is no sign-on, since the
//! service tools users are on `/data`, which is not mounted; times are
//! UTC, since the display zone is there too. It asks the storage
//! service and ends when that has kept a unit.

use appliance::time::Zone;
use panels::Named;
use panels::keep::{ApartUnit, Units};
use stgd::{Calls as _, Client};

/// What the panel asks, answered by the storage service.
pub struct Env {
    storage: Client,
}

impl Env {
    /// The environment of this host.
    pub fn new() -> Self {
        Self {
            storage: Client::of(&release::appliance(|_| {}, |_, _| {})),
        }
    }
}

/// When a disk was last written, as displayed.
fn written(secs: u64) -> String {
    match i64::try_from(secs) {
        Ok(secs) if secs > 0 => Zone::UTC.display(secs),
        _ => "not known".to_string(),
    }
}

/// A reason as a message line words it.
fn sentence(reason: &str) -> String {
    let mut chars = reason.chars();
    let first: String = chars
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    let rest = chars.as_str();
    let stop = if rest.ends_with('.') { "" } else { "." };
    format!("{first}{rest}{stop}")
}

impl Named for Env {
    fn system(&self) -> String {
        release::NAME.into()
    }
}

impl Units for Env {
    fn apart(&mut self) -> Option<Vec<ApartUnit>> {
        let status = self.storage.status().ok()?;
        // Before it has looked at the disks it has nothing to say.
        if status.protection.is_empty() {
            return None;
        }
        if !status.choose {
            return Some(Vec::new());
        }
        Some(
            status
                .units
                .into_iter()
                .map(|u| ApartUnit {
                    number: u.number,
                    disk: u.disk,
                    location: u.location,
                    serial: u.serial,
                    written: written(u.written),
                })
                .collect(),
        )
    }

    fn keep(&mut self, disk: &str) -> Result<(), String> {
        self.storage
            .keep(disk.to_string())
            .map_err(|e| format!("The storage service does not answer: {e}."))?
            .map_err(|e| sentence(&e))
    }
}

#[cfg(target_os = "linux")]
pub fn main() -> ! {
    use std::io::Write as _;
    use std::path::Path;
    use std::time::Duration;

    use console::{Identity, Scale};
    use panels::keep::{KeepUnit, TITLE};
    use panels::screens::{Displays, Ended, Screens, SerialPort, run};
    use panels::{Runner, kit};

    let serial = SerialPort::open(Path::new("/dev/ttyS0")).ok();
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
    println!("the unit to keep is asked for on {}", screens.describe());
    let keys = serial.as_ref().and_then(|s| s.keys().ok());
    if let Some(keys) = &keys {
        keys.attach(&screens);
    }
    let mut env = Env::new();
    let kept = loop {
        let mut runner: Runner<Env, u32> = Runner::new(Box::new(KeepUnit));
        if let Ended::Outcome(kept) = run(&screens, &mut runner, &mut env, None) {
            break kept;
        }
    };
    let said = if kept == 0 {
        "The storage service asks no more.".to_string()
    } else {
        println!("unit {kept} is kept: chosen at the console");
        format!("Unit {kept} is kept.")
    };
    screens.show(kit::page(
        TITLE,
        release::NAME,
        &kit::lines(&[&said, "", "The IPL goes on."]),
        "",
    ));
    std::thread::sleep(Duration::from_millis(300));
    drop(screens);
    // The terminal as PID 1's lines want it.
    if let Some(mut out) = serial.and_then(|s| s.writer().ok()) {
        let _ = out.write_all(b"\x1b[0m\x1b[2J\x1b[H");
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_time_is_shown_in_utc_and_an_unknown_one_said_so() {
        assert_eq!(written(0), "not known");
        assert_eq!(written(1_794_478_923), Zone::UTC.display(1_794_478_923));
        assert!(written(1_794_478_923).ends_with("UTC"));
        assert_eq!(sentence("disk vdb is no unit"), "Disk vdb is no unit.");
    }
}
