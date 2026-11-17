// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The control socket's requests, answered by the host.

use std::os::unix::net::UnixListener;

use logd::Severity;
use release::fact;
use supervise::control::{Control, HostState, HostStatus, MAX_POWER_DELAY, Power, serve};

use super::{Host, Kind};

/// How long a request waits for a state file to be written: less than
/// a client of the control socket waits for its answer.
const WRITE_WAIT: std::time::Duration = std::time::Duration::from_millis(1500);

impl Control for Host {
    fn status(&mut self) -> HostStatus {
        self.status_now()
    }

    fn power_off(&mut self) {
        self.begin_shutdown(Kind::PowerOff, "the control socket");
    }

    fn restart(&mut self) {
        self.begin_shutdown(Kind::Restart, "the control socket");
    }

    fn take_dump(&mut self, by: String) -> Result<String, String> {
        Host::take_dump(self, &by)
    }

    fn slots(&mut self) -> supervise::control::SlotState {
        self.slots
            .as_ref()
            .map(crate::slot::Slots::state)
            .unwrap_or_default()
    }

    fn install_slot(&mut self, slot: String) -> Result<(), String> {
        let slot = match slot.as_str() {
            "A" => abrec::Slot::A,
            "B" => abrec::Slot::B,
            _ => return Err(format!("{slot} is no slot")),
        };
        let slots = self.slots.as_mut().ok_or("this IPL has no A/B record")?;
        slots.install(slot)
    }

    fn cancel_slot(&mut self) -> Result<(), String> {
        let slots = self.slots.as_mut().ok_or("this IPL has no A/B record")?;
        slots.cancel()
    }

    fn activate_slot(&mut self, by: String) -> Result<(), String> {
        if self.shutdown.is_some() {
            return Err("the host is shutting down".into());
        }
        let slots = self.slots.as_mut().ok_or("this IPL has no A/B record")?;
        let slot = slots.activate()?;
        let letter = loadsource::record::letter(slot).to_string();
        self.post_about(
            &refcode::UPDATE_ACTIVATING,
            &format!("slot {letter}, by {by}"),
            &[refcode::about::slot(&letter)],
        );
        self.begin_shutdown(Kind::Restart, &format!("an update's activation, by {by}"));
        Ok(())
    }

    fn post(&mut self, code: String, text: String) {
        Control::post_about(self, code, text, Vec::new());
    }

    fn post_about(&mut self, code: String, text: String, about: Vec<supervise::control::About>) {
        // A device released or taken back takes an interface from the
        // host or gives one back: the inventory says so at once.
        if code == refcode::DEVICE_RELEASED.code || code == refcode::DEVICE_TAKEN_BACK.code {
            self.refresh_inventory();
        }
        match refcode::post::posting_of_about(&code, &text, &about) {
            Some(posted) => self.take(posted),
            None => self.say(
                Severity::Warning,
                &format!("a service posted {code}, which is no code of the table: {text}"),
            ),
        }
    }

    fn set_host(
        &mut self,
        name: String,
        time_zone: String,
        ntp_servers: Vec<String>,
    ) -> Result<(), String> {
        let done = self.host_settings(name, time_zone, ntp_servers);
        match &done {
            Ok(()) => self.say(
                Severity::Info,
                &format!("host settings changed: host {}", self.facts.host),
            ),
            Err(e) => self.say(Severity::Warning, &format!("host settings refused: {e}")),
        }
        done
    }

    fn state(&mut self) -> HostState {
        let (code, text) = self
            .reference
            .split_once(' ')
            .map_or((self.reference.as_str(), ""), |(c, t)| (c, t.trim()));
        HostState {
            power: if self.shutdown.is_some() {
                Power::Ending
            } else if self.ready {
                Power::Running
            } else {
                Power::Starting
            },
            code: code.to_string(),
            text: text.to_string(),
            release: release::RELEASE.to_string(),
            slot: match self.facts.slot.as_str() {
                s @ ("A" | "B") => s.to_string(),
                _ => String::new(),
            },
            power_in: self.power_at.as_ref().map(|l| {
                let left = l.at.saturating_duration_since(std::time::Instant::now());
                u32::try_from(left.as_secs()).unwrap_or(u32::MAX)
            }),
        }
    }

    fn power_down(
        &mut self,
        restart: bool,
        immediate: bool,
        delay: u32,
        by: String,
    ) -> Result<(), String> {
        if delay > MAX_POWER_DELAY {
            return Err(format!("a delay of {delay} s; at most {MAX_POWER_DELAY}"));
        }
        if self.shutdown.is_some() {
            return Err("the host is shutting down already".into());
        }
        let kind = if restart {
            Kind::Restart
        } else {
            Kind::PowerOff
        };
        let by: String = by.chars().filter(|c| !c.is_control()).take(128).collect();
        if delay == 0 {
            self.begin_shutdown_with(kind, immediate, &by);
        } else {
            let what = if restart { "restart" } else { "power off" };
            self.say(Severity::Info, &format!("{what} in {delay} s, by {by}"));
            self.power_at = Some(super::shutdown::Later {
                at: std::time::Instant::now() + std::time::Duration::from_secs(u64::from(delay)),
                kind,
                immediate,
                by,
            });
        }
        Ok(())
    }

    fn note(&mut self, key: String, value: String) {
        if key == fact::SERIAL_CONSOLE {
            self.serial_session(value == fact::SERIAL_SESSION);
        } else if key == fact::MANAGEMENT {
            if self.facts.management != value {
                self.facts.management = value;
                self.refresh_inventory();
            }
        } else {
            self.say(Severity::Info, &format!("noted {key}: {value}"));
        }
    }
}

impl Host {
    /// Write `host.toml` and take the name and the zone into use.
    fn host_settings(
        &mut self,
        name: String,
        time_zone: String,
        ntp_servers: Vec<String>,
    ) -> Result<(), String> {
        use state::Document as _;
        let host = state::Host {
            name: Some(name.clone()),
            time_zone,
            ntp_servers,
        };
        host.validate()?;
        // Written by another thread and waited for a moment only: this
        // loop must never wait on /data (see `refresh_inventory`).
        let (done, written) = std::sync::mpsc::channel();
        let to_write = host.clone();
        std::thread::spawn(move || {
            let store = state::Store::at(std::path::Path::new(release::DATA_DIR).join("state"));
            let _ = done.send(store.save(&to_write).map_err(|e| e.to_string()));
        });
        written
            .recv_timeout(WRITE_WAIT)
            .map_err(|_| "the data file system does not answer".to_string())??;
        let _ = rustix::system::sethostname(name.as_bytes());
        self.facts.host = name;
        self.facts.zone = host.zone();
        Ok(())
    }

    /// Accept one connection and answer its request.
    pub(super) fn serve_control(&mut self) {
        let Some(Ok((stream, _))) = self.listener.as_ref().map(UnixListener::accept) else {
            return;
        };
        if let Err(e) = serve(stream, self) {
            self.say(Severity::Warning, &format!("control socket: {e}"));
        }
    }
}
