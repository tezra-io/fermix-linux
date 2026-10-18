//! Reach from anywhere (M62): the line under its switch, from `mobile.status.remote`, and how the
//! Phones step follows it. The sentences are macOS's, word for word.

use fermix_client::mobile::MobileStatus;
use fermix_client::reach::{
    line, moves, Follow, Kind, Line, BACKOFF_MS, DORMANT, HELPER_MISSING, NOT_CONNECTED, POLLS,
    READY, STARTING,
};
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");

/// The golden `mobile.status`, its remote reach changed by `change`.
fn status(change: impl FnOnce(&mut Value)) -> MobileStatus {
    let golden = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == "mobile_status")
        .expect("a mobile_status golden");
    let mut result = golden["response"]["result"].clone();
    change(&mut result["remote"]);
    serde_json::from_value(result).expect("mobile.status decodes")
}

fn line_of(change: impl FnOnce(&mut Value)) -> Option<Line> {
    line(status(change).remote.as_ref())
}

fn note(text: &str, kind: Kind) -> Option<Line> {
    Some(Line {
        text: text.to_owned(),
        kind,
    })
}

#[test]
fn a_ready_rail_says_whether_this_computer_is_connected_naming_no_relay() {
    assert_eq!(line_of(|_| {}), note(READY, Kind::Note));
    assert_eq!(
        line_of(|r| r["connected"] = json!(false)),
        note(NOT_CONNECTED, Kind::Note)
    );
    for text in [READY, NOT_CONNECTED] {
        assert!(!text.contains("relay.fermix.ai"), "{text}");
    }
}

#[test]
fn each_other_state_has_its_line_and_a_failure_is_the_daemons_sentence() {
    assert_eq!(
        line_of(|r| r["state"] = json!("dormant")),
        note(DORMANT, Kind::Note)
    );
    assert_eq!(
        line_of(|r| r["state"] = json!("helper_missing")),
        note(HELPER_MISSING, Kind::Note)
    );
    assert_eq!(
        line_of(|r| r["state"] = json!("starting")),
        note(STARTING, Kind::Progress)
    );
    let failed = line_of(|r| {
        r["state"] = json!("failed");
        r["failure"] = json!("The relay refused this key.");
    });
    assert_eq!(failed, note("The relay refused this key.", Kind::Failure));
    assert_eq!(line_of(|r| r["state"] = json!("failed")), None);
}

#[test]
fn off_an_unknown_state_and_an_engine_without_the_rail_say_nothing() {
    assert_eq!(line_of(|r| r["state"] = json!("off")), None);
    assert_eq!(line_of(|r| r["state"] = json!("teleporting")), None);
    let older = status(|remote| *remote = Value::Null);
    assert_eq!(older.remote, None, "an engine older than the rail decodes");
    assert_eq!(line(older.remote.as_ref()), None);
}

#[test]
fn only_a_rail_that_can_move_by_itself_is_followed() {
    for state in ["starting", "ready", "failed", "teleporting"] {
        let remote = status(|r| r["state"] = json!(state)).remote;
        assert!(moves(remote.as_ref()), "{state}");
    }
    for state in ["off", "dormant", "helper_missing"] {
        let remote = status(|r| r["state"] = json!(state)).remote;
        assert!(!moves(remote.as_ref()), "{state}");
    }
    assert!(!moves(None));
}

#[test]
fn a_follow_waits_a_second_doubles_per_refusal_up_to_thirty_and_stops_at_its_last_read() {
    let mut follow = Follow::default();
    assert_eq!(follow.wait_ms(), 1_000);
    assert!(
        follow.refused(),
        "the first refusal of a run is worth a log line"
    );
    assert!(!follow.refused());
    assert_eq!(follow.wait_ms(), 4_000);
    for _ in 0..10 {
        follow.refused();
    }
    assert_eq!(follow.wait_ms(), BACKOFF_MS);
    assert_eq!(
        follow.answered(),
        Some(12),
        "the answer ends a run of twelve"
    );
    assert_eq!(follow.wait_ms(), 1_000);
    assert_eq!(follow.answered(), None);
    // Twelve refusals and two answers so far: every one is a read.
    for _ in 14..POLLS {
        assert!(!follow.exhausted());
        follow.answered();
    }
    assert!(follow.exhausted(), "{POLLS} reads in all");
}
