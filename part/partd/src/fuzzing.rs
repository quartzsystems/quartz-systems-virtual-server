// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The fuzz target of the partition manager's handling of what a system
//! sends on its service channel and console port: the manager itself,
//! over a machine that does nothing, with one partition running. The
//! input, in pieces, is what the system sent; time passes between them.
//! Nothing may panic, and what is kept stays within its bounds.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock, PoisonError};

use poold::proto::{Given, Want};
use qvsmsg::Issued;
use resource::cgroup::Step;
use resource::{Host, Settings, Topology};

use crate::manager::{DumpOrder, HISTORY, Manager, Port, Record, System};
use crate::proto::{DumpInfo, Partitions as _, Posted};
use crate::store::Store;

/// A machine that does nothing.
#[derive(Default)]
pub struct Null {
    now: u64,
    sent: usize,
}

impl System for Null {
    fn now(&self) -> u64 {
        self.now
    }
    fn apply(&mut self, _: &[Step]) -> Result<(), String> {
        Ok(())
    }
    fn remove_group(&mut self, _: &str) {}
    fn launch(&mut self, _: u32, _: &str, _: &str) -> Result<u32, String> {
        Ok(1)
    }
    fn end(&mut self, _: u32) {}
    fn kill(&mut self, _: &str) {}
    fn post(&mut self, _: &str, _: &str) {}
    fn record(&mut self, _: u32, _: Option<&Record>) {}
    fn log(&mut self, _: &str) {}
    fn exists(&self, _: &str) -> bool {
        true
    }
    fn hold(&mut self, _: u32, _: &[Want]) -> Result<Vec<Given>, Vec<Issued>> {
        Ok(Vec::new())
    }
    fn give_back(&mut self, _: u32, _: &[Want]) {}
    fn control(&mut self, _: u32, _: &str, _: Option<&Want>) -> Result<String, String> {
        Ok("ok".into())
    }
    fn host_paths(&self) -> bool {
        true
    }
    fn delete_vdisks(&mut self, _: &[(String, String)]) -> Vec<Issued> {
        Vec::new()
    }
    fn switches(&mut self) -> Option<Vec<vswitch::Switch>> {
        None
    }
    fn open_port(&mut self, _: u32, _: &vswitch::port::PortSpec) -> Result<u32, Vec<Issued>> {
        Ok(1500)
    }
    fn close_ports(&mut self, _: u32) {}
    fn reconcile_ports(&mut self, _: &[u32]) {}
    fn channel_sockets(
        &mut self,
        _: u32,
        _: usize,
    ) -> Result<Vec<qsvmm::config::ConsolePort>, String> {
        Ok(Vec::new())
    }
    fn send(&mut self, _: u32, _: Port, frame: Vec<u8>) {
        assert!(frame.len() <= qsipart::frame::HEADER + qsipart::frame::MAX_BODY);
        self.sent += 1;
    }
    fn close_channel(&mut self, _: u32, _: Port) {}
    fn keep(&mut self, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn dump_dir(&mut self, _: u32) -> Option<PathBuf> {
        None
    }
    fn begin_dump(&mut self, _: u32, _: &DumpOrder) -> Result<(), String> {
        Ok(())
    }
    fn dumps(&mut self, _: u32) -> Vec<DumpInfo> {
        Vec::new()
    }
    fn remove_dump(&mut self, _: u32, _: &str) -> bool {
        false
    }
    fn keep_history(&mut self, _: u32, history: &[Posted]) {
        assert!(history.len() <= HISTORY);
    }
    fn kept_history(&mut self, _: u32) -> Vec<Posted> {
        Vec::new()
    }
}

fn manager() -> &'static Mutex<(Manager<Null>, u32)> {
    static ONE: OnceLock<Mutex<(Manager<Null>, u32)>> = OnceLock::new();
    ONE.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("partd-fuzz-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let host = Host {
            topology: Topology::uniform(8, 2),
            installed_mib: 8192,
            configurable_mib: 6144,
            settings: Settings::default(),
        };
        let mut m = Manager::new(Null::default(), Store::at(&dir), host, "FUZZ01");
        let id = m.create("FUZZ".into()).map_or(1, |d| d.id);
        let mut profile = model::sample::dedicated();
        profile.scsi[0].luns[0].backing = "/dev/fuzz".into();
        let _ = m.put_profile(id, profile);
        Mutex::new((m, id))
    })
}

/// One input. The first byte chooses the piece size; the second, which
/// port the pieces go to.
pub fn frames(data: &[u8]) {
    let Some((&size, rest)) = data.split_first() else {
        return;
    };
    let Some((&ports, rest)) = rest.split_first() else {
        return;
    };
    let mut guard = manager().lock().unwrap_or_else(PoisonError::into_inner);
    let (m, id) = &mut *guard;
    let id = *id;
    if m.activation(id).is_none() {
        let _ = m.activate(id, String::new(), None, None);
    }
    m.channel_up(id, Port::Service);
    m.channel_up(id, Port::Console);
    let piece = usize::from(size % 97) + 1;
    for (i, chunk) in rest.chunks(piece).enumerate() {
        let port = if ports >> (i % 8) & 1 == 0 {
            Port::Service
        } else {
            Port::Console
        };
        m.channel_bytes(id, port, chunk);
        m.system().now += 1;
        m.tick();
    }
    let status = m.list();
    for s in &status {
        assert!(s.code.len() <= 16 && s.text.len() <= 1024);
        assert!(s.service.addresses.len() <= 32);
    }
    assert!(m.history(id).map_or(0, |h| h.len()) <= HISTORY);
    let _ = m.screen(id);
    m.channel_down(id, Port::Service);
    m.channel_down(id, Port::Console);
    // An activation that ended under the input begins again next time.
    if m.activation(id)
        .is_some_and(|_| m.list().iter().any(|s| s.id == id && !s.state.active()))
    {
        m.ended(id, crate::manager::Ended::Exit(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsipart::service::FromSystem;

    #[test]
    fn seeds_and_noise_go_through() {
        let mut seed = vec![7u8, 0];
        seed.extend(
            FromSystem::State(qsipart::service::State {
                state: qsipart::service::SysState::Down,
                fix_level: String::new(),
                addresses: vec!["x".into(); 40],
            })
            .encode()
            .unwrap(),
        );
        frames(&seed);
        frames(&[]);
        let mut x: u32 = 9;
        let noise: Vec<u8> = (0..30_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                if x.is_multiple_of(5) {
                    b'Q'
                } else {
                    x.to_le_bytes()[2]
                }
            })
            .collect();
        frames(&noise);
    }
}
