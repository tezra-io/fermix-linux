//! How each provider gets a credential, and what state its row is in.
//! The Anthropic cases exist because the old app offered it a browser sign-in
//! the daemon refuses ("This provider has no browser sign-in.").

use fermix_client::model::ProviderRow;
use fermix_client::providers::{connection, doors, Connection, Door, ImportSource};
use serde_json::json;

fn row(id: &str, auth_modes: &[&str], auth_mode: &str) -> ProviderRow {
    serde_json::from_value(json!({
        "id": id, "label": id, "auth_modes": auth_modes, "auth_mode": auth_mode,
        "configured": false, "primary": false, "present_key": false, "default_model": null,
        "reasoning_effort": null, "fast": null, "account_label": null, "token_state": null
    }))
    .unwrap()
}

#[test]
fn chatgpt_signs_in_through_the_browser_or_imports_the_codex_login() {
    let doors = doors(&row("openai_codex", &["oauth"], "oauth"));
    assert_eq!(
        doors,
        vec![Door::BrowserSignIn, Door::Import(ImportSource::CodexCli)]
    );
}

#[test]
fn anthropic_is_never_offered_a_browser_sign_in() {
    let doors = doors(&row("anthropic", &["api_key", "oauth"], "oauth"));
    assert!(!doors.contains(&Door::BrowserSignIn));
    assert_eq!(
        doors,
        vec![
            Door::Import(ImportSource::ClaudeCode),
            Door::SetupToken,
            Door::ApiKey
        ]
    );
}

#[test]
fn xai_offers_the_browser_then_a_key() {
    assert_eq!(
        doors(&row("xai", &["api_key", "oauth"], "api_key")),
        vec![Door::BrowserSignIn, Door::ApiKey]
    );
}

#[test]
fn key_only_providers_offer_just_the_key() {
    for id in ["openai", "openrouter", "mistral", "venice"] {
        assert_eq!(
            doors(&row(id, &["api_key"], "api_key")),
            vec![Door::ApiKey],
            "{id}"
        );
    }
}

#[test]
fn a_provider_that_needs_nothing_offers_no_door() {
    assert!(doors(&row("ollama", &["none"], "none")).is_empty());
}

#[test]
fn an_unknown_oauth_provider_is_not_guessed_into_a_browser_sign_in() {
    assert!(doors(&row("newcomer", &["oauth"], "oauth")).is_empty());
}

#[test]
fn import_sources_use_the_wire_names() {
    assert_eq!(ImportSource::ClaudeCode.wire(), "claude_code");
    assert_eq!(ImportSource::CodexCli.wire(), "codex_cli");
}

#[test]
fn api_key_secret_ids_follow_the_provider() {
    assert_eq!(Door::api_key_secret_id("mistral"), "mistral_api_key");
}

#[test]
fn connection_reads_the_row_the_daemon_published() {
    let mut r = row("openai_codex", &["oauth"], "oauth");
    assert_eq!(connection(&r), Connection::NotConnected);
    r.configured = true;
    r.token_state = Some("valid".into());
    assert_eq!(connection(&r), Connection::Connected);
    r.token_state = Some("expired".into());
    assert_eq!(connection(&r), Connection::Expired);

    let mut key = row("openai", &["api_key"], "api_key");
    key.configured = true;
    assert_eq!(
        connection(&key),
        Connection::NotConnected,
        "configured without a key is not connected"
    );
    key.present_key = true;
    assert_eq!(connection(&key), Connection::Connected);

    let mut local = row("ollama", &["none"], "none");
    local.configured = true;
    assert_eq!(connection(&local), Connection::Connected);
}

#[test]
fn a_door_round_trips_through_its_action_target() {
    for door in [
        Door::BrowserSignIn,
        Door::Import(ImportSource::ClaudeCode),
        Door::Import(ImportSource::CodexCli),
        Door::SetupToken,
        Door::ApiKey,
    ] {
        let target = door.target("anthropic");
        assert_eq!(
            Door::parse_target(&target),
            Some(("anthropic".to_owned(), door)),
            "{target}"
        );
    }
    assert_eq!(Door::parse_target("anthropic|teleport"), None);
    assert_eq!(Door::parse_target("no-separator"), None);
}
