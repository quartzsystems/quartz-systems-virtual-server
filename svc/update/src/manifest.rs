// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! An update image's manifest (`docs/UPDATE.md`): what it is and what
//! it holds, as text, signed whole. Read as hostile: bounded, every
//! line known, every field checked.

use std::fmt::Write as _;

/// The manifest's first line.
pub const HEAD: &str = "QSVS-UPDATE 1";
/// The longest manifest.
pub const MAX_TEXT: usize = 64 * 1024;
/// The most fixes, state files and files it may name.
pub const MAX_ITEMS: usize = 256;
/// The longest word of a line: a name, a build.
pub const MAX_WORD: usize = 128;

/// One file the image holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    /// Its name.
    pub name: String,
    /// Its length.
    pub bytes: u64,
    /// Its SHA-256, 64 lower-case hex digits.
    pub sha256: String,
}

/// What an update image is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    /// The product's name; a test image's says that it is one.
    pub product: String,
    /// The release, `V1R0M1`.
    pub release: String,
    /// The build ID.
    pub build: String,
    /// The fixes it contains, `QVnnnnn`.
    pub fixes: Vec<String>,
    /// Each state file the release reads, with the newest schema
    /// version it reads.
    pub schemas: Vec<(String, u32)>,
    /// The files: each slot's UKI.
    pub files: Vec<File>,
}

fn word(w: &str) -> Result<String, String> {
    if w.is_empty() || w.len() > MAX_WORD || w.chars().any(char::is_control) {
        return Err(format!(
            "`{}` is not a word of the manifest",
            w.escape_debug()
        ));
    }
    Ok(w.to_string())
}

/// Whether `fix` is a fix's name: `QV` and five digits.
#[must_use]
pub fn valid_fix(fix: &str) -> bool {
    fix.len() == 7 && fix.starts_with("QV") && fix[2..].bytes().all(|b| b.is_ascii_digit())
}

impl Manifest {
    /// The text that is signed.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = format!(
            "{HEAD}\nproduct {}\nrelease {}\nbuild {}\n",
            self.product, self.release, self.build
        );
        for f in &self.fixes {
            let _ = writeln!(out, "fix {f}");
        }
        for (file, version) in &self.schemas {
            let _ = writeln!(out, "state {file} {version}");
        }
        for f in &self.files {
            let _ = writeln!(out, "file {} {} {}", f.name, f.bytes, f.sha256);
        }
        out
    }

    /// The manifest of `text`, or why it is not one.
    ///
    /// # Errors
    /// What is wrong with it.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_TEXT {
            return Err("the manifest is too long".into());
        }
        let mut lines = text.lines();
        if lines.next() != Some(HEAD) {
            return Err(format!("the manifest does not start with `{HEAD}`"));
        }
        let mut m = Manifest::default();
        for line in lines {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            let count = m.fixes.len() + m.schemas.len() + m.files.len();
            if count >= 3 * MAX_ITEMS {
                return Err("the manifest names too much".into());
            }
            match key {
                // A product's name has spaces: `QS/VS (test image)`.
                "product" if m.product.is_empty() => {
                    if rest.is_empty()
                        || rest.len() > MAX_WORD
                        || rest.chars().any(char::is_control)
                    {
                        return Err("the product's name is not one".into());
                    }
                    m.product = rest.to_string();
                }
                "release" if m.release.is_empty() => m.release = word(rest)?,
                "build" if m.build.is_empty() => m.build = word(rest)?,
                "fix" if m.fixes.len() < MAX_ITEMS => {
                    if !valid_fix(rest) {
                        return Err(format!("`{}` is not a fix", rest.escape_debug()));
                    }
                    m.fixes.push(rest.to_string());
                }
                "state" if m.schemas.len() < MAX_ITEMS => {
                    let (file, version) = rest
                        .split_once(' ')
                        .ok_or_else(|| "a state line names a file and a version".to_string())?;
                    let version = version.parse().map_err(|_| {
                        format!("`{}` is not a schema version", version.escape_debug())
                    })?;
                    m.schemas.push((word(file)?, version));
                }
                "file" if m.files.len() < MAX_ITEMS => {
                    let parts: Vec<&str> = rest.split(' ').collect();
                    let [name, bytes, sha256] = parts.as_slice() else {
                        return Err("a file line names a file, its size and its hash".into());
                    };
                    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(format!("`{}` is not a hash", sha256.escape_debug()));
                    }
                    m.files.push(File {
                        name: word(name)?,
                        bytes: bytes
                            .parse()
                            .map_err(|_| format!("`{}` is not a size", bytes.escape_debug()))?,
                        sha256: sha256.to_ascii_lowercase(),
                    });
                }
                _ => {
                    return Err(format!(
                        "`{}` is not a line of the manifest",
                        line.escape_debug()
                    ));
                }
            }
        }
        if m.product.is_empty() || m.release.is_empty() || m.build.is_empty() {
            return Err("the manifest names no product, release or build".into());
        }
        Ok(m)
    }

    /// The file named `name`.
    #[must_use]
    pub fn file(&self, name: &str) -> Option<&File> {
        self.files.iter().find(|f| f.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest as the build writes one.
    pub(crate) fn sample() -> Manifest {
        Manifest {
            product: "QS/VS".into(),
            release: "V1R0M1".into(),
            build: "phase9-1".into(),
            fixes: vec!["QV00001".into(), "QV00002".into()],
            schemas: vec![("host.toml".into(), 1), ("definition.toml".into(), 3)],
            files: vec![File {
                name: "SLOTA.EFI".into(),
                bytes: 15_000_000,
                sha256: "ab".repeat(32),
            }],
        }
    }

    #[test]
    fn a_manifest_round_trips() {
        let m = sample();
        let text = m.render();
        assert!(text.starts_with("QSVS-UPDATE 1\nproduct QS/VS\nrelease V1R0M1\n"));
        assert_eq!(Manifest::parse(&text), Ok(m));
    }

    #[test]
    fn what_is_not_a_manifest_is_refused() {
        let text = sample().render();
        for bad in [
            text.replacen("QSVS-UPDATE 1", "QSVS-UPDATE 2", 1),
            text.replacen("fix QV00001", "fix QV1", 1),
            text.replacen("state host.toml 1", "state host.toml one", 1),
            text.replacen(&"ab".repeat(32), "ab", 1),
            format!("{text}release V9R9M9\n"),
            format!("{text}junk\n"),
            text.replacen("build phase9-1\n", "", 1),
            "x".repeat(MAX_TEXT + 1),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "{bad}");
        }
    }
}
