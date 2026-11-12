// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The machine's [`Kernel`] on a real one.

use std::fmt::Write as _;
use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use dhcp::run::{Agent, Sink};
use dhcp::{Config, Lease};
use netlink::Family;
use netlink::route::{Address, Route, Rtnl};
use state::Cidr;

use crate::manage::{DhcpClient, Kernel, LinkInfo};
use crate::serve::Msg;

/// `ARPHRD_ETHER`.
const ETHERNET: u16 = 1;
/// `RTPROT_STATIC`: a route an administrator configured.
const RTPROT_STATIC: u8 = 4;
/// `RTPROT_DHCP`: a route a DHCP client installed.
const RTPROT_DHCP: u8 = 16;

fn text(e: &netlink::Error) -> String {
    e.to_string()
}

/// A result where "already there" is success.
fn or_exists(r: netlink::Result<()>) -> Result<(), String> {
    match r {
        Err(e) if !e.is_exists() => Err(text(&e)),
        _ => Ok(()),
    }
}

fn address(index: u32, cidr: Cidr) -> Address {
    let mut a = Address::new(index, IpAddr::V4(cidr.addr), cidr.prefix);
    if cidr.prefix < 31 {
        a.broadcast = Some(Address::default_broadcast(cidr.addr, cidr.prefix));
    }
    a
}

fn default_route(index: u32, gateway: Ipv4Addr, protocol: u8) -> Route {
    let mut r = Route::new(Family::V4, None, 0);
    r.gateway = Some(IpAddr::V4(gateway));
    r.oif = Some(index);
    r.protocol = protocol;
    r
}

/// The kernel, and where the resolver's file is.
pub struct Linux {
    rtnl: Rtnl,
    tx: Sender<Msg>,
    resolv: PathBuf,
}

impl Linux {
    /// Open the route socket. Leases are reported on `tx`; the name
    /// servers are written to `resolv`.
    pub fn open(tx: Sender<Msg>, resolv: impl Into<PathBuf>) -> netlink::Result<Self> {
        Ok(Self {
            rtnl: Rtnl::open()?,
            tx,
            resolv: resolv.into(),
        })
    }
}

impl Kernel for Linux {
    fn links(&mut self) -> Vec<LinkInfo> {
        let links = self.rtnl.links().unwrap_or_default();
        links
            .into_iter()
            .filter(|l| !l.is_loopback())
            .filter_map(|l| {
                let device = Path::new("/sys/class/net").join(&l.name).join("device");
                Some(LinkInfo {
                    index: l.index,
                    mac: l.mac.as_slice().try_into().ok()?,
                    physical: l.link_type == ETHERNET && device.exists(),
                    up: l.is_up(),
                    carrier: l.has_carrier(),
                    name: l.name,
                })
            })
            .collect()
    }

    fn set_up(&mut self, link: &LinkInfo) -> Result<(), String> {
        // No address but the link-local one from IPv6.
        let accept_ra = format!("/proc/sys/net/ipv6/conf/{}/accept_ra", link.name);
        let _ = fs::write(accept_ra, "0");
        self.rtnl.set_up(link.index, true).map_err(|e| text(&e))
    }

    fn vlan(&mut self, name: &str, parent: u32, id: u16) -> Result<u32, String> {
        if let Ok(link) = self.rtnl.link_by_name(name) {
            return Ok(link.index);
        }
        self.rtnl
            .create_vlan(name, parent, id)
            .map(|l| l.index)
            .map_err(|e| text(&e))
    }

    fn delete_link(&mut self, index: u32) {
        let _ = self.rtnl.delete_link(index);
    }

    fn add_address(&mut self, index: u32, cidr: Cidr) -> Result<(), String> {
        or_exists(self.rtnl.add_address(&address(index, cidr)))
    }

    fn delete_address(&mut self, index: u32, cidr: Cidr) {
        let _ = self.rtnl.delete_address(&address(index, cidr));
    }

    fn add_gateway(&mut self, index: u32, gateway: Ipv4Addr) -> Result<(), String> {
        or_exists(
            self.rtnl
                .add_route(&default_route(index, gateway, RTPROT_STATIC)),
        )
    }

    fn delete_gateway(&mut self, index: u32, gateway: Ipv4Addr) {
        let _ = self
            .rtnl
            .delete_route(&default_route(index, gateway, RTPROT_STATIC));
    }

    fn dhcp(
        &mut self,
        link: &LinkInfo,
        host_name: &str,
        previous: Option<Lease>,
    ) -> Result<Box<dyn DhcpClient>, String> {
        let mut config = Config::new(link.mac);
        config.host_name = Some(host_name.to_string());
        config.previous = previous;
        let sink = LeaseSink {
            index: link.index,
            tx: self.tx.clone(),
        };
        let agent = Agent::spawn(&link.name, config, Box::new(sink)).map_err(|e| e.to_string())?;
        Ok(Box::new(Client(agent)))
    }

    fn resolver(&mut self, servers: &[IpAddr]) {
        let mut text = String::new();
        for server in servers {
            let _ = writeln!(text, "nameserver {server}");
        }
        if let Err(e) = state::atomic_write(&self.resolv, text.as_bytes()) {
            eprintln!("{}: {e}", self.resolv.display());
        }
    }
}

/// QS/400's DHCP agent: a thread keeping a lease.
struct Client(Agent);

impl DhcpClient for Client {
    fn stop(self: Box<Self>, release: bool) {
        self.0.stop(release);
    }
}

/// Where the agent's lease goes: onto the link, then to the machine.
struct LeaseSink {
    index: u32,
    tx: Sender<Msg>,
}

impl LeaseSink {
    fn cidr(lease: &Lease) -> Cidr {
        Cidr {
            addr: lease.addr,
            prefix: lease.prefix(),
        }
    }
}

impl Sink for LeaseSink {
    fn bind(&mut self, lease: &Lease) -> Result<(), String> {
        let mut rtnl = Rtnl::open().map_err(|e| text(&e))?;
        or_exists(rtnl.add_address(&address(self.index, Self::cidr(lease))))?;
        if let Some(router) = lease.router {
            or_exists(rtnl.add_route(&default_route(self.index, router, RTPROT_DHCP)))?;
        }
        let _ = self.tx.send(Msg::Bound(lease.clone()));
        Ok(())
    }

    fn unbind(&mut self, lease: &Lease) {
        if let Ok(mut rtnl) = Rtnl::open() {
            if let Some(router) = lease.router {
                let _ = rtnl.delete_route(&default_route(self.index, router, RTPROT_DHCP));
            }
            let _ = rtnl.delete_address(&address(self.index, Self::cidr(lease)));
        }
        let _ = self.tx.send(Msg::Unbound);
    }
}
