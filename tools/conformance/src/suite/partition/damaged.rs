// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Load sources a partition cannot IPL from. Each is a small disk the
//! harness writes with one thing wrong; each must give the `A2` code
//! that names what is missing, `qsvmm` must end in order (exit 30, not
//! a signal and not a panic's code), the partition must be Not
//! activated with the message that names the code, and the host must
//! not notice.

use std::path::PathBuf;
use std::time::Instant;

use ipl::{ESP_TYPE, LIC_TYPE, SIDE_A_PATH, SIDE_B_PATH};
use refcode::RefCode;

use super::{
    ID, NAME, ended, expect_code, fresh_host, host, host_unaffected, launch, manager, profile,
    start,
};
use crate::suite::base::Run;

/// Bytes of a sector.
const SECTOR: usize = 512;
/// The memory of a partition that will not get as far as using it.
const MEMORY_MIB: u64 = 256;

/// One damaged load source.
struct Case {
    /// What is wrong with it.
    what: &'static str,
    /// The disk.
    disk: Vec<u8>,
    /// The side to ask for.
    side: &'static str,
    /// The code it must give.
    code: &'static RefCode,
}

/// Where a partition of type `ty` starts on `disk`, in bytes.
fn offset_of(disk: &[u8], ty: uuid::Uuid) -> Result<usize, String> {
    let table = ipl::gpt::read(&mut std::io::Cursor::new(disk), disk.len() as u64)
        .map_err(|e| e.to_string())?;
    let part = table
        .iter()
        .find(|p| p.type_guid == ty)
        .ok_or("the disk has no such partition")?;
    usize::try_from(part.offset()).map_err(|e| e.to_string())
}

fn cases() -> Result<Vec<Case>, String> {
    let kernel = ipl::load::build(64 * 1024);
    let whole = ipl::build_disk(&[(SIDE_A_PATH, &kernel), (SIDE_B_PATH, &kernel)], true);
    let esp = offset_of(&whole, ESP_TYPE)?;
    let lic = offset_of(&whole, LIC_TYPE)?;

    // The ESP's partition entry made another type's: no ESP.
    let mut no_esp = ipl::gpt::build(
        (whole.len() / SECTOR) as u64,
        &[(
            LIC_TYPE,
            (lic / SECTOR) as u64,
            (whole.len() / SECTOR) as u64 - 65,
        )],
    );
    no_esp.resize(whole.len(), 0);

    let only_a = ipl::build_disk(&[(SIDE_A_PATH, &kernel)], true);

    let mut bad_record = whole.clone();
    for byte in &mut bad_record[lic + 30..lic + 60] {
        *byte ^= 0xFF;
    }

    // The UKI's cluster chain ends after its first cluster: the file
    // is shorter on the disk than its directory entry says.
    let mut truncated = only_a.clone();
    let first_cluster = 4;
    let fat = esp + SECTOR + first_cluster * 2;
    truncated[fat..fat + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());

    let not_kernel = ipl::build_disk(&[(SIDE_A_PATH, &vec![0x4D; 64 * 1024])], true);

    let mut bad_volume = whole.clone();
    bad_volume[esp..esp + SECTOR].fill(0xEE);

    Ok(vec![
        Case {
            what: "no partition table",
            disk: vec![0u8; whole.len()],
            side: "a",
            code: &refcode::PARTITION_NO_TABLE,
        },
        Case {
            what: "no EFI system partition",
            disk: no_esp,
            side: "a",
            code: &refcode::PARTITION_NO_ESP,
        },
        Case {
            what: "an ESP that holds no volume",
            disk: bad_volume,
            side: "a",
            code: &refcode::PARTITION_BAD_ESP,
        },
        Case {
            what: "no UKI for side B",
            disk: only_a,
            side: "b",
            code: &refcode::PARTITION_NO_IMAGE,
        },
        Case {
            what: "a corrupt LIC record",
            disk: bad_record,
            side: "recorded",
            code: &refcode::PARTITION_BAD_RECORD,
        },
        Case {
            what: "a truncated UKI",
            disk: truncated,
            side: "a",
            code: &refcode::PARTITION_BAD_IMAGE,
        },
        Case {
            what: "a UKI that is not a bzImage",
            disk: not_kernel,
            side: "a",
            code: &refcode::PARTITION_NOT_KERNEL,
        },
    ])
}

