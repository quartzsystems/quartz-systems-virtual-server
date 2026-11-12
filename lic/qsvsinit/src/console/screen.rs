// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The local status screen: plain text, drawn again when it changes.
//! A placeholder until the Service Tools console; it shows what the
//! control socket's status says and nothing else.

use release::fact as key;
use release::screen as text;
use supervise::control::HostStatus;

pub use release::screen::{BEGIN, END};

/// The rows of the screen: the label and the fact it shows.
const ROWS: [(&str, &str); 5] = [
    (text::SLOT, key::SLOT),
    (text::HOST, key::HOST),
    (text::MANAGEMENT, key::MANAGEMENT),
    (text::DATE, key::DATE),
    (text::REFERENCE, key::REFERENCE),
];

/// The screen for `status`, under `banner`.
pub fn lines(banner: &str, status: &HostStatus) -> Vec<String> {
    let mut out = vec![banner.to_string()];
    for (label, key) in ROWS {
        let mut value = status.fact(key).unwrap_or("").to_string();
        if key == key::SLOT
            && let Some(record) = status.fact(key::RECORD)
        {
            value = format!("{value} ({record})");
        }
        out.push(text::row(label, &value));
    }
    for service in &status.services {
        let failures = match service.failures {
            0 => String::new(),
            1 => ", 1 failure".to_string(),
            n => format!(", {n} failures"),
        };
        out.push(text::service(
            &service.name,
            &format!("{}{failures}", service.state),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use supervise::control::ServiceStatus;

    #[test]
    fn the_screen_is_the_banner_five_facts_and_a_line_a_service() {
        let fact = |k: &str, v: &str| (k.to_string(), v.to_string());
        let status = HostStatus {
            facts: vec![
                fact(key::SLOT, "A"),
                fact(key::RECORD, "default A, nothing pending"),
                fact(key::HOST, "QSVS123456"),
                fact(key::MANAGEMENT, "10.0.2.15/24 on eth0, gateway 10.0.2.2"),
                fact(key::DATE, "2026-11-12 10:22 UTC"),
                fact(key::REFERENCE, "C2001090 Base ready"),
                fact(key::TIME, "1794478923"),
            ],
            services: vec![
                ServiceStatus {
                    name: "logd".into(),
                    state: "ready".into(),
                    pid: 2,
                    failures: 0,
                },
                ServiceStatus {
                    name: "netd".into(),
                    state: "restarting".into(),
                    pid: 0,
                    failures: 2,
                },
            ],
        };
        assert_eq!(
            lines("QS/VS V1R0M0 abc1234", &status),
            [
                "QS/VS V1R0M0 abc1234",
                "Slot           A (default A, nothing pending)",
                "Host name      QSVS123456",
                "Management     10.0.2.15/24 on eth0, gateway 10.0.2.2",
                "Date and time  2026-11-12 10:22 UTC",
                "Reference      C2001090 Base ready",
                "Service        logd       ready",
                "Service        netd       restarting, 2 failures",
            ]
        );
    }
}
