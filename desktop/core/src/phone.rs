//! The Phone row and the words around the Phone dialog's steps (M60 §3.2, §3.3). The row's status
//! comes from `mobile.status` and `mobile.devices.list`, never from `configured`: the phone channel
//! is always configured, so the generic channel word said Connected with no phone paired. The
//! sentences are macOS's, word for word.

use crate::mobile::{ListenerStatus, MobileDevices, MobileStatus};
use crate::settings::Section;

/// The channel's name in `setup.state.get`.
pub const CHANNEL: &str = "mobile";
pub const SECTION: &str = "channels.mobile";
pub const SWITCH_KEY: &str = "mobile_enabled";
/// The cadence the contract recommends for reading a pairing window, and the one the dialog polls at.
pub const POLL_MS: u64 = 1_000;

pub const STATUS_OFF: &str = "Off";
pub const STATUS_RESTART: &str = "Restart to turn on";
pub const STATUS_COULD_NOT_START: &str = "Could not start";
pub const STATUS_NO_PHONE: &str = "No phone paired";
pub const STATUS_CHECKING: &str = "Checking";
pub const PAIR: &str = "Pair a phone…";
pub const CHANGE: &str = "Change…";
pub const TURN_ON_AND_RESTART: &str = "Turn on and restart Fermix";
pub const RESTART: &str = "Restart Fermix";
pub const RESTARTING: &str = "Restarting Fermix";
pub const SCAN_LINE: &str = "On your Android phone, open Fermix and scan this code.";
pub const CODE_LABEL: &str = "Pairing code for your phone";
pub const CANT_SCAN: &str = "Can't scan the code?";
pub const LINK_LABEL: &str = "Pairing link";
pub const COPY_LINK: &str = "Copy link";
pub const COMPARE_LINE: &str = "Approve only if the phone shows the same six digits.";
pub const DENY: &str = "Deny";
pub const APPROVE: &str = "Approve";
pub const DONE: &str = "Done";
pub const CANCEL: &str = "Cancel";
pub const ENDED_EXPIRED: &str = "The code expired. Pairing codes last two minutes.";
pub const ENDED_DENIED: &str = "You denied this phone.";
pub const ENDED_CANCELLED: &str = "Pairing was cancelled.";
pub const ENDED_ELSEWHERE: &str = "A pairing code is already open somewhere else.";
pub const ENDED_UNREADABLE: &str = "Fermix answered a pairing code this app cannot show.";
pub const PAIR_AGAIN: &str = "Pair again";
pub const START_OVER: &str = "Start over";
pub const NOT_SEEN: &str = "Not seen yet";
pub const SEEN_JUST_NOW: &str = "Seen just now";
pub const FORGET: &str = "Forget…";
pub const FORGET_CONFIRM: &str = "Forget this phone";
pub const PAIR_ANOTHER: &str = "Pair another phone";
pub const SETUP_ROW: &str = "Pair your Android phone";

/// Every fixed string the phone surfaces show, so a test can hold them to where Android is named.
pub const STRINGS: [&str; 33] = [
    STATUS_OFF,
    STATUS_RESTART,
    STATUS_COULD_NOT_START,
    STATUS_NO_PHONE,
    STATUS_CHECKING,
    PAIR,
    CHANGE,
    TURN_ON_AND_RESTART,
    RESTART,
    RESTARTING,
    SCAN_LINE,
    CODE_LABEL,
    CANT_SCAN,
    LINK_LABEL,
    COPY_LINK,
    COMPARE_LINE,
    DENY,
    APPROVE,
    DONE,
    CANCEL,
    ENDED_EXPIRED,
    ENDED_DENIED,
    ENDED_CANCELLED,
    ENDED_ELSEWHERE,
    ENDED_UNREADABLE,
    PAIR_AGAIN,
    START_OVER,
    NOT_SEEN,
    SEEN_JUST_NOW,
    FORGET,
    FORGET_CONFIRM,
    PAIR_ANOTHER,
    SETUP_ROW,
];

