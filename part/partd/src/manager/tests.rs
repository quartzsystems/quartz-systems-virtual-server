// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The partition manager against a machine that only writes down what
//! it was asked.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;

use model::{IplSource, Keylock, Units, sample};
use poold::proto::{Given, Want};
use qvsmsg::Issued;
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
    pub held: Vec<(u32, Want)>,
    pub controls: Vec<(u32, String)>,
    pub control_answer: Option<String>,
    pub refuse_hold: Option<Issued>,
    pub refuse_host_paths: bool,
    pub deleted: Vec<(String, String)>,
    pub switches: Option<Vec<vswitch::Switch>>,
    pub ports: Vec<(u32, vswitch::port::PortSpec)>,
    pub refuse_port: Option<Issued>,
    pub reconciled: Vec<Vec<u32>>,
    pub sent: Vec<(u32, super::Port, Vec<u8>)>,
    pub closed: Vec<(u32, super::Port)>,
    pub keep_asked: Vec<u32>,
    pub no_repository: bool,
    pub dump_orders: Vec<(u32, super::DumpOrder)>,
    pub dumps: BTreeMap<u32, Vec<crate::proto::DumpInfo>>,
    pub histories: BTreeMap<u32, Vec<crate::proto::Posted>>,
    pub physio: super::PhysioFacts,
    pub bound: Vec<(String, bool)>,
    pub resets: Vec<String>,
    pub no_reset: Vec<String>,
    pub vfio: Vec<(u32, Vec<u32>, usize)>,
    pub fenced: Vec<String>,
    pub drives: Vec<super::DriveFacts>,
    pub opened_drives: Vec<(u32, String, usize)>,
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
    fn hold(&mut self, id: u32, wants: &[Want]) -> Result<Vec<Given>, Vec<Issued>> {
        if let Some(refused) = &self.refuse_hold {
            return Err(vec![refused.clone()]);
        }
        Ok(wants
            .iter()
            .map(|w| {
                self.held.push((id, w.clone()));
                Given {
                    want: w.clone(),
                    bytes: 1 << 30,
                    capacity_mib: 1024,
                    protected: false,
                }
            })
            .collect())
    }
    fn give_back(&mut self, id: u32, wants: &[Want]) {
        self.held
            .retain(|(i, w)| *i != id || (!wants.is_empty() && !wants.contains(w)));
    }
    fn control(&mut self, id: u32, line: &str, _with: Option<&Want>) -> Result<String, String> {
        self.controls.push((id, line.to_string()));
        Ok(self.control_answer.clone().unwrap_or_else(|| {
            line.split_whitespace()
                .nth(2)
                .map_or("ok".into(), |name| format!("ok {name}"))
        }))
    }
    fn host_paths(&self) -> bool {
        !self.refuse_host_paths
    }
    fn delete_vdisks(&mut self, disks: &[(String, String)]) -> Vec<Issued> {
        self.deleted.extend_from_slice(disks);
        Vec::new()
    }
    fn switches(&mut self) -> Option<Vec<vswitch::Switch>> {
        self.switches.clone()
    }
    fn open_port(&mut self, id: u32, spec: &vswitch::port::PortSpec) -> Result<u32, Vec<Issued>> {
        if let Some(refused) = &self.refuse_port {
            return Err(vec![refused.clone()]);
        }
        self.ports.push((id, spec.clone()));
        Ok(9000)
    }
    fn close_ports(&mut self, id: u32) {
        self.ports.retain(|(i, _)| *i != id);
    }
    fn reconcile_ports(&mut self, running: &[u32]) {
        self.reconciled.push(running.to_vec());
    }
    fn channel_sockets(
        &mut self,
        _id: u32,
        first: usize,
    ) -> Result<Vec<qsvmm::config::ConsolePort>, String> {
        Ok(vec![
            qsvmm::config::ConsolePort {
                name: qsipart::SERVICE_PORT.into(),
                socket: format!("/proc/self/fd/{first}").into(),
            },
            qsvmm::config::ConsolePort {
                name: qsipart::CONSOLE_PORT.into(),
                socket: format!("/proc/self/fd/{}", first + 1).into(),
            },
        ])
    }
    fn send(&mut self, id: u32, port: super::Port, frame: Vec<u8>) {
        self.sent.push((id, port, frame));
    }
    fn close_channel(&mut self, id: u32, port: super::Port) {
        self.closed.push((id, port));
    }
    fn keep(&mut self, id: u32) -> Result<(), String> {
        self.keep_asked.push(id);
        Ok(())
    }
    fn dump_dir(&mut self, id: u32) -> Option<PathBuf> {
        (!self.no_repository).then(|| PathBuf::from(format!("/pools/MEDIA/dumps/{id}")))
    }
    fn begin_dump(&mut self, id: u32, order: &super::DumpOrder) -> Result<(), String> {
        self.dump_orders.push((id, order.clone()));
        Ok(())
    }
    fn dumps(&mut self, id: u32) -> Vec<crate::proto::DumpInfo> {
        self.dumps.get(&id).cloned().unwrap_or_default()
    }
    fn remove_dump(&mut self, id: u32, name: &str) -> bool {
        let Some(all) = self.dumps.get_mut(&id) else {
            return false;
        };
        let before = all.len();
        all.retain(|d| d.name != name);
        all.len() != before
    }
    fn keep_history(&mut self, id: u32, history: &[crate::proto::Posted]) {
        self.histories.insert(id, history.to_vec());
    }
    fn kept_history(&mut self, id: u32) -> Vec<crate::proto::Posted> {
        self.histories.get(&id).cloned().unwrap_or_default()
    }
    fn physio_facts(&mut self) -> super::PhysioFacts {
        self.physio.clone()
    }
    fn bind(&mut self, address: &str, passthrough: bool) -> Result<(), String> {
        self.bound.push((address.to_string(), passthrough));
        Ok(())
    }
    fn reset(&mut self, address: &str) -> Result<(), String> {
        self.resets.push(address.to_string());
        if self.no_reset.iter().any(|a| a == address) {
            return Err("the double does not reset it".into());
        }
        Ok(())
    }
    fn open_vfio(
        &mut self,
        id: u32,
        groups: &[u32],
        first: usize,
    ) -> Result<(String, Vec<(u32, String)>), String> {
        self.vfio.push((id, groups.to_vec(), first));
        let paths = (first + 1..)
            .zip(groups)
            .map(|(fd, g)| (*g, format!("/proc/self/fd/{fd}")))
            .collect();
        Ok((format!("/proc/self/fd/{first}"), paths))
    }
    fn keep_fenced(&mut self, fenced: &[String]) {
        self.fenced = fenced.to_vec();
    }
    fn fenced(&mut self) -> Vec<String> {
        self.fenced.clone()
    }
    fn tape_drives(&mut self, _held: &[String]) -> Vec<super::DriveFacts> {
        self.drives.clone()
    }
    fn open_drive(
        &mut self,
        id: u32,
        location: &str,
        _sg: &str,
        fd: usize,
    ) -> Result<String, String> {
        self.opened_drives.push((id, location.to_string(), fd));
        Ok(format!("/proc/self/fd/{fd}"))
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

/// Bring `id` to Running, as its VMM's report would for a system with
/// no service channel: the kernel entered, and the wait for a channel
/// over.
fn run(m: &mut Manager<Fake>, id: u32) {
    m.activate(id, String::new(), None, None).unwrap();
    m.report(id, "REF C2005000 Partition activation started: X", false);
    m.report(id, "REF C2005090 Partition started: X, side A", false);
    assert_eq!(state(m, id), State::Starting);
    m.system.now += u64::from(m.host.settings.service.channel_wait_secs);
    m.tick();
    m.system.now -= u64::from(m.host.settings.service.channel_wait_secs);
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
    again.delete(second, false).unwrap();
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
    // Kept as given, at the next revision.
    changed.revision += 1;
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
        ids(&m.delete(id, false).unwrap_err()),
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
    for _ in 0..super::HISTORY + 50 {
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
    assert!(config.contains("console = \"partition\""));
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
    // IPL source D wants an optical drive; a device the host does not
    // have is refused as the profile is put.
    m.ended(id, Ended::Exit(0));
    let refused = m
        .activate(id, String::new(), Some(IplSource::D), None)
        .unwrap_err();
    assert!(refused[0].is(&qvsmsg::NO_OPTICAL));
    let mut later = m.profile(id, "NORMAL".into()).unwrap();
    later.name = "NET".into();
    later.physical_io.push(model::PhysicalIo {
        location: "P0000-B00-D05-F0".into(),
        required: true,
    });
    let refused = m.put_profile(id, later).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_UNKNOWN), "{}", refused[0]);
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
    again.system.now += u64::from(again.host.settings.service.channel_wait_secs);
    again.tick();
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
            ..model::AutoStart::default()
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

#[test]
fn a_write_at_a_stale_revision_is_refused_and_events_say_each_revision() {
    let scratch = Scratch::new("revisions");
    let mut m = scratch.manager();
    let (events, seen) = mpsc::channel();
    m.subscribe(events);
    let id = partition(&mut m, "A");
    let entries = m.profile_entries(id).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| (e.name.as_str(), e.revision, e.default))
            .collect::<Vec<_>>(),
        [("NORMAL", 1, true), ("SHARED", 1, false)]
    );
    // The first profile made the definition's default: revision 2.
    let definition = m.definition(id).unwrap();
    assert_eq!(definition.revision, 2);
    // A put at the revision read; then the same again is stale.
    let mut normal = m.profile(id, "NORMAL".into()).unwrap();
    normal.memory.desired_mib = normal.memory.minimum_mib;
    assert_eq!(m.put_profile_at(id, normal.clone(), Some(1)).unwrap(), 2);
    let stale = m.put_profile_at(id, normal.clone(), Some(1)).unwrap_err();
    assert_eq!(ids(&stale), [qvsmsg::PROFILE_CHANGED.id]);
    // A new one with no revision; a name that is there without one.
    let mut other = normal.clone();
    other.name = "OTHER".into();
    assert_eq!(m.put_profile_at(id, other.clone(), None).unwrap(), 1);
    let there = m.put_profile_at(id, other, None).unwrap_err();
    assert_eq!(ids(&there), [qvsmsg::PROFILE_EXISTS.id]);
    // A revision of a profile deleted since is stale too.
    m.delete_profile(id, "OTHER".into()).unwrap();
    let gone = m.put_profile_at(id, normal.clone(), Some(7)).unwrap_err();
    assert_eq!(ids(&gone), [qvsmsg::PROFILE_CHANGED.id]);
    // A definition changed at its revision, then stale.
    let mut d = m.definition(id).unwrap();
    d.keylock = Keylock::Manual;
    assert_eq!(m.change_at(d.clone(), 2).unwrap(), 3);
    let stale = m.change_at(d, 2).unwrap_err();
    assert_eq!(ids(&stale), [qvsmsg::DEFINITION_CHANGED.id]);
    // A plain put still bumps the revision a console reads.
    m.put_profile(id, normal).unwrap();
    assert_eq!(m.profile(id, "NORMAL".into()).unwrap().revision, 3);
    // Every put, delete and new default is an event with its revision.
    let said: Vec<(String, Option<u64>, bool)> = seen
        .try_iter()
        .filter_map(|e| match e {
            Event::Profile {
                profile,
                revision,
                default,
                ..
            } => Some((profile, revision, default)),
            _ => None,
        })
        .collect();
    assert_eq!(
        said,
        [
            ("NORMAL".into(), Some(1), true),
            ("SHARED".into(), Some(1), false),
            ("NORMAL".into(), Some(2), true),
            ("OTHER".into(), Some(1), false),
            ("OTHER".into(), None, false),
            ("NORMAL".into(), Some(3), true),
        ]
    );
}

