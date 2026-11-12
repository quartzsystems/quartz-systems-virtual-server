// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The kernel log, kept per boot: `kernel/<boot id>.log`, one line a
//! record, with the oldest boots' files deleted.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The directory under the log directory.
pub const DIR: &str = "kernel";

/// One record of `/dev/kmsg` as a line: `[    1.234567] <6> text`.
/// `None` for what is not a record.
pub fn line(record: &str) -> Option<String> {
    let (prefix, rest) = record.split_once(';')?;
    let mut fields = prefix.split(',');
    let priority: u32 = fields.next()?.parse().ok()?;
    let _sequence = fields.next()?;
    let micros: u64 = fields.next()?.parse().ok()?;
    // Lines after the first are the record's key and value pairs.
    let text = rest.lines().next().unwrap_or("");
    Some(format!(
        "[{:5}.{:06}] <{}> {text}",
        micros / 1_000_000,
        micros % 1_000_000,
        priority & 7
    ))
}

/// This boot's kernel log file.
#[derive(Debug)]
pub struct KernelLog {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    limit: u64,
}

impl KernelLog {
    /// The file for `boot` under `dir`, written up to `limit` bytes;
    /// only the newest `boots` files are kept.
    pub fn open(dir: &Path, boot: &str, limit: u64, boots: usize) -> Self {
        let dir = dir.join(DIR);
        let path = dir.join(format!("{boot}.log"));
        let _ = fs::create_dir_all(&dir);
        prune(&dir, &path, boots);
        Self {
            path,
            file: None,
            size: 0,
            limit,
        }
    }

    /// Append one record of `/dev/kmsg`.
    pub fn record(&mut self, record: &str) {
        let Some(line) = line(record) else { return };
        if self.size >= self.limit {
            return;
        }
        if self.file.is_none() {
            // A restarted log service reads the kernel's buffer from
            // its start again, so the file is written anew.
            self.file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.path)
                .ok();
        }
        if let Some(file) = &mut self.file
            && writeln!(file, "{line}").is_ok()
        {
            self.size += line.len() as u64 + 1;
        }
    }

    /// Bring it to the disk.
    pub fn sync(&mut self) {
        if let Some(file) = &self.file {
            let _ = file.sync_data();
        }
    }
}

/// Delete the oldest files of `dir` so that, with `current`, `boots`
/// are left.
fn prune(dir: &Path, current: &Path, boots: usize) {
    let mut old: Vec<(SystemTime, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p != current && p.extension().is_some_and(|x| x == "log"))
        .filter_map(|p| Some((fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    old.sort();
    let keep = boots.saturating_sub(1);
    let excess = old.len().saturating_sub(keep);
    for (_, path) in old.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tests::scratch;

    #[test]
    fn a_record_becomes_one_line() {
        assert_eq!(
            line("6,339,5140900,-;NET: Registered PF_INET protocol family\n SUBSYSTEM=net\n"),
            Some("[    5.140900] <6> NET: Registered PF_INET protocol family".into())
        );
        // The facility is not part of the level.
        assert_eq!(line("30,1,0,-;x").as_deref(), Some("[    0.000000] <6> x"));
        assert_eq!(line("not a record"), None);
        assert_eq!(line("a,b,c;x"), None);
    }

    #[test]
    fn each_boot_has_a_file_and_the_oldest_go() {
        let dir = scratch("kernel");
        for (n, boot) in ["b1", "b2", "b3", "b4"].iter().enumerate() {
            let mut log = KernelLog::open(&dir, boot, 1 << 20, 3);
            log.record(&format!("6,{n},1000000,-;boot {boot}"));
            log.sync();
            // Modification times a second apart on any file system.
            let when = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + n as u64);
            File::options()
                .write(true)
                .open(dir.join(DIR).join(format!("{boot}.log")))
                .unwrap()
                .set_modified(when)
                .unwrap();
        }
        let mut names: Vec<String> = fs::read_dir(dir.join(DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["b2.log", "b3.log", "b4.log"]);
        assert_eq!(
            fs::read_to_string(dir.join(DIR).join("b4.log")).unwrap(),
            "[    1.000000] <6> boot b4\n"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_boot_s_log_is_bounded() {
        let dir = scratch("kernel-cap");
        let mut log = KernelLog::open(&dir, "b", 100, 3);
        for n in 0..50 {
            log.record(&format!("6,{n},0,-;line {n}"));
        }
        let len = fs::metadata(dir.join(DIR).join("b.log")).unwrap().len();
        assert!((100..160).contains(&len), "{len}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
