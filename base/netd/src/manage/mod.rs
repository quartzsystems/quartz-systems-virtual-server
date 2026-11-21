// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The interfaces of `network.toml` as a machine: for each, which link,
//! whether it has carrier, what address it has, and what to say when it
//! has neither for too long. The first is the management interface; the
//! others are run the same way, each on its own. The kernel is behind
//! [`Kernel`], so the machine is tested without one.

#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use appliance::Event;
use dhcp::Lease;
use state::{Cidr, Device, Interface, Mode, Network};

use crate::control::NetStatus;

/// How long an interface may be without carrier, or without an
/// address, before the host says so.
pub const ATTENTION_AFTER: Duration = Duration::from_secs(30);

/// One link, as the machine needs to see it.
// Each flag is a fact of its own about the link.
#[allow(clippy::struct_excessive_bools)]
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
    /// Another's: on a bridge or a bond, or reserved by the appliance
    /// for other use. Never one of these interfaces.
    pub taken: bool,
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
    /// Start a DHCP client on a link. It puts the lease's address on
    /// the link itself, and its router as the default route when
    /// `route`, and reports through [`Manager::bound`] and
    /// [`Manager::unbound`] with the link's index.
    fn dhcp(
        &mut self,
        link: &LinkInfo,
        host_name: &str,
        previous: Option<Lease>,
        route: bool,
    ) -> Result<Box<dyn DhcpClient>, String>;
    /// Record the name servers for the resolver.
    fn resolver(&mut self, servers: &[IpAddr]);
}

/// How an interface's status line is told: its role as written, and
/// the line.
pub type Note = Box<dyn FnMut(&str, &str) + Send>;

