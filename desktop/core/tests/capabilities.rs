//! Meetings and Computer: the notetaker's sign-in state and the helper's probe,
//! read as the daemon reports them and never inferred (M38 §5.7, §8.6).

use fermix_client::capabilities::{meetbot_state, verdict, ComputerPermissions, MeetbotState};
use fermix_client::management::decode_response;
use fermix_client::model::{DetectResult, DetectRow};
use serde::de::DeserializeOwned;
use serde_json::Value;

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");

fn decode<T: DeserializeOwned>(name: &str) -> T {
    let found = SUCCESS
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|v| v["name"] == name)
        .unwrap();
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    let result = decode_response(&serde_json::to_vec(response).unwrap(), id).unwrap();
    serde_json::from_value(result).unwrap()
}

fn meetbot(present: bool, signed_in: Option<bool>, detail: Option<&str>) -> DetectRow {
    DetectRow {
        target: "meetbot".into(),
        present,
        detail: detail.map(str::to_owned),
        signed_in,
    }
}

#[test]
fn the_notetaker_sign_in_is_read_from_its_detection_row() {
    let detected: DetectResult = decode("setup_detect");
    let row = detected.results.iter().find(|r| r.target == "meetbot");
    assert_eq!(
        meetbot_state(row),
        MeetbotState::SignedIn("Signed in to Google".into())
    );
}

#[test]
fn only_an_explicit_false_is_signed_out_and_nothing_else_is_guessed() {
    let out = meetbot(true, Some(false), Some("Not signed in to Google"));
    assert_eq!(
        meetbot_state(Some(&out)),
        MeetbotState::SignedOut(Some("Not signed in to Google".into()))
    );
    assert_eq!(
        meetbot_state(Some(&meetbot(false, None, None))),
        MeetbotState::Absent
    );
    assert_eq!(
        meetbot_state(Some(&meetbot(true, None, None))),
        MeetbotState::Unanswered
    );
    assert_eq!(meetbot_state(None), MeetbotState::Unanswered);
}

#[test]
fn a_probe_verdict_says_available_not_available_or_not_checked() {
    let probe: ComputerPermissions = decode("computer_use_permissions_get");
    assert!(probe.installed);
    assert_eq!(
        verdict(probe.screen_capture, probe.probed_at.as_deref()),
        "Available"
    );
    assert_eq!(
        verdict(probe.input_control, probe.probed_at.as_deref()),
        "Not available"
    );
    assert_eq!(verdict(true, None), "Not checked yet");
}

#[test]
fn a_running_job_says_what_it_is_doing_in_words() {
    use fermix_client::capabilities::phase_words;
    assert_eq!(
        phase_words(Some("sidecar_downloading")),
        "Downloading the helper…"
    );
    assert_eq!(
        phase_words(Some("downloading")),
        "Downloading the notetaker's browser…"
    );
    assert_eq!(
        phase_words(Some("awaiting_signin")),
        "Finish signing in to Google in the window that opened."
    );
    assert_eq!(phase_words(Some("brand_new_phase")), "Working…");
    assert_eq!(phase_words(None), "Working…");
}
