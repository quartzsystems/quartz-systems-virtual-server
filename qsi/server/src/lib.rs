// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The QSI host server (`docs/QSISERVER.md`): the one listener a QS/VS
//! host has. It speaks QS/400's QSI (`qsi`, `qsisess`, `qsitrust`) to
//! a paired management console, and translates each request of the
//! host's families to the local request of the service that owns the
//! state. It owns no partition, pool or switch state of its own: only
//! the consoles it has paired and its event log.

pub mod trust;