/// What the machine reports through.
pub struct Reporter {
    /// Post an event.
    pub post: Box<dyn FnMut(&Event<'_>) + Send>,
    /// An interface's status line: its role as written, and the line.
    pub note: Note,
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

/// One interface.
struct Iface {
    config: Interface,
    active: Option<Active>,
    /// The address in use and its gateway.
    address: Option<(Cidr, Option<Ipv4Addr>)>,
    /// The last lease, for the next start of the client.
    lease: Option<Lease>,
    /// The name servers of the lease in use.
    lease_dns: Vec<IpAddr>,
    /// Since when the interface has lacked what it lacks now.
    since: Instant,
    /// Whether that lack has been posted.
    warned: bool,
    noted: String,
}

impl Iface {
    fn new(config: Interface, now: Instant) -> Self {
        Self {
            config,
            active: None,
            address: None,
            lease: None,
            lease_dns: Vec::new(),
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

    /// Its status line: the address, or why there is none.
    fn line(&self) -> String {
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

    fn status(&self) -> NetStatus {
        NetStatus {
            role: self.config.role.clone(),
            interface: self
                .active
                .as_ref()
                .map_or_else(String::new, |a| a.iface.name.clone()),
            device: self
                .active
                .as_ref()
                .map_or_else(String::new, |a| a.base.name.clone()),
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
}

/// The interfaces.
pub struct Manager<K: Kernel> {
    kernel: K,
    report: Reporter,
    config: Network,
    host_name: String,
    ifaces: Vec<Iface>,
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

/// Whether every interface of `network` has a role of `roles` (an
/// appliance's), the first interface the first role: an interface that
/// names none has the first.
pub fn check_roles(network: &Network, roles: &[&str]) -> Result<(), String> {
    let first = roles.first().copied().unwrap_or_default();
    for (i, interface) in network.interfaces.iter().enumerate() {
        let role = if interface.role.is_empty() {
            first
        } else {
            interface.role.as_str()
        };
        if !roles.contains(&role) {
            return Err(format!(
                "interface {}: `{role}` is not a role of this appliance ({})",
                i + 1,
                roles.join(", ")
            ));
        }
        if i == 0 && role != first {
            return Err(format!("the first interface is the {first} interface"));
        }
        if i > 0 && role == first {
            return Err(format!("only the first interface is the {first} interface"));
        }
    }
    Ok(())
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
        let ifaces = config
            .interfaces
            .iter()
            .map(|c| Iface::new(c.clone(), now))
            .collect();
        Self {
            kernel,
            report,
            config,
            host_name,
            ifaces,
        }
    }

    /// Choose interface `i`'s device and make the interface on it.
    fn choose(&mut self, i: usize, links: &[LinkInfo]) {
        // A link another interface uses as its device is not taken by
        // one that would take any.
        let used: Vec<u32> = self
            .ifaces
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .filter_map(|(_, o)| o.active.as_ref().map(|a| a.base.index))
            .collect();
        let Self { kernel, ifaces, .. } = self;
        let me = &mut ifaces[i];
        let device = &me.config.device;
        let candidates: Vec<&LinkInfo> = links
            .iter()
            .filter(|l| match device {
                Device::Auto => l.physical && !used.contains(&l.index),
                Device::Name(n) => l.name == *n,
                Device::Mac(m) => l.physical && l.mac == *m,
            })
            .filter(|l| !l.taken)
            .collect();
        // Carrier shows only on a link that is up.
        for link in &candidates {
            if !link.up
                && let Err(e) = kernel.set_up(link)
            {
                eprintln!("{}: cannot be brought up: {e}", link.name);
            }
        }
        let pick = match device {
            Device::Auto => candidates.iter().find(|l| l.carrier),
            _ => candidates.first(),
        };
        let Some(base) = pick.map(|l| (*l).clone()) else {
            return;
        };
        let (iface, made_vlan) = match me.config.vlan {
            None => (base.clone(), false),
            Some(id) => {
                let name = vlan_name(&base.name, id);
                let existed = links.iter().any(|l| l.name == name);
                match kernel.vlan(&name, base.index, id) {
                    Ok(index) => {
                        let iface = LinkInfo {
                            index,
                            name,
                            physical: false,
                            up: false,
                            carrier: false,
                            ..base.clone()
                        };
                        if let Err(e) = kernel.set_up(&iface) {
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
        me.active = Some(Active {
            base,
            iface,
            made_vlan,
            carrier: false,
            dhcp: None,
            applied: None,
        });
    }

    /// Carrier came to interface `i`: say so and get an address.
    fn carrier_up(&mut self, i: usize, now: Instant) {
        let me = &mut self.ifaces[i];
        let Some(active) = &mut me.active else {
            return;
        };
        active.carrier = true;
        me.since = now;
        me.warned = false;
        let iface = active.iface.clone();
        (self.report.post)(&Event::InterfaceUp {
            interface: &iface.name,
        });
        let config = me.config.clone();
        if let (Mode::Static, Some(address)) = (config.mode, config.address) {
            let gateway = config.gateway.filter(|_| config.carries_route);
            let kernel = &mut self.kernel;
            let done = kernel
                .add_address(iface.index, address)
                .and_then(|()| gateway.map_or(Ok(()), |g| kernel.add_gateway(iface.index, g)));
            match done {
                Ok(()) => {
                    if let Some(active) = &mut self.ifaces[i].active {
                        active.applied = Some((address, gateway));
                    }
                    self.acquired(i, address, gateway, &[]);
                }
                Err(e) => eprintln!("{}: {address}: {e}", iface.name),
            }
        } else {
            let lease = me.lease.clone();
            match self
                .kernel
                .dhcp(&iface, &self.host_name, lease, config.carries_route)
            {
                Ok(client) => {
                    if let Some(active) = &mut self.ifaces[i].active {
                        active.dhcp = Some(client);
                    }
                }
                Err(e) => eprintln!("{}: DHCP: {e}", iface.name),
            }
        }
    }

    /// Carrier went from interface `i`: a lease is no longer trusted; a
    /// static address stays.
    fn carrier_down(&mut self, i: usize, now: Instant) {
        let me = &mut self.ifaces[i];
        let Some(active) = &mut me.active else {
            return;
        };
        active.carrier = false;
        if let Some(client) = active.dhcp.take() {
            client.stop(false);
        }
        me.address = None;
        me.lease_dns.clear();
        me.since = now;
        me.warned = false;
    }

    /// The name servers of every interface that carries them: each
    /// one's own, then its lease's; one another named before is left out.
    fn name_servers(&self) -> Vec<IpAddr> {
        let mut all: Vec<IpAddr> = Vec::new();
        for me in self.ifaces.iter().filter(|m| m.config.carries_dns) {
            let mut own = me.config.dns.clone();
            own.extend(me.lease_dns.iter().filter(|d| !me.config.dns.contains(d)));
            let before = all.len();
            for server in own {
                if !all[..before].contains(&server) {
                    all.push(server);
                }
            }
        }
        all
    }

    fn acquired(
        &mut self,
        i: usize,
        address: Cidr,
        gateway: Option<Ipv4Addr>,
        lease_dns: &[IpAddr],
    ) {
        self.ifaces[i].lease_dns = lease_dns.to_vec();
        let dns = self.name_servers();
        if self.ifaces[i].config.carries_dns {
            self.kernel.resolver(&dns);
        }
        let me = &mut self.ifaces[i];
        let changed = me.address != Some((address, gateway));
        me.address = Some((address, gateway));
        me.warned = false;
        if changed && let Some(active) = &me.active {
            (self.report.post)(&Event::AddressAcquired {
                interface: &active.iface.name,
                address: &address.to_string(),
                gateway: gateway.map(|g| g.to_string()).as_deref(),
            });
        }
    }

    /// The interface whose DHCP client runs on the link `index`.
    fn served(&self, index: u32) -> Option<usize> {
        self.ifaces.iter().position(|m| {
            m.active
                .as_ref()
                .is_some_and(|a| a.dhcp.is_some() && a.iface.index == index)
        })
    }

    /// The DHCP client on the link `index` has put a lease on it.
    pub fn bound(&mut self, index: u32, lease: &Lease) {
        let Some(i) = self.served(index) else {
            return;
        };
        let address = Cidr {
            addr: lease.addr,
            prefix: lease.prefix(),
        };
        let dns: Vec<IpAddr> = lease.dns.iter().copied().map(IpAddr::V4).collect();
        self.ifaces[i].lease = Some(lease.clone());
        let router = lease.router.filter(|_| self.ifaces[i].config.carries_route);
        self.acquired(i, address, router, &dns);
        self.note();
    }

    /// The DHCP client on the link `index` has taken its lease off. A
    /// client that was stopped says so too, after the interface it
    /// served is gone: that word is about no address in use now.
    pub fn unbound(&mut self, index: u32, now: Instant) {
        let Some(i) = self.served(index) else {
            return;
        };
        let me = &mut self.ifaces[i];
        me.lease_dns.clear();
        if me.address.take().is_some() {
            me.since = now;
            me.warned = false;
        }
        self.note();
    }

    /// Undo everything this machine did to the kernel for `me`.
    fn teardown(kernel: &mut K, me: &mut Iface, release: bool) {
        let Some(mut active) = me.active.take() else {
            return;
        };
        if let Some(client) = active.dhcp.take() {
            client.stop(release);
        }
        if let Some((address, gateway)) = active.applied {
            if let Some(g) = gateway {
                kernel.delete_gateway(active.iface.index, g);
            }
            kernel.delete_address(active.iface.index, address);
        }
        if active.made_vlan {
            kernel.delete_link(active.iface.index);
        }
        me.address = None;
        me.lease_dns.clear();
    }

    /// Look at the links and do what follows for each interface: choose
    /// its device, take carrier changes, and post what has been lacking
    /// too long. Called on every link event and at least every second.
    pub fn refresh(&mut self, now: Instant) {
        // Read again for each: the one before may have made a link or
        // brought one up.
        for i in 0..self.ifaces.len() {
            let links = self.kernel.links();
            self.refresh_one(i, &links, now);
        }
        self.note();
    }

    fn refresh_one(&mut self, i: usize, links: &[LinkInfo], now: Instant) {
        // A device that went away takes the interface with it.
        let me = &mut self.ifaces[i];
        if let Some(active) = &me.active
            && !links.iter().any(|l| l.index == active.base.index)
        {
            Self::teardown(&mut self.kernel, me, false);
            me.since = now;
            me.warned = false;
        }
        if self.ifaces[i].active.is_none() {
            self.choose(i, links);
        }
        let carrier = self.ifaces[i].active.as_ref().and_then(|a| {
            links
                .iter()
                .find(|l| l.index == a.base.index)
                .map(|l| (a.carrier, l.carrier))
        });
        match carrier {
            Some((false, true)) => self.carrier_up(i, now),
            Some((true, false)) => self.carrier_down(i, now),
            _ => {}
        }
        let me = &mut self.ifaces[i];
        if !me.warned && now.saturating_duration_since(me.since) >= ATTENTION_AFTER {
            let wanted = me.wanted();
            let event = match &me.active {
                None => Some(Event::NoInterface { wanted: &wanted }),
                Some(a) if !a.carrier => Some(Event::NoCarrier {
                    interface: &a.iface.name,
                }),
                Some(a) if me.address.is_none() => Some(Event::NoAddress {
                    interface: &a.iface.name,
                }),
                Some(_) => None,
            };
            if let Some(event) = event {
                (self.report.post)(&event);
                me.warned = true;
            }
        }
    }

    /// Take a new configuration: each interface that changed is undone
    /// and made again; one that did not is left running. Nothing
    /// happens when nothing changed. A new host name changes them all.
    pub fn reload(&mut self, config: Network, host_name: String, now: Instant) {
        if config == self.config && host_name == self.host_name {
            return;
        }
        let renamed = host_name != self.host_name;
        let mut old: Vec<Option<Iface>> = std::mem::take(&mut self.ifaces)
            .into_iter()
            .map(Some)
            .collect();
        let mut ifaces = Vec::new();
        for c in &config.interfaces {
            let kept = (!renamed)
                .then(|| {
                    old.iter_mut()
                        .find(|o| o.as_ref().is_some_and(|o| o.config == *c))
                        .and_then(Option::take)
                })
                .flatten();
            ifaces.push(kept);
        }
        // What is not kept is undone first, so its links are free. A
        // role the new configuration has no interface for is said to
        // have none, so nothing keeps showing its last line.
        for mut gone in old.into_iter().flatten() {
            Self::teardown(&mut self.kernel, &mut gone, true);
            let role = &gone.config.role;
            if !config.interfaces.iter().any(|c| c.role == *role) {
                (self.report.note)(role, REMOVED);
            }
        }
        self.ifaces = ifaces
            .into_iter()
            .zip(&config.interfaces)
            .map(|(kept, c)| kept.unwrap_or_else(|| Iface::new(c.clone(), now)))
            .collect();
        self.config = config;
        self.host_name = host_name;
        self.refresh(now);
    }

    /// The configuration in use.
    pub fn config(&self) -> &Network {
        &self.config
    }
}

/// The line of a role a change removed (QS/MC's item 28).
pub const REMOVED: &str = "none: not in the network configuration";

impl<K: Kernel> Manager<K> {
    /// The first interface's status line: the address, or why there is
    /// none.
    pub fn line(&self) -> String {
        self.ifaces.first().map(Iface::line).unwrap_or_default()
    }

    fn note(&mut self) {
        for me in &mut self.ifaces {
            let line = me.line();
            if line != me.noted {
                (self.report.note)(&me.config.role, &line);
                me.noted = line;
            }
        }
    }

    /// Whether an interface that carries the time has its address: the
    /// clock can be kept.
    pub fn online(&self) -> bool {
        self.ifaces
            .iter()
            .any(|m| m.config.carries_time && m.address.is_some())
    }

    /// What the service's status request answers: the first interface.
    pub fn status(&self) -> NetStatus {
        self.ifaces.first().map(Iface::status).unwrap_or_default()
    }

    /// Every interface's status, in the configuration's order.
    pub fn statuses(&self) -> Vec<NetStatus> {
        self.ifaces.iter().map(Iface::status).collect()
    }

    /// The kernel, for the tests.
    #[cfg(test)]
    fn kernel(&mut self) -> &mut K {
        &mut self.kernel
    }
}
