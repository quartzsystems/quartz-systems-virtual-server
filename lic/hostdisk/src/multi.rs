// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! One record kept on every load-source disk. Each disk holds its own
//! two copies (QS/400's `abrec`); the copy with the highest valid
//! sequence on any disk is the record, and a write goes to every disk
//! with one sequence, so the disks agree again after one was away.

use std::io::{self, Read, Seek, SeekFrom, Write};

use abrec::{AbError, AbPart, Device, Slot, Spec, Stored, target};

/// A device seen from `base` on: a record that does not start its
/// partition.
pub struct At<D> {
    dev: D,
    base: u64,
}

impl<D> At<D> {
    /// `dev` from byte `base`.
    pub fn new(dev: D, base: u64) -> Self {
        Self { dev, base }
    }
}

impl<D: Read> Read for At<D> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.dev.read(buf)
    }
}

impl<D: Write> Write for At<D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.dev.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.dev.flush()
    }
}

impl<D: Seek> Seek for At<D> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let to = match pos {
            SeekFrom::Start(n) => SeekFrom::Start(self.base + n),
            other => other,
        };
        Ok(self.dev.seek(to)?.saturating_sub(self.base))
    }
}

impl<D: Device> Device for At<D> {
    fn sync(&mut self) -> io::Result<()> {
        self.dev.sync()
    }
}

/// The record's copies on every disk that was found.
pub struct Copies<D> {
    spec: Spec,
    parts: Vec<AbPart<D>>,
}

impl<D: Device> Copies<D> {
    /// The record of `spec` on `devices`, one a disk.
    pub fn new(spec: Spec, devices: Vec<D>) -> Self {
        Self {
            spec,
            parts: devices
                .into_iter()
                .map(|dev| AbPart::open(spec, dev))
                .collect(),
        }
    }

    /// How many disks hold it.
    pub fn disks(&self) -> usize {
        self.parts.len()
    }

    /// The newest valid copy of each disk, in disk order.
    pub fn each(&mut self) -> Vec<Option<Stored>> {
        self.parts.iter_mut().map(|p| p.read().ok()).collect()
    }

    /// The copy with the highest sequence on any disk; the first disk's
    /// when two agree.
    pub fn read(&mut self) -> Result<Stored, AbError> {
        let mut best: Option<Stored> = None;
        for found in self.each().into_iter().flatten() {
            if best.as_ref().is_none_or(|b| found.sequence > b.sequence) {
                best = Some(found);
            }
        }
        best.ok_or(AbError::NoValidCopy)
    }

    /// Write `payload` to every disk, over each disk's older copy, with
    /// one sequence past every copy there is. The sequence, and how
    /// many disks took it; an error when none did.
    pub fn write(&mut self, payload: &[u8]) -> Result<(u64, usize), AbError> {
        let mut targets: Vec<Slot> = Vec::with_capacity(self.parts.len());
        let mut sequence = 1;
        for part in &mut self.parts {
            let seen = part.read_slots().map(|s| s.map(|s| s.sequence));
            let (slot, next) = target(seen);
            targets.push(slot);
            sequence = sequence.max(next);
        }
        let mut written = 0;
        let mut failed = None;
        for (part, slot) in self.parts.iter_mut().zip(targets) {
            match part.write_slot(slot, sequence, payload) {
                Ok(()) => written += 1,
                Err(e) => failed = Some(e),
            }
        }
        match (written, failed) {
            (0, Some(e)) => Err(e),
            (0, None) => Err(AbError::NotFound),
            _ => Ok((sequence, written)),
        }
    }

    /// The spec the copies are read by.
    pub fn spec(&self) -> &Spec {
        &self.spec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const SPEC: Spec = Spec {
        magic: *b"QSVSTEST",
        version: 1,
        type_guid: uuid::Uuid::nil(),
        slot_bytes: 4096,
        min_payload: 1,
    };

    fn disk() -> At<Cursor<Vec<u8>>> {
        At::new(Cursor::new(vec![0u8; 3 * 8192]), 8192)
    }

    #[test]
    fn the_highest_sequence_wins_and_a_write_goes_to_both() {
        let mut both = Copies::new(SPEC, vec![disk(), disk()]);
        assert!(matches!(both.read(), Err(AbError::NoValidCopy)));
        assert_eq!(both.write(b"one").unwrap(), (1, 2));
        assert_eq!(both.write(b"two").unwrap(), (2, 2));
        assert_eq!(both.read().unwrap().payload, b"two");

        // The second disk is away for two writes, then back.
        let mut parts = std::mem::take(&mut both.parts);
        let away = parts.pop().unwrap();
        let mut one = Copies { spec: SPEC, parts };
        one.write(b"three").unwrap();
        assert_eq!(one.write(b"four").unwrap(), (4, 1));
        one.parts.push(away);
        let each: Vec<u64> = one
            .each()
            .into_iter()
            .map(|s| s.unwrap().sequence)
            .collect();
        assert_eq!(each, [4, 2]);
        assert_eq!(one.read().unwrap().payload, b"four");
        // The next write brings both to one sequence.
        assert_eq!(one.write(b"five").unwrap(), (5, 2));
        let each: Vec<u64> = one
            .each()
            .into_iter()
            .map(|s| s.unwrap().sequence)
            .collect();
        assert_eq!(each, [5, 5]);
    }

    #[test]
    fn the_record_is_where_the_window_starts() {
        let mut one = Copies::new(SPEC, vec![disk()]);
        one.write(b"here").unwrap();
        let bytes = one.parts.pop().unwrap().into_device().dev.into_inner();
        assert!(bytes[..8192].iter().all(|b| *b == 0));
        assert_eq!(&bytes[8192..8200], b"QSVSTEST");
    }
}