#[test]
fn the_current_configuration_is_saved_and_a_partition_is_created_with_its_profile() {
    let scratch = Scratch::new("current");
    let mut m = scratch.manager();
    let id = partition(&mut m, "A");
    let refused = m.save_current(id, "NOW".into(), None).unwrap_err();
    assert_eq!(ids(&refused), [qvsmsg::NOTHING_TO_SAVE.id]);
    run(&mut m, id);
    let given = m.list()[0].current.clone().unwrap();
    assert_eq!(m.save_current(id, "NOW".into(), None).unwrap(), 1);
    let saved = m.profile(id, "NOW".into()).unwrap();
    let Processors::Dedicated { cores } = &given.processors else {
        panic!("the sample is dedicated");
    };
    assert_eq!(saved.processors.desired as usize, cores.len());
    assert_eq!(saved.memory.desired_mib, given.memory_mib);
    assert!(model::validate::profile(&saved, m.limits()).is_empty());
    // Saved again only at its revision.
    let stale = m.save_current(id, "NOW".into(), None).unwrap_err();
    assert_eq!(ids(&stale), [qvsmsg::PROFILE_EXISTS.id]);
    assert_eq!(m.save_current(id, "NOW".into(), Some(1)).unwrap(), 2);

    // Created with its first profile, its default; a profile refused
    // leaves nothing.
    let mut profile = sample::dedicated();
    profile.scsi[0].luns[0].backing = "/dev/B".into();
    profile.scsi[0].luns[0].serial = None;
    let b = m.create_with("B".into(), profile.clone()).unwrap();
    assert_eq!(b.default_profile, "NORMAL");
    assert_eq!(m.profile_entries(b.id).unwrap().len(), 1);
    profile.memory.desired_mib = 0;
    let before = m.list().len();
    assert!(m.create_with("C".into(), profile).is_err());
    assert_eq!(m.list().len(), before);
    assert!(!m.list().iter().any(|s| s.name == "C"));
}

