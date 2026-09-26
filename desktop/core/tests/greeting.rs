//! Chat's empty state greets the person (plan §3.4): by the time of day on this
//! computer's clock, and by the first name About you holds.

use fermix_client::greeting::{about_you_name, greeting, time_of_day, TimeOfDay};
use fermix_client::settings::SectionRows;
use serde_json::json;

#[test]
fn the_greeting_follows_the_clock() {
    let cases = [
        (0, TimeOfDay::Evening),
        (4, TimeOfDay::Evening),
        (5, TimeOfDay::Morning),
        (11, TimeOfDay::Morning),
        (12, TimeOfDay::Afternoon),
        (17, TimeOfDay::Afternoon),
        (18, TimeOfDay::Evening),
        (23, TimeOfDay::Evening),
    ];
    for (hour, expected) in cases {
        assert_eq!(time_of_day(hour), expected, "at {hour}:00");
    }
}

#[test]
#[should_panic(expected = "an hour of the day")]
fn an_hour_past_23_is_a_bug() {
    time_of_day(24);
}

#[test]
fn the_greeting_names_the_person_when_about_you_has_a_name() {
    assert_eq!(
        greeting(Some(TimeOfDay::Morning), Some("Sujeeth")),
        "Good morning, Sujeeth"
    );
    assert_eq!(
        greeting(Some(TimeOfDay::Afternoon), Some("Ada")),
        "Good afternoon, Ada"
    );
}

#[test]
fn no_name_means_no_comma() {
    assert_eq!(greeting(Some(TimeOfDay::Evening), None), "Good evening");
}

#[test]
fn without_a_clock_the_greeting_is_hello() {
    assert_eq!(greeting(None, Some("Ada")), "Hello, Ada");
    assert_eq!(greeting(None, None), "Hello");
}

fn about_you(value: serde_json::Value) -> SectionRows {
    serde_json::from_value(json!({
        "id": "personalization",
        "title": "About you",
        "rows": [
            {"key": "assistant_name", "kind": "text", "label": "Assistant name", "value": "Fermix"},
            {"key": "user_name", "kind": "text", "label": "Your name", "value": value}
        ]
    }))
    .unwrap()
}

#[test]
fn a_full_name_greets_by_its_first_word() {
    let name = about_you_name(Some(&about_you(json!("Sujeeth Shetty"))));
    assert_eq!(name.as_deref(), Some("Sujeeth"));
    let spaced = about_you_name(Some(&about_you(json!("  ada   lovelace "))));
    assert_eq!(spaced.as_deref(), Some("ada"));
}

#[test]
fn an_empty_blank_or_missing_name_is_no_name() {
    for value in [json!(""), json!("   "), json!(null)] {
        assert_eq!(
            about_you_name(Some(&about_you(value.clone()))),
            None,
            "{value}"
        );
    }
    assert_eq!(about_you_name(None), None);
}
