// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 9: QS/400 on the network. It is installed from its ISO, as
//! `pool-install` does, but from a copy whose response file also has
//! `ip` and `gateway`: the installation configures the interface on the
//! partition's adapter, unattended (`docs/QS400_GUEST.md`). The adapter
//! is on a switch whose uplink is on QEMU's user network, where a port of
//! the harness's host is forwarded to QS/400's telnet port: the harness
//! connects to it, negotiates a 5250 session, and reads the sign-on
//! display.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use model::profile::{IplSource, MediaDevice};
use partd::Calls as _;
use poold::proto::Calls as _;
use vswitch::Uplink;

use super::{Net, Rig, adapter, boot, set_switches, switch, uplink_location};
use crate::qemu::image::blank;
use crate::suite::base::{Run, channel, posted};
use crate::suite::partition::manager::{self, Spec};
use crate::suite::partition::qs400::ipl_complete;
use crate::suite::partition::{NAME, QS400_MEMORY_MIB};
use crate::suite::storage::{POOL_DISK_BYTES, disk_names, pool_until, poold, qs400_iso, said};

/// QS/400's address on QEMU's user network.
const ADDRESS: &str = "10.0.2.50";
/// How long the installation may take, three levels deep.
const INSTALL_WITHIN: Duration = Duration::from_secs(3600);

/// The response file of this suite: `pool-install`'s with QS/400's
/// address and gateway, and no comment, so that it is no longer than
/// the file it takes the place of.
fn response() -> String {
    format!(
        "QS400-INSTALL-RSP 1\nlic-option = 2\nload-source = *FIRST\nlanguage = 2924\n\
         system-name = QSNET\nqsecofr-password = Quartz1\nlicpgm = *NONE\nip = {ADDRESS}/24\n\
         gateway = 10.0.2.2\n"
    )
}

/// A copy of QS/400's ISO at `to`, with its response file's bytes
/// replaced by this suite's, padded with a comment to the same length.
pub fn iso_with_address(iso: &Path, to: &Path) -> Result<String, String> {
    let mut bytes = std::fs::read(iso).map_err(|e| format!("{}: {e}", iso.display()))?;
    let old = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("qs400/pool-install.rsp"),
    )
    .map_err(|e| format!("pool-install.rsp: {e}"))?;
    let at = bytes
        .windows(old.len())
        .position(|w| w == old.as_bytes())
        .ok_or("the ISO's response file is not pool-install.rsp")?;
    let mut new = response();
    if new.len() + 2 > old.len() {
        return Err(format!(
            "the response file of {} bytes does not fit in {}",
            new.len(),
            old.len()
        ));
    }
    new.push('#');
    while new.len() < old.len() - 1 {
        new.push(' ');
    }
    new.push('\n');
    bytes[at..at + old.len()].copy_from_slice(new.as_bytes());
    std::fs::write(to, &bytes).map_err(|e| format!("{}: {e}", to.display()))?;
    Ok(new)
}

/// `Sign On` in EBCDIC (CCSID 37).
const SIGN_ON: [u8; 7] = [0xE2, 0x89, 0x87, 0x95, 0x40, 0xD6, 0x95];

const IAC: u8 = 255;
const SB: u8 = 250;
const SE: u8 = 240;
const WILL: u8 = 251;
const WONT: u8 = 252;
const DO: u8 = 253;
const DONT: u8 = 254;
const TTYPE: u8 = 24;
const EOR: u8 = 25;
const BINARY: u8 = 0;
const ENVIRON: u8 = 39;

