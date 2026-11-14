// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The media library: a directory, `media`, in the pool the operator
//! designates as the media repository. There is none until a pool is
//! designated.
//!
//! ```text
//! <pool>/media/optical/<name>.iso   an optical image
//! <pool>/media/tape/<name>.aws      a tape volume's image (stg/tape)
//! <pool>/media/tape/<name>.toml     its identifier, capacity, protection
//! ```
//!
//! An image is imported under a name of its own (`.<name>.part`),
//! checked, and renamed when whole: an import that is cut off leaves
//! that file, which the next import of the name and the service's start
//! remove. Nothing is under the image's name until it is whole.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use tape::volume::{self, Volume};

/// The library's directory in the repository pool.
pub const DIR: &str = "media";
/// Optical images.
pub const OPTICAL: &str = "optical";
/// Tape volumes.
pub const TAPE: &str = "tape";
/// An optical image's file is its name and this.
pub const ISO_SUFFIX: &str = ".iso";
/// A tape volume's image.
pub const AWS_SUFFIX: &str = ".aws";
/// A tape volume's description.
pub const TOML_SUFFIX: &str = ".toml";
/// What an import in progress is called.
const PART_SUFFIX: &str = ".part";
/// The most bytes of one image: a dual-layer Blu-ray's worth.
pub const MAX_IMAGE_BYTES: u64 = 64 << 30;
/// An ISO 9660 primary volume descriptor's place.
const PVD_AT: usize = 16 * 2048;

/// Why the library did not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// The name is not one.
    #[error("not a name of the media library")]
    Name,
    /// Something of that name is there.
    #[error("it exists")]
    Exists,
    /// Nothing of that name.
    #[error("no such medium")]
    Missing,
    /// The image is not an ISO 9660 volume.
    #[error("not an ISO 9660 image")]
    NotIso,
    /// The image is larger than the library takes, or not what was said.
    #[error("the image's size")]
    Size,
    /// A tape volume's description is not one.
    #[error("the volume: {0}")]
    Volume(String),
    /// The host's file system failed.
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// An optical image as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// Its name.
    pub name: String,
    /// Its size.
    pub bytes: u64,
    /// Its volume identifier.
    pub label: String,
}

/// A tape volume as listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeVolume {
    /// Its name in the library.
    pub name: String,
    /// What is kept beside it.
    pub volume: Volume,
    /// Bytes of its image in use.
    pub used: u64,
}

/// Whether `name` is a name in the library: as a virtual disk's.
pub fn valid_name(name: &str) -> bool {
    vdisk::valid_name(name)
}

/// The library in the repository pool mounted at `pool`.
#[derive(Clone, Debug)]
pub struct Library {
    root: PathBuf,
}

/// The volume identifier of an ISO 9660 image's first 34 KiB; `None`
/// when they are not one.
pub fn iso_label(head: &[u8]) -> Option<String> {
    let d = head.get(PVD_AT..PVD_AT + 2048)?;
    if d[0] != 1 || &d[1..6] != b"CD001" || d[6] != 1 {
        return None;
    }
    if u16::from_le_bytes([d[128], d[129]]) != 2048 {
        return None;
    }
    let id: String = d[40..72].iter().map(|b| char::from(*b)).collect();
    Some(id.trim_end().to_string())
}

impl Library {
    /// The library of the pool mounted at `pool`.
    pub fn new(pool: &Path) -> Self {
        Self {
            root: pool.join(DIR),
        }
    }

    fn dir(&self, kind: &str) -> PathBuf {
        self.root.join(kind)
    }

    /// Make the library's directories, and remove what an import cut
    /// off left behind.
    pub fn prepare(&self) -> io::Result<()> {
        for kind in [OPTICAL, TAPE] {
            fs::create_dir_all(self.dir(kind))?;
            for e in fs::read_dir(self.dir(kind))?.flatten() {
                if e.file_name().to_string_lossy().ends_with(PART_SUFFIX) {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        Ok(())
    }

    /// The file of image `name`.
    pub fn image_path(&self, name: &str) -> PathBuf {
        self.dir(OPTICAL).join(format!("{name}{ISO_SUFFIX}"))
    }

    fn names(&self, kind: &str, suffix: &str) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(self.dir(kind))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.strip_suffix(suffix)
                    .filter(|b| valid_name(b))
                    .map(str::to_string)
            })
            .collect();
        out.sort();
        out
    }

