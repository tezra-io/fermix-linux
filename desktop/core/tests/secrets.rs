//! Where Fermix keeps secrets: which refusals the file store would avoid, what a
//! refused save says, and the store the `secrets` section names.

use fermix_client::management::{decode_response, Refusal};
use fermix_client::secrets::{
    configured_store, refused_sentence, store_refused, FILE, KEYRING, SECTION,
};
use fermix_client::settings::{sections_for, SectionRows, Sections};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");

fn golden<T: DeserializeOwned>(name: &str) -> T {
    let found: Value = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"));
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    let result = decode_response(&serde_json::to_vec(response).unwrap(), id).unwrap();
    serde_json::from_value(result).unwrap()
}

fn refusal(code: &str, reason: Option<&str>) -> Refusal {
    Refusal {
        code: code.into(),
        sentence: "The secret could not be stored.".into(),
        field: None,
        reason: reason.map(str::to_owned),
    }
}

fn section(value: &str) -> SectionRows {
    serde_json::from_value(json!({
        "id": "secrets",
        "title": "Secrets",
        "rows": [{
            "key": "secret_store", "kind": "choice", "label": "Keep secrets in",
            "footer": null, "info": null, "value": value, "present": null,
            "options": [
                {"value": "keyring", "label": "Your keyring", "hint": null, "disabled": false},
                {"value": "file", "label": "A file in the Fermix folder", "hint": null, "disabled": false}
            ],
            "min": null, "max": null, "step": null, "restart": false, "read_only": false,
            "suggestions": false, "unit": null, "format": null
        }]
    }))
    .unwrap()
}

#[test]
fn only_the_secret_store_refusing_is_a_store_refusal() {
    for reason in ["locked", "unavailable", "timeout"] {
        assert!(store_refused(&refusal("secret_store_failed", Some(reason))));
    }
    assert!(!store_refused(&refusal("invalid_params", None)));
    assert!(!store_refused(&refusal("external_change", None)));
}

#[test]
fn a_refused_save_says_what_the_keyring_did() {
    let locked = refused_sentence(&refusal("secret_store_failed", Some("locked")));
    assert!(locked.contains("keyring is locked"), "{locked}");
    let absent = refused_sentence(&refusal("secret_store_failed", Some("unavailable")));
    assert!(absent.contains("No keyring"), "{absent}");
    let slow = refused_sentence(&refusal("secret_store_failed", Some("timeout")));
    assert!(slow.contains("in time"), "{slow}");
    // A reason this app does not know, or another code's reason, keeps the daemon's words.
    let other = refused_sentence(&refusal("secret_store_failed", Some("new_reason")));
    assert_eq!(other, "The secret could not be stored.");
    let elsewhere = refused_sentence(&refusal("unavailable", Some("timeout")));
    assert_eq!(elsewhere, "The secret could not be stored.");
}

#[test]
fn the_engine_publishes_the_store_on_the_secrets_pane() {
    let sections: Sections = golden("settings_sections");
    let on_pane: Vec<&str> = sections_for("secrets", &sections.sections)
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(on_pane, [SECTION]);
    let secrets: SectionRows = golden("settings_get_secrets");
    assert_eq!(configured_store(&secrets), Some(KEYRING));
    let row = &secrets.rows[0];
    assert!(!row.restart, "the store applies at once");
    let options: Vec<&str> = row.options.iter().map(|o| o.value.as_str()).collect();
    assert_eq!(options, [KEYRING, FILE]);
}

#[test]
fn the_section_names_the_configured_store() {
    assert_eq!(configured_store(&section("keyring")), Some(KEYRING));
    assert_eq!(configured_store(&section("file")), Some(FILE));
    assert_eq!(configured_store(&section("elsewhere")), None);
    let mut empty = section("keyring");
    empty.rows.clear();
    assert_eq!(configured_store(&empty), None);
}
