// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! Civil time. The system clock is UTC; a [`Zone`] moves what is
//! displayed and nothing else. A zone is `UTC` or a fixed offset,
//! `UTC+01:00`.

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the epoch, now.
pub fn now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

/// Nanoseconds since the epoch, now.
pub fn now_nanos() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
    }
}

/// A calendar date and a time of day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    /// Year.
    pub year: i64,
    /// Month, from 1.
    pub month: u32,
    /// Day of the month, from 1.
    pub day: u32,
    /// Hour.
    pub hour: u32,
    /// Minute.
    pub minute: u32,
    /// Second.
    pub second: u32,
}

/// The date and time of `secs` since the epoch (proleptic Gregorian).
pub fn civil(secs: i64) -> Civil {
    let days = secs.div_euclid(86_400);
    let rem = u32::try_from(secs.rem_euclid(86_400)).unwrap_or(0);
    // Days to a date, counting from 0000-03-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    Civil {
        year,
        month,
        day,
        hour: rem / 3600,
        minute: rem % 3600 / 60,
        second: rem % 60,
    }
}

/// `2026-11-12T10:22:03.123456789Z`, what a log record carries.
pub fn rfc3339(nanos: i64) -> String {
    let c = civil(nanos.div_euclid(1_000_000_000));
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z",
        c.year,
        c.month,
        c.day,
        c.hour,
        c.minute,
        c.second,
        nanos.rem_euclid(1_000_000_000)
    )
}

/// A display zone: minutes east of UTC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Zone {
    /// Minutes east of UTC.
    pub minutes: i32,
}

impl Zone {
    /// UTC.
    pub const UTC: Zone = Zone { minutes: 0 };

    /// `2026-11-12 10:22 UTC+01:00`: `secs` as this zone shows it, to the minute.
    pub fn display(self, secs: i64) -> String {
        let c = civil(secs + i64::from(self.minutes) * 60);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02} {self}",
            c.year, c.month, c.day, c.hour, c.minute
        )
    }
}

impl fmt::Display for Zone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.minutes == 0 {
            return f.write_str("UTC");
        }
        let sign = if self.minutes < 0 { '-' } else { '+' };
        let m = self.minutes.unsigned_abs();
        write!(f, "UTC{sign}{:02}:{:02}", m / 60, m % 60)
    }
}

impl FromStr for Zone {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || format!("`{s}` is not a time zone (UTC, or UTC+hh:mm, UTC-hh:mm)");
        let rest = s.strip_prefix("UTC").ok_or_else(bad)?;
        if rest.is_empty() {
            return Ok(Zone::UTC);
        }
        let (sign, digits) = match rest.split_at_checked(1) {
            Some(("+", d)) => (1, d),
            Some(("-", d)) => (-1, d),
            _ => return Err(bad()),
        };
        let (h, m) = digits.split_once(':').unwrap_or((digits, "00"));
        let two = |t: &str| (t.len() == 2).then(|| t.parse::<i32>().ok()).flatten();
        match (two(h), two(m)) {
            (Some(h), Some(m)) if h <= 14 && m < 60 => Ok(Zone {
                minutes: sign * (h * 60 + m),
            }),
            _ => Err(bad()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_gregorian() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000000000Z");
        // 2026-11-12 10:22:03 UTC.
        let s = 1_794_478_923;
        assert_eq!(
            rfc3339(s * 1_000_000_000 + 5),
            "2026-11-12T10:22:03.000000005Z"
        );
        // A leap day, and the day before the epoch.
        assert_eq!(civil(951_782_400).day, 29);
        assert_eq!(civil(951_782_400).month, 2);
        assert_eq!(rfc3339(-1_000_000_000), "1969-12-31T23:59:59.000000000Z");
    }

    #[test]
    fn zones_parse_print_and_shift_the_display_only() {
        let s = 1_794_478_923;
        assert_eq!(Zone::UTC.display(s), "2026-11-12 10:22 UTC");
        let east: Zone = "UTC+05:30".parse().unwrap();
        assert_eq!(east.minutes, 330);
        assert_eq!(east.display(s), "2026-11-12 15:52 UTC+05:30");
        let west: Zone = "UTC-11".parse().unwrap();
        assert_eq!(west.to_string(), "UTC-11:00");
        assert_eq!(west.display(s), "2026-11-11 23:22 UTC-11:00");
        assert_eq!("UTC".parse::<Zone>().unwrap(), Zone::UTC);
        for bad in ["", "utc", "Europe/Berlin", "UTC+1", "UTC+15:00", "UTC+01:60", "UTC01"] {
            assert!(bad.parse::<Zone>().is_err(), "{bad}");
        }
    }
}
