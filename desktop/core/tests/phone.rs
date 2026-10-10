//! The Phone row and the words around the dialog's steps (M60 §3.2, §3.3): the row's status from
//! `mobile.status` and `mobile.devices.list`, never from `configured`, and every sentence the app
//! adds to the daemon's facts.

use fermix_client::mobile::{MobileDevices, MobileStatus};
use fermix_client::phone::{
    countdown, detail, grouped, heading, offers_phone, paired_line, row, seen, spoken,
    unix_seconds, Forgetting, Intent, CHANGE, PAIR, SCAN_LINE, SETUP_ROW, STRINGS,
};
use fermix_client::settings::Sections;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const SEEN: i64 = 1_790_424_280;

fn golden<T: DeserializeOwned>(name: &str, change: impl FnOnce(&mut Value)) -> T {
    let line = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no golden named {name}"));
    let mut result = line["response"]["result"].clone();
    change(&mut result);
    serde_json::from_value(result).unwrap_or_else(|e| panic!("{name} decodes: {e}"))
}

fn status(change: impl FnOnce(&mut Value)) -> Option<Result<MobileStatus, String>> {
    Some(Ok(golden("mobile_status", change)))
}

fn devices() -> Option<Result<MobileDevices, String>> {
    Some(Ok(golden("mobile_devices_list", |_| {})))
}

fn status_of(
    status: Option<Result<MobileStatus, String>>,
    devices: Option<Result<MobileDevices, String>>,
) -> String {
    row(status.as_ref(), devices.as_ref()).status
}

#[test]
fn the_row_says_off_restart_or_could_not_start_before_it_counts_phones() {
    let off = status(|s| s["enabled"] = json!(false));
    assert_eq!(status_of(off, devices()), "Off");
    let owed = status(|s| s["started"] = json!(false));
    assert_eq!(status_of(owed, devices()), "Restart to turn on");
    let refused = status(|s| {
        s["started"] = json!(false);
        s["refused"] = json!(true);
    });
    assert_eq!(status_of(refused, devices()), "Could not start");
    let deaf = status(|s| s["listener"]["status"] = json!("unavailable"));
    assert_eq!(status_of(deaf, devices()), "Could not start");
}

#[test]
fn a_running_channel_names_its_one_phone_or_counts_them() {
    let none = status(|s| s["paired_devices"] = json!(0));
    let row_none = row(none.as_ref(), devices().as_ref());
    assert_eq!(row_none.status, "No phone paired");
    assert_eq!(row_none.opens, Intent::Pair);
    assert_eq!(row_none.action_title(), PAIR);
    assert_eq!(PAIR, "Pair a phone…");

    let one = row(status(|_| {}).as_ref(), devices().as_ref());
    assert_eq!(one.status, "Sam's phone");
    assert_eq!(one.opens, Intent::Phones);
    assert_eq!(one.action_title(), CHANGE);
    assert_eq!(CHANGE, "Change…");

    let three = row(
        status(|s| s["paired_devices"] = json!(3)).as_ref(),
        devices().as_ref(),
    );
    assert_eq!(three.status, "3 phones");
    assert_eq!(three.opens, Intent::Phones);
}

#[test]
fn a_row_not_read_yet_says_checking_and_a_refused_read_says_why() {
    let unread = row(None, None);
    assert_eq!(unread.status, "Checking");
    assert_eq!(unread.opens, Intent::Pair);
    let refused: Option<Result<MobileStatus, String>> = Some(Err("Fermix is not running.".into()));
    assert_eq!(status_of(refused, None), "Fermix is not running.");
    assert_eq!(status_of(status(|_| {}), None), "Checking");
    let unlisted: Option<Result<MobileDevices, String>> = Some(Err("No answer.".into()));
    assert_eq!(status_of(status(|_| {}), unlisted), "No answer.");
}

#[test]
fn the_countdown_rounds_up_so_an_open_window_never_reads_zero() {
    assert_eq!(countdown(120_000), "Expires in 2:00");
    assert_eq!(countdown(83_400), "Expires in 1:24");
    assert_eq!(countdown(59_001), "Expires in 1:00");
    assert_eq!(countdown(1), "Expires in 0:01");
    assert_eq!(countdown(0), "Expires in 0:00");
}

#[test]
fn the_digits_are_grouped_in_threes_and_read_one_by_one_after_the_phones_name() {
    assert_eq!(grouped("481062"), "481 062");
    assert_eq!(spoken("481062", "Sam's phone"), "Sam's phone, 4 8 1 0 6 2");
    assert_eq!(heading("Sam's phone"), "Sam's phone wants to pair");
    assert_eq!(paired_line("Sam's phone"), "Paired with Sam's phone.");
}

