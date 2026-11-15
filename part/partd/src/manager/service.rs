// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service channel of each partition (`docs/SERVICE.md`): what the
//! system says on it, and what that does to the partition.
//!
//! | The system | The partition |
//! |---|---|
//! | its state `ipl` | Starting |
//! | `complete` | Running; auto start's wait is over |
//! | `ending` | Shutting down |
//! | a `B1` code, then `down` | held in Error with its memory, the code current |
//! | no hello in the time set after its kernel was entered | Running, with no service channel |
//!
//! A shut down asked of the system waits for its answer; accepted, the
//! partition is Shutting down until its VMM ends, and is shut down
//! immediately when its time has passed.

use std::sync::mpsc::{self, Receiver, SyncSender};

use qsipart::service::{Hello, Keylock as SysKeylock, Refcode, SysState};
use qvsmsg::Issued;
use svcchan::{Ask, Channel, Happened};

use super::{Manager, Plan, System};
use crate::proto::{Event, Posted, Refused, Service as Facts, Shutdown, Source, State};

/// Seconds a terminal code waits for the system's `down` before the
/// partition is held all the same.
const TERMINAL_WAIT: u64 = 10;

/// One of the channel's two ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Port {
    /// The service port.
    Service,
    /// The console port.
    Console,
}

/// What a request that may wait for the system answers: at once, or
/// later on the receiver.
pub enum Answer {
    /// Done.
    Now(Result<String, Refused>),
    /// The system's answer, or the time it had for it, will say.
    Later(Receiver<Result<String, Refused>>),
}

/// What a request asked of the system is for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Asked {
    /// A shut down: the plan once accepted.
    Shutdown {
        kind: Shutdown,
        delay: u32,
        restart: bool,
        /// The host's own shut down: refused or not answered, it is
        /// ended immediately.
        host: bool,
    },
    /// Function 21.
    Panel,
}

/// Where the answer to a request asked of the system goes.
pub(crate) type Waiter = SyncSender<Result<String, Refused>>;

/// A partition's channel, for one activation.
#[derive(Default)]
pub(crate) struct Service {
    pub(crate) channel: Option<Channel>,
    /// What the LIC record said of the system's channel.
    pub(crate) recorded: Recorded,
    /// What is shown of it.
    pub(crate) facts: Facts,
    /// When the kernel was entered.
    pub(crate) kernel_at: Option<u64>,
    /// Whether the system has said hello in this activation.
    pub(crate) greeted: bool,
    /// Whether it has posted a code in this activation.
    pub(crate) system_posted: bool,
    /// Whether it reported its IPL complete.
    pub(crate) complete: bool,
    /// A terminal code it posted, and when.
    pub(crate) terminal: Option<(String, u64)>,
    /// What the hello was checked against already: a mismatch is said
    /// once an activation.
    pub(crate) said: Vec<&'static str>,
    /// Requests that wait for the system, by tag.
    pub(crate) asks: Vec<(u64, Asked, Option<Waiter>)>,
}

/// What a partition's LIC record says of its system's service channel,
/// as its VMM reported it (`CHANNEL` lines).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Recorded {
    /// Nothing: a system older than the record's word, waited for
    /// `channel_wait_secs`; also a blank disk, an IPL from media.
    #[default]
    Unsaid,
    /// It has none: Running once its kernel is entered.
    None,
    /// It has one: Starting until it says its IPL is complete.
    Has,
}

impl Recorded {
    /// From the line's words after `CHANNEL`; `None` for nonsense.
    pub(crate) fn parse(words: &str) -> Option<Self> {
        let mut w = words.split_whitespace();
        match (w.next(), w.next()) {
            (Some("unsaid"), None) => Some(Self::Unsaid),
            (Some("none"), None) => Some(Self::None),
            (Some("has"), Some(v)) if v.parse::<u16>().is_ok_and(|v| v > 0) => Some(Self::Has),
            _ => None,
        }
    }
}

