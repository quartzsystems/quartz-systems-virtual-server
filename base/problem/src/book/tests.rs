// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

use super::*;
use crate::table::sample::RULES;

fn about(kind: &str, name: &str, location: &str, serial: &str) -> About {
    About {
        kind: kind.into(),
        name: name.into(),
        location: location.into(),
        serial: serial.into(),
        ..About::default()
    }
}

fn partition(id: u32, name: &str) -> About {
    About {
        kind: kind::PARTITION.into(),
        id,
        name: name.into(),
        ..About::default()
    }
}

fn posting(seq: u64, code: &str, text: &str, about: Vec<About>) -> Posting {
    Posting {
        posted: Posted {
            seq,
            code: code.into(),
            words: Vec::new(),
            text: text.into(),
            time_ms: seq * 1000,
        },
        about,
    }
}

fn pulled(seq: u64, serial: &str) -> Posting {
    posting(
        seq,
        "A2007020",
        "Pool unit failed: POOL1, unit 2",
        vec![
            about(kind::DISK_UNIT, "DD002", "U3-P1-D2", serial),
            about(kind::STORAGE_POOL, "POOL1", "", ""),
        ],
    )
}

fn rebuilt(seq: u64) -> Posting {
    posting(
        seq,
        "C2007035",
        "Pool rebuild finished: POOL1",
        vec![about(kind::STORAGE_POOL, "POOL1", "", "")],
    )
}

fn notices(n: &[Noticed]) -> Vec<(Notice, u64)> {
    n.iter().map(|e| (e.notice, e.problem.number)).collect()
}

#[test]
fn a_pulled_pool_disk_opens_one_problem_and_its_rebuild_clears_it() {
    let mut b = Book::default();
    let n = b.take(&RULES, &pulled(1, "S1"), 10);
    assert_eq!(notices(&n), [(Notice::Opened, 1)]);
    let p = b.get(1).unwrap();
    assert_eq!(
        p.summary.description,
        "Disk unit DD002 of storage pool POOL1 failed or is missing."
    );
    assert_eq!(p.summary.msgid, "XMP5130");
    assert_eq!(p.summary.refcode.code, "A2007020");
    assert_eq!(p.summary.source, Source::host());
    assert_eq!(p.resources.len(), 2);
    assert_eq!(p.resources[0].location, "U3-P1-D2");
    // Seen again at the next IPL: counted, not opened again.
    let n = b.take(&RULES, &pulled(2, "S1"), 20);
    assert_eq!(notices(&n), [(Notice::Seen, 1)]);
    assert_eq!(b.get(1).unwrap().summary.count, 2);
    // Replaced: the rebuild's end clears it, and it stays open.
    let n = b.take(&RULES, &rebuilt(3), 30);
    assert_eq!(notices(&n), [(Notice::Cleared, 1)]);
    let s = b.get(1).unwrap().summary;
    assert!(!s.holds && s.status == Status::Open);
    // The new disk pulled: a second problem.
    let n = b.take(&RULES, &pulled(4, "S2"), 40);
    assert_eq!(notices(&n), [(Notice::Opened, 2)]);
    // Another pool's rebuild clears neither.
    let other = posting(
        5,
        "C2007035",
        "",
        vec![about(kind::STORAGE_POOL, "POOL2", "", "")],
    );
    assert!(b.take(&RULES, &other, 50).is_empty());
}

#[test]
fn a_problem_is_closed_by_a_person() {
    let mut b = Book::default();
    b.take(&RULES, &pulled(1, "S1"), 10);
    let refused = b.close(&RULES, 1, false, "ADMIN", 11).unwrap_err();
    assert_eq!(refused[0].id, "XMP5003");
    let closed = b.close(&RULES, 1, true, "ADMIN", 12).unwrap();
    assert_eq!(closed.notice, Notice::Closed);
    assert_eq!(
        (closed.problem.closed_by.as_str(), closed.problem.closed_at),
        ("ADMIN", 12)
    );
    assert!(
        b.get(1)
            .unwrap()
            .log
            .last()
            .unwrap()
            .text
            .contains("while its condition held")
    );
    assert_eq!(
        b.close(&RULES, 1, true, "X", 13).unwrap_err()[0].id,
        "XMP5002"
    );
    assert_eq!(
        b.close(&RULES, 9, true, "X", 13).unwrap_err()[0].id,
        "XMP5001"
    );
    // Seen again once closed: a new problem.
    let n = b.take(&RULES, &pulled(2, "S1"), 20);
    assert_eq!(notices(&n), [(Notice::Opened, 2)]);
    // A cleared one closes without leave.
    b.take(&RULES, &rebuilt(3), 30);
    assert!(b.close(&RULES, 2, false, "", 31).is_ok());
    assert_eq!(b.get(2).unwrap().summary.closed_by, "unknown");
}

