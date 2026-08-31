//! When a manifest was written.
//!
//! A manifest records both the Unix second — unambiguous, and what a machine
//! compares — and the same instant as an RFC 3339 UTC string, which is what an
//! operator auditing a corpus reads. Formatting is done here rather than by a
//! date dependency: the whole need is one instant, in UTC, in one format.

use std::time::{SystemTime, UNIX_EPOCH};

/// The current instant as `(unix seconds, RFC 3339 UTC)`.
///
/// A clock before the Unix epoch is reported as the epoch itself rather than
/// failing the run: the timestamp is provenance, not a correctness input, and
/// a corpus that was produced should not go unpublished over a misset clock.
pub fn now() -> (u64, String) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    (seconds, rfc3339(seconds))
}

/// `seconds` since the Unix epoch as an RFC 3339 UTC timestamp.
///
/// UTC only, second resolution, always `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(seconds: u64) -> String {
    let days = seconds / 86_400;
    let time_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time_of_day / 3_600,
        (time_of_day % 3_600) / 60,
        time_of_day % 60
    )
}

/// The civil `(year, month, day)` `days` after 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, with the era shifted so the proleptic
/// Gregorian calendar is used throughout.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Shift the epoch to 0000-03-01, so a leap day ends the era.
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_position = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_position + 2) / 5 + 1;
    let month = if month_position < 10 {
        month_position + 3
    } else {
        month_position - 9
    };

    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_instants() {
        let cases = [
            (0, "1970-01-01T00:00:00Z"),
            (1, "1970-01-01T00:00:01Z"),
            (951_825_600, "2000-02-29T12:00:00Z"),
            (1_709_164_799, "2024-02-28T23:59:59Z"),
            (1_756_598_400, "2025-08-31T00:00:00Z"),
            (253_402_300_799, "9999-12-31T23:59:59Z"),
        ];

        for (seconds, expected) in cases {
            assert_eq!(rfc3339(seconds), expected, "{seconds}");
        }
    }

    #[test]
    fn reports_the_current_instant_in_both_forms() {
        let (seconds, formatted) = now();

        assert!(seconds > 1_700_000_000, "a plausible clock: {seconds}");
        assert_eq!(formatted, rfc3339(seconds));
    }
}
