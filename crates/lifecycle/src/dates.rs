//! Calendar days without a timezone library: the event date is a civil day
//! (`YYYY-MM-DD`), the erasure date is that day plus a number of days, and
//! both are shown to entrants as dates. Only the clock the housekeeping run
//! compares against is an instant; the organizer's browser supplies it (the
//! end of the event day where they are), or it defaults to the end of the day
//! in UTC.
//!
//! The algorithms are Howard Hinnant's `days_from_civil` and
//! `civil_from_days`, valid for every date the software will ever see.

/// Seconds in a day.
pub const DAY_SECS: i64 = 86_400;

/// A calendar day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i64,
    pub month: u8,
    pub day: u8,
}

impl Date {
    /// The date `s` names, in `YYYY-MM-DD`; `None` if it isn't one.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let mut parts = s.split('-');
        let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || y.len() != 4 || m.len() != 2 || d.len() != 2 {
            return None;
        }
        let date = Self {
            year: y.parse().ok()?,
            month: m.parse().ok()?,
            day: d.parse().ok()?,
        };
        date.valid().then_some(date)
    }

    fn valid(self) -> bool {
        (1970..=9999).contains(&self.year)
            && (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
    }

    /// Days since 1970-01-01.
    pub fn days(self) -> i64 {
        let (y, m, d) = (self.year, i64::from(self.month), i64::from(self.day));
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// The date `days` after 1970-01-01.
    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        Self {
            year: y,
            month: m as u8,
            day: d as u8,
        }
    }

    /// The date `n` days later (or earlier, if negative).
    pub fn plus_days(self, n: i64) -> Self {
        Self::from_days(self.days() + n)
    }

    /// The instant the day starts, in UTC (unix seconds).
    pub fn starts_utc(self) -> i64 {
        self.days() * DAY_SECS
    }

    /// The instant the day ends, in UTC (unix seconds): the next day's start.
    pub fn ends_utc(self) -> i64 {
        self.starts_utc() + DAY_SECS
    }

    /// The day `unix` falls on, in UTC.
    pub fn of_utc(unix: i64) -> Self {
        Self::from_days(unix.div_euclid(DAY_SECS))
    }

    /// `YYYY-MM-DD`.
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// The date in words, as entrants read it: "28 October 2026".
    pub fn long(self) -> String {
        format!(
            "{} {} {}",
            self.day,
            MONTHS[usize::from(self.month) - 1],
            self.year
        )
    }
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.iso())
    }
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

/// How far from the end of the day in UTC the organizer's end of day may be:
/// a timezone, at most 14 hours either way (UTC-12 to UTC+14).
pub const TIMEZONE_SLACK_SECS: i64 = 14 * 3600;

/// The instant `date` ends where the organizer is: `ends_at` if the browser
/// said and it is within a timezone of UTC's, otherwise the end of the day in
/// UTC.
pub fn ends_at(date: Date, ends_at: Option<i64>) -> i64 {
    let utc = date.ends_utc();
    match ends_at {
        Some(at) if (at - utc).abs() <= TIMEZONE_SLACK_SECS => at,
        _ => utc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_dates() {
        let d = Date::parse("2026-10-28").unwrap();
        assert_eq!((d.year, d.month, d.day), (2026, 10, 28));
        assert_eq!(d.iso(), "2026-10-28");
        assert_eq!(d.long(), "28 October 2026");
        assert_eq!(Date::parse(" 2026-01-05 ").unwrap().day, 5);
        for bad in [
            "",
            "2026-13-01",
            "2026-02-30",
            "26-10-28",
            "2026/10/28",
            "2026-1-5",
            "1969-12-31",
        ] {
            assert!(Date::parse(bad).is_none(), "{bad}");
        }
        assert!(Date::parse("2024-02-29").is_some(), "leap year");
        assert!(Date::parse("2025-02-29").is_none());
        assert!(
            Date::parse("2000-02-29").is_some(),
            "a century divisible by 400 leaps"
        );
        assert!(
            Date::parse("2100-02-29").is_none(),
            "a century otherwise doesn't"
        );
        assert!(Date::parse("1900-02-29").is_none());
        assert!(Date::parse("9999-12-31").is_some());
        assert!(Date::parse("10000-01-01").is_none());
        // Arithmetic past the last parseable day doesn't panic.
        assert_eq!(
            Date::parse("9999-12-31").unwrap().plus_days(97).year,
            10_000
        );
    }

    #[test]
    fn days_round_trip() {
        assert_eq!(Date::parse("1970-01-01").unwrap().days(), 0);
        assert_eq!(Date::parse("2000-03-01").unwrap().days(), 11_017);
        for days in [0, 1, 59, 60, 11_016, 11_017, 20_000, 36_524, 2_932_896] {
            assert_eq!(Date::from_days(days).days(), days, "{days}");
        }
        let d = Date::parse("2026-10-28").unwrap();
        assert_eq!(d.plus_days(7).iso(), "2026-11-04");
        assert_eq!(d.plus_days(90 + 7).iso(), "2027-02-02");
        assert_eq!(Date::of_utc(d.starts_utc() + 100), d);
        assert_eq!(Date::of_utc(d.ends_utc()), d.plus_days(1));
    }

    #[test]
    fn the_end_of_the_day_follows_the_organizer_within_reason() {
        let d = Date::parse("2026-10-28").unwrap();
        let utc = d.ends_utc();
        assert_eq!(ends_at(d, None), utc);
        assert_eq!(ends_at(d, Some(utc - 3600)), utc - 3600, "CET");
        assert_eq!(ends_at(d, Some(utc + 8 * 3600)), utc + 8 * 3600, "PST");
        assert_eq!(ends_at(d, Some(utc + 15 * 3600)), utc, "no such timezone");
        assert_eq!(ends_at(d, Some(0)), utc, "nonsense");
        // The edges: UTC+14 and UTC-12 exist; a second past them doesn't.
        let slack = TIMEZONE_SLACK_SECS;
        assert_eq!(ends_at(d, Some(utc - slack)), utc - slack, "UTC+14");
        assert_eq!(ends_at(d, Some(utc + slack)), utc + slack, "UTC-14");
        assert_eq!(ends_at(d, Some(utc - slack - 1)), utc);
        assert_eq!(ends_at(d, Some(utc + slack + 1)), utc);
    }
}
