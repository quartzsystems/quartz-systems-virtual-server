// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! QS/VS as the base installer sees it: the machine's disks and links
//! from the inventory, its words, and the codes of the steps that write
//! a load source, which `base/install::write` carries out.

use install::write::{self, STEP_NAMES, Writer};
use install::{Answers, DiskChoice, Step, Target};
use panels::forms::LinkChoice;
use release::LOAD_SOURCE;
use state::Host;

use crate::media::Payload;

/// The steps, each with the code posted when it is done.
const STEPS: &[Step] = &[
    Step {
        name: STEP_NAMES[0],
        code: refcode::INSTALL_PARTITIONED.code,
    },
    Step {
        name: STEP_NAMES[1],
        code: refcode::INSTALL_PROTECTED.code,
    },
    Step {
        name: STEP_NAMES[2],
        code: refcode::INSTALL_FORMATTED.code,
    },
    Step {
        name: STEP_NAMES[3],
        code: refcode::INSTALL_SLOT_A.code,
    },
    Step {
        name: STEP_NAMES[4],
        code: refcode::INSTALL_SLOT_B.code,
    },
    Step {
        name: STEP_NAMES[5],
        code: refcode::INSTALL_RECORD.code,
    },
    Step {
        name: STEP_NAMES[6],
        code: refcode::INSTALL_BOOT.code,
    },
    Step {
        name: STEP_NAMES[7],
        code: refcode::INSTALL_STATE.code,
    },
];

/// What is QS/VS's in writing its load source.
static PRODUCT: write::Product = write::Product {
    names: &LOAD_SOURCE,
    name: release::NAME,
    started: (refcode::INSTALL_STARTED.code, refcode::INSTALL_STARTED.text),
    boot_not_written: (
        refcode::INSTALL_BOOT_NOT_WRITTEN.code,
        refcode::INSTALL_BOOT_NOT_WRITTEN.text,
    ),
};

/// QS/VS, being installed.
pub struct QsVs {
    /// Disks that are not to be offered: the media and the seed volume.
    skip: Vec<String>,
    writer: Writer,
}

impl QsVs {
    /// The installer over verified media. `skip` names the disks that
    /// hold the media and the answers.
    pub fn new(payload: Payload, skip: Vec<String>, log: Box<dyn FnMut(&str)>) -> Self {
        Self {
            skip,
            writer: Writer::new(&PRODUCT, payload, log),
        }
    }

    /// Bring every Ethernet link up, so that carrier can be shown.
    pub fn links_up() {
        let Ok(mut rtnl) = netlink::route::Rtnl::open() else {
            return;
        };
        for link in appliance::links::ethernet() {
            let _ = rtnl.set_up(link.index, true);
        }
    }
}

impl Target for QsVs {
    fn product(&self) -> &'static str {
        release::NAME
    }

    fn secure_boot(&self) -> Option<bool> {
        loadsource::boot::secure_boot(std::path::Path::new(loadsource::boot::EFIVARS))
    }

    fn banner(&self) -> String {
        format!("{} ({})", release::banner(), release::PRODUCT)
    }

    fn what(&self) -> &'static [&'static str] {
        &[
            "This installs the hypervisor on this machine: it erases the disks",
            "you choose, makes them the load source the host starts from, and",
            "sets the host name, the management network, the time and the first",
            "service tools user. Partitions are then made and run from the",
            "management console, QS/MC.",
        ]
    }

    fn disks(&mut self) -> Vec<DiskChoice> {
        hostinv::disks(&hostinv::Roots::default())
            .into_iter()
            .filter(|d| !self.skip.contains(&d.name) && !d.name.starts_with("sr"))
            .map(DiskChoice::from)
            .collect()
    }

    fn min_disk_bytes(&self) -> u64 {
        loadsource::layout::min_disk_bytes(&LOAD_SOURCE)
    }

    fn links(&mut self) -> Vec<LinkChoice> {
        hostinv::interfaces(&hostinv::Roots::default())
            .into_iter()
            .map(LinkChoice::from)
            .collect()
    }

    fn default_host(&self) -> String {
        let first = appliance::links::ethernet().first().map(|l| l.mac);
        Host::default_name(release::HOST_PREFIX, first)
    }

    fn now(&self) -> i64 {
        appliance::time::now_secs()
    }

    fn steps(&self) -> &'static [Step] {
        STEPS
    }

    fn step(&mut self, index: usize, answers: &Answers) -> Result<(), String> {
        self.writer.step(index, answers)
    }

    fn failed(&self) -> &'static str {
        refcode::INSTALL_FAILED.code
    }

    fn log(&mut self, line: &str) {
        self.writer.log(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_are_the_eight_of_the_progress_display_each_with_its_code() {
        assert_eq!(STEPS.len(), 8);
        for (i, step) in STEPS.iter().enumerate() {
            assert!(step.code.starts_with("C2004"), "{}", step.code);
            assert!(refcode::find(step.code).is_some());
            assert!(STEPS[..i].iter().all(|s| s.code < step.code));
        }
        assert_eq!(STEPS[0].name, "Partitioning the disks");
        assert_eq!(STEPS[7].name, "Writing state");
    }
}
