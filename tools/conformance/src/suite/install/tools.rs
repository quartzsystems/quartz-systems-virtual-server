// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suites 9 and 10: Service Tools on an installed host, driven at the
//! serial port. The management network is changed without a restart,
//! and a change that loses the network is taken back; and the sign-on
//! rules hold: a wrong password, the lockout, the idle sign-off, users
//! added and removed, the last one kept.

use std::time::{Duration, Instant};

use release::{fact, screen};

use super::{
    Kind, MAIN_MENU, PASSWORD, SIGN_ON, USER, boot, fill, golden, installed, machine, option,
    power_off, sign_on, titled,
};
use crate::qemu::term::key;
use crate::suite::base::{DHCP_ADDRESS, DHCP_GATEWAY, Guest, MAC, Run, STATE_DIR, channel};

/// The title of the network panel and of the trial under it.
const NETWORK: &str = "Configure Management Network";
/// The lengths of the network form's fields, in order.
const NET_FIELDS: [usize; 8] = [40, 2, 1, 15, 2, 15, 4, 46];
/// The static address the suite gives the host: the one QEMU's user
/// network routes to it.
const STATIC_ADDRESS: &str = "10.0.2.15";
/// The static management line.
const STATIC_LINE: &str = "10.0.2.15/24 on eth0, gateway 10.0.2.2";

/// The management line PID 1 shows, once it is `wanted`.
fn management(guest: &mut Guest<'_>, wanted: &str, within: Duration) -> Result<String, String> {
    let deadline = Instant::now() + within;
    let status = channel(guest)?.until(&format!("management `{wanted}`"), deadline, |s| {
        s.fact(fact::MANAGEMENT)
            .is_some_and(|m| m.starts_with(wanted))
    })?;
    Ok(status.fact(fact::MANAGEMENT).unwrap_or("").to_string())
}

fn netd_pid(guest: &mut Guest<'_>) -> Result<u32, String> {
    Ok(channel(guest)?
        .status()?
        .service("netd")
        .map_or(0, |s| s.pid))
}

fn network_toml(guest: &mut Guest<'_>) -> Result<String, String> {
    channel(guest)?.text(&format!("{STATE_DIR}/network.toml"))
}

/// Type a change into the network form and wait for the word of it.
fn change(
    guest: &mut Guest<'_>,
    fields: &[Option<&str>; 8],
    word: &str,
    within: Duration,
) -> Result<String, String> {
    guest.type_keys(&format!("{}{}", fill(fields, &NET_FIELDS), key::ENTER))?;
    guest.screen(
        &format!("`{word}` after the change"),
        Instant::now() + within,
        |grid| grid.row(1).trim() == NETWORK && grid.contains(word),
    )
}

