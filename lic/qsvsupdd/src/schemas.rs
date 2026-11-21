// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The state files this release reads and the newest schema version of
//! each (`base/state`'s documents): what an update image's manifest
//! says, so a host can refuse a release that cannot read its state.

use state::Document;

/// The QSI server's consoles, which `qsi/server` keeps (a test there
/// holds this to its document).
pub const CONSOLES: (&str, u32) = ("qsi-consoles.toml", 1);

/// Each state file and the newest version this release reads.
#[must_use]
pub fn this_release() -> Vec<(String, u32)> {
    let mut all: Vec<(String, u32)> = [
        (state::Host::FILE, state::Host::VERSION),
        (state::Identity::FILE, state::Identity::VERSION),
        (state::Inventory::FILE, state::Inventory::VERSION),
        (state::Network::FILE, state::Network::VERSION),
        (vswitch::Switches::FILE, vswitch::Switches::VERSION),
        (model::Definition::FILE, model::Definition::VERSION),
        (model::Activations::FILE, model::Activations::VERSION),
        (model::Profile::FILE, model::Profile::VERSION),
        (resource::Settings::FILE, resource::Settings::VERSION),
        CONSOLES,
    ]
    .into_iter()
    .map(|(f, v)| (f.to_string(), v))
    .collect();
    all.sort();
    all
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_file_is_named_once() {
        let all = super::this_release();
        for (i, (f, v)) in all.iter().enumerate() {
            assert!(
                std::path::Path::new(f)
                    .extension()
                    .is_some_and(|x| x == "toml")
                    && *v >= 1,
                "{f}"
            );
            assert!(all[..i].iter().all(|(o, _)| o != f), "{f} twice");
        }
    }
}
