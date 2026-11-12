// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The management interface as a machine: which link, whether it has
//! carrier, what address it has, and what to say when it has neither
//! for too long. The kernel is behind [`Kernel`], so the machine is
//! tested without one.

#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use appliance::Event;
use dhcp::Lease;
use state::{Cidr, Device, Mode, Network};

use crate::control::NetStatus;

/// How long the interface may be without carrier, or without an
/// address, before the host says so.
pub const ATTENTION_AFTER: Duration = Duration::from_secs(30);

/// One link, as the machine needs to see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInfo {
    /// Interface index.
    pub index: u32,
    /// Name.
    pub name: String,
    /// Hardware address.
    pub mac: [u8; 6],
    /// An Ethernet link with a device behind it.
    pub physical: bool,
    /// Administratively up.
    pub up: bool,
    /// Carrier present.
    pub carrier: bool,
}

/// A running DHCP client.
pub trait DhcpClient {
    /// End it; `release` gives the address back to the server.
    fn stop(self: Box<Self>, release: bool);
}

/// What the machine asks of the kernel.
pub trait Kernel {
    /// Every link.
    fn links(&mut self) -> Vec<LinkInfo>;
    /// Bring a link up, with router advertisements ignored: IPv6 is
    /// link-local only.
    fn set_up(&mut self, link: &LinkInfo) -> Result<(), String>;
    /// The 802.1Q sub-interface `name` of `parent`, created if it is
    /// not there; its index.
    fn vlan(&mut self, name: &str, parent: u32, id: u16) -> Result<u32, String>;
    /// Delete a link this machine created.
    fn delete_link(&mut self, index: u32);
    /// Put an address on a link.
    fn add_address(&mut self, index: u32, address: Cidr) -> Result<(), String>;
    /// Take it off.
    fn delete_address(&mut self, index: u32, address: Cidr);
    /// Set the default route.
    fn add_gateway(&mut self, index: u32, gateway: Ipv4Addr) -> Result<(), String>;
    /// Remove it.
    fn delete_gateway(&mut self, index: u32, gateway: Ipv4Addr);
    /// Start a DHCP client on a link. It puts the lease's address and
    /// router on the link itself and reports through
    /// [`Manager::bound`] and [`Manager::unbound`].
    fn dhcp(
        &mut self,
        link: &LinkInfo,
        host_name: &str,
        previous: Option<Lease>,
    ) -> Result<Box<dyn DhcpClient>, String>;
    /// Record the name servers for the resolver.
    fn resolver(&mut self, servers: &[IpAddr]);
}

/// What the machine reports through.
pub struct Reporter {
    /// Post an event.
    pub post: Box<dyn FnMut(&Event<'_>) + Send>,
    /// The status screen's management line.
    pub note: Box<dyn FnMut(&str) + Send>,
}

/// The link in use.
struct Active {
    /// The device.
    base: LinkInfo,
    /// The interface the address goes on: the device, or its VLAN.
    iface: LinkInfo,
    /// Whether the VLAN sub-interface is this machine's to delete.
    made_vlan: bool,
    carrier: bool,
    dhcp: Option<Box<dyn DhcpClient>>,
    /// The static address and gateway put on the interface.
    applied: Option<(Cidr, Option<Ipv4Addr>)>,
}

/// The management interface.
pub struct Manager<K: Kernel> {
    kernel: K,
    report: Reporter,
    config: Network,
    host_name: String,
    active: Option<Active>,
    /// The address in use and its gateway.
    address: Option<(Cidr, Option<Ipv4Addr>)>,
    /// The last lease, for the next start of the client.
    lease: Option<Lease>,
    /// Since when the interface has lacked what it lacks now.
    since: Instant,
    /// Whether that lack has been posted.
    warned: bool,
    noted: String,
}

/// The name of a VLAN sub-interface: `eth0.100`, or `vlan100` when
/// that is longer than an interface name may be.
pub fn vlan_name(parent: &str, id: u16) -> String {
    let name = format!("{parent}.{id}");
    if name.len() <= 15 {
        name
    } else {
        format!("vlan{id}")
    }
}

impl<K: Kernel> Manager<K> {
    /// A machine for `config`; [`Manager::refresh`] starts it.
    pub fn new(
        kernel: K,
        report: Reporter,
        config: Network,
        host_name: String,
        now: Instant,
    ) -> Self {
        Self {
            kernel,
            report,
            config,
            host_name,
            active: None,
            address: None,
            lease: None,
            since: now,
            warned: false,
            noted: String::new(),
        }
    }