/// Suite 9.
pub fn network(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Single)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    launch.nic = Some(MAC.into());
    launch.keys = true;
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();
    let dhcp_line = management(&mut guest, DHCP_ADDRESS, Duration::from_secs(30))?;
    let pid = netd_pid(&mut guest)?;
    sign_on(&mut guest, USER, PASSWORD, deadline)?;
    let form = option(&mut guest, 3, NETWORK, deadline)?;
    golden("tools-network", &form)?;

    // DHCP to a static address: kept as soon as it works.
    let kept = "The new configuration is kept.";
    let fixed = [
        None,
        None,
        Some("2"),
        Some(STATIC_ADDRESS),
        Some("24"),
        Some(DHCP_GATEWAY),
        None,
        None,
    ];
    let shown = change(&mut guest, &fixed, kept, Duration::from_secs(20))?;
    golden("tools-network-kept", &shown)?;
    if let Err(e) = management(&mut guest, STATIC_LINE, Duration::from_secs(10)) {
        return Err(super::with_log(&mut guest, e));
    }
    let written = network_toml(&mut guest)?;
    if !written.contains("mode = \"static\"") || !written.contains("10.0.2.15/24") {
        return Err(format!(
            "network.toml after the change to static:\n{written}"
        ));
    }
    guest.type_keys(key::ENTER)?;
    let menu = titled(&mut guest, MAIN_MENU, deadline)?;
    if !menu.contains("Management network configuration changed.") {
        return Err(format!(
            "the main menu does not say the network changed:\n{menu}"
        ));
    }

    // And back to DHCP.
    option(&mut guest, 3, NETWORK, deadline)?;
    let dhcp = [
        None,
        None,
        Some("1"),
        Some(""),
        Some(""),
        Some(""),
        None,
        None,
    ];
    change(&mut guest, &dhcp, kept, Duration::from_secs(20))?;
    management(&mut guest, DHCP_ADDRESS, Duration::from_secs(10))?;
    let before = network_toml(&mut guest)?;
    if !before.contains("mode = \"dhcp\"") {
        return Err(format!("network.toml after the change back:\n{before}"));
    }
    guest.type_keys(key::ENTER)?;
    titled(&mut guest, MAIN_MENU, deadline)?;

    // A VLAN nothing answers on: no address, so the change is taken back.
    option(&mut guest, 3, NETWORK, deadline)?;
    let vlan = [None, None, None, None, None, None, Some("100"), None];
    let started = Instant::now();
    let trial = change(&mut guest, &vlan, "is being tried", Duration::from_secs(10))?;
    golden("tools-network-trial", &trial)?;
    let restored = guest.screen(
        "the change taken back",
        Instant::now() + Duration::from_secs(50),
        |grid| grid.contains("The new configuration was not kept."),
    )?;
    let took = started.elapsed();
    golden("tools-network-restored", &restored)?;
    if took < Duration::from_secs(28) || took > Duration::from_secs(40) {
        return Err(format!(
            "the change was taken back after {took:?}, not 30 s"
        ));
    }
    let line = management(&mut guest, DHCP_ADDRESS, Duration::from_secs(20))?;
    if network_toml(&mut guest)? != before {
        return Err("network.toml was written by a change that was taken back".into());
    }
    if channel(&mut guest)?
        .ls("/sys/class/net")?
        .iter()
        .any(|l| l == "eth0.100")
    {
        return Err("eth0.100 is still there".into());
    }
    let now = netd_pid(&mut guest)?;
    if now != pid {
        return Err(format!("netd was restarted: process {pid}, now {now}"));
    }
    guest.type_keys(key::ENTER)?;
    guest.type_keys(key::F3)?;
    guest.type_keys(key::F3)?;
    power_off(&mut guest)?;
    Ok(format!(
        "from Service Tools option 3: `{dhcp_line}` changed to `{STATIC_LINE}` and back, each \
         kept at once and written, in the same netd process {pid}; a change to VLAN 100, where \
         nothing answers, was taken back after {:.0} s: `{line}` again, network.toml untouched, \
         eth0.100 gone",
        took.as_secs_f64()
    ))
}

/// The message line after a sign-on with `password`.
fn refused(guest: &mut Guest<'_>, user: &str, password: &str) -> Result<String, String> {
    guest.type_keys(&format!(
        "{}{}",
        fill(&[Some(user), Some(password)], &[10, 32]),
        key::ENTER
    ))?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let shown = guest.screen("the word of the sign-on", deadline, |grid| {
        grid.row(1).trim() == SIGN_ON && !grid.message().is_empty()
    })?;
    Ok(shown.lines().last().unwrap_or("").trim().to_string())
}

/// The user the suite adds, and locks out.
const SECOND: &str = "OPER";
/// That user's password.
const SECOND_PASSWORD: &str = "second pass 22";

/// Users: one added, the first removed, the last one kept.
fn manage_users(guest: &mut Guest<'_>, deadline: Instant) -> Result<(), String> {
    let users_title = "Work with Service Tools User IDs";
    let list = option(guest, 5, users_title, deadline)?;
    golden("tools-users", &list)?;
    guest.type_keys("\x1b[17~")?;
    let add = titled(guest, "Add Service Tools User ID", deadline)?;
    golden("tools-user-add", &add)?;
    guest.type_keys(&format!(
        "{}{}",
        fill(
            &[Some("oper"), Some(SECOND_PASSWORD), Some(SECOND_PASSWORD)],
            &[10, 32, 32]
        ),
        key::ENTER
    ))?;
    let listed = match guest.screen("the user added", deadline, |grid| {
        grid.message() == format!("User ID {SECOND} added.")
    }) {
        Ok(listed) => listed,
        Err(e) => return Err(super::with_log(guest, e)),
    };
    if !listed.contains(SECOND) {
        return Err(format!("the list lacks {SECOND}:\n{listed}"));
    }
    // Remove the first user: option 4 by it, then F10.
    guest.type_keys(&format!("4{}", key::ENTER))?;
    let confirm = titled(guest, "Confirm Remove of User ID", deadline)?;
    golden("tools-user-remove", &confirm)?;
    guest.type_keys(key::F10)?;
    guest.screen("the user removed", deadline, |grid| {
        grid.message() == format!("User ID {USER} removed.")
    })?;
    // The one left cannot be removed.
    guest.type_keys(&format!("4{}", key::ENTER))?;
    titled(guest, "Confirm Remove of User ID", deadline)?;
    guest.type_keys(key::F10)?;
    guest.screen("the last user kept", deadline, |grid| {
        grid.message() == "The last user ID cannot be removed."
    })?;
    Ok(())
}

