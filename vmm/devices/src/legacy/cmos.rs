// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The CMOS clock: the index port and the data port, the time
//! registers in BCD and 24-hour form, UTC, which is how an x86 kernel
//! reads them whatever the mode bit says. A guest that sets the
//! clock moves it for as long as the partition runs: the difference
//! from the host's clock is kept in memory only. It raises no
//! interrupt.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::bus::BusDevice;

const SECONDS: u8 = 0x00;
const MINUTES: u8 = 0x02;
const HOURS: u8 = 0x04;
const WEEKDAY: u8 = 0x06;
const DAY: u8 = 0x07;
const MONTH: u8 = 0x08;
const YEAR: u8 = 0x09;
const STATUS_A: u8 = 0x0A;
const STATUS_B: u8 = 0x0B;
const STATUS_D: u8 = 0x0D;
const CENTURY: u8 = 0x32;

/// Status A: the usual divider and rate; never an update in progress.
const STATUS_A_VALUE: u8 = 0x26;
/// Status B: 24-hour; the data mode bit is clear, BCD.
const B_24_HOUR: u8 = 0x02;
/// Status B: the guest is setting the time.
const B_SET: u8 = 0x80;
/// Status D: the clock's memory is valid.
const STATUS_D_VALUE: u8 = 0x80;

/// A civil date and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Civil {
    /// The year, in full.
    pub year: i64,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
    /// 0 to 59.
    pub second: u8,
}

/// A value below 100 as two decimal digits.
const fn bcd(value: u8) -> u8 {
    ((value / 10) << 4) | (value % 10)
}

/// Two decimal digits as a value; `None` when a digit is not one.
const fn from_bcd(value: u8) -> Option<u8> {
    if value >> 4 > 9 || value & 0xF > 9 {
        None
    } else {
        Some((value >> 4) * 10 + (value & 0xF))
    }
}

/// Days from 1970-01-01 to a civil date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // each value is in its range
impl Civil {
    /// The civil time of `unix` seconds.
    pub fn from_unix(unix: i64) -> Self {
        let days = unix.div_euclid(86_400);
        let rem = unix.rem_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        Self {
            year,
            month: month as u8,
            day: day as u8,
            hour: (rem / 3600) as u8,
            minute: (rem % 3600 / 60) as u8,
            second: (rem % 60) as u8,
        }
    }

    /// Unix seconds of this civil time.
    pub fn to_unix(&self) -> i64 {
        days_from_civil(self.year, i64::from(self.month), i64::from(self.day)) * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    /// 1 for Sunday to 7 for Saturday.
    fn weekday(&self) -> u8 {
        let days = days_from_civil(self.year, i64::from(self.month), i64::from(self.day));
        ((days + 4).rem_euclid(7) + 1) as u8
    }
}

/// Where the clock takes the time from.
pub type Now = Box<dyn Fn() -> i64 + Send>;

/// The clock.
pub struct Cmos {
    index: u8,
    status_b: u8,
    /// Seconds the guest's clock is ahead of the host's.
    offset: i64,
    /// What the guest wrote while setting the time.
    staged: Option<Civil>,
    now: Now,
}

impl Cmos {
    /// A clock on the host's.
    pub fn new() -> Self {
        Self::with_clock(Box::new(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        }))
    }

    /// A clock on `now`.
    pub fn with_clock(now: Now) -> Self {
        Self {
            index: 0,
            status_b: B_24_HOUR,
            offset: 0,
            staged: None,
            now,
        }
    }

    fn time(&self) -> Civil {
        self.staged
            .unwrap_or_else(|| Civil::from_unix((self.now)().saturating_add(self.offset)))
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // reduced first
    fn register(&self, index: u8) -> u8 {
        let t = self.time();
        match index {
            SECONDS => bcd(t.second),
            MINUTES => bcd(t.minute),
            HOURS => bcd(t.hour),
            WEEKDAY => t.weekday(),
            DAY => bcd(t.day),
            MONTH => bcd(t.month),
            YEAR => bcd(t.year.rem_euclid(100) as u8),
            CENTURY => bcd(t.year.div_euclid(100).clamp(0, 99) as u8),
            STATUS_A => STATUS_A_VALUE,
            STATUS_B => self.status_b,
            STATUS_D => STATUS_D_VALUE,
            // Status C, where no interrupt is ever pending, and the rest
            // of the memory read zero.
            _ => 0,
        }
    }

    /// Take what the guest wrote as the time, if it is one.
    fn commit(&mut self, t: Civil) {
        let valid = (1..=12).contains(&t.month)
            && (1..=31).contains(&t.day)
            && t.hour < 24
            && t.minute < 60
            && t.second < 60
            && (1970..=9999).contains(&t.year);
        if valid {
            self.offset = t.to_unix().saturating_sub((self.now)());
        }
    }

    fn set_register(&mut self, index: u8, raw: u8) {
        let mut t = self.time();
        if index == STATUS_B {
            let was_setting = self.status_b & B_SET != 0;
            // The data mode stays BCD and 24-hour.
            self.status_b = (raw & B_SET) | B_24_HOUR;
            if was_setting && raw & B_SET == 0 {
                if let Some(staged) = self.staged.take() {
                    self.commit(staged);
                }
            } else if !was_setting && raw & B_SET != 0 {
                self.staged = Some(t);
            }
            return;
        }
        // A value that is not BCD is no value.
        let Some(value) = from_bcd(raw) else {
            return;
        };
        match index {
            SECONDS => t.second = value,
            MINUTES => t.minute = value,
            HOURS => t.hour = value,
            DAY => t.day = value,
            MONTH => t.month = value,
            YEAR => t.year = t.year.div_euclid(100) * 100 + i64::from(value),
            CENTURY => t.year = i64::from(value) * 100 + t.year.rem_euclid(100),
            _ => return,
        }
        if self.staged.is_some() {
            self.staged = Some(t);
        } else {
            self.commit(t);
        }
    }
}

impl Default for Cmos {
    fn default() -> Self {
        Self::new()
    }
}

impl BusDevice for Cmos {
    fn read(&mut self, offset: u64, data: &mut [u8]) {
        data.fill(0xFF);
        if let Some(first) = data.first_mut() {
            *first = match offset {
                0 => self.index,
                1 => self.register(self.index),
                _ => 0xFF,
            };
        }
    }

