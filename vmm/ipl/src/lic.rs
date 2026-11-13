// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The guest's LIC record, through QS/400's `licpart`: the side it
//! last completed an IPL from, and the two things a host sets before
//! an IPL as an operator would at the panel: the keylock position and
//! the console mode. A write is `licpart`'s: to the older copy, with
//! the next sequence and the record's checksum.
//!
//! Two facts of the record are QS/400's and are not in `licpart`
//! (`docs/QS400_GUEST.md`): the console mode's TLV tag and its two
//! values, which QS/400 keeps in its service tools crate.

use licpart::ipla::IplAttrs;
use licpart::{Device, IplMode, LicError, LicPart, Record, Slot};

/// The TLV the console mode is kept under: QS/400's
/// `TAG_CONSOLE_MODE`.
pub const TAG_CONSOLE_MODE: u16 = 0x0105;

/// The keylock position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keylock {
    /// Unattended.
    Normal,
    /// Attended: the IPL stops at its menu.
    Manual,
    /// Unattended, and remote and timed IPLs are allowed.
    Auto,
    /// Unattended, and the panel is locked.
    Secure,
}

impl Keylock {
    /// The value of the `KEYLCKPOS` IPL attribute.
    pub fn attribute(self) -> &'static str {
        match self {
            Keylock::Normal => "*NORMAL",
            Keylock::Manual => "*MANUAL",
            Keylock::Auto => "*AUTO",
            Keylock::Secure => "*SECURE",
        }
    }

    /// From the attribute's value.
    pub fn from_attribute(value: &str) -> Option<Self> {
        [
            Keylock::Normal,
            Keylock::Manual,
            Keylock::Auto,
            Keylock::Secure,
        ]
        .into_iter()
        .find(|k| k.attribute() == value)
    }

    /// The mode the IPL runs in.
    fn mode(self) -> IplMode {
        match self {
            Keylock::Manual => IplMode::Manual,
            _ => IplMode::Normal,
        }
    }
}

/// Where the guest's console is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleMode {
    /// The display and its keyboard. A partition has neither.
    Local,
    /// The serial port.
    Serial,
}

impl ConsoleMode {
    /// The mode's name, as a profile has it.
    pub fn name(self) -> &'static str {
        match self {
            ConsoleMode::Local => "local",
            ConsoleMode::Serial => "serial",
        }
    }

    fn byte(self) -> u8 {
        match self {
            ConsoleMode::Local => 1,
            ConsoleMode::Serial => 2,
        }
    }

    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            1 => Some(ConsoleMode::Local),
            2 => Some(ConsoleMode::Serial),
            _ => None,
        }
    }
}

/// What the host sets before an IPL; `None` leaves what is recorded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// The keylock position.
    pub keylock: Option<Keylock>,
    /// The console mode.
    pub console: Option<ConsoleMode>,
}

impl Settings {
    /// Whether anything is to be written.
    pub fn any(&self) -> bool {
        self.keylock.is_some() || self.console.is_some()
    }
}

/// What the record says of the IPL to come.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The side of the last IPL that completed.
    pub side: Slot,
    /// The keylock position.
    pub keylock: Keylock,
    /// The console mode; the display when the record says nothing.
    pub console: ConsoleMode,
    /// The record's sequence.
    pub sequence: u64,
}

/// Why the record cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// Neither copy is valid and the partition is not blank.
    #[error("no valid copy of the LIC record")]
    NoValidCopy,
    /// The partition could not be read or written.
    #[error("the LIC partition: {0}")]
    Device(String),
}

fn recorded(record: &Record) -> Recorded {
    let attributes = IplAttrs::of(record);
    let position = attributes
        .get("KEYLCKPOS")
        .and_then(Keylock::from_attribute)
        .unwrap_or(Keylock::Normal);
    // The one-time override decides between attended and not, as it
    // does in the guest's own IPL; the attribute says the rest.
    let keylock = match (record.header.next_mode, position) {
        (Some(IplMode::Manual), _) => Keylock::Manual,
        (Some(IplMode::Normal), Keylock::Manual) => Keylock::Normal,
        (_, position) => position,
    };
    Recorded {
        side: record.header.last_good_slot,
        keylock,
        console: record
            .tlv(TAG_CONSOLE_MODE)
            .and_then(|data| data.first().copied())
            .and_then(ConsoleMode::from_byte)
            .unwrap_or(ConsoleMode::Local),
        sequence: record.sequence,
    }
}

fn apply(record: &mut Record, settings: Settings) -> Result<(), LicError> {
    if let Some(keylock) = settings.keylock {
        let mut attributes = IplAttrs::of(record);
        attributes.set("KEYLCKPOS", keylock.attribute())?;
        attributes.store(record)?;
        // The guest takes this first and clears it; the attribute says
        // what `DSPIPLA` then shows.
        record.header.next_mode = Some(keylock.mode());
    }
    if let Some(console) = settings.console {
        record.set_tlv(TAG_CONSOLE_MODE, vec![console.byte()])?;
    }
    Ok(())
}

/// Read the record on `dev`, the LIC partition, and write `settings`
/// into it when there are any. `Ok(None)` for a blank partition with
/// nothing to set.
pub fn prepare<D: Device>(
    dev: D,
    settings: Settings,
    now: u64,
) -> Result<Option<Recorded>, RecordError> {
    let device = |e: LicError| RecordError::Device(e.to_string());
    let mut dev = dev;
    let is_blank = heads_blank(&mut dev);
    let mut part = LicPart::open(dev);
    let (mut part, mut record) = match part.read() {
        Ok(record) => (part, record),
        Err(LicError::NoValidCopy) if is_blank => {
            if !settings.any() {
                return Ok(None);
            }
            // The first record, as the guest would write it, so the
            // settings have somewhere to be.
            LicPart::init(part.into_device(), now).map_err(device)?
        }
        Err(LicError::NoValidCopy) => return Err(RecordError::NoValidCopy),
        Err(e) => return Err(device(e)),
    };
    if settings.any() {
        apply(&mut record, settings).map_err(device)?;
        part.write(&mut record).map_err(device)?;
    }
    Ok(Some(recorded(&record)))
}

