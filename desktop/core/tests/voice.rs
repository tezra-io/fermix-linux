//! What the Voice page says before a call can begin (spec_voice §4.3): one row,
//! the first that applies, from what the daemon reports and what the last
//! connection attempt found.

use fermix_client::mascot::Expression;
use fermix_client::model::SetupState;
use fermix_client::overview::RealtimeFacts;
use fermix_client::realtime::client::ConnectError;
use fermix_client::realtime::protocol::ServerError;
use fermix_client::realtime::session::{Input, Mode, Palette, Session};
use fermix_client::voice::{
    call_status, expression, voice_gate, GateAction, Microphone, Reach, Source, VoiceFacts,
};
use serde_json::json;

fn state(failures: serde_json::Value, reasons: serde_json::Value) -> SetupState {
    let required = reasons.as_array().is_some_and(|r| !r.is_empty());
    serde_json::from_value(json!({
        "readiness": {"status": "ready", "failures": failures},
        "restart": {"required": required, "reasons": reasons},
        "providers": [],
        "channels": [],
        "features": {"voice": true, "voice_notes": false, "meetings": false, "computer_use": false},
        "coexistence": {"config_state": "clear"}
    }))
    .unwrap()
}

fn realtime(enabled: bool, status: &str) -> RealtimeFacts {
    serde_json::from_value(json!({"enabled": enabled, "status": status, "socket_alive": enabled}))
        .unwrap()
}

fn gate(
    state: Option<&SetupState>,
    realtime: Option<&RealtimeFacts>,
    reach: Reach,
) -> (String, Option<GateAction>, bool) {
    gate_with(state, realtime, reach, &Microphone::Unknown)
}

fn gate_with(
    state: Option<&SetupState>,
    realtime: Option<&RealtimeFacts>,
    reach: Reach,
    microphone: &Microphone,
) -> (String, Option<GateAction>, bool) {
    let g = voice_gate(&VoiceFacts {
        state,
        realtime,
        reach,
        microphone,
    });
    (g.sentence, g.action, g.ready)
}

#[test]
fn a_daemon_that_does_not_answer_comes_first() {
    let (sentence, action, ready) = gate(None, None, Reach::Untried);
    assert_eq!(sentence, "Fermix is not running.");
    assert_eq!(action, Some(GateAction::StartFermix));
    assert!(!ready);
}

#[test]
fn voice_off_offers_to_turn_it_on() {
    let s = state(json!([]), json!([]));
    let (sentence, action, _) = gate(Some(&s), Some(&realtime(false, "disabled")), Reach::Untried);
    assert_eq!(
        sentence,
        "Voice is off. Turn it on to talk to Fermix from this app."
    );
    assert_eq!(action, Some(GateAction::TurnOn));
}

#[test]
fn a_voice_change_waiting_for_restart_outranks_voice_off() {
    let reasons = json!([{"section": "realtime", "sentence": "Voice settings changed since Fermix started."}]);
    let s = state(json!([]), reasons);
    let (sentence, action, _) = gate(Some(&s), Some(&realtime(false, "disabled")), Reach::Untried);
    assert_eq!(sentence, "Voice settings changed since Fermix started.");
    assert_eq!(action, Some(GateAction::Restart));
}

#[test]
fn a_missing_key_says_a_sign_in_does_not_cover_voice() {
    let failure = json!([{"component": "realtime:openai", "gating": false, "pane": "voice", "detail_key": "realtime:openai"}]);
    let s = state(failure, json!([]));
    let (sentence, action, _) = gate(
        Some(&s),
        Some(&realtime(true, "setup_required")),
        Reach::Untried,
    );
    assert!(
        sentence.contains("sign-in does not authorize"),
        "{sentence}"
    );
    assert_eq!(action, Some(GateAction::AddKey));
}

#[test]
fn a_provider_change_waits_for_restart_once_voice_is_on() {
    let reasons = json!([{"section": "providers", "sentence": "Provider settings changed since Fermix started."}]);
    let s = state(json!([]), reasons);
    let (sentence, action, _) = gate(Some(&s), Some(&realtime(true, "ready")), Reach::Untried);
    assert_eq!(sentence, "Provider settings changed since Fermix started.");
    assert_eq!(action, Some(GateAction::Restart));
}

