// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The readiness signal. A service inherits a pipe as its standard
//! input (no descriptor number crosses `exec`) and writes one byte to
//! it when it is ready to be depended on.

use std::io;

/// The byte written.
pub const READY: u8 = b'R';

/// Say this service is ready. Call once.
#[cfg(unix)]
pub fn signal() -> io::Result<()> {
    use std::io::Write;
    use std::os::fd::AsFd;

    let fd = io::stdin().as_fd().try_clone_to_owned()?;
    std::fs::File::from(fd).write_all(&[READY])
}

/// Say this service is ready. Services run on Linux only.
#[cfg(not(unix))]
pub fn signal() -> io::Result<()> {
    Err(io::Error::other("services run on Linux only"))
}
