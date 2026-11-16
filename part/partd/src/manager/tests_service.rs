// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service channel, the panel, the console and dumps, against the
//! machine that only writes down what it was asked.

use std::path::PathBuf;
use std::sync::mpsc;

use model::{Keylock, sample};
use qsipart::console as qcon;
use qsipart::service::{self as qs, FromSystem, ToSystem};
use resource::{Host, Topology};

use super::tests::Fake;
use super::{Answer, Ended, Manager, Port};
use crate::proto::{
    ConsoleIn, ConsoleOut, DumpInfo, Event, PanelArgs, Partitions, Shutdown, Source, State,
};
use crate::store::Store;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("partd-svc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }

    fn manager_with(&self, fake: Fake) -> Manager<Fake> {
        let store = Store::at(&self.0);
        let settings = store.settings().unwrap();
        let host = Host {
            topology: Topology::uniform(8, 2),
            installed_mib: 8192,
            configurable_mib: settings.configurable_mib(8192),
            settings,
        };
        let mut m = Manager::new(fake, store, host, "HOST01");
        m.system.now = 1_794_000_000;
        m.start();
        m
    }

    fn manager(&self) -> Manager<Fake> {
        self.manager_with(Fake::default())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn partition(m: &mut Manager<Fake>, name: &str, keylock: Keylock) -> u32 {
    let id = m.create(name.to_string()).unwrap().id;
    let mut profile = sample::dedicated();
    profile.scsi[0].luns[0].backing = format!("/dev/{name}");
    m.put_profile(id, profile).unwrap();
    let mut d = m.definition(id).unwrap();
    d.keylock = keylock;
    m.change(d).unwrap();
    id
}

fn says(m: &mut Manager<Fake>, id: u32, message: &FromSystem) {
    m.channel_bytes(id, Port::Service, &message.encode().unwrap());
}

fn hello(serial: &str, keylock: qs::Keylock) -> FromSystem {
    FromSystem::Hello(qs::Hello {
        version: 1,
        caps: qsipart::caps::SERVICE_V1,
        system: "QS400".into(),
        release: "V1R2M0".into(),
        serial: serial.into(),
        slot: "A".into(),
        ipl_type: "A".into(),
        keylock,
    })
}

fn code(c: &str) -> FromSystem {
    FromSystem::Refcode(qs::Refcode {
        code: c.into(),
        words: vec!["00000042".into()],
        text: format!("text of {c}"),
        time_ms: 0,
        current: false,
    })
}

fn sys_state(s: qs::SysState) -> FromSystem {
    FromSystem::State(qs::State {
        state: s,
        fix_level: "C26318110".into(),
        addresses: vec!["10.0.2.15/24".into()],
    })
}

fn status(m: &mut Manager<Fake>, id: u32) -> crate::proto::Status {
    m.list().into_iter().find(|s| s.id == id).unwrap()
}

/// What was sent to the system on the service port, decoded.
fn sent(m: &Manager<Fake>) -> Vec<ToSystem> {
    let mut r = qsipart::Reader::new();
    let mut out = Vec::new();
    for (_, port, frame) in &m.system.sent {
        if *port == Port::Service {
            r.push(frame);
            while let Some(f) = r.next_frame() {
                out.extend(ToSystem::decode(&f).ok().flatten());
            }
        }
    }
    out
}

/// Activate `id`, its kernel entered and its system up to IPL complete.
fn up(m: &mut Manager<Fake>, id: u32) {
    m.activate(id, String::new(), None, None).unwrap();
    m.report(id, "REF C2005090 Partition started: X, side A", false);
    m.channel_up(id, Port::Service);
    let serial = m.definition(id).unwrap().serial;
    says(m, id, &hello(&serial, qs::Keylock::Normal));
    says(m, id, &sys_state(qs::SysState::Ipl));
    assert_eq!(status(m, id).state, State::Starting);
    says(m, id, &code("C1001000"));
    says(m, id, &code("C6001090"));
    says(m, id, &sys_state(qs::SysState::Complete));
    assert_eq!(status(m, id).state, State::Running);
}

#[test]
fn the_system_s_codes_and_states_drive_the_partition() {
    let scratch = Scratch::new("states");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Normal);
    up(&mut m, id);
    let s = status(&mut m, id);
    assert!(s.service.up && s.service.responding);
    assert_eq!(s.service.release, "V1R2M0");
    assert_eq!(s.service.fix_level, "C26318110");
    assert_eq!(s.service.addresses, ["10.0.2.15/24"]);
    assert_eq!(s.code, "C6001090");
    // The host answered the hello with its own.
    assert!(matches!(sent(&m)[0], ToSystem::Hello(_)));
    // The codes are in the history in order, each once, said whose.
    let history = m.history(id).unwrap();
    let system: Vec<&str> = history
        .iter()
        .filter(|p| p.source == Source::System)
        .map(|p| p.code.as_str())
        .collect();
    assert_eq!(system, ["C1001000", "C6001090"]);
    assert!(history.iter().any(|p| p.code == "C2009000"));
    // A host code of progress does not take the current code from the
    // system; a condition of the host's does, and the next progress
    // gives it back.
    m.report(id, "REF C2005160 Partition resumed: PROD", false);
    assert_eq!(status(&mut m, id).code, "C6001090");
    m.report(
        id,
        "REF A2005160 Partition held for storage pool space: PROD",
        false,
    );
    assert_eq!(status(&mut m, id).code, "A2005160");
    m.report(id, "REF C2005160 Partition resumed: PROD", false);
    assert_eq!(status(&mut m, id).code, "C6001090");
    // The code it says is current when the channel comes again is not
    // a second entry.
    m.channel_down(id, Port::Service);
    m.channel_up(id, Port::Service);
    let mut again = code("C6001090");
    if let FromSystem::Refcode(r) = &mut again {
        r.current = true;
    }
    says(&mut m, id, &again);
    let count = m
        .history(id)
        .unwrap()
        .iter()
        .filter(|p| p.code == "C6001090")
        .count();
    assert_eq!(count, 1);
    // The system ends by its own doing: Shutting down, then its VMM
    // ends and it is Not activated with the host's end current.
    says(&mut m, id, &code("D1001000"));
    says(&mut m, id, &sys_state(qs::SysState::Ending));
    assert_eq!(status(&mut m, id).state, State::ShuttingDown);
    m.ended(id, Ended::Exit(0));
    assert_eq!(status(&mut m, id).state, State::NotActivated);
    assert_eq!(status(&mut m, id).code, "C2005100");
}

#[test]
fn a_system_without_a_channel_is_running_after_the_wait_and_refuses_its_shut_downs() {
    let scratch = Scratch::new("none");
    let mut m = scratch.manager();
    let id = partition(&mut m, "OLD", Keylock::Normal);
    m.activate(id, String::new(), None, None).unwrap();
    m.report(id, "REF C2005090 Partition started: OLD, side A", false);
    m.tick();
    assert_eq!(status(&mut m, id).state, State::Starting);
    m.system.now += u64::from(m.host.settings.service.channel_wait_secs);
    m.tick();
    let s = status(&mut m, id);
    assert_eq!(s.state, State::Running);
    assert!(s.service.none && !s.service.up);
    assert_eq!(s.code, "C2005090", "the wait's end is a note");
    // A manager started again: found running is a note too.
    let fake = Fake {
        histories: m.system.histories.clone(),
        ..Fake::default()
    };
    let record = m.system.records[&id].clone();
    let mut again = scratch.manager_with(fake);
    again.adopt(&record, &[]);
    let after = status(&mut again, id);
    assert_eq!(
        (after.state, after.code.as_str()),
        (State::Running, "C2005090")
    );
    assert!(record.no_channel);
    assert!(
        after.service.none,
        "found with no channel, it has none still"
    );
    for kind in [Shutdown::Delayed, Shutdown::System] {
        let Answer::Now(Err(e)) = m.begin_shut_down(id, kind, 10, false) else {
            panic!("not refused");
        };
        assert_eq!(e[0].id, "QVS2301");
        assert!(e[0].text.contains(kind.word()));
    }
    // Immediate is the VMM stopped.
    assert!(matches!(
        m.begin_shut_down(id, Shutdown::Immediate, 0, false),
        Answer::Now(Ok(_))
    ));
    assert_eq!(status(&mut m, id).state, State::ShuttingDown);
}

#[test]
fn a_delayed_shut_down_is_accepted_or_refused_and_forced_when_its_time_passes() {
    let scratch = Scratch::new("delayed");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Normal);
    up(&mut m, id);
    let Answer::Later(rx) = m.begin_shut_down(id, Shutdown::Delayed, 30, true) else {
        panic!("not asked");
    };
    let ToSystem::PowerDown(p) = sent(&m).pop().unwrap() else {
        panic!("not a power down");
    };
    assert_eq!((p.immediate, p.delay, p.restart), (false, 30, true));
    says(&mut m, id, &FromSystem::Ack(qs::Ack { id: p.id }));
    assert_eq!(rx.try_recv().unwrap(), Ok(String::new()));
    assert_eq!(status(&mut m, id).state, State::ShuttingDown);
    // A second one is refused while it goes on.
    assert!(matches!(
        m.begin_shut_down(id, Shutdown::System, 0, false),
        Answer::Now(Err(_))
    ));
    // Its delay and the grace pass: forced, with a message and a code.
    m.system.now += 30 + u64::from(m.host.settings.service.delayed_grace_secs);
    m.tick();
    assert_eq!(m.system.asked.len(), 1);
    assert!(m.system.posted.iter().any(|(c, _)| c == "A2009040"));
    assert_eq!(status(&mut m, id).message.unwrap().id, "QVS2305");
    // Forced with restart: activated again.
    m.ended(id, Ended::Exit(50));
    assert_eq!(status(&mut m, id).state, State::Starting);

    // The system refuses, with its message passed on.
    let other = partition(&mut m, "TEST", Keylock::Normal);
    up(&mut m, other);
    let Answer::Later(rx) = m.begin_shut_down(other, Shutdown::System, 0, false) else {
        panic!("not asked");
    };
    let ToSystem::PowerDown(p) = sent(&m).pop().unwrap() else {
        panic!("not a power down");
    };
    let refused = FromSystem::Refused(qs::Refused {
        id: p.id,
        msgid: "QSI0011".into(),
        text: "A power down is under way.".into(),
    });
    says(&mut m, other, &refused);
    let e = rx.try_recv().unwrap().unwrap_err();
    assert_eq!(e[0].id, "QVS2303");
    assert!(e[0].text.contains("QSI0011"));
    // One not answered times out.
    let Answer::Later(rx) = m.begin_shut_down(other, Shutdown::System, 0, false) else {
        panic!("not asked");
    };
    m.system.now += svcchan::ANSWER_SECS;
    m.tick();
    assert_eq!(rx.try_recv().unwrap().unwrap_err()[0].id, "QVS2304");
    assert!(matches!(
        m.begin_shut_down(other, Shutdown::Delayed, 100_000, false),
        Answer::Now(Err(e)) if e[0].id == "QVS2302"
    ));
}

