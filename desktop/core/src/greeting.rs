//! Chat's empty state greets the person (plan §3.4): by the time of day on this
//! computer's clock, and by the first name About you holds. Sentence case, no
//! exclamation marks; the words are the product plan's.

use crate::settings::SectionRows;

/// The settings section About you is written to, and its name row.
pub const ABOUT_YOU: &str = "personalization";
const NAME_KEY: &str = "user_name";

/// The quieter line under the greeting.
pub const QUIET_LINE: &str = "What is on your mind?";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeOfDay {
    Morning,
    Afternoon,
    Evening,
}

/// 05:00 to 11:59 is morning, 12:00 to 17:59 afternoon, and the rest evening.
pub fn time_of_day(hour: u32) -> TimeOfDay {
    assert!(hour < 24, "an hour of the day is 0 to 23, got {hour}");
    match hour {
        5..=11 => TimeOfDay::Morning,
        12..=17 => TimeOfDay::Afternoon,
        _ => TimeOfDay::Evening,
    }
}

/// "Good morning, Ada", or "Good morning" with no name; "Hello" when the clock
/// cannot say what time it is.
pub fn greeting(time: Option<TimeOfDay>, first_name: Option<&str>) -> String {
    let words = match time {
        Some(TimeOfDay::Morning) => "Good morning",
        Some(TimeOfDay::Afternoon) => "Good afternoon",
        Some(TimeOfDay::Evening) => "Good evening",
        None => "Hello",
    };
    match first_name {
        Some(name) => format!("{words}, {name}"),
        None => words.to_owned(),
    }
}

/// The first word of About you's "Your name", when the section has been read
/// and the row holds one.
pub fn about_you_name(section: Option<&SectionRows>) -> Option<String> {
    let row = section?.rows.iter().find(|row| row.key == NAME_KEY)?;
    let name = row.value.as_str()?.split_whitespace().next()?;
    Some(name.to_owned())
}
