// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The kernel's PCI files, written: a function given to the
//! pass-through driver or back to the host's, reset alone, its group
//! read. PID 1 binds at the IPL and the partition manager after; both
//! through this, on the sysfs root they are given.

use std::fs;
use std::path::{Path, PathBuf};

/// The pass-through driver.
pub const PASSTHROUGH: &str = "vfio-pci";

fn device(sys: &Path, address: &str) -> PathBuf {
    sys.join("bus/pci/devices").join(address)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The driver bound to the function at `address`, if any.
pub fn driver(sys: &Path, address: &str) -> Option<String> {
    let target = fs::read_link(device(sys, address).join("driver")).ok()?;
    Some(target.file_name()?.to_string_lossy().into_owned())
}

/// The isolation group of the function at `address`.
pub fn group(sys: &Path, address: &str) -> Option<u32> {
    let target = fs::read_link(device(sys, address).join("iommu_group")).ok()?;
    target.file_name()?.to_str()?.parse().ok()
}

fn checked(address: &str) -> Result<(), String> {
    let fine = address.len() <= 16
        && !address.is_empty()
        && address
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    if fine {
        Ok(())
    } else {
        Err(format!("{address:?} is not a PCI address"))
    }
}

/// Give the function at `address` to the pass-through driver: its
/// driver override set so that no other driver takes it, its host
/// driver let go of it, and the pass-through driver probed.
pub fn to_passthrough(sys: &Path, address: &str) -> Result<(), String> {
    checked(address)?;
    let dir = device(sys, address);
    if !dir.exists() {
        return Err(format!("{address} is not on the host"));
    }
    write(&dir.join("driver_override"), PASSTHROUGH)?;
    match driver(sys, address) {
        Some(d) if d == PASSTHROUGH => return Ok(()),
        Some(_) => write(&dir.join("driver/unbind"), address)?,
        None => {}
    }
    write(&sys.join("bus/pci/drivers_probe"), address)?;
    match driver(sys, address) {
        Some(d) if d == PASSTHROUGH => Ok(()),
        other => Err(format!(
            "{address} has {} after the probe",
            other.unwrap_or_else(|| "no driver".into())
        )),
    }
}

/// Give the function at `address` back to the host's drivers: no
/// override, the pass-through driver let go of it, the host's probed.
pub fn to_host(sys: &Path, address: &str) -> Result<(), String> {
    checked(address)?;
    let dir = device(sys, address);
    write(&dir.join("driver_override"), "\n")?;
    if driver(sys, address).as_deref() == Some(PASSTHROUGH) {
        write(&dir.join("driver/unbind"), address)?;
    }
    write(&sys.join("bus/pci/drivers_probe"), address)
}

/// Reset the function at `address` alone, by the first method the
/// kernel has for it.
pub fn reset(sys: &Path, address: &str) -> Result<(), String> {
    checked(address)?;
    let file = device(sys, address).join("reset");
    if !file.exists() {
        return Err(format!("{address} cannot be reset"));
    }
    write(&file, "1")
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("pio-bind-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("bus/pci/devices/0000:00:06.0")).unwrap();
        fs::create_dir_all(root.join("bus/pci/drivers/nvme")).unwrap();
        fs::create_dir_all(root.join("bus/pci/drivers/vfio-pci")).unwrap();
        fs::create_dir_all(root.join("kernel/iommu_groups/7")).unwrap();
        root
    }

    #[test]
    fn a_function_bound_to_its_host_driver_is_let_go_and_probed() {
        let root = tree("pass");
        let dev = root.join("bus/pci/devices/0000:00:06.0");
        symlink(root.join("bus/pci/drivers/nvme"), dev.join("driver")).unwrap();
        symlink(root.join("kernel/iommu_groups/7"), dev.join("iommu_group")).unwrap();
        assert_eq!(group(&root, "0000:00:06.0"), Some(7));
        assert_eq!(driver(&root, "0000:00:06.0").as_deref(), Some("nvme"));
        // Nothing binds in a tree of files: the probe is seen to fail.
        let e = to_passthrough(&root, "0000:00:06.0").unwrap_err();
        assert!(e.contains("nvme after the probe"), "{e}");
        assert_eq!(
            fs::read_to_string(dev.join("driver_override")).unwrap(),
            PASSTHROUGH
        );
        assert_eq!(
            fs::read_to_string(root.join("bus/pci/drivers/nvme/unbind")).unwrap(),
            "0000:00:06.0"
        );
        assert_eq!(
            fs::read_to_string(root.join("bus/pci/drivers_probe")).unwrap(),
            "0000:00:06.0"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn one_already_passed_is_left_and_one_given_back_is_probed() {
        let root = tree("back");
        let dev = root.join("bus/pci/devices/0000:00:06.0");
        symlink(root.join("bus/pci/drivers/vfio-pci"), dev.join("driver")).unwrap();
        to_passthrough(&root, "0000:00:06.0").unwrap();
        assert!(!root.join("bus/pci/drivers_probe").exists());
        to_host(&root, "0000:00:06.0").unwrap();
        assert_eq!(
            fs::read_to_string(dev.join("driver_override")).unwrap(),
            "\n"
        );
        assert!(root.join("bus/pci/drivers/vfio-pci/unbind").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_reset_needs_the_kernel_s_file_and_a_real_address() {
        let root = tree("reset");
        assert!(
            reset(&root, "0000:00:06.0")
                .unwrap_err()
                .contains("cannot be reset")
        );
        fs::write(root.join("bus/pci/devices/0000:00:06.0/reset"), "").unwrap();
        reset(&root, "0000:00:06.0").unwrap();
        assert!(reset(&root, "../../x").is_err());
        assert!(
            to_passthrough(&root, "0000:00:09.0")
                .unwrap_err()
                .contains("not on the host")
        );
        let _ = fs::remove_dir_all(root);
    }
}
