//! A minimal proleptic-Gregorian calendar date, with no external dependency.
//!
//! JAMA only ever needs calendar dates (no times, no timezones), so a small
//! self-contained implementation keeps the dependency tree — and the binary —
//! smaller than pulling in `chrono` or `time`.

use std::fmt;

/// A calendar date (year-month-day), stored as days since the Unix epoch
/// (1970-01-01) for cheap arithmetic and ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    days_since_epoch: i64,
}

impl Date {
    /// Construct a date from a (year, month, day) triple, validating that it
    /// is a real calendar date.
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Self, String> {
        if !(1..=12).contains(&month) {
            return Err(format!(
                "invalid month {month} in date {year:04}-{month:02}-{day:02}"
            ));
        }
        let dim = days_in_month(year, month);
        if day == 0 || day > dim {
            return Err(format!(
                "invalid day {day} in date {year:04}-{month:02}-{day:02}"
            ));
        }
        Ok(Self {
            days_since_epoch: days_from_civil(year, month, day),
        })
    }

    pub fn from_days_since_epoch(days: i64) -> Self {
        Self {
            days_since_epoch: days,
        }
    }

    pub fn days_since_epoch(&self) -> i64 {
        self.days_since_epoch
    }

    /// The current UTC-ish calendar date, derived from the system clock.
    /// Good enough for a local single-user CLI where "today" just needs to
    /// be roughly right; no timezone database is bundled.
    pub fn today() -> Self {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Self::from_days_since_epoch(secs.div_euclid(86_400))
    }

    pub fn year_month_day(&self) -> (i32, u32, u32) {
        civil_from_days(self.days_since_epoch)
    }

    pub fn year(&self) -> i32 {
        self.year_month_day().0
    }

    pub fn month(&self) -> u32 {
        self.year_month_day().1
    }

    pub fn day(&self) -> u32 {
        self.year_month_day().2
    }

    /// Parse `YYYY-MM-DD`, or the convenience keywords `today` / `yesterday`.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim() {
            "today" => Ok(Self::today()),
            "yesterday" => Ok(Self::today().add_days(-1)),
            other => Self::parse_iso(other),
        }
    }

    pub fn parse_iso(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 3 {
            return Err(format!("expected date in YYYY-MM-DD form, got {s:?}"));
        }
        let year: i32 = parts[0].parse().map_err(|_| format!("bad year in {s:?}"))?;
        let month: u32 = parts[1]
            .parse()
            .map_err(|_| format!("bad month in {s:?}"))?;
        let day: u32 = parts[2].parse().map_err(|_| format!("bad day in {s:?}"))?;
        Self::from_ymd(year, month, day)
    }

    /// Parse a date against a strptime-ish format string, supporting the
    /// small set of directives banks' CSV exports actually use:
    /// `%Y` (4-digit year), `%y` (2-digit year, 20xx), `%m`, `%d`, and the
    /// literal separators between them (`/`, `-`, `.`, space).
    pub fn parse_with_format(s: &str, format: &str) -> Result<Self, String> {
        let mut year: Option<i32> = None;
        let mut month: Option<u32> = None;
        let mut day: Option<u32> = None;

        let mut fmt_chars = format.chars().peekable();
        let mut input = s.trim();

        while let Some(fc) = fmt_chars.next() {
            if fc == '%' {
                let spec = fmt_chars.next().ok_or("dangling % in date format")?;
                let (digits, rest) = take_digits(input, spec_len(spec));
                if digits.is_empty() {
                    return Err(format!(
                        "could not read {spec} field from {s:?} using {format:?}"
                    ));
                }
                let value: i32 = digits
                    .parse()
                    .map_err(|_| format!("bad numeric field in {s:?}"))?;
                match spec {
                    'Y' => year = Some(value),
                    'y' => year = Some(2000 + value),
                    'm' => month = Some(value as u32),
                    'd' => day = Some(value as u32),
                    other => return Err(format!("unsupported date format directive %{other}")),
                }
                input = rest;
            } else {
                input = input.strip_prefix(fc).ok_or_else(|| {
                    format!("expected literal {fc:?} while parsing {s:?} with format {format:?}")
                })?;
            }
        }

        let (year, month, day) = (
            year.ok_or("date format has no %Y/%y")?,
            month.ok_or("date format has no %m")?,
            day.ok_or("date format has no %d")?,
        );
        Self::from_ymd(year, month, day)
    }

    pub fn add_days(&self, n: i64) -> Self {
        Self {
            days_since_epoch: self.days_since_epoch + n,
        }
    }

    /// First day of the month containing this date.
    pub fn month_start(&self) -> Self {
        let (y, m, _) = self.year_month_day();
        Self::from_ymd(y, m, 1).expect("valid")
    }

    /// First day of the following month.
    pub fn next_month_start(&self) -> Self {
        let (y, m, _) = self.year_month_day();
        if m == 12 {
            Self::from_ymd(y + 1, 1, 1).expect("valid")
        } else {
            Self::from_ymd(y, m + 1, 1).expect("valid")
        }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = self.year_month_day();
        write!(f, "{y:04}-{m:02}-{d:02}")
    }
}

fn spec_len(spec: char) -> usize {
    match spec {
        'Y' => 4,
        'y' | 'm' | 'd' => 2,
        _ => 4,
    }
}

/// Take up to `max` leading ASCII digits from `s`, returning (digits, rest).
/// Accepts fewer than `max` digits (e.g. single-digit day "5/1/2024").
fn take_digits(s: &str, max: usize) -> (&str, &str) {
    let mut count = 0;
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() && count < max {
            count += 1;
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    (&s[..end], &s[end..])
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Howard Hinnant's `days_from_civil` algorithm: maps a (possibly negative)
/// proleptic Gregorian calendar date to a day count relative to 1970-01-01.
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m as i64 + 9) % 12; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// The inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_known_dates() {
        for (y, m, d) in [
            (1970, 1, 1),
            (2024, 2, 29),
            (2000, 1, 1),
            (1999, 12, 31),
            (2026, 9, 21),
        ] {
            let date = Date::from_ymd(y, m, d).unwrap();
            assert_eq!(
                date.year_month_day(),
                (y, m, d),
                "roundtrip failed for {y}-{m}-{d}"
            );
        }
    }

    #[test]
    fn rejects_invalid_dates() {
        assert!(Date::from_ymd(2023, 2, 29).is_err());
        assert!(Date::from_ymd(2024, 13, 1).is_err());
        assert!(Date::from_ymd(2024, 4, 31).is_err());
    }

    #[test]
    fn parses_iso() {
        let date = Date::parse("2026-09-01").unwrap();
        assert_eq!(date.to_string(), "2026-09-01");
    }

    #[test]
    fn parses_custom_format() {
        let date = Date::parse_with_format("21/09/2026", "%d/%m/%Y").unwrap();
        assert_eq!(date.to_string(), "2026-09-21");
    }

    #[test]
    fn ordering_is_chronological() {
        let a = Date::from_ymd(2026, 1, 1).unwrap();
        let b = Date::from_ymd(2026, 1, 2).unwrap();
        assert!(a < b);
    }
}
