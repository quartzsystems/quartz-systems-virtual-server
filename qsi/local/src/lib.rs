// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The QSI host server's local socket: Service Tools option 4
//! (`docs/QSISERVER.md`) asks it for the consoles, a pairing password,
//! a console's removal and what the server listens on. Root's alone;
//! nothing of QSI's crosses it.

#![forbid(unsafe_code)]

/// The socket's file, in the run directory.
pub const SOCKET_FILE: &str = "qsvsqsid.sock";

/// One paired console, as the panels show it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Console {
    /// The name it gave when it paired.
    pub name: String,
    /// Its certificate's SHA-256, in hexadecimal.
    pub fingerprint: String,
    /// When it paired, seconds since the epoch.
    pub paired: u64,
    /// When it last had a session; 0 for never.
    pub last_contact: u64,
    /// The address it last connected from.
    pub address: String,
    /// Whether it has a session now.
    pub connected: bool,
}
ipc::wire_struct!(Console {
    name,
    fingerprint,
    paired,
    last_contact,
    address,
    connected
});

/// A pairing password, shown once.
#[derive(Clone, PartialEq, Eq)]
pub struct Password {
    /// The password: shown at the console, kept nowhere.
    pub text: String,
    /// When it expires, seconds since the epoch.
    pub until: u64,
    /// The attempts it has.
    pub attempts: u32,
}
ipc::wire_struct!(Password {
    text,
    until,
    attempts
});

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Password")
            .field("until", &self.until)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

/// The password out, without its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pairing {
    /// When it expires, seconds since the epoch.
    pub until: u64,
    /// The attempts left.
    pub attempts: u32,
}
ipc::wire_struct!(Pairing { until, attempts });

/// What the server listens on, and who is connected.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interface {
    /// The addresses, each with its port.
    pub addresses: Vec<String>,
    /// The host's certificate's SHA-256, in hexadecimal: what a console
    /// is told it pairs with.
    pub fingerprint: String,
    /// Why it listens on nothing, when it does not.
    pub problem: String,
    /// The sessions now: each address, and its console's name or
    /// `pairing`.
    pub sessions: Vec<(String, String)>,
}
ipc::wire_struct!(Interface {
    addresses,
    fingerprint,
    problem,
    sessions
});

ipc::rpc! {
    request Request, service Local, client Calls, dispatch dispatch;
    /// The consoles paired, in the order they paired.
    0 => fn consoles / Consoles () -> Vec<Console>;
    /// Make a pairing password, in place of any out: refused when two
    /// consoles are paired.
    1 => fn start_pairing / StartPairing () -> Result<Password, String>;
    /// The password out, if one is.
    2 => fn pairing / GetPairing () -> Option<Pairing>;
    /// End pairing: the password is gone.
    3 => fn end_pairing / EndPairing () -> ();
    /// Remove a console by its fingerprint: its session ends.
    4 => fn remove / Remove (fingerprint: String) -> Result<Console, String>;
    /// What the server listens on.
    5 => fn interface / GetInterface () -> Interface;
}

/// A client: each call is its own connection.
#[cfg(unix)]
#[derive(Clone, Debug)]
pub struct Client {
    path: std::path::PathBuf,
}

#[cfg(unix)]
impl Client {
    /// A client of the socket at `path`.
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

#[cfg(unix)]
impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        let wait = Some(std::time::Duration::from_secs(10));
        stream.set_read_timeout(wait)?;
        stream.set_write_timeout(wait)?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(req))?;
        ipc::frame::read_frame_max(&mut stream, 64 * 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_is_never_in_its_debug_text() {
        let p = Password {
            text: "ABCDEFGHJKMNPQRS".into(),
            until: 1,
            attempts: 5,
        };
        assert!(!format!("{p:?}").contains("ABCD"));
        let back: Password = ipc::decode(&ipc::encode(&p)).unwrap();
        assert_eq!(back, p);
    }
}
