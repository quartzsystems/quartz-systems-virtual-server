// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A physical tape drive passed through (`docs/STORAGE.md`): each
//! command is checked against the allow-list, forwarded as a frame on
//! the link to the partition manager's relay, which checks it again and
//! sends it to the drive, and the drive's status, sense data, data and
//! residue are given to the guest as they came. REPORT LUNS is the
//! target's own. A link that fails, or a drive the relay says is gone,
//! ends the command with a transport failure.

use std::io::{Read, Write};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tape::passthrough::{self as pt, Answer, Command};

use super::backing::Bounce;
use super::lun::{Counts, Failure};
use super::unit::{Unit, give, report_luns};

/// The link to the relay: a stream.
pub trait Link: Read + Write + Send {}
impl<T: Read + Write + Send> Link for T {}

/// The drive, as the guest has it.
pub struct PassedTape {
    link: Mutex<Box<dyn Link>>,
    reads: AtomicU64,
    writes: AtomicU64,
    refused: AtomicU64,
}

impl PassedTape {
    /// The drive at the end of `link`.
    pub fn new(link: Box<dyn Link>) -> Self {
        Self {
            link: Mutex::new(link),
            reads: AtomicU64::new(0),
            writes: AtomicU64::new(0),
            refused: AtomicU64::new(0),
        }
    }

    /// Send `command` and wait for its answer; none when the link failed.
    fn ask(&self, command: &Command) -> Option<Answer> {
        let mut link = self.link.lock().ok()?;
        link.write_all(&command.encode()).ok()?;
        let body = pt::read_frame(&mut *link).ok()??;
        Answer::decode(&body)
    }
}

impl Unit for PassedTape {
    fn run(
        &self,
        cdb: &[u8],
        luns: usize,
        data_out: &mut dyn Read,
        data_in: &mut dyn Write,
        _bounce: &mut Bounce,
    ) -> Result<usize, Failure> {
        let cdb = &cdb[..cdb.len().min(pt::MAX_CDB)];
        if cdb.first() == Some(&0xA0) {
            let room = cdb
                .get(6..10)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
            return Ok(give(data_in, &report_luns(luns), room));
        }
        if let Err(why) = pt::allowed(cdb) {
            self.refused.fetch_add(1, Ordering::Relaxed);
            return Err(Failure::Answer {
                status: pt::STATUS_CHECK,
                sense: why.sense(),
                given: 0,
            });
        }
        // What the guest sends, bounded: a command moves a block at most.
        let mut out = Vec::new();
        if data_out
            .take(pt::MAX_TRANSFER as u64 + 1)
            .read_to_end(&mut out)
            .is_err()
            || out.len() > pt::MAX_TRANSFER
        {
            return Err(Failure::Overrun);
        }
        let command = Command {
            cdb: cdb.to_vec(),
            data_in: u32::try_from(pt::MAX_TRANSFER).unwrap_or(u32::MAX),
            data_out: out,
        };
        if !command.data_out.is_empty() {
            self.writes.fetch_add(1, Ordering::Relaxed);
        }
        let Some(answer) = self.ask(&command) else {
            return Err(Failure::Unreachable);
        };
        if answer.gone {
            return Err(Failure::Unreachable);
        }
        let given = give(data_in, &answer.data, answer.data.len());
        if given > 0 {
            self.reads.fetch_add(1, Ordering::Relaxed);
        }
        if answer.status == pt::STATUS_GOOD && answer.sense.is_empty() {
            return Ok(given);
        }
        Err(Failure::Answer {
            status: answer.status,
            sense: answer.sense,
            given,
        })
    }

    fn counts(&self) -> Counts {
        Counts {
            reads: self.reads.load(Ordering::Relaxed),
            writes: self.writes.load(Ordering::Relaxed),
            refused: self.refused.load(Ordering::Relaxed),
            ..Counts::default()
        }
    }

    fn blocks(&self) -> u64 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;
    use std::thread;

    /// The relay's end, over `drive`, on a thread.
    fn relay(mut far: UnixStream, mut drive: pt::Double) -> thread::JoinHandle<pt::Double> {
        thread::spawn(move || {
            while let Ok(Some(body)) = pt::read_frame(&mut far) {
                let Some(c) = Command::decode(&body) else {
                    break;
                };
                let (answer, _) = pt::relay(&mut drive, &c);
                if far.write_all(&answer.encode()).is_err() {
                    break;
                }
            }
            drive
        })
    }

    fn run(unit: &PassedTape, cdb: &[u8], out: &[u8]) -> (Result<usize, Failure>, Vec<u8>) {
        let mut data_in = Vec::new();
        let r = unit.run(cdb, 1, &mut &out[..], &mut data_in, &mut Bounce::new(4096));
        (r, data_in)
    }

    #[test]
    fn a_command_reaches_the_drive_and_its_answer_comes_back_whole() {
        let (near, far) = UnixStream::pair().unwrap();
        let mut double = pt::Double::default();
        double.answers.push_back(Answer {
            data: b"QUANTUM ULTRIUM 8".to_vec(),
            ..Answer::default()
        });
        let sense = pt::fixed_sense(0x00, 0x00, 0x01);
        double.answers.push_back(Answer {
            status: pt::STATUS_CHECK,
            sense: sense.clone(),
            data: vec![1; 80],
            resid: 100,
            gone: false,
        });
        let handle = relay(far, double);
        let unit = PassedTape::new(Box::new(near));
        let (r, data) = run(&unit, &[0x12, 0, 0, 0, 0x24, 0], &[]);
        assert_eq!(r, Ok(17));
        assert_eq!(data, b"QUANTUM ULTRIUM 8");
        let (r, data) = run(&unit, &[0x08, 0, 0, 0, 0xB4, 0], &[]);
        assert_eq!(
            r,
            Err(Failure::Answer {
                status: pt::STATUS_CHECK,
                sense,
                given: 80
            })
        );
        assert_eq!(data.len(), 80);
        // Firmware is refused here and never reaches the relay.
        let (r, _) = run(&unit, &[0x3B, 5, 0, 0, 0, 0, 0, 0, 0x10, 0], &[0; 16]);
        assert!(matches!(r, Err(Failure::Answer { status: 2, .. })));
        // REPORT LUNS is the target's.
        let (r, _) = run(&unit, &[0xA0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0], &[]);
        assert_eq!(r, Ok(16));
        drop(unit);
        let drive = handle.join().unwrap();
        let ops: Vec<u8> = drive.seen.iter().map(|c| c.cdb[0]).collect();
        assert_eq!(ops, [0x12, 0x08]);
    }

    #[test]
    fn a_drive_gone_or_a_broken_link_is_a_transport_failure() {
        let (near, far) = UnixStream::pair().unwrap();
        let handle = relay(
            far,
            pt::Double {
                gone: true,
                ..pt::Double::default()
            },
        );
        let unit = PassedTape::new(Box::new(near));
        assert_eq!(run(&unit, &[0; 6], &[]).0, Err(Failure::Unreachable));
        drop(unit);
        handle.join().unwrap();
        let (near, far) = UnixStream::pair().unwrap();
        drop(far);
        let unit = PassedTape::new(Box::new(near));
        assert_eq!(run(&unit, &[0; 6], &[]).0, Err(Failure::Unreachable));
    }
}
