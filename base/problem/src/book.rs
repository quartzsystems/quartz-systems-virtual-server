// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The problem log. A posted code that opens a problem (the
//! catalogue's table) opens one for its condition, or counts the one
//! open; a code that clears a condition marks its problems cleared; a
//! person closes one. The log and the place in the history it has read
//! to are kept together, in one file written whole, so a service
//! started again neither loses a code nor takes one twice.

use std::path::Path;

use msg::Issued;
use serde::{Deserialize, Serialize};
use supervise::control::{About, about as kind};

use crate::feed::Cursor;
use crate::record::{
    Filter, Line, Note, Notice, Noticed, Page, Posted, Problem, Rated, RatedPage, RatedProblem,
    Source, Status, Summary, said,
};
use crate::table::{Condition, Opening, Rules};

/// The file, in the problem service's directory.
pub const FILE: &str = "problems.json";
/// The most problems kept: past it the oldest closed go first, and only
/// then the oldest open.
pub const MAX_PROBLEMS: usize = 500;
/// The most lines an analysis log keeps: the first and the newest.
pub const MAX_LINES: usize = 50;
/// The longest a person's name for a close may be.
pub const MAX_BY: usize = 64;
/// The longest note a person may add, in bytes.
pub const MAX_NOTE: usize = 256;

/// A posting the history gave: a code, with what it is about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Posting {
    /// The code as posted, with its place.
    pub posted: Posted,
    /// What it is about, as whoever posted said.
    pub about: Vec<About>,
}

/// A problem with the identity of its condition.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct Kept {
    /// The condition: the code that opened it, whose it is and what it
    /// is about.
    key: String,
    /// The code that opened it, which the table names.
    opened_by: String,
    problem: Problem,
}

/// The log.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Book {
    /// The next problem's number.
    next: u64,
    /// Where the history was read to.
    cursor: Cursor,
    /// Oldest first.
    problems: Vec<Kept>,
}

impl Default for Book {
    fn default() -> Self {
        Self {
            next: 1,
            cursor: Cursor::default(),
            problems: Vec::new(),
        }
    }
}

/// The partition a posting is about, as its source.
fn source_of(about: &[About]) -> Source {
    about
        .iter()
        .find(|a| a.kind == kind::PARTITION && a.id != 0)
        .map_or_else(Source::host, |a| Source {
            partition: a.id,
            name: a.name.clone(),
        })
}

/// The resources a problem lists: what the posting is about but the
/// system's own code.
fn resources_of(about: &[About]) -> Vec<About> {
    about
        .iter()
        .filter(|a| a.kind != kind::SYSTEM_CODE)
        .cloned()
        .collect()
}

fn key_of(code: &str, source: &Source, resources: &[About]) -> String {
    let mut parts: Vec<String> = resources
        .iter()
        .map(|a| format!("{}:{}:{}:{}", a.kind, a.name, a.location, a.serial))
        .collect();
    parts.sort();
    format!("{code}|{}|{}", source.partition, parts.join("|"))
}

/// The values a condition's message is issued with: the name of each
/// kind it names, in order; the text as posted last.
fn values_of(c: &Opening, about: &[About], text: &str) -> Vec<String> {
    let mut values: Vec<String> = c
        .resources
        .iter()
        .map(|k| {
            about
                .iter()
                .find(|a| a.kind == *k)
                .map_or_else(String::new, |a| a.name.clone())
        })
        .collect();
    values.push(text.to_string());
    values
}

fn issue(m: &msg::Message, values: &[String]) -> Issued {
    let refs: Vec<&str> = values.iter().map(String::as_str).collect();
    m.with(&refs)
}