#[test]
fn voice_on_without_its_socket_suggests_a_restart() {
    let s = state(json!([]), json!([]));
    let expected = "Voice is on, but Fermix has not opened its voice connection. Restarting Fermix usually fixes this.";
    let (sentence, action, _) = gate(Some(&s), Some(&realtime(true, "degraded")), Reach::Untried);
    assert_eq!(sentence, expected);
    assert_eq!(action, Some(GateAction::Restart));
    let (sentence, _, _) = gate(Some(&s), Some(&realtime(true, "ready")), Reach::NoSocket);
    assert_eq!(sentence, expected);
}

#[test]
fn a_version_refusal_names_the_side_to_update() {
    let s = state(json!([]), json!([]));
    let on = realtime(true, "ready");
    let (old, action, _) = gate(Some(&s), Some(&on), Reach::ClientTooOld);
    assert_eq!(old, "Update this app to talk to this version of Fermix.");
    assert_eq!(action, None);
    let (new, _, _) = gate(Some(&s), Some(&on), Reach::ClientTooNew);
    assert_eq!(new, "Update Fermix to talk to this app.");
    let (busy, _, _) = gate(Some(&s), Some(&on), Reach::Busy);
    assert_eq!(
        busy,
        "Four other voice clients are already connected to Fermix."
    );
}

#[test]
fn everything_in_place_is_ready() {
    let s = state(json!([]), json!([]));
    let (sentence, action, ready) = gate(Some(&s), Some(&realtime(true, "ready")), Reach::Untried);
    assert_eq!(sentence, "Ready");
    assert_eq!(action, None);
    assert!(ready);
}

#[test]
fn without_an_overview_the_setup_state_says_whether_voice_is_on() {
    let mut s = state(json!([]), json!([]));
    s.features.voice = false;
    let (_, action, _) = gate(Some(&s), None, Reach::Untried);
    assert_eq!(action, Some(GateAction::TurnOn));
}

fn refusal(frame: serde_json::Value) -> ConnectError {
    let error: ServerError = serde_json::from_value(frame).unwrap();
    ConnectError::Rejected(Box::new(error))
}

#[test]
fn a_refused_connection_names_what_stands_in_the_way() {
    assert_eq!(
        Reach::from_connect(&ConnectError::NotFound),
        Reach::NoSocket
    );
    assert_eq!(Reach::from_connect(&ConnectError::Refused), Reach::NoSocket);
    let old =
        refusal(json!({"reason": "unsupported_protocol_version", "direction": "client_too_old"}));
    assert_eq!(Reach::from_connect(&old), Reach::ClientTooOld);
    let new =
        refusal(json!({"reason": "unsupported_protocol_version", "direction": "client_too_new"}));
    assert_eq!(Reach::from_connect(&new), Reach::ClientTooNew);
    let busy = refusal(json!({"reason": "max_clients_reached"}));
    assert_eq!(Reach::from_connect(&busy), Reach::Busy);
}

#[test]
fn a_connection_that_merely_failed_leaves_the_gate_alone() {
    assert_eq!(Reach::from_connect(&ConnectError::Timeout), Reach::Untried);
    let io = ConnectError::Io(std::io::Error::other("broken pipe"));
    assert_eq!(Reach::from_connect(&io), Reach::Untried);
    let other = refusal(json!({"reason": "provider_refused"}));
    assert_eq!(Reach::from_connect(&other), Reach::Untried);
}

#[test]
fn the_mascot_follows_what_voice_is_doing() {
    assert_eq!(expression(Mode::Listening), Expression::Listening);
    assert_eq!(expression(Mode::Speaking), Expression::Speaking);
    for mode in [
        Mode::Thinking,
        Mode::ToolUse,
        Mode::Connecting,
        Mode::Reconnecting,
    ] {
        assert_eq!(expression(mode), Expression::Thinking, "{mode:?}");
    }
    for mode in [Mode::Offline, Mode::Idle, Mode::Muted, Mode::Error] {
        assert_eq!(expression(mode), Expression::Idle, "{mode:?}");
    }
}

#[test]
fn before_a_call_the_word_rests_on_ready_unless_the_gate_has_the_reason() {
    let session = Session::new();
    let ready = call_status(&session, true);
    assert_eq!(
        (ready.label.as_str(), ready.palette),
        ("Ready", Palette::Secondary)
    );
    let blocked = call_status(&session, false);
    assert_eq!(
        (blocked.label.as_str(), blocked.palette),
        ("Unavailable", Palette::Faint)
    );
}