#[test]
fn a_terminal_code_holds_the_partition_with_its_memory() {
    let scratch = Scratch::new("held");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Manual);
    up(&mut m, id);
    says(&mut m, id, &code("B1001234"));
    says(&mut m, id, &sys_state(qs::SysState::Down));
    assert_eq!(m.system.keep_asked, [id]);
    let s = status(&mut m, id);
    assert_eq!(s.state, State::Error);
    assert!(s.service.kept);
    assert_eq!(s.code, "B1001234", "the terminal code stays current");
    // Its processors are stopped: its silence is not its own.
    for _ in 0..6 {
        m.system.now += 10;
        m.tick();
    }
    let s = status(&mut m, id);
    assert_eq!(s.code, "B1001234", "held, it is not called not responding");
    assert!(!m.history(id).unwrap().iter().any(|p| p.code == "A2009020"));
    // It still holds what it held: no activation, but a dump and an
    // immediate shut down.
    assert!(m.activate(id, String::new(), None, None).is_err());
    assert_eq!(m.held().len(), 1);
    let text = m.panel(id, 34, PanelArgs::default()).unwrap();
    assert!(text.starts_with("34"));
    let (_, order) = m.system.dump_orders.pop().unwrap();
    assert_eq!(order.code, "B1001234");
    assert_eq!(order.reason, "function34");
    m.system.dumps.insert(
        id,
        vec![DumpInfo {
            name: order.name.clone(),
            time: order.time,
            bytes: 1 << 30,
            allocated: 1 << 20,
            ..DumpInfo::default()
        }],
    );
    let (events, seen) = mpsc::channel();
    m.subscribe(events);
    m.dumped(id, &order.name, Ok(()));
    assert!(
        seen.try_iter()
            .any(|e| matches!(e, Event::Dump { error: None, .. }))
    );
    assert_eq!(status(&mut m, id).state, State::Error);
    m.shut_down(id).unwrap();
    m.ended(id, Ended::Exit(50));
    assert_eq!(status(&mut m, id).state, State::NotActivated);

    // A guest fault the VMM keeps.
    m.activate(id, String::new(), None, None).unwrap();
    m.report(
        id,
        "REF A2005170 Partition held after a guest fault: PROD, processor 0: triple fault",
        false,
    );
    let s = status(&mut m, id);
    assert_eq!((s.state, s.code.as_str()), (State::Error, "A2005170"));
    assert!(s.service.kept);
}