#[test]
fn a_release_image_takes_no_host_path_as_a_backing_store() {
    let scratch = Scratch::new("release");
    let mut m = scratch.manager();
    m.system.refuse_host_paths = true;
    let id = partition(&mut m, "A");
    let refused = m.activate(id, String::new(), None, None).unwrap_err();
    assert_eq!(ids(&refused), [qvsmsg::BACKING_NOT_VDISK.id]);
    assert!(refused[0].text.contains("/dev/A"));
    assert!(m.system.launched.is_empty());
    assert_eq!(state(&mut m, id), State::NotActivated);
}

fn adapter(
    slot: u32,
    switch: &str,
    vlan: u32,
    tagged: &[u32],
    required: bool,
) -> model::EthernetAdapter {
    model::EthernetAdapter {
        slot,
        switch: Some(switch.into()),
        vlan,
        tagged: tagged.to_vec(),
        mac: None,
        required,
    }
}

fn switch(name: &str, vlans: &[u16]) -> vswitch::Switch {
    vswitch::Switch {
        name: name.into(),
        id: 1,
        vlans: vlans.to_vec(),
        mtu: 9000,
        uplink: None,
    }
}

#[test]
fn an_adapter_is_given_its_address_once_and_checked_against_the_switches() {
    let scratch = Scratch::new("ethernet");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    let other = partition(&mut m, "TEST");
    m.system.switches = Some(vec![switch("PRODNET", &[10, 20])]);
    let mut p = m.profile(id, "NORMAL".into()).unwrap();
    p.ethernet = vec![adapter(5, "PRODNET", 10, &[20], true)];
    m.put_profile(id, p).unwrap();
    // Made from the host's prefix, the partition and the slot; kept in
    // the profile, and the prefix in the settings.
    let prefix = vswitch::mac::prefix_of("HOST01");
    let made = vswitch::mac::text(&vswitch::mac::made(prefix, id, 5));
    let kept = m.profile(id, "NORMAL".into()).unwrap();
    assert_eq!(kept.ethernet[0].mac.as_deref(), Some(made.as_str()));
    assert_eq!(m.settings().mac_prefix.as_deref(), Some(&made[..8]));
    let again = scratch.manager();
    assert_eq!(again.prefix(), prefix);
    // Another partition may not have it; a switch or VLAN that is not
    // there is refused by name.
    let mut q = m.profile(other, "NORMAL".into()).unwrap();
    q.ethernet = vec![
        model::EthernetAdapter {
            mac: Some(made.clone()),
            ..adapter(5, "PRODNET", 10, &[], true)
        },
        adapter(6, "NONE", 10, &[], true),
        adapter(7, "PRODNET", 30, &[40], false),
    ];
    assert_eq!(
        ids(&m.put_profile(other, q).unwrap_err()),
        [
            qvsmsg::MAC_IN_USE.id,
            qvsmsg::ADAPTER_SWITCH_MISSING.id,
            qvsmsg::ADAPTER_VLAN_MISSING.id,
            qvsmsg::ADAPTER_VLAN_MISSING.id,
        ]
    );
    // Every adapter, as the switch service asks for them.
    let all = m.adapters(0);
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].partition, all[0].slot, all[0].vlan), (id, 5, 10));
    assert_eq!(all[0].mac, made);
    assert!(m.adapters(other).is_empty());
    // The partition deleted, its address is free.
    m.delete(id, false).unwrap();
    let mut q = m.profile(other, "NORMAL".into()).unwrap();
    q.ethernet = vec![model::EthernetAdapter {
        mac: Some(made),
        ..adapter(5, "PRODNET", 10, &[], true)
    }];
    m.put_profile(other, q).unwrap();
}

