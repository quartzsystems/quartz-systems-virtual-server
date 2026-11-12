// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The two ends of the control socket.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipc::frame::{read_frame_max, write_frame};
use ipc::{WireError, decode};

use super::{Calls, Control, Request, dispatch};

/// The longest request or answer.
const MAX_FRAME: usize = 256 * 1024;
/// How long either end waits for the other. PID 1 serves a request in
/// its main loop and must not hang on a client.
const WAIT: Duration = Duration::from_secs(2);

/// A client: each call is its own connection.
#[derive(Clone, Debug)]
pub struct Client {
    path: PathBuf,
}

impl Client {
    /// A client of the socket at `path`. Nothing is connected yet.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The socket's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// One request and its answer, over a connection of their own to the
/// socket at `path`. Other services' sockets are served the same way.
pub fn exchange(path: &Path, request: &[u8]) -> Result<Vec<u8>, WireError> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    write_frame(&mut stream, request)?;
    read_frame_max(&mut stream, MAX_FRAME)
}

/// Read the one request of an accepted connection and send back what
/// `answer` makes of it.
pub fn answer(
    mut stream: UnixStream,
    answer: impl FnOnce(&[u8]) -> Result<Vec<u8>, WireError>,
) -> Result<(), WireError> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(WAIT))?;
    let request = read_frame_max(&mut stream, MAX_FRAME)?;
    write_frame(&mut stream, &answer(&request)?)
}

impl Calls for Client {
    fn call(&mut self, req: &Request) -> Result<Vec<u8>, WireError> {
        exchange(&self.path, &ipc::encode(req))
    }
}

/// Serve the one request of an accepted connection.
pub fn serve(stream: UnixStream, service: &mut impl Control) -> Result<(), WireError> {
    answer(stream, |request| Ok(dispatch(service, decode(request)?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::HostStatus;
    use std::os::unix::net::UnixListener;

    #[derive(Default)]
    struct Host {
        posted: Vec<(String, String)>,
        off: bool,
    }

    impl Control for Host {
        fn status(&mut self) -> HostStatus {
            HostStatus {
                facts: self.posted.clone(),
                services: Vec::new(),
            }
        }
        fn power_off(&mut self) {
            self.off = true;
        }
        fn restart(&mut self) {}
        fn post(&mut self, code: String, text: String) {
            self.posted.push((code, text));
        }
        fn note(&mut self, _key: String, _value: String) {}
        fn set_host(
            &mut self,
            _name: String,
            _time_zone: String,
            _ntp_servers: Vec<String>,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_client_calls_a_server_one_connection_each() {
        let path = std::env::temp_dir().join(format!("supervise-ctl-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let mut host = Host::default();
            for _ in 0..3 {
                let (stream, _) = listener.accept().unwrap();
                serve(stream, &mut host).unwrap();
            }
            host.off
        });
        let mut client = Client::new(&path);
        client.post("X0000001".into(), "hello".into()).unwrap();
        let status = client.status().unwrap();
        assert_eq!(status.fact("X0000001"), Some("hello"));
        client.power_off().unwrap();
        assert!(server.join().unwrap());
        std::fs::remove_file(&path).unwrap();
        // Nobody listening is an error, not a hang.
        assert!(client.status().is_err());
    }
}