#[test]
fn panel_functions_display_select_and_need_the_keylock() {
    let scratch = Scratch::new("panel");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Normal);
    let args = PanelArgs::default();
    assert!(m.panel(id, 1, args).unwrap().contains("keylock normal"));
    assert!(m.panel(id, 20, args).unwrap().contains("serial number"));
    for f in [4, 10, 35, 0, 255] {
        assert_eq!(m.panel(id, f, args).unwrap_err()[0].id, "QVS2306");
    }
    // 02 selects for the next activation.
    let select = PanelArgs {
        source: Some(model::IplSource::B),
        ipl_type: Some(model::IplType::Attended),
        keylock: Some(Keylock::Manual),
    };
    assert!(m.panel(id, 2, select).unwrap().contains("IPL source B"));
    up(&mut m, id);
    let config = &m.system.launched.last().unwrap().2;
    assert!(config.contains("side = \"b\""), "{config}");
    assert!(config.contains("ipl_type = \"attended\""));
    assert!(config.contains("keylock = \"manual\""));
    // The words of the current code.
    assert_eq!(m.panel(id, 11, args).unwrap(), "11  C6001090");
    assert_eq!(m.panel(id, 12, args).unwrap(), "12  00000042");
    assert_eq!(m.panel(id, 19, args).unwrap(), "19  00000000");
    // 21 goes to the system, whose answer comes later.
    assert!(matches!(m.begin_panel(id, 21, args), Answer::Later(_)));
    assert!(matches!(sent(&m).pop(), Some(ToSystem::Panel(p)) if p.function == 21));
    // 22 dumps, then restarts the IPL when the dump is written.
    assert!(m.panel(id, 22, args).unwrap().starts_with("22"));
    let (_, order) = m.system.dump_orders.pop().unwrap();
    assert_eq!(m.panel(id, 22, args).unwrap_err()[0].id, "QVS2313");
    m.dumped(id, &order.name, Ok(()));
    assert_eq!(m.system.asked.len(), 1);
    assert_eq!(m.panel(id, 34, args).unwrap_err()[0].id, "QVS2309");
    // 08 powers off.
    m.ended(id, Ended::Exit(50));
    m.report(id, "REF C2005090 Partition started: X, side A", false);
    m.panel(id, 8, args).unwrap();

    // At the keylock in normal, what interrupts is refused.
    let normal = partition(&mut m, "TEST", Keylock::Normal);
    up(&mut m, normal);
    for f in [3, 8, 21, 22, 34] {
        assert_eq!(
            m.panel(normal, f, args).unwrap_err()[0].id,
            "QVS2307",
            "{f}"
        );
    }
}

