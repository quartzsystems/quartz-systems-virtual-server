// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The ingress filter of a partition's port: what the bridge cannot
//! refuse by itself. With spanning tree off, the bridge forwards a BPDU
//! rather than take it; and a frame tagged with the port's own VLAN is
//! taken into that VLAN like an untagged one. So, on the tap's ingress
//! (frames from the guest), before the bridge sees them:
//!
//! 1. a frame to a reserved link-local group (`01:80:C2:00:00:0X`,
//!    BPDUs among them) is dropped;
//! 2. a frame tagged with one of the adapter's tagged VLANs, or with a
//!    priority tag (VLAN 0), passes;
//! 3. any other 802.1Q or 802.1ad tagged frame is dropped.
//!
//! A clsact qdisc and flower classifiers with gact actions: tc, not
//! netfilter.

#[allow(clippy::wildcard_imports)] // the numbers module
use crate::consts::*;
use netlink::codec::{Builder, NLM_F_CREATE, NLM_F_EXCL, NLM_F_REQUEST};

/// The reserved link-local groups and their mask.
const LINK_LOCAL: [u8; 6] = [0x01, 0x80, 0xC2, 0x00, 0x00, 0x00];
const LINK_LOCAL_MASK: [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF0];

fn tcmsg(ifindex: u32, handle: u32, parent: u32, info: u32) -> [u8; TCMSG_LEN] {
    let mut b = [0u8; TCMSG_LEN];
    b[4..8].copy_from_slice(&ifindex.to_ne_bytes());
    b[8..12].copy_from_slice(&handle.to_ne_bytes());
    b[12..16].copy_from_slice(&parent.to_ne_bytes());
    b[16..20].copy_from_slice(&info.to_ne_bytes());
    b
}

/// The clsact qdisc on `ifindex`, where ingress filters hang.
pub fn clsact_request(ifindex: u32) -> Builder {
    let mut b = Builder::new(RTM_NEWQDISC, NLM_F_REQUEST | NLM_F_CREATE | NLM_F_EXCL);
    b.bytes(&tcmsg(ifindex, TC_H_CLSACT_HANDLE, TC_H_CLSACT, 0))
        .str(TCA_KIND, "clsact");
    b
}

/// What a frame that matches is done with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// It goes on to the bridge.
    Pass,
    /// It is dropped.
    Drop,
}

/// One flower rule of an ingress filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Its priority: lower first.
    pub priority: u16,
    /// The Ethernet type it matches (`ETH_P_*`).
    pub protocol: u16,
    /// A destination and its mask.
    pub dst: Option<([u8; 6], [u8; 6])>,
    /// An outer VLAN ID, for `protocol` 802.1Q.
    pub vlan: Option<u16>,
    /// What is done with it.
    pub verdict: Verdict,
}

/// The rules of a partition's port that passes tagged frames of
/// `tagged` only.
pub fn partition_rules(tagged: &[u16]) -> Vec<Rule> {
    let mut rules = vec![Rule {
        priority: 1,
        protocol: ETH_P_ALL,
        dst: Some((LINK_LOCAL, LINK_LOCAL_MASK)),
        vlan: None,
        verdict: Verdict::Drop,
    }];
    let mut pass: Vec<u16> = std::iter::once(0).chain(tagged.iter().copied()).collect();
    pass.sort_unstable();
    pass.dedup();
    for (i, vid) in pass.into_iter().enumerate() {
        rules.push(Rule {
            priority: 10 + u16::try_from(i).unwrap_or(u16::MAX - 20),
            protocol: ETH_P_8021Q,
            dst: None,
            vlan: Some(vid),
            verdict: Verdict::Pass,
        });
    }
    for (priority, protocol) in [(u16::MAX - 1, ETH_P_8021Q), (u16::MAX, ETH_P_8021AD)] {
        rules.push(Rule {
            priority,
            protocol,
            dst: None,
            vlan: None,
            verdict: Verdict::Drop,
        });
    }
    rules
}

/// `tc_gact`: index, capab, action, refcnt, bindcnt.
fn gact(action: i32) -> [u8; 20] {
    let mut b = [0u8; 20];
    b[8..12].copy_from_slice(&action.to_ne_bytes());
    b
}