    /// What `config.device` asks for, in words.
    fn wanted(&self) -> String {
        match &self.config.device {
            Device::Auto => "Ethernet link with carrier".into(),
            Device::Name(n) => format!("interface {n}"),
            Device::Mac(_) => format!("interface with address {}", self.config.device),
        }
    }

    /// Choose the device and make the interface on it.
    fn choose(&mut self, links: &[LinkInfo]) {
        let candidates: Vec<&LinkInfo> = links
            .iter()
            .filter(|l| match &self.config.device {
                Device::Auto => l.physical,
                Device::Name(n) => l.name == *n,
                Device::Mac(m) => l.physical && l.mac == *m,
            })
            .collect();
        // Carrier shows only on a link that is up.
        for link in &candidates {
            if !link.up
                && let Err(e) = self.kernel.set_up(link)
            {
                eprintln!("{}: cannot be brought up: {e}", link.name);
            }
        }
        let pick = match self.config.device {
            Device::Auto => candidates.iter().find(|l| l.carrier),
            _ => candidates.first(),
        };
        let Some(base) = pick.map(|l| (*l).clone()) else {
            return;
        };
        let (iface, made_vlan) = match self.config.vlan {
            None => (base.clone(), false),
            Some(id) => {
                let name = vlan_name(&base.name, id);
                let existed = links.iter().any(|l| l.name == name);
                match self.kernel.vlan(&name, base.index, id) {
                    Ok(index) => {
                        let iface = LinkInfo {
                            index,
                            name,
                            physical: false,
                            up: false,
                            carrier: false,
                            ..base.clone()
                        };
                        if let Err(e) = self.kernel.set_up(&iface) {
                            eprintln!("{}: cannot be brought up: {e}", iface.name);
                        }
                        (iface, !existed)
                    }
                    Err(e) => {
                        eprintln!("{name}: cannot be created: {e}");
                        return;
                    }
                }
            }
        };
        self.active = Some(Active {
            base,
            iface,
            made_vlan,
            carrier: false,
            dhcp: None,
            applied: None,
        });
    }

    /// Carrier came: say so and get an address.
    fn carrier_up(&mut self, now: Instant) {
        let Some(active) = &mut self.active else {
            return;
        };
        active.carrier = true;
        self.since = now;
        self.warned = false;
        let iface = active.iface.clone();
        (self.report.post)(&Event::InterfaceUp {
            interface: &iface.name,
        });
        match (self.config.mode, self.config.address) {
            (Mode::Static, Some(address)) => {
                let gateway = self.config.gateway;
                let done = self
                    .kernel
                    .add_address(iface.index, address)
                    .and_then(|()| {
                        gateway.map_or(Ok(()), |g| self.kernel.add_gateway(iface.index, g))
                    });
                match done {
                    Ok(()) => {
                        if let Some(active) = &mut self.active {
                            active.applied = Some((address, gateway));
                        }
                        self.acquired(address, gateway, &[]);
                    }
                    Err(e) => eprintln!("{}: {address}: {e}", iface.name),
                }
            }
            _ => match self
                .kernel
                .dhcp(&iface, &self.host_name, self.lease.clone())
            {
                Ok(client) => {
                    if let Some(active) = &mut self.active {
                        active.dhcp = Some(client);
                    }
                }
                Err(e) => eprintln!("{}: DHCP: {e}", iface.name),
            },
        }
    }

    /// Carrier went: a lease is no longer trusted; a static address stays.
    fn carrier_down(&mut self, now: Instant) {
        let Some(active) = &mut self.active else {
            return;
        };
        active.carrier = false;
        if let Some(client) = active.dhcp.take() {
            client.stop(false);
        }
        self.address = None;
        self.since = now;
        self.warned = false;
    }

    fn acquired(&mut self, address: Cidr, gateway: Option<Ipv4Addr>, lease_dns: &[IpAddr]) {
        let mut dns = self.config.dns.clone();
        dns.extend(lease_dns.iter().filter(|d| !self.config.dns.contains(d)));
        self.kernel.resolver(&dns);
        let changed = self.address != Some((address, gateway));
        self.address = Some((address, gateway));
        self.warned = false;
        if changed && let Some(active) = &self.active {
            (self.report.post)(&Event::AddressAcquired {
                interface: &active.iface.name,
                address: &address.to_string(),
                gateway: gateway.map(|g| g.to_string()).as_deref(),
            });
        }
    }

    /// The DHCP client has put a lease on the interface.
    pub fn bound(&mut self, lease: &Lease) {
        if self.active.as_ref().is_none_or(|a| a.dhcp.is_none()) {
            return;
        }
        let address = Cidr {
            addr: lease.addr,
            prefix: lease.prefix(),
        };
        let dns: Vec<IpAddr> = lease.dns.iter().copied().map(IpAddr::V4).collect();
        self.lease = Some(lease.clone());
        self.acquired(address, lease.router, &dns);
        self.note();
    }

