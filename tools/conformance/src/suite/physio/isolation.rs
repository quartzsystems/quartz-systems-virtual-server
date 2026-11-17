// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Suite 11: isolation. The mappings the IOMMU has for a device given to
//! a partition cover the partition's memory and nothing else, read from
//! the kernel's own record of them: the Intel IOMMU driver's debugfs
//! dump of the device's page tables.

use qsvshook::proto::command;

use super::devices::{activate, end, located, probed, profile, release, rig};
use crate::qemu::PASSED_NVME;
use crate::qemu::session::Session;
use crate::suite::base::{Run, channel};
use crate::suite::partition::QSVS_MEMORY_MIB;
use crate::suite::partition::manager;

/// The driver's debugfs directory.
const INTEL: &str = "/sys/kernel/debug/iommu/intel";

/// The page tables of the device at `address`, as the driver dumps them.
fn record(g: &mut Session, address: &str) -> Result<(String, String), String> {
    channel(g)?.ask(command::DEBUGFS)?;
    // One file of every domain in older kernels; one for each device
    // and PASID since 6.6, the device's own DMA under PASID 0.
    let candidates = [
        format!("{INTEL}/{address}/0/domain_translation_struct"),
        format!("{INTEL}/domain_translation_struct"),
    ];
    let mut tried = Vec::new();
    for path in candidates {
        match channel(g)?.text(&path) {
            Ok(text) if !text.trim().is_empty() => return Ok((path, text)),
            Ok(_) => tried.push(format!("{path}: empty")),
            Err(e) => tried.push(format!("{path}: {e}")),
        }
    }
    Err(format!("no record of {address}: {}", tried.join("; ")))
}

/// The ranges of I/O virtual addresses a dump maps: each leaf line is
/// `0x<pfn> |` and its entries from the top level down, the last one
/// the leaf; three entries are a 1 GiB page, four 2 MiB, five 4 KiB.
/// For a file of every domain, only the lines under `address`'s
/// heading.
fn mapped(text: &str, address: &str, whole: bool) -> Vec<(u64, u64)> {
    let mut mine = !whole;
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Device ") {
            mine = rest.split_whitespace().next() == Some(address);
            continue;
        }
        if !mine {
            continue;
        }
        let Some((pfn, entries)) = line.split_once('|') else {
            continue;
        };
        let Some(pfn) = pfn
            .trim()
            .strip_prefix("0x")
            .and_then(|h| u64::from_str_radix(h, 16).ok())
        else {
            continue;
        };
        let bytes = match entries.split_whitespace().count() {
            3 => 1 << 30,
            4 => 2 << 20,
            5 => 4 << 10,
            _ => continue,
        };
        let start = pfn << 12;
        match ranges.last_mut() {
            Some(last) if last.0 + last.1 == start => last.1 += bytes,
            _ => ranges.push((start, bytes)),
        }
    }
    ranges
}

/// Suite 11.
pub fn isolation(run: &Run<'_>) -> Result<String, String> {
    let mut guest = rig(run, &[])?;
    let g = &mut guest;
    let nvme = located(g, PASSED_NVME, 0)?;
    release(g, &nvme)?;
    let id = manager::partition(g, "IO")?;
    activate(g, id, profile("IO", &[(&nvme, true)]))?;
    probed(g, "IO", "DISK nvme", run)?;
    let address = format!("0000:00:{PASSED_NVME:02x}.0");
    let (path, text) = record(g, &address)?;
    let ranges = mapped(
        &text,
        &address,
        path.ends_with("intel/domain_translation_struct"),
    );
    let memory = QSVS_MEMORY_MIB << 20;
    let shown: Vec<String> = ranges
        .iter()
        .map(|(s, b)| format!("{s:#x}+{b:#x}"))
        .collect();
    if ranges != [(0, memory)] {
        return Err(format!(
            "{address}'s mappings in {path}: {} (the partition has {memory:#x} bytes at 0)",
            shown.join(", ")
        ));
    }
    end(g, id)?;
    // Given back to the host: the partition's mappings are gone with it.
    let after = record(g, &address)
        .map(|(p, t)| mapped(&t, &address, p.ends_with("intel/domain_translation_struct")))
        .unwrap_or_default();
    if after == ranges {
        return Err(format!("the mappings outlived the partition: {after:?}"));
    }
    Ok(format!(
        "{path}: {address} maps {} and nothing else, the partition's {QSVS_MEMORY_MIB} MiB at \
         guest address 0; after the partition ended {} range(s) remain",
        shown.join(", "),
        after.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_lines_make_ranges_by_their_depth() {
        let text = "IOVA_PFN\t\tPML5E\t\t\tPML4E\t\t\tPDPE\t\t\tPDE\t\t\tPTE\n\
            0x0000000000000 |\t0x0\t0x1\t0x2\t0x83\n\
            0x0000000000200 |\t0x0\t0x1\t0x2\t0x83\n\
            0x0000000100000 |\t0x0\t0x1\t0x2\t0x3\t0x5\n";
        assert_eq!(
            mapped(text, "x", false),
            [(0, 4 << 20), (0x1_0000_0000, 4096)]
        );
    }

    #[test]
    fn a_file_of_every_domain_is_read_under_the_device_alone() {
        let text = "Device 0000:00:02.0 @0x1\n0x0000000000000 |\t0\t1\t2\t3\n\
            Device 0000:00:1b.0 @0x2\n0x0000000000200 |\t0\t1\t2\t3\n";
        assert_eq!(mapped(text, "0000:00:1b.0", true), [(2 << 20, 2 << 20)]);
    }
}
