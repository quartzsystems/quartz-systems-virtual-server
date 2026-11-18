// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The readers of bytes that come from outside a guest but are no more
//! to be trusted: a seed volume and its answers, any disk put in the
//! host (another host's load source or pool member among them), and a
//! console that has not yet paired (`docs/SECURITY.md`).

use std::io::Cursor;

use abrec::AbPart;
use install::{Answers, DiskChoice, Step, Target as Install};
use panels::forms::LinkChoice;
use uuid::Uuid;

/// A machine with two disks, for the answers to be held against.
struct Machine;

impl Install for Machine {
    fn product(&self) -> &'static str {
        "QS/VS"
    }
    fn banner(&self) -> String {
        String::new()
    }
    fn what(&self) -> &'static [&'static str] {
        &[]
    }
    fn disks(&mut self) -> Vec<DiskChoice> {
        ["S4EVNX0M702341", "S4EVNX0M702342"]
            .iter()
            .enumerate()
            .map(|(i, serial)| DiskChoice {
                name: format!("sd{}", char::from(b'a' + u8::try_from(i).unwrap_or(0))),
                location: format!("P0000-B03-D00-F0-U{i}"),
                model: "DISK".into(),
                serial: (*serial).into(),
                bytes: 64 << 30,
            })
            .collect()
    }
    fn min_disk_bytes(&self) -> u64 {
        8 << 30
    }
    fn links(&mut self) -> Vec<LinkChoice> {
        Vec::new()
    }
    fn default_host(&self) -> String {
        "vs".into()
    }
    fn now(&self) -> i64 {
        0
    }
    fn steps(&self) -> &'static [Step] {
        &[]
    }
    fn step(&mut self, _: usize, _: &Answers) -> Result<(), String> {
        Ok(())
    }
    fn failed(&self) -> &'static str {
        ""
    }
    fn log(&mut self, _: &str) {}
}

/// `seed.toml` from a removable volume, and its `[install]` table held
/// against the machine.
pub fn seed(data: &[u8]) {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(seed) = seed::format::parse(text) {
        let _ = install::answers::from_seed(&seed, &mut Machine);
    }
}

/// Seeds of the `seed` target: a seed of each version.
pub fn seed_seeds() -> Vec<Vec<u8>> {
    vec![
        b"schema_version = 1\n\n[host]\nname = \"vs-01\"\ntime_zone = \"UTC+01:00\"\n\
ntp_servers = [\"time1.example.net\", \"192.0.2.123:123\"]\n\n[network]\n\
device = \"52:54:00:12:34:56\"\nmode = \"static\"\naddress = \"192.0.2.10/24\"\n\
gateway = \"192.0.2.1\"\nvlan = 100\ndns = [\"192.0.2.53\"]\n"
            .to_vec(),
        b"schema_version = 2\n\n[install]\ndisks = [\"S4EVNX0M702341\", \"P0000-B03-D00-F0-U1\"]\n\
protection = \"mirrored\"\nuser_id = \"ADMIN\"\n\
password_hash = \"$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHRzYWx0$YWJjZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXo\"\n\
\n[host]\nname = \"vs-01\"\n\n[network]\nmode = \"dhcp\"\n"
            .to_vec(),
    ]
}

/// A pool disk's record partition: its labels, as the pool service
/// reads every disk it finds, and a disk's first bytes as it tells
/// what a disk is.
pub fn pool_label(data: &[u8]) {
    let names = &release::POOL_DISK;
    let _ = pool::disk::signature(data);
    let _ = pool::label::PoolLabel::decode(data);
    let _ = pool::disk::read_label(names, Cursor::new(data), 0);
}

/// Seeds of the `pool-label` target: a label, and a record partition holding it.
pub fn pool_label_seeds() -> Vec<Vec<u8>> {
    let names = &release::POOL_DISK;
    let label = pool::label::PoolLabel {
        host: [7; 16],
        pool: Uuid::from_bytes([1; 16]),
        name: "POOL1".into(),
        protection: pool::label::Protection::Raid5,
        set_blocks: 1 << 20,
        member_blocks: 1 << 19,
        parity: (0, 0),
        sync: Some(4096),
        clean: true,
        written: 3,
        generation: 9,
        units: (0..3u8)
            .map(|i| pool::label::Unit {
                partition: Uuid::from_bytes([i + 2; 16]),
                state: pool::label::UnitState::Active,
                serial: format!("SERIAL{i}"),
                location: format!("P0000-B01-D0{i}"),
            })
            .collect(),
    };
    let payload = label.encode();
    let spec = pool::label::spec(names);
    let mut part = AbPart::open(spec, Cursor::new(vec![0u8; 2 * spec.slot_bytes]));
    let mut out = vec![payload];
    if part.init(&out[0]).is_ok() {
        let mut bytes = part.into_device().into_inner();
        bytes.truncate(64 << 10);
        out.push(bytes);
    }
    out
}

/// A load-source disk's record partition: the A/B record and the
/// mirrored set's label, as the host reads every disk at its IPL.
pub fn load_source_label(data: &[u8]) {
    let names = &release::LOAD_SOURCE;
    let _ = loadsource::record::read_bytes(names, data);
    let _ = loadsource::record::HostRecord::decode(data);
    let _ = loadsource::set::SetLabel::decode(data);
}

/// Seeds of the `ls-label` target: a record partition, a record, a set label.
pub fn load_source_label_seeds() -> Vec<Vec<u8>> {
    let names = &release::LOAD_SOURCE;
    let mut out = Vec::new();
    if let Ok(mut bytes) =
        loadsource::record::initial_bytes(names, loadsource::record::HostRecord::INITIAL)
    {
        bytes.truncate(64 << 10);
        out.push(bytes);
    }
    out.push(loadsource::record::HostRecord::INITIAL.encode().to_vec());
    let label = loadsource::set::SetLabel {
        id: Uuid::from_bytes([3; 16]),
        blocks: 1 << 22,
        clean: false,
        written: 12,
        members: (0..2u8)
            .map(|i| loadsource::set::Member {
                partition: Uuid::from_bytes([i + 5; 16]),
                state: loadsource::set::MemberState::Active,
                serial: format!("S{i}"),
                location: format!("P0000-B03-D00-F0-U{i}"),
            })
            .collect(),
    };
    out.push(label.encode());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seeds_are_read_as_valid() {
        let text = String::from_utf8(seed_seeds()[1].clone()).unwrap();
        let parsed = seed::format::parse(&text).unwrap();
        assert!(parsed.install.is_some());
        let label = &pool_label_seeds()[0];
        assert!(pool::label::PoolLabel::decode(label).is_some());
        let set = load_source_label_seeds().pop().unwrap();
        assert!(loadsource::set::SetLabel::decode(&set).is_some());
        let record = &load_source_label_seeds()[0];
        assert!(loadsource::record::read_bytes(&release::LOAD_SOURCE, record).is_ok());
    }
}
