// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The state step of the IPL: the seed, the state files with their
//! defaults, the inventory, the host name.

use appliance::time::Zone;
use hostinv::Roots;
use rustix::system::sethostname;
use state::{Host, Identity, Inventory, Network, Store};

/// What the IPL learned from the state files.
pub struct Loaded {
    /// The host name in use.
    pub host_name: String,
    /// The display zone.
    pub zone: Zone,
    /// The system serial number, empty when there is none.
    pub serial: String,
    /// The state files that were refused: the file and why. Their
    /// defaults are in use.
    pub refused: Vec<String>,
}

fn load_or_default<D: state::Document + Default>(store: &Store, refused: &mut Vec<String>) -> D {
    match store.load::<D>() {
        Ok(found) => found.unwrap_or_default(),
        Err(e) => {
            refused.push(e.to_string());
            D::default()
        }
    }
}

fn save<D: state::Document>(store: &Store, doc: &D, refused: &mut Vec<String>) {
    if let Err(e) = store.save(doc) {
        refused.push(e.to_string());
    }
}

/// Read the state files, write the inventory, set the host name.
pub fn load(store: &Store, host_prefix: &str) -> Loaded {
    let mut refused = Vec::new();
    let host: Host = load_or_default(store, &mut refused);
    let mut identity: Identity = load_or_default(store, &mut refused);
    // The network service reads its own file; a bad one is said here.
    let _: Network = load_or_default(store, &mut refused);

    let inventory: Inventory = hostinv::read(&Roots::default(), identity.serial.as_deref());
    // A serial derived from the first NIC is derived once and kept, in
    // a file of its own: `host.toml` is the operator's.
    if identity.serial.is_none() && inventory.system.serial_source == "derived" {
        identity.serial = Some(inventory.system.serial.clone());
        save(store, &identity, &mut refused);
    }
    save(store, &inventory, &mut refused);

    let first = appliance::links::ethernet().first().map(|l| l.mac);
    let host_name = host.name_or_default(host_prefix, first);
    let _ = sethostname(host_name.as_bytes());
    Loaded {
        host_name,
        zone: host.zone(),
        serial: inventory.system.serial,
        refused,
    }
}
