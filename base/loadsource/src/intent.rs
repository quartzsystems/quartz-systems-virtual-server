// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The write-intent maps of a mirrored load source, kept on every
//! load-source disk in the tails of the label's two copies, which the
//! label never reaches: the set's own map in the first, the map of a
//! member out of service in the second. A host installed before the
//! maps has zeros there, which read as no map: its first unclean stop
//! after the change copies every region once, as before.
//!
//! Each disk holds a copy; a write goes to every disk. The copies are
//! read together and their bits joined, so a copy that missed a write
//! can only make more regions dirty, never fewer.

use std::io::{self, SeekFrom};

use abrec::Device;
use protect::intent::{DEFAULT_REGION_BYTES, HEADER_LEN, IntentMap, IntentStore};

use crate::set::{BLOCK, LABEL_OFFSET};

/// Where a map starts within a copy of the label.
pub const MAP_IN_SLOT: u64 = 32 * 1024;
/// Bytes kept for one map.
pub const MAP_BYTES: usize = 32 * 1024;
/// Where the set's map starts in the record partition.
pub const SET_MAP_OFFSET: u64 = LABEL_OFFSET + MAP_IN_SLOT;
/// Where the map of a member out of service starts.
pub const AWAY_MAP_OFFSET: u64 = LABEL_OFFSET + abrec::SLOT_BYTES as u64 + MAP_IN_SLOT;

/// Blocks per region for a set of `blocks`: 16 MiB, doubled until the
/// map fits its room. Fixed by the set's size, so every IPL agrees.
pub fn region_blocks(blocks: u64) -> u64 {
    let mut rb = DEFAULT_REGION_BYTES / BLOCK;
    while (HEADER_LEN as u64) + IntentMap::regions_for(blocks, rb).div_ceil(8) > MAP_BYTES as u64 {
        rb *= 2;
    }
    rb
}

/// One map on the record partitions of the disks given.
pub struct MapStore<D> {
    devs: Vec<D>,
    base: u64,
    magic: [u8; 8],
}

impl<D> MapStore<D> {
    /// The map from byte `base` of each of `devs`, under `magic`.
    pub fn new(devs: Vec<D>, base: u64, magic: [u8; 8]) -> Self {
        Self { devs, base, magic }
    }
}

fn same(a: &IntentMap, b: &IntentMap) -> bool {
    a.kind == b.kind
        && a.set_id == b.set_id
        && a.member == b.member
        && a.region_blocks == b.region_blocks
        && a.regions == b.regions
}

impl<D: Device + Send> IntentStore for MapStore<D> {
    fn read_all(&mut self) -> io::Result<Vec<u8>> {
        let mut joined: Option<IntentMap> = None;
        for dev in &mut self.devs {
            let mut buf = vec![0u8; MAP_BYTES];
            let read = dev
                .seek(SeekFrom::Start(self.base))
                .and_then(|_| dev.read_exact(&mut buf));
            let Some(map) = read
                .ok()
                .and_then(|()| IntentMap::decode(&self.magic, &buf))
            else {
                continue;
            };
            match &mut joined {
                None => joined = Some(map),
                Some(j) if same(j, &map) => j.merge(&map),
                // Another set's or another member's: not this map.
                Some(_) => {}
            }
        }
        Ok(joined.map(|m| m.encode(&self.magic)).unwrap_or_default())
    }

