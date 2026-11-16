// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The problem service's socket (`docs/PROBLEMS.md`): a request a
//! connection and its answer, or a subscription whose events follow.
//! Root's and the QSI server's group's.

use qvsmsg::Issued;

use crate::record::{Filter, Noticed, Page, Problem};

/// The service's name in PID 1's table.
pub const NAME: &str = "probd";
/// The socket's file, in the run directory.
pub const SOCKET_FILE: &str = "qsvsprobd.sock";
/// The longest request, answer or event.
pub const MAX_FRAME: usize = 256 * 1024;

/// The socket in `run_dir`.
#[must_use]
pub fn socket(run_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(run_dir).join(SOCKET_FILE)
}

ipc::rpc! {
    request Request, service Problems, client Calls, dispatch dispatch;
    /// A page of the problems `filter` wants, newest first, numbered
    /// below `before`.
    0 => fn list / List (filter: Filter, before: Option<u64>, limit: u32) -> Page;
    /// One problem whole.
    1 => fn get / Get (number: u64) -> Result<Problem, Vec<Issued>>;
    /// Close a problem for `by` (who closed it: a console's user, or a
    /// service tools user); one whose condition holds only `anyway`.
    2 => fn close / Close (number: u64, anyway: bool, by: String) -> Result<Problem, Vec<Issued>>;
    /// Subscribe: the connection every event then arrives on.
    3 => fn subscribe / Subscribe () -> ();
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

    /// Subscribe, and call `f` with each event until it returns false
    /// or the connection ends; `ready` once the subscription is
    /// answered, before the first event.
    ///
    /// # Errors
    /// The service is not there, or the connection broke.
    pub fn events(
        &self,
        ready: impl FnOnce(),
        mut f: impl FnMut(Noticed) -> bool,
    ) -> Result<(), ipc::WireError> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.path)?;
        ipc::frame::write_frame(&mut stream, &ipc::encode(&Request::Subscribe {}))?;
        ipc::decode::<()>(&ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?)?;
        ready();
        loop {
            let frame = ipc::frame::read_frame_max(&mut stream, MAX_FRAME)?;
            if !f(ipc::decode(&frame)?) {
                return Ok(());
            }
        }
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
        ipc::frame::read_frame_max(&mut stream, MAX_FRAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{Notice, Status, Summary};

    #[test]
    fn what_the_socket_carries_round_trips() {
        let r = Request::List {
            filter: Filter {
                status: Some(Status::Open),
                partition: Some(3),
                since: None,
            },
            before: Some(9),
            limit: 20,
        };
        let back = ipc::decode::<Request>(&ipc::encode(&r)).unwrap();
        assert_eq!(ipc::encode(&back), ipc::encode(&r));
        let e = Noticed {
            notice: Notice::Cleared,
            problem: Summary {
                number: 4,
                ..Summary::default()
            },
            time: 7,
        };
        assert_eq!(ipc::decode::<Noticed>(&ipc::encode(&e)).unwrap(), e);
    }
}