#[test]
fn dumps_need_the_repository_and_keep_within_their_limits() {
    let scratch = Scratch::new("dumps");
    let fake = Fake {
        no_repository: true,
        ..Fake::default()
    };
    let mut m = scratch.manager_with(fake);
    let id = partition(&mut m, "PROD", Keylock::Normal);
    up(&mut m, id);
    assert_eq!(m.take_dump(id).unwrap_err()[0].id, "QVS2310");
    m.system.no_repository = false;
    let old: Vec<DumpInfo> = (0..3)
        .map(|i| DumpInfo {
            name: format!("PROD-{i}.core"),
            time: i,
            allocated: 100,
            ..DumpInfo::default()
        })
        .collect();
    m.system.dumps.insert(id, old);
    let name = m.take_dump(id).unwrap();
    m.system.dumps.get_mut(&id).unwrap().push(DumpInfo {
        name: name.clone(),
        time: 10,
        allocated: 100,
        ..DumpInfo::default()
    });
    m.dumped(id, &name, Ok(()));
    let left: Vec<String> = m.dumps(id).unwrap().into_iter().map(|d| d.name).collect();
    assert_eq!(left, ["PROD-1.core", "PROD-2.core", name.as_str()]);
    assert!(m.system.posted.iter().any(|(c, _)| c == "C2009060"));
    // One that fails says why.
    let again = m.take_dump(id).unwrap();
    let (events, seen) = mpsc::channel();
    m.subscribe(events);
    m.dumped(id, &again, Err("no space left".into()));
    assert!(
        seen.try_iter()
            .any(|e| matches!(e, Event::Dump { error: Some(e), .. } if e.id == "QVS2311"))
    );
    assert_eq!(
        m.delete_dump(id, "../../etc".into()).unwrap_err()[0].id,
        "QVS2312"
    );
    m.delete_dump(id, "PROD-1.core".into()).unwrap();
}