#[test]
fn an_activation_opens_each_adapters_port_and_the_end_closes_them() {
    let scratch = Scratch::new("ports");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    let mut p = m.profile(id, "NORMAL".into()).unwrap();
    p.ethernet = vec![
        adapter(7, "BACKEND", 30, &[], false),
        adapter(5, "PRODNET", 10, &[20], true),
    ];
    m.put_profile(id, p).unwrap();
    run(&mut m, id);
    let opened: Vec<u32> = m.system.ports.iter().map(|(_, s)| s.slot).collect();
    assert_eq!(opened, [5, 7]);
    assert_eq!(m.system.ports[0].1.tagged, [20]);
    let config = &m.system.launched[0].2;
    assert!(config.contains("[[net]]"), "{config}");
    assert!(config.contains("tap = \"/proc/self/fd/3\""), "{config}");
    assert!(config.contains("tap = \"/proc/self/fd/4\""), "{config}");
    assert!(config.contains("mtu = 9000"), "{config}");
    let listed = m.list().into_iter().find(|s| s.id == id).unwrap();
    assert_eq!(listed.ethernet.len(), 2);
    assert!(listed.ethernet.iter().all(|e| e.link));
    m.ended(id, Ended::Exit(0));
    assert!(m.system.ports.is_empty());

    // A switch service that refuses: the required adapter fails the
    // activation by name, and nothing is held.
    m.system.refuse_port = Some(qvsmsg::ADAPTER_SWITCH_MISSING.with(&["5", "PRODNET"]));
    let refused = m.activate(id, String::new(), None, None).unwrap_err();
    assert_eq!(
        ids(&refused),
        [qvsmsg::PORT_NOT_GIVEN.id, qvsmsg::ADAPTER_SWITCH_MISSING.id]
    );
    assert!(refused[0].text.contains("slot 5"), "{}", refused[0]);
    assert!(m.system.held.is_empty());
    assert_eq!(state(&mut m, id), State::NotActivated);
    // Not required: it comes up without link.
    let mut p = m.profile(id, "NORMAL".into()).unwrap();
    p.ethernet = vec![adapter(7, "BACKEND", 30, &[], false)];
    m.put_profile(id, p).unwrap();
    run(&mut m, id);
    let config = &m.system.launched.last().unwrap().2;
    assert!(
        config.contains("[[net]]") && !config.contains("tap ="),
        "{config}"
    );
    let listed = m.list().into_iter().find(|s| s.id == id).unwrap();
    assert_eq!(listed.ethernet.len(), 1);
    assert!(!listed.ethernet[0].link);
}