impl Service {
    /// The channel of a new activation.
    pub(crate) fn fresh() -> Self {
        Self::default()
    }

    /// Whether the system can be asked for what needs `cap`.
    pub(crate) fn can(&self, cap: u64) -> bool {
        self.channel.as_ref().is_some_and(|c| c.has(cap)) && !self.facts.kept
    }
}

fn keylock_word(k: SysKeylock) -> &'static str {
    match k {
        SysKeylock::Normal => "normal",
        SysKeylock::Manual => "manual",
        SysKeylock::Other(_) => "other",
    }
}

impl<S: System> Manager<S> {
    fn publish_channel(&mut self, id: u32) {
        let time = self.system.now();
        let Some(p) = self.partitions.get(&id) else {
            return;
        };
        let event = Event::Channel {
            id,
            name: p.definition.name.clone(),
            service: p.service.facts.clone(),
            time,
        };
        self.publish(&event);
    }

    /// Whether the connections of `id`'s channel are still wanted: its
    /// VMM of the activation `sequence` runs.
    pub fn channel_wanted(&self, id: u32, sequence: u64) -> bool {
        self.partitions
            .get(&id)
            .and_then(|p| p.run.as_ref())
            .is_some_and(|r| r.sequence == sequence)
    }

    /// The activation `id` has now, for a connection to be made for it.
    pub fn activation(&self, id: u32) -> Option<u64> {
        self.partitions
            .get(&id)
            .and_then(|p| p.run.as_ref())
            .map(|r| r.sequence)
    }

    /// A port of `id`'s channel connected.
    pub fn channel_up(&mut self, id: u32, port: Port) {
        let now = self.system.now();
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        match port {
            Port::Service => p.service.channel = Some(Channel::new(now)),
            Port::Console => {
                p.console.connected();
                self.system
                    .send(id, Port::Console, console_port::Console::peer_hello());
            }
        }
    }

    /// A port of `id`'s channel went away.
    pub fn channel_down(&mut self, id: u32, port: Port) {
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        if port == Port::Console {
            return;
        }
        let Some(mut channel) = p.service.channel.take() else {
            return;
        };
        let abandoned = channel.abandon();
        let was_up = p.service.facts.up;
        p.service.facts.up = false;
        p.service.facts.responding = false;
        let name = p.definition.name.clone();
        for (tag, _) in abandoned {
            let why = qvsmsg::NO_CHANNEL.with(&[&name, "requested"]);
            self.answer(id, tag, Err(vec![why]));
        }
        if was_up {
            self.publish_channel(id);
        }
    }

    /// Bytes from `id`'s system on `port`.
    pub fn channel_bytes(&mut self, id: u32, port: Port, data: &[u8]) {
        let now = self.system.now();
        match port {
            Port::Console => self.console_bytes(id, data),
            Port::Service => {
                let Some(channel) = self
                    .partitions
                    .get_mut(&id)
                    .and_then(|p| p.service.channel.as_mut())
                else {
                    return;
                };
                for happened in channel.bytes(data, now) {
                    self.happened(id, happened);
                }
            }
        }
    }

