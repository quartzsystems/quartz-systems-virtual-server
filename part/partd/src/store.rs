// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partitions in state: `partitions.toml` beside the host's other
//! state files, and a directory for each partition.
//!
//! ```text
//! <state>/partitions.toml                        the settings and the pools
//! <state>/partitions/<ID>/definition.toml        the definition
//! <state>/partitions/<ID>/profiles/<NAME>/profile.toml
//! ```
//!
//! Each is a state file of `base/state`: a schema version, written
//! atomically.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use model::{Definition, Profile};
use resource::Settings;

/// The directory of the partitions in the state directory.
pub const DIR: &str = "partitions";
/// The directory of a partition's profiles in its own.
const PROFILES: &str = "profiles";
/// The most profiles read of one partition.
pub const MAX_PROFILES: usize = 64;

/// One partition as it is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// Its definition.
    pub definition: Definition,
    /// Its profiles, by name.
    pub profiles: BTreeMap<String, Profile>,
}

/// The partitions' place in state.
#[derive(Clone, Debug)]
pub struct Store {
    state_dir: PathBuf,
}

impl Store {
    /// The store under the state directory `state_dir`.
    pub fn at(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
        }
    }

    fn partition_dir(&self, id: u32) -> PathBuf {
        self.state_dir.join(DIR).join(id.to_string())
    }

    fn profile_dir(&self, id: u32, name: &str) -> PathBuf {
        self.partition_dir(id).join(PROFILES).join(name)
    }

    /// The settings; the defaults when there is no file. `Err` when
    /// the file is refused.
    pub fn settings(&self) -> Result<Settings, String> {
        state::Store::at(&self.state_dir)
            .load::<Settings>()
            .map(Option::unwrap_or_default)
            .map_err(|e| e.to_string())
    }

    /// Keep the settings.
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        state::Store::at(&self.state_dir)
            .save(settings)
            .map_err(|e| e.to_string())
    }

    /// Every partition that is kept, in ID order, and what was
    /// refused: a file and the reason.
    pub fn load(&self) -> (Vec<Kept>, Vec<String>) {
        let mut kept = Vec::new();
        let mut refused = Vec::new();
        let mut ids: Vec<u32> = names(&self.state_dir.join(DIR))
            .iter()
            .filter_map(|name| name.parse().ok())
            .collect();
        ids.sort_unstable();
        for id in ids {
            let dir = self.partition_dir(id);
            let definition = match state::Store::at(&dir).load::<Definition>() {
                Ok(Some(definition)) if definition.id == id => definition,
                Ok(Some(_)) => {
                    refused.push(format!("{}: not the definition of {id}", dir.display()));
                    continue;
                }
                // A directory left by a creation that was cut off.
                Ok(None) => continue,
                Err(e) => {
                    refused.push(format!("partition {id}: {e}"));
                    continue;
                }
            };
            let mut profiles = BTreeMap::new();
            for name in names(&dir.join(PROFILES)).into_iter().take(MAX_PROFILES) {
                match state::Store::at(self.profile_dir(id, &name)).load::<Profile>() {
                    Ok(Some(profile)) if profile.name == name => {
                        profiles.insert(name, profile);
                    }
                    Ok(Some(_)) => {
                        refused.push(format!("partition {id}: profile {name} is another's"));
                    }
                    Ok(None) => {}
                    Err(e) => refused.push(format!("partition {id}, profile {name}: {e}")),
                }
            }
            kept.push(Kept {
                definition,
                profiles,
            });
        }
        (kept, refused)
    }

    /// Keep a definition.
    pub fn save_definition(&self, definition: &Definition) -> Result<(), String> {
        state::Store::at(self.partition_dir(definition.id))
            .save(definition)
            .map_err(|e| e.to_string())
    }

    /// Keep a profile of the partition `id`.
    pub fn save_profile(&self, id: u32, profile: &Profile) -> Result<(), String> {
        state::Store::at(self.profile_dir(id, &profile.name))
            .save(profile)
            .map_err(|e| e.to_string())
    }

    /// Forget a profile of the partition `id`.
    pub fn delete_profile(&self, id: u32, name: &str) -> Result<(), String> {
        remove(&self.profile_dir(id, name))
    }

    /// Forget the partition `id`: its definition first, so that what
    /// is left of a deletion that was cut off is no partition.
    pub fn delete(&self, id: u32) -> Result<(), String> {
        let dir = self.partition_dir(id);
        match fs::remove_file(state::Store::at(&dir).path::<Definition>()) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        remove(&dir)
    }
}

fn remove(dir: &Path) -> Result<(), String> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

/// The entries of a directory, sorted; none when it is not there.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::sample;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("partd-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn what_is_kept_is_read_back_in_id_order() {
        let dir = scratch("kept");
        let store = Store::at(&dir);
        assert_eq!(store.load(), (Vec::new(), Vec::new()));
        assert_eq!(store.settings().unwrap(), Settings::default());
        for id in [10, 2] {
            let mut definition = sample::definition();
            definition.id = id;
            definition.name = format!("P{id}");
            store.save_definition(&definition).unwrap();
            store.save_profile(id, &sample::dedicated()).unwrap();
        }
        store.save_profile(2, &sample::shared()).unwrap();
        let (kept, refused) = store.load();
        assert_eq!(refused, Vec::<String>::new());
        let ids: Vec<u32> = kept.iter().map(|k| k.definition.id).collect();
        assert_eq!(ids, [2, 10]);
        assert_eq!(
            kept[0].profiles.keys().collect::<Vec<_>>(),
            ["NORMAL", "SHARED"]
        );
        assert_eq!(kept[0].profiles["SHARED"], sample::shared());

        store.delete_profile(2, "SHARED").unwrap();
        store.delete(10).unwrap();
        let (kept, _) = store.load();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].profiles.len(), 1);
        // Forgetting what is not there is no error.
        store.delete(10).unwrap();
        store.delete_profile(2, "NONE").unwrap();

        let settings = Settings {
            max_partitions: 3,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.settings().unwrap(), settings);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_is_refused_is_named_and_the_rest_is_read() {
        let dir = scratch("refused");
        let store = Store::at(&dir);
        store.save_definition(&sample::definition()).unwrap();
        store.save_profile(1, &sample::dedicated()).unwrap();
        // A definition under another partition's number, a file that
        // is no TOML, a directory with nothing in it, a stray name.
        let mut other = sample::definition();
        other.id = 5;
        store.save_definition(&other).unwrap();
        fs::rename(dir.join("partitions/5"), dir.join("partitions/6")).unwrap();
        fs::create_dir_all(dir.join("partitions/7")).unwrap();
        fs::write(dir.join("partitions/7/definition.toml"), "not = [toml").unwrap();
        fs::create_dir_all(dir.join("partitions/8")).unwrap();
        fs::create_dir_all(dir.join("partitions/notanumber")).unwrap();
        fs::create_dir_all(dir.join("partitions/1/profiles/BROKEN")).unwrap();
        fs::write(
            dir.join("partitions/1/profiles/BROKEN/profile.toml"),
            "schema_version = 99\n",
        )
        .unwrap();
        let (kept, refused) = store.load();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].profiles.len(), 1);
        assert_eq!(refused.len(), 3, "{refused:?}");
        fs::write(dir.join("partitions.toml"), "max_partitions = 1\n").unwrap();
        assert!(store.settings().is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