/// The harness's devices: a management adapter, a network adapter and
/// a disk controller each alone, a pair that share a group.
fn physio() -> super::PhysioFacts {
    let pci = |address: &str, location: &str, class: &str, group: u32| state::doc::inventory::Pci {
        address: address.into(),
        vendor: "8086".into(),
        device: "10d3".into(),
        class: class.into(),
        driver: None,
        location: location.into(),
        group: Some(group),
        reset: vec!["flr".into()],
        interrupts: vec!["msix".into()],
        boot_display: false,
    };
    super::PhysioFacts {
        pci: vec![
            pci("0000:00:03.0", "P0000-B00-D03-F0", "020000", 3),
            pci("0000:00:05.0", "P0000-B00-D05-F0", "020000", 5),
            pci("0000:00:06.0", "P0000-B00-D06-F0", "010802", 6),
            pci("0000:00:07.0", "P0000-B00-D07-F0", "020000", 7),
            pci("0000:00:07.1", "P0000-B00-D07-F1", "020000", 7),
        ],
        iommu: state::doc::inventory::Iommu {
            translation: true,
            remapping: true,
        },
        uses: pio::HostUses {
            management: vec!["P0000-B00-D03-F0-E0".into()],
            ..pio::HostUses::default()
        },
    }
}

fn with_devices(m: &mut Manager<Fake>, id: u32, name: &str, wanted: &[(&str, bool)]) {
    let mut p = m.profile(id, "NORMAL".into()).unwrap();
    p.name = name.into();
    p.physical_io = wanted
        .iter()
        .map(|(l, r)| model::PhysicalIo {
            location: (*l).into(),
            required: *r,
        })
        .collect();
    m.put_profile(id, p).unwrap();
}

