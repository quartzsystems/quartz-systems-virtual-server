// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The machine over a kernel in memory.

use std::sync::{Arc, Mutex};

use super::*;

/// What the fake kernel was asked to do, in order.
type Log = Arc<Mutex<Vec<String>>>;

struct Fake {
    links: Vec<LinkInfo>,
    log: Log,
}

struct FakeDhcp(Log, String);

impl DhcpClient for FakeDhcp {
    fn stop(self: Box<Self>, release: bool) {
        let how = if release { "release" } else { "keep" };
        self.0
            .lock()
            .unwrap()
            .push(format!("dhcp stop {} {how}", self.1));
    }
}

impl Fake {
    fn say(&self, what: String) {
        self.log.lock().unwrap().push(what);
    }

    fn link(&mut self, name: &str) -> &mut LinkInfo {
        self.links.iter_mut().find(|l| l.name == name).unwrap()
    }
}

impl Kernel for Fake {
    fn links(&mut self) -> Vec<LinkInfo> {
        self.links.clone()
    }
    fn set_up(&mut self, link: &LinkInfo) -> Result<(), String> {
        self.say(format!("up {}", link.name));
        if let Some(l) = self.links.iter_mut().find(|l| l.index == link.index) {
            l.up = true;
        }
        Ok(())
    }
    fn vlan(&mut self, name: &str, parent: u32, id: u16) -> Result<u32, String> {
        self.say(format!("vlan {name} on {parent} id {id}"));
        let index = 100 + u32::from(id);
        let parent = self
            .links
            .iter()
            .find(|l| l.index == parent)
            .unwrap()
            .clone();
        self.links.push(LinkInfo {
            index,
            name: name.to_string(),
            physical: false,
            up: false,
            ..parent
        });
        Ok(index)
    }
    fn delete_link(&mut self, index: u32) {
        self.say(format!("delete link {index}"));
        self.links.retain(|l| l.index != index);
    }
    fn add_address(&mut self, index: u32, address: Cidr) -> Result<(), String> {
        self.say(format!("address {address} on {index}"));
        Ok(())
    }
    fn delete_address(&mut self, index: u32, address: Cidr) {
        self.say(format!("no address {address} on {index}"));
    }
    fn add_gateway(&mut self, index: u32, gateway: Ipv4Addr) -> Result<(), String> {
        self.say(format!("gateway {gateway} on {index}"));
        Ok(())
    }
    fn delete_gateway(&mut self, index: u32, gateway: Ipv4Addr) {
        self.say(format!("no gateway {gateway} on {index}"));
    }
    fn dhcp(
        &mut self,
        link: &LinkInfo,
        host_name: &str,
        previous: Option<Lease>,
    ) -> Result<Box<dyn DhcpClient>, String> {
        let again = if previous.is_some() { " again" } else { "" };
        self.say(format!("dhcp start {} as {host_name}{again}", link.name));
        Ok(Box::new(FakeDhcp(self.log.clone(), link.name.clone())))
    }
    fn resolver(&mut self, servers: &[IpAddr]) {
        let list: Vec<String> = servers.iter().map(ToString::to_string).collect();
        self.say(format!("dns {}", list.join(",")));
    }
}

fn eth(index: u32, name: &str, carrier: bool) -> LinkInfo {
    LinkInfo {
        index,
        name: name.to_string(),
        mac: [0x52, 0x54, 0, 0, 0, u8::try_from(index).unwrap()],
        physical: true,
        up: false,
        carrier,
    }
}

/// A machine, its kernel's log and what it reported.
fn machine(config: Network, links: Vec<LinkInfo>) -> (Manager<Fake>, Log, Log, Instant) {
    let log = Log::default();
    let said = Log::default();
    let (posts, notes) = (said.clone(), said.clone());
    let report = Reporter {
        post: Box::new(move |e| posts.lock().unwrap().push(format!("{e:?}"))),
        note: Box::new(move |n| notes.lock().unwrap().push(format!("note {n}"))),
    };
    let fake = Fake {
        links,
        log: log.clone(),
    };
    let now = Instant::now();
    let manager = Manager::new(fake, report, config, "HOST1".into(), now);
    (manager, log, said, now)
}

