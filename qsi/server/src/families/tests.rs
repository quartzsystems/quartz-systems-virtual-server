// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use std::sync::{Arc, Mutex};

use partd::Request as P;
use qsi::any::{AnyReply, AnyRequest};
use qsi::host::{partition, partitions, profiles, refcodes};

use super::*;

/// A partition manager that answers from a script, and what it was
/// asked.
#[derive(Clone, Default)]
struct Fake {
    asked: Arc<Mutex<Vec<String>>>,
    down: bool,
}

fn posted(n: u64) -> partd::Posted {
    partd::Posted {
        time: n,
        code: format!("C20050{n:02}"),
        text: format!("code {n}"),
        ..partd::Posted::default()
    }
}

impl partd::Calls for Fake {
    fn call(&mut self, req: &P) -> Result<Vec<u8>, ipc::WireError> {
        if self.down {
            return Err(ipc::WireError::Io(std::io::Error::other("no socket")));
        }
        self.asked.lock().unwrap().push(format!("{req:?}"));
        let definition = model::sample::definition();
        Ok(match req {
            P::GetDefinition { id } if *id == definition.id => {
                ipc::encode(&Ok::<_, partd::proto::Refused>(definition))
            }
            P::GetDefinition { id } => ipc::encode(&Err::<model::Definition, _>(vec![
                qvsmsg::PARTITION_MISSING.with(&[&id.to_string()]),
            ])),
            P::Change { .. } => ipc::encode(&Ok::<(), partd::proto::Refused>(())),
            P::PutProfileAt { revision, .. } if *revision == Some(3) => {
                ipc::encode(&Ok::<u64, partd::proto::Refused>(4))
            }
            P::PutProfileAt { profile, .. } => ipc::encode(&Err::<u64, _>(vec![
                qvsmsg::PROFILE_CHANGED.with(&[&profile.name]),
            ])),
            P::History { .. } => ipc::encode(&Ok::<_, partd::proto::Refused>(
                (1..=5).map(posted).collect::<Vec<_>>(),
            )),
            _ => panic!("not scripted: {req:?}"),
        })
    }
}

/// A service with no socket.
#[derive(Clone, Copy, Default)]
struct Gone;

fn gone() -> ipc::WireError {
    ipc::WireError::Io(std::io::Error::other("no socket"))
}