    fn happened(&mut self, id: u32, happened: Happened) {
        match happened {
            Happened::Hello(h) => self.hello(id, &h),
            Happened::Refcode(r) => self.system_code(id, &r),
            Happened::State(s) => self.system_state(id, &s),
            Happened::Answered { tag, result, .. } => {
                self.answered(id, tag, result);
            }
            Happened::TimedOut { tag, .. } => {
                let name = self.name_of(id);
                let why = qvsmsg::SYSTEM_SILENT.with(&[&name, &svcchan::ANSWER_SECS.to_string()]);
                self.not_answered(id, tag, why);
            }
            Happened::TimeRequest => {
                let ns = i64::try_from(self.system.now()).unwrap_or(0) * 1_000_000_000;
                self.system.send(id, Port::Service, Channel::time(ns));
            }
            Happened::NotResponding => {
                if let Some(p) = self.partitions.get_mut(&id) {
                    p.service.facts.responding = false;
                }
                let name = self.name_of(id);
                let code = &refcode::NOT_RESPONDING;
                self.code(id, code.code, &code.text_with(&name), true);
                self.publish_channel(id);
            }
            Happened::Responding => {
                if let Some(p) = self.partitions.get_mut(&id) {
                    p.service.facts.responding = true;
                }
                let name = self.name_of(id);
                let code = &refcode::RESPONDING;
                self.code(id, code.code, &code.text_with(&name), true);
                self.publish_channel(id);
            }
            Happened::Nonsense(why) => {
                let name = self.name_of(id);
                let message = qvsmsg::CHANNEL_CLOSED.with(&[&name, &why]);
                self.system.log(&message.to_string());
                let code = &refcode::CHANNEL_CLOSED;
                self.code(
                    id,
                    code.code,
                    &code.text_with(&format!("{name}, {why}")),
                    true,
                );
                self.system.close_channel(id, Port::Service);
                self.channel_down(id, Port::Service);
            }
        }
    }

    /// The system's hello: checked against the definition, its facts
    /// kept, answered with the host's. Its fields are the system's, and
    /// bounded before anything is made of them.
    fn hello(&mut self, id: u32, h: &Hello) {
        let bound = |s: &str, most: usize| -> String {
            s.chars().filter(|c| !c.is_control()).take(most).collect()
        };
        let h = &Hello {
            system: bound(&h.system, 16),
            release: bound(&h.release, 16),
            serial: bound(&h.serial, 16),
            slot: bound(&h.slot, 4),
            ipl_type: bound(&h.ipl_type, 4),
            ..h.clone()
        };
        let name = self.name_of(id);
        let host_name = format!("QS/VS {}", release::RELEASE);
        self.system
            .send(id, Port::Service, Channel::peer_hello(&host_name));
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        let first = !p.service.facts.up;
        p.service.greeted = true;
        let (version, caps) = p
            .service
            .channel
            .as_ref()
            .map_or((0, 0), |c| (c.version, c.caps));
        let changed = p.service.facts.release != h.release || p.service.facts.system != h.system;
        p.service.facts.up = true;
        p.service.facts.responding = true;
        p.service.facts.version = version;
        p.service.facts.caps = caps;
        p.service.facts.release.clone_from(&h.release);
        p.service.facts.system.clone_from(&h.system);
        if p.service.facts.none {
            // A channel that came late after all.
            p.service.facts.none = false;
        }
        // What differs from what the host gave it, said once.
        let keylock = p
            .run
            .as_ref()
            .and_then(|r| r.keylock)
            .unwrap_or(p.definition.keylock);
        let given = match keylock {
            model::Keylock::Manual => "manual",
            _ => "normal",
        };
        let mut differs = Vec::new();
        if !h.serial.is_empty() && h.serial != p.definition.serial {
            differs.push((
                "serial number",
                h.serial.clone(),
                p.definition.serial.clone(),
            ));
        }
        if matches!(h.keylock, SysKeylock::Normal | SysKeylock::Manual)
            && keylock_word(h.keylock) != given
        {
            differs.push((
                "keylock",
                keylock_word(h.keylock).to_string(),
                given.to_string(),
            ));
        }
        let mut say = Vec::new();
        for (what, theirs, ours) in differs {
            if !p.service.said.contains(&what) {
                p.service.said.push(what);
                say.push(qvsmsg::IDENTITY_DIFFERS.with(&[&name, what, &theirs, &ours]));
            }
        }
        for message in say {
            self.system.log(&message.to_string());
            let code = &refcode::IDENTITY_DIFFERS;
            let detail = format!("{name}, {}", message.text);
            self.code(id, code.code, &code.text_with(&detail), true);
        }
        if first {
            let code = &refcode::CHANNEL_UP;
            let detail = format!("{name}, {} {}, version {version}", h.system, h.release);
            self.code(id, code.code, &code.text_with(&detail), true);
        }
        if first || changed {
            self.publish_channel(id);
        }
    }

