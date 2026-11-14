// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Virtual disks: raw files in a pool's `vdisks` directory, one a disk,
//! named `<name>.img`. A thin disk is sparse: it takes space as the
//! guest writes it and gives it back when the guest discards. A thick
//! disk has all of its space allocated when it is made, so a write to
//! it can never find the pool full. Which kind a disk is, is kept on
//! the file itself, in an extended attribute.
//!
//! A disk is opened for a partition with direct I/O, read and write;
//! the pool service hands the open file to the partition manager, and
//! records whose it is (`docs/STORAGE.md`).

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::{FallocateFlags, XattrFlags, fallocate, fgetxattr, fsetxattr};

/// The directory of a pool that holds its virtual disks.
pub const DIR: &str = "vdisks";
/// A disk's file is its name and this.
pub const SUFFIX: &str = ".img";
/// A name's most characters.
pub const NAME_MAX: usize = 32;
/// The least size: a megabyte.
pub const MIN_BYTES: u64 = 1 << 20;
/// The most: 64 TiB, the most a pool is.
pub const MAX_BYTES: u64 = 64 << 40;
/// Sizes are whole megabytes.
pub const GRANULE: u64 = 1 << 20;

const KIND_ATTR: &str = "user.qsvs.vdisk";
/// `O_DIRECT`.
const O_DIRECT: i32 = 0o40000;

/// How a disk's space is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Sparse: space as it is written.
    Thin,
    /// Allocated whole when made.
    Thick,
}

impl Kind {
    /// As it is written and shown.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Thin => "thin",
            Kind::Thick => "thick",
        }
    }

    /// From its word.
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "thin" => Some(Kind::Thin),
            "thick" => Some(Kind::Thick),
            _ => None,
        }
    }
}

/// Why a disk was not made, grown or deleted.
#[derive(Debug, thiserror::Error)]
pub enum VdiskError {
    /// The name is not one.
    #[error("not a virtual disk's name")]
    Name,
    /// The size is out of range or not whole megabytes.
    #[error("not a virtual disk's size")]
    Size,
    /// A disk of that name is there.
    #[error("the virtual disk exists")]
    Exists,
    /// No disk of that name.
    #[error("no such virtual disk")]
    Missing,
    /// A disk only grows.
    #[error("a virtual disk only grows")]
    Shrink,
    /// The pool has no room for a thick disk.
    #[error("the pool has no room")]
    NoSpace,
    /// The host's file system failed.
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// A disk as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vdisk {
    /// Its name.
    pub name: String,
    /// Its size in bytes: what the guest sees.
    pub bytes: u64,
    /// Bytes the pool has given it.
    pub allocated: u64,
    /// Thin or thick.
    pub kind: Kind,
}

/// Whether `name` is a virtual disk's name: 1 to 32 letters, digits,
/// `_` and `-`, a letter first.
pub fn valid_name(name: &str) -> bool {
    (1..=NAME_MAX).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && name.as_bytes()[0].is_ascii_alphabetic()
}

/// Whether `bytes` is a virtual disk's size.
pub fn valid_size(bytes: u64) -> bool {
    (MIN_BYTES..=MAX_BYTES).contains(&bytes) && bytes.is_multiple_of(GRANULE)
}

/// The file of disk `name` in the pool mounted at `pool`.
pub fn path(pool: &Path, name: &str) -> PathBuf {
    pool.join(DIR).join(format!("{name}{SUFFIX}"))
}

fn no_space(e: io::Error) -> VdiskError {
    if e.raw_os_error() == Some(28) {
        VdiskError::NoSpace
    } else {
        VdiskError::Io(e)
    }
}

fn allocate(file: &File, from: u64, len: u64) -> Result<(), VdiskError> {
    fallocate(file, FallocateFlags::empty(), from, len).map_err(|e| no_space(io::Error::from(e)))
}

fn kind_of(file: &File) -> Kind {
    let mut buf = [0u8; 8];
    match fgetxattr(file, KIND_ATTR, &mut buf[..]) {
        Ok(n) if &buf[..n] == b"thick" => Kind::Thick,
        _ => Kind::Thin,
    }
}

/// Make disk `name` of `bytes` in the pool at `pool`. A thick disk's
/// space is allocated now: when the pool has not that much, nothing is
/// left behind.
pub fn create(pool: &Path, name: &str, bytes: u64, kind: Kind) -> Result<Vdisk, VdiskError> {
    if !valid_name(name) {
        return Err(VdiskError::Name);
    }
    if !valid_size(bytes) {
        return Err(VdiskError::Size);
    }
    fs::create_dir_all(pool.join(DIR))?;
    let target = path(pool, name);
    // Made under a name of its own and renamed when whole.
    let partial = pool.join(DIR).join(format!(".{name}.new"));
    let _ = fs::remove_file(&partial);
    if target.exists() {
        return Err(VdiskError::Exists);
    }
    let made = (|| {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&partial)?;
        fsetxattr(
            &file,
            KIND_ATTR,
            kind.word().as_bytes(),
            XattrFlags::empty(),
        )
        .map_err(io::Error::from)?;
        match kind {
            Kind::Thin => file.set_len(bytes)?,
            Kind::Thick => allocate(&file, 0, bytes)?,
        }
        file.sync_all()?;
        Ok::<_, VdiskError>(())
    })();
    if let Err(e) = made {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    if target.exists() {
        let _ = fs::remove_file(&partial);
        return Err(VdiskError::Exists);
    }
    fs::rename(&partial, &target)?;
    sync_dir(&pool.join(DIR));
    describe(pool, name)
}

