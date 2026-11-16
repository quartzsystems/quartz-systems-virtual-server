// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The reference code history (`refcodes.ndjson`, the log service's,
//! never rotated), read from a place: the byte after the last whole
//! line taken, and how many codes were before it. A code's place in
//! the history counts the lines that read, as the QSI server's pages
//! do.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};
use supervise::control::About;

use crate::book::Posting;
use crate::record::Posted;

/// Where the history was read to.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// The byte after the last whole line taken.
    pub offset: u64,
    /// How many codes were before it.
    pub seq: u64,
}

/// The most read at one time.
const MAX_READ: u64 = 4 * 1024 * 1024;

#[derive(Deserialize)]
struct HistoryLine {
    time: String,
    code: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    about: Vec<About>,
}

/// The posting a history line holds, the `seq`th; none when it does
/// not read.
#[must_use]
pub fn posting(line: &str, seq: u64) -> Option<Posting> {
    let l: HistoryLine = serde_json::from_str(line).ok()?;
    let secs = appliance::time::parse_rfc3339(&l.time)
        .and_then(|s| u64::try_from(s).ok())
        .unwrap_or(0);
    Some(Posting {
        posted: Posted {
            seq,
            code: l.code,
            words: Vec::new(),
            text: l.text,
            time_ms: secs.saturating_mul(1000),
        },
        about: l.about,
    })
}

/// What `bytes`, the history from `from`, holds in whole lines: each
/// posting, and the place after the last.
#[must_use]
pub fn postings(bytes: &[u8], from: Cursor) -> (Vec<Posting>, Cursor) {
    let mut out = Vec::new();
    let mut at = from;
    let mut rest = bytes;
    while let Some(end) = rest.iter().position(|b| *b == b'\n') {
        let line = String::from_utf8_lossy(&rest[..end]);
        if let Some(p) = posting(&line, at.seq + 1) {
            at.seq += 1;
            out.push(p);
        }
        at.offset += end as u64 + 1;
        rest = &rest[end + 1..];
    }
    (out, at)
}

/// What the history file at `path` holds after `from`. A file shorter
/// than the place (made again) is read from its start.
///
/// # Errors
/// The file could not be read.
pub fn read(path: &Path, from: Cursor) -> std::io::Result<(Vec<Posting>, Cursor)> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let from = if len < from.offset {
        Cursor::default()
    } else {
        from
    };
    f.seek(SeekFrom::Start(from.offset))?;
    let mut bytes = Vec::new();
    f.take(MAX_READ).read_to_end(&mut bytes)?;
    Ok(postings(&bytes, from))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str =
        r#"{"time":"2026-11-16T10:00:00Z","boot":"b","code":"C2001000","text":"Host IPL started"}"#;
    const B: &str = r#"{"time":"2026-11-16T10:00:05Z","boot":"b","code":"A2007020","text":"Pool unit failed: POOL1","about":[{"kind":"storage-pool","id":0,"name":"POOL1","location":"","serial":""}]}"#;

    #[test]
    fn whole_lines_are_taken_and_a_part_waits() {
        let text = format!("{A}\nnot json\n{B}\n{{\"time\":");
        let (got, at) = postings(text.as_bytes(), Cursor::default());
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].posted.seq, got[1].posted.seq), (1, 2));
        assert_eq!(got[1].about[0].name, "POOL1");
        assert_eq!(got[1].posted.time_ms - got[0].posted.time_ms, 5000);
        assert_eq!(at.seq, 2);
        assert_eq!(at.offset, text.rfind('\n').unwrap() as u64 + 1);
        // Read on from there: the part line once it is whole.
        let more = format!("{A}\n");
        let (got, at2) = postings(more.as_bytes(), at);
        assert_eq!(got[0].posted.seq, 3);
        assert_eq!(at2.offset, at.offset + more.len() as u64);
    }

    #[test]
    fn the_file_is_read_from_the_place() {
        let d = std::env::temp_dir().join(format!("problem-feed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("refcodes.ndjson");
        std::fs::write(&path, format!("{A}\n")).unwrap();
        let (got, at) = read(&path, Cursor::default()).unwrap();
        assert_eq!(got.len(), 1);
        std::fs::write(&path, format!("{A}\n{B}\n")).unwrap();
        let (got, at) = read(&path, at).unwrap();
        assert_eq!(
            (got.len(), got[0].posted.code.as_str(), at.seq),
            (1, "A2007020", 2)
        );
        // Made again shorter: from its start.
        std::fs::write(&path, format!("{B}\n")).unwrap();
        let (got, _) = read(&path, at).unwrap();
        assert_eq!((got.len(), got[0].posted.seq), (1, 1));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
