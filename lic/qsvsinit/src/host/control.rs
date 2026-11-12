// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The control socket's requests, answered by the host.

use std::os::unix::net::UnixListener;

use logd::Severity;
use refcode::post::posting_of;
use release::fact;
use supervise::control::{Control, HostStatus, serve};

use super::{Host, Kind};

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

    fn post(&mut self, code: String, text: String) {
        match posting_of(&code, &text) {
            Some(posted) => self.take(posted),
            None => self.say(
                Severity::Warning,
                &format!("a service posted {code}, which is no code of the table: {text}"),
            ),
        }
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
