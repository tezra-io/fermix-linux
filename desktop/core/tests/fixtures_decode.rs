//! Every answer the app decodes is decoded from the engine's own published fixtures,
//! so a shape drift in the engine fails here rather than on a user's screen.

use fermix_client::capabilities::{ComputerPermissions, RowStatus};
use fermix_client::management::decode_response;
use fermix_client::model::{
    AuthStart, DetectResult, JobStatus, JobView, RestartOnly, SecretSetResult, SetupState,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const ERRORS: &str = include_str!("../contracts/management/fixtures/errors.jsonl");
/// The answers the engine says a client owes an older daemon.
const COMPATIBILITY: &str = include_str!("../contracts/management/fixtures/compatibility.jsonl");
/// Hand answers in the shape of the engine's Linux probe golden, for sessions it does not show.
const PROBES: &str = include_str!("fixtures/computer-use/permissions.jsonl");

fn fixture(name: &str) -> Value {
    let found: Vec<Value> = SUCCESS
        .lines()
        .chain(ERRORS.lines())
        .chain(COMPATIBILITY.lines())
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
    // The first boot seeds personalization, so it only advises (engine d9309720).
    assert_eq!(state.readiness.status, "ready");
    assert!(state.readiness.failures.iter().all(|f| !f.gating));
    let about = state
        .readiness
        .failures
        .iter()
        .find(|f| f.detail_key == "personalization")
        .unwrap();
    assert_eq!(about.pane.as_deref(), Some("personality"));
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

fn probe(name: &str) -> ComputerPermissions {
    let found = PROBES
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("probe line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no probe named {name}"));
    serde_json::from_value(found["result"].clone()).expect("probe decodes into the app's model")
}

#[test]
fn a_linux_probe_carries_its_platform_display_server_and_rows() {
    let linux: ComputerPermissions = decode("computer_use_permissions_get_linux");
    assert!(linux.installed && !linux.screen_capture && linux.input_control);
    assert_eq!(linux.platform.as_deref(), Some("linux"));
    assert_eq!(linux.display_server.as_deref(), Some("wayland"));
    let rows = linux.rows.unwrap();
    assert_eq!(rows.len(), 9);
    assert_eq!(rows[1].name, "accessibility bus");
    assert_eq!(rows[1].status, RowStatus::Ok);
    // The route the helper took leads the pixel rows.
    let route = rows.iter().position(|r| r.name == "pixel route").unwrap();
    assert_eq!(rows[route].status, RowStatus::Ok);
    assert_eq!(rows[route + 1].name, "x11 pixels");
    let stored = rows.iter().find(|r| r.name == "portal screenshot").unwrap();
    assert_eq!(stored.status, RowStatus::Warn);
}

#[test]
fn an_older_helpers_rows_carry_no_pixel_route() {
    let older = probe("linux_older_engine");
    assert_eq!(older.display_server.as_deref(), Some("x11"));
    let rows = older.rows.unwrap();
    assert!(rows.iter().all(|r| r.name != "pixel route"));
    assert!(rows.iter().any(|r| r.name == "x11 pixels"));
}

#[test]
fn a_macos_probe_carries_only_its_platform() {
    let macos = probe("macos_platform_only");
    assert_eq!(macos.platform.as_deref(), Some("macos"));
    assert_eq!(macos.display_server, None);
    assert_eq!(macos.rows, None);
}

#[test]
fn an_older_daemons_probe_has_no_platform_or_rows() {
    let older: ComputerPermissions = decode("computer_use_permissions_without_platform");
    assert!(older.installed && older.screen_capture && !older.input_control);
    assert_eq!(older.platform, None);
    assert_eq!(older.display_server, None);
    assert_eq!(older.rows, None);
}

#[test]
fn a_status_word_never_seen_decodes_rather_than_failing_the_answer() {
    let rows = probe("linux_unknown_rows").rows.unwrap();
    let status = |name: &str| rows.iter().find(|r| r.name == name).unwrap().status;
    assert_eq!(status("input method"), RowStatus::Unknown);
    assert_eq!(status("x11 pixels"), RowStatus::Fail);
    assert_eq!(status("window list"), RowStatus::NotApplicable);
}