/// A TN5250 session to `port` of the harness's host: the telnet options
/// negotiated as a 5250 display (`IBM-3179-2`), then the display's data
/// read until it has the sign-on text. The data stream read.
pub fn tn5250(port: u16, within: Duration) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + within;
    let mut stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(e) if Instant::now() > deadline => return Err(format!("telnet: {e}")),
            Err(_) => std::thread::sleep(Duration::from_secs(2)),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let mut data = Vec::new();
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    while Instant::now() < deadline {
        let n = match stream.read(&mut buf) {
            Ok(0) => {
                return Err(format!(
                    "the telnet server closed after {} bytes",
                    data.len()
                ));
            }
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(e) => return Err(format!("telnet: {e}")),
        };
        pending.extend_from_slice(&buf[..n]);
        let mut out = Vec::new();
        let mut i = 0;
        while i < pending.len() {
            if pending[i] != IAC {
                data.push(pending[i]);
                i += 1;
                continue;
            }
            let Some(&cmd) = pending.get(i + 1) else {
                break;
            };
            match cmd {
                DO | DONT | WILL | WONT => {
                    let Some(&opt) = pending.get(i + 2) else {
                        break;
                    };
                    let ours = [TTYPE, EOR, BINARY, ENVIRON].contains(&opt);
                    match cmd {
                        DO => out.extend_from_slice(&[IAC, if ours { WILL } else { WONT }, opt]),
                        WILL => out.extend_from_slice(&[IAC, if ours { DO } else { DONT }, opt]),
                        _ => {}
                    }
                    i += 3;
                }
                SB => {
                    let Some(end) = pending[i..].windows(2).position(|w| w == [IAC, SE]) else {
                        break;
                    };
                    let body = &pending[i + 2..i + end];
                    match body.first() {
                        Some(&TTYPE) => {
                            out.extend_from_slice(&[IAC, SB, TTYPE, 0]);
                            out.extend_from_slice(b"IBM-3179-2");
                            out.extend_from_slice(&[IAC, SE]);
                        }
                        Some(&ENVIRON) => out.extend_from_slice(&[IAC, SB, ENVIRON, 0, IAC, SE]),
                        _ => {}
                    }
                    i += end + 2;
                }
                IAC => {
                    data.push(IAC);
                    i += 2;
                }
                _ => i += 2,
            }
        }
        pending.drain(..i);
        if !out.is_empty() {
            stream.write_all(&out).map_err(|e| format!("telnet: {e}"))?;
        }
        if data.windows(SIGN_ON.len()).any(|w| w == SIGN_ON) {
            return Ok(data);
        }
    }
    Err(format!(
        "no sign-on display within {within:?}: {} bytes of 5250 data",
        data.len()
    ))
}

/// Suite 9.
pub fn suite(run: &Run<'_>) -> Result<String, String> {
    let iso = qs400_iso()?;
    let patched = run.file("qs400-net.iso");
    let rsp = iso_with_address(&iso, &patched)?;
    let pool = run.file("pool0.img");
    blank(&pool, 4 * POOL_DISK_BYTES + (2 << 30))?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map_err(|e| e.to_string())?
        .port();
    let rig = Rig {
        user: vec![Some((port, ADDRESS.into(), 23))],
        disks: vec![pool, patched.clone()],
        ..Rig::default()
    };
    let Net { mut guest, .. } = boot(run, &rig)?;
    let g = &mut guest;
    // The host's seed is QSVSUNIT0; the pool's disk and the ISO follow.
    let names = disk_names(g, 3)?;
    let names = &names[1..];
    said(poold(g)?.create_pool(
        "POOL1".into(),
        "none".into(),
        vec![names[0].clone()],
        Vec::new(),
        Vec::new(),
    ))?;
    pool_until(g, "POOL1", run.deadline(), |p| !p.mount.is_empty())?;
    said(poold(g)?.designate("POOL1".into()))?;
    channel(g)?.ask_within(
        &format!("upload QS400 {}", names[1]),
        Duration::from_mins(15),
    )?;
    said(poold(g)?.create_vdisk("POOL1".into(), "PRODLS".into(), 8192, false))?;
    let at = uplink_location(g, 0)?;
    set_switches(
        g,
        &[switch(
            "PRODNET",
            1,
            &[10],
            1500,
            Some(Uplink {
                interfaces: vec![at.clone()],
                bond: None,
                native_vlan: Some(10),
            }),
        )],
    )?;
    let id = manager::partition(g, NAME)?;
    let mut spec = Spec::new(1, QS400_MEMORY_MIB, "POOL1/PRODLS");
    spec.name = "INSTALL".into();
    spec.ethernet = vec![adapter(5, "PRODNET", 10, &[])];
    let mut profile = spec.profile();
    profile.optical = vec![MediaDevice {
        slot: 3,
        media: Some("QS400".into()),
    }];
    manager::said(manager::partd(g)?.put_profile(id, profile))?;
    let started = Instant::now();
    manager::said(manager::partd(g)?.activate(id, "INSTALL".into(), Some(IplSource::D), None))?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    g.seen(&posted(&refcode::PARTITION_MEDIA_READ, ""), deadline)?;
    g.seen(&posted(&refcode::PARTITION_RESTART, ""), deadline)?;
    ipl_complete(g, deadline)?;
    let installed = started.elapsed();
    let data = tn5250(port, Duration::from_secs(300))?;
    Ok(format!(
        "QS/400 installed in {:.0} s from a copy of its ISO whose response file has `ip = \
         {ADDRESS}/24` and `gateway = 10.0.2.2` ({} bytes); on PRODNET, uplink {at} to QEMU's user \
         network: its telnet port answered on 127.0.0.1:{port}, and a 5250 session negotiated as \
         IBM-3179-2 read the sign-on display ({} bytes of data stream)",
        installed.as_secs_f64(),
        rsp.len(),
        data.len()
    ))
}

/// The suite's ISO path, for the record.
pub fn _iso(run: &Run<'_>) -> PathBuf {
    run.file("qs400-net.iso")
}