impl poold::Calls for Gone {
    fn call(&mut self, _: &poold::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}

impl problem::proto::Calls for Gone {
    fn call(&mut self, _: &problem::proto::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}
impl vswitchd::Calls for Gone {
    fn call(&mut self, _: &vswitchd::Request) -> Result<Vec<u8>, ipc::WireError> {
        Err(gone())
    }
}

impl Facts for Gone {
    fn inventory(&mut self) -> Result<state::Inventory, Vec<Note>> {
        Err(down("inventory", &gone()))
    }
    fn identity(&mut self) -> Result<qsi::base::Identity, Vec<Note>> {
        Err(down("PID 1", &gone()))
    }
    fn state(&mut self) -> Result<supervise::control::HostState, Vec<Note>> {
        Err(down("PID 1", &gone()))
    }
    fn power(&mut self, _: bool, _: bool, _: u32, _: &str) -> Result<(), Vec<Note>> {
        Err(down("PID 1", &gone()))
    }
    fn history(&mut self) -> Result<Vec<base::Posted>, Vec<Note>> {
        Err(down("log", &gone()))
    }
    fn time(&mut self) -> qsi::base::Time {
        qsi::base::Time {
            unix_ns: 0,
            offset_s: 0,
            zone: "UTC".into(),
        }
    }
}

fn services(fake: &Fake) -> Services {
    Services {
        partd: Box::new(fake.clone()),
        poold: Box::new(Gone),
        vswitchd: Box::new(Gone),
        probd: Box::new(Gone),
        facts: Box::new(Gone),
    }
}

fn answer_as(s: &mut Services, r: AnyRequest) -> Answer {
    answer(s, r, "QSECOFR")
}

fn refused(a: Answer) -> Vec<String> {
    match a {
        Answer::Refused(notes) => notes.into_iter().map(|n| n.msgid).collect(),
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn a_refusal_carries_every_message_and_a_service_down_is_said() {
    let fake = Fake::default();
    let mut s = services(&fake);
    let a = answer_as(
        &mut s,
        AnyRequest::Partitions(partitions::Request::Get(partitions::Get { id: 7 })),
    );
    assert_eq!(refused(a), vec!["QVS2001"]);
    let mut s = services(&Fake {
        down: true,
        ..Fake::default()
    });
    let a = answer_as(
        &mut s,
        AnyRequest::Partition(partition::Request::Restart(partition::Restart { id: 1 })),
    );
    assert_eq!(refused(a), vec![SERVICE_DOWN]);
}

#[test]
fn a_stale_put_is_led_by_qsis_conflict_and_a_good_one_says_its_revision() {
    let fake = Fake::default();
    let mut s = services(&fake);
    let profile = convert::profile(&model::sample::dedicated());
    let put = |revision| {
        AnyRequest::Profiles(profiles::Request::Put(profiles::Put {
            id: 1,
            profile: profile.clone(),
            revision,
        }))
    };
    assert_eq!(
        refused(answer_as(&mut s, put(Some(2)))),
        vec![profiles::msgid::CHANGED, qvsmsg::PROFILE_CHANGED.id]
    );
    assert_eq!(
        answer_as(&mut s, put(Some(3))),
        Answer::Reply(AnyReply::Profiles(profiles::Reply::Put(profiles::Saved {
            revision: 4
        })))
    );
}

#[test]
fn set_default_is_a_change_of_the_definition() {
    let fake = Fake::default();
    let mut s = services(&fake);
    let id = model::sample::definition().id;
    let a = answer_as(
        &mut s,
        AnyRequest::Profiles(profiles::Request::SetDefault(profiles::SetDefault {
            id,
            name: "OTHER".into(),
        })),
    );
    assert!(matches!(a, Answer::Reply(_)), "{a:?}");
    let asked = fake.asked.lock().unwrap();
    assert!(asked[1].contains("default_profile: \"OTHER\""), "{asked:?}");
}

#[test]
fn a_history_is_paged_newest_first_by_place() {
    let all: Vec<partd::Posted> = (1..=5).map(posted).collect();
    let (codes, more) = super::partitions::page(&all, None, 2);
    let places: Vec<u64> = codes.iter().map(|c| c.refcode.seq).collect();
    assert_eq!((places, more), (vec![5, 4], Some(4)));
    let (codes, more) = super::partitions::page(&all, Some(3), 2);
    let places: Vec<u64> = codes.iter().map(|c| c.refcode.seq).collect();
    assert_eq!((places, more), (vec![2, 1], None));
    assert_eq!(codes[0].refcode.time_ms, 2000);

    let fake = Fake::default();
    let mut s = services(&fake);
    let a = answer_as(
        &mut s,
        AnyRequest::Refcodes(refcodes::Request::History(refcodes::History {
            id: 1,
            before: None,
            limit: 0,
        })),
    );
    let Answer::Reply(AnyReply::Refcodes(refcodes::Reply::History(page))) = a else {
        panic!("{a:?}");
    };
    assert_eq!((page.codes.len(), page.more), (1, Some(5)));
}

#[test]
fn streams_are_left_to_the_connection() {
    let fake = Fake::default();
    let mut s = services(&fake);
    let a = answer_as(
        &mut s,
        AnyRequest::Dumps(qsi::host::dumps::Request::Download(
            qsi::host::dumps::Download {
                id: 2,
                name: "D1".into(),
            },
        )),
    );
    assert_eq!(
        a,
        Answer::Open(Open::Dump {
            id: 2,
            name: "D1".into()
        })
    );
    assert!(fake.asked.lock().unwrap().is_empty());
}

#[test]
fn each_item_of_the_hardware_has_its_owner() {
    use qsi::host::host::Owner;
    use state::doc::inventory as inv;
    let disk = |name: &str| inv::Disk {
        name: name.into(),
        location: format!("U-{name}"),
        ..inv::Disk::default()
    };
    let nic = |name: &str| inv::Interface {
        name: name.into(),
        location: format!("U-{name}"),
        ..inv::Interface::default()
    };
    let inventory = state::Inventory {
        disks: vec![disk("sda"), disk("sdb"), disk("sdc"), disk("sdd")],
        interfaces: vec![nic("eth0"), nic("eth1"), nic("eth2")],
        ..state::Inventory::default()
    };
    let seen = |name: &str| poold::Disk {
        name: name.into(),
        ..poold::Disk::default()
    };
    let disks = vec![
        seen("sda"),
        poold::Disk {
            pool: "POOL1".into(),
            ..seen("sdb")
        },
        poold::Disk {
            foreign: true,
            ..seen("sdc")
        },
        poold::Disk {
            free: true,
            ..seen("sdd")
        },
    ];
    let iface = |name: &str| vswitchd::Interface {
        name: name.into(),
        ..vswitchd::Interface::default()
    };
    let interfaces = vec![
        vswitchd::Interface {
            management: true,
            ..iface("eth0")
        },
        vswitchd::Interface {
            switch: "LAN".into(),
            ..iface("eth1")
        },
        iface("eth2"),
    ];
    let h = host::hardware(&inventory, &disks, &interfaces);
    let owners: Vec<(Owner, &str)> = h
        .items
        .iter()
        .map(|i| (i.owner, i.owner_name.as_str()))
        .collect();
    assert_eq!(
        owners,
        vec![
            (Owner::Host, ""),
            (Owner::Pool, "POOL1"),
            (Owner::Foreign, ""),
            (Owner::None, ""),
            (Owner::Host, ""),
            (Owner::Switch, "LAN"),
            (Owner::None, ""),
        ]
    );
}

#[test]
fn what_is_the_hosts_alone_is_left_out() {
    let p = poold::PoolStatus {
        name: "POOL1".into(),
        mount: "/pools/POOL1".into(),
        ..poold::PoolStatus::default()
    };
    let q = vres::pool(&p);
    assert_eq!(q.name, "POOL1");
    assert!(!format!("{q:?}").contains("/pools"));
    let s = vswitchd::SwitchStatus {
        bridge: "qsvsbr1".into(),
        ..vswitchd::SwitchStatus::default()
    };
    assert_eq!(vres::switch(&s), None, "no switch kept, nothing to say");
}

#[test]
fn an_upload_is_left_to_the_connection() {
    let fake = Fake::default();
    let mut s = services(&fake);
    let a = answer_as(
        &mut s,
        AnyRequest::Vres(qsi::host::vres::Request::ImageUpload(
            qsi::host::vres::ImageUpload {
                name: "CD1".into(),
                bytes: 2048,
            },
        )),
    );
    assert_eq!(
        a,
        Answer::Open(Open::Upload {
            name: "CD1".into(),
            bytes: 2048
        })
    );
}

/// A problem service with problem 1 open and holding, and what it was
/// asked to close.
#[derive(Clone, Default)]
struct Problems {
    closed: Arc<Mutex<Vec<(u64, bool, String)>>>,
}

fn open_problem() -> problem::Problem {
    problem::Problem {
        summary: problem::Summary {
            number: 1,
            holds: true,
            count: 2,
            description: "Disk unit 2 of storage pool POOL1 failed or is missing.".into(),
            msgid: "QVS5130".into(),
            refcode: problem::Posted {
                seq: 7,
                code: "A2007020".into(),
                ..problem::Posted::default()
            },
            ..problem::Summary::default()
        },
        resources: vec![
            refcode::about::disk_unit("2", "P0-B3-D0-F0-U1", "S2"),
            refcode::about::service("netd"),
        ],
        log: Vec::new(),
    }
}

impl problem::proto::Calls for Problems {
    fn call(&mut self, req: &problem::proto::Request) -> Result<Vec<u8>, ipc::WireError> {
        use problem::proto::Request as R;
        type Got = Result<problem::Problem, Vec<qvsmsg::Issued>>;
        Ok(match req {
            R::List { .. } => ipc::encode(&problem::Page {
                problems: vec![open_problem().summary],
                more: None,
            }),
            R::Get { number: 1 } => ipc::encode(&Ok::<_, Vec<qvsmsg::Issued>>(open_problem())),
            R::Get { number } => ipc::encode(&Err::<problem::Problem, _>(vec![
                qvsmsg::NO_PROBLEM.with(&[&number.to_string()]),
            ])),
            R::Close { number, anyway, by } => {
                self.closed
                    .lock()
                    .unwrap()
                    .push((*number, *anyway, by.clone()));
                let got: Got = if *anyway {
                    let mut p = open_problem();
                    p.summary.status = problem::Status::Closed;
                    p.summary.closed_by.clone_from(by);
                    Ok(p)
                } else {
                    Err(vec![qvsmsg::PROBLEM_HOLDS.with(&[&number.to_string()])])
                };
                ipc::encode(&got)
            }
            R::Subscribe {} => ipc::encode(&()),
        })
    }
}

#[test]
fn problems_are_the_problem_services_and_a_close_is_the_users() {
    use qsi::service::{Close, Get, List, Reply, Request};
    let fake = Fake::default();
    let probs = Problems::default();
    let mut s = services(&fake);
    s.probd = Box::new(probs.clone());
    let a = answer_as(
        &mut s,
        AnyRequest::Service(Request::List(List {
            filter: qsi::service::Filter::default(),
            before: None,
            limit: 10,
        })),
    );
    let Answer::Reply(AnyReply::Service(Reply::List(page))) = a else {
        panic!("{a:?}")
    };
    assert_eq!(page.problems[0].refcode.code, "A2007020");
    assert_eq!(
        page.problems[0].source.kind,
        qsi::service::SourceKind::System
    );
    let a = answer_as(&mut s, AnyRequest::Service(Request::Get(Get { number: 1 })));
    let Answer::Reply(AnyReply::Service(Reply::Get(p))) = a else {
        panic!("{a:?}")
    };
    assert_eq!(p.resources[0].kind, qsi::service::ResourceKind::DiskUnit);
    assert_eq!(p.resources[0].serial, "S2");
    assert_eq!(
        p.resources[1].name, "service netd",
        "a service is named as one"
    );
    // Refusals QSI names lead with its identifier.
    let a = answer_as(&mut s, AnyRequest::Service(Request::Get(Get { number: 9 })));
    let Answer::Refused(notes) = a else {
        panic!("{a:?}")
    };
    assert_eq!(
        (notes[0].msgid.as_str(), notes[1].msgid.as_str()),
        ("QSI1002", "QVS5001")
    );
    let close = |anyway| AnyRequest::Service(Request::Close(Close { number: 1, anyway }));
    let Answer::Refused(notes) = answer_as(&mut s, close(false)) else {
        panic!()
    };
    assert_eq!(
        (notes[0].msgid.as_str(), notes[1].msgid.as_str()),
        ("QSI1001", "QVS5003")
    );
    let a = answer_as(&mut s, close(true));
    let Answer::Reply(AnyReply::Service(Reply::Close(closed))) = a else {
        panic!("{a:?}")
    };
    assert_eq!(closed.closed_by, "QSECOFR");
    assert_eq!(probs.closed.lock().unwrap().last().unwrap().2, "QSECOFR");
}

#[test]
fn problems_changed_while_the_server_was_away_are_said() {
    use crate::events::said::{self, Said};
    let mut p = open_problem().summary;
    let mut log = Said::default();
    // Never said: opened.
    let e = said::problems(&log, std::slice::from_ref(&p), 1);
    assert!(matches!(
        e[..],
        [qsi::any::AnyEvent::Service(qsi::service::Event::Opened(_))]
    ));
    for e in &e {
        log.note(e);
    }
    assert!(said::problems(&log, std::slice::from_ref(&p), 2).is_empty());
    // Cleared, then closed, while away.
    p.holds = false;
    let e = said::problems(&log, std::slice::from_ref(&p), 3);
    assert!(matches!(
        e[..],
        [qsi::any::AnyEvent::Service(qsi::service::Event::Cleared(_))]
    ));
    p.status = problem::Status::Closed;
    let e = said::problems(&log, std::slice::from_ref(&p), 4);
    assert!(matches!(
        e[..],
        [qsi::any::AnyEvent::Service(qsi::service::Event::Closed(_))]
    ));
}
