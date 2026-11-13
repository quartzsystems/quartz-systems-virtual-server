// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The reference code QS/VS posts for each event the base crates
//! report, and what the posting adds to the code's text.

use appliance::{Event, Storage};

use crate::RefCode;

/// The code of `event` and its detail.
pub fn code_of(event: &Event<'_>) -> (&'static RefCode, String) {
    match event {
        Event::ServiceStopped { service } => (&crate::SERVICE_STOPPED, (*service).to_string()),
        Event::SeedApplied => (&crate::SEED_APPLIED, String::new()),
        Event::SeedInvalid { reason } => (&crate::SEED_INVALID, (*reason).to_string()),
        Event::InterfaceUp { interface } => (&crate::INTERFACE_UP, (*interface).to_string()),
        Event::AddressAcquired {
            interface,
            address,
            gateway,
        } => {
            let via = gateway.map_or_else(String::new, |g| format!(", gateway {g}"));
            (
                &crate::ADDRESS_ACQUIRED,
                format!("{address} on {interface}{via}"),
            )
        }
        Event::NoInterface { wanted } => (&crate::NO_INTERFACE, format!("no {wanted}")),
        Event::NoCarrier { interface } => (&crate::NO_CARRIER, (*interface).to_string()),
        Event::NoAddress { interface } => (&crate::NO_ADDRESS, (*interface).to_string()),
        Event::TimeSynchronized { server } => (&crate::TIME_SYNCHRONIZED, (*server).to_string()),
        Event::Storage { what, detail } => (storage(*what), (*detail).to_string()),
    }
}

/// The code of each thing the storage service says.
fn storage(what: Storage) -> &'static RefCode {
    match what {
        Storage::SetAssembled => &crate::SET_ASSEMBLED,
        Storage::SetNotAssembled => &crate::SET_NOT_ASSEMBLED,
        Storage::PairSplit => &crate::UNITS_APART,
        Storage::UnitKept => &crate::UNIT_KEPT,
        Storage::UnitFailed => &crate::UNIT_FAILED,
        Storage::ResyncStarted => &crate::RESYNC_STARTED,
        Storage::ResyncFinished => &crate::RESYNC_FINISHED,
        Storage::RebuildStarted => &crate::REBUILD_STARTED,
        Storage::RebuildFinished => &crate::REBUILD_FINISHED,
        Storage::RebuildFailed => &crate::REBUILD_FAILED,
        Storage::ProtectionStarted => &crate::PROTECTION_STARTED,
        Storage::BootEntriesNotWritten => &crate::INSTALL_BOOT_NOT_WRITTEN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Family;

    #[test]
    fn events_have_their_codes() {
        let (code, detail) = code_of(&Event::AddressAcquired {
            interface: "eth0",
            address: "10.0.2.15/24",
            gateway: Some("10.0.2.2"),
        });
        assert_eq!(code.code, "C2002020");
        assert_eq!(detail, "10.0.2.15/24 on eth0, gateway 10.0.2.2");
        let (code, detail) = code_of(&Event::ServiceStopped { service: "netd" });
        assert_eq!((code.family, detail.as_str()), (Family::Attention, "netd"));
        let (code, _) = code_of(&Event::NoCarrier { interface: "eth0" });
        assert_eq!(code.family, Family::Attention);
        let (code, detail) = code_of(&Event::NoInterface {
            wanted: "interface eth7",
        });
        assert_eq!(
            code.text_with(&detail),
            "No management interface: no interface eth7"
        );
        let (code, detail) = code_of(&Event::Storage {
            what: Storage::UnitFailed,
            detail: "unit 2: missing",
        });
        assert_eq!(
            (code.code, detail.as_str()),
            ("A2003020", "unit 2: missing")
        );
    }
}
