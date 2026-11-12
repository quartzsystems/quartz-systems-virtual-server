// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! A file lines are appended to. When the file cannot be written the
//! lines are held in memory and written, in order, once it can.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::rotated;

/// The most lines held in memory; past it the oldest are dropped.
const HELD_MAX: usize = 16_384;

/// Rotation by size under a total cap.
#[derive(Debug)]
struct Rotation {
    dir: PathBuf,
    name: String,
    file_bytes: u64,
    total_bytes: u64,
}

/// One file being appended to.
#[derive(Debug)]
pub struct Sink {
    path: PathBuf,
    rotation: Option<Rotation>,
    file: Option<File>,
    size: u64,
    held: VecDeque<String>,
}

impl Sink {
    /// `<dir>/<name>.ndjson`, rotated when it passes `file_bytes`, with
    /// the older files deleted to keep all of them under `total_bytes`.
    pub fn rotating(dir: &Path, name: &str, file_bytes: u64, total_bytes: u64) -> Self {
        Self {
            path: rotated(dir, name, 0),
            rotation: Some(Rotation {
                dir: dir.to_path_buf(),
                name: name.to_string(),
                file_bytes,
                total_bytes,
            }),
            file: None,
            size: 0,
            held: VecDeque::new(),
        }
    }

    /// A file that only grows.
    pub fn append_only(path: PathBuf) -> Self {
        Self {
            path,
            rotation: None,
            file: None,
            size: 0,
            held: VecDeque::new(),
        }
    }

    fn open(&mut self) -> bool {
        if self.file.is_some() {
            return true;
        }
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let Ok(file) = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.path)
        else {
            return false;
        };
        self.size = file.metadata().map_or(0, |m| m.len());
        self.file = Some(file);
        true
    }

    fn put(&mut self, line: &str) -> bool {
        if !self.open() {
            return false;
        }
        let text = format!("{line}\n");
        let written = self
            .file
            .as_mut()
            .is_some_and(|f| f.write_all(text.as_bytes()).is_ok());
        if written {
            self.size += text.len() as u64;
            self.rotate_if_full();
        } else {
            // Opened again on the next line.
            self.file = None;
        }
        written
    }

    /// Append one line, after any that were held.
    pub fn line(&mut self, line: &str) {
        while let Some(first) = self.held.pop_front() {
            if !self.put(&first) {
                self.held.push_front(first);
                break;
            }
        }
        if !self.held.is_empty() || !self.put(line) {
            if self.held.len() >= HELD_MAX {
                self.held.pop_front();
            }
            self.held.push_back(line.to_string());
        }
    }

    /// Bring what was written to the disk.
    pub fn sync(&mut self) {
        if let Some(file) = &self.file {
            let _ = file.sync_data();
        }
    }

    /// Lines held in memory.
    pub fn held(&self) -> usize {
        self.held.len()
    }

    fn rotate_if_full(&mut self) {
        let full = |r: &Rotation| self.size >= r.file_bytes;
        if !self.rotation.as_ref().is_some_and(full) {
            return;
        }
        self.sync();
        self.file = None;
        let Some(r) = &self.rotation else { return };
        // The older files, newest first, kept while they fit the cap
        // beside the one being retired and the one to come.
        let mut budget = r.total_bytes.saturating_sub(self.size + r.file_bytes);
        let mut keep = 0;
        loop {
            let old = rotated(&r.dir, &r.name, keep + 1);
            let Ok(meta) = fs::metadata(&old) else { break };
            if meta.len() > budget {
                break;
            }
            budget -= meta.len();
            keep += 1;
        }
        // Everything past the kept ones goes.
        let mut n = keep + 1;
        while fs::remove_file(rotated(&r.dir, &r.name, n)).is_ok() {
            n += 1;
        }
        for n in (1..=keep).rev() {
            let _ = fs::rename(rotated(&r.dir, &r.name, n), rotated(&r.dir, &r.name, n + 1));
        }
        let _ = fs::rename(&self.path, rotated(&r.dir, &r.name, 1));
        self.size = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::scratch;

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_log_rotates_by_size_and_stays_under_the_total() {
        let dir = scratch("rotate");
        // 100-byte lines: ten to a file, four files at most.
        let mut sink = Sink::rotating(&dir, "system", 1000, 4000);
        let line = |n: usize| format!("{n:099}");
        for n in 0..95 {
            sink.line(&line(n));
        }
        assert_eq!(
            names(&dir),
            [
                "system.1.ndjson",
                "system.2.ndjson",
                "system.3.ndjson",
                "system.ndjson"
            ]
        );
        let total: u64 = names(&dir)
            .iter()
            .map(|n| fs::metadata(dir.join(n)).unwrap().len())
            .sum();
        assert!(total <= 4000, "{total}");
        // The newest lines are there, in order, oldest file first.
        let mut all = String::new();
        for n in [
            "system.3.ndjson",
            "system.2.ndjson",
            "system.1.ndjson",
            "system.ndjson",
        ] {
            all.push_str(&fs::read_to_string(dir.join(n)).unwrap());
        }
        let kept: Vec<usize> = all.lines().map(|l| l.parse().unwrap()).collect();
        assert_eq!(kept, (60..95).collect::<Vec<_>>());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_existing_file_is_appended_to() {
        let dir = scratch("append");
        let path = dir.join("h.ndjson");
        for text in ["one", "two"] {
            let mut sink = Sink::append_only(path.clone());
            sink.line(text);
            sink.sync();
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lines_are_held_while_the_directory_cannot_be_written_then_flushed_in_order() {
        let dir = scratch("held");
        fs::create_dir_all(&dir).unwrap();
        // A file where the directory should be: nothing can be created.
        let blocked = dir.join("log");
        fs::write(&blocked, b"").unwrap();
        let mut sink = Sink::append_only(blocked.join("h.ndjson"));
        sink.line("first");
        sink.line("second");
        assert_eq!(sink.held(), 2);
        fs::remove_file(&blocked).unwrap();
        sink.line("third");
        assert_eq!(sink.held(), 0);
        assert_eq!(
            fs::read_to_string(blocked.join("h.ndjson")).unwrap(),
            "first\nsecond\nthird\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
