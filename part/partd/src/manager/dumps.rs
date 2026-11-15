// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Dumps of a partition, taken by the host from outside
//! (`docs/DUMPS.md`). The machine writes one in the background; when it
//! is whole, the partition's oldest dumps go until the rest are within
//! their number and size, and function 22's IPL restarts.

use super::{DumpOrder, Manager, System};
use crate::proto::{Event, Refused};

/// The most characters of a dump's name.
const NAME_MAX: usize = 64;

/// Whether `name` is a dump's name as the host makes them: letters,
/// digits, `_` and `-`, then `.core`.
pub fn valid_name(name: &str) -> bool {
    name.len() <= NAME_MAX
        && name.strip_suffix(dump::SUFFIX).is_some_and(|stem| {
            !stem.is_empty()
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

impl<S: System> Manager<S> {
    /// Begin a dump of `id` for `reason`; with `restart` its IPL restarts
    /// once it is written. The dump's name.
    pub(crate) fn begin_dump(
        &mut self,
        id: u32,
        reason: &str,
        restart: bool,
    ) -> Result<String, Refused> {
        let now = self.system.now();
        let p = self.partition(id)?;
        let name = p.definition.name.clone();
        if p.run.is_none() {
            return Err(vec![
                qvsmsg::PARTITION_NOT_ACTIVE.with(&[&name, p.state.word()]),
            ]);
        }
        if p.dumping.is_some() {
            return Err(vec![qvsmsg::DUMP_BUSY.with(&[&name])]);
        }
        let current = p.current.clone().unwrap_or_default();
        if self.system.dump_dir(id).is_none() {
            return Err(vec![qvsmsg::DUMPS_NO_REPOSITORY.with(&[&name])]);
        }
        let taken = self.system.dumps(id);
        let mut file = format!("{name}-{now}{}", dump::SUFFIX);
        let mut n = 2;
        while taken.iter().any(|d| d.name == file) {
            file = format!("{name}-{now}-{n}{}", dump::SUFFIX);
            n += 1;
        }
        let order = DumpOrder {
            name: file.clone(),
            code: if current.code.is_empty() {
                "00000000".into()
            } else {
                current.code
            },
            text: current.text,
            time: now,
            reason: reason.to_string(),
        };
        self.system
            .begin_dump(id, &order)
            .map_err(|e| vec![qvsmsg::DUMP_NOT_WRITTEN.with(&[&name, &e])])?;
        if let Some(p) = self.partitions.get_mut(&id) {
            p.dumping = Some((file.clone(), restart));
        }
        Ok(file)
    }

    /// The dump `dump` of `id` was written, or not and why.
    pub fn dumped(&mut self, id: u32, dump: &str, result: Result<(), String>) {
        let time = self.system.now();
        let name = self.name_of(id);
        let restart = self
            .partitions
            .get_mut(&id)
            .and_then(|p| p.dumping.take())
            .is_some_and(|(_, restart)| restart);
        let error = match result {
            Ok(()) => {
                self.trim_dumps(id, dump);
                None
            }
            Err(why) => {
                let message = qvsmsg::DUMP_NOT_WRITTEN.with(&[&name, &why]);
                self.system.log(&message.to_string());
                Some(message)
            }
        };
        let written = error.is_none();
        self.publish(&Event::Dump {
            id,
            name,
            dump: dump.to_string(),
            error,
            time,
        });
        if restart && written {
            let _ = self.end_one(id, true);
        }
    }

    /// Remove the oldest dumps of `id` but `kept` until the rest are
    /// within the limits.
    fn trim_dumps(&mut self, id: u32, kept: &str) {
        let service = self.host.settings.service;
        let all = self.system.dumps(id);
        let adding = all
            .iter()
            .find(|d| d.name == kept)
            .map_or(0, |d| d.allocated);
        let others: Vec<dump::limits::Kept> = all
            .iter()
            .filter(|d| d.name != kept)
            .map(|d| dump::limits::Kept {
                name: d.name.clone(),
                time: d.time,
                bytes: d.allocated,
            })
            .collect();
        let gone = dump::limits::to_remove(
            &others,
            adding,
            service.max_dumps as usize,
            service.max_dump_mib.saturating_mul(1 << 20),
        );
        let name = self.name_of(id);
        for old in gone {
            if self.system.remove_dump(id, &old) {
                let code = &refcode::DUMP_REMOVED;
                self.code(
                    id,
                    code.code,
                    &code.text_with(&format!("{name}, {old}")),
                    true,
                );
            }
        }
    }
}
