// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The numbers this crate needs that QS/400's `netlink::route::consts`
//! has not: `if_link.h`, `if_bridge.h`, `neighbour.h`, `rtnetlink.h`,
//! `pkt_cls.h`, `tc_act/tc_gact.h`.

#![allow(missing_docs)]

pub const AF_BRIDGE: u8 = 7;

pub const RTM_SETLINK: u16 = 19;
pub const RTM_NEWQDISC: u16 = 36;
pub const RTM_DELQDISC: u16 = 37;
pub const RTM_NEWTFILTER: u16 = 44;
pub const RTM_DELTFILTER: u16 = 45;
pub const RTM_GETTFILTER: u16 = 46;

pub const IFLA_PROTINFO: u16 = 12;
pub const IFLA_AF_SPEC: u16 = 26;
pub const IFLA_EXT_MASK: u16 = 29;
pub const RTEXT_FILTER_BRVLAN: u32 = 1 << 1;

// IFLA_BR_*: a bridge's own attributes.
pub const IFLA_BR_FORWARD_DELAY: u16 = 1;
pub const IFLA_BR_STP_STATE: u16 = 5;
pub const IFLA_BR_VLAN_FILTERING: u16 = 7;
pub const IFLA_BR_MCAST_SNOOPING: u16 = 23;
pub const IFLA_BR_VLAN_DEFAULT_PVID: u16 = 39;

// IFLA_BRPORT_*: a bridge port's attributes, in IFLA_PROTINFO.
pub const IFLA_BRPORT_GUARD: u16 = 5;
pub const IFLA_BRPORT_LEARNING: u16 = 8;
pub const IFLA_BRPORT_UNICAST_FLOOD: u16 = 9;
pub const IFLA_BRPORT_MCAST_FLOOD: u16 = 27;
pub const IFLA_BRPORT_BCAST_FLOOD: u16 = 30;
pub const IFLA_BRPORT_LOCKED: u16 = 39;

// IFLA_AF_SPEC of AF_BRIDGE.
pub const IFLA_BRIDGE_FLAGS: u16 = 0;
pub const IFLA_BRIDGE_VLAN_INFO: u16 = 2;
pub const BRIDGE_FLAGS_MASTER: u16 = 1;
pub const BRIDGE_FLAGS_SELF: u16 = 2;
pub const BRIDGE_VLAN_INFO_PVID: u16 = 1 << 1;
pub const BRIDGE_VLAN_INFO_UNTAGGED: u16 = 1 << 2;

// The forwarding table: ndmsg of AF_BRIDGE.
pub const NDA_LLADDR: u16 = 2;
pub const NDA_VLAN: u16 = 5;
pub const NDA_MASTER: u16 = 9;
pub const NUD_NOARP: u16 = 0x40;
pub const NUD_PERMANENT: u16 = 0x80;
pub const NTF_SELF: u8 = 0x02;
pub const NTF_MASTER: u8 = 0x04;
pub const NTF_EXT_LEARNED: u8 = 0x10;

// tc: tcmsg, the clsact qdisc, flower and gact.
pub const TCMSG_LEN: usize = 20;
pub const TCA_KIND: u16 = 1;
pub const TCA_OPTIONS: u16 = 2;
pub const TC_H_CLSACT: u32 = 0xFFFF_FFF1;
pub const TC_H_MIN_INGRESS: u32 = 0xFFF2;
pub const TC_H_INGRESS_PARENT: u32 = 0xFFFF_0000 | TC_H_MIN_INGRESS;
pub const TC_H_CLSACT_HANDLE: u32 = 0xFFFF_0000;
pub const TCA_FLOWER_ACT: u16 = 3;
pub const TCA_FLOWER_KEY_ETH_DST: u16 = 4;
pub const TCA_FLOWER_KEY_ETH_DST_MASK: u16 = 5;
pub const TCA_FLOWER_KEY_ETH_TYPE: u16 = 8;
pub const TCA_FLOWER_FLAGS: u16 = 22;
pub const TCA_FLOWER_KEY_VLAN_ID: u16 = 23;
pub const TCA_CLS_FLAGS_SKIP_HW: u32 = 1;
pub const TCA_ACT_KIND: u16 = 1;
pub const TCA_ACT_OPTIONS: u16 = 2;
pub const TCA_GACT_PARMS: u16 = 2;
pub const TC_ACT_OK: i32 = 0;
pub const TC_ACT_SHOT: i32 = 2;

pub const ETH_P_ALL: u16 = 0x0003;
pub const ETH_P_8021Q: u16 = 0x8100;
pub const ETH_P_8021AD: u16 = 0x88A8;
