//! Home: the status word, "Answers with", and the Attention rows (design_final §2, §5).
//! Readiness failures carry a key, not a sentence, so the copy is the app's own,
//! ported from the macOS AttentionCatalogue so both apps say the same thing.

use fermix_client::management::{CallError, Refusal};
use fermix_client::model::Hello;
use fermix_client::model::SetupState;
use fermix_client::view::{
    answers_with, attention_rows, daemon_problem, hello_problem, status_word, AttentionAction,
    DaemonProblem,
};
use serde_json::json;
use std::io::ErrorKind;

fn state(status: &str, failures: serde_json::Value, restart: serde_json::Value) -> SetupState {
    serde_json::from_value(json!({
        "readiness": {"status": status, "failures": failures},
        "restart": restart,
        "providers": [
            {"id": "openai_codex", "label": "OpenAI Codex (ChatGPT)", "auth_modes": ["oauth"],
             "auth_mode": "oauth", "configured": true, "primary": true, "present_key": false,
             "default_model": "gpt-6-astra", "reasoning_effort": null, "fast": null,
             "account_label": null, "token_state": "valid"},
            {"id": "anthropic", "label": "Anthropic", "auth_modes": ["api_key", "oauth"],
             "auth_mode": "oauth", "configured": false, "primary": false, "present_key": false,
             "default_model": null, "reasoning_effort": null, "fast": null,
             "account_label": null, "token_state": null}
        ],
        "channels": [],
        "features": {"voice": false, "voice_notes": false, "meetings": false, "computer_use": false},
        "coexistence": {"config_state": "clear"}
    }))
    .unwrap()
}

fn no_restart() -> serde_json::Value {
    json!({"required": false, "reasons": []})
}

#[test]
fn a_ready_daemon_reads_ready() {
    assert_eq!(
        status_word(&state("ready", json!([]), no_restart())),
        "Ready"
    );
}

#[test]
fn a_pending_restart_outranks_ready() {
    let s = state(
        "ready",
        json!([]),
        json!({"required": true, "reasons": [{"section": "providers", "sentence": "Provider settings changed."}]}),
    );
    assert_eq!(status_word(&s), "Restart to finish updating");
}

#[test]
fn setup_required_outranks_a_pending_restart() {
    let s = state(
        "setup_required",
        json!([]),
        json!({"required": true, "reasons": []}),
    );
    assert_eq!(status_word(&s), "Setup required");
}

#[test]
fn answers_with_names_the_primary_and_its_model() {
    assert_eq!(
        answers_with(&state("ready", json!([]), no_restart())),
        "OpenAI Codex (ChatGPT) · gpt-6-astra"
    );
}

#[test]
fn answers_with_says_so_when_there_is_no_primary() {
    let mut s = state("ready", json!([]), no_restart());
    s.providers.iter_mut().for_each(|p| p.primary = false);
    assert_eq!(answers_with(&s), "No provider yet");
}

#[test]
fn a_missing_credential_routes_to_providers_under_the_providers_own_name() {
    let s = state(
        "setup_required",
        json!([{"component": "provider:anthropic", "gating": true, "pane": "providers", "detail_key": "provider:missing_credentials:anthropic"}]),
        no_restart(),
    );
    let rows = attention_rows(&s);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Connect Anthropic");
    assert_eq!(
        rows[0].body,
        "Fermix has no working credential for this provider yet."
    );
    assert_eq!(
        rows[0].action,
        Some(AttentionAction::Door {
            target: "anthropic|import:claude_code".into(),
            verb: "Import from Claude Code".into()
        })
    );
}

#[test]
fn a_gap_opens_the_settings_pane_the_daemon_names() {
    let s = state(
        "setup_required",
        json!([{"component": "personalization", "gating": true, "pane": "personality", "detail_key": "personalization"}]),
        no_restart(),
    );
    let rows = attention_rows(&s);
    assert_eq!(rows[0].title, "Tell Fermix about you");
    assert_eq!(
        rows[0].action,
        Some(AttentionAction::OpenPane("personality".into()))
    );
}

#[test]
fn a_gap_in_a_pane_this_app_does_not_know_opens_the_setup_page() {
    let s = state(
        "setup_required",
        json!([{"component": "x", "gating": true, "pane": "holodeck", "detail_key": "personalization"}]),
        no_restart(),
    );
    assert_eq!(
        attention_rows(&s)[0].action,
        Some(AttentionAction::OpenSetupPage)
    );
}