/// What the Phone dialog opens for: pairing a phone, or the phones already paired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Pair,
    Phones,
}

/// The Phone row's status and the one button it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneRow {
    pub status: String,
    pub opens: Intent,
}

impl PhoneRow {
    pub fn action_title(&self) -> &'static str {
        match self.opens {
            Intent::Pair => PAIR,
            Intent::Phones => CHANGE,
        }
    }

    fn pair(status: &str) -> PhoneRow {
        PhoneRow {
            status: status.to_owned(),
            opens: Intent::Pair,
        }
    }
}

/// The row from the two reads: None where a read has not answered yet, the daemon's sentence
/// where it refused. The checks run in §3.2's order.
pub fn row(
    status: Option<&Result<MobileStatus, String>>,
    devices: Option<&Result<MobileDevices, String>>,
) -> PhoneRow {
    let status = match status {
        None => return PhoneRow::pair(STATUS_CHECKING),
        Some(Err(sentence)) => return PhoneRow::pair(sentence),
        Some(Ok(status)) => status,
    };
    if !status.enabled {
        return PhoneRow::pair(STATUS_OFF);
    }
    if !status.started && !status.refused {
        return PhoneRow::pair(STATUS_RESTART);
    }
    if status.refused || status.listener.status == ListenerStatus::Unavailable {
        return PhoneRow::pair(STATUS_COULD_NOT_START);
    }
    let words = match status.paired_devices {
        ..=0 => return PhoneRow::pair(STATUS_NO_PHONE),
        1 => phone_name(devices),
        n => format!("{n} phones"),
    };
    PhoneRow {
        status: words,
        opens: Intent::Phones,
    }
}

/// The one paired phone's name, from the list that names it.
fn phone_name(devices: Option<&Result<MobileDevices, String>>) -> String {
    match devices {
        Some(Ok(list)) => list
            .devices
            .first()
            .map_or_else(|| STATUS_CHECKING.to_owned(), |d| d.name.clone()),
        Some(Err(sentence)) => sentence.clone(),
        None => STATUS_CHECKING.to_owned(),
    }
}

/// Whether the daemon publishes the phone channel, which is when the last setup screen offers it.
pub fn offers_phone(sections: &[Section]) -> bool {
    sections.iter().any(|s| s.id == SECTION)
}

/// "Expires in 1:45": the daemon's `ttl_ms`, rounded up to the second so a window that is still
/// open never reads 0:00.
pub fn countdown(ttl_ms: i64) -> String {
    let seconds = (ttl_ms.max(0) + 999) / 1000;
    format!("Expires in {}:{:02}", seconds / 60, seconds % 60)
}

/// Whether the countdown is in its last ten seconds, which is when it is announced again (§7).
pub fn final_countdown(ttl_ms: i64) -> bool {
    ttl_ms <= 10_000
}

/// The six digits in threes, as the phone draws them: `481 062`.
pub fn grouped(digits: &str) -> String {
    let half = digits.len() / 2;
    match (digits.get(..half), digits.get(half..)) {
        (Some(first), Some(second)) => format!("{first} {second}"),
        _ => digits.to_owned(),
    }
}

/// The digits one by one after the phone's name, which is how a screen reader reads them (§7).
pub fn spoken(digits: &str, name: &str) -> String {
    let one_by_one: Vec<String> = digits.chars().map(String::from).collect();
    format!("{name}, {}", one_by_one.join(" "))
}

/// Compare's heading.
pub fn heading(name: &str) -> String {
    format!("{name} wants to pair")
}

pub fn paired_line(name: &str) -> String {
    format!("Paired with {name}.")
}