fn frame(text: &str) -> qcon::Frame {
    let mut cells: Vec<qcon::Cell> = text.chars().map(|ch| qcon::Cell { ch, attr: 0 }).collect();
    cells.resize(24 * 80, qcon::Cell::default());
    qcon::Frame {
        seq: 1,
        rows: 24,
        cols: 80,
        cells,
        status: qcon::Status::default(),
    }
}

#[test]
fn the_console_is_kept_whole_and_has_one_viewer() {
    let scratch = Scratch::new("console");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Normal);
    up(&mut m, id);
    m.channel_up(id, Port::Console);
    let shown = qcon::FromSystem::Frame(frame("Sign On")).encode().unwrap();
    m.channel_bytes(id, Port::Console, &shown);
    assert!(m.screen(id).unwrap().text.starts_with("Sign On"));
    let (to, from) = mpsc::sync_channel(16);
    let (first, serial) = m.console_attach(id, false, to).unwrap();
    assert!(!serial);
    assert!(matches!(from.try_recv(), Ok(ConsoleOut::Frame { .. })));
    let (to2, _from2) = mpsc::sync_channel(16);
    assert_eq!(
        m.console_attach(id, false, to2).unwrap_err()[0].id,
        "QVS2314"
    );
    // Keys go to the system from the viewer that has it.
    let before = m.system.sent.len();
    assert!(m.console_input(
        id,
        first,
        &ConsoleIn::Key {
            key: qcon::Key::Enter
        }
    ));
    assert_eq!(m.system.sent.len(), before + 1);
    let (to3, from3) = mpsc::sync_channel(16);
    let (second, _) = m.console_attach(id, true, to3).unwrap();
    assert!(matches!(from.try_recv(), Ok(ConsoleOut::Ended { .. })));
    assert!(!m.console_input(id, first, &ConsoleIn::Refresh));
    assert!(matches!(from3.try_recv(), Ok(ConsoleOut::Frame { .. })));
    m.console_detach(id, second);
    // Whatever the system sends that does not fit is dropped and a
    // whole frame asked for.
    let before = m.system.sent.len();
    m.channel_bytes(
        id,
        Port::Console,
        &qsipart::frame::encode(qcon::kind::FRAME, &[1, 2]).unwrap(),
    );
    assert_eq!(m.system.sent.len(), before + 1);
}

#[test]
fn a_silent_system_is_not_responding_and_nonsense_closes_its_channel() {
    let scratch = Scratch::new("silent");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Normal);
    up(&mut m, id);
    let (events, seen) = mpsc::channel();
    m.subscribe(events);
    for _ in 0..svcchan::PING_SECS * svcchan::MISSED {
        m.system.now += 1;
        m.tick();
    }
    assert!(sent(&m).iter().any(|s| matches!(s, ToSystem::Ping(_))));
    assert!(!status(&mut m, id).service.responding);
    assert_eq!(status(&mut m, id).code, "A2009020");
    assert!(seen.try_iter().any(|e| matches!(e, Event::Channel { .. })));
    says(
        &mut m,
        id,
        &FromSystem::Pong(qs::Pong { id: 1, time_ms: 0 }),
    );
    assert!(status(&mut m, id).service.responding);
    assert_eq!(
        status(&mut m, id).code,
        "C6001090",
        "the system's code again"
    );
    assert!(m.history(id).unwrap().iter().any(|p| p.code == "C2009020"));
    // Nothing is done to the partition for either.
    assert_eq!(status(&mut m, id).state, State::Running);

    let bad = qsipart::frame::encode(qs::kind::STATE, &[1]).unwrap();
    for _ in 0..=svcchan::MAX_BAD {
        m.channel_bytes(id, Port::Service, &bad);
    }
    assert_eq!(m.system.closed, [(id, Port::Service)]);
    assert!(!status(&mut m, id).service.up);
    assert_eq!(status(&mut m, id).state, State::Running);
}

