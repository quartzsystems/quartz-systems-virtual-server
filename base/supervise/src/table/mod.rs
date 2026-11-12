// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The service table: fixed, compiled into PID 1.

mod policy;

use std::time::Duration;

pub use policy::{Policy, Verdict};

/// One service.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Service {
    /// Its name, in status and in logs.
    pub name: &'static str,
    /// The binary's path.
    pub binary: &'static str,
    /// Its arguments.
    pub args: &'static [&'static str],
    /// Its place in the start order; stopping goes the other way.
    pub order: u32,
    /// The services that must be ready before it starts.
    pub needs: &'static [&'static str],
    /// How long it may take to say it is ready.
    pub ready_within: Duration,
}

/// Why a table is not one.
pub fn check(table: &[Service]) -> Result<(), String> {
    for (i, s) in table.iter().enumerate() {
        if table[..i].iter().any(|o| o.name == s.name) {
            return Err(format!("service {} is listed twice", s.name));
        }
        if table[..i].iter().any(|o| o.order == s.order) {
            return Err(format!("service {} shares its order {}", s.name, s.order));
        }
        for need in s.needs {
            match table.iter().find(|o| o.name == *need) {
                Some(o) if o.order < s.order => {}
                Some(_) => return Err(format!("service {} starts before {need}", s.name)),
                None => return Err(format!("service {} needs {need}, which is not one", s.name)),
            }
        }
    }
    Ok(())
}

/// The table's indexes in start order.
pub fn start_order(table: &[Service]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..table.len()).collect();
    order.sort_by_key(|i| table[*i].order);
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn svc(name: &'static str, order: u32, needs: &'static [&'static str]) -> Service {
        Service {
            name,
            binary: "/bin/true",
            args: &[],
            order,
            needs,
            ready_within: Duration::from_secs(10),
        }
    }

    #[test]
    fn a_table_is_checked() {
        let good = [svc("netd", 20, &["logd"]), svc("logd", 10, &[])];
        assert_eq!(check(&good), Ok(()));
        assert_eq!(start_order(&good), [1, 0]);
        for bad in [
            vec![svc("a", 1, &[]), svc("a", 2, &[])],
            vec![svc("a", 1, &[]), svc("b", 1, &[])],
            vec![svc("a", 1, &["b"]), svc("b", 2, &[])],
            vec![svc("a", 1, &["c"])],
            vec![svc("a", 1, &["a"])],
        ] {
            assert!(check(&bad).is_err(), "{bad:?}");
        }
    }
}