/// The device a case's disk is on the host: the cases are the disks
/// after the host's own, in order.
fn device(index: usize) -> String {
    let letter = char::from(b'b' + u8::try_from(index).unwrap_or(0));
    format!("/dev/vd{letter}")
}

/// Suite 9: damaged load sources each give their `A2` code and no crash.
pub fn damaged(run: &Run<'_>) -> Result<String, String> {
    let cases = cases()?;
    fresh_host(run)?;
    let mut disks: Vec<PathBuf> = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let path = run.file(&format!("damaged-{i}.img"));
        std::fs::write(&path, &case.disk).map_err(|e| e.to_string())?;
        disks.push(path);
    }
    let spec = |i: usize| {
        let side = format!("side {}", cases[i].side);
        profile(1, MEMORY_MIB, &side, &device(i))
    };
    let mut guest = host(run, &launch(run, &disks)?)?;
    manager::partition(&mut guest, NAME)?;
    let mut seen = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        // The activation itself is taken: it is the VMM that finds
        // the load source wanting.
        start(&mut guest, Some(&spec(i)))?;
        let deadline = Instant::now() + run.timeout;
        let at = |e: String| format!("{}: {e}", case.what);
        let detail = expect_code(&mut guest, case.code, deadline).map_err(at)?;
        expect_code(&mut guest, &refcode::PARTITION_NOT_IPLED, deadline).map_err(at)?;
        let how = ended(&mut guest, deadline).map_err(at)?;
        if how != "exit 30" {
            return Err(at(format!("qsvmm gave `{how}`, not `exit 30`")));
        }
        let status = manager::status(&mut guest, ID).map_err(at)?;
        let message = status.message.map(|m| m.to_string()).unwrap_or_default();
        if status.state != partd::State::NotActivated
            || status.current.is_some()
            || !message.starts_with(qvsmsg::DID_NOT_IPL.id)
            || !message.contains(case.code.code)
        {
            return Err(at(format!(
                "the partition is {} with the message `{message}`",
                status.state.word()
            )));
        }
        host_unaffected(&mut guest).map_err(at)?;
        seen.push(format!("{}: {}{detail}", case.what, case.code.code));
    }
    if guest.said(refcode::PARTITION_STARTED.code) {
        return Err("a partition started from a damaged load source".into());
    }
    Ok(format!(
        "{} damaged load sources, each refused with its code and exit 30, the partition Not \
         activated with QVS2108 naming the code, the host's base ready after each: {}",
        cases.len(),
        seen.join("; ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipl::lic::Settings;
    use ipl::{Fault, Side};

    /// The fault the loader itself finds in a case's disk.
    fn fault(case: &Case) -> Fault {
        let side = match case.side {
            "a" => Side::A,
            "b" => Side::B,
            _ => Side::Recorded,
        };
        let mut disk = std::io::Cursor::new(case.disk.clone());
        let bytes = case.disk.len() as u64;
        match ipl::read(&mut disk, bytes, side, Settings::default(), 0, &mut |_| {}) {
            Err(fault) => fault,
            Ok((_, image)) => Fault::NotKernel(ipl::load::check(&image).unwrap_err()),
        }
    }

    #[test]
    fn each_case_is_damaged_the_way_it_says() {
        let cases = cases().unwrap();
        let faults: Vec<Fault> = cases.iter().map(fault).collect();
        assert!(matches!(faults[0], Fault::NoTable(_)), "{:?}", faults[0]);
        assert!(matches!(faults[1], Fault::NoEsp), "{:?}", faults[1]);
        assert!(matches!(faults[2], Fault::BadEsp(_)), "{:?}", faults[2]);
        assert!(matches!(faults[3], Fault::NoImage(_)), "{:?}", faults[3]);
        assert!(matches!(faults[4], Fault::BadRecord(_)), "{:?}", faults[4]);
        assert!(matches!(faults[5], Fault::BadImage(..)), "{:?}", faults[5]);
        assert!(matches!(faults[6], Fault::NotKernel(_)), "{:?}", faults[6]);
        assert_eq!(device(0), "/dev/vdb");
        assert_eq!(device(6), "/dev/vdh");
    }
}