#[test]
fn gating_rows_come_first_then_restart_reasons_in_the_daemons_words() {
    let s = state(
        "setup_required",
        json!([
            {"component": "channel:whatsapp", "gating": false, "pane": "channels", "detail_key": "channel:whatsapp"},
            {"component": "personalization", "gating": true, "pane": "personality", "detail_key": "personalization"}
        ]),
        json!({"required": true, "reasons": [{"section": "providers", "sentence": "Provider settings changed since Fermix started."}]}),
    );
    let rows = attention_rows(&s);
    let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Tell Fermix about you",
            "WhatsApp is on but not finished",
            "Restart to apply your changes"
        ]
    );
    assert_eq!(
        rows[2].body,
        "Provider settings changed since Fermix started."
    );
    assert_eq!(rows[2].action, Some(AttentionAction::Restart));
}

#[test]
fn an_unknown_key_shows_the_daemons_own_name_for_it() {
    let s = state(
        "setup_required",
        json!([{"component": "x", "gating": true, "pane": "memory", "detail_key": "brand_new_gap"}]),
        no_restart(),
    );
    let rows = attention_rows(&s);
    assert_eq!(rows[0].title, "brand_new_gap");
    assert_eq!(rows[0].body, "This Fermix build has no description for that gap, so the daemon's own name for it is shown.");
}

#[test]
fn daemon_problems_map_to_the_screens_words() {
    assert_eq!(
        daemon_problem(&CallError::DaemonDown(ErrorKind::NotFound)),
        DaemonProblem::NotRunning
    );
    assert_eq!(
        daemon_problem(&CallError::DaemonDown(ErrorKind::ConnectionRefused)),
        DaemonProblem::NotResponding
    );
    assert_eq!(
        daemon_problem(&CallError::Timeout),
        DaemonProblem::NotResponding
    );
    assert!(matches!(
        daemon_problem(&CallError::Protocol("x".into())),
        DaemonProblem::Broken(_)
    ));
    assert_eq!(DaemonProblem::NotRunning.status_word(), "Not running");
    assert_eq!(DaemonProblem::NotResponding.status_word(), "Not responding");
}

fn refused(code: &str) -> CallError {
    CallError::Refused(Refusal {
        code: code.into(),
        sentence: "The daemon's words.".into(),
        field: None,
    })
}

#[test]
fn a_version_refusal_says_which_side_to_update() {
    let app_old = daemon_problem(&refused("client_too_old"));
    assert_eq!(
        app_old,
        DaemonProblem::UpdateNeeded("This app is older than Fermix. Update the Fermix app.".into())
    );
    let daemon_old = DaemonProblem::UpdateNeeded(
        "Fermix is older than this app. Update the fermix package.".into(),
    );
    assert_eq!(daemon_problem(&refused("daemon_too_old")), daemon_old);
    assert_eq!(daemon_problem(&refused("method_not_found")), daemon_old);
    assert_eq!(daemon_old.status_word(), "Update needed");
}

#[test]
fn every_problem_has_a_sentence_a_dialog_can_show() {
    assert_eq!(
        DaemonProblem::NotRunning.sentence(),
        "Fermix is not running."
    );
    assert_eq!(
        DaemonProblem::NotResponding.sentence(),
        "Fermix is not responding."
    );
    assert_eq!(
        DaemonProblem::Broken("x".into()).sentence(),
        "Fermix answered in a way this app does not understand."
    );
    assert_eq!(
        DaemonProblem::UpdateNeeded("Update it.".into()).sentence(),
        "Update it."
    );
}

fn hello(min: u64, max: u64, setup_state_min: u64) -> Hello {
    serde_json::from_value(json!({
        "protocol": {"current_version": max, "minimum_version": min, "maximum_version": max},
        "capabilities": {"methods": [], "minimum_versions": {"setup.state.get": setup_state_min}},
        "engine": {"product_version": "0.11.0", "pid": "1"},
        "setup": {}
    }))
    .unwrap()
}

#[test]
fn a_daemon_that_cannot_serve_this_app_is_caught_at_hello() {
    assert_eq!(hello_problem(&hello(1, 2, 2)), None);
    assert!(
        matches!(hello_problem(&hello(1, 1, 2)), Some(DaemonProblem::UpdateNeeded(s)) if s.contains("fermix package"))
    );
    assert!(
        matches!(hello_problem(&hello(3, 4, 2)), Some(DaemonProblem::UpdateNeeded(s)) if s.contains("Fermix app"))
    );
}
