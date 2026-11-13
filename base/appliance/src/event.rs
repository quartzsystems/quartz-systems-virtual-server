// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! What the base crates report. The product decides which reference
//! code each one is.

/// What the storage service has to say of the load source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// The mirrored set was assembled and is served.
    SetAssembled,
    /// The mirrored set could not be assembled.
    SetNotAssembled,
    /// A unit is missing or failed.
    UnitFailed,
    /// A unit is being brought back in step.
    ResyncStarted,
    /// The unit is in step again.
    ResyncFinished,
    /// A replacement unit is being rebuilt.
    RebuildStarted,
    /// The replacement unit is in step.
    RebuildFinished,
    /// A resynchronization or a rebuild ended before the unit was in step.
    RebuildFailed,
    /// Mirrored protection was started; it takes effect at the next IPL.
    ProtectionStarted,
    /// The firmware's boot entries could not be written for a new unit.
    BootEntriesNotWritten,
}

/// Something a base crate has to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// A service failed too often and is no longer restarted.
    ServiceStopped {
        /// The service.
        service: &'a str,
    },
    /// A seed volume was read and the state files written from it.
    SeedApplied,
    /// A seed volume was found and refused.
    SeedInvalid {
        /// Why.
        reason: &'a str,
    },
    /// The management interface is up and has carrier.
    InterfaceUp {
        /// The interface.
        interface: &'a str,
    },
    /// The management interface has its address.
    AddressAcquired {
        /// The interface.
        interface: &'a str,
        /// `address/prefix`.
        address: &'a str,
        /// The default gateway, when there is one.
        gateway: Option<&'a str>,
    },
    /// No interface to manage the host through.
    NoInterface {
        /// What was looked for.
        wanted: &'a str,
    },
    /// The management interface has had no carrier for too long.
    NoCarrier {
        /// The interface.
        interface: &'a str,
    },
    /// The management interface has had no address for too long.
    NoAddress {
        /// The interface.
        interface: &'a str,
    },
    /// The clock was brought in step with a time server.
    TimeSynchronized {
        /// The server.
        server: &'a str,
    },
    /// The storage service says something of the load source.
    Storage {
        /// What.
        what: Storage,
        /// The units, sizes and reasons of this occurrence.
        detail: &'a str,
    },
}