    /// The DHCP client has taken its lease off the interface.
    pub fn unbound(&mut self, now: Instant) {
        if self.address.take().is_some() {
            self.since = now;
            self.warned = false;
        }
        self.note();
    }

    /// Undo everything this machine did to the kernel.
    fn teardown(&mut self, release: bool) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        if let Some(client) = active.dhcp.take() {
            client.stop(release);
        }
        if let Some((address, gateway)) = active.applied {
            if let Some(g) = gateway {
                self.kernel.delete_gateway(active.iface.index, g);
            }
            self.kernel.delete_address(active.iface.index, address);
        }
        if active.made_vlan {
            self.kernel.delete_link(active.iface.index);
        }
        self.address = None;
    }

    /// Look at the links and do what follows: choose the device, take
    /// carrier changes, and post what has been lacking too long. Called
    /// on every link event and at least every second.
    pub fn refresh(&mut self, now: Instant) {
        let links = self.kernel.links();
        // A device that went away takes the interface with it.
        if let Some(active) = &self.active
            && !links.iter().any(|l| l.index == active.base.index)
        {
            self.teardown(false);
            self.since = now;
            self.warned = false;
        }
        if self.active.is_none() {
            self.choose(&links);
        }
        let carrier = self.active.as_ref().and_then(|a| {
            links
                .iter()
                .find(|l| l.index == a.base.index)
                .map(|l| (a.carrier, l.carrier))
        });
        match carrier {
            Some((false, true)) => self.carrier_up(now),
            Some((true, false)) => self.carrier_down(now),
            _ => {}
        }
        if !self.warned && now.saturating_duration_since(self.since) >= ATTENTION_AFTER {
            let wanted = self.wanted();
            let event = match &self.active {
                None => Some(Event::NoInterface { wanted: &wanted }),
                Some(a) if !a.carrier => Some(Event::NoCarrier {
                    interface: &a.iface.name,
                }),
                Some(a) if self.address.is_none() => Some(Event::NoAddress {
                    interface: &a.iface.name,
                }),
                Some(_) => None,
            };
            if let Some(event) = event {
                (self.report.post)(&event);
                self.warned = true;
            }
        }
        self.note();
    }

    /// Take a new configuration: the old interface is undone and the
    /// new one made. Nothing happens when nothing changed.
    pub fn reload(&mut self, config: Network, host_name: String, now: Instant) {
        if config == self.config && host_name == self.host_name {
            return;
        }
        self.teardown(true);
        self.config = config;
        self.host_name = host_name;
        self.lease = None;
        self.since = now;
        self.warned = false;
        self.refresh(now);
    }

    /// The configuration in use.
    pub fn config(&self) -> &Network {
        &self.config
    }

    /// The management line: the address, or why there is none.
    pub fn line(&self) -> String {
        match (&self.active, &self.address) {
            (None, _) => format!("none: no {}", self.wanted()),
            (Some(a), _) if !a.carrier => format!("none: no carrier on {}", a.iface.name),
            (Some(a), None) => match self.config.mode {
                Mode::Dhcp => format!("none: waiting for DHCP on {}", a.iface.name),
                Mode::Static => format!("none: the address could not be set on {}", a.iface.name),
            },
            (Some(a), Some((address, gateway))) => {
                let via = gateway.map_or_else(String::new, |g| format!(", gateway {g}"));
                format!("{address} on {}{via}", a.iface.name)
            }
        }
    }

    fn note(&mut self) {
        let line = self.line();
        if line != self.noted {
            (self.report.note)(&line);
            self.noted = line;
        }
    }

    /// Whether the interface has its address.
    pub fn online(&self) -> bool {
        self.address.is_some()
    }

    /// What the service's status request answers.
    pub fn status(&self) -> NetStatus {
        NetStatus {
            interface: self
                .active
                .as_ref()
                .map_or_else(String::new, |a| a.iface.name.clone()),
            carrier: self.active.as_ref().is_some_and(|a| a.carrier),
            address: self
                .address
                .map_or_else(String::new, |(a, _)| a.to_string()),
            gateway: self
                .address
                .and_then(|(_, g)| g)
                .map_or_else(String::new, |g| g.to_string()),
            line: self.line(),
        }
    }

    /// The kernel, for the tests.
    #[cfg(test)]
    fn kernel(&mut self) -> &mut K {
        &mut self.kernel
    }
}