/// When a paired phone was last seen, in its largest unit. None for a time written in a shape
/// this app cannot read, so the row says nothing rather than something untrue.
pub fn seen(last_seen: Option<&str>, now: i64) -> Option<String> {
    let Some(last_seen) = last_seen else {
        return Some(NOT_SEEN.to_owned());
    };
    let ago = now - unix_seconds(last_seen)?;
    let units = [
        (365 * 86_400, "year"),
        (30 * 86_400, "month"),
        (86_400, "day"),
        (3_600, "hour"),
        (60, "minute"),
    ];
    let Some((size, unit)) = units.into_iter().find(|(size, _)| ago >= *size) else {
        return Some(SEEN_JUST_NOW.to_owned());
    };
    let count = ago / size;
    let plural = if count == 1 { "" } else { "s" };
    Some(format!("Seen {count} {unit}{plural} ago"))
}

/// Under a paired phone's name: its model, and when it was last seen where the daemon says so.
pub fn detail(model: &str, last_seen: Option<&str>, now: i64) -> String {
    match seen(last_seen, now) {
        Some(seen) => format!("{model} · {seen}"),
        None => model.to_owned(),
    }
}

/// An RFC 3339 timestamp as Unix seconds: `2026-09-26T12:04:40Z`, with or without fractional
/// seconds, in UTC or at an offset. None for anything else.
pub fn unix_seconds(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() >= 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':';
    if !shaped {
        return None;
    }
    let field = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = text.get(range)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (year, month, day) = (field(0..4)?, field(5..7)?, field(8..10)?);
    let (hour, minute, second) = (field(11..13)?, field(14..16)?, field(17..19)?);
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 61;
    if !valid {
        return None;
    }
    let local = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(local - offset_seconds(text.get(19..)?)?)
}

/// What follows the seconds: optional fractions, then `Z` or `±hh:mm`.
fn offset_seconds(rest: &str) -> Option<i64> {
    let rest = match rest.strip_prefix('.') {
        Some(fraction) => fraction.trim_start_matches(|c: char| c.is_ascii_digit()),
        None => rest,
    };
    if rest == "Z" || rest == "z" {
        return Some(0);
    }
    let sign = match rest.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (hours, minutes) = rest.get(1..)?.split_once(':')?;
    let digits = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(hours) || !digits(minutes) {
        return None;
    }
    Some(sign * (hours.parse::<i64>().ok()? * 3_600 + minutes.parse::<i64>().ok()? * 60))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Forget, asked in the phone's own row: its button becomes Forget this phone and Cancel, with
/// no confirmation over the dialog. Only the second press forgets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forgetting {
    asking: Option<String>,
    forgetting: Option<String>,
    /// The daemon's sentence under a row whose forget it refused, by device id.
    refusals: Vec<(String, String)>,
}

impl Forgetting {
    /// The first press: the row asks. Asking about one phone withdraws the question from another.
    pub fn ask(&mut self, device: &str) {
        assert!(!device.is_empty(), "a phone is forgotten by its id");
        if self.forgetting.is_some() {
            return;
        }
        self.asking = Some(device.to_owned());
        self.refusals.retain(|(id, _)| id != device);
    }

    pub fn withdraw(&mut self) {
        self.asking = None;
    }

    /// The second press: the phone the row asked about, which is then the one being forgotten.
    pub fn confirm(&mut self) -> Option<String> {
        if self.forgetting.is_some() {
            return None;
        }
        let device = self.asking.take()?;
        self.forgetting = Some(device.clone());
        Some(device)
    }

    /// The daemon answered. A refusal stays under the row it refused.
    pub fn finished(&mut self, device: &str, refusal: Option<String>) {
        if self.forgetting.as_deref() != Some(device) {
            return;
        }
        self.forgetting = None;
        if let Some(sentence) = refusal {
            self.refusals.push((device.to_owned(), sentence));
        }
    }

    pub fn asking(&self) -> Option<&str> {
        self.asking.as_deref()
    }

    pub fn forgetting(&self) -> Option<&str> {
        self.forgetting.as_deref()
    }

    pub fn refusal(&self, device: &str) -> Option<&str> {
        self.refusals
            .iter()
            .find(|(id, _)| id == device)
            .map(|(_, sentence)| sentence.as_str())
    }
}
