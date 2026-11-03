//! How Chat writes a daemon's time, beside its rows and on its proposal cards alike, as the Mac's
//! `CueTimes` does: the time alone on the reader's day, the day and time within the year, and the
//! date with its year before or after it.

use fermix_client::chat::{written_time, LocalTime};

fn at(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> LocalTime {
    LocalTime {
        year,
        month,
        day,
        hour,
        minute,
    }
}

/// The reader's clock: 2026-10-27, 21:15.
const NOW: LocalTime = LocalTime {
    year: 2026,
    month: 10,
    day: 27,
    hour: 21,
    minute: 15,
};

#[test]
fn a_time_on_the_readers_day_is_the_time_alone() {
    assert_eq!(written_time(at(2026, 10, 27, 9, 5), NOW), "09:05");
    assert_eq!(written_time(at(2026, 10, 27, 23, 59), NOW), "23:59");
}

#[test]
fn another_day_this_year_adds_the_day() {
    assert_eq!(written_time(at(2026, 10, 26, 23, 59), NOW), "Oct 26, 23:59");
    assert_eq!(written_time(at(2026, 9, 27, 14, 5), NOW), "Sep 27, 14:05");
    assert_eq!(written_time(at(2026, 1, 1, 0, 0), NOW), "Jan 1, 00:00");
    assert_eq!(
        written_time(at(2026, 12, 31, 8, 30), NOW),
        "Dec 31, 08:30",
        "a card can wait into a later day"
    );
}

#[test]
fn another_year_adds_the_year() {
    assert_eq!(
        written_time(at(2025, 10, 27, 21, 15), NOW),
        "Oct 27, 2025, 21:15"
    );
    let new_year = at(2027, 1, 1, 0, 10);
    assert_eq!(
        written_time(at(2026, 12, 31, 23, 50), new_year),
        "Dec 31, 2026, 23:50"
    );
    assert_eq!(
        written_time(at(2027, 1, 1, 0, 5), NOW),
        "Jan 1, 2027, 00:05"
    );
}
