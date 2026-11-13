// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager against a machine that only writes down what
//! it was asked.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;

use model::{IplSource, Keylock, Units, sample};
use resource::cgroup::Step;
use resource::{Host, Pool, Processors, Settings, Topology};

use super::{Ended, Manager, Record, System};
use crate::proto::{Event, Partitions, State};
use crate::store::Store;

/// A machine that does nothing and remembers everything.
#[derive(Default)]
pub struct Fake {
    pub now: u64,
    pub applied: Vec<Step>,
    pub removed: Vec<String>,
    pub launched: Vec<(u32, String, String)>,
    pub asked: Vec<u32>,
    pub killed: Vec<String>,
    pub posted: Vec<(String, String)>,
    pub records: BTreeMap<u32, Record>,
    pub log: Vec<String>,
    pub refuse_launch: Option<String>,
    pub missing: Vec<String>,
}

impl System for Fake {
    fn now(&self) -> u64 {
        self.now
    }
    fn apply(&mut self, plan: &[Step]) -> Result<(), String> {
        self.applied.extend_from_slice(plan);
        Ok(())
    }
    fn remove_group(&mut self, group: &str) {
        self.removed.push(group.to_string());
    }
    fn launch(&mut self, id: u32, group: &str, config: &str) -> Result<u32, String> {
        if let Some(why) = &self.refuse_launch {
            return Err(why.clone());
        }
        self.launched
            .push((id, group.to_string(), config.to_string()));
        Ok(1000 + u32::try_from(self.launched.len()).unwrap())
    }
    fn end(&mut self, pid: u32) {
        self.asked.push(pid);
    }
    fn kill(&mut self, group: &str) {
        self.killed.push(group.to_string());
    }
    fn post(&mut self, code: &str, text: &str) {
        self.posted.push((code.to_string(), text.to_string()));
    }
    fn record(&mut self, id: u32, record: Option<&Record>) {
        match record {
            Some(record) => self.records.insert(id, record.clone()),
            None => self.records.remove(&id),
        };
    }
    fn log(&mut self, line: &str) {
        self.log.push(line.to_string());
    }
    fn exists(&self, backing: &str) -> bool {
        !self.missing.iter().any(|m| m == backing)
    }
}