    /// A reference code the system posted.
    fn system_code(&mut self, id: u32, r: &Refcode) {
        let r = &Refcode {
            code: r
                .code
                .chars()
                .filter(|c| !c.is_control())
                .take(16)
                .collect(),
            text: r
                .text
                .chars()
                .filter(|c| !c.is_control())
                .take(256)
                .collect(),
            ..r.clone()
        };
        let now = self.system.now();
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        // The code it says is current when the channel comes up again is
        // in the history already.
        if r.current
            && p.last_system()
                .is_some_and(|last| last.code == r.code && last.text == r.text)
        {
            p.current = p.last_system().cloned();
            p.service.system_posted = true;
            return;
        }
        p.service.system_posted = true;
        if r.code.starts_with("B1") {
            p.service.terminal = Some((r.code.clone(), now));
        }
        let posted = Posted {
            time: now,
            code: r.code.chars().take(16).collect(),
            text: r.text.chars().take(256).collect(),
            source: Source::System,
            words: r
                .words
                .iter()
                .take(8)
                .map(|w| w.chars().take(16).collect())
                .collect(),
        };
        self.add(id, posted, true);
    }

    /// The system's state.
    fn system_state(&mut self, id: u32, s: &qsipart::service::State) {
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        let facts_changed =
            p.service.facts.fix_level != s.fix_level || p.service.facts.addresses != s.addresses;
        p.service.facts.fix_level = s.fix_level.chars().take(32).collect();
        p.service.facts.addresses = s
            .addresses
            .iter()
            .take(32)
            .map(|a| a.chars().take(64).collect())
            .collect();
        let kept = p.run.as_ref().is_some_and(|r| r.kept);
        let state = p.state;
        let terminal = p.service.terminal.is_some();
        if facts_changed {
            self.publish_channel(id);
        }
        if kept || !state.active() {
            return;
        }
        match s.state {
            SysState::Ipl if state == State::Running => self.set_state(id, State::Starting),
            SysState::Complete => {
                if let Some(p) = self.partitions.get_mut(&id) {
                    p.service.complete = true;
                }
                if state == State::Starting {
                    self.set_state(id, State::Running);
                }
            }
            SysState::Down if terminal => self.keep_partition(id),
            SysState::Ending | SysState::Down => self.set_state(id, State::ShuttingDown),
            _ => {}
        }
    }

    /// Hold the partition with its memory: after a terminal code.
    pub(crate) fn keep_partition(&mut self, id: u32) {
        let name = self.name_of(id);
        if self
            .partitions
            .get(&id)
            .and_then(|p| p.run.as_ref())
            .is_none_or(|r| r.kept)
        {
            return;
        }
        match self.system.keep(id) {
            Ok(()) => {
                self.kept(id);
                let code = &refcode::PARTITION_KEPT;
                self.code(id, code.code, &code.text_with(&name), true);
            }
            Err(e) => self
                .system
                .log(&format!("{name}: the partition was not held: {e}")),
        }
    }

    /// The partition `id` is held: its VMM said so, or did as asked.
    pub(crate) fn kept(&mut self, id: u32) {
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        let Some(run) = p.run.as_mut() else {
            return;
        };
        run.kept = true;
        run.plan = None;
        p.service.facts.kept = true;
        self.set_state(id, State::Error);
        self.publish_channel(id);
    }

