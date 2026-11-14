// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The AWSTAPE header. A volume is a sequence of chunks, each a header
//! and its bytes: a block of data is one chunk, or several when it is
//! larger than a chunk holds, the first marked as a block's beginning
//! and the last as its end; a filemark is a header alone. The header
//! is six bytes: this chunk's length and the one before it, little
//! endian, then the flags and a byte that is zero. The end of the file
//! is the end of data.

/// Bytes of a header.
pub const HEADER: usize = 6;
/// The most bytes a chunk holds.
pub const MAX_CHUNK: usize = 0xFFFF;

/// The chunk begins a block.
pub const BEGIN: u8 = 0x80;
/// The chunk is a filemark.
pub const FILEMARK: u8 = 0x40;
/// The chunk ends a block.
pub const END: u8 = 0x20;

/// One chunk's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// Bytes of this chunk's data.
    pub len: u16,
    /// Bytes of the chunk before it; 0 for the first.
    pub prev: u16,
    /// [`BEGIN`], [`END`], [`FILEMARK`].
    pub flags: u8,
}

impl Header {
    /// A filemark after a chunk of `prev` bytes.
    pub fn filemark(prev: u16) -> Self {
        Self {
            len: 0,
            prev,
            flags: FILEMARK,
        }
    }

    /// Whether it is a filemark.
    pub fn is_filemark(&self) -> bool {
        self.flags & FILEMARK != 0
    }

    /// The six bytes.
    pub fn encode(&self) -> [u8; HEADER] {
        let mut b = [0u8; HEADER];
        b[..2].copy_from_slice(&self.len.to_le_bytes());
        b[2..4].copy_from_slice(&self.prev.to_le_bytes());
        b[4] = self.flags;
        b
    }

    /// From six bytes; `None` when they are not a header: unknown
    /// flags, a filemark with data, or a chunk of data with none.
    pub fn decode(b: &[u8; HEADER]) -> Option<Self> {
        let h = Self {
            len: u16::from_le_bytes([b[0], b[1]]),
            prev: u16::from_le_bytes([b[2], b[3]]),
            flags: b[4],
        };
        let known = h.flags & !(BEGIN | END | FILEMARK) == 0;
        let shape = if h.is_filemark() {
            h.len == 0 && h.flags == FILEMARK
        } else {
            h.len > 0
        };
        (known && shape && b[5] == 0).then_some(h)
    }

    /// The headers a block of `len` bytes is written with, after a
    /// chunk of `prev` bytes: each with its length.
    pub fn for_block(len: usize, prev: u16) -> Vec<Header> {
        let mut out = Vec::new();
        let mut left = len;
        let mut prev = prev;
        while left > 0 || out.is_empty() {
            let n = left.min(MAX_CHUNK);
            let mut flags = 0;
            if out.is_empty() {
                flags |= BEGIN;
            }
            if n == left {
                flags |= END;
            }
            #[allow(clippy::cast_possible_truncation)] // at most MAX_CHUNK
            let len16 = n as u16;
            out.push(Header {
                len: len16,
                prev,
                flags,
            });
            prev = len16;
            left -= n;
            if n == 0 {
                break;
            }
        }
        out
    }
}

/// Bytes a block of `len` bytes takes in the image.
pub fn block_bytes(len: usize) -> u64 {
    let chunks = len.div_ceil(MAX_CHUNK).max(1);
    (len + chunks * HEADER) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_round_trips_and_nonsense_is_refused() {
        let h = Header {
            len: 80,
            prev: 0,
            flags: BEGIN | END,
        };
        assert_eq!(h.encode(), [80, 0, 0, 0, 0xA0, 0]);
        assert_eq!(Header::decode(&h.encode()), Some(h));
        assert_eq!(
            Header::decode(&Header::filemark(80).encode()),
            Some(Header::filemark(80))
        );
        assert_eq!(Header::decode(&[0, 0, 0, 0, 0xA0, 0]), None, "no data");
        assert_eq!(
            Header::decode(&[1, 0, 0, 0, 0x40, 0]),
            None,
            "data in a mark"
        );
        assert_eq!(
            Header::decode(&[1, 0, 0, 0, 0x01, 0]),
            None,
            "an unknown flag"
        );
        assert_eq!(Header::decode(&[1, 0, 0, 0, 0xA0, 7]), None);
    }

    #[test]
    fn a_large_block_is_split_into_chunks() {
        let hs = Header::for_block(150_000, 12);
        assert_eq!(hs.len(), 3);
        assert_eq!(hs[0].flags, BEGIN);
        assert_eq!((hs[0].len, hs[0].prev), (0xFFFF, 12));
        assert_eq!(hs[1].flags, 0);
        assert_eq!(hs[2].flags, END);
        assert_eq!(
            hs.iter().map(|h| usize::from(h.len)).sum::<usize>(),
            150_000
        );
        assert_eq!(hs[2].prev, 0xFFFF);
        assert_eq!(block_bytes(150_000), 150_000 + 18);
        let one = Header::for_block(10, 0);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].flags, BEGIN | END);
        assert_eq!(block_bytes(MAX_CHUNK), MAX_CHUNK as u64 + 6);
    }
}