#[test]
fn the_lic_record_says_whether_to_wait_for_a_channel() {
    let scratch = Scratch::new("recorded");
    let mut m = scratch.manager();
    let wait = u64::from(m.host.settings.service.channel_wait_secs);
    // It says none: Running once the kernel is entered, no wait.
    let none = partition(&mut m, "NONE", Keylock::Normal);
    m.activate(none, String::new(), None, None).unwrap();
    m.report(none, "CHANNEL none", false);
    m.report(none, "REF C2005090 Partition started: NONE, side A", false);
    m.tick();
    let s = status(&mut m, none);
    assert_eq!(s.state, State::Running);
    assert!(s.service.none);
    // It says it has one: Starting however long it takes.
    let has = partition(&mut m, "HAS", Keylock::Normal);
    m.activate(has, String::new(), None, None).unwrap();
    m.report(has, "CHANNEL has 1", false);
    m.report(has, "REF C2005090 Partition started: HAS, side A", false);
    m.system.now += 10 * wait;
    m.tick();
    let s = status(&mut m, has);
    assert_eq!(s.state, State::Starting);
    assert!(!s.service.none);
    // Nonsense on the line is passed over: the wait is today's.
    let old = partition(&mut m, "OLD", Keylock::Normal);
    m.activate(old, String::new(), None, None).unwrap();
    m.report(old, "CHANNEL has 0", false);
    m.report(old, "REF C2005090 Partition started: OLD, side A", false);
    m.tick();
    assert_eq!(status(&mut m, old).state, State::Starting);
    m.system.now += wait;
    m.tick();
    assert_eq!(status(&mut m, old).state, State::Running);
    // The line is not a code of the history.
    assert!(
        !m.history(none)
            .unwrap()
            .iter()
            .any(|p| p.code.contains("CHANNEL"))
    );
}

#[test]
fn the_history_is_kept_across_managers_and_the_host_s_end_goes_through_the_system() {
    let scratch = Scratch::new("history");
    let mut m = scratch.manager();
    let a = partition(&mut m, "A", Keylock::Normal);
    let b = partition(&mut m, "B", Keylock::Normal);
    up(&mut m, a);
    m.activate(b, String::new(), None, None).unwrap();
    let kept = m.system.histories.clone();
    let fake = Fake {
        histories: kept,
        ..Fake::default()
    };
    let mut again = scratch.manager_with(fake);
    let codes: Vec<String> = again
        .history(a)
        .unwrap()
        .into_iter()
        .map(|p| p.code)
        .collect();
    assert!(codes.contains(&"C1001000".to_string()) && codes.contains(&"C6001090".to_string()));
    assert_eq!(status(&mut again, a).code, codes.last().unwrap().as_str());

    // The host's end: the last activated first, immediately when it has
    // no channel; the one that has one through its system.
    assert_eq!(m.end_all(), 2);
    assert_eq!(m.system.asked.len(), 1, "B, without a channel");
    m.ended(b, Ended::Exit(50));
    let ToSystem::PowerDown(p) = sent(&m).pop().unwrap() else {
        panic!("A was not asked");
    };
    assert!(p.immediate);
    says(&mut m, a, &FromSystem::Ack(qs::Ack { id: p.id }));
    assert_eq!(status(&mut m, a).state, State::ShuttingDown);
    m.system.now += u64::from(m.host.settings.service.host_timeout_secs);
    m.tick();
    assert_eq!(m.system.asked.len(), 2, "forced at the host's timeout");
}

#[test]
fn a_hello_that_differs_from_the_definition_is_said_once() {
    let scratch = Scratch::new("hello");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD", Keylock::Normal);
    m.activate(id, String::new(), None, None).unwrap();
    m.channel_up(id, Port::Service);
    says(&mut m, id, &hello("OTHER01", qs::Keylock::Manual));
    says(&mut m, id, &hello("OTHER01", qs::Keylock::Manual));
    let said: Vec<&(String, String)> = m
        .system
        .posted
        .iter()
        .filter(|(c, _)| c == "A2009030")
        .collect();
    assert_eq!(
        said.len(),
        2,
        "the serial number and the keylock, once each"
    );
    // A time request is answered from the host's clock.
    says(&mut m, id, &FromSystem::TimeRequest);
    assert!(matches!(sent(&m).pop(), Some(ToSystem::Time(_))));
}