    /// Ask `id`'s system for `ask`. The receiver has the answer.
    pub(crate) fn ask(
        &mut self,
        id: u32,
        ask: Ask,
        asked: Asked,
    ) -> Result<Receiver<Result<String, Refused>>, Refused> {
        let now = self.system.now();
        let tag = self.tag();
        let name = self.name_of(id);
        let (to, from) = mpsc::sync_channel(1);
        let p = self
            .partitions
            .get_mut(&id)
            .ok_or_else(|| vec![qvsmsg::PARTITION_MISSING.with(&[&id.to_string()])])?;
        let Some(frame) = p
            .service
            .channel
            .as_mut()
            .and_then(|c| c.ask(ask, tag, now))
        else {
            return Err(vec![qvsmsg::SYSTEM_SILENT.with(&[&name, "0"])]);
        };
        p.service.asks.push((tag, asked, Some(to)));
        self.system.send(id, Port::Service, frame);
        Ok(from)
    }

    fn answer(&mut self, id: u32, tag: u64, result: Result<String, Refused>) {
        let Some(p) = self.partitions.get_mut(&id) else {
            return;
        };
        if let Some(at) = p.service.asks.iter().position(|(t, ..)| *t == tag) {
            let (_, _, to) = p.service.asks.remove(at);
            if let Some(to) = to {
                let _ = to.try_send(result);
            }
        }
    }

    fn asked(&self, id: u32, tag: u64) -> Option<Asked> {
        self.partitions
            .get(&id)?
            .service
            .asks
            .iter()
            .find(|(t, ..)| *t == tag)
            .map(|(_, a, _)| *a)
    }

    /// The system answered request `tag`.
    fn answered(&mut self, id: u32, tag: u64, result: Result<(), (String, String)>) {
        let name = self.name_of(id);
        let asked = self.asked(id, tag);
        match (asked, result) {
            (
                Some(Asked::Shutdown {
                    kind,
                    delay,
                    restart,
                    ..
                }),
                Ok(()),
            ) => {
                self.accepted(id, kind, delay, restart);
                self.answer(id, tag, Ok(String::new()));
            }
            (Some(Asked::Panel), Ok(())) => {
                let code = &refcode::PANEL_DONE;
                self.code(id, code.code, &code.text_with(&format!("{name}, 21")), true);
                self.answer(
                    id,
                    tag,
                    Ok("Dedicated service tools are on the partition's console.".into()),
                );
            }
            (_, Err((msgid, text))) => {
                let why = qvsmsg::SYSTEM_REFUSED.with(&[&name, &msgid, &text]);
                self.not_answered(id, tag, why);
            }
            (None, Ok(())) => {}
        }
    }

    /// A request refused or not answered: the asker is told, and the
    /// host's own shut down goes on without the system.
    fn not_answered(&mut self, id: u32, tag: u64, why: Issued) {
        if let Some(Asked::Shutdown { host: true, .. }) = self.asked(id, tag) {
            self.system.log(&why.to_string());
            self.begin_end(id, false);
        }
        self.answer(id, tag, Err(vec![why]));
    }

    /// The system accepted a shut down: the partition is Shutting down
    /// until its VMM ends, and is forced at the plan's deadline.
    fn accepted(&mut self, id: u32, kind: Shutdown, delay: u32, restart: bool) {
        let now = self.system.now();
        let name = self.name_of(id);
        let service = self.host.settings.service;
        let stopping = self.ending.is_some();
        let allowed = match kind {
            Shutdown::Delayed => u64::from(delay) + u64::from(service.delayed_grace_secs),
            _ if stopping => u64::from(service.host_timeout_secs),
            _ => u64::from(service.system_timeout_secs),
        };
        if let Some(run) = self.partitions.get_mut(&id).and_then(|p| p.run.as_mut()) {
            run.plan = Some(Plan {
                kind,
                restart,
                deadline: now + allowed,
            });
        }
        let (code, detail) = match kind {
            Shutdown::Delayed => (
                &refcode::SHUTDOWN_DELAYED,
                format!("{name}, delay {delay} s"),
            ),
            _ => (&refcode::SHUTDOWN_SYSTEM, name.clone()),
        };
        self.code(id, code.code, &code.text_with(&detail), true);
        self.set_state(id, State::ShuttingDown);
    }