#[test]
fn a_refusal_the_gate_does_not_cover_shows_as_the_sessions_sentence() {
    let mut session = Session::new();
    session.apply(Input::Begin);
    session.apply(Input::ConnectFailed(refusal(
        json!({"reason": "provider_refused"}),
    )));
    let status = call_status(&session, true);
    assert_eq!(status.palette, Palette::Error);
    assert_eq!(status.label, "OpenAI refused the voice call.");
}

fn source(name: &str, monitor: bool, default: bool) -> Source {
    Source {
        name: name.into(),
        monitor,
        default,
    }
}

#[test]
fn a_call_records_from_the_sound_servers_default_input() {
    let sources = [
        source("Monitor of Speakers", true, false),
        source("Built-in Audio", false, false),
        source("fifine Microphone", false, true),
    ];
    assert_eq!(
        Microphone::from_sources(&sources),
        Microphone::Named("fifine Microphone".into())
    );
}

#[test]
fn copies_of_outputs_are_not_microphones() {
    assert_eq!(Microphone::from_sources(&[]), Microphone::Missing);
    let monitors = [
        source("Monitor of Speakers", true, true),
        source("Monitor of HDMI", true, false),
    ];
    assert_eq!(Microphone::from_sources(&monitors), Microphone::Missing);
}

#[test]
fn a_default_copy_of_an_output_is_named_because_a_call_records_it() {
    let sources = [
        source("Built-in Audio", false, false),
        source("Monitor of Speakers", true, true),
    ];
    assert_eq!(
        Microphone::from_sources(&sources),
        Microphone::Named("Monitor of Speakers".into())
    );
}

#[test]
fn without_a_default_the_first_microphone_is_named() {
    let sources = [
        source("Monitor of Speakers", true, false),
        source("USB Microphone", false, false),
        source("Built-in Audio", false, false),
    ];
    assert_eq!(
        Microphone::from_sources(&sources),
        Microphone::Named("USB Microphone".into())
    );
}

#[test]
fn a_missing_microphone_blocks_the_call_and_nothing_here_fixes_it() {
    let s = state(json!([]), json!([]));
    let on = realtime(true, "ready");
    let (sentence, action, ready) =
        gate_with(Some(&s), Some(&on), Reach::Untried, &Microphone::Missing);
    assert_eq!(sentence, "No microphone is connected.");
    assert_eq!(action, None);
    assert!(!ready);
}

#[test]
fn a_named_or_unknown_microphone_leaves_the_call_ready() {
    let s = state(json!([]), json!([]));
    let on = realtime(true, "ready");
    let named = Microphone::Named("fifine Microphone".into());
    for microphone in [&named, &Microphone::Unknown] {
        let (sentence, _, ready) = gate_with(Some(&s), Some(&on), Reach::Untried, microphone);
        assert_eq!(sentence, "Ready");
        assert!(ready);
    }
}

#[test]
fn what_the_daemon_or_the_versions_block_comes_before_the_microphone() {
    let s = state(json!([]), json!([]));
    let off = realtime(false, "disabled");
    let (_, action, _) = gate_with(Some(&s), Some(&off), Reach::Untried, &Microphone::Missing);
    assert_eq!(action, Some(GateAction::TurnOn));
    let on = realtime(true, "ready");
    let (sentence, _, _) = gate_with(
        Some(&s),
        Some(&on),
        Reach::ClientTooOld,
        &Microphone::Missing,
    );
    assert_eq!(
        sentence,
        "Update this app to talk to this version of Fermix."
    );
}

#[test]
fn the_microphone_row_names_the_device_or_says_none_or_unknown() {
    let named = Microphone::Named("fifine Microphone".into());
    assert_eq!(named.label(), "fifine Microphone");
    assert_eq!(Microphone::Missing.label(), "None");
    assert_eq!(Microphone::Unknown.label(), "Unknown");
}

#[test]
fn plugging_a_microphone_in_reopens_the_call() {
    let s = state(json!([]), json!([]));
    let on = realtime(true, "ready");
    let before = gate_with(Some(&s), Some(&on), Reach::Untried, &Microphone::Missing);
    let plugged = Microphone::from_sources(&[source("fifine Microphone", false, true)]);
    let after = gate_with(Some(&s), Some(&on), Reach::Untried, &plugged);
    assert!(!before.2 && after.2);
}

#[test]
fn a_daemon_gap_outranks_a_missing_microphone() {
    let (sentence, _, _) = gate_with(None, None, Reach::Untried, &Microphone::Missing);
    assert_eq!(sentence, "Fermix is not running.");
}
