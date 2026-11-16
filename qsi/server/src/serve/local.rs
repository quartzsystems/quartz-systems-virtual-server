// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The local socket's requests (`qsilocal`), answered from the trust
//! and the sessions: Service Tools option 4's.

use std::os::unix::net::UnixListener;
use std::sync::Arc;
use std::time::Duration;

use qsilocal::{Console, Interface, Local, Pairing, Password};

use super::Server;
use crate::trust::Refusal;

/// One connection's handler.
struct Handler<'a>(&'a Arc<Server>);

fn refusal(r: &Refusal) -> String {
    match r {
        Refusal::Full => "Two consoles are paired with this host: remove one first.".into(),
        Refusal::NoConsole => "No console with that fingerprint is paired.".into(),
        Refusal::NotKept(e) => format!("The consoles could not be kept: {e}"),
    }
}

impl Local for Handler<'_> {
    fn consoles(&mut self) -> Vec<Console> {
        let connected = self.0.sessions.connected();
        self.0
            .trust
            .consoles()
            .consoles
            .into_iter()
            .map(|c| Console {
                connected: connected.contains(&c.fingerprint),
                name: c.name,
                fingerprint: c.fingerprint,
                paired: c.paired,
                last_contact: c.last_contact,
                address: c.address,
            })
            .collect()
    }

    fn start_pairing(&mut self) -> Result<Password, String> {
        let p = self.0.trust.start_pairing().map_err(|r| refusal(&r))?;
        // The password itself is shown at the console and said nowhere.
        self.0.post(
            &refcode::QSI_PAIRING_STARTED,
            &format!("until {}, {} attempts", p.until, p.attempts),
        );
        Ok(Password {
            text: p.text,
            until: p.until,
            attempts: p.attempts,
        })
    }

    fn pairing(&mut self) -> Option<Pairing> {
        self.0.trust.password().map(|p| Pairing {
            until: p.until,
            attempts: p.attempts,
        })
    }

    fn end_pairing(&mut self) {
        if self.0.trust.password().is_some() {
            self.0.trust.end_pairing();
            self.0.post(&refcode::QSI_PAIRING_ENDED, "at the host");
        }
    }

    fn remove(&mut self, fingerprint: String) -> Result<Console, String> {
        let print = qsitrust::Fingerprint::from_hex(&fingerprint)
            .ok_or_else(|| "That is not a certificate fingerprint.".to_string())?;
        let gone = self.0.trust.remove(&print).map_err(|r| refusal(&r))?;
        Ok(Console {
            name: gone.name,
            fingerprint: gone.fingerprint,
            paired: gone.paired,
            last_contact: gone.last_contact,
            address: gone.address,
            connected: false,
        })
    }

    fn interface(&mut self) -> Interface {
        self.0.interface()
    }

    fn management(&mut self, fact: String) {
        self.0.word.tell(fact);
    }
}

/// The local socket of a server that cannot run its cryptography: the
/// consoles from their file, the reason for everything else.
struct Without {
    own: std::path::PathBuf,
    problem: String,
}

impl Local for Without {
    fn consoles(&mut self) -> Vec<Console> {
        state::Store::at(&self.own)
            .load::<crate::trust::consoles::Consoles>()
            .ok()
            .flatten()
            .unwrap_or_default()
            .consoles
            .into_iter()
            .map(|c| Console {
                name: c.name,
                fingerprint: c.fingerprint,
                paired: c.paired,
                last_contact: c.last_contact,
                address: c.address,
                connected: false,
            })
            .collect()
    }

    fn start_pairing(&mut self) -> Result<Password, String> {
        Err(self.problem.clone())
    }

    fn pairing(&mut self) -> Option<Pairing> {
        None
    }

    fn end_pairing(&mut self) {}

    fn remove(&mut self, _fingerprint: String) -> Result<Console, String> {
        Err(self.problem.clone())
    }

    fn interface(&mut self) -> Interface {
        Interface {
            problem: self.problem.clone(),
            ..Interface::default()
        }
    }

    fn management(&mut self, _fact: String) {}
}

/// Answer the local socket of a server that does not run because the
/// processor lacks what its cryptography needs (`problem`): nothing of
/// TLS or of QSI is touched. Never returns.
pub fn without(listener: &UnixListener, own: std::path::PathBuf, problem: String) -> ! {
    let mut handler = Without { own, problem };
    loop {
        for stream in listener.incoming().flatten() {
            answer(&mut handler, stream);
        }
    }
}

fn answer(handler: &mut impl Local, mut stream: std::os::unix::net::UnixStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Ok(request) = ipc::frame::read_frame_max(&mut stream, 64 * 1024) else {
        return;
    };
    let Ok(request) = ipc::decode::<qsilocal::Request>(&request) else {
        return;
    };
    let reply = qsilocal::dispatch(handler, request);
    let _ = ipc::frame::write_frame(&mut stream, &reply);
}

/// Answer the local socket, a connection at a time, for ever.
pub fn serve(server: Arc<Server>, listener: UnixListener) {
    let _ = std::thread::Builder::new()
        .name("local".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(&mut Handler(&server), stream);
            }
        });
}