    /// Begin a shut down of `kind`; immediate is done at once.
    pub fn begin_shut_down(
        &mut self,
        id: u32,
        kind: Shutdown,
        delay: u32,
        restart: bool,
    ) -> Answer {
        let Some(p) = self.partitions.get(&id) else {
            return Answer::Now(Err(vec![
                qvsmsg::PARTITION_MISSING.with(&[&id.to_string()]),
            ]));
        };
        let name = p.definition.name.clone();
        if kind == Shutdown::Immediate {
            return Answer::Now(self.end_one(id, restart).map(|()| String::new()));
        }
        let Some(run) = &p.run else {
            return Answer::Now(Err(vec![
                qvsmsg::PARTITION_NOT_ACTIVE.with(&[&name, p.state.word()]),
            ]));
        };
        if run.asked.is_some() || run.plan.is_some() {
            return Answer::Now(Err(vec![qvsmsg::PARTITION_ENDING.with(&[&name])]));
        }
        if delay > qsipart::service::MAX_DELAY {
            return Answer::Now(Err(vec![
                qvsmsg::DELAY_OUT_OF_RANGE
                    .with(&[&delay.to_string(), &qsipart::service::MAX_DELAY.to_string()]),
            ]));
        }
        if !p.service.can(qsipart::caps::POWER_DOWN) {
            return Answer::Now(Err(vec![qvsmsg::NO_CHANNEL.with(&[&name, kind.word()])]));
        }
        let ask = Ask::PowerDown {
            immediate: kind == Shutdown::System,
            delay: if kind == Shutdown::Delayed { delay } else { 0 },
            restart,
        };
        let asked = Asked::Shutdown {
            kind,
            delay,
            restart,
            host: false,
        };
        match self.ask(id, ask, asked) {
            Ok(rx) => Answer::Later(rx),
            Err(e) => Answer::Now(Err(e)),
        }
    }

    /// End `id` for the host's own shut down: through its system when
    /// it can be asked, else immediately.
    pub(crate) fn end_for_host(&mut self, id: u32) {
        let can = self.partitions.get(&id).is_some_and(|p| {
            p.service.can(qsipart::caps::POWER_DOWN)
                && p.service.complete
                && p.run.as_ref().is_some_and(|r| r.plan.is_none())
        });
        if !can {
            self.begin_end(id, false);
            return;
        }
        let ask = Ask::PowerDown {
            immediate: true,
            delay: 0,
            restart: false,
        };
        let asked = Asked::Shutdown {
            kind: Shutdown::System,
            delay: 0,
            restart: false,
            host: true,
        };
        if self.ask(id, ask, asked).is_err() {
            self.begin_end(id, false);
        }
    }

    /// The switches whose uplink is down now: each running partition
    /// with an adapter on one is told, and told again when it is back.
    pub fn uplinks(&mut self, down: &[String]) {
        let mut said = Vec::new();
        for (id, p) in &mut self.partitions {
            let Some(run) = p.run.as_mut() else {
                continue;
            };
            let Some(profile) = p.profiles.get(&run.profile) else {
                continue;
            };
            let on: Vec<String> = profile
                .ethernet
                .iter()
                .filter(|a| run.ports.contains(&(a.slot, true)))
                .filter_map(|a| a.switch.clone())
                .collect();
            for switch in &on {
                let is_down = down.contains(switch);
                let was_down = run.uplinks_down.contains(switch);
                if is_down && !was_down {
                    run.uplinks_down.push(switch.clone());
                    said.push((*id, true, switch.clone()));
                } else if !is_down && was_down {
                    run.uplinks_down.retain(|s| s != switch);
                    said.push((*id, false, switch.clone()));
                }
            }
        }
        for (id, lost, switch) in said {
            let name = self.name_of(id);
            let code = if lost {
                &refcode::ADAPTER_UPLINK_LOST
            } else {
                &refcode::ADAPTER_UPLINK_BACK
            };
            self.code(
                id,
                code.code,
                &code.text_with(&format!("{name}, switch {switch}")),
                true,
            );
        }
    }