    fn write(&mut self, offset: u64, bytes: &[u8]) -> io::Result<()> {
        let end = offset.saturating_add(bytes.len() as u64);
        if end > MAP_BYTES as u64 {
            return Err(io::Error::other(format!(
                "{end} bytes of map; there is room for {MAP_BYTES}"
            )));
        }
        let mut last = None;
        let mut written = 0;
        for dev in &mut self.devs {
            let done = dev
                .seek(SeekFrom::Start(self.base + offset))
                .and_then(|_| dev.write_all(bytes))
                .and_then(|()| dev.sync());
            match done {
                Ok(()) => written += 1,
                Err(e) => last = Some(e),
            }
        }
        // A disk that cannot take the write is failing as a member too;
        // the map holds while one disk has it.
        match (written, last) {
            (0, Some(e)) => Err(e),
            (0, None) => Err(io::Error::other("no load-source disk for the map")),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::set::label_spec;
    use appliance::LoadSourceNames;
    use protect::intent::MapKind;
    use std::io::{Cursor, Read, Seek, Write};
    use std::sync::{Arc, Mutex};

    const MAGIC: [u8; 8] = *b"QSDMIMAP";

    /// A disk the test can look into after the store has written it.
    #[derive(Clone)]
    struct Shared(Arc<Mutex<Cursor<Vec<u8>>>>, bool);

    impl Shared {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Cursor::new(vec![0; 1 << 20]))), false)
        }
        fn failing() -> Self {
            Self(Arc::new(Mutex::new(Cursor::new(Vec::new()))), true)
        }
        fn peek(&self, at: u64, n: usize) -> Vec<u8> {
            let c = self.0.lock().unwrap();
            let at = usize::try_from(at).unwrap();
            c.get_ref()[at..at + n].to_vec()
        }
    }

    impl Read for Shared {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.0.lock().unwrap().read(buf)
        }
    }
    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.1 {
                return Err(io::Error::other("pulled"));
            }
            self.0.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Seek for Shared {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.0.lock().unwrap().seek(pos)
        }
    }
    impl Device for Shared {}

    fn map(blocks: u64) -> IntentMap {
        IntentMap::new(
            MapKind::Set,
            [7; 16],
            [0; 16],
            blocks,
            region_blocks(blocks),
        )
    }

    #[test]
    fn the_maps_lie_beyond_the_largest_label_and_inside_its_copies() {
        let spec = label_spec(&LoadSourceNames::DEMO);
        // A label of 255 members, the most its count can say.
        let largest = abrec::HEAD_LEN + 36 + 255 * 84;
        assert!((largest as u64) < MAP_IN_SLOT);
        assert!(MAP_IN_SLOT + MAP_BYTES as u64 <= spec.slot_bytes as u64);
        assert_eq!(SET_MAP_OFFSET, LABEL_OFFSET + MAP_IN_SLOT);
        assert_eq!(AWAY_MAP_OFFSET, SET_MAP_OFFSET + spec.slot_bytes as u64);
        assert!(AWAY_MAP_OFFSET + MAP_BYTES as u64 <= crate::set::RESUME_OFFSET);
    }

    #[test]
    fn a_region_grows_only_when_the_map_would_not_fit() {
        let tib = (1u64 << 40) / BLOCK;
        assert_eq!(region_blocks(tib), DEFAULT_REGION_BYTES / BLOCK);
        assert_eq!(region_blocks(3 * tib), DEFAULT_REGION_BYTES / BLOCK);
        assert_eq!(region_blocks(5 * tib), 2 * DEFAULT_REGION_BYTES / BLOCK);
        for size in [1, tib, 5 * tib, 64 * tib, 1000 * tib] {
            let rb = region_blocks(size);
            assert!(rb.is_multiple_of(256), "a multiple of the resume chunk");
            assert!(map(size).encode(&MAGIC).len() <= MAP_BYTES);
        }
    }

    #[test]
    fn a_write_reaches_every_disk_and_the_copies_are_joined() {
        let (a, b) = (Shared::new(), Shared::new());
        let mut store = MapStore::new(vec![a.clone(), b.clone()], SET_MAP_OFFSET, MAGIC);
        assert!(store.read_all().unwrap().is_empty(), "zeros are no map");
        let mut m = map(1 << 20);
        m.set(3);
        let bytes = m.encode(&MAGIC);
        store.write(0, &bytes).unwrap();
        assert_eq!(a.peek(SET_MAP_OFFSET, bytes.len()), bytes);
        assert_eq!(b.peek(SET_MAP_OFFSET, bytes.len()), bytes);
        // Disk b alone takes a later bit: both bits are read back.
        let mut only_b = MapStore::new(vec![b.clone()], SET_MAP_OFFSET, MAGIC);
        let mut later = m.clone();
        later.set(9);
        only_b.write(0, &later.encode(&MAGIC)).unwrap();
        let read = IntentMap::decode(&MAGIC, &store.read_all().unwrap()).unwrap();
        assert_eq!(read.dirty().collect::<Vec<_>>(), [3, 9]);
        // Nothing before the map was touched: the label's room is zeros.
        assert!(
            a.peek(LABEL_OFFSET, usize::try_from(MAP_IN_SLOT).unwrap())
                .iter()
                .all(|b| *b == 0)
        );
    }

    #[test]
    fn a_disk_that_fails_does_not_fail_the_map_while_another_holds_it() {
        let a = Shared::new();
        let mut store = MapStore::new(vec![a.clone(), Shared::failing()], AWAY_MAP_OFFSET, MAGIC);
        store.write(0, &map(1 << 20).encode(&MAGIC)).unwrap();
        assert!(store.read_all().is_ok_and(|b| !b.is_empty()));
        let mut none = MapStore::new(vec![Shared::failing()], AWAY_MAP_OFFSET, MAGIC);
        assert!(none.write(0, &[1]).is_err());
        assert!(
            store.write(MAP_BYTES as u64, &[1]).is_err(),
            "past its room"
        );
    }

    #[test]
    fn another_set_s_map_is_not_joined() {
        let (a, b) = (Shared::new(), Shared::new());
        let mut m = map(1 << 20);
        m.set(1);
        MapStore::new(vec![a.clone()], 0, MAGIC)
            .write(0, &m.encode(&MAGIC))
            .unwrap();
        let mut other = IntentMap::new(
            MapKind::Set,
            [8; 16],
            [0; 16],
            1 << 20,
            region_blocks(1 << 20),
        );
        other.set(2);
        MapStore::new(vec![b.clone()], 0, MAGIC)
            .write(0, &other.encode(&MAGIC))
            .unwrap();
        let read = MapStore::new(vec![a, b], 0, MAGIC).read_all().unwrap();
        let read = IntentMap::decode(&MAGIC, &read).unwrap();
        assert_eq!(read.dirty().collect::<Vec<_>>(), [1]);
    }
}
