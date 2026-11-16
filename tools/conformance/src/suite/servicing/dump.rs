// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 5: a host dump taken over QSI, fetched, and opened with a
//! standard tool; it holds what its manifest says and no secret.

use std::process::Command;

use qsictl::service::HostDump;

use super::{Opts, console, host};
use crate::suite::base::Run;

/// The entries a host dump holds, by a tool every workstation has:
/// Python's own `zipfile`, which also checks every entry's CRC.
fn listed(file: &std::path::Path) -> Result<Vec<String>, String> {
    let tested = Command::new("python3")
        .args(["-m", "zipfile", "-t"])
        .arg(file)
        .output()
        .map_err(|e| format!("python3: {e}"))?;
    if !tested.status.success() {
        return Err(format!(
            "python3 -m zipfile -t: {}",
            String::from_utf8_lossy(&tested.stderr)
        ));
    }
    let out = Command::new("python3")
        .args(["-m", "zipfile", "-l"])
        .arg(file)
        .output()
        .map_err(|e| format!("python3: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect())
}

/// Suite 5.
pub fn host_dump(run: &Run<'_>) -> Result<String, String> {
    let mut rig = host(run, &Opts::default())?;
    let mut c = console(&mut rig.q, "MC1", run.deadline())?;
    qsictl::service::host_dump(&mut c.link, HostDump::Take)?;
    let reply = c.ask(qsi::any::AnyRequest::Base(qsi::base::Request::Dumps(
        qsi::base::ListDumps {},
    )))?;
    let qsi::any::AnyReply::Base(qsi::base::Reply::Dumps(all)) = reply else {
        return Err(format!("{reply:?}"));
    };
    let dump = all.last().ok_or("no host dump listed")?.clone();
    if dump.kind != qsi::base::DumpKind::ServiceData
        || !dump.by.contains("MC") && !dump.by.contains("console")
    {
        return Err(format!("the dump listed: {dump:?}"));
    }
    let file = run.file("hostdump.zip");
    qsictl::service::host_dump(
        &mut c.link,
        HostDump::Fetch {
            name: dump.name.clone(),
            file: file.clone(),
        },
    )?;
    c.link.close();
    let names = listed(&file)?;
    let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
    let entries = hostdump::zip::entries(&bytes)?;
    let manifest = entries
        .iter()
        .find(|(n, _)| n == "MANIFEST.txt")
        .map(|(_, d)| String::from_utf8_lossy(d).into_owned())
        .ok_or("no MANIFEST.txt")?;
    for name in &names {
        if name != "MANIFEST.txt" && !manifest.contains(&format!("  {name}  ")) {
            return Err(format!("{name} is not in the manifest"));
        }
    }
    for want in [
        "log/refcodes.ndjson",
        "state/host.toml",
        "services/status.txt",
        "services/partitions.txt",
        "host/cgroups.txt",
    ] {
        if !names.iter().any(|n| n == want) {
            return Err(format!("{want} is not in the dump: {names:?}"));
        }
    }
    for (name, data) in &entries {
        let text = String::from_utf8_lossy(data);
        for secret in ["PRIVATE KEY", "$argon2"] {
            if text.contains(secret) {
                return Err(format!("{name} holds `{secret}`"));
            }
        }
        for gone in [identity::KEY_FILE, users::FILE, "qsi-consoles.toml"] {
            if name.ends_with(gone) {
                return Err(format!("{name} is in the dump"));
            }
        }
    }
    if !manifest.contains("identity.key: the host's private key") {
        return Err(format!(
            "the manifest does not say the key was left out:\n{manifest}"
        ));
    }
    Ok(format!(
        "{} taken over QSI ({} bytes, by {}), fetched whole, opened and checked by python3 -m \
         zipfile: {} entries, each in its manifest; the host's key, the users' hashes and the \
         consoles' pins left out and said so; no private key or password hash in any entry",
        dump.name,
        dump.bytes,
        dump.by,
        names.len()
    ))
}
