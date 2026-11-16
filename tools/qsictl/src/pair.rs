// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Pairing with a host: QS/400's exchange over a link that accepts any
//! host, the password shown at the host's Service Tools.

use qsi::any::AnyRequest;
use qsi::pair;
use qsitrust::Identity;
use qsitrust::pair::Paired;

use crate::link::Link;

/// A refusal's identifier and text, every message of it.
#[must_use]
pub fn refusal(reply: &qsisess::Reply) -> String {
    match &reply.outcome {
        qsisess::Outcome::Refused {
            msgid, text, also, ..
        } => {
            let mut lines = vec![format!("{msgid} {text}")];
            lines.extend(also.iter().map(|n| format!("{} {}", n.msgid, n.text)));
            lines.join("\n")
        }
        qsisess::Outcome::Result { .. } => "not refused".into(),
    }
}

/// Pair with the host at `address` as `identity`, named `name`: the
/// host as it paired, its certificate and name.
///
/// # Errors
/// No connection, or the host refused (its message), or its
/// confirmation does not hold.
pub fn pair(
    address: &str,
    identity: &Identity,
    password: &str,
    name: &str,
) -> Result<Paired, String> {
    let mut link = Link::connect(address, identity, None)?;
    let done = exchange(&mut link, identity, password, name);
    link.close();
    done
}

/// The exchange on an open link.
///
/// # Errors
/// As [`pair`].
pub fn exchange(
    link: &mut Link,
    identity: &Identity,
    password: &str,
    name: &str,
) -> Result<Paired, String> {
    let binding = link.binding(identity)?;
    let (console, start) =
        qsitrust::pair::Console::start(password, binding, name).map_err(|e| e.to_string())?;
    let reply = link.ask(&AnyRequest::Pair(pair::Request::Start(pair::Start {
        spake: start.spake,
        name: start.name,
    })))?;
    let Ok(Some(pair::Reply::Answer(a))) = reply.typed::<pair::Reply>() else {
        return Err(refusal(&reply));
    };
    let answered = console.answer(&qsitrust::pair::Answer {
        spake: a.spake,
        confirm: a.confirm,
        name: a.name,
    });
    let Ok((confirm, paired)) = answered else {
        // The host's confirmation does not hold: a wrong password, or
        // not that host. Confirm nonsense so the host counts it.
        let r = link.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
            confirm: vec![0; 32],
        })))?;
        return Err(format!(
            "the host's confirmation does not hold: {}",
            refusal(&r)
        ));
    };
    let done = link.ask(&AnyRequest::Pair(pair::Request::Confirm(pair::Confirm {
        confirm: confirm.confirm,
    })))?;
    if done.refusal().is_some() {
        return Err(refusal(&done));
    }
    Ok(paired)
}