#[test]
fn devices_are_released_given_whole_reset_and_fenced() {
    let scratch = Scratch::new("physio");
    let mut m = scratch.manager();
    m.system.physio = physio();
    let a = partition(&mut m, "FIRST");
    let b = partition(&mut m, "SECOND");
    // The host's own is not released; a release is typed in full.
    let refused = m
        .release("P0000-B00-D03-F0".into(), "P0000-B00-D03-F0".into())
        .unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_HOST_USES), "{}", refused[0]);
    let refused = m
        .release("P0000-B00-D06-F0".into(), "P0000".into())
        .unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_CONFIRM));
    // The pair goes together.
    let pair = m
        .release("P0000-B00-D07-F1".into(), "P0000-B00-D07-F1".into())
        .unwrap();
    assert_eq!(pair, ["P0000-B00-D07-F1", "P0000-B00-D07-F0"]);
    assert_eq!(m.system.bound.len(), 2);
    for l in ["P0000-B00-D06-F0", "P0000-B00-D05-F0"] {
        m.release(l.into(), l.into()).unwrap();
    }
    assert_eq!(Store::at(&scratch.0).released().unwrap().devices.len(), 4);
    // A profile that names one of the pair is refused by name.
    let mut split = m.profile(a, "NORMAL".into()).unwrap();
    split.name = "SPLIT".into();
    split.physical_io = vec![model::PhysicalIo {
        location: "P0000-B00-D07-F0".into(),
        required: true,
    }];
    let refused = m.put_profile(a, split).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_GROUP_SPLIT), "{}", refused[0]);
    // FIRST gets the controller, reset, its group opened after the
    // channel's sockets.
    with_devices(&mut m, a, "IO", &[("P0000-B00-D06-F0", true)]);
    m.activate(a, "IO".into(), None, None).unwrap();
    assert_eq!(m.system.resets, ["0000:00:06.0"]);
    // After the service channel's two sockets, 3 and 4.
    assert_eq!(m.system.vfio, [(a, vec![6], 5)]);
    let (_, _, config) = m.system.launched.last().unwrap();
    assert!(config.contains("vfio = \"/proc/self/fd/5\""), "{config}");
    assert!(config.contains("address = \"0000:00:06.0\""), "{config}");
    assert!(config.contains("group = \"/proc/self/fd/6\""), "{config}");
    // Held: SECOND fails by name when it requires it, and goes without
    // it when it does not.
    with_devices(&mut m, b, "IO", &[("P0000-B00-D06-F0", true)]);
    let refused = m.activate(b, "IO".into(), None, None).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_HELD) && refused[0].text.contains("FIRST"));
    with_devices(
        &mut m,
        b,
        "SOME",
        &[("P0000-B00-D06-F0", false), ("P0000-B00-D05-F0", true)],
    );
    m.activate(b, "SOME".into(), None, None).unwrap();
    assert_eq!(m.system.vfio.last().unwrap().1, [5]);
    // A device a profile names is not taken back; one held neither.
    let refused = m.take_back("P0000-B00-D06-F0".into()).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_HELD), "{}", refused[0]);
    // FIRST's VMM is killed: the controller is reset and free again.
    m.ended(a, Ended::Signal(9));
    assert_eq!(m.system.resets.last().unwrap(), "0000:00:06.0");
    let devices = m.devices();
    let nvme = devices
        .iter()
        .find(|d| d.location == "P0000-B00-D06-F0")
        .unwrap();
    assert!(nvme.holder.is_none() && nvme.assignable());
    // A reset that fails fences it until the host restarts.
    m.system.no_reset.push("0000:00:06.0".into());
    let refused = m.activate(a, "IO".into(), None, None).unwrap_err();
    assert!(
        refused[0].is(&qvsmsg::DEVICE_NOT_ASSIGNABLE),
        "{}",
        refused[0]
    );
    assert!(m.system.posted.iter().any(|(c, _)| c == "A2013050"));
    assert_eq!(m.system.fenced, ["P0000-B00-D06-F0"]);
    m.system.no_reset.clear();
    let refused = m.activate(a, "IO".into(), None, None).unwrap_err();
    assert!(refused[0].text.contains("fenced"), "{}", refused[0]);
    // Taken back once no profile names it.
    let refused = m.take_back("P0000-B00-D06-F0".into()).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_NAMED), "{}", refused[0]);
    m.delete_profile(a, "IO".into()).unwrap();
    m.ended(b, Ended::Exit(0));
    m.delete_profile(b, "IO".into()).unwrap();
    m.delete_profile(b, "SOME".into()).unwrap();
    assert_eq!(
        m.take_back("P0000-B00-D06-F0".into()).unwrap(),
        ["P0000-B00-D06-F0"]
    );
    assert!(m.system.fenced.is_empty());
    assert_eq!(
        m.system.bound.last().unwrap(),
        &("0000:00:06.0".to_string(), false)
    );
}