    fn write(&mut self, offset: u64, data: &[u8]) {
        let Some(value) = data.first().copied() else {
            return;
        };
        match offset {
            // Bit 7 of the index port is the NMI mask.
            0 => self.index = value & 0x7F,
            1 => self.set_register(self.index, value),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-12 13:14:15 UTC, a Thursday.
    const THEN: i64 = 1_794_489_255;

    fn clock() -> Cmos {
        Cmos::with_clock(Box::new(|| THEN))
    }

    fn get(cmos: &mut Cmos, index: u8) -> u8 {
        cmos.write(0, &[index]);
        let mut value = [0u8];
        cmos.read(1, &mut value);
        value[0]
    }

    fn put(cmos: &mut Cmos, index: u8, value: u8) {
        cmos.write(0, &[index]);
        cmos.write(1, &[value]);
    }

    #[test]
    fn civil_time_round_trips() {
        for unix in [0, 951_868_800, THEN, 4_102_444_799, 253_402_300_799] {
            assert_eq!(Civil::from_unix(unix).to_unix(), unix);
        }
        let t = Civil::from_unix(THEN);
        assert_eq!((t.year, t.month, t.day), (2026, 11, 12));
        assert_eq!((t.hour, t.minute, t.second), (13, 14, 15));
        assert_eq!(t.weekday(), 5);
    }

    #[test]
    fn the_registers_give_the_time_in_bcd() {
        let mut cmos = clock();
        assert_eq!(get(&mut cmos, SECONDS), 0x15);
        assert_eq!(get(&mut cmos, MINUTES), 0x14);
        assert_eq!(get(&mut cmos, HOURS), 0x13);
        assert_eq!(get(&mut cmos, DAY), 0x12);
        assert_eq!(get(&mut cmos, MONTH), 0x11);
        assert_eq!(get(&mut cmos, YEAR), 0x26);
        assert_eq!(get(&mut cmos, CENTURY), 0x20);
        assert_eq!(get(&mut cmos, STATUS_A) & 0x80, 0);
        assert_eq!(get(&mut cmos, STATUS_B), 0x02);
        assert_eq!(get(&mut cmos, STATUS_D), 0x80);
        assert_eq!(get(&mut cmos, 0x7F), 0);
    }

    #[test]
    fn a_guest_that_sets_the_clock_reads_its_own_time() {
        let mut cmos = clock();
        put(&mut cmos, STATUS_B, 0x82);
        put(&mut cmos, YEAR, 0x30);
        put(&mut cmos, MONTH, 1);
        put(&mut cmos, DAY, 2);
        put(&mut cmos, HOURS, 3);
        put(&mut cmos, MINUTES, 4);
        put(&mut cmos, SECONDS, 5);
        put(&mut cmos, STATUS_B, 0x02);
        assert_eq!(get(&mut cmos, YEAR), 0x30);
        assert_eq!(get(&mut cmos, MONTH), 1);
        assert_eq!(get(&mut cmos, HOURS), 3);
        assert_eq!(get(&mut cmos, SECONDS), 5);
        assert_eq!(
            cmos.offset,
            Civil {
                year: 2030,
                month: 1,
                day: 2,
                hour: 3,
                minute: 4,
                second: 5
            }
            .to_unix()
                - THEN
        );
    }

    #[test]
    fn a_time_that_is_none_is_not_taken() {
        let mut cmos = clock();
        put(&mut cmos, STATUS_B, 0x80);
        put(&mut cmos, MONTH, 0x99);
        put(&mut cmos, HOURS, 200);
        put(&mut cmos, STATUS_B, 0);
        assert_eq!(cmos.offset, 0);
        assert_eq!(get(&mut cmos, MONTH), 0x11);
        // Outside a set, a bad value alone is dropped too.
        put(&mut cmos, SECONDS, 250);
        assert_eq!(get(&mut cmos, SECONDS), 0x15);
    }
}