    /// Every optical image.
    pub fn images(&self) -> Vec<Image> {
        self.names(OPTICAL, ISO_SUFFIX)
            .into_iter()
            .filter_map(|n| self.image(&n).ok())
            .collect()
    }

    /// Image `name`.
    pub fn image(&self, name: &str) -> Result<Image, MediaError> {
        let mut file = File::open(self.image_path(name)).map_err(missing)?;
        let bytes = file.metadata()?.len();
        let mut head = vec![0u8; PVD_AT + 2048];
        let n = read_up_to(&mut file, &mut head)?;
        Ok(Image {
            name: name.to_string(),
            bytes,
            label: iso_label(&head[..n]).unwrap_or_default(),
        })
    }

    /// Begin importing image `name` of `bytes`: the bytes are written
    /// to what is returned, which checks and renames them when whole.
    pub fn import(&self, name: &str, bytes: u64) -> Result<Import, MediaError> {
        if !valid_name(name) {
            return Err(MediaError::Name);
        }
        if bytes == 0 || bytes > MAX_IMAGE_BYTES || !bytes.is_multiple_of(2048) {
            return Err(MediaError::Size);
        }
        fs::create_dir_all(self.dir(OPTICAL))?;
        let target = self.image_path(name);
        if target.exists() {
            return Err(MediaError::Exists);
        }
        let part = self.dir(OPTICAL).join(format!(".{name}{PART_SUFFIX}"));
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&part)?;
        Ok(Import {
            file: Some(file),
            part,
            target,
            expected: bytes,
            written: 0,
            head: Vec::with_capacity(PVD_AT + 2048),
        })
    }

    /// Rename image `from` to `to`.
    pub fn rename_image(&self, from: &str, to: &str) -> Result<(), MediaError> {
        if !valid_name(to) {
            return Err(MediaError::Name);
        }
        let (old, new) = (self.image_path(from), self.image_path(to));
        if !old.exists() {
            return Err(MediaError::Missing);
        }
        if new.exists() {
            return Err(MediaError::Exists);
        }
        fs::rename(old, new)?;
        vdisk::sync_dir(&self.dir(OPTICAL));
        Ok(())
    }

    /// Delete image `name`. Whether it is in use is the caller's to say.
    pub fn delete_image(&self, name: &str) -> Result<(), MediaError> {
        fs::remove_file(self.image_path(name)).map_err(missing)?;
        vdisk::sync_dir(&self.dir(OPTICAL));
        Ok(())
    }

    /// Open image `name` to read.
    pub fn open_image(&self, name: &str) -> Result<File, MediaError> {
        File::open(self.image_path(name)).map_err(missing)
    }

    fn volume_paths(&self, name: &str) -> (PathBuf, PathBuf) {
        let d = self.dir(TAPE);
        (
            d.join(format!("{name}{AWS_SUFFIX}")),
            d.join(format!("{name}{TOML_SUFFIX}")),
        )
    }

    /// Make tape volume `name`: an empty image and its description.
    pub fn create_volume(&self, name: &str, volume: &Volume) -> Result<(), MediaError> {
        if !valid_name(name) {
            return Err(MediaError::Name);
        }
        let problems = volume.problems();
        if !problems.is_empty() {
            return Err(MediaError::Volume(problems.join(", ")));
        }
        fs::create_dir_all(self.dir(TAPE))?;
        let (image, desc) = self.volume_paths(name);
        if image.exists() || desc.exists() {
            return Err(MediaError::Exists);
        }
        self.write_volume(name, volume)?;
        File::create(&image)?.sync_all()?;
        vdisk::sync_dir(&self.dir(TAPE));
        Ok(())
    }

    fn write_volume(&self, name: &str, volume: &Volume) -> Result<(), MediaError> {
        let (_, desc) = self.volume_paths(name);
        let text = toml::to_string(volume).map_err(|e| MediaError::Volume(e.to_string()))?;
        let part = self
            .dir(TAPE)
            .join(format!(".{name}{TOML_SUFFIX}{PART_SUFFIX}"));
        let mut f = File::create(&part)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        fs::rename(part, desc)?;
        Ok(())
    }

    /// Tape volume `name`.
    pub fn volume(&self, name: &str) -> Result<TapeVolume, MediaError> {
        let (image, desc) = self.volume_paths(name);
        let text = fs::read_to_string(desc).map_err(missing)?;
        let volume: Volume =
            toml::from_str(&text).map_err(|e| MediaError::Volume(e.to_string()))?;
        let used = fs::metadata(image).map_err(missing)?.len();
        Ok(TapeVolume {
            name: name.to_string(),
            volume,
            used,
        })
    }

    /// Every tape volume.
    pub fn volumes(&self) -> Vec<TapeVolume> {
        self.names(TAPE, TOML_SUFFIX)
            .into_iter()
            .filter_map(|n| self.volume(&n).ok())
            .collect()
    }

    /// Set or clear volume `name`'s write protection.
    pub fn protect_volume(&self, name: &str, on: bool) -> Result<(), MediaError> {
        let mut v = self.volume(name)?;
        v.volume.write_protected = on;
        self.write_volume(name, &v.volume)
    }

    /// Delete tape volume `name`.
    pub fn delete_volume(&self, name: &str) -> Result<(), MediaError> {
        let (image, desc) = self.volume_paths(name);
        if !desc.exists() {
            return Err(MediaError::Missing);
        }
        let _ = fs::remove_file(image);
        fs::remove_file(desc)?;
        vdisk::sync_dir(&self.dir(TAPE));
        Ok(())
    }

    /// Open tape volume `name`'s image to read and write.
    pub fn open_volume(&self, name: &str) -> Result<(File, Volume), MediaError> {
        let v = self.volume(name)?;
        let (image, _) = self.volume_paths(name);
        let file = OpenOptions::new()
            .read(true)
            .write(!v.volume.write_protected)
            .open(image)
            .map_err(missing)?;
        Ok((file, v.volume))
    }
}