impl Book {
    /// The log in `dir`; an empty one when it has none. A file that
    /// does not read is kept aside as `problems.json.bad` and a new log
    /// begun, numbering after the last number it can find.
    #[must_use]
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        match std::fs::read(&path) {
            Err(_) => Self::default(),
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                let _ = std::fs::rename(&path, dir.join(format!("{FILE}.bad")));
                Self::default()
            }),
        }
    }

    /// Write it whole, atomically.
    ///
    /// # Errors
    /// The file could not be written.
    pub fn keep(&self, dir: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(self).map_err(std::io::Error::other)?;
        std::fs::create_dir_all(dir)?;
        state::store::atomic_write(&dir.join(FILE), &bytes)
    }

    /// Where the history was read to.
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// The history was read to `cursor`.
    pub fn read_to(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }

    /// Take a posting: what it opened, counted and cleared.
    pub fn take(&mut self, rules: &Rules, p: &Posting, now: u64) -> Vec<Noticed> {
        let mut out = Vec::new();
        if let Some(Condition {
            opens: Some(opening),
            ..
        }) = rules.condition(&p.posted.code)
        {
            out.push(self.seen(&p.posted.code, opening, p, now));
        }
        for c in rules.cleared(&p.posted.code) {
            out.extend(self.clear(c.code, p, now));
        }
        self.bound();
        out
    }

    fn seen(&mut self, code: &str, c: &Opening, p: &Posting, now: u64) -> Noticed {
        let source = source_of(&p.about);
        let resources = resources_of(&p.about);
        let key = key_of(code, &source, &resources);
        let mut refcode = p.posted.clone();
        if c.system_code
            && let Some(sys) = p.about.iter().find(|a| a.kind == kind::SYSTEM_CODE)
        {
            // The system's own code is the problem's; its words were
            // the system's to give, and the host's posting has none.
            refcode.code.clone_from(&sys.name);
            refcode.words.clear();
        }
        let line = format!("{} {}", p.posted.code, p.posted.text);
        if let Some(k) = self
            .problems
            .iter_mut()
            .find(|k| k.key == key && k.problem.summary.status == Status::Open)
        {
            let s = &mut k.problem.summary;
            s.count = s.count.saturating_add(1);
            s.last_seen = now;
            s.holds = true;
            s.refcode = refcode;
            push_line(&mut k.problem.log, now, format!("Seen again: {line}"));
            return Noticed {
                notice: Notice::Seen,
                problem: s.clone(),
                time: now,
                note: None,
            };
        }
        let issued = issue(c.message, &values_of(c, &p.about, &p.posted.text));
        let mut log = vec![Line {
            time: now,
            text: format!("Opened: {line}"),
        }];
        for r in &resources {
            push_line(&mut log, now, format!("About {}", said(r)));
        }
        let summary = Summary {
            number: self.next,
            status: Status::Open,
            first_seen: now,
            last_seen: now,
            count: 1,
            refcode,
            source,
            description: issued.text,
            msgid: issued.id,
            holds: true,
            closed_by: String::new(),
            closed_at: 0,
        };
        self.next += 1;
        self.problems.push(Kept {
            key,
            opened_by: code.to_string(),
            problem: Problem {
                summary: summary.clone(),
                resources,
                log,
            },
        });
        Noticed {
            notice: Notice::Opened,
            problem: summary,
            time: now,
            note: None,
        }
    }

    /// Mark cleared each open problem of the condition `code` that
    /// still holds and that the posting `p` names: its source, and
    /// each resource it says of a kind the problem lists.
    fn clear(&mut self, code: &str, p: &Posting, now: u64) -> Vec<Noticed> {
        let source = source_of(&p.about);
        let named = resources_of(&p.about);
        let mut out = Vec::new();
        for k in &mut self.problems {
            let s = &k.problem.summary;
            if k.opened_by != code || s.status != Status::Open || !s.holds {
                continue;
            }
            if source.partition != 0 && source.partition != s.source.partition {
                continue;
            }
            let names_it = named
                .iter()
                .filter(|a| a.kind != kind::PARTITION)
                .filter(|a| k.problem.resources.iter().any(|r| r.kind == a.kind))
                .all(|a| {
                    k.problem
                        .resources
                        .iter()
                        .any(|r| r.kind == a.kind && r.name == a.name)
                });
            if !names_it {
                continue;
            }
            let s = &mut k.problem.summary;
            s.holds = false;
            let text = format!("Cleared by {} {}", p.posted.code, p.posted.text);
            push_line(&mut k.problem.log, now, text);
            out.push(Noticed {
                notice: Notice::Cleared,
                problem: s.clone(),
                time: now,
                note: None,
            });
        }
        out
    }

    /// Close problem `number` for `by`; one whose condition holds only
    /// when `anyway`.
    ///
    /// # Errors
    /// No such problem, closed already, or its condition holds.
    pub fn close(
        &mut self,
        rules: &Rules,
        number: u64,
        anyway: bool,
        by: &str,
        now: u64,
    ) -> Result<Noticed, Vec<Issued>> {
        let n = number.to_string();
        let Some(k) = self
            .problems
            .iter_mut()
            .find(|k| k.problem.summary.number == number)
        else {
            return Err(vec![rules.messages.no_problem.with(&[&n])]);
        };
        let s = &mut k.problem.summary;
        if s.status == Status::Closed {
            return Err(vec![rules.messages.closed.with(&[&n, &s.closed_by])]);
        }
        if s.holds && !anyway {
            return Err(vec![rules.messages.holds.with(&[&n])]);
        }
        let by: String = by.chars().take(MAX_BY).collect();
        let by = if by.trim().is_empty() {
            "unknown".to_string()
        } else {
            by
        };
        s.status = Status::Closed;
        s.closed_by.clone_from(&by);
        s.closed_at = now;
        let text = if anyway && s.holds {
            format!("Closed by {by} while its condition held")
        } else {
            format!("Closed by {by}")
        };
        push_line(&mut k.problem.log, now, text);
        Ok(Noticed {
            notice: Notice::Closed,
            problem: s.clone(),
            time: now,
            note: None,
        })
    }

    /// Add `by`'s note to problem `number`'s analysis log.
    ///
    /// # Errors
    /// No such problem, closed, or a note that is not one.
    pub fn note(
        &mut self,
        rules: &Rules,
        number: u64,
        text: &str,
        by: &str,
        now: u64,
    ) -> Result<Noticed, Vec<Issued>> {
        let n = number.to_string();
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_NOTE || text.chars().any(char::is_control) {
            return Err(vec![
                rules
                    .messages
                    .note_invalid
                    .with(&[&text.len().to_string(), &MAX_NOTE.to_string()]),
            ]);
        }
        let Some(k) = self
            .problems
            .iter_mut()
            .find(|k| k.problem.summary.number == number)
        else {
            return Err(vec![rules.messages.no_problem.with(&[&n])]);
        };
        let s = &k.problem.summary;
        if s.status == Status::Closed {
            return Err(vec![rules.messages.closed.with(&[&n, &s.closed_by])]);
        }
        let by: String = by.chars().take(MAX_BY).collect();
        let by = if by.trim().is_empty() {
            "unknown".to_string()
        } else {
            by
        };
        let line = Line {
            time: now,
            text: format!("Note by {by}: {text}"),
        };
        push_line(&mut k.problem.log, now, line.text.clone());
        Ok(Noticed {
            notice: Notice::Noted,
            problem: k.problem.summary.clone(),
            time: now,
            note: Some(Note { line, by }),
        })
    }

    /// The severity of problem `number`, from the condition that opened
    /// it.
    fn severity_of(&self, rules: &Rules, number: u64) -> crate::record::Severity {
        self.problems
            .iter()
            .find(|k| k.problem.summary.number == number)
            .map(|k| (rules.severity)(&k.opened_by))
            .unwrap_or_default()
    }

    /// [`Book::list`] with each one's severity.
    #[must_use]
    pub fn list_rated(
        &self,
        rules: &Rules,
        filter: &Filter,
        before: Option<u64>,
        limit: u32,
    ) -> RatedPage {
        let page = self.list(filter, before, limit);
        RatedPage {
            problems: page
                .problems
                .into_iter()
                .map(|summary| Rated {
                    severity: self.severity_of(rules, summary.number),
                    summary,
                })
                .collect(),
            more: page.more,
        }
    }

    /// [`Book::get`] with its severity.
    #[must_use]
    pub fn get_rated(&self, rules: &Rules, number: u64) -> Option<RatedProblem> {
        self.get(number).map(|problem| RatedProblem {
            severity: self.severity_of(rules, number),
            problem,
        })
    }

    /// A page of the problems `filter` wants, newest first, numbered
    /// below `before`.
    #[must_use]
    pub fn list(&self, filter: &Filter, before: Option<u64>, limit: u32) -> Page {
        let limit = usize::try_from(limit.clamp(1, 1000)).unwrap_or(1000);
        let wanted: Vec<&Summary> = self
            .problems
            .iter()
            .rev()
            .map(|k| &k.problem.summary)
            .filter(|s| before.is_none_or(|b| s.number < b))
            .filter(|s| filter.wants(s))
            .collect();
        let more = (wanted.len() > limit).then(|| wanted[limit - 1].number);
        Page {
            problems: wanted.into_iter().take(limit).cloned().collect(),
            more,
        }
    }

    /// Problem `number` whole.
    #[must_use]
    pub fn get(&self, number: u64) -> Option<Problem> {
        self.problems
            .iter()
            .find(|k| k.problem.summary.number == number)
            .map(|k| k.problem.clone())
    }

    /// Whether a problem the code `code` opened is open and holds.
    #[must_use]
    pub fn holds(&self, code: &str) -> bool {
        self.problems.iter().any(|k| {
            k.opened_by == code
                && k.problem.summary.status == Status::Open
                && k.problem.summary.holds
        })
    }

    /// How many are open.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.problems
            .iter()
            .filter(|k| k.problem.summary.status == Status::Open)
            .count()
    }

    /// Past [`MAX_PROBLEMS`]: the oldest closed go, then the oldest.
    fn bound(&mut self) {
        while self.problems.len() > MAX_PROBLEMS {
            let i = self
                .problems
                .iter()
                .position(|k| k.problem.summary.status == Status::Closed)
                .unwrap_or(0);
            self.problems.remove(i);
        }
    }
}

/// Add a line to a log, keeping its first and its newest.
fn push_line(log: &mut Vec<Line>, time: u64, text: String) {
    log.push(Line { time, text });
    if log.len() > MAX_LINES {
        log.remove(1);
    }
}

#[cfg(test)]
mod tests;