    /// Whether `id`'s system reported its IPL complete, or the partition
    /// is no longer starting: auto start's wait.
    pub fn ipl_complete(&self, id: u32) -> bool {
        self.partitions
            .get(&id)
            .is_none_or(|p| p.service.complete || p.state != State::Starting || p.run.is_none())
    }

    /// What time does to the channels: pings, answers that did not come,
    /// a system without one, a terminal code without `down`, a shut down
    /// that did not end in time.
    pub(crate) fn service_tick(&mut self) {
        let now = self.system.now();
        let wait = u64::from(self.host.settings.service.channel_wait_secs);
        let ids: Vec<u32> = self.partitions.keys().copied().collect();
        for id in ids {
            let mut happened = Vec::new();
            let mut send = Vec::new();
            let Some(p) = self.partitions.get_mut(&id) else {
                continue;
            };
            // Held, dumped or waiting for space, its processors are
            // stopped by the host: it is not the system that is silent.
            let stopped = p.run.as_ref().is_some_and(|r| r.kept)
                || p.dumping.is_some()
                || p.current.as_ref().is_some_and(|c| {
                    c.source == Source::Host && c.code == refcode::PARTITION_HELD.code
                });
            if let Some(c) = p.service.channel.as_mut() {
                if stopped {
                    c.excuse(now);
                }
                (happened, send) = c.tick(now);
            }
            for frame in send {
                self.system.send(id, Port::Service, frame);
            }
            for h in happened {
                self.happened(id, h);
            }
            let Some(p) = self.partitions.get_mut(&id) else {
                continue;
            };
            let Some(run) = &p.run else {
                continue;
            };
            // The record says how long a system is given to open its
            // channel: none at all, for ever, or the setting's guess.
            let given = match p.service.recorded {
                Recorded::None => Some(0),
                Recorded::Has => None,
                Recorded::Unsaid => Some(wait),
            };
            let none_yet = !p.service.greeted
                && !p.service.facts.none
                && given.is_some_and(|given| {
                    p.service
                        .kernel_at
                        .is_some_and(|at| now.saturating_sub(at) >= given)
                });
            let terminal = !run.kept
                && p.service
                    .terminal
                    .as_ref()
                    .is_some_and(|(_, at)| now.saturating_sub(*at) >= TERMINAL_WAIT);
            let overdue = run.asked.is_none() && run.plan.is_some_and(|plan| now >= plan.deadline);
            let plan = run.plan;
            let name = p.definition.name.clone();
            if none_yet {
                p.service.facts.none = true;
                let code = &refcode::NO_CHANNEL;
                self.code(id, code.code, &code.text_with(&name), true);
                if self
                    .partitions
                    .get(&id)
                    .is_some_and(|p| p.state == State::Starting)
                {
                    self.set_state(id, State::Running);
                }
                self.publish_channel(id);
            }
            if terminal {
                self.keep_partition(id);
            }
            if overdue && let Some(plan) = plan {
                let allowed = match plan.kind {
                    Shutdown::Delayed => "delayed",
                    _ => "operating system",
                };
                let secs = self.host.settings.service;
                let given = match plan.kind {
                    Shutdown::Delayed => secs.delayed_grace_secs,
                    _ if self.ending.is_some() => secs.host_timeout_secs,
                    _ => secs.system_timeout_secs,
                };
                let message = qvsmsg::SHUTDOWN_FORCED.with(&[&name, allowed, &given.to_string()]);
                self.system.log(&message.to_string());
                if let Some(p) = self.partitions.get_mut(&id) {
                    p.message = Some(message);
                }
                let code = &refcode::SHUTDOWN_FORCED;
                self.code(id, code.code, &code.text_with(&name), true);
                self.begin_end(id, plan.restart);
            }
        }
    }
}
