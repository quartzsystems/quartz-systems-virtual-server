// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 1 and 2: an attended installation on one blank disk. Over
//! the serial port every screen is held against its golden file; on
//! the display the same keys are typed through QEMU's monitor and the
//! result is judged by what was installed.

use std::path::Path;
use std::time::{Duration, Instant};

use release::{fact, screen};

use super::{
    HOST, INSTALL_WITHIN, INSTALLER_ENDED, PASSWORD, USER, blank_disks, boot, golden, machine,
    power_off, sign_on, titled, with_media,
};
use crate::qemu::Launch;
use crate::qemu::term::key;
use crate::suite::base::{DHCP_ADDRESS, Guest, MAC, Run, all_ready, channel, fact_of};

/// The installer's title.
const TITLE: &str = "Install QS/VS";

/// A screen of the installer whose text has `mark`, by `deadline`.
fn installer(guest: &mut Guest<'_>, mark: &str, deadline: Instant) -> Result<String, String> {
    guest.screen(&format!("the installer's `{mark}`"), deadline, |grid| {
        grid.row(1).trim() == TITLE && grid.contains(mark)
    })
}

/// What marks each screen of the installer, in order, with the name of
/// its golden file and the keys that answer it over the serial port.
fn screens() -> Vec<(&'static str, &'static str, String)> {
    vec![
        (
            "install-welcome",
            "Press Enter to continue",
            key::ENTER.into(),
        ),
        (
            "install-disks",
            "Type 1 by the load-source disk",
            format!("1{}", key::ENTER),
        ),
        (
            "install-erase",
            "Type ERASE to confirm",
            format!("ERASE{}", key::ENTER),
        ),
        (
            "install-network",
            "Interfaces (blank takes the first",
            // The host name, the rest of the field cleared; DHCP on
            // the first interface with carrier as it stands.
            format!("{HOST}{}{}", key::FIELD_EXIT, key::ENTER),
        ),
        ("install-time", "The clock now shows", key::ENTER.into()),
        (
            "install-user",
            "A user ID is 1 to 10 letters",
            format!(
                "{}{}{PASSWORD}{}{PASSWORD}{}",
                USER.to_lowercase(),
                key::TAB,
                key::TAB,
                key::ENTER
            ),
        ),
        (
            "install-summary",
            "F10 erases the disks named",
            key::F10.into(),
        ),
    ]
}

/// The machine an attended installation runs on: one blank disk, a
/// NIC, the media in the CD-ROM drive.
fn installing(run: &Run<'_>, disk: &Path) -> Result<Launch, String> {
    let mut launch = with_media(run, machine(run, &[Some(disk)]), false)?;
    launch.nic = Some(MAC.into());
    launch.keys = true;
    Ok(launch)
}

/// Boot what was installed and check it is the host that was asked for.
fn installed<'a>(run: &Run<'a>, disk: &Path) -> Result<Guest<'a>, String> {
    let mut launch = machine(run, &[Some(disk)]);
    launch.nic = Some(MAC.into());
    launch.keys = true;
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();
    let status = channel(&mut guest)?.until("the management address", deadline, |s| {
        s.fact(fact::MANAGEMENT)
            .is_some_and(|m| m.starts_with(DHCP_ADDRESS))
    })?;
    all_ready(&status)?;
    if fact_of(&status, fact::HOST)? != HOST {
        return Err(format!("the host is {}", fact_of(&status, fact::HOST)?));
    }
    if fact_of(&status, fact::SLOT)? != "A" {
        return Err(format!(
            "the IPL is from slot {}",
            fact_of(&status, fact::SLOT)?
        ));
    }
    Ok(guest)
}

