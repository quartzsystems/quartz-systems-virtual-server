// Copyright (C), 2026 Quartz Systems. Some rights reserved. This work is
// licensed under the terms of the MIT license which can be found in the
// root directory of this project.

//! The system clock as QS/400's `ntp` adjusts it, and the write back
//! to the RTC. The two calls no safe wrapper exists for are here.

#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

use appliance::time::{civil, now_nanos};
use ntp::Clock;
use rustix::time::{ClockId, Timespec, clock_settime};

/// The first RTC.
const RTC: &str = "/dev/rtc0";
/// `RTC_SET_TIME`: `_IOW('p', 0x0a, struct rtc_time)`. The request's
/// type is the C library's.
#[cfg(target_env = "musl")]
const RTC_SET_TIME: libc::c_int = 0x4024_700a;
#[cfg(not(target_env = "musl"))]
const RTC_SET_TIME: libc::c_ulong = 0x4024_700a;

/// `CLOCK_REALTIME`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        now_nanos()
    }

    fn slew(&self, offset: i64) -> Result<(), String> {
        // SAFETY: a zeroed timex is a valid one; adjtimex reads the
        // fields `modes` names and writes the structure back.
        let done = unsafe {
            let mut t: libc::timex = std::mem::zeroed();
            t.modes = libc::ADJ_OFFSET | libc::ADJ_NANO;
            t.offset = offset;
            libc::adjtimex(&raw mut t)
        };
        if done < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    fn step(&self, offset: i64) -> Result<(), String> {
        let to = now_nanos().saturating_add(offset);
        let at = Timespec {
            tv_sec: to.div_euclid(1_000_000_000),
            tv_nsec: to.rem_euclid(1_000_000_000),
        };
        clock_settime(ClockId::Realtime, at).map_err(|e| e.to_string())
    }
}

/// `struct rtc_time`.
#[repr(C)]
struct RtcTime {
    sec: libc::c_int,
    min: libc::c_int,
    hour: libc::c_int,
    mday: libc::c_int,
    mon: libc::c_int,
    year: libc::c_int,
    wday: libc::c_int,
    yday: libc::c_int,
    isdst: libc::c_int,
}

fn int(n: impl TryInto<libc::c_int>) -> libc::c_int {
    n.try_into().unwrap_or(0)
}

/// Set the RTC from the system clock (UTC).
pub fn write_rtc() -> io::Result<()> {
    let now = civil(now_nanos().div_euclid(1_000_000_000));
    let time = RtcTime {
        sec: int(now.second),
        min: int(now.minute),
        hour: int(now.hour),
        mday: int(now.day),
        mon: int(now.month) - 1,
        year: int(now.year - 1900),
        wday: 0,
        yday: 0,
        isdst: 0,
    };
    let rtc = File::open(RTC)?;
    // SAFETY: the descriptor is open, the request takes a pointer to a
    // `struct rtc_time`, and `time` is one that outlives the call.
    let done = unsafe { libc::ioctl(rtc.as_raw_fd(), RTC_SET_TIME, &raw const time) };
    if done < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