/// Grow disk `name` to `bytes`. A thick disk's new space is allocated.
pub fn resize(pool: &Path, name: &str, bytes: u64) -> Result<Vdisk, VdiskError> {
    if !valid_size(bytes) {
        return Err(VdiskError::Size);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path(pool, name))
        .map_err(|e| missing(e, name))?;
    let now = file.metadata()?.len();
    if bytes < now {
        return Err(VdiskError::Shrink);
    }
    match kind_of(&file) {
        Kind::Thin => file.set_len(bytes)?,
        Kind::Thick => allocate(&file, now, bytes - now)?,
    }
    file.sync_all()?;
    describe(pool, name)
}

fn missing(e: io::Error, _name: &str) -> VdiskError {
    if e.kind() == io::ErrorKind::NotFound {
        VdiskError::Missing
    } else {
        VdiskError::Io(e)
    }
}

/// Delete disk `name`.
pub fn delete(pool: &Path, name: &str) -> Result<(), VdiskError> {
    fs::remove_file(path(pool, name)).map_err(|e| missing(e, name))?;
    sync_dir(&pool.join(DIR));
    Ok(())
}

/// Disk `name` as listed.
pub fn describe(pool: &Path, name: &str) -> Result<Vdisk, VdiskError> {
    let file = File::open(path(pool, name)).map_err(|e| missing(e, name))?;
    let meta = file.metadata()?;
    Ok(Vdisk {
        name: name.to_string(),
        bytes: meta.len(),
        allocated: meta.blocks() * 512,
        kind: kind_of(&file),
    })
}

/// Every disk of the pool at `pool`, by name. Files that are not a
/// disk's (a disk being made) are left out.
pub fn list(pool: &Path) -> Vec<Vdisk> {
    let mut names: Vec<String> = fs::read_dir(pool.join(DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.strip_suffix(SUFFIX)
                .filter(|base| valid_name(base))
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
        .iter()
        .filter_map(|n| describe(pool, n).ok())
        .collect()
}

/// Open disk `name` for a partition: read and write, direct I/O.
pub fn open(pool: &Path, name: &str) -> Result<File, VdiskError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_DIRECT)
        .open(path(pool, name))
        .map_err(|e| missing(e, name))
}

/// Make a directory's entries durable: a rename or a removal in it.
pub fn sync_dir(dir: &Path) {
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vdisk-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_and_sizes_are_checked() {
        assert!(valid_name("PROD-LS"));
        assert!(valid_name("a1_b"));
        assert!(!valid_name("1disk"));
        assert!(!valid_name("disk.img"));
        assert!(!valid_name(&"x".repeat(33)));
        assert!(valid_size(1 << 30));
        assert!(!valid_size((1 << 30) + 1));
        assert!(!valid_size(0));
        assert!(!valid_size(MAX_BYTES + GRANULE));
    }

    #[test]
    fn a_thin_disk_is_sparse_and_grows() {
        let p = pool();
        let d = create(&p, "THIN", 64 << 20, Kind::Thin).unwrap();
        assert_eq!(d.bytes, 64 << 20);
        assert!(d.allocated < 1 << 20, "sparse: {}", d.allocated);
        // The kind is thin whether or not the file system keeps xattrs.
        assert_eq!(d.kind, Kind::Thin);
        assert!(matches!(
            create(&p, "THIN", 64 << 20, Kind::Thin),
            Err(VdiskError::Exists)
        ));
        let grown = resize(&p, "THIN", 128 << 20).unwrap();
        assert_eq!(grown.bytes, 128 << 20);
        assert!(matches!(
            resize(&p, "THIN", 1 << 20),
            Err(VdiskError::Shrink)
        ));
        assert_eq!(list(&p).len(), 1);
        delete(&p, "THIN").unwrap();
        assert!(matches!(delete(&p, "THIN"), Err(VdiskError::Missing)));
        assert!(list(&p).is_empty());
        let _ = fs::remove_dir_all(&p);
    }

    #[test]
    fn a_thick_disk_has_its_space() {
        let p = pool();
        // tmpfs and ext4 keep user xattrs; where they are refused the
        // disk is not made at all.
        if let Ok(d) = create(&p, "THICK", 8 << 20, Kind::Thick) {
            assert!(d.allocated >= 8 << 20);
            assert_eq!(d.kind, Kind::Thick);
            let grown = resize(&p, "THICK", 16 << 20).unwrap();
            assert!(grown.allocated >= 16 << 20);
        }
        assert!(matches!(
            create(&p, "bad.name", 1 << 20, Kind::Thin),
            Err(VdiskError::Name)
        ));
        assert!(matches!(
            create(&p, "X", 100, Kind::Thin),
            Err(VdiskError::Size)
        ));
        // Nothing half made is left.
        assert!(
            fs::read_dir(p.join(DIR))
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().starts_with('.'))
        );
        let _ = fs::remove_dir_all(&p);
    }
}