#[test]
fn a_running_shared_partition_s_entitlement_changes_until_its_next_activation() {
    let scratch = Scratch::new("entitle");
    let mut m = scratch.manager();
    let id = partition(&mut m, "PROD");
    let up = |units: u32| resource::Entitlement {
        units: Some(model::Units(units)),
        ..resource::Entitlement::default()
    };
    let refused = m.entitlement(id, up(100), "OPER".into()).unwrap_err();
    assert!(refused[0].is(&qvsmsg::ENTITLEMENT_NOT_RUNNING));
    m.activate(id, "SHARED".into(), None, None).unwrap();
    let before = m.list().remove(0).current.unwrap();
    let held = before.processors.units();
    let after = m.entitlement(id, up(held.0 + 10), "OPER".into()).unwrap();
    assert_eq!(after.processors.units(), model::Units(held.0 + 10));
    assert_eq!(m.list().remove(0).current.unwrap(), after);
    assert!(
        m.system
            .applied
            .iter()
            .any(|s| matches!(s, Step::Write(f, _) if f == "qsvs/pools/DEFAULT/p1/cpu.max"))
    );
    assert_eq!(m.system.records[&id].current, after);
    assert!(
        m.system
            .posted
            .iter()
            .any(|(c, t)| c == "C2013040" && t.contains("by OPER"))
    );
    // Outside the profile: refused by name.
    let refused = m.entitlement(id, up(10_000), "OPER".into()).unwrap_err();
    assert!(
        refused.iter().any(|r| r.is(&qvsmsg::ENTITLEMENT_OUTSIDE)),
        "{refused:?}"
    );
    // The next activation is the profile's again.
    m.ended(id, Ended::Exit(0));
    m.activate(id, "SHARED".into(), None, None).unwrap();
    assert_eq!(m.list().remove(0).current.unwrap(), before);
    // A dedicated partition is refused.
    m.ended(id, Ended::Exit(0));
    m.activate(id, "NORMAL".into(), None, None).unwrap();
    let refused = m.entitlement(id, up(100), "OPER".into()).unwrap_err();
    assert!(refused[0].is(&qvsmsg::ENTITLEMENT_DEDICATED));
}