fn take(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

fn lease(addr: [u8; 4]) -> Lease {
    Lease {
        addr: addr.into(),
        mask: Ipv4Addr::new(255, 255, 255, 0),
        router: Some(Ipv4Addr::new(10, 0, 2, 2)),
        dns: vec![Ipv4Addr::new(10, 0, 2, 3)],
        domain: String::new(),
        server: Ipv4Addr::new(10, 0, 2, 2),
        boot: None,
        obtained: 0,
        secs: 86_400,
        t1: 43_200,
        t2: 75_600,
    }
}

fn static_config() -> Network {
    Network {
        mode: Mode::Static,
        address: Some("192.0.2.10/24".parse().unwrap()),
        gateway: Some(Ipv4Addr::new(192, 0, 2, 1)),
        dns: vec!["192.0.2.53".parse().unwrap()],
        ..Network::default()
    }
}

#[test]
fn dhcp_on_the_first_link_with_carrier() {
    let links = vec![eth(2, "eth0", false), eth(3, "eth1", true)];
    let (mut m, log, said, now) = machine(Network::default(), links);
    m.refresh(now);
    // Every candidate is brought up; the one with carrier is used.
    assert_eq!(
        take(&log),
        ["up eth0", "up eth1", "dhcp start eth1 as HOST1"]
    );
    assert_eq!(
        take(&said),
        [
            "InterfaceUp { interface: \"eth1\" }",
            "note none: waiting for DHCP on eth1"
        ]
    );
    assert!(!m.online());
    m.bound(&lease([10, 0, 2, 15]));
    assert_eq!(take(&log), ["dns 10.0.2.3"]);
    assert_eq!(
        take(&said),
        [
            "AddressAcquired { interface: \"eth1\", address: \"10.0.2.15/24\", gateway: Some(\"10.0.2.2\") }",
            "note 10.0.2.15/24 on eth1, gateway 10.0.2.2"
        ]
    );
    assert!(m.online());
    let status = m.status();
    assert_eq!(
        (
            status.interface.as_str(),
            status.carrier,
            status.address.as_str()
        ),
        ("eth1", true, "10.0.2.15/24")
    );
    // A renewal of the same lease says nothing new.
    m.bound(&lease([10, 0, 2, 15]));
    assert!(take(&said).is_empty());
}

#[test]
fn a_static_address_on_a_vlan_of_a_named_device() {
    let config = Network {
        device: Device::Name("eth0".into()),
        vlan: Some(100),
        ..static_config()
    };
    let (mut m, log, said, now) = machine(config, vec![eth(2, "eth0", true), eth(3, "eth1", true)]);
    m.refresh(now);
    assert_eq!(
        take(&log),
        [
            "up eth0",
            "vlan eth0.100 on 2 id 100",
            "up eth0.100",
            "address 192.0.2.10/24 on 200",
            "gateway 192.0.2.1 on 200",
            "dns 192.0.2.53",
        ]
    );
    assert_eq!(
        take(&said),
        [
            "InterfaceUp { interface: \"eth0.100\" }",
            "AddressAcquired { interface: \"eth0.100\", address: \"192.0.2.10/24\", gateway: Some(\"192.0.2.1\") }",
            "note 192.0.2.10/24 on eth0.100, gateway 192.0.2.1"
        ]
    );
    assert_eq!(vlan_name("enp130s0f1np1", 4094), "vlan4094");
}

#[test]
fn carrier_loss_and_return_are_followed_without_a_restart() {
    let (mut m, log, said, now) = machine(Network::default(), vec![eth(2, "eth0", true)]);
    m.refresh(now);
    m.bound(&lease([10, 0, 2, 15]));
    take(&log);
    take(&said);

    m.kernel().link("eth0").carrier = false;
    m.refresh(now + Duration::from_secs(5));
    assert_eq!(take(&log), ["dhcp stop eth0 keep"]);
    assert_eq!(take(&said), ["note none: no carrier on eth0"]);
    assert!(!m.online());
    // Thirty seconds without carrier is posted, once.
    m.refresh(now + Duration::from_secs(34));
    assert!(take(&said).is_empty());
    m.refresh(now + Duration::from_secs(35));
    m.refresh(now + Duration::from_secs(60));
    assert_eq!(take(&said), ["NoCarrier { interface: \"eth0\" }"]);

    // It returns: the client starts from the lease it had.
    m.kernel().link("eth0").carrier = true;
    m.refresh(now + Duration::from_secs(70));
    assert_eq!(take(&log), ["dhcp start eth0 as HOST1 again"]);
    m.bound(&lease([10, 0, 2, 15]));
    assert_eq!(
        take(&said),
        [
            "InterfaceUp { interface: \"eth0\" }",
            "note none: waiting for DHCP on eth0",
            "AddressAcquired { interface: \"eth0\", address: \"10.0.2.15/24\", gateway: Some(\"10.0.2.2\") }",
            "note 10.0.2.15/24 on eth0, gateway 10.0.2.2"
        ]
    );
}

#[test]
fn no_address_and_no_interface_are_posted_after_thirty_seconds() {
    // Carrier, and a DHCP server that never answers.
    let (mut m, _log, said, now) = machine(Network::default(), vec![eth(2, "eth0", true)]);
    m.refresh(now);
    m.refresh(now + Duration::from_secs(29));
    take(&said);
    m.refresh(now + Duration::from_secs(30));
    m.refresh(now + Duration::from_secs(90));
    assert_eq!(take(&said), ["NoAddress { interface: \"eth0\" }"]);

    // No link at all.
    let (mut m, _log, said, now) = machine(Network::default(), Vec::new());
    m.refresh(now);
    assert_eq!(take(&said), ["note none: no Ethernet link with carrier"]);
    m.refresh(now + Duration::from_secs(30));
    assert_eq!(
        take(&said),
        ["NoInterface { wanted: \"Ethernet link with carrier\" }"]
    );
    // One appears later and is taken.
    m.kernel().links.push(eth(2, "eth0", true));
    m.refresh(now + Duration::from_secs(40));
    assert_eq!(m.status().interface, "eth0");

    // A named device that is not there.
    let config = Network {
        device: Device::Name("eth7".into()),
        ..Network::default()
    };
    let (mut m, _log, said, now) = machine(config, vec![eth(2, "eth0", true)]);
    m.refresh(now);
    assert_eq!(take(&said), ["note none: no interface eth7"]);
}

#[test]
fn a_reload_undoes_the_old_interface_and_makes_the_new() {
    let config = Network {
        vlan: Some(7),
        ..static_config()
    };
    let (mut m, log, said, now) = machine(config.clone(), vec![eth(2, "eth0", true)]);
    m.refresh(now);
    take(&log);
    take(&said);
    // The same configuration: nothing happens.
    m.reload(config, "HOST1".into(), now);
    assert!(take(&log).is_empty());

    m.reload(Network::default(), "HOST1".into(), now);
    assert_eq!(
        take(&log),
        [
            "no gateway 192.0.2.1 on 107",
            "no address 192.0.2.10/24 on 107",
            "delete link 107",
            "dhcp start eth0 as HOST1",
        ]
    );
    assert_eq!(
        take(&said),
        [
            "InterfaceUp { interface: \"eth0\" }",
            "note none: waiting for DHCP on eth0"
        ]
    );
    m.bound(&lease([10, 0, 2, 15]));
    take(&log);
    // And back to static: the lease is released.
    m.reload(static_config(), "HOST1".into(), now);
    assert_eq!(
        take(&log)[..2],
        ["dhcp stop eth0 release", "address 192.0.2.10/24 on 2"]
    );
}

#[test]
fn a_device_that_goes_away_takes_the_interface_with_it() {
    let config = Network {
        device: Device::Mac([0x52, 0x54, 0, 0, 0, 2]),
        ..Network::default()
    };
    let (mut m, log, said, now) = machine(config, vec![eth(2, "eth0", true)]);
    m.refresh(now);
    m.bound(&lease([10, 0, 2, 15]));
    take(&log);
    take(&said);
    m.kernel().links.clear();
    m.refresh(now + Duration::from_secs(1));
    assert_eq!(take(&log), ["dhcp stop eth0 keep"]);
    assert_eq!(
        take(&said),
        ["note none: no interface with address 52:54:00:00:00:02"]
    );
}

#[test]
fn a_stopped_client_s_last_word_does_not_take_a_static_address_away() {
    let (mut m, log, said, now) = machine(Network::default(), vec![eth(2, "eth0", true)]);
    m.refresh(now);
    m.bound(&lease([10, 0, 2, 15]));
    take(&log);
    take(&said);
    // The same address, now static: the client is stopped, the address set.
    let fixed = Network {
        mode: Mode::Static,
        address: Some("10.0.2.15/24".parse().unwrap()),
        gateway: Some(Ipv4Addr::new(10, 0, 2, 2)),
        ..Network::default()
    };
    m.reload(fixed, "HOST1".into(), now);
    assert!(take(&log).contains(&"dhcp stop eth0 release".to_string()));
    assert!(m.online());
    // The stopped client's word that its lease is off arrives afterwards.
    m.unbound(now);
    assert!(m.online());
    assert_eq!(m.line(), "10.0.2.15/24 on eth0, gateway 10.0.2.2");
    // A client that is running and loses its lease is believed.
    m.reload(Network::default(), "HOST1".into(), now);
    m.bound(&lease([10, 0, 2, 15]));
    assert!(m.online());
    m.unbound(now);
    assert!(!m.online());
}