/// Eight cores of two threads and 8 GiB: seven cores and 6144 MiB for
/// partitions.
fn host(settings: Settings) -> Host {
    Host {
        topology: Topology::uniform(8, 2),
        installed_mib: 8192,
        configurable_mib: settings.configurable_mib(8192),
        settings,
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("partd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }

    fn manager(&self) -> Manager<Fake> {
        // As the service does: the settings are what state keeps.
        let store = Store::at(&self.0);
        let settings = store.settings().unwrap();
        let mut manager = Manager::new(Fake::default(), store, host(settings), "HOST01");
        manager.system.now = 1_794_000_000;
        manager.start();
        manager
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A partition called `name` with the two sample profiles, the
/// dedicated one its default.
fn partition(m: &mut Manager<Fake>, name: &str) -> u32 {
    let id = m.create(name.to_string()).unwrap().id;
    let mut dedicated = sample::dedicated();
    dedicated.scsi[0].luns[0].backing = format!("/dev/{name}");
    dedicated.scsi[0].luns[0].serial = None;
    let mut shared = sample::shared();
    shared.scsi = dedicated.scsi.clone();
    m.put_profile(id, dedicated).unwrap();
    m.put_profile(id, shared).unwrap();
    id
}

fn state(m: &mut Manager<Fake>, id: u32) -> State {
    m.list().into_iter().find(|s| s.id == id).unwrap().state
}

fn ids(errors: &[qvsmsg::Issued]) -> Vec<&str> {
    errors.iter().map(|e| e.id.as_str()).collect()
}

/// Bring `id` to Running, as its VMM's report would.
fn run(m: &mut Manager<Fake>, id: u32) {
    m.activate(id, String::new(), None, None).unwrap();
    m.report(id, "REF C2005000 Partition activation started: X", false);
    m.report(id, "REF C2005090 Partition started: X, side A", false);
    assert_eq!(state(m, id), State::Running);
}

#[test]
fn definitions_and_profiles_are_kept_and_a_bad_profile_gives_every_error() {
    let scratch = Scratch::new("kept");
    let mut m = scratch.manager();
    let first = partition(&mut m, "PROD");
    let second = partition(&mut m, "TEST");
    assert_eq!((first, second), (1, 2));
    let definition = m.definition(first).unwrap();
    // The first profile became the default; the serial number is the
    // host's five characters and the ID.
    assert_eq!(definition.default_profile, "NORMAL");
    assert_eq!(
        definition.serial,
        model::definition::serial_for("HOST01", 1)
    );
    assert_eq!(definition.created, 1_794_000_000);
    assert_eq!(m.profiles(first).unwrap(), ["NORMAL", "SHARED"]);

    let mut bad = sample::shared();
    bad.name = "BAD".into();
    bad.processors.minimum = 12;
    bad.memory.desired_mib = 100;
    bad.scsi.clear();
    let errors = m.put_profile(first, bad).unwrap_err();
    assert_eq!(
        ids(&errors),
        [
            qvsmsg::PROCESSORS_ORDER.id,
            qvsmsg::UNITS_TOO_FEW.id,
            qvsmsg::MEMORY_ORDER.id,
            qvsmsg::MEMORY_BLOCK.id,
            qvsmsg::MEMORY_RANGE.id,
            qvsmsg::NO_LOAD_SOURCE.id,
        ]
    );
    assert_eq!(m.profiles(first).unwrap().len(), 2);

    // A manager started later, as after a host restart, has it all.
    let mut again = scratch.manager();
    let listed = again.list();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().all(|s| s.state == State::NotActivated));
    assert_eq!(again.definition(first).unwrap(), definition);
    assert_eq!(
        again.profile(second, "SHARED".into()).unwrap(),
        m.profile(second, "SHARED".into()).unwrap()
    );
    // And the ID of a partition that still exists is not given again.
    assert_eq!(again.create("THIRD".into()).unwrap().id, 3);
    again.delete(second).unwrap();
    assert_eq!(again.create("FOURTH".into()).unwrap().id, 2);
}

#[test]
fn what_cannot_be_created_or_changed_says_why() {
    let scratch = Scratch::new("rules");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    assert_eq!(
        ids(&m.create("prod".into()).unwrap_err()),
        [qvsmsg::NAME_INVALID.id]
    );
    assert_eq!(
        ids(&m.create("PROD".into()).unwrap_err()),
        [qvsmsg::NAME_IN_USE.id]
    );
    assert_eq!(
        ids(&m.definition(9).unwrap_err()),
        [qvsmsg::PARTITION_MISSING.id]
    );
    let other = partition(&mut m, "TEST");

    let mut changed = m.definition(id).unwrap();
    changed.name = "TEST".into();
    changed.serial = "AAAAAAA".into();
    changed.default_profile = "NOSUCH".into();
    changed.created += 1;
    assert_eq!(
        ids(&m.change(changed).unwrap_err()),
        [
            qvsmsg::FIELD_FIXED.id,
            qvsmsg::FIELD_FIXED.id,
            qvsmsg::NAME_IN_USE.id,
            qvsmsg::DEFAULT_PROFILE_MISSING.id,
        ]
    );
    let mut changed = m.definition(id).unwrap();
    changed.name = "LIVE".into();
    changed.keylock = Keylock::Manual;
    changed.default_profile = "SHARED".into();
    changed.auto_start.enabled = true;
    m.change(changed.clone()).unwrap();
    assert_eq!(m.definition(id).unwrap(), changed);
    assert_eq!(
        ids(&m.delete_profile(id, "SHARED".into()).unwrap_err()),
        [qvsmsg::PROFILE_IS_DEFAULT.id]
    );
    m.delete_profile(id, "NORMAL".into()).unwrap();
    assert_eq!(
        ids(&m.profile(id, "NORMAL".into()).unwrap_err()),
        [qvsmsg::PROFILE_MISSING.id]
    );

    // While it is activated nothing of its definition changes.
    run(&mut m, id);
    assert_eq!(
        ids(&m.change(changed).unwrap_err()),
        [qvsmsg::PARTITION_ACTIVE.id]
    );
    assert_eq!(
        ids(&m.delete(id).unwrap_err()),
        [qvsmsg::PARTITION_ACTIVE.id]
    );
    assert_eq!(
        ids(&m.activate(id, String::new(), None, None).unwrap_err()),
        [qvsmsg::PARTITION_ACTIVE.id]
    );
    assert_eq!(
        ids(&m.shut_down(other).unwrap_err()),
        [qvsmsg::PARTITION_NOT_ACTIVE.id]
    );

    // The limit is the setting's.
    let mut settings = m.settings();
    settings.max_partitions = 2;
    m.change_settings(settings).unwrap();
    assert_eq!(
        ids(&m.create("THIRD".into()).unwrap_err()),
        [qvsmsg::TOO_MANY_PARTITIONS.id]
    );
}

#[test]
fn three_partitions_run_together_and_a_fourth_without_memory_is_refused_whole() {
    let scratch = Scratch::new("three");
    let mut m = scratch.manager();
    let (events, received) = mpsc::channel();
    m.subscribe(events);
    let mut ids_of = Vec::new();
    for name in ["A", "B", "C"] {
        let id = partition(&mut m, name);
        run(&mut m, id);
        ids_of.push(id);
    }
    let resources = m.resources();
    // Three of two cores and 2048 MiB each.
    assert_eq!(resources.processors.available, 100);
    assert_eq!(resources.memory.available, 0);
    assert_eq!(m.system.launched.len(), 3);
    assert_eq!(m.system.launched[0].1, "qsvs/dedicated/p1");
    assert!(m.system.launched[0].2.contains("pin = [6, 14, 7, 15]"));
    assert!(m.system.launched[1].2.contains("pin = [4, 12, 5, 13]"));
    assert!(m.system.launched[0].2.contains("huge_pages = true"));
    assert_eq!(m.system.records.len(), 3);

    let fourth = partition(&mut m, "D");
    let applied = m.system.applied.len();
    let errors = m.activate(fourth, String::new(), None, None).unwrap_err();
    assert_eq!(ids(&errors), [qvsmsg::MEMORY_NOT_AVAILABLE.id]);
    assert_eq!(
        errors[0].text,
        "Partition D needs 1024 MB of memory; 0 MB are available."
    );
    // Nothing is left behind: no group, no VMM, no record, no state.
    assert_eq!(m.system.applied.len(), applied);
    assert_eq!(m.system.launched.len(), 3);
    assert_eq!(m.system.records.len(), 3);
    let status = m.list().into_iter().find(|s| s.id == fourth).unwrap();
    assert_eq!(status.state, State::NotActivated);
    assert_eq!(status.current, None);
    assert_eq!(status.message, Some(errors[0].clone()));
    assert_eq!(status.code, refcode::PARTITION_NOT_ACTIVATED.code);

    // Every state change, code and resource change was published.
    let seen: Vec<Event> = received.try_iter().collect();
    let changes = seen
        .iter()
        .filter(|e| matches!(e, Event::State { .. }))
        .count();
    assert_eq!(changes, 6);
    assert!(seen.iter().any(
        |e| matches!(e, Event::Resources { id, current: Some(c), .. } if *id == 2 && c.memory_mib == 2048)
    ));
    assert!(seen.iter().any(
        |e| matches!(e, Event::Code { id, posted, .. } if *id == fourth && posted.code == "A2006010")
    ));
}

#[test]
fn an_activation_takes_what_there_is_when_the_desired_is_not_there() {
    let scratch = Scratch::new("less");
    let mut m = scratch.manager();
    let first = partition(&mut m, "A");
    let second = partition(&mut m, "B");
    let third = partition(&mut m, "C");
    run(&mut m, first);
    run(&mut m, second);
    // 2048 MiB are left; the third wants 4096 and takes 1024 at least.
    let mut wants = m.profile(third, "NORMAL".into()).unwrap();
    wants.memory.desired_mib = 4096;
    wants.processors.desired = 4;
    m.put_profile(third, wants).unwrap();
    let got = m.activate(third, "NORMAL".into(), None, None).unwrap();
    assert_eq!(got.memory_mib, 2048);
    assert_eq!(
        got.processors,
        Processors::Dedicated {
            cores: vec![1, 2, 3]
        }
    );
    let status = m.list().into_iter().find(|s| s.id == third).unwrap();
    assert_eq!(status.current, Some(got));
    assert_eq!(status.state, State::Starting);
    assert!(m.system.launched[2].2.contains("memory_mib = 2048"));
    assert!(m.system.launched[2].2.contains("vcpus = 6"));
}

#[test]
fn each_end_of_a_vmm_leaves_the_partition_in_its_documented_state() {
    let scratch = Scratch::new("ends");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");

    // The guest powers off.
    run(&mut m, id);
    m.ended(id, Ended::Exit(0));
    let status = m.list().remove(0);
    assert_eq!(status.state, State::NotActivated);
    assert_eq!((status.current, status.pid), (None, 0));
    assert_eq!(status.last_end, "exit 0");
    assert_eq!(status.code, "C2005100");
    assert_eq!(status.text, "Partition powered off: PROD, exit 0");
    assert_eq!(m.system.removed.last().unwrap(), "qsvs/dedicated/p1");
    assert!(m.system.records.is_empty());

    // The guest asks for a restart: activated again at once, with
    // what it held, from the side it asked for.
    m.activate(id, String::new(), Some(IplSource::B), None)
        .unwrap();
    m.report(id, "REF C2005090 Partition started: PROD, side B", false);
    let held = m.list().remove(0).current;
    m.ended(id, Ended::Exit(10));
    let status = m.list().remove(0);
    assert_eq!(status.state, State::Starting);
    assert_eq!(status.current, held);
    let (_, _, config) = m.system.launched.last().unwrap();
    assert!(config.contains("side = \"recorded\""), "{config}");
    assert_eq!(m.system.launched.len(), 3);

    // Shut down by the partition manager.
    m.report(id, "REF C2005090 Partition started: PROD, side A", false);
    m.shut_down(id).unwrap();
    assert_eq!(state(&mut m, id), State::ShuttingDown);
    assert_eq!(m.system.asked.last(), Some(&1003));
    assert_eq!(
        ids(&m.shut_down(id).unwrap_err()),
        [qvsmsg::PARTITION_ENDING.id]
    );
    m.ended(id, Ended::Exit(50));
    assert_eq!(state(&mut m, id), State::NotActivated);
    assert_eq!(m.system.launched.len(), 3);

    // Restarted by the partition manager: ended, then activated anew.
    run(&mut m, id);
    m.restart(id).unwrap();
    assert_eq!(state(&mut m, id), State::ShuttingDown);
    m.ended(id, Ended::Exit(50));
    assert_eq!(state(&mut m, id), State::Starting);
    assert_eq!(m.system.launched.len(), 5);

    // A VMM that does not end when asked is killed after a while.
    m.shut_down(id).unwrap();
    m.tick();
    assert!(m.system.killed.is_empty());
    m.system.now += super::END_GRACE_SECS;
    m.tick();
    assert_eq!(m.system.killed, ["qsvs/dedicated/p1"]);
    m.ended(id, Ended::Signal(9));
    assert_eq!(state(&mut m, id), State::NotActivated);
}

#[test]
fn a_vmm_that_fails_is_an_error_and_one_that_cannot_ipl_is_not_activated() {
    let scratch = Scratch::new("errors");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    let other = partition(&mut m, "TEST");
    run(&mut m, other);

    // Killed while it ran.
    run(&mut m, id);
    m.ended(id, Ended::Signal(9));
    let status = m.list().remove(0);
    assert_eq!(status.state, State::Error);
    assert_eq!(status.code, "A2005150");
    assert_eq!(status.last_end, "signal 9");
    assert_eq!(status.current, None);
    let message = status.message.unwrap();
    assert!(message.is(&qvsmsg::VMM_FAILED));
    assert!(message.text.contains("A2005150") && message.text.contains("signal 9"));
    // Nothing started it again, and the other partition is as it was.
    assert_eq!(m.system.launched.len(), 2);
    assert_eq!(state(&mut m, other), State::Running);
    assert_eq!(m.resources().processors.available, 500);
    // It activates again afterwards.
    run(&mut m, id);

    // A guest fault and a VMM error are errors too.
    m.ended(id, Ended::Exit(20));
    assert_eq!(state(&mut m, id), State::Error);
    run(&mut m, id);
    m.ended(id, Ended::Exit(101));
    assert_eq!(m.list().remove(0).code, "A2005130");
    // One that is gone without a word was killed.
    run(&mut m, id);
    m.ended(id, Ended::Unknown);
    assert_eq!(m.list().remove(0).last_end, "signal ?");
    assert_eq!(state(&mut m, id), State::Error);

    // It could not IPL: the code before the end says why.
    m.activate(id, String::new(), None, None).unwrap();
    m.report(id, "REF C2005000 Partition activation started: PROD", false);
    m.report(
        id,
        "REF A2005010 Partition disk unit cannot be read: PROD, slot 2 unit 0",
        false,
    );
    m.ended(id, Ended::Exit(30));
    let status = m.list().remove(0);
    assert_eq!(status.state, State::NotActivated);
    assert_eq!(status.code, "A2005100");
    let message = status.message.unwrap();
    assert!(message.is(&qvsmsg::DID_NOT_IPL));
    assert!(message.text.contains("A2005010"), "{message}");

    // A backing store that is not there: refused by name, before
    // anything is started.
    m.system.missing.push("/dev/PROD".into());
    let launched = m.system.launched.len();
    let errors = m.activate(id, String::new(), None, None).unwrap_err();
    assert_eq!(ids(&errors), [qvsmsg::BACKING_MISSING.id]);
    assert!(errors[0].text.contains("/dev/PROD") && errors[0].text.contains("slot 2"));
    assert_eq!(m.system.launched.len(), launched);
    m.system.missing.clear();

    // The VMM could not be started at all.
    m.system.refuse_launch = Some("no such file".into());
    let errors = m.activate(id, String::new(), None, None).unwrap_err();
    assert_eq!(ids(&errors), [qvsmsg::VMM_NOT_STARTED.id]);
    assert_eq!(state(&mut m, id), State::NotActivated);
    assert_eq!(m.resources().processors.available, 500);
    assert_eq!(m.system.records.len(), 1);
}

#[test]
fn a_report_is_bounded_and_only_a_code_of_the_table_is_taken() {
    let scratch = Scratch::new("report");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    m.activate(id, String::new(), None, None).unwrap();
    let posted = m.system.posted.len();
    m.report(id, "REF C1001000 IPL started", false);
    m.report(id, "REF Z9999999 not a code", false);
    m.report(id, "qsvmm: unit 0 reads 1 writes 2", false);
    m.report(id, &"x".repeat(100_000), false);
    assert_eq!(m.system.posted.len(), posted);
    assert_eq!(state(&mut m, id), State::Starting);
    assert!(m.system.log.iter().all(|l| l.len() < 600));
    // The history is bounded too.
    for _ in 0..200 {
        m.report(id, "REF C2005020 Partition load-source table read", false);
    }
    assert_eq!(m.history(id).unwrap().len(), super::HISTORY);
}

#[test]
fn the_keylock_is_the_definition_s_unless_the_activation_says() {
    let scratch = Scratch::new("keylock");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    let mut definition = m.definition(id).unwrap();
    definition.keylock = Keylock::Manual;
    m.change(definition).unwrap();
    m.activate(id, String::new(), None, None).unwrap();
    let config = &m.system.launched[0].2;
    assert!(config.contains("keylock = \"manual\""), "{config}");
    assert!(config.contains("side = \"recorded\""));
    assert!(config.contains("ipl_type = \"unattended\""));
    assert!(config.contains("console = \"serial\""));
    m.ended(id, Ended::Exit(0));
    m.activate(
        id,
        "SHARED".into(),
        Some(IplSource::A),
        Some(Keylock::Secure),
    )
    .unwrap();
    let (_, group, config) = &m.system.launched[1];
    assert_eq!(group, "qsvs/pools/DEFAULT/p1");
    assert!(config.contains("keylock = \"secure\"") && config.contains("side = \"a\""));
    assert!(config.contains("vcpus = 4") && !config.contains("pin"));
    assert_eq!(m.list().remove(0).profile, "SHARED");
    // The sections of later phases, and IPL source D, are refused by
    // the phase that brings them.
    m.ended(id, Ended::Exit(0));
    let refused = m
        .activate(id, String::new(), Some(IplSource::D), None)
        .unwrap_err();
    assert!(refused[0].is(&qvsmsg::NOT_YET) && refused[0].text.contains("Phase 5"));
    let mut later = m.profile(id, "NORMAL".into()).unwrap();
    later.name = "NET".into();
    later.ethernet.push(model::EthernetAdapter {
        slot: 9,
        vlan: 1,
        mac: None,
    });
    m.put_profile(id, later).unwrap();
    let refused = m.activate(id, "NET".into(), None, None).unwrap_err();
    assert!(refused[0].text.contains("Phase 6"), "{}", refused[0]);
    assert_eq!(
        ids(&m.activate(id, "NOSUCH".into(), None, None).unwrap_err()),
        [qvsmsg::PROFILE_MISSING.id]
    );
}

#[test]
fn pools_are_created_changed_and_deleted_while_nothing_runs_in_them() {
    let scratch = Scratch::new("pools");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    let pool = |cores: &[u32], max: Option<u32>| Pool {
        name: "BATCH".into(),
        cores: cores.to_vec(),
        maximum_units: max.map(Units),
    };
    m.create_pool(pool(&[6, 7], Some(150))).unwrap();
    assert_eq!(
        ids(&m.create_pool(pool(&[5], None)).unwrap_err()),
        [qvsmsg::POOL_EXISTS.id]
    );
    assert_eq!(
        ids(&m.change_pool(pool(&[0, 9], None)).unwrap_err()),
        [qvsmsg::POOL_CORE_INVALID.id; 2]
    );
    let resources = m.resources();
    assert_eq!(resources.pools.len(), 2);
    assert_eq!(resources.pools[0].cores, [1, 2, 3, 4, 5]);
    assert_eq!(resources.pools[1].maximum, Units(150));

    // A partition in the pool: the pool is in use.
    let mut shared = m.profile(id, "SHARED".into()).unwrap();
    shared.processors.pool = Some("BATCH".into());
    m.put_profile(id, shared).unwrap();
    let got = m.activate(id, "SHARED".into(), None, None).unwrap();
    assert!(matches!(
        got.processors,
        Processors::Shared {
            units: Units(150),
            virtuals: 2,
            ..
        }
    ));
    assert_eq!(m.system.launched[0].1, "qsvs/pools/BATCH/p1");
    assert_eq!(
        ids(&m.delete_pool("BATCH".into()).unwrap_err()),
        [qvsmsg::POOL_IN_USE.id]
    );
    assert_eq!(
        ids(&m.change_pool(pool(&[6], None)).unwrap_err()),
        [qvsmsg::POOL_IN_USE.id]
    );
    m.ended(id, Ended::Exit(0));
    m.change_pool(pool(&[7], None)).unwrap();
    m.delete_pool("BATCH".into()).unwrap();
    assert_eq!(m.system.removed.last().unwrap(), "qsvs/pools/BATCH");
    assert_eq!(
        ids(&m.delete_pool("BATCH".into()).unwrap_err()),
        [qvsmsg::POOL_MISSING.id]
    );
    assert_eq!(
        ids(&m.delete_pool("DEFAULT".into()).unwrap_err()),
        [qvsmsg::POOL_DEFAULT.id]
    );
    // The pools are kept with the settings.
    m.create_pool(pool(&[4, 5], None)).unwrap();
    let mut again = scratch.manager();
    assert_eq!(again.settings().pools, [pool(&[4, 5], None)]);

    // A pool may not take the cores the default pool's partitions need.
    let mut shared = again.profile(id, "SHARED".into()).unwrap();
    shared.processors.pool = None;
    shared.processors.desired = 4;
    shared.processors.desired_units = Some(Units(400));
    again.put_profile(id, shared).unwrap();
    again.activate(id, "SHARED".into(), None, None).unwrap();
    let wide = Pool {
        name: "WIDE".into(),
        cores: vec![1, 2],
        maximum_units: None,
    };
    assert_eq!(
        ids(&again.create_pool(wide).unwrap_err()),
        [qvsmsg::POOL_CORES_NEEDED.id]
    );
}

#[test]
fn the_host_s_shutdown_ends_the_partitions_last_activated_first() {
    let scratch = Scratch::new("endall");
    let mut m = scratch.manager();
    let a = partition(&mut m, "A");
    let b = partition(&mut m, "B");
    let c = partition(&mut m, "C");
    run(&mut m, b);
    run(&mut m, a);
    run(&mut m, c);
    m.ended(c, Ended::Exit(0));
    // Activated in the order B, A: ended in the order A, B.
    assert_eq!(m.end_all(), 2);
    assert_eq!(m.system.asked, [1002]);
    assert_eq!(state(&mut m, a), State::ShuttingDown);
    assert_eq!(state(&mut m, b), State::Running);
    // Nothing is activated from now on, and a guest's restart is not
    // followed.
    assert_eq!(
        ids(&m.activate(c, String::new(), None, None).unwrap_err()),
        [qvsmsg::HOST_STOPPING.id]
    );
    m.ended(a, Ended::Exit(50));
    assert_eq!(m.system.asked, [1002, 1001]);
    assert_eq!(state(&mut m, b), State::ShuttingDown);
    m.ended(b, Ended::Exit(10));
    assert!(m.list().iter().all(|s| s.state == State::NotActivated));
    assert_eq!(m.system.launched.len(), 3);
    assert_eq!(m.end_all(), 0);
}

#[test]
fn a_manager_started_later_takes_up_the_partitions_that_run() {
    let scratch = Scratch::new("adopt");
    let mut m = scratch.manager();
    let a = partition(&mut m, "A");
    let b = partition(&mut m, "B");
    run(&mut m, a);
    m.activate(b, "SHARED".into(), None, None).unwrap();
    let records: Vec<Record> = m.system.records.values().cloned().collect();
    let before = m.list();
    let resources = m.resources();

    // The manager ends; its partitions do not.
    let mut again = scratch.manager();
    assert!(again.list().iter().all(|s| s.state == State::NotActivated));
    let posted = again.system.posted.len();
    again.adopt(
        &records[0],
        &[
            "REF C2005000 Partition activation started: A".to_string(),
            "REF C2005090 Partition started: A, side A".to_string(),
        ],
    );
    again.adopt(&records[1], &[]);
    let after = again.list();
    assert_eq!(after[0].state, State::Running);
    assert_eq!(after[1].state, State::Starting);
    assert_eq!(after[0].current, before[0].current);
    assert_eq!(after[1].current, before[1].current);
    assert_eq!((after[0].pid, after[1].pid), (before[0].pid, before[1].pid));
    assert_eq!(again.resources(), resources);
    // Only that they were found is posted: their codes were before.
    let new: Vec<&str> = again.system.posted[posted..]
        .iter()
        .map(|(code, _)| code.as_str())
        .collect();
    assert_eq!(new, ["C2006010", "C2006010"]);
    // They are watched again: one goes on to Running, both end.
    again.report(b, "REF C2005090 Partition started: B, side A", false);
    assert_eq!(state(&mut again, b), State::Running);
    again.ended(a, Ended::Exit(0));
    again.ended(b, Ended::Unknown);
    assert_eq!(state(&mut again, a), State::NotActivated);
    assert_eq!(state(&mut again, b), State::Error);
    // The next activation's place is after the ones taken up.
    let c = partition(&mut again, "C");
    again.activate(c, "SHARED".into(), None, None).unwrap();
    assert!(again.system.records[&c].sequence > records[1].sequence);
}

#[test]
fn auto_start_is_in_the_order_of_the_definitions() {
    let scratch = Scratch::new("auto");
    let mut m = scratch.manager();
    for (name, enabled, order, delay) in [
        ("A", true, 20, 5),
        ("B", false, 1, 0),
        ("C", true, 10, 0),
        ("D", true, 10, 3),
    ] {
        let id = partition(&mut m, name);
        let mut definition = m.definition(id).unwrap();
        definition.auto_start = model::AutoStart {
            enabled,
            order,
            delay_secs: delay,
        };
        m.change(definition).unwrap();
    }
    assert_eq!(
        m.auto_start(),
        [
            (3, "C".to_string(), 0),
            (4, "D".to_string(), 3),
            (1, "A".to_string(), 5)
        ]
    );
}
