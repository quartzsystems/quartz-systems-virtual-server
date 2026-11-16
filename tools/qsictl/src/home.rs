// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What `qsictl` keeps: its identity and the hosts it paired with, in
//! its directory (`$XDG_CONFIG_HOME/qsictl`, `~/.config/qsictl`, or
//! `%APPDATA%\qsictl` on Windows).

use std::path::{Path, PathBuf};

use qsitrust::{Fingerprint, Identity};

const CERT: &str = "console.crt";
const KEY: &str = "console.key";
const HOSTS: &str = "hosts";

/// The default directory.
#[must_use]
pub fn default_dir() -> PathBuf {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        env("APPDATA")
            .unwrap_or_else(|| PathBuf::from("."))
            .join("qsictl")
    } else {
        env("XDG_CONFIG_HOME")
            .or_else(|| env("HOME").map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("qsictl")
    }
}

/// A host paired with: a name for it, where it is, its certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// The name `qsictl` calls it by.
    pub name: String,
    /// `address:port`.
    pub address: String,
    /// Its certificate's fingerprint.
    pub fingerprint: Fingerprint,
}

/// The directory and what is in it.
#[derive(Clone, Debug)]
pub struct Home {
    dir: PathBuf,
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(path)?, text.as_bytes())
}

impl Home {
    /// The home at `dir`.
    #[must_use]
    pub fn at(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Make an identity named `name`, unless there is one.
    ///
    /// # Errors
    /// It could not be made or written.
    pub fn init(&self, name: &str) -> Result<Identity, String> {
        if let Ok(i) = self.identity() {
            return Ok(i);
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        let id = Identity::make(name, "", now).map_err(|e| e.to_string())?;
        write_private(
            &self.dir.join(KEY),
            &qsitrust::to_pem("PRIVATE KEY", &id.key),
        )
        .map_err(|e| e.to_string())?;
        std::fs::write(
            self.dir.join(CERT),
            qsitrust::to_pem("CERTIFICATE", &id.cert),
        )
        .map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// The identity.
    ///
    /// # Errors
    /// There is none: `qsictl init` makes one.
    pub fn identity(&self) -> Result<Identity, String> {
        let read = |f: &str| {
            std::fs::read_to_string(self.dir.join(f))
                .map_err(|_| format!("no identity in {}: run `qsictl init`", self.dir.display()))
        };
        Identity::from_pem(&read(CERT)?, &read(KEY)?).map_err(|e| e.to_string())
    }

    /// The hosts paired with.
    #[must_use]
    pub fn hosts(&self) -> Vec<Host> {
        let text = std::fs::read_to_string(self.dir.join(HOSTS)).unwrap_or_default();
        text.lines()
            .filter_map(|l| {
                let mut f = l.split_whitespace();
                Some(Host {
                    name: f.next()?.to_string(),
                    address: f.next()?.to_string(),
                    fingerprint: Fingerprint::from_hex(f.next()?)?,
                })
            })
            .collect()
    }

    fn save(&self, hosts: &[Host]) -> Result<(), String> {
        use std::fmt::Write as _;
        let mut text = String::new();
        for h in hosts {
            let _ = writeln!(text, "{} {} {}", h.name, h.address, h.fingerprint.hex());
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        std::fs::write(self.dir.join(HOSTS), text).map_err(|e| e.to_string())
    }

    /// Keep `host`, in place of one of its name.
    ///
    /// # Errors
    /// The file could not be written.
    pub fn keep(&self, host: Host) -> Result<(), String> {
        let mut all: Vec<Host> = self
            .hosts()
            .into_iter()
            .filter(|h| h.name != host.name)
            .collect();
        all.push(host);
        self.save(&all)
    }

    /// Forget the host named `name`.
    ///
    /// # Errors
    /// No such host, or the file could not be written.
    pub fn forget(&self, name: &str) -> Result<(), String> {
        let all = self.hosts();
        if !all.iter().any(|h| h.name == name) {
            return Err(format!("no host named {name}"));
        }
        let kept: Vec<Host> = all.into_iter().filter(|h| h.name != name).collect();
        self.save(&kept)
    }

    /// The host named `name`, or the one host when there is one.
    ///
    /// # Errors
    /// None named so, or no name and not one host.
    pub fn host(&self, name: Option<&str>) -> Result<Host, String> {
        let all = self.hosts();
        match name {
            Some(n) => all
                .into_iter()
                .find(|h| h.name == n)
                .ok_or_else(|| format!("no host named {n}: `qsictl hosts` lists them")),
            None => match all.as_slice() {
                [one] => Ok(one.clone()),
                [] => Err("no host is paired: `qsictl pair` pairs one".into()),
                _ => Err("more than one host is paired: name one with --host".into()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_kept_and_forgotten() {
        let dir = std::env::temp_dir().join(format!("qsictl-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = Home::at(dir.clone());
        assert!(home.host(None).is_err());
        let h = Host {
            name: "vs1".into(),
            address: "192.0.2.1:2300".into(),
            fingerprint: Fingerprint([3; 32]),
        };
        home.keep(h.clone()).unwrap();
        assert_eq!(home.host(None).unwrap(), h);
        home.keep(Host {
            name: "vs2".into(),
            ..h.clone()
        })
        .unwrap();
        assert!(home.host(None).is_err());
        assert_eq!(home.host(Some("vs1")).unwrap(), h);
        home.forget("vs1").unwrap();
        assert_eq!(home.host(None).unwrap().name, "vs2");
        let id = home.init("me").unwrap();
        assert_eq!(home.identity().unwrap().fingerprint(), id.fingerprint());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
