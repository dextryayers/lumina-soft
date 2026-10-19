//! Local time without a calendar dependency.
//!
//! # Why this exists
//!
//! Lumina needs one thing from a calendar library: turn a `SystemTime` into a
//! local civil date, with correct month lengths, leap years, and weekdays. That
//! is roughly forty lines of arithmetic (Howard Hinnant's `civil_from_days`),
//! and carrying a full timezone database for it would cost more than the whole
//! widget crate. The offset comes from the operating system; the calendar math
//! is pure and exhaustively testable.
//!
//! # Accuracy
//!
//! The offset is the one in effect *now*, which is what a clock displaying the
//! current time needs. Rendering a historical timestamp in the past with the
//! current offset would be wrong across a daylight saving boundary; nothing in
//! Lumina does that.

use std::time::{SystemTime, UNIX_EPOCH};

/// A civil date and time in a fixed UTC offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilTime {
    /// Year, for example 2026.
    pub year: i32,
    /// Month, 1 through 12.
    pub month: u32,
    /// Day of month, 1 through 31.
    pub day: u32,
    /// Weekday, 0 is Sunday and 6 is Saturday.
    pub weekday: u32,
    /// Hour, 0 through 23.
    pub hour: u32,
    /// Minute, 0 through 59.
    pub minute: u32,
    /// Second, 0 through 59.
    pub second: u32,
    /// The UTC offset used, in minutes east of UTC.
    pub offset_minutes: i32,
}

/// The UTC offset currently in effect, in minutes east of UTC.
///
/// On Windows this reads the active time zone, including daylight saving. On
/// Unix it asks `localtime_r`, so `TZ` and the system database both apply.
/// Platforms without an implementation return zero, which renders UTC rather
/// than a wrong local time.
#[must_use]
pub fn local_utc_offset_minutes() -> i32 {
    platform_offset()
}

#[cfg(target_os = "windows")]
fn platform_offset() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};

    /// `TIME_ZONE_ID_DAYLIGHT` from the platform SDK; the `windows` crate
    /// exposes it in `SystemServices`, which we do not otherwise need.
    const TIME_ZONE_ID_DAYLIGHT: u32 = 2;

    let mut zone = TIME_ZONE_INFORMATION::default();
    // The return value is a TIME_ZONE_ID_* constant, not an error code.
    let state = unsafe { GetTimeZoneInformation(&raw mut zone) };
    if state == u32::MAX {
        return 0;
    }
    let bias = if state == TIME_ZONE_ID_DAYLIGHT {
        zone.Bias + zone.DaylightBias
    } else {
        zone.Bias + zone.StandardBias
    };
    // The Win32 bias is minutes *west* of UTC, the opposite sign of the usual
    // offset convention, to which every part of Lumina adheres.
    -bias
}

#[cfg(all(unix, not(target_os = "windows")))]
fn platform_offset() -> i32 {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    if now == -1 {
        return 0;
    }
    let mut local = unsafe { std::mem::zeroed::<libc::tm>() };
    let result = unsafe { libc::localtime_r(&raw const now, &raw mut local) };
    if result.is_null() {
        return 0;
    }
    (local.tm_gmtoff / 60) as i32
}

#[cfg(not(any(target_os = "windows", unix)))]
fn platform_offset() -> i32 {
    0
}

/// Convert a Unix timestamp and a UTC offset into civil time.
///
/// Floor division and a non-negative remainder are used throughout, so dates
/// before 1970 are handled correctly rather than being off by a day.
#[must_use]
pub fn civil_from_unix_with_offset(unix_secs: i64, offset_minutes: i32) -> CivilTime {
    let offset_secs = i64::from(offset_minutes) * 60;
    let local_secs = unix_secs.saturating_add(offset_secs);
    let days = local_secs.div_euclid(86_400);
    let seconds_of_day = local_secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    CivilTime {
        year,
        month,
        day,
        weekday: weekday_from_days(days),
        hour: (seconds_of_day / 3_600) as u32,
        minute: ((seconds_of_day % 3_600) / 60) as u32,
        second: (seconds_of_day % 60) as u32,
        offset_minutes,
    }
}

/// Convert a `SystemTime` to civil time in the system's current local offset.
#[must_use]
pub fn local_civil_time(now: SystemTime) -> CivilTime {
    let unix_secs = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    civil_from_unix_with_offset(unix_secs, local_utc_offset_minutes())
}

/// Days since the Unix epoch to a Gregorian date.
///
/// This is Howard Hinnant's `civil_from_days`, valid for all `i64` inputs.
#[must_use]
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

/// Days since the Unix epoch to a weekday, 0 is Sunday.
#[must_use]
pub fn weekday_from_days(days: i64) -> u32 {
    (days + 4).rem_euclid(7) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_unix_epoch_is_a_thursday() {
        let civil = civil_from_unix_with_offset(0, 0);
        assert_eq!((civil.year, civil.month, civil.day), (1970, 1, 1));
        assert_eq!(civil.weekday, 4);
        assert_eq!((civil.hour, civil.minute, civil.second), (0, 0, 0));
    }

    #[test]
    fn a_leap_day_is_not_mistaken_for_march_first() {
        let civil = civil_from_unix_with_offset(951_782_400, 0);
        assert_eq!((civil.year, civil.month, civil.day), (2000, 2, 29));
        assert_eq!(civil.weekday, 2);
    }

    #[test]
    fn the_current_date_from_the_plan_is_a_monday() {
        let civil = civil_from_unix_with_offset(1_792_368_000, 0);
        assert_eq!((civil.year, civil.month, civil.day), (2026, 10, 19));
        assert_eq!(civil.weekday, 1);
    }

    #[test]
    fn a_positive_offset_can_roll_into_the_next_day() {
        let civil = civil_from_unix_with_offset(0, 120);
        assert_eq!((civil.year, civil.month, civil.day), (1970, 1, 1));
        assert_eq!(civil.hour, 2);
        let civil = civil_from_unix_with_offset(86_400 - 7_200, 120);
        assert_eq!((civil.year, civil.month, civil.day), (1970, 1, 2));
        assert_eq!(civil.hour, 0);
    }

    #[test]
    fn dates_before_the_epoch_are_handled() {
        let civil = civil_from_unix_with_offset(-86_400, 0);
        assert_eq!((civil.year, civil.month, civil.day), (1969, 12, 31));
        assert_eq!(civil.weekday, 3);
    }

    #[test]
    fn month_boundaries_across_a_year_are_correct() {
        // 2024-03-01 00:00:00 UTC, the day after a leap day.
        let civil = civil_from_unix_with_offset(1_709_251_200, 0);
        assert_eq!((civil.year, civil.month, civil.day), (2024, 3, 1));
        // 2023-12-31 23:59:59 UTC.
        let civil = civil_from_unix_with_offset(1_704_067_199, 0);
        assert_eq!((civil.year, civil.month, civil.day), (2023, 12, 31));
        assert_eq!((civil.hour, civil.minute, civil.second), (23, 59, 59));
    }

    #[test]
    fn local_civil_time_uses_the_system_offset() {
        let civil = local_civil_time(UNIX_EPOCH + Duration::from_secs(0));
        assert_eq!(civil.offset_minutes, local_utc_offset_minutes());
    }

    #[test]
    fn the_offset_is_within_a_sane_range() {
        let offset = local_utc_offset_minutes();
        assert!(
            (-12 * 60..=14 * 60).contains(&offset),
            "offset was {offset}"
        );
    }
}