/// What was logged, and that no password is anywhere in clear.
fn nothing_in_clear(guest: &mut Guest<'_>) -> Result<(), String> {
    // What was logged, and what was not.
    std::thread::sleep(Duration::from_secs(1));
    let log = channel(guest)?.text("/data/log/system.ndjson")?;
    for wanted in [
        "sign-on refused at the serial port for service tools user \\\"ADMIN\\\"",
        "sign-on refused at the serial port for service tools user \\\"NOBODY\\\"",
        "user ID disabled for 300 s",
        "signed off after no key for the idle time",
        "service tools user OPER added by ADMIN",
    ] {
        if !log.contains(wanted) {
            return Err(format!("the system log lacks `{wanted}`"));
        }
    }
    let users = channel(guest)?.cat(&format!("{STATE_DIR}/{}", users::FILE))?;
    let users = String::from_utf8_lossy(&users).into_owned();
    let history = channel(guest)?.text("/data/log/refcodes.ndjson")?;
    let serial_log = guest.log.clone();
    for secret in [
        PASSWORD,
        SECOND_PASSWORD,
        "wrong password 1",
        "not the password 9",
    ] {
        for (place, text) in [
            ("the system log", &log),
            ("the users' file", &users),
            ("the reference code history", &history),
            ("the serial port's output", &serial_log),
        ] {
            if text.contains(secret) {
                return Err(format!("a password is in {place} in clear"));
            }
        }
    }
    if !users.contains("$argon2id$") || users.contains(USER) {
        return Err("the users' file is not the one user's hash".into());
    }
    Ok(())
}

/// Suite 10.
pub fn sign_on_rules(run: &Run<'_>) -> Result<String, String> {
    let disks = installed(run, Kind::Single)?;
    let mut launch = machine(run, &[Some(disks[0].as_path())]);
    launch.keys = true;
    launch.oem = vec![qsvshook::oem_string("short-idle")];
    let mut guest = boot(run, &launch)?;
    let deadline = run.deadline();

    // A wrong password is refused and says no more than that.
    guest.type_keys(key::ENTER)?;
    let display = titled(&mut guest, SIGN_ON, deadline)?;
    golden("tools-sign-on", &display)?;
    let wrong = refused(&mut guest, USER, "not the password 9")?;
    if wrong != "User ID or password not correct." {
        return Err(format!("a wrong password is answered with `{wrong}`"));
    }
    let nobody = refused(&mut guest, "NOBODY", PASSWORD)?;
    if nobody != wrong {
        return Err(format!("an unknown user ID is answered with `{nobody}`"));
    }

    // Users: added, removed, and the last one kept.
    guest.type_keys(&format!(
        "{}{}",
        fill(&[Some(USER), Some(PASSWORD)], &[10, 32]),
        key::ENTER
    ))?;
    titled(&mut guest, MAIN_MENU, deadline)?;
    manage_users(&mut guest, deadline)?;

    // No key for the idle time: signed off, the port PID 1's again.
    let after = guest.log.len();
    let idle_from = Instant::now();
    guest.wait_for(
        "the status screen after the idle sign-off",
        Instant::now() + Duration::from_secs(40),
        |log| log[after.min(log.len())..].contains(screen::BEGIN),
    )?;
    let idle = idle_from.elapsed();

    // Five wrong passwords disable the ID; the right one then does not help.
    guest.type_keys(key::ENTER)?;
    titled(&mut guest, SIGN_ON, deadline)?;
    for attempt in 1..=4 {
        let word = refused(&mut guest, SECOND, "wrong password 1")?;
        if word != wrong {
            return Err(format!(
                "wrong password {attempt} is answered with `{word}`"
            ));
        }
    }
    let locked = refused(&mut guest, SECOND, "wrong password 1")?;
    if !locked.starts_with("User ID disabled after 5 wrong passwords.") {
        return Err(format!(
            "the fifth wrong password is answered with `{locked}`"
        ));
    }
    let still = refused(&mut guest, SECOND, SECOND_PASSWORD)?;
    if !still.starts_with("User ID disabled") {
        return Err(format!("a disabled user ID signs on: `{still}`"));
    }
    guest.type_keys(key::F3)?;

    nothing_in_clear(&mut guest)?;
    power_off(&mut guest)?;
    Ok(format!(
        "a wrong password and an unknown user ID are refused alike; {SECOND} added, {USER} \
         removed, the last user ID kept; signed off after {:.0} s without a key; five wrong \
         passwords disable {SECOND} and the right one is then refused too; every refusal is in \
         the log, and no password is in a log, the users' file or the serial output",
        idle.as_secs_f64()
    ))
}
