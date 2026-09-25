//! Every answer the app decodes is decoded from the engine's own published fixtures,
//! so a shape drift in the engine fails here rather than on a user's screen.

use fermix_client::management::decode_response;
use fermix_client::model::{
    AuthStart, DetectResult, JobStatus, JobView, RestartOnly, SecretSetResult, SetupState,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");
const ERRORS: &str = include_str!("fixtures/management/errors.jsonl");

fn fixture(name: &str) -> Value {
    let found: Vec<Value> = SUCCESS
        .lines()
        .chain(ERRORS.lines())
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .filter(|v| v["name"] == name)
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one fixture named {name}");
    found.into_iter().next().unwrap()
}

fn decode<T: DeserializeOwned>(name: &str) -> T {
    let f = fixture(name);
    let response = &f["response"];
    let request_id = response["request_id"].as_str().expect("fixture has an id");
    let bytes = serde_json::to_vec(response).unwrap();
    let result = decode_response(&bytes, request_id).expect("fixture is a success");
    serde_json::from_value(result).expect("result decodes into the app's model")
}

#[test]
fn setup_state_decodes_providers_readiness_and_restart() {
    let state: SetupState = decode("setup_state_get");
    assert_eq!(state.readiness.status, "setup_required");
    assert!(state.readiness.failures.iter().any(|f| f.gating));
    assert!(state.restart.required);
    assert_eq!(
        state.restart.reasons[0].sentence,
        "Provider settings changed since Fermix started."
    );
    let codex = state
        .providers
        .iter()
        .find(|p| p.id == "openai_codex")
        .unwrap();
    assert!(codex.primary);
    assert_eq!(codex.token_state.as_deref(), Some("valid"));
    assert_eq!(codex.account_label.as_deref(), Some("owner@example.com"));
}

#[test]
fn setup_state_carries_channels_features_and_the_settings_file_state() {
    let state: SetupState = decode("setup_state_get");
    let whatsapp = state
        .channels
        .iter()
        .find(|c| c.name == "whatsapp")
        .unwrap();
    assert!(whatsapp.enabled && !whatsapp.configured);
    assert_eq!(whatsapp.status.as_deref(), Some("setup_required"));
    assert!(state.features.voice);
    assert!(!state.features.computer_use);
    assert_eq!(state.coexistence.config_state, "clear");
}

#[test]
fn auth_start_carries_the_url_once_beside_the_job() {
    let start: AuthStart = decode("auth_start");
    assert_eq!(start.job.status, JobStatus::Running);
    assert_eq!(start.job.phase.as_deref(), Some("awaiting_browser"));
    assert_eq!(start.job.budget_ms, 300_000);
    assert!(start.authorize_url.unwrap().starts_with("https://"));
}

#[test]
fn a_finished_job_decodes_its_flat_result() {
    let job: JobView = decode("job_get_completed");
    assert_eq!(job.status, JobStatus::Completed);
    assert!(job.failure.is_none());
    assert_eq!(job.result.unwrap()["latency_ms"], 812);
}

#[test]
fn import_logout_secret_and_primary_answers_decode() {
    let import: JobView = decode("auth_import_start");
    assert_eq!(import.status, JobStatus::Running);
    let logout: RestartOnly = decode("auth_logout");
    assert!(logout.restart.required);
    let secret: SecretSetResult = decode("secret_set_anthropic_setup_token");
    assert!(secret.present);
    let primary: RestartOnly = decode("providers_set_primary");
    assert!(primary.restart.required);
}

#[test]
fn detect_reports_claude_code_presence() {
    let detect: DetectResult = decode("setup_detect");
    let claude = detect
        .results
        .iter()
        .find(|r| r.target == "claude_code")
        .unwrap();
    assert!(claude.present);
}
