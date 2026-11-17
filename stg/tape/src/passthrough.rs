// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A physical tape drive passed through to a partition
//! (`docs/STORAGE.md`): each command of the guest's sent to the drive
//! and its answer, sense data and residue returned as they are, through
//! an allow-list. The VMM forwards a command as a [`Command`] frame to
//! the partition manager's relay, which checks it against [`allowed`]
//! again and has the [`Drive`] carry it out; the [`Answer`] frame comes
//! back. The relay reads what a VMM sends as hostile: a frame is
//! bounded and read whole or refused.
//!
//! The allow-list is what QS/400 and Linux's `st` send to a drive
//! (`docs/QS400_GUEST.md`) and the standard set a backup needs; firmware
//! download, diagnostics, a changer's commands and anything addressed
//! past the drive are never passed.

/// The most bytes a command moves either way: the most a block can be.
pub const MAX_TRANSFER: usize = 1 << 20;
/// The most bytes of a command descriptor block.
pub const MAX_CDB: usize = 16;
/// The most bytes of sense data.
pub const MAX_SENSE: usize = 96;

/// GOOD.
pub const STATUS_GOOD: u8 = 0x00;
/// CHECK CONDITION.
pub const STATUS_CHECK: u8 = 0x02;

/// The commands passed, by operation code, with their names for the log.
pub const ALLOWED: &[(u8, &str)] = &[
    (0x00, "TEST UNIT READY"),
    (0x01, "REWIND"),
    (0x03, "REQUEST SENSE"),
    (0x05, "READ BLOCK LIMITS"),
    (0x08, "READ(6)"),
    (0x0A, "WRITE(6)"),
    (0x10, "WRITE FILEMARKS(6)"),
    (0x11, "SPACE(6)"),
    (0x12, "INQUIRY"),
    (0x13, "VERIFY(6)"),
    (0x15, "MODE SELECT(6)"),
    (0x16, "RESERVE(6)"),
    (0x17, "RELEASE(6)"),
    (0x19, "ERASE(6)"),
    (0x1A, "MODE SENSE(6)"),
    (0x1B, "LOAD UNLOAD"),
    (0x1E, "PREVENT ALLOW MEDIUM REMOVAL"),
    (0x2B, "LOCATE(10)"),
    (0x34, "READ POSITION"),
    (0x44, "REPORT DENSITY SUPPORT"),
    (0x4D, "LOG SENSE"),
    (0x55, "MODE SELECT(10)"),
    (0x5A, "MODE SENSE(10)"),
    (0x80, "WRITE FILEMARKS(16)"),
    (0x8C, "READ ATTRIBUTE"),
    (0x91, "SPACE(16)"),
    (0x92, "LOCATE(16)"),
    (0xA0, "REPORT LUNS"),
];

/// Why a command is not passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Its operation code is not on the list: its number.
    NotAllowed(u8),
    /// A firmware download or a diagnostic: never passed.
    Never(u8),
    /// It names a logical unit other than the drive.
    PastTheDrive(u8),
    /// It is no command: empty, too long, or moving too much.
    Malformed,
}

impl Refusal {
    /// As the log says it.
    pub fn words(&self) -> String {
        match self {
            Refusal::NotAllowed(op) => format!("operation {op:#04x} is not on the allow-list"),
            Refusal::Never(op) => format!("operation {op:#04x} is never passed to a drive"),
            Refusal::PastTheDrive(op) => {
                format!("operation {op:#04x} is addressed past the drive")
            }
            Refusal::Malformed => "not a command a drive takes".into(),
        }
    }

    /// The sense data the guest is answered with: ILLEGAL REQUEST,
    /// invalid command operation code or invalid field in the CDB.
    pub fn sense(&self) -> Vec<u8> {
        let asc = match self {
            Refusal::PastTheDrive(_) | Refusal::Malformed => 0x24,
            _ => 0x20,
        };
        fixed_sense(0x05, asc, 0x00)
    }
}

/// Fixed-format sense data of `key`, `asc`, `ascq`.
pub fn fixed_sense(key: u8, asc: u8, ascq: u8) -> Vec<u8> {
    let mut s = vec![0u8; 18];
    s[0] = 0x70;
    s[2] = key;
    s[7] = 10;
    s[12] = asc;
    s[13] = ascq;
    s
}