#[test]
fn a_partitions_problem_is_its_own() {
    let mut b = Book::default();
    let quiet = |seq, id, name| {
        posting(
            seq,
            "A2009020",
            &format!("Partition not responding: {name}"),
            vec![partition(id, name)],
        )
    };
    b.take(&RULES, &quiet(1, 1, "PROD"), 10);
    b.take(&RULES, &quiet(2, 2, "TEST"), 10);
    assert_eq!(b.open_count(), 2);
    assert_eq!(b.get(1).unwrap().summary.source.name, "PROD");
    assert_eq!(
        b.get(1).unwrap().summary.description,
        "Partition PROD is not responding on its service channel."
    );
    // PROD answers again: only its problem is cleared.
    let back = posting(
        3,
        "C2009020",
        "Partition responding again: PROD",
        vec![partition(1, "PROD")],
    );
    assert_eq!(notices(&b.take(&RULES, &back, 20)), [(Notice::Cleared, 1)]);
    // The system's own terminal code is the problem's.
    let terminal = posting(
        4,
        "A2009055",
        "Partition held on a terminal code: PROD, B6001234",
        vec![
            partition(1, "PROD"),
            About {
                kind: kind::SYSTEM_CODE.into(),
                name: "B6001234".into(),
                ..About::default()
            },
        ],
    );
    b.take(&RULES, &terminal, 30);
    let p = b.get(3).unwrap();
    assert_eq!(p.summary.refcode.code, "B6001234");
    assert_eq!(p.resources.len(), 1, "the system's code is no resource");
    let page = b.list(
        &Filter {
            partition: Some(1),
            ..Filter::default()
        },
        None,
        10,
    );
    let numbers: Vec<u64> = page.problems.iter().map(|s| s.number).collect();
    assert_eq!(numbers, [3, 1]);
}

#[test]
fn a_code_that_opens_nothing_opens_nothing() {
    let mut b = Book::default();
    assert!(
        b.take(
            &RULES,
            &posting(1, "C2001000", "Host IPL started", Vec::new()),
            1
        )
        .is_empty()
    );
    assert!(
        b.take(
            &RULES,
            &posting(2, "A2005000", "Partition profile not valid", Vec::new()),
            1
        )
        .is_empty()
    );
    assert_eq!(b.open_count(), 0);
}

#[test]
fn a_list_is_paged_newest_first() {
    let mut b = Book::default();
    for n in 1..=5 {
        b.take(&RULES, &pulled(n, &format!("S{n}")), n);
    }
    let p = b.list(&Filter::default(), None, 2);
    assert_eq!(
        (
            p.problems.iter().map(|s| s.number).collect::<Vec<_>>(),
            p.more
        ),
        (vec![5, 4], Some(4))
    );
    let p = b.list(&Filter::default(), p.more, 2);
    assert_eq!(p.problems[0].number, 3);
    let p = b.list(
        &Filter {
            since: Some(5),
            ..Filter::default()
        },
        None,
        9,
    );
    assert_eq!(p.problems.len(), 1);
}

#[test]
fn the_log_and_its_place_are_kept_together() {
    let d = std::env::temp_dir().join(format!("problem-book-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    let mut b = Book::default();
    b.take(&RULES, &pulled(1, "S1"), 10);
    b.read_to(Cursor {
        offset: 400,
        seq: 7,
    });
    b.keep(&d).unwrap();
    let back = Book::open(&d);
    assert_eq!(back, b);
    assert_eq!(back.cursor().seq, 7);
    // Numbers go on after a start.
    let mut back = back;
    let n = back.take(&RULES, &pulled(8, "S9"), 20);
    assert_eq!(n[0].problem.number, 2);
    // A damaged file is put aside.
    std::fs::write(d.join(FILE), "{").unwrap();
    assert_eq!(Book::open(&d), Book::default());
    assert!(d.join(format!("{FILE}.bad")).exists());
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn the_log_is_bounded() {
    let mut b = Book::default();
    for n in 0..(MAX_PROBLEMS as u64 + 10) {
        b.take(&RULES, &pulled(n, &format!("S{n}")), n);
    }
    // Past the bound with none closed: the oldest open went.
    assert_eq!(b.problems.len(), MAX_PROBLEMS);
    assert!(b.get(10).is_none() && b.get(11).is_some());
    // With some closed, the oldest closed go first.
    for n in 20..25 {
        b.close(&RULES, n, true, "X", 1).unwrap();
    }
    b.take(&RULES, &pulled(9999, "LAST"), 1);
    assert_eq!(b.problems.len(), MAX_PROBLEMS);
    assert!(b.get(11).is_some() && b.get(20).is_none());
    let mut log = Vec::new();
    for n in 0..(MAX_LINES as u64 + 5) {
        push_line(&mut log, n, n.to_string());
    }
    assert_eq!(log.len(), MAX_LINES);
    assert_eq!(log[0].text, "0", "the first line stays");
}
