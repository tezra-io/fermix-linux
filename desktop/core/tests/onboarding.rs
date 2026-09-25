//! The setup assistant's decisions (M38 §5.5): where Starting lands, when a
//! stage may be left, what About you writes, and the one finish gate.

use fermix_client::model::SetupState;
use fermix_client::onboarding::{
    connect_done, finish_gate, landing, personalization_answer, Stage, STYLES,
};
use serde_json::{json, Value};

fn state(failures: Value, restart: bool) -> SetupState {
    serde_json::from_value(json!({
        "readiness": {"status": "setup_required", "failures": failures},
        "restart": {"required": restart, "reasons": []},
        "providers": [],
        "channels": [],
        "features": {"voice": false, "voice_notes": false, "meetings": false, "computer_use": false},
        "coexistence": {"config_state": "clear"}
    }))
    .unwrap()
}

fn gap(pane: &str, key: &str, gating: bool) -> Value {
    json!({"component": key, "gating": gating, "pane": pane, "detail_key": key})
}

#[test]
fn starting_lands_on_the_first_gap_that_has_a_screen() {
    let provider = gap("providers", "provider:missing_credentials:openai", true);
    let about = gap("personality", "personalization", true);
    assert_eq!(
        landing(&state(json!([about.clone(), provider]), false)),
        Stage::Connect
    );
    assert_eq!(landing(&state(json!([about]), false)), Stage::AboutYou);
    assert_eq!(landing(&state(json!([]), true)), Stage::Applying);
    let advisory = gap("channels", "channel:whatsapp", false);
    assert_eq!(landing(&state(json!([advisory]), false)), Stage::Ready);
}

#[test]
fn connect_is_done_only_when_no_provider_gap_gates() {
    let provider = gap("providers", "provider:missing_credentials:openai", true);
    assert!(!connect_done(&state(json!([provider]), false)));
    let about = gap("personality", "personalization", true);
    assert!(connect_done(&state(json!([about]), false)));
}

#[test]
fn about_you_writes_all_four_even_with_nothing_typed() {
    let answer = personalization_answer("  ", "", 1, "", "sam");
    assert_eq!(answer["user_name"], json!("sam"));
    assert_eq!(answer["timezone"], json!("UTC"));
    assert_eq!(answer["communication_style"], json!(STYLES[1].1));
    assert_eq!(answer["bot_name"], json!("Fermix"));
    let typed = personalization_answer(" Ada ", "Europe/Paris", 0, "Ivy", "sam");
    assert_eq!(typed["user_name"], json!("Ada"));
    assert_eq!(
        typed["communication_style"],
        json!("Answer in as few words as the question allows.")
    );
    assert_eq!(typed.len(), 4);
}

/// The protocol window is checked on every read (`hello_problem`), so the gate
/// holds the other three: the Linux package, no gating gap, no pending restart.
#[test]
fn the_finish_gate_holds_its_conditions() {
    let good = Some("linux_package");
    assert_eq!(finish_gate(good, &state(json!([]), false)), Ok(()));
    assert!(finish_gate(Some("macos_app"), &state(json!([]), false)).is_err());
    assert!(finish_gate(None, &state(json!([]), false)).is_err());
    let gating = gap("personality", "personalization", true);
    assert!(finish_gate(good, &state(json!([gating]), false)).is_err());
    assert!(finish_gate(good, &state(json!([]), true)).is_err());
    let advisory = gap("channels", "channel:whatsapp", false);
    assert_eq!(finish_gate(good, &state(json!([advisory]), false)), Ok(()));
}