/// The name of an allowed operation.
pub fn name_of(op: u8) -> Option<&'static str> {
    ALLOWED.iter().find(|(o, _)| *o == op).map(|(_, n)| *n)
}

/// Whether `cdb` may reach a drive.
pub fn allowed(cdb: &[u8]) -> Result<(), Refusal> {
    let Some(&op) = cdb.first() else {
        return Err(Refusal::Malformed);
    };
    if cdb.len() > MAX_CDB {
        return Err(Refusal::Malformed);
    }
    // WRITE BUFFER, READ BUFFER, SEND and RECEIVE DIAGNOSTIC, a
    // changer's MOVE MEDIUM and the maintenance commands.
    if matches!(op, 0x3B | 0x3C | 0x1C | 0x1D | 0xA5 | 0xA3 | 0xA4) {
        return Err(Refusal::Never(op));
    }
    if name_of(op).is_none() {
        return Err(Refusal::NotAllowed(op));
    }
    let wanted = match op {
        0x00..=0x1F => 6,
        0x20..=0x5F => 10,
        0x80..=0x9F => 16,
        _ => 12,
    };
    if cdb.len() < wanted {
        return Err(Refusal::Malformed);
    }
    // A six-byte command's second byte keeps the old LUN field: a drive
    // is LUN 0 of what it is given as.
    if wanted == 6 && cdb[1] & 0xE0 != 0 {
        return Err(Refusal::PastTheDrive(op));
    }
    Ok(())
}

/// A command as the VMM forwards it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    /// The command descriptor block.
    pub cdb: Vec<u8>,
    /// What the guest sends with it.
    pub data_out: Vec<u8>,
    /// How much the guest has room to be sent back.
    pub data_in: u32,
}

/// The drive's answer, as the relay gives it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    /// The SCSI status.
    pub status: u8,
    /// The sense data, as the drive gave it.
    pub sense: Vec<u8>,
    /// What the drive sent.
    pub data: Vec<u8>,
    /// What of the transfer the drive did not do.
    pub resid: u32,
    /// The drive is not there: nothing reached it.
    pub gone: bool,
}

impl Answer {
    /// A refusal of the relay's, which never reached the drive.
    pub fn refused(why: &Refusal, data_in: u32) -> Self {
        Self {
            status: STATUS_CHECK,
            sense: why.sense(),
            data: Vec::new(),
            resid: data_in,
            gone: false,
        }
    }

    /// The drive is gone.
    pub fn gone(data_in: u32) -> Self {
        Self {
            resid: data_in,
            gone: true,
            ..Self::default()
        }
    }
}

/// The drive is not there, with the system's words.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the drive is gone: {0}")]
pub struct Gone(pub String);

/// A drive: a real one through the kernel's SCSI generic driver, or a
/// test's.
pub trait Drive: Send {
    /// Carry out `command` as it is.
    fn execute(&mut self, command: &Command) -> Result<Answer, Gone>;
}

/// The relay's one step: `command` checked against the allow-list and
/// carried out by `drive`, or refused without it. Whether it reached
/// the drive.
pub fn relay(drive: &mut dyn Drive, command: &Command) -> (Answer, Result<(), Refusal>) {
    let too_much = command.data_out.len() > MAX_TRANSFER || command.data_in as usize > MAX_TRANSFER;
    let checked = allowed(&command.cdb).and(if too_much {
        Err(Refusal::Malformed)
    } else {
        Ok(())
    });
    match checked {
        Err(why) => (Answer::refused(&why, command.data_in), Err(why)),
        Ok(()) => match drive.execute(command) {
            Ok(mut a) => {
                a.sense.truncate(MAX_SENSE);
                a.data.truncate(command.data_in as usize);
                (a, Ok(()))
            }
            Err(_) => (Answer::gone(command.data_in), Ok(())),
        },
    }
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn get32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// The most bytes of a frame's body.
pub const MAX_FRAME: usize = MAX_TRANSFER + 256;

impl Command {
    /// Its frame: the body's length, then the CDB's length and bytes,
    /// the room for data in, and the data out.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::with_capacity(8 + self.cdb.len() + self.data_out.len());
        #[allow(clippy::cast_possible_truncation)] // 16 at most
        body.push(self.cdb.len().min(MAX_CDB) as u8);
        body.extend_from_slice(&self.cdb[..self.cdb.len().min(MAX_CDB)]);
        put32(&mut body, self.data_in);
        body.extend_from_slice(&self.data_out);
        let mut out = Vec::with_capacity(4 + body.len());
        put32(&mut out, u32::try_from(body.len()).unwrap_or(u32::MAX));
        out.extend_from_slice(&body);
        out
    }

    /// A frame's body read back; none when it is not one.
    pub fn decode(body: &[u8]) -> Option<Self> {
        let n = usize::from(*body.first()?);
        if n == 0 || n > MAX_CDB {
            return None;
        }
        let cdb = body.get(1..1 + n)?.to_vec();
        let data_in = get32(body, 1 + n)?;
        let data_out = body.get(5 + n..)?.to_vec();
        if data_out.len() > MAX_TRANSFER || data_in as usize > MAX_TRANSFER {
            return None;
        }
        Some(Self {
            cdb,
            data_out,
            data_in,
        })
    }
}

