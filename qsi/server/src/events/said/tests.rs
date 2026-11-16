// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use super::*;

fn status(id: u32, name: &str, state: partd::State) -> partd::Status {
    partd::Status {
        id,
        name: name.into(),
        state,
        code: String::new(),
        text: String::new(),
        current: None,
        profile: String::new(),
        last_end: String::new(),
        message: None,
        pid: 0,
        ethernet: Vec::new(),
        service: partd::proto::Service::default(),
    }
}

fn posted(time: u64, code: &str) -> partd::Posted {
    partd::Posted {
        time,
        code: code.into(),
        ..partd::Posted::default()
    }
}

/// Each event's kind and what it names, in a word.
fn words(events: &[AnyEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            AnyEvent::Partitions(partitions::Event::Defined(d)) => {
                format!("{} {}", if d.exists { "made" } else { "gone" }, d.name)
            }
            AnyEvent::Partitions(partitions::Event::State(s)) => format!("state {}", s.name),
            AnyEvent::Refcodes(refcodes::Event::Code(c)) => {
                format!("code {} {}", c.name, c.posted.refcode.code)
            }
            AnyEvent::Vres(vres::Event::Switch(s)) => {
                format!("switch {} {}", s.name, s.switch.is_some())
            }
            other => format!("{other:?}"),
        })
        .collect()
}

fn said_by(events: &[AnyEvent]) -> Said {
    let mut said = Said::default();
    for e in events {
        said.note(e);
    }
    said
}

#[test]
fn what_changed_while_the_server_was_down_is_said_once() {
    let at_first = [
        status(1, "PROD", partd::State::Running),
        status(3, "OLD", partd::State::NotActivated),
    ];
    let first = partitions(
        &Said::default(),
        &at_first,
        |id| {
            if id == 1 {
                vec![posted(4, "C2005000"), posted(5, "C2005090")]
            } else {
                Vec::new()
            }
        },
        10,
    );
    let said = said_by(&first);

    // While the server was down: PROD ended with a code, TEST was
    // made, OLD was deleted.
    let now = [
        status(1, "PROD", partd::State::NotActivated),
        status(2, "TEST", partd::State::NotActivated),
    ];
    let history = |id| match id {
        1 => vec![
            posted(4, "C2005000"),
            posted(5, "C2005090"),
            posted(6, "A2005140"),
        ],
        _ => vec![posted(7, "C2006010")],
    };
    let missed = partitions(&said, &now, history, 20);
    assert_eq!(
        words(&missed),
        [
            "state PROD",
            "code PROD A2005140",
            "made TEST",
            "state TEST",
            "code TEST C2006010",
            "gone OLD",
        ]
    );
    let said = said_by(&[first, missed].concat());
    assert!(
        partitions(&said, &now, history, 30).is_empty(),
        "nothing twice"
    );
}

#[test]
fn an_event_that_repeats_what_was_said_is_not_pushed() {
    let mut said = Said::default();
    let e = partitions(
        &said,
        &[status(1, "PROD", partd::State::Running)],
        |_| vec![posted(5, "C2005000"), posted(5, "C2005090")],
        1,
    );
    for e in &e {
        assert!(said.note(e));
    }
    // The subscription brings what the comparison said already.
    for e in &e {
        assert!(!said.note(e), "{e:?}");
    }
    let later = translate::partd(&partd::Event::Code {
        id: 1,
        name: "PROD".into(),
        posted: posted(5, "C2005100"),
    });
    assert!(said.note(&later), "a new code in the same second is said");
}

#[test]
fn switches_made_changed_and_deleted_are_said() {
    let switch = |name: &str, mtu| vswitchd::SwitchStatus {
        switch: Some(vswitch::Switch {
            name: name.into(),
            id: 1,
            vlans: vec![1],
            mtu,
            uplink: None,
        }),
        ..vswitchd::SwitchStatus::default()
    };
    let said = said_by(&switches(
        &Said::default(),
        &[switch("LAN", 1500), switch("DMZ", 1500)],
        1,
    ));
    let now = [switch("LAN", 9000), switch("SAN", 1500)];
    assert_eq!(
        words(&switches(&said, &now, 2)),
        ["switch LAN true", "switch SAN true", "switch DMZ false"]
    );
}

#[test]
fn what_was_said_is_kept_on_disk() {
    let d = std::env::temp_dir().join(format!("qsi-said-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut said = Said::open(&d);
    assert!(!said.knows("partd"));
    said.known("partd");
    let pool = poold::PoolStatus {
        name: "POOL1".into(),
        ..poold::PoolStatus::default()
    };
    let look = Look::of(&[pool], &[], &[], &[]);
    for e in translate::pools(&Look::default(), &look, 1) {
        said.note(&e);
    }
    said.note(&AnyEvent::Base(qsi::base::Event::Refcode(
        qsi::base::Refcode {
            seq: 7,
            code: "C2001005".into(),
            words: Vec::new(),
            text: String::new(),
            time_ms: 1,
        },
    )));
    said.sync();
    let back = Said::open(&d);
    assert!(back.knows("partd"));
    assert_eq!(back.look(), look);
    assert_eq!(back.host_seen(), Some(7));
    std::fs::remove_dir_all(&d).unwrap();
}