#[test]
fn a_physical_tape_drive_is_released_held_by_one_and_never_loaded_from_a_console() {
    const DRIVE: &str = "NOPCI-ST0";
    let scratch = Scratch::new("drives");
    let mut m = scratch.manager();
    m.system.drives = vec![super::DriveFacts {
        location: DRIVE.into(),
        vendor: "Linux".into(),
        product: "scsi_debug".into(),
        serial: "SD0".into(),
        sg: "sg1".into(),
        loaded: Some(true),
    }];
    let a = partition(&mut m, "FIRST");
    let b = partition(&mut m, "SECOND");
    let with_drive = |m: &mut Manager<Fake>, id: u32, location: &str| {
        let mut p = m.profile(id, "NORMAL".into()).unwrap();
        p.name = "TAPE".into();
        p.tape = vec![model::MediaDevice {
            slot: 7,
            media: Some(format!("@{location}")),
        }];
        m.put_profile(id, p)
    };
    let refused = with_drive(&mut m, a, "NOPCI-ST9").unwrap_err();
    assert!(refused[0].is(&qvsmsg::NOT_A_DRIVE), "{}", refused[0]);
    with_drive(&mut m, a, DRIVE).unwrap();
    with_drive(&mut m, b, DRIVE).unwrap();
    // Not released: refused by name.
    let refused = m.activate(a, "TAPE".into(), None, None).unwrap_err();
    assert!(
        refused[0].is(&qvsmsg::DEVICE_NOT_RELEASED),
        "{}",
        refused[0]
    );
    assert_eq!(m.release(DRIVE.into(), DRIVE.into()).unwrap(), [DRIVE]);
    m.activate(a, "TAPE".into(), None, None).unwrap();
    assert_eq!(m.system.opened_drives, [(a, DRIVE.to_string(), 5)]);
    let (_, _, config) = m.system.launched.last().unwrap();
    assert!(config.contains("drive = \"/proc/self/fd/5\""), "{config}");
    assert!(
        config.contains(&format!("location = \"{DRIVE}\"")),
        "{config}"
    );
    let listed = m.tape_drives();
    assert_eq!((listed[0].holder, listed[0].slot), (a, 7));
    // One holder at a time.
    let refused = m.activate(b, "TAPE".into(), None, None).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_HELD) && refused[0].text.contains("FIRST"));
    // Its cartridge is changed at the drive.
    let refused = m.load_media(a, 7, "SAVE1".into()).unwrap_err();
    assert!(
        refused[0].is(&qvsmsg::DRIVE_SLOT_PHYSICAL),
        "{}",
        refused[0]
    );
    // Named by profiles: not taken back.
    m.ended(a, Ended::Exit(0));
    let refused = m.take_back(DRIVE.into()).unwrap_err();
    assert!(refused[0].is(&qvsmsg::DEVICE_NAMED), "{}", refused[0]);
    assert_eq!(m.tape_drives()[0].holder, 0);
}