impl Answer {
    /// Its frame: the body's length, then the status, whether the drive
    /// is gone, the residue, the sense's length and bytes, the data.
    pub fn encode(&self) -> Vec<u8> {
        let sense = &self.sense[..self.sense.len().min(MAX_SENSE)];
        let mut body = Vec::with_capacity(8 + sense.len() + self.data.len());
        body.push(self.status);
        body.push(u8::from(self.gone));
        put32(&mut body, self.resid);
        #[allow(clippy::cast_possible_truncation)] // 96 at most
        body.push(sense.len() as u8);
        body.extend_from_slice(sense);
        body.extend_from_slice(&self.data);
        let mut out = Vec::with_capacity(4 + body.len());
        put32(&mut out, u32::try_from(body.len()).unwrap_or(u32::MAX));
        out.extend_from_slice(&body);
        out
    }

    /// A frame's body read back; none when it is not one.
    pub fn decode(body: &[u8]) -> Option<Self> {
        let status = *body.first()?;
        let gone = *body.get(1)? != 0;
        let resid = get32(body, 2)?;
        let n = usize::from(*body.get(6)?);
        if n > MAX_SENSE {
            return None;
        }
        let sense = body.get(7..7 + n)?.to_vec();
        let data = body.get(7 + n..)?.to_vec();
        if data.len() > MAX_TRANSFER {
            return None;
        }
        Some(Self {
            status,
            sense,
            data,
            resid,
            gone,
        })
    }
}