/// A LIC partition whose copy A holds `payload` under the record's
/// framing and checksum, whatever `payload` is. For the fuzzer, to
/// reach what is behind the checksum. `None` when it does not fit.
pub fn sealed(payload: &[u8]) -> Option<Vec<u8>> {
    let copy = abrec::encode(&licpart::SPEC, 1, payload).ok()?;
    let mut partition = vec![0u8; 2 * licpart::SLOT_BYTES];
    partition.get_mut(..copy.len())?.copy_from_slice(&copy);
    Some(partition)
}

/// Whether both copies' first bytes are zero: a disk that has never
/// run an IPL. The guest writes the first record itself.
fn heads_blank<D: Device>(dev: &mut D) -> bool {
    use std::io::SeekFrom;
    [0u64, licpart::SLOT_BYTES as u64].iter().all(|offset| {
        let mut head = [0xFFu8; 64];
        dev.seek(SeekFrom::Start(*offset)).is_ok()
            && dev.read_exact(&mut head).is_ok()
            && head.iter().all(|b| *b == 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn partition() -> Cursor<Vec<u8>> {
        Cursor::new(vec![0u8; 4 * licpart::SLOT_BYTES])
    }

    fn installed() -> Cursor<Vec<u8>> {
        let (part, _) = LicPart::init(partition(), 1_700_000_000).unwrap();
        part.into_device()
    }

    #[test]
    fn a_fresh_record_says_side_a_normal_and_the_display() {
        let mut dev = installed();
        let got = prepare(&mut dev, Settings::default(), 0).unwrap().unwrap();
        assert_eq!(got.side, Slot::A);
        assert_eq!(got.keylock, Keylock::Normal);
        assert_eq!(got.console, ConsoleMode::Local);
        assert_eq!(got.sequence, 1);
        // Nothing was written.
        assert_eq!(LicPart::open(dev).read().unwrap().sequence, 1);
    }

    #[test]
    fn settings_are_written_by_the_record_s_own_rules() {
        let mut dev = installed();
        let settings = Settings {
            keylock: Some(Keylock::Manual),
            console: Some(ConsoleMode::Serial),
        };
        let got = prepare(&mut dev, settings, 0).unwrap().unwrap();
        assert_eq!(
            (got.keylock, got.console, got.sequence),
            (Keylock::Manual, ConsoleMode::Serial, 2)
        );
        // As the guest reads it: the override, the attribute, the TLV.
        let mut part = LicPart::open(dev);
        let record = part.read().unwrap();
        assert_eq!(record.header.next_mode, Some(IplMode::Manual));
        assert_eq!(IplAttrs::of(&record).get("KEYLCKPOS"), Some("*MANUAL"));
        assert_eq!(record.tlv(TAG_CONSOLE_MODE), Some(&[2u8][..]));
        // The older copy took the write: both copies are valid now.
        assert!(part.read_slots().iter().all(Option::is_some));
        // One setting leaves the other.
        let mut dev = part.into_device();
        let settings = Settings {
            keylock: Some(Keylock::Secure),
            console: None,
        };
        let got = prepare(&mut dev, settings, 0).unwrap().unwrap();
        assert_eq!(
            (got.keylock, got.console, got.sequence),
            (Keylock::Secure, ConsoleMode::Serial, 3)
        );
        let record = LicPart::open(dev).read().unwrap();
        assert_eq!(IplAttrs::of(&record).get("KEYLCKPOS"), Some("*SECURE"));
    }

    #[test]
    fn a_blank_partition_is_left_for_the_guest_unless_there_is_something_to_set() {
        let mut dev = partition();
        assert!(prepare(&mut dev, Settings::default(), 0).unwrap().is_none());
        assert!(dev.get_ref().iter().all(|b| *b == 0));
        let settings = Settings {
            keylock: None,
            console: Some(ConsoleMode::Serial),
        };
        let got = prepare(&mut dev, settings, 77).unwrap().unwrap();
        assert_eq!(got.console, ConsoleMode::Serial);
        let record = LicPart::open(dev).read().unwrap();
        assert_eq!(record.header.install_time, 77);
    }

    #[test]
    fn a_corrupt_record_is_an_error_and_one_good_copy_is_enough() {
        let mut bytes = installed().into_inner();
        // Damage copy A, the only one written.
        bytes[40] ^= 0xFF;
        let mut dev = Cursor::new(bytes);
        assert!(matches!(
            prepare(&mut dev, Settings::default(), 0),
            Err(RecordError::NoValidCopy)
        ));
        // Two copies, one damaged: the other is the record.
        let mut dev = installed();
        let settings = Settings {
            keylock: Some(Keylock::Auto),
            console: None,
        };
        prepare(&mut dev, settings, 0).unwrap();
        let mut bytes = dev.into_inner();
        bytes[licpart::SLOT_BYTES + 40] ^= 0xFF;
        let mut dev = Cursor::new(bytes);
        let got = prepare(&mut dev, Settings::default(), 0).unwrap().unwrap();
        assert_eq!(got.sequence, 1);
        // A partition too small to hold a record.
        let mut tiny = Cursor::new(vec![1u8; 100]);
        assert!(prepare(&mut tiny, Settings::default(), 0).is_err());
    }
}
