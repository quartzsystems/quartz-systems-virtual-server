// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use super::*;
use qsipart::service::{Ack, Keylock, Pong, Refused, SysState, kind};

fn hello() -> FromSystem {
    FromSystem::Hello(Hello {
        version: 1,
        caps: caps::SERVICE_V1 | (1 << 40),
        system: "QS400".into(),
        release: "V1R1M0".into(),
        serial: "K2M4107".into(),
        slot: "A".into(),
        ipl_type: "A".into(),
        keylock: Keylock::Normal,
    })
}

fn bytes(m: &FromSystem) -> Vec<u8> {
    m.encode().unwrap()
}

fn decode_to_system(b: &[u8]) -> ToSystem {
    let mut r = Reader::new();
    r.push(b);
    ToSystem::decode(&r.next_frame().unwrap()).unwrap().unwrap()
}

#[test]
fn hello_code_and_state_come_through_in_pieces_and_after_junk() {
    let mut c = Channel::new(100);
    let mut stream = b"noise".to_vec();
    stream.extend(bytes(&hello()));
    stream.extend(bytes(&FromSystem::Refcode(Refcode {
        code: "C1001000".into(),
        words: Vec::new(),
        text: "IPL started".into(),
        time_ms: 1,
        current: true,
    })));
    stream.extend(bytes(&FromSystem::State(State {
        state: SysState::Ipl,
        fix_level: String::new(),
        addresses: Vec::new(),
    })));
    let mut got = Vec::new();
    for piece in stream.chunks(7) {
        got.extend(c.bytes(piece, 100));
    }
    assert!(matches!(got[0], Happened::Hello(ref h) if h.serial == "K2M4107"));
    assert!(matches!(got[1], Happened::Refcode(ref r) if r.code == "C1001000" && r.current));
    assert!(matches!(got[2], Happened::State(ref s) if s.state == SysState::Ipl));
    assert_eq!(got.len(), 3);
    // Only what both have: the system's later bit is not the host's.
    assert_eq!(c.caps, caps::SERVICE_V1);
    assert!(c.has(caps::POWER_DOWN));
    assert_eq!(c.version, 1);
}

#[test]
fn a_request_is_answered_refused_or_timed_out() {
    let mut c = Channel::new(0);
    c.bytes(&bytes(&hello()), 0);
    let f = c
        .ask(
            Ask::PowerDown {
                immediate: false,
                delay: 30,
                restart: true,
            },
            7,
            0,
        )
        .unwrap();
    let ToSystem::PowerDown(p) = decode_to_system(&f) else {
        panic!("not a power down");
    };
    assert_eq!((p.delay, p.restart, p.immediate), (30, true, false));
    let got = c.bytes(&bytes(&FromSystem::Ack(Ack { id: p.id })), 1);
    assert_eq!(
        got,
        [Happened::Answered {
            tag: 7,
            ask: Ask::PowerDown {
                immediate: false,
                delay: 30,
                restart: true
            },
            result: Ok(())
        }]
    );
    // An answer to nothing that waits is passed over.
    assert!(
        c.bytes(&bytes(&FromSystem::Ack(Ack { id: p.id })), 1)
            .is_empty()
    );

    let f = c.ask(Ask::Panel(21), 8, 2).unwrap();
    let ToSystem::Panel(panel) = decode_to_system(&f) else {
        panic!("not a panel");
    };
    let refused = FromSystem::Refused(Refused {
        id: panel.id,
        msgid: "QSI0021".into(),
        text: "The keylock is not in manual.".into(),
    });
    let got = c.bytes(&bytes(&refused), 3);
    assert!(matches!(
        &got[0],
        Happened::Answered { tag: 8, result: Err((id, _)), .. } if id == "QSI0021"
    ));

    c.ask(Ask::Panel(21), 9, 10).unwrap();
    let (happened, _) = c.tick(10 + ANSWER_SECS);
    assert!(happened.contains(&Happened::TimedOut {
        tag: 9,
        ask: Ask::Panel(21)
    }));
    // At most a few wait.
    for tag in 0..MAX_WAITING as u64 {
        assert!(c.ask(Ask::Panel(21), tag, 100).is_some());
    }
    assert!(c.ask(Ask::Panel(21), 99, 100).is_none());
    assert_eq!(c.abandon().len(), MAX_WAITING);
}

#[test]
fn pings_go_and_a_system_that_stops_answering_is_not_responding() {
    let mut c = Channel::new(0);
    // No ping before a hello.
    assert!(c.tick(5).1.is_empty());
    c.bytes(&bytes(&hello()), 5);
    let (_, send) = c.tick(5);
    let ToSystem::Ping(ping) = decode_to_system(&send[0]) else {
        panic!("not a ping");
    };
    assert!(c.tick(6).1.is_empty(), "one at a time");
    c.bytes(
        &bytes(&FromSystem::Pong(Pong {
            id: ping.id,
            time_ms: 0,
        })),
        7,
    );
    let mut silent = false;
    for now in 8..8 + PING_SECS * MISSED {
        silent |= c.tick(now).0.contains(&Happened::NotResponding);
    }
    assert!(silent && c.silent);
    let back = c.bytes(&bytes(&FromSystem::Pong(Pong { id: 0, time_ms: 0 })), 60);
    assert_eq!(back, [Happened::Responding]);
    assert!(!c.silent);
}

#[test]
fn nonsense_closes_the_channel_once_and_nothing_grows() {
    // Bodies of a known kind that do not read.
    let mut c = Channel::new(0);
    let bad = qsipart::frame::encode(kind::REFCODE, &[1, 2, 3]).unwrap();
    let mut said = Vec::new();
    for _ in 0..=MAX_BAD {
        said.extend(c.bytes(&bad, 0));
    }
    assert!(matches!(said.last(), Some(Happened::Nonsense(_))));
    assert!(c.bytes(&bytes(&hello()), 0).is_empty(), "closed");

    // A kind of a later version's is passed over, not nonsense.
    let mut c = Channel::new(0);
    let later = qsipart::frame::encode(0x00FF, b"later").unwrap();
    for _ in 0..100 {
        assert!(c.bytes(&later, 0).is_empty());
    }

    // A flood.
    let mut c = Channel::new(0);
    let state = bytes(&FromSystem::State(State {
        state: SysState::Complete,
        fix_level: String::new(),
        addresses: Vec::new(),
    }));
    let flood: Vec<u8> = state.repeat(usize::try_from(MAX_FRAMES_A_SECOND).unwrap() + 1);
    let got = c.bytes(&flood, 0);
    assert!(matches!(got.last(), Some(Happened::Nonsense(_))));

    // Junk without end.
    let mut c = Channel::new(0);
    let mut closed = false;
    for _ in 0..(MAX_JUNK / 4096 + 2) {
        closed |= c
            .bytes(&[b'x'; 4096], 0)
            .iter()
            .any(|h| matches!(h, Happened::Nonsense(_)));
    }
    assert!(closed);
}