/// Read one frame's body from `r`: none at its end or when the length
/// is past [`MAX_FRAME`]. Nothing is allocated by a length beyond it.
pub fn read_frame(r: &mut dyn std::io::Read) -> std::io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other(format!("a frame of {len} bytes")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// A drive for tests: it answers each command from a script, records
/// what reached it, and can be made to go away.
#[derive(Debug, Default)]
pub struct Double {
    /// Every command that reached it.
    pub seen: Vec<Command>,
    /// What it answers, in turn; GOOD with no data when it runs out.
    pub answers: std::collections::VecDeque<Answer>,
    /// It is not there.
    pub gone: bool,
}

impl Drive for Double {
    fn execute(&mut self, command: &Command) -> Result<Answer, Gone> {
        if self.gone {
            return Err(Gone("the double was taken away".into()));
        }
        self.seen.push(command.clone());
        Ok(self.answers.pop_front().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(cdb: &[u8], data_in: u32) -> Command {
        Command {
            cdb: cdb.to_vec(),
            data_out: Vec::new(),
            data_in,
        }
    }

    #[test]
    fn what_a_save_sends_is_passed_and_firmware_never_is() {
        for cdb in [
            &[0x00, 0, 0, 0, 0, 0][..],
            &[0x08, 1, 0, 1, 0, 0],
            &[0x0A, 0, 1, 0, 0, 0],
            &[0x10, 0, 0, 0, 1, 0],
            &[0x11, 1, 0, 0, 1, 0],
            &[0x1B, 0, 0, 0, 1, 0],
            &[0x2B, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0x34, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            &[0x5A, 0, 0x3F, 0, 0, 0, 0, 0, 0xFF, 0],
            &[0xA0, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0],
        ] {
            assert_eq!(allowed(cdb), Ok(()), "{cdb:02x?}");
        }
        let firmware = [0x3B, 0x05, 0, 0, 0, 0, 0, 0x10, 0, 0];
        assert_eq!(allowed(&firmware), Err(Refusal::Never(0x3B)));
        assert_eq!(allowed(&[0xA5; 12]), Err(Refusal::Never(0xA5)));
        assert_eq!(
            allowed(&[0x04, 0, 0, 0, 0, 0]),
            Err(Refusal::NotAllowed(0x04))
        );
        // The old LUN field: past the drive.
        assert_eq!(
            allowed(&[0x00, 0x20, 0, 0, 0, 0]),
            Err(Refusal::PastTheDrive(0))
        );
        assert_eq!(allowed(&[]), Err(Refusal::Malformed));
        assert_eq!(allowed(&[0x2B, 0, 0]), Err(Refusal::Malformed));
        assert_eq!(allowed(&[0x00; 17]), Err(Refusal::Malformed));
    }

    #[test]
    fn an_allowed_command_reaches_the_drive_and_comes_back_as_it_is() {
        let mut d = Double::default();
        let sense = fixed_sense(0x00, 0x00, 0x01);
        d.answers.push_back(Answer {
            status: STATUS_CHECK,
            sense: sense.clone(),
            data: vec![7; 80],
            resid: 65_456,
            gone: false,
        });
        let read = cmd(&[0x08, 0, 0x01, 0, 0, 0], 65_536);
        let (a, reached) = relay(&mut d, &read);
        assert_eq!(reached, Ok(()));
        assert_eq!(d.seen, [read]);
        assert_eq!(
            (a.status, a.sense, a.data.len(), a.resid),
            (STATUS_CHECK, sense, 80, 65_456)
        );
    }

    #[test]
    fn a_refused_command_never_reaches_the_drive() {
        let mut d = Double::default();
        let (a, reached) = relay(&mut d, &cmd(&[0x3B, 0, 0, 0, 0, 0, 0, 0, 0, 0], 0));
        assert_eq!(reached, Err(Refusal::Never(0x3B)));
        assert!(d.seen.is_empty());
        assert_eq!(a.status, STATUS_CHECK);
        assert_eq!((a.sense[2], a.sense[12]), (0x05, 0x20));
        let mut big = cmd(&[0x0A, 0, 0, 0, 0, 0], 0);
        big.data_out = vec![0; MAX_TRANSFER + 1];
        assert_eq!(relay(&mut d, &big).1, Err(Refusal::Malformed));
        assert!(d.seen.is_empty());
    }

    #[test]
    fn a_drive_gone_answers_gone() {
        let mut d = Double {
            gone: true,
            ..Double::default()
        };
        let (a, reached) = relay(&mut d, &cmd(&[0x00; 6], 0));
        assert_eq!(reached, Ok(()));
        assert!(a.gone);
    }

    #[test]
    fn frames_read_back_and_a_hostile_one_is_refused() {
        let c = Command {
            cdb: vec![0x0A, 0, 0, 0, 0x50, 0],
            data_out: vec![1; 80],
            data_in: 0,
        };
        let f = c.encode();
        let body = read_frame(&mut &f[..]).unwrap().unwrap();
        assert_eq!(Command::decode(&body), Some(c));
        let a = Answer {
            status: STATUS_GOOD,
            sense: Vec::new(),
            data: vec![9; 6],
            resid: 0,
            gone: false,
        };
        let f = a.encode();
        assert_eq!(
            Answer::decode(&read_frame(&mut &f[..]).unwrap().unwrap()),
            Some(a)
        );
        // A length past the bound is refused before anything is kept.
        let huge = u32::MAX.to_le_bytes();
        assert!(read_frame(&mut &huge[..]).is_err());
        assert_eq!(read_frame(&mut &[][..]).unwrap(), None);
        for bad in [&[][..], &[0], &[17], &[1, 0], &[1, 0, 1, 2]] {
            assert_eq!(Command::decode(bad), None, "{bad:?}");
        }
        assert_eq!(Answer::decode(&[0, 0, 0, 0, 0, 0, 97]), None);
    }
}
