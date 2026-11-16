// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The pairing exchange on one connection (`docs/QSI.md` section 3):
//! QS/400's trust crate does the exchange, the host's trust keeps the
//! password and the pins.

use qsi::{Empty, msgid, pair};
use qsitrust::pair::{Confirm, Managed, Start};

use super::{Conn, note};

impl Conn {
    pub(super) fn pair(&mut self, id: u32, request: pair::Request) {
        if self.config.peer.pinned {
            let n = note(
                msgid::ALREADY_PAIRED,
                "This console is paired with this host already.",
            );
            self.refuse(id, &[n]);
            return;
        }
        let mut guard = self.trust.guard(&self.config.address);
        match request {
            pair::Request::Start(s) => {
                let started = self.config.binding.ok_or(()).and_then(|binding| {
                    let start = Start {
                        spake: s.spake,
                        name: s.name,
                    };
                    let name = self.config.hello.name.clone();
                    Managed::answer(&mut guard, binding, &start, &name).map_err(|_| ())
                });
                if let Ok((managed, answer)) = started {
                    self.pairing = Some(managed);
                    let reply = pair::Reply::Answer(pair::Answer {
                        spake: answer.spake,
                        confirm: answer.confirm,
                        name: answer.name,
                    });
                    let _ = self.session.reply_msg(id, &reply);
                } else {
                    let n = note(
                        msgid::NO_PAIRING,
                        "No pairing password is out at this host, or it has no attempts left.",
                    );
                    self.refuse(id, &[n]);
                }
            }
            pair::Request::Confirm(c) => {
                let Some(managed) = self.pairing.take() else {
                    let n = note(msgid::PAIRING_ORDER, "A confirmation came before a start.");
                    self.refuse(id, &[n]);
                    return;
                };
                if let Ok(console) = managed.confirm(&mut guard, &Confirm { confirm: c.confirm }) {
                    self.say(format!(
                        "console {:?} paired from {}, certificate {}",
                        console.name,
                        self.config.address,
                        console.fingerprint.hex()
                    ));
                    let _ = self.session.reply_msg(id, &pair::Reply::Done(Empty));
                } else {
                    self.say(format!("a pairing from {} failed", self.config.address));
                    self.failed += 1;
                    let n = note(msgid::PAIRING_FAILED, "The pairing did not confirm.");
                    self.refuse(id, &[n]);
                }
            }
        }
    }
}
