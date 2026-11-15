// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The control socket, `<run>/partitions/<ID>/control.sock`: made as
//! root before the privilege drop and root's, so the partition manager
//! connects to it and no partition can. One request a connection, a
//! line of text, after a byte that carries an open file when there is
//! one (`base/fdpass`):
//!
//! ```text
//! load <slot> <name> <capacity MiB> <protected>   a medium; its file comes with it
//! unload <slot> <forced>
//! media
//! hold                                            keep the partition: stop its processors
//! kept                                            whether it is kept
//! dump <time> <code> <reason> <text>              the dump's file comes with it
//! ```
//!
//! The answer is a line: `ok ...` or `refused <why>`; `media` answers a
//! line a drive and then `end` (`crate::media`). A dump answers
//! `ok <bytes> <bytes of memory written>` when it is whole.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::thread;
use std::time::Duration;

use crate::dump::Dumper;
use crate::machine::pause::Reason;
use crate::media::Drives;

/// A request's most bytes.
const MAX_LINE: u64 = 512;

/// What the socket acts on.
#[derive(Clone)]
pub struct Control {
    /// The optical and tape drives.
    pub drives: Drives,
    /// The partition's dumps and its processors.
    pub dumper: Dumper,
}

impl Control {
    /// Carry out one request.
    pub fn carry_out(&self, line: &str, file: Option<File>) -> String {
        let (word, rest) = line.split_once(' ').unwrap_or((line, ""));
        match word {
            "hold" => {
                if self.dumper.pause.hold(Reason::Kept) {
                    "ok kept".into()
                } else {
                    "refused the processors did not stop".into()
                }
            }
            "kept" => format!("ok {}", self.dumper.pause.kept()),
            "dump" => {
                let mut words = rest.splitn(4, ' ');
                let (Some(time), Some(code), Some(reason)) = (
                    words.next().and_then(|t| t.parse::<u64>().ok()),
                    words.next(),
                    words.next(),
                ) else {
                    return "refused dump <time> <code> <reason> <text>".into();
                };
                let text = words.next().unwrap_or_default();
                let Some(file) = file else {
                    return "refused no file came with the request".into();
                };
                match self.dumper.take(&file, code, text, time, reason) {
                    Ok(w) => format!("ok {} {}", w.bytes, w.data),
                    Err(e) => format!("refused {e}"),
                }
            }
            _ => self.drives.carry_out(line, file),
        }
    }

    fn answer(&self, stream: &UnixStream) -> io::Result<()> {
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let file = fdpass::recv(stream, 1)?.into_iter().next().map(File::from);
        let mut line = String::new();
        BufReader::new(Read::take(stream, MAX_LINE)).read_line(&mut line)?;
        let answer = self.carry_out(line.trim(), file);
        let mut out = stream;
        out.write_all(format!("{answer}\n").as_bytes())
    }
}

/// Serve the control socket on a thread.
pub fn spawn(listener: UnixListener, control: Control) -> io::Result<()> {
    thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Err(e) = control.answer(&stream) {
                    eprintln!("qsvmm: control: {e}");
                }
            }
        })?;
    Ok(())
}
