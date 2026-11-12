// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! How a service posts: it asks PID 1, which owns the console and the
//! order of the history. These are the `post` and `note` of a
//! service's `Appliance`.

use std::time::Duration;

use appliance::Event;
use supervise::control::{Calls, Client};

use crate::event::code_of;

/// PID 1 serves one request at a time; a busy moment is waited out.
const TRIES: u32 = 5;

fn ask(what: &str, call: impl Fn(&mut Client) -> bool) {
    let mut client = Client::new(release::CONTROL_SOCKET);
    for _ in 0..TRIES {
        if call(&mut client) {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    // Standard error reaches the log through PID 1's pipe.
    eprintln!("PID 1 did not take: {what}");
}

/// Post the code of `event` through PID 1.
pub fn post(event: &Event<'_>) {
    let (code, detail) = code_of(event);
    let text = code.text_with(&detail);
    ask(&format!("{} {text}", code.code), |c| {
        c.post(code.code.to_string(), text.clone()).is_ok()
    });
}

/// Post `code` through PID 1, with `detail` added to its text.
pub fn code(code: &crate::RefCode, detail: &str) {
    let text = code.text_with(detail);
    ask(&format!("{} {text}", code.code), |c| {
        c.post(code.code.to_string(), text.clone()).is_ok()
    });
}

/// Tell PID 1 a fact for its status screen.
pub fn note(key: &str, value: &str) {
    ask(&format!("{key} = {value}"), |c| {
        c.note(key.to_string(), value.to_string()).is_ok()
    });
}
