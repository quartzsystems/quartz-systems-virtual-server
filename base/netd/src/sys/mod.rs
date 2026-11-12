// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the service asks of Linux: the links, addresses and routes
//! through QS/400's `netlink`, the DHCP client through its `dhcp`, and
//! the clocks.

pub mod clock;
pub mod kernel;