/// Suite 1.
pub fn serial(run: &Run<'_>) -> Result<String, String> {
    let disks = blank_disks(run, "attended", 1)?;
    let mut guest = run.boot(&installing(run, &disks[0])?)?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    let mut seen = 0;
    for (name, mark, keys) in screens() {
        let text = installer(&mut guest, mark, deadline)?;
        golden(name, &text)?;
        seen += 1;
        // Help is on every screen; the first one's is held too.
        if name == "install-welcome" {
            guest.type_keys(key::F1)?;
            let help = titled(&mut guest, "Help: Welcome", deadline)?;
            golden("install-help", &help)?;
            guest.type_keys(key::ENTER)?;
            installer(&mut guest, mark, deadline)?;
            seen += 1;
        }
        guest.type_keys(&keys)?;
    }
    // The progress as it starts: nothing done, the first step under way.
    let progress = installer(&mut guest, ">  C2004020  Partitioning the disks", deadline)?;
    golden("install-progress", &progress)?;
    let complete = installer(&mut guest, "Remove the installation media.", deadline)?;
    golden("install-complete", &complete)?;
    seen += 2;
    guest.type_keys(key::ENTER)?;
    guest.seen(INSTALLER_ENDED, deadline)?;
    guest.wait_exit(deadline)?;
    drop(guest);

    // The installed host: base ready, then a sign-on at the serial port.
    let mut guest = installed(run, &disks[0])?;
    let deadline = run.deadline();
    let menu = sign_on(&mut guest, USER, PASSWORD, deadline)?;
    golden("tools-main", &menu)?;
    guest.type_keys(key::F3)?;
    // Signed off: the port is PID 1's again and shows the status.
    let after = guest.log.len();
    guest.wait_for("the status screen after the sign-off", deadline, |log| {
        log[after.min(log.len())..].contains(screen::BEGIN)
    })?;
    if guest.log.contains(PASSWORD) {
        return Err("the password is in the serial log".into());
    }
    power_off(&mut guest)?;
    Ok(format!(
        "{seen} screens of the installer matched their golden files, typed at the serial port; \
         the installed host is {HOST} at {DHCP_ADDRESS} from slot A, base ready; {USER} signed \
         on to Service Tools and off again"
    ))
}

/// The `sendkey` name of a character.
fn key_name(c: char) -> Result<String, String> {
    Ok(match c {
        'a'..='z' | '0'..='9' => c.to_string(),
        'A'..='Z' => format!("shift-{}", c.to_ascii_lowercase()),
        ' ' => "spc".into(),
        '-' => "minus".into(),
        '.' => "dot".into(),
        other => return Err(format!("no key is known for `{other}`")),
    })
}

/// The keys of the display's keyboard that type what the serial port
/// is sent: the same answers, as `sendkey` names.
fn display_keys(typed: &str) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    let mut rest = typed;
    while !rest.is_empty() {
        let named = [
            (key::ENTER, "ret"),
            (key::TAB, "tab"),
            // Field Exit is the keypad's plus.
            (key::FIELD_EXIT, "kp_add"),
            (key::F10, "f10"),
            (key::F1, "f1"),
            (key::F3, "f3"),
        ]
        .into_iter()
        .find(|(bytes, _)| rest.starts_with(bytes));
        if let Some((bytes, name)) = named {
            keys.push(name.to_string());
            rest = &rest[bytes.len()..];
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            keys.push(key_name(c)?);
            rest = &rest[c.len_utf8()..];
        }
    }
    Ok(keys)
}

/// Suite 2.
pub fn display(run: &Run<'_>) -> Result<String, String> {
    let disks = blank_disks(run, "display", 1)?;
    let mut launch = installing(run, &disks[0])?;
    // Nothing is typed at the serial port: it only shows where the
    // installer is, since it draws the same screens there.
    launch.keys = false;
    launch.monitor = Some(run.file("monitor.sock"));
    let mut guest = run.boot(&launch)?;
    let deadline = Instant::now() + INSTALL_WITHIN;
    let mut typed = 0;
    for (_, mark, keys) in screens() {
        installer(&mut guest, mark, deadline)?;
        let keys = display_keys(&keys)?;
        typed += keys.len();
        guest.send_keys(&keys)?;
    }
    installer(&mut guest, "Remove the installation media.", deadline)?;
    guest.send_keys(&["ret".to_string()])?;
    typed += 1;
    guest.seen(INSTALLER_ENDED, deadline)?;
    guest.wait_exit(deadline)?;
    drop(guest);

    // Judged by what was installed: the host, its network, its user.
    let mut guest = installed(run, &disks[0])?;
    let deadline = run.deadline();
    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    guest.type_keys(key::F3)?;
    std::thread::sleep(Duration::from_millis(300));
    power_off(&mut guest)?;
    Ok(format!(
        "{typed} keys typed at the display's keyboard through the monitor installed the same \
         host: {HOST} at {DHCP_ADDRESS}, base ready, and {USER} signs on with the password typed"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answers_typed_at_the_serial_port_have_keys_on_the_display() {
        let keys = display_keys(&format!("ERASE{}", key::ENTER)).unwrap();
        assert_eq!(
            keys,
            ["shift-e", "shift-r", "shift-a", "shift-s", "shift-e", "ret"]
        );
        let keys = display_keys(&format!("vs-1{} a{}", key::FIELD_EXIT, key::F10)).unwrap();
        assert_eq!(keys, ["v", "s", "minus", "1", "kp_add", "spc", "a", "f10"]);
        // Every answer of the installation can be typed.
        for (_, _, typed) in screens() {
            display_keys(&typed).unwrap();
        }
        assert!(display_keys("é").is_err());
    }
}
