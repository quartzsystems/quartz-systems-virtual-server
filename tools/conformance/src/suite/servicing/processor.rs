// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 14: a processor without what QSI's cryptography needs. QEMU
//! gives the host's processor without AVX2, ADX, AES and PCLMULQDQ: the
//! host IPLs, the service interface is down with its code and its
//! problem, and Service Tools works.

use std::time::Duration;

use crate::suite::base::{Run, channel};
use crate::suite::install::{Kind, PASSWORD, USER, boot, installed, machine, option, sign_on};
use crate::suite::qsi::eventually;

/// Suite 14.
pub fn processor(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Single)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    launch.nic = Some(crate::suite::base::MAC.into());
    launch.keys = true;
    launch.qsi = Some(crate::suite::qsi::free_port()?);
    launch.cpu = Some("host,-avx2,-adx,-aes,-pclmulqdq".into());
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();
    guest.seen(&refcode::QSI_PROCESSOR.tag(), deadline)?;
    // Nothing listens on the QSI port.
    let address = format!("127.0.0.1:{}", launch.qsi.unwrap_or(0));
    let id = crate::suite::qsi::identity("MC1")?;
    if qsictl::link::Link::connect(&address, &id, None).is_ok() {
        return Err("a session was made with the service interface down".into());
    }
    // The problem, as the problem service keeps it.
    let found = eventually(Duration::from_mins(2), || {
        channel(&mut guest)
            .ok()?
            .text("/data/problems/problems.json")
            .ok()
            .filter(|t| t.contains(refcode::QSI_PROCESSOR.code))
    })
    .ok_or("no problem of the processor")?;
    // Service Tools: its status and option 4, which says why.
    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    let menu = option(&mut guest, 4, crate::suite::qsi::PAIRING, deadline)?;
    if !menu.contains("The processor lacks") || !menu.contains("Everything at this console works.")
    {
        return Err(format!("option 4 does not say why:\n{menu}"));
    }
    let lacking = menu
        .lines()
        .find(|l| l.contains("The processor lacks"))
        .unwrap_or("")
        .trim()
        .to_string();
    let _ = guest.type_keys(crate::qemu::term::key::F3);
    let status = channel(&mut guest)?.status()?;
    let services = status
        .services
        .iter()
        .filter(|s| s.state == "ready")
        .count();
    crate::suite::install::power_off(&mut guest)?;
    Ok(format!(
        "with QEMU's processor short of AVX2, ADX, AES and PCLMULQDQ the host IPLed, posted {}, \
         listened on nothing, and opened its problem ({} bytes of the log); Service Tools option 4 \
         said `{lacking}`; {services} services ready",
        refcode::QSI_PROCESSOR.code,
        found.len()
    ))
}