#[test]
fn a_timestamp_reads_in_rfc_3339_with_or_without_fractions_and_offsets() {
    assert_eq!(unix_seconds("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(unix_seconds("2026-09-26T12:04:40Z"), Some(SEEN));
    assert_eq!(unix_seconds("2026-09-26T12:04:40.123456Z"), Some(SEEN));
    assert_eq!(unix_seconds("2026-09-26T14:04:40+02:00"), Some(SEEN));
    assert_eq!(unix_seconds("2000-02-29T23:59:59Z"), Some(951_868_799));
    for bad in [
        "",
        "yesterday",
        "2026-09-26",
        "2026-13-26T12:04:40Z",
        "2026-09-26T12:04:40",
        "2026-09-26 12:04:40Z",
    ] {
        assert_eq!(unix_seconds(bad), None, "{bad}");
    }
}

#[test]
fn a_phone_was_seen_in_its_largest_unit_or_not_yet() {
    let at = Some("2026-09-26T12:04:40Z");
    assert_eq!(seen(None, SEEN).as_deref(), Some("Not seen yet"));
    assert_eq!(seen(at, SEEN + 30).as_deref(), Some("Seen just now"));
    assert_eq!(seen(at, SEEN - 30).as_deref(), Some("Seen just now"));
    assert_eq!(seen(at, SEEN + 90).as_deref(), Some("Seen 1 minute ago"));
    assert_eq!(
        seen(at, SEEN + 2 * 3_600).as_deref(),
        Some("Seen 2 hours ago")
    );
    assert_eq!(
        seen(at, SEEN + 3 * 86_400).as_deref(),
        Some("Seen 3 days ago")
    );
    assert_eq!(
        seen(at, SEEN + 45 * 86_400).as_deref(),
        Some("Seen 1 month ago")
    );
    assert_eq!(
        seen(at, SEEN + 800 * 86_400).as_deref(),
        Some("Seen 2 years ago")
    );
    assert_eq!(
        seen(Some("last week"), SEEN),
        None,
        "a time this app cannot read says nothing"
    );
}

#[test]
fn a_phones_detail_is_its_model_and_when_it_was_seen() {
    let at = Some("2026-09-26T12:04:40Z");
    assert_eq!(
        detail("Google Pixel 9 Pro", at, SEEN + 2 * 3_600),
        "Google Pixel 9 Pro · Seen 2 hours ago"
    );
    assert_eq!(
        detail("Google Pixel 9 Pro", None, SEEN),
        "Google Pixel 9 Pro · Not seen yet"
    );
    assert_eq!(
        detail("Google Pixel 9 Pro", Some("soon"), SEEN),
        "Google Pixel 9 Pro"
    );
}

#[test]
fn forgetting_asks_in_the_row_and_only_the_second_press_forgets() {
    let mut forgetting = Forgetting::default();
    assert_eq!(forgetting.confirm(), None, "nothing was asked");
    forgetting.ask("a");
    forgetting.ask("b");
    assert_eq!(
        forgetting.asking(),
        Some("b"),
        "asking one withdraws the other"
    );
    forgetting.withdraw();
    assert_eq!(forgetting.asking(), None);
    assert_eq!(forgetting.confirm(), None);

    forgetting.ask("a");
    assert_eq!(forgetting.confirm().as_deref(), Some("a"));
    assert_eq!(forgetting.forgetting(), Some("a"));
    forgetting.ask("b");
    assert_eq!(
        forgetting.asking(),
        None,
        "nothing is asked while a forget runs"
    );
    forgetting.finished("a", Some("Only the owner can forget a phone.".into()));
    assert_eq!(forgetting.forgetting(), None);
    assert_eq!(
        forgetting.refusal("a"),
        Some("Only the owner can forget a phone.")
    );
    forgetting.ask("a");
    assert_eq!(forgetting.refusal("a"), None, "asking again clears it");
}

/// Android is named where a person is told to reach for their phone, and nowhere else.
#[test]
fn android_is_named_in_exactly_two_strings() {
    let naming: Vec<&str> = STRINGS
        .iter()
        .copied()
        .filter(|s| s.contains("Android"))
        .collect();
    assert_eq!(naming, [SCAN_LINE, SETUP_ROW]);
    assert_eq!(
        SCAN_LINE,
        "On your Android phone, open Fermix and scan this code."
    );
    assert_eq!(SETUP_ROW, "Pair your Android phone");
}

#[test]
fn the_setup_row_is_offered_only_where_the_daemon_publishes_the_phone_section() {
    let sections: Sections = golden("settings_sections", |_| {});
    assert!(offers_phone(&sections.sections));
    let without: Sections = golden("settings_sections", |s| {
        let list = s["sections"].as_array_mut().expect("a list");
        list.retain(|section| section["id"] != "channels.mobile");
    });
    assert!(!offers_phone(&without.sections));
}