/// Add `rule` to the ingress of `ifindex`.
pub fn rule_request(ifindex: u32, rule: &Rule) -> Builder {
    // The protocol is in network order in the low half of `info`.
    let info = (u32::from(rule.priority) << 16) | u32::from(rule.protocol.to_be());
    let mut b = Builder::new(RTM_NEWTFILTER, NLM_F_REQUEST | NLM_F_CREATE | NLM_F_EXCL);
    b.bytes(&tcmsg(ifindex, 0, TC_H_INGRESS_PARENT, info))
        .str(TCA_KIND, "flower");
    b.nest(TCA_OPTIONS, |b| {
        b.u32(TCA_FLOWER_FLAGS, TCA_CLS_FLAGS_SKIP_HW);
        // Flower takes VLAN keys only of a rule whose type is a VLAN's.
        if rule.protocol != ETH_P_ALL {
            b.be16(TCA_FLOWER_KEY_ETH_TYPE, rule.protocol);
        }
        if let Some((dst, mask)) = rule.dst {
            b.attr(TCA_FLOWER_KEY_ETH_DST, &dst);
            b.attr(TCA_FLOWER_KEY_ETH_DST_MASK, &mask);
        }
        if let Some(vid) = rule.vlan {
            b.u16(TCA_FLOWER_KEY_VLAN_ID, vid);
        }
        b.nest(TCA_FLOWER_ACT, |b| {
            b.nest(1, |b| {
                b.str(TCA_ACT_KIND, "gact");
                b.nest(TCA_ACT_OPTIONS, |b| {
                    let action = match rule.verdict {
                        Verdict::Pass => TC_ACT_OK,
                        Verdict::Drop => TC_ACT_SHOT,
                    };
                    b.attr(TCA_GACT_PARMS, &gact(action));
                });
            });
        });
    });
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use netlink::codec::{Attrs, NLMSG_HDRLEN};

    #[test]
    fn an_access_port_drops_every_tag_but_a_priority_tag_and_every_bpdu() {
        let rules = partition_rules(&[]);
        assert_eq!(rules.len(), 4);
        assert_eq!(rules[0].verdict, Verdict::Drop);
        assert_eq!(rules[0].dst.unwrap().0[0..3], [0x01, 0x80, 0xC2]);
        assert_eq!((rules[1].vlan, rules[1].verdict), (Some(0), Verdict::Pass));
        assert_eq!(
            (rules[2].protocol, rules[2].verdict),
            (ETH_P_8021Q, Verdict::Drop)
        );
        assert_eq!(rules[3].protocol, ETH_P_8021AD);
        let tagged = partition_rules(&[30, 20, 20]);
        let passed: Vec<u16> = tagged.iter().filter_map(|r| r.vlan).collect();
        assert_eq!(passed, [0, 20, 30]);
        // Priorities are distinct and in the order the rules are tried.
        let mut p: Vec<u16> = tagged.iter().map(|r| r.priority).collect();
        let sorted = p.clone();
        p.dedup();
        assert_eq!(p, sorted);
    }

    #[test]
    fn a_rule_is_a_flower_filter_on_the_ingress_with_its_protocol_in_network_order() {
        let rule = partition_rules(&[20])[2];
        let msg = rule_request(5, &rule).finish(1);
        let tc = &msg[NLMSG_HDRLEN..NLMSG_HDRLEN + TCMSG_LEN];
        assert_eq!(u32::from_ne_bytes(tc[4..8].try_into().unwrap()), 5);
        assert_eq!(
            u32::from_ne_bytes(tc[12..16].try_into().unwrap()),
            TC_H_INGRESS_PARENT
        );
        let info = u32::from_ne_bytes(tc[16..20].try_into().unwrap());
        assert_eq!(info >> 16, u32::from(rule.priority));
        assert_eq!(
            u16::from_be(u16::try_from(info & 0xFFFF).unwrap()),
            ETH_P_8021Q
        );
        let attrs = Attrs::new(&msg[NLMSG_HDRLEN + TCMSG_LEN..]);
        assert_eq!(attrs.get(TCA_KIND).unwrap().str(), "flower");
        let opts = attrs.get(TCA_OPTIONS).unwrap().attrs();
        assert_eq!(opts.get(TCA_FLOWER_KEY_VLAN_ID).unwrap().u16(), Some(20));
        let act = opts
            .get(TCA_FLOWER_ACT)
            .unwrap()
            .attrs()
            .get(1)
            .unwrap()
            .attrs();
        assert_eq!(act.get(TCA_ACT_KIND).unwrap().str(), "gact");
        let parms = act
            .get(TCA_ACT_OPTIONS)
            .unwrap()
            .attrs()
            .get(TCA_GACT_PARMS)
            .unwrap();
        assert_eq!(
            i32::from_ne_bytes(parms.data[8..12].try_into().unwrap()),
            TC_ACT_OK
        );
        let q = clsact_request(5).finish(2);
        assert_eq!(u16::from_ne_bytes([q[4], q[5]]), RTM_NEWQDISC);
    }
}
