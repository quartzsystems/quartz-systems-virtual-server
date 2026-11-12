// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The store: one directory of TOML documents.

mod atomic;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

pub use atomic::atomic_write;

/// The key every file carries.
pub const SCHEMA_KEY: &str = "schema_version";

/// One step of a migration: the table of version `n` made into the
/// table of version `n + 1`.
pub type Migration = fn(&mut Table) -> Result<(), String>;

/// A state file's type.
pub trait Document: Serialize + DeserializeOwned {
    /// The file's name in the state directory.
    const FILE: &'static str;
    /// The schema version this code writes.
    const VERSION: u32;
    /// The registered steps: `MIGRATIONS[n - 1]` takes version `n` to
    /// `n + 1`, so there are `VERSION - 1` of them.
    const MIGRATIONS: &'static [Migration];

    /// Whether the values make sense together.
    fn validate(&self) -> Result<(), String>;
}

/// Why a document was not read or written.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The file system.
    #[error("{file}: {source}")]
    Io {
        /// The file.
        file: String,
        /// The error.
        #[source]
        source: io::Error,
    },
    /// The file was written by newer code.
    #[error("{file}: schema version {found} is newer than {known}, the newest this release knows")]
    Newer {
        /// The file.
        file: String,
        /// The version in the file.
        found: u32,
        /// The version this code writes.
        known: u32,
    },
    /// The file is not a document of its type.
    #[error("{file}: {reason}")]
    Invalid {
        /// The file.
        file: String,
        /// What is wrong.
        reason: String,
    },
}

fn invalid<D: Document>(reason: impl ToString) -> StateError {
    StateError::Invalid {
        file: D::FILE.to_string(),
        reason: reason.to_string(),
    }
}

/// A document from its text: the version checked, older versions
/// migrated, the values validated.
pub fn parse<D: Document>(text: &str) -> Result<D, StateError> {
    let mut table: Table = text.parse().map_err(invalid::<D>)?;
    let found = match table.remove(SCHEMA_KEY) {
        Some(Value::Integer(v)) => u32::try_from(v).ok().filter(|v| *v >= 1),
        _ => None,
    }
    .ok_or_else(|| invalid::<D>(format!("no {SCHEMA_KEY}")))?;
    if found > D::VERSION {
        return Err(StateError::Newer {
            file: D::FILE.to_string(),
            found,
            known: D::VERSION,
        });
    }
    for version in found..D::VERSION {
        let step = D::MIGRATIONS
            .get(version as usize - 1)
            .ok_or_else(|| invalid::<D>(format!("no migration from version {version}")))?;
        step(&mut table).map_err(|e| invalid::<D>(format!("migrating from {version}: {e}")))?;
    }
    let doc: D = table.try_into().map_err(invalid::<D>)?;
    doc.validate().map_err(invalid::<D>)?;
    Ok(doc)
}

/// A document's text: `schema_version`, then its values.
pub fn render<D: Document>(doc: &D) -> Result<String, StateError> {
    doc.validate().map_err(invalid::<D>)?;
    let mut table = Table::try_from(doc).map_err(invalid::<D>)?;
    table.insert(SCHEMA_KEY.into(), Value::Integer(i64::from(D::VERSION)));
    toml::to_string(&table).map_err(invalid::<D>)
}

/// A state directory.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

impl Store {
    /// The store in `dir`. Nothing is created until something is saved.
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// The directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where `D` is kept.
    pub fn path<D: Document>(&self) -> PathBuf {
        self.dir.join(D::FILE)
    }

    /// Whether `D`'s file is there.
    pub fn has<D: Document>(&self) -> bool {
        self.path::<D>().is_file()
    }

    /// Read `D`; `None` when there is no file.
    pub fn load<D: Document>(&self) -> Result<Option<D>, StateError> {
        match fs::read_to_string(self.path::<D>()) {
            Ok(text) => parse(&text).map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StateError::Io {
                file: D::FILE.to_string(),
                source,
            }),
        }
    }

    /// Write `D` atomically.
    pub fn save<D: Document>(&self, doc: &D) -> Result<(), StateError> {
        let text = render(doc)?;
        let io = |source| StateError::Io {
            file: D::FILE.to_string(),
            source,
        };
        fs::create_dir_all(&self.dir).map_err(io)?;
        atomic_write(&self.path::<D>(), text.as_bytes()).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// Version 1 had `name`; version 2 calls it `title`; version 3 adds
    /// `count`.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Note {
        title: String,
        count: u32,
    }

    impl Document for Note {
        const FILE: &'static str = "note.toml";
        const VERSION: u32 = 3;
        const MIGRATIONS: &'static [Migration] = &[
            |t| {
                let name = t.remove("name").ok_or("no name")?;
                t.insert("title".into(), name);
                Ok(())
            },
            |t| {
                t.insert("count".into(), Value::Integer(1));
                Ok(())
            },
        ];

        fn validate(&self) -> Result<(), String> {
            if self.title.is_empty() {
                return Err("the title is empty".into());
            }
            Ok(())
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_document_round_trips_with_its_version() {
        let store = Store::at(scratch("round"));
        assert_eq!(store.load::<Note>().unwrap(), None);
        assert!(!store.has::<Note>());
        let note = Note {
            title: "a".into(),
            count: 7,
        };
        store.save(&note).unwrap();
        let text = fs::read_to_string(store.path::<Note>()).unwrap();
        assert!(text.contains("schema_version = 3"), "{text}");
        assert_eq!(store.load::<Note>().unwrap(), Some(note));
        // Nothing but the document is left in the directory.
        let names: Vec<_> = fs::read_dir(store.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["note.toml"]);
        fs::remove_dir_all(store.dir()).unwrap();
    }

    #[test]
    fn older_versions_migrate_through_every_step() {
        let v1: Note = parse("schema_version = 1\nname = \"old\"\n").unwrap();
        assert_eq!(
            v1,
            Note {
                title: "old".into(),
                count: 1
            }
        );
        let v2: Note = parse("schema_version = 2\ntitle = \"mid\"\n").unwrap();
        assert_eq!(v2.count, 1);
        // A step that cannot run says which.
        let e = parse::<Note>("schema_version = 1\n").unwrap_err();
        assert!(e.to_string().contains("migrating from 1"), "{e}");
    }

    #[test]
    fn a_newer_version_is_refused() {
        let e = parse::<Note>("schema_version = 4\ntitle = \"x\"\ncount = 1\n").unwrap_err();
        assert!(matches!(
            e,
            StateError::Newer {
                found: 4,
                known: 3,
                ..
            }
        ));
    }

    #[test]
    fn junk_is_refused() {
        for text in [
            "",
            "title = \"x\"\ncount = 1\n",
            "schema_version = 0\ntitle = \"x\"\ncount = 1\n",
            "schema_version = \"3\"\ntitle = \"x\"\ncount = 1\n",
            "schema_version = 3\ntitle = \"x\"\n",
            "schema_version = 3\ntitle = \"x\"\ncount = 1\nextra = true\n",
            "schema_version = 3\ntitle = \"\"\ncount = 1\n",
            "schema_version = 3\ntitle = \"x\"\ncou",
        ] {
            assert!(
                matches!(parse::<Note>(text), Err(StateError::Invalid { .. })),
                "{text:?}"
            );
        }
        // An invalid document is not written either.
        let store = Store::at(scratch("junk"));
        let bad = Note {
            title: String::new(),
            count: 0,
        };
        assert!(store.save(&bad).is_err());
        assert!(!store.has::<Note>());
    }
}
