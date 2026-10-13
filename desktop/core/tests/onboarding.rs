//! The setup assistant's decisions (M38 §5.5): what a launch opens on, where
//! Starting lands, when a stage may be left, what About you writes, and the
//! one finish gate.

use fermix_client::model::SetupState;
use fermix_client::onboarding::{
    already_answered, connect_done, finish_gate, landing, opening, personalization_answer,
    starting_silent, Launch, Opening, Stage, STYLES,
};
use fermix_client::service::Binding;
use fermix_client::view::DaemonProblem;
use serde_json::{json, Value};
use std::time::Duration;

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
    // A home the first boot seeded still names personalization, as advice.
    let seeded = gap("personality", "personalization", false);
    assert_eq!(landing(&state(json!([seeded]), false)), Stage::Ready);
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

fn ready() -> SetupState {
    let mut ready = state(json!([]), false);
    ready.readiness.status = "ready".into();
    ready
}

const UNREADABLE: &str =
    "The service binding at /home/someone/.config/fermix/service.json could not be read as JSON.";

fn bindings() -> [Binding; 3] {
    [
        Binding::Absent,
        Binding::Unreadable(UNREADABLE.into()),
        Binding::Present,
    ]
}

#[test]
fn a_fresh_account_launched_by_its_user_opens_the_assistant_at_welcome() {
    assert_eq!(
        opening(Launch::User, &Binding::Absent, None),
        Opening::Welcome
    );
}

#[test]
fn a_login_launch_opens_no_assistant() {
    let unfinished = state(json!([]), false);
    for binding in bindings() {
        assert_eq!(opening(Launch::Login, &binding, None), Opening::Home);
        assert_eq!(
            opening(Launch::Login, &binding, Some(&unfinished)),
            Opening::Home,
            "{binding:?}"
        );
        assert_eq!(
            opening(Launch::Login, &binding, Some(&ready())),
            Opening::Chat,
            "{binding:?}"
        );
    }
}

#[test]
fn a_bound_account_whose_setup_is_unfinished_opens_the_assistant_as_before() {
    let provider = gap("providers", "provider:missing_credentials:openai", true);
    let unfinished = state(json!([provider]), false);
    assert_eq!(
        opening(Launch::User, &Binding::Present, Some(&unfinished)),
        Opening::Welcome
    );
}

#[test]
fn a_bound_account_with_no_engine_running_lands_on_home() {
    assert_eq!(
        opening(Launch::User, &Binding::Present, None),
        Opening::Home
    );
}

#[test]
fn a_ready_fermix_opens_on_chat_whatever_the_binding_file_says_of_a_first_run() {
    assert_eq!(
        opening(Launch::User, &Binding::Present, Some(&ready())),
        Opening::Chat
    );
    // A daemon that answers is a home in use, however it was started.
    assert_eq!(
        opening(Launch::User, &Binding::Absent, Some(&ready())),
        Opening::Chat
    );
}

/// macOS sends an unreadable bootstrap record to Recovery before it looks at
/// anything else; so does this, whether or not a daemon answers.
#[test]
fn an_unreadable_binding_stops_on_its_reason_and_never_guesses_a_home() {
    let unreadable = Binding::Unreadable(UNREADABLE.into());
    let stopped = Opening::Recovery(UNREADABLE.into());
    assert_eq!(opening(Launch::User, &unreadable, None), stopped);
    let unfinished = state(json!([]), false);
    assert_eq!(
        opening(Launch::User, &unreadable, Some(&unfinished)),
        stopped
    );
    assert_eq!(opening(Launch::User, &unreadable, Some(&ready())), stopped);
}

#[test]
fn starting_says_how_long_it_waited_for_an_answer() {
    let sentence = starting_silent(Duration::from_secs(90));
    assert!(sentence.contains("within 90 seconds"), "{sentence}");
    assert!(
        sentence.contains("journalctl --user -u fermix"),
        "{sentence}"
    );
}

#[test]
fn starting_starts_fermix_only_when_nothing_answered() {
    assert_eq!(already_answered(&DaemonProblem::NotRunning), None);
    assert_eq!(already_answered(&DaemonProblem::NotResponding), None);
}

#[test]
fn a_daemon_that_answered_but_cannot_serve_this_app_stops_starting_with_its_sentence() {
    let problems = [
        DaemonProblem::UpdateNeeded(
            "Fermix is older than this app. Update the fermix package.".into(),
        ),
        DaemonProblem::Broken("unexpected reply".into()),
    ];
    for problem in problems {
        assert_eq!(
            already_answered(&problem),
            Some(problem.sentence()),
            "{problem:?}"
        );
    }
}