fn missing(e: io::Error) -> MediaError {
    if e.kind() == io::ErrorKind::NotFound {
        MediaError::Missing
    } else {
        MediaError::Io(e)
    }
}

fn read_up_to(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut got = 0;
    while got < buf.len() {
        match file.read(&mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

/// An image being imported.
pub struct Import {
    file: Option<File>,
    part: PathBuf,
    target: PathBuf,
    expected: u64,
    written: u64,
    /// The first bytes, to check it is an image.
    head: Vec<u8>,
}

impl Import {
    /// Write the next bytes.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), MediaError> {
        if self.written + bytes.len() as u64 > self.expected {
            return Err(MediaError::Size);
        }
        let room = (PVD_AT + 2048).saturating_sub(self.head.len());
        self.head.extend_from_slice(&bytes[..room.min(bytes.len())]);
        let file = self.file.as_mut().ok_or(MediaError::Size)?;
        file.write_all(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Check what was written and give it its name.
    pub fn finish(mut self) -> Result<Image, MediaError> {
        if self.written != self.expected {
            return Err(MediaError::Size);
        }
        let label = iso_label(&self.head).ok_or(MediaError::NotIso)?;
        let file = self.file.take().ok_or(MediaError::Size)?;
        file.sync_all()?;
        drop(file);
        if self.target.exists() {
            return Err(MediaError::Exists);
        }
        fs::rename(&self.part, &self.target)?;
        if let Some(dir) = self.target.parent() {
            vdisk::sync_dir(dir);
        }
        let name = self
            .target
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Image {
            name,
            bytes: self.expected,
            label,
        })
    }
}

impl Drop for Import {
    /// An import not finished leaves nothing.
    fn drop(&mut self) {
        if self.part.exists() {
            let _ = fs::remove_file(&self.part);
        }
    }
}

/// A tape volume's description, made now.
pub fn new_volume(identifier: &str, capacity_mib: u64, created: u64) -> Volume {
    Volume {
        schema_version: 1,
        identifier: identifier.to_string(),
        capacity_mib,
        write_protected: false,
        created,
    }
}

/// Whether `id` is a tape volume identifier.
pub fn valid_identifier(id: &str) -> bool {
    volume::valid_identifier(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> (Library, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "media-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let l = Library::new(&dir);
        l.prepare().unwrap();
        (l, dir)
    }

    fn iso(label: &str, blocks: usize) -> Vec<u8> {
        let mut img = vec![0u8; blocks * 2048];
        let d = &mut img[PVD_AT..PVD_AT + 2048];
        d[0] = 1;
        d[1..6].copy_from_slice(b"CD001");
        d[6] = 1;
        d[40..72].fill(b' ');
        d[40..40 + label.len()].copy_from_slice(label.as_bytes());
        d[128..130].copy_from_slice(&2048u16.to_le_bytes());
        img
    }

    #[test]
    fn an_image_is_imported_in_pieces_and_named_when_whole() {
        let (l, dir) = library();
        let img = iso("QS400_V1R1M0", 40);
        let mut import = l.import("QS400", img.len() as u64).unwrap();
        for chunk in img.chunks(7000) {
            import.write(chunk).unwrap();
        }
        // Not there until whole.
        assert!(l.images().is_empty());
        let made = import.finish().unwrap();
        assert_eq!(made.label, "QS400_V1R1M0");
        assert_eq!(l.images(), std::slice::from_ref(&made));
        assert!(matches!(l.import("QS400", 2048), Err(MediaError::Exists)));
        l.rename_image("QS400", "QS400V1R1").unwrap();
        assert_eq!(l.images()[0].name, "QS400V1R1");
        l.delete_image("QS400V1R1").unwrap();
        assert!(matches!(
            l.delete_image("QS400V1R1"),
            Err(MediaError::Missing)
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_cut_off_or_wrong_import_leaves_nothing() {
        let (l, dir) = library();
        let img = iso("X", 40);
        {
            let mut import = l.import("CUT", img.len() as u64).unwrap();
            import.write(&img[..10_000]).unwrap();
            // Dropped: the connection went away.
        }
        let not_iso = vec![0u8; 40 * 2048];
        let mut import = l.import("JUNK", not_iso.len() as u64).unwrap();
        import.write(&not_iso).unwrap();
        assert!(matches!(import.finish(), Err(MediaError::NotIso)));
        let mut short = l.import("SHORT", img.len() as u64).unwrap();
        short.write(&img[..2048]).unwrap();
        assert!(matches!(short.finish(), Err(MediaError::Size)));
        let mut long = l.import("LONG", 2048).unwrap();
        assert!(matches!(long.write(&img[..4096]), Err(MediaError::Size)));
        drop(long);
        let left: Vec<_> = fs::read_dir(dir.join(DIR).join(OPTICAL))
            .unwrap()
            .flatten()
            .collect();
        assert!(left.is_empty(), "{left:?}");
        assert!(matches!(l.import("bad name", 2048), Err(MediaError::Name)));
        assert!(matches!(l.import("ODD", 1000), Err(MediaError::Size)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn tape_volumes_are_made_protected_and_deleted() {
        let (l, dir) = library();
        l.create_volume("SAVE1", &new_volume("SAV001", 1024, 5))
            .unwrap();
        assert!(matches!(
            l.create_volume("SAVE1", &new_volume("SAV001", 1024, 5)),
            Err(MediaError::Exists)
        ));
        assert!(matches!(
            l.create_volume("BAD", &new_volume("sav", 1, 5)),
            Err(MediaError::Volume(_))
        ));
        let v = l.volume("SAVE1").unwrap();
        assert_eq!(v.volume.identifier, "SAV001");
        assert_eq!(v.used, 0);
        l.protect_volume("SAVE1", true).unwrap();
        let (_, desc) = l.open_volume("SAVE1").unwrap();
        assert!(desc.write_protected);
        assert_eq!(l.volumes().len(), 1);
        l.delete_volume("SAVE1").unwrap();
        assert!(l.volumes().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
