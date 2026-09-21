//! The models, against the vendored goldens.
//!
//! Every test here drives the real model through the real decoding path: the
//! peer answers the engine's own published responses, and the command line is
//! the fake `fermix` under `tests/fixtures/cli/`. Nothing touches a real
//! daemon, a real command line, a real `~/.config` or a real keyring.

use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Once};
use std::time::Duration;

use fermix_desktop::copy::{self, Key};
use fermix_desktop::management::errors::{ManagementError, WireError};
use fermix_desktop::management::types::{
    CheckStatus, DoctorScope, JobKind, ModelSource, PluginAction, RemediationKind, SettingValue,
    SettingsPane,
};
use fermix_desktop::models::activation::{Action, Activation, Step, StepState};
use fermix_desktop::models::api::{UNLOCK_DEADLINE, WRITE_DEADLINE};
use fermix_desktop::models::computer::{ComputerModel, Standing};
use fermix_desktop::models::doctor::{CheckName, DoctorModel};
use fermix_desktop::models::home::{
    attention_rows, facts, skew_rows, status_word, toolbar_action, uptime, AttentionAction,
    AttentionRow, HomeModel, RowTitle, StatusWord, ToolbarAction,
};
use fermix_desktop::models::jobs::JobRunner;
use fermix_desktop::models::ledger::{PermissionLedger, Right};
use fermix_desktop::models::logs::LogsModel;
use fermix_desktop::models::meetings::{MeetingsModel, SignInState};
use fermix_desktop::models::onboarding::{
    ApplyStep, Block, Leading, OnboardingModel, Primary, Stage,
};
use fermix_desktop::models::peer::FixturePeer;
use fermix_desktop::models::plugins::{EnableOutcome, IntegrationFilter, PluginsModel};
use fermix_desktop::models::providers::{
    probe_sentence, ProviderRow, ProviderStanding, ProviderVerb, ProvidersModel,
};
use fermix_desktop::models::recovery::{Cause, RecoveryModel};
use fermix_desktop::models::secret_store::{Availability, StoreKind, StoreRefusal};
use fermix_desktop::models::settings_model::Sentence;
use fermix_desktop::models::{Change, SettingsModel};
use fermix_desktop::service::runner::ServiceRunner;
use fermix_desktop::session::build::GuiAlignment;
use fermix_desktop::testing::TempDirectory;
use gtk4::glib::MainContext;

/// Where this process keeps everything it would otherwise take from the host:
/// the application's state directory, its autostart entry, and one command line
/// per fake state.
///
/// One directory for the whole binary, built once. The command line stands in
/// as a wrapper per state rather than one binary reading a variable, because a
/// variable is process-wide and these tests run several at a time: two tests
/// sharing one variable is exactly the flake this harness exists to avoid.
fn harness() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    static ONCE: Once = Once::new();

    let root = ROOT.get_or_init(|| {
        let directory = Box::leak(Box::new(TempDirectory::new("models-host")));
        directory.path().to_path_buf()
    });

    ONCE.call_once(|| {
        std::env::set_var(
            fermix_desktop::session::state::STATE_DIRECTORY_OVERRIDE,
            root.join("state"),
        );
        std::env::set_var(
            fermix_desktop::session::autostart::DIRECTORY_OVERRIDE,
            root.join("autostart"),
        );

        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli");
        let wrappers = root.join("cli");
        std::fs::create_dir_all(&wrappers).expect("the wrapper directory is created");

        for entry in std::fs::read_dir(fixtures.join("states")).expect("the states are readable") {
            let state = entry.expect("a state directory").path();
            let name = state
                .file_name()
                .expect("a named state")
                .to_string_lossy()
                .into_owned();

            let wrapper = wrappers.join(&name);
            std::fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nFERMIX_FAKE_CLI_STATE='{}' exec '{}' \"$@\"\n",
                    state.display(),
                    fixtures.join("fermix").display()
                ),
            )
            .expect("the wrapper is written");

            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))
                .expect("the wrapper runs");
        }
    });

    root
}

/// A model over one scenario's goldens and one command line state.
fn model(scenario: &str, cli_state: &str) -> (Rc<SettingsModel>, Rc<FixturePeer>) {
    let cli = harness().join("cli").join(cli_state);
    assert!(cli.exists(), "no fake command line for {cli_state}");

    let peer = Rc::new(FixturePeer::new(scenario));
    let service = Rc::new(
        ServiceRunner::new(cli).with_deadlines(Duration::from_secs(5), Duration::from_secs(5)),
    );

    (
        SettingsModel::new(
            Rc::clone(&peer) as Rc<dyn fermix_desktop::models::api::ManagementApi>,
            service,
        ),
        peer,
    )
}

/// Run one future to its end. The models are ordinary async functions, so a
/// test drives them directly rather than through a main loop.
fn run<F: std::future::Future>(future: F) -> F::Output {
    MainContext::new().block_on(future)
}

// ---------------------------------------------------------------------------
// Home
// ---------------------------------------------------------------------------

#[test]
fn home_renders_running_without_anything_needing_attention() {
    let (model, _) = model("default", "active_aligned");
    run(model.refresh_all());

    let state = model.state();
    assert_eq!(status_word(&state), StatusWord::Running);
    assert_eq!(toolbar_action(&state), None);
}

#[test]
fn home_renders_the_gaps_the_daemon_reported_with_one_action_each() {
    let (model, _) = model("restart_pending", "pending_restart");
    run(model.refresh_all());

    let state = model.state();
    let rows = attention_rows(&state);

    let titles: Vec<RowTitle> = rows.iter().map(|row| row.title.clone()).collect();
    assert!(titles.contains(&RowTitle::Words(
        fermix_desktop::copy::Key::AttentionPersonalizationTitle
    )));
    assert!(titles.contains(&RowTitle::Words(
        fermix_desktop::copy::Key::AttentionChannelTitle
    )));

    for row in &rows {
        assert!(
            row.action.is_some(),
            "{} has nothing to do about it",
            row.id
        );
    }

    // The channel row names the channel the daemon named, in the daemon's own
    // words for it rather than a spelling this application invented.
    let channel = rows
        .iter()
        .find(|row| row.id == "channel:whatsapp")
        .expect("the whatsapp gap is reported");
    assert_eq!(channel.detail.as_deref(), Some("WhatsApp"));

    // The restart row carries the daemon's own reasons and opens the one
    // confirmation.
    let restart = rows
        .iter()
        .find(|row| row.id == "restart")
        .expect("the restart gap is reported");
    assert_eq!(restart.action, Some(AttentionAction::Restart));
    assert!(restart
        .detail
        .as_deref()
        .expect("the daemon published its reasons")
        .contains("Provider settings changed"));
}

#[test]
fn home_renders_setup_required_with_the_action_that_continues_it() {
    let (model, _) = model("setup_required", "active_aligned");
    run(model.refresh_all());

    let state = model.state();
    assert_eq!(status_word(&state), StatusWord::SetupRequired);
    assert_eq!(toolbar_action(&state), Some(ToolbarAction::ContinueSetup));
}

#[test]
fn home_renders_a_daemon_that_is_not_running() {
    let (model, peer) = model("default", "bound_disabled");
    peer.set_running(false);
    run(model.refresh_all());

    let state = model.state();
    assert_eq!(status_word(&state), StatusWord::NotRunning);
    assert_eq!(toolbar_action(&state), None);
    assert!(!state.background_enabled(), "the unit is disabled");
}

#[test]
fn home_renders_an_engine_waiting_for_a_restart_to_finish_updating() {
    let (model, _) = model("default", "pending_restart");
    run(model.refresh_all());

    let state = model.state();
    assert_eq!(status_word(&state), StatusWord::RestartToFinishUpdating);
    assert_eq!(toolbar_action(&state), Some(ToolbarAction::FinishUpdating));
}

#[test]
fn the_runtime_facts_are_the_daemons_own() {
    let (model, _) = model("default", "active_aligned");
    run(model.refresh_all());

    let state = model.state();
    let facts = facts(&state, None, model.api().negotiated_version());
    let value = |label| {
        facts
            .iter()
            .find(|fact| fact.label == label)
            .map(|fact| fact.value.clone())
            .unwrap_or_default()
    };

    use fermix_desktop::copy::Key;
    assert_eq!(value(Key::HomeRuntimeEngine), "0.9.0");
    assert_eq!(value(Key::HomeRuntimeManagementProtocol), "2");
    assert_eq!(value(Key::HomeRuntimeUptime), uptime(864_213));
    assert_eq!(value(Key::HomeRuntimeChannels), "telegram");
    assert_eq!(value(Key::HomeRuntimeSkills), "9");
    // Tools are what this build ships plus what the integrations add.
    assert_eq!(value(Key::HomeRuntimeTools), "53");
    // The provider fact carries the model beside the daemon's own label.
    assert_eq!(
        value(Key::HomeRuntimeProvider),
        "OpenAI Codex (ChatGPT) \u{00b7} gpt-5.6-sol"
    );
    // The service fact is the unit's own word, from the command line.
    assert_eq!(value(Key::HomeRuntimeService), "running");
}

#[test]
fn the_background_switch_reverts_with_the_command_lines_own_sentence() {
    let (model, _) = model("default", "linger_denied");
    run(model.refresh_service());

    let refusal = run(model.set_background_service(true))
        .expect_err("the command line refuses to enable linger in this state");

    assert_eq!(refusal.code.as_deref(), Some("linger_denied"));
    assert!(
        refusal
            .text
            .contains("has to keep running after you log out"),
        "the command line's own sentence, rendered rather than composed: {}",
        refusal.text
    );

    let state = model.state();
    assert!(
        state.background_pending.is_none(),
        "the optimistic position is released whichever way the answer went"
    );
    assert!(!state.background_enabled(), "the switch goes back");
}

#[test]
fn the_background_switch_holds_its_position_until_the_answer_lands() {
    let (model, _) = model("default", "install_ok");
    run(model.refresh_service());

    run(model.set_background_service(true)).expect("the command line installs");

    assert!(model.state().background_enabled());
}

#[test]
fn opening_at_login_is_one_file_in_this_persons_own_home() {
    let (model, _) = model("default", "active_aligned");

    assert!(!model.state().open_at_login);
    model.set_open_at_login(true).expect("the entry is written");
    assert!(model.state().open_at_login);

    model.set_open_at_login(false).expect("the entry is hidden");
    assert!(!model.state().open_at_login);
}

// ---------------------------------------------------------------------------
// Doctor
// ---------------------------------------------------------------------------

#[test]
fn doctor_renders_a_run_that_finished_clean() {
    let (model, _) = model("doctor_healthy", "active_aligned");
    let doctor = DoctorModel::new(model);

    run(doctor.start(DoctorScope::Local));

    assert!(!doctor.is_running(), "the session finished");
    assert_eq!(doctor.failed(), 0);
    assert_eq!(doctor.rows().len(), 4);
    assert!(doctor
        .rows()
        .iter()
        .all(|row| row.status == CheckStatus::Passed));
}

#[test]
fn doctor_renders_a_failure_with_its_remediation_and_its_evidence() {
    let (model, _) = model("doctor_failed", "active_aligned");
    let doctor = DoctorModel::new(model);

    run(doctor.start(DoctorScope::Local));

    assert_eq!(doctor.failed(), 1);
    let rows = doctor.rows();

    let failed = rows
        .iter()
        .find(|row| row.status == CheckStatus::Failed)
        .expect("the failed check is rendered");

    // The name is the daemon's own, out of the evidence it published beside it.
    assert_eq!(failed.name, CheckName::Words("listener port".into()));
    assert!(failed.needs_action());

    let remediation = failed
        .remediation
        .as_ref()
        .expect("the daemon published a remediation");
    assert_eq!(remediation.action.kind, RemediationKind::SettingsPane);
    assert_eq!(remediation.action.target.as_deref(), Some("sandbox"));
    assert!(!remediation.title.is_empty());

    assert!(
        failed
            .evidence
            .iter()
            .any(|(label, value)| label == "holder_pid" && value == "914"),
        "the evidence is the daemon's own"
    );

    // A run can end in more than one way, and each keeps its own word.
    assert!(rows.iter().any(|row| row.status == CheckStatus::TimedOut));
    assert!(rows
        .iter()
        .any(|row| row.status == CheckStatus::NotApplicable));
}

#[test]
fn doctor_renders_a_run_that_is_still_going() {
    let (model, _) = model("default", "active_aligned");
    let doctor = DoctorModel::new(model);

    run(doctor.start(DoctorScope::Local));

    assert!(doctor.is_running());
    assert!(doctor.session().is_some());
}

#[test]
fn network_checks_run_only_when_they_are_asked_for() {
    let (model, peer) = model("default", "active_aligned");
    let doctor = DoctorModel::new(model);

    run(doctor.start(DoctorScope::Local));
    assert_eq!(
        peer.last("doctor.start"),
        Some(serde_json::json!({"scope": "local"})),
        "entering the surface runs the local checks and nothing else"
    );

    run(doctor.start(DoctorScope::Network));
    assert_eq!(
        peer.last("doctor.start"),
        Some(serde_json::json!({"scope": "network"}))
    );
}

#[test]
fn leaving_doctor_asks_the_daemon_to_stop_the_run() {
    let (model, peer) = model("default", "active_aligned");
    let doctor = DoctorModel::new(model);

    run(doctor.start(DoctorScope::Local));
    run(doctor.cancel());

    assert_eq!(peer.count("doctor.cancel"), 1);
    assert!(!doctor.is_polling());
}

#[test]
fn a_refused_run_keeps_the_daemons_own_sentence() {
    let (model, peer) = model("default", "active_aligned");
    let doctor = DoctorModel::new(model);
    peer.refuse_next("doctor.start", "busy");

    run(doctor.start(DoctorScope::Local));

    let refusal = doctor.refusal().expect("the refusal is kept");
    assert_eq!(refusal.code.as_deref(), Some("busy"));
    assert!(!refusal.text.is_empty());
    assert!(doctor.session().is_none());
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

#[test]
fn logs_reads_the_tail_the_daemon_publishes() {
    let (model, peer) = model("default", "active_aligned");
    let logs = LogsModel::new(model);

    run(logs.refresh());

    assert_eq!(logs.count(), 2);
    assert_eq!(
        peer.last("logs.query"),
        Some(serde_json::json!({"limit": 200, "direction": "backward"})),
        "the tail is the protocol's own initial window"
    );
}

#[test]
fn a_second_read_of_the_same_tail_leaves_the_list_alone() {
    let (model, _) = model("default", "active_aligned");
    let logs = LogsModel::new(model);

    run(logs.refresh());
    let before = logs.entries();
    run(logs.refresh());

    assert_eq!(
        logs.entries(),
        before,
        "a poll that finds nothing new must not move anyone's place in the list"
    );
}

#[test]
fn an_expired_cursor_starts_the_window_again_with_the_daemons_own_sentence() {
    let (model, peer) = model("default", "active_aligned");
    let logs = LogsModel::new(model);

    run(logs.refresh());
    assert_eq!(logs.count(), 2);

    peer.refuse_next("logs.query", "cursor_expired");
    run(logs.refresh());

    assert_eq!(logs.count(), 0, "what was held is no longer what is there");
    let notice = logs.notice().expect("the daemon said why");
    assert!(notice.contains("rotation"));

    logs.clear_notice();
    assert!(logs.notice().is_none());
}

#[test]
fn a_daemon_that_is_not_running_leaves_logs_with_nothing_to_read() {
    let (model, peer) = model("default", "active_aligned");
    let logs = LogsModel::new(model);
    peer.set_running(false);

    run(logs.refresh());

    assert!(logs.is_unavailable());
    assert_eq!(logs.count(), 0);
}

#[test]
fn a_filter_is_sent_to_the_daemon_rather_than_applied_here() {
    let (model, peer) = model("default", "active_aligned");
    let logs = LogsModel::new(Rc::clone(&model));

    // The filters are set directly rather than through the surface, because
    // what is being proven is the query, not the widget.
    logs.set_level(Some(fermix_desktop::management::types::LogLevel::Warning));
    logs.set_search(Some("socket".into()));
    run(logs.refresh());

    let sent = peer.last("logs.query").expect("a query was sent");
    assert_eq!(sent["level"], "warning");
    assert_eq!(sent["search"], "socket");
}

// ---------------------------------------------------------------------------
// The descriptor rows, through the model
// ---------------------------------------------------------------------------

#[test]
fn a_write_sends_one_key_of_one_section() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_sections());
    run(model.refresh_section("memory"));

    run(model.apply(
        "memory",
        "review_interval_hours",
        SettingValue::Number(12.0),
    ));

    assert_eq!(
        peer.last("settings.apply"),
        Some(serde_json::json!({
            "section": "memory",
            "values": {"review_interval_hours": 12.0}
        }))
    );
}

#[test]
fn a_written_value_holds_until_the_accepted_re_read_lands() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("memory"));

    // The daemon answers the apply, and the re-read that follows carries the
    // value it accepted.
    let mut updated = serde_json::json!({"id": "memory", "title": "Memory", "rows": []});
    updated["rows"] = serde_json::json!([{
        "key": "review_interval_hours", "kind": "number", "label": "Review memory every",
        "footer": null, "value": 12, "present": null, "options": [], "min": 0, "max": null,
        "step": 1, "restart": false, "read_only": false, "suggestions": false,
        "unit": "hours", "format": "hours"
    }]);
    peer.set_result("settings.get", Some("memory"), updated);

    run(model.apply(
        "memory",
        "review_interval_hours",
        SettingValue::Number(12.0),
    ));

    let state = model.state();
    assert_eq!(
        state.daemon_value("memory", "review_interval_hours"),
        Some(SettingValue::Number(12.0)),
        "the accepted re-read is what the row now shows"
    );
    assert!(
        !state
            .in_flight
            .contains_key(&("memory".to_string(), "review_interval_hours".to_string())),
        "the optimistic value is released by the re-read that confirmed it"
    );
}

#[test]
fn a_refusal_puts_the_daemons_value_back_and_keeps_its_sentence() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("memory"));
    let before = model
        .state()
        .daemon_value("memory", "review_interval_hours")
        .expect("the daemon published a value");

    peer.refuse_next("settings.apply", "invalid_params");
    run(model.apply(
        "memory",
        "review_interval_hours",
        SettingValue::Number(999.0),
    ));

    let state = model.state();
    let id = ("memory".to_string(), "review_interval_hours".to_string());

    assert_eq!(state.value_of(&id), None, "nothing optimistic is left over");
    assert_eq!(
        state.daemon_value("memory", "review_interval_hours"),
        Some(before)
    );

    let refusal = state.refusal(&id).expect("the refusal is under the row");
    assert_eq!(refusal.code.as_deref(), Some("invalid_params"));
    assert!(!refusal.text.is_empty());
}

#[test]
fn a_write_refused_because_the_file_changed_moves_the_whole_surface() {
    let (model, _) = model("external_change", "active_aligned");
    run(model.refresh_all());
    run(model.refresh_section("memory"));

    run(model.apply(
        "memory",
        "review_interval_hours",
        SettingValue::Number(12.0),
    ));

    assert_eq!(
        model.state().config,
        fermix_desktop::management::ConfigCondition::ExternalChange
    );
}

#[test]
fn a_file_that_cannot_be_read_carries_the_parsers_own_sentence_and_no_reload() {
    let (model, _) = model("unreadable", "active_aligned");
    run(model.refresh_all());
    run(model.apply(
        "memory",
        "review_interval_hours",
        SettingValue::Number(12.0),
    ));

    let state = model.state();
    assert_eq!(
        state.config,
        fermix_desktop::management::ConfigCondition::Unreadable
    );
    let sentence = state.unreadable.as_ref().expect("the parser said why");
    assert!(sentence.text.contains("line 41"));
}

#[test]
fn a_draft_survives_a_refresh_landing_underneath_it() {
    let (model, _) = model("default", "active_aligned");
    run(model.refresh_section("memory"));

    model.set_draft(
        "memory",
        "review_interval_hours",
        SettingValue::Number(48.0),
    );
    run(model.refresh_section("memory"));

    assert_eq!(
        model.draft("memory", "review_interval_hours"),
        Some(SettingValue::Number(48.0)),
        "only the person who typed it can end a draft"
    );
    assert_eq!(
        model
            .state()
            .value_of(&("memory".into(), "review_interval_hours".into())),
        Some(SettingValue::Number(48.0)),
        "the row shows what was typed, not what was re-read"
    );
}

#[test]
fn a_discarded_draft_leaves_the_daemons_value_showing() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("memory"));

    model.set_draft(
        "memory",
        "review_interval_hours",
        SettingValue::Number(48.0),
    );
    model.discard_draft("memory", "review_interval_hours");

    assert_eq!(model.draft("memory", "review_interval_hours"), None);
    assert_eq!(
        peer.count("settings.apply"),
        0,
        "restoring the daemon's value sends nothing"
    );
}

#[test]
fn a_secret_is_stored_by_its_own_key_and_never_read_back() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("realtime"));

    let stored = run(model.set_secret("realtime", "openai_api_key", "sk-not-a-real-key".into()));
    assert!(stored.is_ok(), "the daemon took the key");

    assert_eq!(
        peer.last("secret.set"),
        Some(serde_json::json!({"id": "openai_api_key", "value": "sk-not-a-real-key"}))
    );
}

#[test]
fn a_blank_secret_is_never_sent() {
    let (model, peer) = model("default", "active_aligned");

    let blank = run(model.set_secret("realtime", "openai_api_key", String::new()));
    assert!(
        blank.is_ok(),
        "a blank value is not an error, it is nothing to send"
    );

    assert_eq!(peer.count("secret.set"), 0);
}

#[test]
fn removing_a_secret_forgets_it_by_the_same_key() {
    let (model, peer) = model("default", "active_aligned");

    let cleared = run(model.clear_secret("realtime", "openai_api_key"));
    assert!(cleared.is_ok(), "the daemon forgot the key");

    assert_eq!(
        peer.last("secret.clear"),
        Some(serde_json::json!({"id": "openai_api_key"}))
    );
}

// ---------------------------------------------------------------------------
// The model itself
// ---------------------------------------------------------------------------

#[test]
fn a_result_from_a_connection_that_has_gone_never_reaches_the_model() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_all());
    let before = model.state().overview.is_some();
    assert!(before);

    // What a restart does: the epoch moves, and everything issued under the old
    // connection stops being acceptable.
    peer.set_running(false);
    peer.disconnect();
    run(model.refresh_overview());

    assert_eq!(model.state().reachable, Some(false));
}

#[test]
fn the_client_is_pointed_at_the_socket_the_command_line_reported() {
    let (model, peer) = model("default", "active_aligned");

    run(model.refresh_service());

    assert_eq!(
        peer.rebinds(),
        vec![std::path::PathBuf::from(
            "/home/operator/.fermix/daemon.sock"
        )],
        "a packaged run learns where the socket is from the command line"
    );
}

#[test]
fn the_selected_pane_is_the_models_and_it_tells_whoever_is_listening() {
    let (model, _) = model("default", "active_aligned");
    let heard = Rc::new(std::cell::RefCell::new(Vec::new()));
    {
        let heard = Rc::clone(&heard);
        model.observe(move |change| heard.borrow_mut().push(change.clone()));
    }

    model.select_pane(SettingsPane::Memory);
    model.select_pane(SettingsPane::Memory);

    assert_eq!(model.pane(), SettingsPane::Memory);
    assert_eq!(
        heard
            .borrow()
            .iter()
            .filter(|change| **change == Change::Pane)
            .count(),
        1,
        "selecting the pane that is already selected is not a change"
    );
}

#[test]
fn the_sections_are_the_daemons_inventory_and_nothing_else() {
    let (model, _) = model("default", "active_aligned");
    run(model.refresh_sections());

    let state = model.state();
    assert_eq!(state.sections.len(), 25);

    let voice: Vec<&str> = state
        .sections_of(SettingsPane::Voice)
        .iter()
        .map(|section| section.id.as_str())
        .collect();
    assert_eq!(voice, vec!["realtime", "transcription"]);
}

#[test]
fn reloading_takes_the_daemons_own_answer_about_the_file() {
    let (model, peer) = model("external_change", "active_aligned");
    run(model.refresh_all());
    run(model.apply(
        "memory",
        "review_interval_hours",
        SettingValue::Number(12.0),
    ));
    assert_eq!(
        model.state().config,
        fermix_desktop::management::ConfigCondition::ExternalChange
    );

    // A reload changes what the daemon says about the file, which is the only
    // thing that can clear this state.
    let mut cleared = peer_setup_state(&peer);
    cleared["coexistence"]["config_state"] = serde_json::json!("clear");
    peer.set_result("setup.state.get", None, cleared);

    run(model.reload());

    assert_eq!(peer.count("settings.reload"), 1);
    assert_eq!(
        model.state().config,
        fermix_desktop::management::ConfigCondition::Clear,
        "the daemon's own answer is what clears it"
    );
    assert!(
        model.state().refusals.is_empty(),
        "the refusals the changed file caused go with it"
    );
}

/// The setup state this peer is answering with, as a value a test can change
/// one field of. A daemon that was asked again after something happened is the
/// state this represents.
fn peer_setup_state(peer: &FixturePeer) -> serde_json::Value {
    let _ = peer;
    let goldens = fermix_desktop::fixtures::Goldens::load("external_change").expect("loads");
    goldens.answer(&serde_json::json!({
        "request_id": "x",
        "protocol_version": 2,
        "method": "setup.state.get",
        "params": {}
    }))["result"]
        .clone()
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

#[test]
fn recovery_reads_the_offline_evidence_through_the_command_line() {
    let (model, _) = model("unreadable", "active_aligned");
    let recovery = RecoveryModel::new(Rc::clone(&model));

    run(model.refresh_all());
    run(recovery.collect());

    assert!(matches!(recovery.cause(), Some(Cause::ConfigUnreadable(_))));
    assert!(recovery.can_export());
    assert!(
        !recovery.evidence().is_empty(),
        "the export carries the log tail the daemon could not serve"
    );
}

#[test]
fn recovery_says_so_when_the_command_line_cannot_run_at_all() {
    let (model, _) = model("default", "user_manager_unreachable");
    let recovery = RecoveryModel::new(Rc::clone(&model));

    run(model.refresh_service());
    run(recovery.collect());

    let cause = recovery.cause().expect("the surface has a cause to show");
    let Cause::ServiceRefused(sentence) = &cause else {
        panic!("expected the command line's own refusal, got {cause:?}");
    };
    assert!(
        sentence.contains("no user service manager"),
        "the cause is the command line's own sentence"
    );

    // The collector could not run either, so the page says so rather than
    // offering an export that would write nothing.
    assert!(!recovery.can_export(), "there is nothing to export from");
    assert!(recovery.refusal().is_some(), "the collector said why too");
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

#[test]
fn a_job_is_followed_until_it_stops() {
    let (model, peer) = model("default", "active_aligned");
    let runner = JobRunner::new(model.api());

    let started: fermix_desktop::management::types::JobView =
        serde_json::from_value(serde_json::json!({
            "job_id": "job:2Kd9mQ", "kind": "provider_probe", "status": "running",
            "phase": "calling", "progress": null, "budget_ms": 15000,
            "started_at": "2026-08-19T12:00:00Z", "finished_at": null,
            "result": null, "failure": null
        }))
        .expect("decodes");

    runner.adopt(started);
    assert!(runner.is_polling());

    run(runner.read_once());

    assert_eq!(peer.count("job.get"), 1);
    assert!(runner.is_terminal(), "the golden answers a finished job");
}

#[test]
fn dismissing_a_view_detaches_the_poll_without_stopping_the_work() {
    let (model, peer) = model("default", "active_aligned");
    let runner = JobRunner::new(model.api());

    runner.adopt(running_job());
    runner.detach();

    assert!(!runner.is_polling());
    assert!(runner.is_detached());
    assert_eq!(peer.count("job.cancel"), 0, "detaching cancels nothing");
}

#[test]
fn reopening_a_surface_finds_the_run_it_started() {
    let (model, peer) = model("default", "active_aligned");
    let runner = JobRunner::new(model.api());

    let found = run(runner.reattach(JobKind::CapabilityInstall));

    assert!(found, "the daemon still retains the run");
    assert_eq!(peer.count("job.list"), 1);
    assert_eq!(
        runner.job().expect("the job is followed").job_id,
        "job:9Mn0pQ"
    );

    let missing = run(runner.reattach(JobKind::MeetingsSignin));
    assert!(!missing, "a kind the daemon is not running is not adopted");
}

#[test]
fn cancelling_asks_the_daemon_to_stop() {
    let (model, peer) = model("default", "active_aligned");
    let runner = JobRunner::new(model.api());

    runner.adopt(running_job());
    run(runner.cancel());

    assert_eq!(peer.count("job.cancel"), 1);
    assert!(!runner.is_polling());
}

#[test]
fn a_job_poll_is_bounded_by_the_daemons_budget_and_by_a_ceiling() {
    use fermix_desktop::models::jobs::{poll_cap, POLL_CAP};

    assert_eq!(poll_cap(15_000), 20, "the daemon's budget, in ticks");
    assert_eq!(poll_cap(u64::MAX), POLL_CAP, "and never more than the cap");
}

fn running_job() -> fermix_desktop::management::types::JobView {
    serde_json::from_value(serde_json::json!({
        "job_id": "job:2Kd9mQ", "kind": "capability_install", "status": "running",
        "phase": "sidecar_downloading", "progress": null, "budget_ms": 900000,
        "started_at": "2026-08-19T12:00:00Z", "finished_at": null,
        "result": null, "failure": null
    }))
    .expect("decodes")
}

// ---------------------------------------------------------------------------
// Home's own model
// ---------------------------------------------------------------------------

#[test]
fn the_home_model_snapshots_what_the_surface_draws() {
    let (model, _) = model("default", "active_aligned");
    let home = HomeModel::new(Rc::clone(&model));
    run(model.refresh_all());

    let snapshot = home.snapshot();

    assert_eq!(snapshot.status, StatusWord::Running);
    assert_eq!(snapshot.facts.len(), 9);
    assert!(snapshot.background_enabled);
    assert!(!home.is_polling(), "nothing polls until a surface asks");

    // Nothing is asserted here about opening at login: that is one file in one
    // directory this whole binary shares, and the test below owns it. A second
    // test asserting its value would pass or fail on which ran first.
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

/// The rows one scenario's providers make.
fn provider_rows(scenario: &str) -> Vec<ProviderRow> {
    let (model, _) = model(scenario, "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));
    run(providers.refresh());
    providers.rows()
}

/// One row, by provider id.
fn provider_row(rows: &[ProviderRow], id: &str) -> ProviderRow {
    rows.iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("{id} is not a published provider"))
        .clone()
}

/// What an OAuth provider offers once the daemon reports it working.
///
/// The owner signed in to Codex, the engine reported it configured with a
/// valid token, and the row went on offering Sign In — because the guard also
/// demanded the provider be primary or hold a key, and an OAuth provider that
/// was not signed in first is neither. A row that offers to sign you in to
/// something you are already signed in to is wrong whoever is primary, so the
/// matrix below pins every combination rather than the one case reported.
#[test]
fn a_signed_in_provider_offers_nothing_whoever_is_primary() {
    for (configured, primary, present_key, token_state, expected) in [
        // Signed in and working: nothing for the list to do, primary or not.
        (true, true, false, "valid", None),
        (true, false, false, "valid", None),
        (true, false, true, "valid", None),
        (true, true, true, "valid", None),
        // Not signed in: the door is offered.
        (false, false, false, "", Some(ProviderVerb::SignIn)),
        (false, true, false, "", Some(ProviderVerb::SignIn)),
        // Signed in but the token has gone stale: the door is offered again,
        // because a stale token is not a working provider.
        (true, true, false, "expired", Some(ProviderVerb::SignIn)),
        (true, false, false, "expired", Some(ProviderVerb::SignIn)),
    ] {
        let (model, peer) = model("default", "active_aligned");
        let mut state = peer.result("setup.state.get", None);
        for provider in state["providers"]
            .as_array_mut()
            .expect("the snapshot publishes providers")
        {
            if provider["id"] == "openai_codex" {
                provider["configured"] = serde_json::json!(configured);
                provider["primary"] = serde_json::json!(primary);
                provider["present_key"] = serde_json::json!(present_key);
                provider["token_state"] = if token_state.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(token_state)
                };
            }
        }
        peer.set_result("setup.state.get", None, state);

        let providers = ProvidersModel::new(Rc::clone(&model));
        run(providers.refresh());
        assert_eq!(
            provider_row(&providers.rows(), "openai_codex").verb,
            expected,
            "configured={configured} primary={primary} key={present_key} token={token_state}"
        );
    }
}

#[test]
fn a_provider_row_leads_with_a_verb_the_daemon_answers() {
    let rows = provider_rows("default");

    // The goldens report the Claude Code command line present on this machine,
    // so Anthropic's door is the sign-in it already holds rather than a browser
    // hop its auth_modes would suggest and auth.start would refuse.
    assert_eq!(
        provider_row(&rows, "anthropic").verb,
        Some(ProviderVerb::ImportClaudeCode)
    );

    // A key provider offers the key.
    assert_eq!(
        provider_row(&rows, "openrouter").verb,
        Some(ProviderVerb::AddKey)
    );

    // A provider that takes no credential offers nothing: a key verb on Ollama
    // is a button that can never be pressed.
    assert_eq!(provider_row(&rows, "ollama").verb, None);

    // A provider the daemon already reports as working has nothing for the list
    // to do; everything about it lives on its own page.
    assert_eq!(provider_row(&rows, "openai_codex").verb, None);
}

#[test]
fn a_verb_that_writes_a_secret_waits_for_the_slot_the_daemon_named() {
    let rows = provider_rows("default");
    let openai = provider_row(&rows, "openai");

    assert_eq!(openai.verb, Some(ProviderVerb::AddKey));
    assert_eq!(openai.secret_id.as_deref(), Some("openai_api_key"));
    assert!(openai.can_perform());

    // A provider whose section has not been read has no slot yet, so its verb
    // is not performable rather than pointed at a guessed id.
    let (model, _) = model("default", "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));
    run(model.refresh_setup());
    let unread = provider_row(&providers.rows(), "openai");

    assert_eq!(unread.secret_id, None);
    assert!(!unread.can_perform());
}

#[test]
fn the_status_words_are_the_ones_the_booleans_and_the_token_atom_mean() {
    let rows = provider_rows("default");

    assert_eq!(
        provider_row(&rows, "openai_codex").standing,
        ProviderStanding::Primary
    );
    assert_eq!(
        provider_row(&rows, "openai").standing,
        ProviderStanding::NotConnected
    );
    assert_eq!(
        provider_row(&rows, "openai_codex").account.as_deref(),
        Some("owner@example.com"),
        "the account beside the word is the daemon's own"
    );
}

#[test]
fn making_a_provider_primary_answers_with_the_daemons_own_side_effects() {
    let (model, peer) = model("default", "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));
    run(providers.refresh());

    let effects = run(providers.set_primary("anthropic")).expect("the daemon took it");

    assert_eq!(
        peer.last("providers.set_primary"),
        Some(serde_json::json!({"provider": "anthropic"}))
    );
    // The published golden changed nothing the operator did not type, and an
    // empty list is that answer rather than a missing one: what the surface
    // renders is whatever sentences the daemon put here, never its own.
    assert!(effects.is_empty());
    assert!(
        peer.count("setup.state.get") > 0,
        "the rows are read again, because the primary moved"
    );
}

#[test]
fn a_live_model_listing_that_fails_never_degrades_to_the_catalog() {
    let (model, peer) = model("default", "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));

    peer.refuse_next("providers.models.list", "unavailable");
    let refused = run(providers.models("openai", None, None, true));

    let sentence = refused.expect_err("a live listing that fails is a refusal");
    assert!(!sentence.text.is_empty(), "the daemon's own sentence");
    assert_eq!(
        peer.count("providers.models.list"),
        1,
        "nothing asks again without live, which would be the catalog wearing the live answer's clothes"
    );

    // The listing the row opens is the catalog, and it says so on the wire.
    let page = run(providers.models("openai", None, None, false)).expect("a page");
    assert_eq!(page.source, ModelSource::Catalog);
}

#[test]
fn the_probe_reports_the_daemons_own_model_and_latency() {
    let (model, peer) = model("default", "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));

    let started = run(providers.probe("anthropic")).expect("a job");
    assert_eq!(started.kind, JobKind::ProviderProbe);
    assert_eq!(
        peer.last("providers.probe.start"),
        Some(serde_json::json!({"provider": "anthropic"}))
    );

    run(providers.probe_job().read_once());
    let job = providers.probe_job().job().expect("the job was read");
    let sentence = probe_sentence(&job).expect("a finished probe says what it found");

    assert!(
        sentence.contains("812"),
        "the latency is the daemon's: {sentence}"
    );
    assert!(
        sentence.contains("claude-opus-5"),
        "the model is the daemon's: {sentence}"
    );
}

#[test]
fn signing_out_forgets_the_local_session_and_reads_the_rows_again() {
    let (model, peer) = model("default", "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));

    run(providers.sign_out("openai_codex")).expect("the daemon took it");

    assert_eq!(
        peer.last("auth.logout"),
        Some(serde_json::json!({"provider": "openai_codex"}))
    );
    assert!(
        peer.count("setup.state.get") > 0,
        "the rows are read again, because what it answers has changed"
    );
}

#[test]
fn a_sign_in_carries_the_address_the_daemon_returned_once() {
    let (model, _) = model("default", "active_aligned");
    let providers = ProvidersModel::new(Rc::clone(&model));

    let started = run(providers.start_sign_in("openai_codex")).expect("a sign-in");

    assert!(
        started.authorize_url.is_some(),
        "the address is shown where the browser did not open"
    );
    assert!(!started.imported);
    assert!(providers.sign_in_job().job().is_some());
}

// ---------------------------------------------------------------------------
// Integrations
// ---------------------------------------------------------------------------

/// A plugins model over one scenario.
fn plugins(scenario: &str) -> (Rc<PluginsModel>, Rc<FixturePeer>, Rc<SettingsModel>) {
    let (model, peer) = model(scenario, "active_aligned");
    let plugins = PluginsModel::new(Rc::clone(&model));
    run(model.refresh_setup());
    run(plugins.refresh());
    (plugins, peer, model)
}

#[test]
fn an_installed_row_reads_the_daemons_status_and_an_available_one_reads_the_summary() {
    let (plugins, _, _) = plugins("default");
    let rows = plugins.rows();

    let installed = rows
        .iter()
        .find(|row| row.name == "google_calendar")
        .expect("an installed plugin");
    let available = rows
        .iter()
        .find(|row| row.name == "obsidian")
        .expect("a plugin that is not installed");

    assert_eq!(installed.subtitle(), installed.status);
    assert_eq!(available.subtitle(), available.summary.as_deref().unwrap());
}

#[test]
fn the_four_filters_count_what_the_daemon_published() {
    let (plugins, _, _) = plugins("default");

    assert_eq!(plugins.count(IntegrationFilter::Installed), 2);
    assert_eq!(plugins.count(IntegrationFilter::Available), 2);
    assert_eq!(plugins.count(IntegrationFilter::Mcps), 2);
    assert_eq!(
        plugins.count(IntegrationFilter::Features),
        2,
        "two native drivers on this platform: computer history is macOS-only"
    );
}

#[test]
fn a_button_is_painted_with_the_word_beside_the_id_it_runs() {
    let (plugins, _, _) = plugins("default");
    let acme = plugins.row("acme").expect("acme is published");

    let buttons = acme.buttons();
    assert_eq!(
        buttons
            .first()
            .map(|(word, action)| (word.as_str(), *action)),
        Some(("Choose workspace", PluginAction::ChooseWorkspace)),
        "the word and the id travel together"
    );
    assert!(
        !buttons
            .iter()
            .any(|(_, action)| matches!(action, PluginAction::AddToken)),
        "the credential slot is the secret row, not a second button beside it"
    );
}

#[test]
fn the_sign_in_client_is_the_daemons_own_tie_rather_than_a_guess_from_the_name() {
    let (plugins, _, _) = plugins("default");

    let calendar = plugins.row("google_calendar").expect("published");
    assert_eq!(
        plugins.client_for(&calendar).map(|client| client.provider),
        Some("google".to_string())
    );

    let acme = plugins.row("acme").expect("published");
    assert!(
        plugins.client_for(&acme).is_none(),
        "a plugin with no sign-in family has no client row"
    );
}

#[test]
fn switching_on_something_installed_enables_it_and_opens_only_a_step_that_needs_a_person() {
    let (plugins, peer, _) = plugins("integrations_states");

    let outcome = run(plugins.set_enabled(true, "google_calendar"));

    assert_eq!(
        peer.last("plugins.enable"),
        Some(serde_json::json!({"name": "google_calendar"}))
    );
    assert_eq!(
        outcome,
        EnableOutcome::Done,
        "a row whose next step is a check the daemon runs itself opens nothing"
    );

    // A row the daemon leaves waiting for a person opens its own page.
    let waiting = plugins.next_step("github");
    assert!(
        matches!(waiting, EnableOutcome::Configure(row) if row.name == "github"),
        "a sign-in is a door the person walks through"
    );
}

#[test]
fn a_refused_stage_stops_the_chain_and_keeps_the_daemons_sentence() {
    let (plugins, peer, _) = plugins("default");

    peer.refuse_next("plugins.enable", "external_change");
    let outcome = run(plugins.set_enabled(true, "google_calendar"));

    match outcome {
        EnableOutcome::Refused(sentence) => assert!(!sentence.text.is_empty()),
        other => panic!("a refused enable is a refusal, not {other:?}"),
    }
    assert_eq!(
        peer.count("plugins.install.start"),
        0,
        "nothing else in the chain ran"
    );
}

#[test]
fn an_install_that_worked_is_followed_by_the_enable_the_switch_asked_for() {
    let (plugins, peer, _) = plugins("integrations_states");

    assert_eq!(
        run(plugins.perform(PluginAction::Install, "obsidian")),
        None,
        "the install started"
    );
    assert_eq!(
        peer.last("plugins.install.start"),
        Some(serde_json::json!({"name": "obsidian"}))
    );

    // The job finishes on the third read, which is what the scenario publishes.
    let runner = plugins.job();
    for _ in 0..3 {
        run(runner.read_once());
    }
    assert!(
        runner.is_terminal(),
        "the install reached a terminal outcome"
    );

    let outcome = run(plugins.install_finished("obsidian"));
    assert_eq!(
        peer.last("plugins.enable"),
        Some(serde_json::json!({"name": "obsidian"}))
    );
    assert_eq!(outcome, EnableOutcome::Done);
}

#[test]
fn a_plugin_setting_is_written_as_the_string_the_wire_takes() {
    let (plugins, peer, _) = plugins("default");

    run(plugins.set_setting("obsidian", "OBSIDIAN_VAULT_PATH", "/home/owner/vault"));

    assert_eq!(
        peer.last("plugins.setting.set"),
        Some(serde_json::json!({
            "name": "obsidian",
            "key": "OBSIDIAN_VAULT_PATH",
            "value": "/home/owner/vault"
        }))
    );
}

#[test]
fn a_sign_in_client_sends_its_region_only_where_the_daemon_offers_regions() {
    let (plugins, peer, _) = plugins("default");

    run(plugins.set_oauth_client("google", "1042.apps.googleusercontent.com", 1455, None));

    assert_eq!(
        peer.last("plugins.oauth_client.set"),
        Some(serde_json::json!({
            "provider": "google",
            "client_id": "1042.apps.googleusercontent.com",
            "redirect_port": 1455
        })),
        "a provider with one region is sent none"
    );
}

// ---------------------------------------------------------------------------
// Meetings
// ---------------------------------------------------------------------------

/// A meetings model over one scenario.
fn meetings(scenario: &str) -> (Rc<MeetingsModel>, Rc<FixturePeer>) {
    let (model, peer) = model(scenario, "active_aligned");
    let meetings = MeetingsModel::new(Rc::clone(&model));
    run(meetings.refresh());
    (meetings, peer)
}

#[test]
fn the_sign_in_state_is_the_probes_answer_and_absence_is_not_signed_out() {
    let (signed_in, _) = meetings("default");
    assert!(matches!(
        signed_in.sign_in_state(),
        SignInState::SignedIn(Some(_))
    ));

    let (signed_out, _) = meetings("meetings_signed_out");
    assert!(matches!(
        signed_out.sign_in_state(),
        SignInState::SignedOut(Some(_))
    ));

    let (absent, _) = meetings("meetings_absent");
    assert_eq!(
        absent.sign_in_state(),
        SignInState::Unanswered,
        "an absent notetaker is not an account that signed out"
    );

    let (refused, _) = meetings("meetings_refused");
    assert_eq!(
        refused.sign_in_state(),
        SignInState::Unanswered,
        "a refused probe claims nothing either way"
    );
}

#[test]
fn turning_meetings_on_installs_the_notetaker_before_it_writes_anything() {
    let (meetings, peer) = meetings("meetings_absent");

    run(meetings.enable()).expect("the install started");

    assert_eq!(
        peer.last("capabilities.install.start"),
        Some(serde_json::json!({"target": "meetbot"}))
    );
    assert_eq!(
        peer.count("settings.apply"),
        0,
        "a switch reading on over an install that has not finished is a switch with nothing behind it"
    );
}

#[test]
fn an_install_that_failed_writes_nothing_and_keeps_the_daemons_sentence() {
    let (meetings, peer) = meetings("meetings_absent");
    run(meetings.enable()).expect("the install started");

    // The daemon's own failed capability install, out of the published goldens.
    meetings.install_job().adopt(failed_install());
    let refused = run(meetings.install_finished()).expect_err("a failed install is a refusal");

    assert!(!refused.text.is_empty(), "the daemon's own sentence");
    assert_eq!(peer.count("settings.apply"), 0, "nothing was written");
}

#[test]
fn the_notetaker_is_probed_again_after_a_sign_in_reaches_any_outcome() {
    let (meetings, peer) = meetings("default");
    let before = peer.count("setup.detect");

    run(meetings.sign_in_finished());

    assert_eq!(
        peer.count("setup.detect"),
        before + 1,
        "job success is never taken for an account"
    );
}

// ---------------------------------------------------------------------------
// Computer, and the one ledger
// ---------------------------------------------------------------------------

#[test]
fn the_two_verdicts_and_the_time_are_the_daemons_own() {
    let (model, _) = model("default", "active_aligned");
    let ledger = PermissionLedger::new(Rc::clone(&model));
    run(ledger.refresh());

    assert_eq!(ledger.holds(Right::ScreenCapture), Some(true));
    assert_eq!(ledger.holds(Right::InputSynthesis), Some(false));
    assert_eq!(ledger.probed_at().as_deref(), Some("2026-08-19T12:00:00Z"));
}

#[test]
fn a_probe_nobody_has_taken_claims_nothing() {
    let (model, _) = model("default", "active_aligned");
    let ledger = PermissionLedger::new(Rc::clone(&model));

    assert_eq!(ledger.holds(Right::ScreenCapture), None);
    assert_eq!(ledger.probed_at(), None);
}

#[test]
fn the_ledger_is_the_seven_rights_the_table_names() {
    let (model, _) = model("default", "active_aligned");
    let ledger = PermissionLedger::new(Rc::clone(&model));

    let rows = ledger.rows();
    assert_eq!(rows.len(), 7);

    // A right with nothing kept has nothing to report, which is not "off".
    let microphone = ledger.row(Right::Microphone);
    assert_eq!(microphone.standing, None);

    // The two the helper holds are the probe's answer.
    run(ledger.refresh());
    assert!(ledger.row(Right::ScreenCapture).standing.is_some());
}

#[test]
fn the_computer_pane_reads_the_helpers_state_from_the_probe_and_not_from_the_session() {
    let (model, _) = model("default", "active_aligned");
    let ledger = PermissionLedger::new(Rc::clone(&model));
    let computer = ComputerModel::new(Rc::clone(&model), Rc::clone(&ledger));

    assert_eq!(
        computer.standing(),
        Standing::Unread,
        "nothing is said before the probe has been taken"
    );

    run(ledger.refresh());
    assert_eq!(
        computer.standing(),
        Standing::Installed,
        "the daemon says the helper is installed, whatever this session is"
    );
}

#[test]
fn a_machine_with_no_helper_installed_says_why_rather_than_offering_nothing() {
    let (model, _) = model("computer_states", "active_aligned");
    let ledger = PermissionLedger::new(Rc::clone(&model));
    let computer = ComputerModel::new(Rc::clone(&model), ledger);
    run(computer.refresh());

    let standing = computer.standing();
    assert_ne!(standing, Standing::Installed);
    assert!(
        standing.statement().is_some(),
        "unavailable is a finished sentence with a reason"
    );
    assert_eq!(
        standing.offers_install(),
        matches!(standing, Standing::Installable),
        "a machine the helper cannot run on is offered no install"
    );
}

/// The daemon's own failed capability install, from the published goldens.
fn failed_install() -> fermix_desktop::management::types::JobView {
    serde_json::from_value(serde_json::json!({
        "job_id": "job:9Mn0pQ", "kind": "capability_install", "status": "failed",
        "phase": "verifying", "progress": null, "budget_ms": 900000,
        "started_at": "2026-08-19T12:00:00.317045Z",
        "finished_at": "2026-08-19T12:00:09.000000Z",
        "result": null,
        "failure": {"code": "unavailable", "sentence": "The install did not finish: :enospc."}
    }))
    .expect("decodes")
}

// ---------------------------------------------------------------------------
// The Setup assistant
// ---------------------------------------------------------------------------

/// One assistant, and the main context it runs inside.
///
/// Its own context rather than the process's: the assistant starts work of its
/// own, a test binary runs its tests on several threads, and a future on the
/// shared default context would be run by whichever thread happened to be
/// iterating it. The toolkit's values refuse that, loudly.
struct Assistant {
    onboarding: Rc<OnboardingModel>,
    settings: Rc<SettingsModel>,
    peer: Rc<FixturePeer>,
    context: MainContext,
}

impl Assistant {
    /// Await one of the model's own operations.
    fn run<F: std::future::Future>(&self, future: F) -> F::Output {
        self.context.block_on(future)
    }

    /// Read everything the window reads before it draws, which is what the
    /// assistant's own routing is decided on.
    fn load(&self) {
        self.run(self.settings.refresh_all());
    }

    /// Turn the context until something holds or a bound is reached, so a test
    /// never waits on what is not coming.
    fn pump(&self, done: impl Fn() -> bool, turns: u32) {
        for _ in 0..turns {
            if done() {
                return;
            }
            self.context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn snapshot(&self) -> fermix_desktop::models::onboarding::Snapshot {
        self.onboarding.snapshot()
    }

    fn stage(&self) -> Stage {
        self.onboarding.stage()
    }
}

/// An assistant over one scenario's goldens and one command line state, inside
/// a budget short enough to prove a wait rather than sit in one, with its own
/// context pushed for as long as `body` runs.
fn assistant(scenario: &str, cli_state: &str, body: impl FnOnce(&Assistant)) {
    let context = MainContext::new();
    let (settings, peer) = model(scenario, cli_state);

    context
        .clone()
        .with_thread_default(|| {
            let activation = Activation::with_budget(Rc::clone(&settings), Duration::from_secs(2));
            let assistant = Assistant {
                onboarding: OnboardingModel::with_activation(Rc::clone(&settings), activation),
                settings,
                peer,
                context,
            };
            body(&assistant);
        })
        .expect("the context is this thread's default while the test runs");
}

/// The same, with everything the window reads already read.
fn loaded(scenario: &str, cli_state: &str, body: impl FnOnce(&Assistant)) {
    assistant(scenario, cli_state, |assistant| {
        assistant.load();
        body(assistant);
    });
}

#[test]
fn the_assistant_opens_on_welcome_while_nothing_is_answering() {
    assistant("not_running", "bound_disabled", |assistant| {
        // Nothing on the socket, which is the account a fresh package meets.
        assistant.peer.set_running(false);
        assistant.load();
        assistant.onboarding.resume();

        let snapshot = assistant.snapshot();
        assert_eq!(snapshot.stage, Stage::Welcome);
        assert_eq!(snapshot.primary, Some(Primary::Begin));
        assert_eq!(snapshot.progress, Some(0));
    });
}

#[test]
fn the_assistant_opens_at_the_first_gap_the_daemon_reported() {
    for (scenario, expected) in [
        ("onboarding_connect_ai", Stage::ConnectAi),
        ("onboarding_about_you", Stage::AboutYou),
        ("onboarding_restart_needed", Stage::Applying),
        ("onboarding_ready", Stage::Ready),
    ] {
        loaded(scenario, "active_aligned", |assistant| {
            assistant.onboarding.resume();
            assert_eq!(
                assistant.stage(),
                expected,
                "{scenario} opens on the screen its readiness owes"
            );
        });
    }
}

#[test]
fn the_ladder_runs_every_step_against_a_daemon_that_answers() {
    let door = health_door();

    assistant("onboarding_connect_ai", "install_ok", |assistant| {
        assistant
            .peer
            .set_result("hello", None, hello_on(&door.origin));
        assistant.load();

        assistant.onboarding.begin();
        assistant.pump(|| assistant.stage() != Stage::Starting, 400);

        let snapshot = assistant.snapshot();
        assert!(
            snapshot
                .steps
                .iter()
                .all(|(_, state)| *state == StepState::Done),
            "every row of the ladder is done: {:?}",
            snapshot.steps
        );
        assert_eq!(snapshot.failure, None);
        assert_eq!(
            snapshot.stage,
            Stage::ConnectAi,
            "a home with no provider lands on the one required decision"
        );
    });

    assert_eq!(door.requests(), 1, "the web door was asked exactly once");
}

#[test]
fn a_web_door_that_never_answers_is_a_named_state_inside_the_budget() {
    assistant("onboarding_connect_ai", "install_ok", |assistant| {
        // Port zero is not a port: the operating system refuses the connect
        // without a packet leaving this machine, which is the state where
        // nothing is accepting on the origin at all. Nothing here binds
        // anything, so no test and no capture can be held up by whatever else
        // is listening on this host.
        assistant
            .peer
            .set_result("hello", None, hello_on("http://127.0.0.1:0"));
        assistant.load();

        assistant.onboarding.begin();
        assistant.pump(|| assistant.stage() == Stage::BootFailed, 1_200);

        let failure = assistant.snapshot().failure.expect("a named refusal");
        assert_eq!(failure.title, Key::ActivationWebUnavailableTitle);
        assert!(!failure.before_mutation, "the install had already run");
        assert!(
            failure.actions.contains(&Action::RunDoctor)
                && failure.actions.contains(&Action::ViewLog),
            "the state the design names offers Doctor and the log"
        );
    });
}

#[test]
fn a_port_something_else_is_holding_is_a_different_named_state() {
    // A door of this test's own, on a port the operating system chose: it
    // accepts and then says nothing, which is what something else holding the
    // daemon's port looks like from here. The two states read differently to a
    // person and the assistant keeps them apart.
    let door = silent_door();

    assistant("onboarding_connect_ai", "install_ok", |assistant| {
        assistant
            .peer
            .set_result("hello", None, hello_on(&door.origin));
        assistant.load();

        assistant.onboarding.begin();
        assistant.pump(|| assistant.stage() == Stage::BootFailed, 1_200);

        let failure = assistant.snapshot().failure.expect("a named refusal");
        assert_eq!(failure.title, Key::ActivationBindFailureTitle);
        assert!(!failure.before_mutation, "the install had already run");
        assert!(
            door.requests() > 0,
            "the gate asked the door this test opened rather than anything on this host"
        );
    });
}

#[test]
fn a_refused_install_lands_on_the_state_its_own_code_names() {
    loaded("onboarding_connect_ai", "linger_denied", |assistant| {
        assistant.onboarding.begin();
        assistant.pump(|| assistant.stage() == Stage::BootFailed, 400);

        let snapshot = assistant.snapshot();
        let failure = snapshot.failure.expect("a named refusal");

        assert_eq!(failure.title, Key::LingerDeniedTitle);
        assert_eq!(
            failure.body,
            copy::text(Key::LingerDeniedBody),
            "the Linux moment the design fixes"
        );
        assert!(
            failure
                .sentence
                .as_deref()
                .is_some_and(|sentence| sentence.contains("linger")),
            "the command line's own words travel with it: {:?}",
            failure.sentence
        );
        assert!(
            failure
                .command
                .as_ref()
                .is_some_and(|(_, command)| command.starts_with("sudo loginctl enable-linger")),
            "the one command that grants it"
        );
        assert_eq!(
            failure.actions,
            vec![Action::CopyCommand, Action::TryAgain],
            "the two buttons the design names"
        );
        assert!(
            !failure.evidence.is_empty(),
            "the offline boot evidence the command line collected"
        );
        assert_eq!(
            snapshot
                .steps
                .iter()
                .find(|(step, _)| *step == Step::EnableLinger)
                .map(|(_, state)| *state),
            Some(StepState::Failed),
            "the ladder stopped on the row the code names"
        );
    });
}

#[test]
fn a_host_with_no_login_manager_is_offered_no_command_to_run() {
    loaded("onboarding_connect_ai", "loginctl_absent", |assistant| {
        assistant.onboarding.begin();
        assistant.pump(|| assistant.stage() == Stage::BootFailed, 400);

        let failure = assistant.snapshot().failure.expect("a named refusal");
        assert_eq!(failure.title, Key::LoginManagerAbsentTitle);
        assert_eq!(failure.next_action, Some(Key::LoginManagerAbsentNextAction));
        assert_eq!(
            failure.command, None,
            "a host that cannot be helped by a command is offered none"
        );
        assert!(failure.actions.contains(&Action::Cancel));
    });
}

#[test]
fn the_preflight_refuses_before_anything_is_written() {
    for (state, title) in [
        // The first two are refusals the status read answers with; the
        // third is a fact it publishes; the fourth is its own verdict.
        ("user_manager_unreachable", Key::PreflightNoUserManagerTitle),
        (
            "foreign_distribution",
            Key::PreflightForeignDistributionTitle,
        ),
        ("foreign_unit", Key::PreflightForeignDaemonTitle),
        ("pending_restart", Key::PreflightEngineSkewTitle),
    ] {
        assistant("onboarding_connect_ai", state, |assistant| {
            assistant.onboarding.begin();
            assistant.pump(|| assistant.stage() == Stage::BootFailed, 400);

            let failure = assistant.snapshot().failure.expect("a named refusal");
            assert_eq!(failure.title, title, "{state}");
            assert!(
                failure.before_mutation,
                "{state} is refused before anything is written"
            );
            assert!(
                failure.evidence.is_empty(),
                "{state} has no boot to explain"
            );
        });
    }
}

#[test]
fn the_ladder_can_only_be_left_by_stopping_it() {
    loaded("onboarding_connect_ai", "install_ok", |assistant| {
        assistant.onboarding.begin();

        let snapshot = assistant.snapshot();
        assert_eq!(snapshot.stage, Stage::Starting);
        assert_eq!(snapshot.leading, Some(Leading::Cancel));
        assert_eq!(snapshot.primary, None, "there is no decision on the ladder");
        assert!(
            !Leading::Cancel.answers_escape(),
            "stopping a transaction is a press rather than a stray key"
        );

        let left = Rc::new(std::cell::Cell::new(false));
        let watched = Rc::clone(&left);
        assistant.onboarding.on_leave(move || watched.set(true));
        assistant.onboarding.cancel();

        assert!(left.get(), "cancelling leaves the assistant");
        assistant.pump(|| false, 40);
    });
}

#[test]
fn connecting_an_ai_is_the_one_decision_nothing_walks_past() {
    loaded("onboarding_connect_ai", "active_aligned", |assistant| {
        assistant.onboarding.resume();
        assert_eq!(assistant.stage(), Stage::ConnectAi);

        assistant.onboarding.advance();

        assert_eq!(
            assistant.stage(),
            Stage::ConnectAi,
            "the screen holds until the daemon says a provider is connected"
        );
        assert_eq!(assistant.snapshot().block, Some(Block::Provider));
        assert!(assistant.snapshot().provider_gap);
    });
}

#[test]
fn a_refused_personalization_write_stays_on_about_you_with_the_daemons_sentence() {
    loaded(
        "onboarding_refused_personalization",
        "active_aligned",
        |assistant| {
            assistant.onboarding.resume();
            assert_eq!(assistant.stage(), Stage::AboutYou);

            assistant.onboarding.start_applying(true);
            assistant.pump(|| assistant.snapshot().refusal.is_some(), 200);

            let snapshot = assistant.snapshot();
            assert_eq!(snapshot.stage, Stage::AboutYou);
            assert_eq!(
                snapshot.refusal.map(|sentence| sentence.text),
                Some("That time zone is not one this computer knows.".to_string()),
                "the daemon's own sentence, not a word of ours"
            );
        },
    );
}

#[test]
fn a_home_that_needs_no_restart_never_draws_the_restart_row() {
    loaded("onboarding_no_restart", "active_aligned", |assistant| {
        assistant.onboarding.start_applying(true);
        assistant.pump(|| assistant.snapshot().block.is_some(), 200);

        let snapshot = assistant.snapshot();
        let rows: Vec<ApplyStep> = snapshot.applying.iter().map(|(step, _)| *step).collect();

        assert_eq!(
            rows,
            vec![ApplyStep::Save],
            "one row, because one thing ran"
        );
        assert_eq!(snapshot.applying[0].1, StepState::Done);
        assert_eq!(
            snapshot.stage,
            Stage::Applying,
            "a gap the assistant has no screen for keeps the person here"
        );
        assert!(matches!(snapshot.block, Some(Block::Elsewhere(_))));
    });
}

#[test]
fn a_step_that_finished_keeps_its_tick_when_the_restart_row_joins_the_ladder() {
    loaded(
        "onboarding_restart_needed",
        "restart_refused",
        |assistant| {
            // The write, then the restart the daemon says it owes. The restart
            // row joins a ladder that was built for the write, and the write's
            // own row must still read as the thing that finished.
            assistant.onboarding.start_applying(true);
            assistant.pump(|| assistant.snapshot().refusal.is_some(), 200);

            let snapshot = assistant.snapshot();
            assert_eq!(
                snapshot.applying,
                vec![
                    (ApplyStep::Save, StepState::Done),
                    (ApplyStep::Restart, StepState::Failed),
                ],
                "a rebuilt ladder forgets the step that already ran"
            );
        },
    );
}

#[test]
fn a_home_that_owes_a_restart_takes_it_without_writing_anything_again() {
    loaded(
        "onboarding_restart_needed",
        "restart_refused",
        |assistant| {
            assistant.onboarding.resume();
            assistant.pump(|| assistant.snapshot().refusal.is_some(), 200);

            let snapshot = assistant.snapshot();
            assert_eq!(snapshot.stage, Stage::Applying);
            assert_eq!(
                snapshot
                    .applying
                    .iter()
                    .map(|(step, _)| *step)
                    .collect::<Vec<ApplyStep>>(),
                vec![ApplyStep::Restart],
                "a restart-only entry writes the personalization nobody changed \
                 again, so it draws no row for a write it will never make"
            );
            assert_eq!(snapshot.applying[0].1, StepState::Failed);
            assert!(
                snapshot
                    .refusal
                    .is_some_and(|sentence| sentence.text.contains("would not open a window")),
                "a refused restart stays visible with the command line's own cause"
            );
            assert_eq!(
                snapshot.primary,
                Some(Primary::Retry),
                "and with the one action that tries it again"
            );
        },
    );
}

#[test]
fn the_finish_gate_is_the_daemons_own() {
    loaded("onboarding_connect_ai", "active_aligned", |assistant| {
        assert_eq!(assistant.onboarding.blocked(), Some(Block::Provider));
    });
    loaded("onboarding_restart_needed", "active_aligned", |assistant| {
        assert_eq!(assistant.onboarding.blocked(), Some(Block::Restart));
    });
    loaded("onboarding_ready", "active_aligned", |assistant| {
        assert_eq!(
            assistant.onboarding.blocked(),
            None,
            "nothing gating and nothing to restart"
        );
    });
    assistant("not_running", "bound_disabled", |assistant| {
        assistant.peer.set_running(false);
        assistant.load();
        assert_eq!(assistant.onboarding.blocked(), Some(Block::DaemonNotLive));
    });
}

#[test]
fn closing_the_assistant_marks_nothing_complete() {
    loaded("onboarding_connect_ai", "active_aligned", |assistant| {
        assistant.onboarding.resume();
        assistant.onboarding.leave();

        assert_eq!(
            status_word(&assistant.settings.state()),
            StatusWord::SetupRequired,
            "Home goes on saying what the daemon says"
        );
        assert_eq!(
            toolbar_action(&assistant.settings.state()),
            Some(ToolbarAction::ContinueSetup)
        );
    });
}

#[test]
fn the_home_a_person_chose_is_held_here_and_written_by_the_command_line() {
    loaded("onboarding_connect_ai", "install_ok", |assistant| {
        assistant
            .onboarding
            .choose_home(std::path::Path::new("/home/person/somewhere-else"));

        assert_eq!(
            assistant.onboarding.home(),
            Some(std::path::PathBuf::from("/home/person/somewhere-else"))
        );
    });
}

#[test]
fn the_four_answers_are_filled_from_the_daemon_before_anybody_types() {
    loaded("onboarding_about_you", "active_aligned", |assistant| {
        assistant.run(assistant.settings.refresh_section("personalization"));
        let rows = assistant.settings.state().rows("personalization");
        assistant.onboarding.prefill(&rows);

        let answers = assistant.onboarding.answers();
        assert_eq!(answers.assistant, "fermix", "the daemon's own value");
        assert!(
            !answers.timezone.is_empty() && !answers.style.is_empty(),
            "a blank field is filled from this computer rather than left empty: {answers:?}"
        );
    });
}

// ---------------------------------------------------------------------------
// Version skew
// ---------------------------------------------------------------------------

#[test]
fn the_skew_rows_are_the_alignment_the_command_line_published() {
    for (state, title, action) in [
        (
            "pending_restart",
            Some(Key::SkewNewerInstalledTitle),
            Some(AttentionAction::FinishUpdating),
        ),
        ("active_aligned", None, None),
    ] {
        let (model, _) = model("default", state);
        run(model.refresh_service());

        let rows = skew_rows(&model.state(), GuiAlignment::Unknown);
        match title {
            Some(title) => {
                assert_eq!(rows.len(), 1, "{state}");
                assert_eq!(rows[0].title, RowTitle::Words(title));
                assert_eq!(rows[0].action, action);
                assert!(rows[0].detail.is_some(), "the row says what it means");
            }
            None => assert!(rows.is_empty(), "{state} needs nothing"),
        }
    }
}

#[test]
fn an_identity_nobody_could_establish_stays_unknown_rather_than_aligned() {
    // A fresh account reports `not_running`, which is not a claim about builds.
    let (fresh, _) = model("default", "fresh");
    run(fresh.refresh_service());
    assert!(skew_rows(&fresh.state(), GuiAlignment::Unknown).is_empty());

    let rows = rows_for("unknown_identity");
    assert_eq!(
        rows[0].title,
        RowTitle::Words(Key::SkewUnknownIdentityTitle)
    );
    assert_eq!(
        rows[0].action, None,
        "nothing is claimed and nothing offered"
    );
}

#[test]
fn an_ownership_conflict_refuses_the_lifecycle_rather_than_offering_one() {
    let rows = rows_for("ownership_conflict");
    assert_eq!(
        rows[0].title,
        RowTitle::Words(Key::SkewOwnershipConflictTitle)
    );
    assert_eq!(rows[0].action, None);
}

#[test]
fn a_window_older_than_the_application_installed_asks_to_be_reopened() {
    let (model, _) = model("default", "active_aligned");
    run(model.refresh_service());

    let rows = skew_rows(&model.state(), GuiAlignment::Stale);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, RowTitle::Words(Key::SkewStaleGuiTitle));
    assert_eq!(
        rows[0].action,
        Some(AttentionAction::Quit),
        "this process cannot reopen itself, and the row says so"
    );

    assert!(
        skew_rows(&model.state(), GuiAlignment::Unknown).is_empty(),
        "an unknown identity claims nothing"
    );
}

/// The skew rows for one command line state, read the way the window reads them.
fn rows_for(cli_state: &str) -> Vec<AttentionRow> {
    let (model, _) = model("default", cli_state);
    run(model.refresh_service());

    let rows = skew_rows(&model.state(), GuiAlignment::Unknown);
    assert!(!rows.is_empty(), "an alignment that is not aligned says so");
    rows
}

/// A `hello` answering one origin, which is how a test stands a web door up and
/// names it.
fn hello_on(origin: &str) -> serde_json::Value {
    let goldens = fermix_desktop::fixtures::Goldens::load("default").expect("loads");
    let mut hello = goldens.answer(&serde_json::json!({
        "request_id": "test",
        "protocol_version": 2,
        "method": "hello",
        "params": {}
    }))["result"]
        .clone();
    hello["setup"]["origin"] = serde_json::json!(origin);
    hello
}

/// A loopback door that answers the one request the finish gate makes.
struct HealthDoor {
    origin: String,
    asked: Arc<AtomicUsize>,
}

impl HealthDoor {
    fn requests(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }
}

/// A door that accepts and then answers nothing, for as long as the caller is
/// willing to wait.
///
/// The connections are held rather than dropped: a closed socket is a refusal,
/// and the state this stands for is a web server that took the request and
/// never replied.
fn silent_door() -> HealthDoor {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port");
    let port = listener.local_addr().expect("an address").port();
    let asked = Arc::new(AtomicUsize::new(0));

    let counted = Arc::clone(&asked);
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for connection in listener.incoming().flatten() {
            counted.fetch_add(1, Ordering::SeqCst);
            held.push(connection);
        }
    });

    HealthDoor {
        origin: format!("http://127.0.0.1:{port}"),
        asked,
    }
}

/// Stand one up on a port the operating system chooses, so two tests running at
/// once never meet on one port and nothing on this host is asked anything.
fn health_door() -> HealthDoor {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port");
    let port = listener.local_addr().expect("an address").port();
    let asked = Arc::new(AtomicUsize::new(0));

    let counted = Arc::clone(&asked);
    std::thread::spawn(move || {
        // One request, which is what the gate makes. The thread ends with it.
        if let Ok((mut connection, _)) = listener.accept() {
            use std::io::{Read, Write};
            let mut request = [0u8; 1024];
            let _ = connection.read(&mut request);
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = connection.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 0\r\n\r\n");
        }
    });

    HealthDoor {
        origin: format!("http://127.0.0.1:{port}"),
        asked,
    }
}

// ---------------------------------------------------------------------------
// The secret store, and which of three situations the owner is in
// ---------------------------------------------------------------------------

/// The reason survives the trip from the wire to the surface that routes on it.
///
/// `Sentence` carried only a code and its words, so the one field that
/// separates a locked keyring from an absent one was dropped before any
/// dialog could read it. That is why the old surface could not tell them
/// apart, whatever its wording said.
#[test]
fn a_store_refusal_carries_the_reason_the_engine_gave() {
    let wire = ManagementError::Wire(store_failure(serde_json::json!({ "reason": "locked" })));

    let sentence = Sentence::of(&wire);

    assert_eq!(sentence.code.as_deref(), Some("secret_store_failed"));
    assert_eq!(sentence.reason.as_deref(), Some("locked"));
}

/// A refusal with no reason is not invented into one.
#[test]
fn a_store_refusal_without_a_reason_claims_none() {
    let wire = ManagementError::Wire(store_failure(serde_json::json!({})));

    assert_eq!(Sentence::of(&wire).reason, None);
}

/// Each published reason routes to its own surface.
///
/// The three are the engine's closed set. `timeout` is deliberately not the
/// locked dialog: a helper that hung while the collection was unlocked has
/// nothing to unlock, so offering an unlock would be a button that cannot
/// work.
#[test]
fn each_published_reason_routes_to_its_own_surface() {
    let cases = [
        ("locked", Some(StoreRefusal::KeyringLocked)),
        ("unavailable", Some(StoreRefusal::NoKeyring)),
        ("timeout", Some(StoreRefusal::HelperDidNotAnswer)),
    ];

    for (reason, expected) in cases {
        let sentence = store_sentence(reason);
        assert_eq!(
            StoreRefusal::of(&sentence),
            expected,
            "{reason} routed somewhere unexpected"
        );
    }
}

/// A reason this build does not know is not guessed at.
///
/// A future engine may publish a fourth word. Routing it to the locked dialog
/// would offer an unlock for a state we know nothing about, so an unknown
/// reason falls through to the generic refusal the row already shows.
#[test]
fn an_unknown_reason_is_not_guessed_into_a_dialog() {
    assert_eq!(StoreRefusal::of(&store_sentence("moon_phase")), None);
    assert_eq!(
        StoreRefusal::of(&Sentence {
            code: Some("secret_store_failed".into()),
            text: "The key was not saved.".into(),
            reason: None,
        }),
        None
    );
}

/// Only the store refusal routes here.
#[test]
fn another_refusal_carrying_a_reason_is_not_a_store_refusal() {
    let sentence = Sentence {
        code: Some("invalid_params".into()),
        text: "A secret cannot be empty.".into(),
        reason: Some("locked".into()),
    };

    assert_eq!(StoreRefusal::of(&sentence), None);
}

/// Only the locked keyring can be unlocked.
///
/// This is the property the dialogs are built on: the unlock action exists on
/// exactly one of the three surfaces.
#[test]
fn the_unlock_is_offered_only_where_there_is_something_to_unlock() {
    assert!(StoreRefusal::KeyringLocked.offers_unlock());
    assert!(!StoreRefusal::NoKeyring.offers_unlock());
    assert!(!StoreRefusal::HelperDidNotAnswer.offers_unlock());
}

/// Where values live, as the engine now names it.
///
/// This is the first store kind the engine has ever published, so there is no
/// earlier spelling to accept and `pass` was never one of them.
#[test]
fn the_store_kind_reads_the_three_words_the_engine_publishes() {
    assert_eq!(StoreKind::of("keyring"), Some(StoreKind::Keyring));
    assert_eq!(StoreKind::of("file"), Some(StoreKind::File));
    // Two values, not three. No response can carry "none": a refused save
    // answers an error and has no result to put a store in, and a home that
    // can store nothing still SAVES TO the keyring, the save simply refuses.
    // "Cannot store now" is the availability question, not this one.
    assert_eq!(StoreKind::of("none"), None);
    assert_eq!(StoreKind::of("pass"), None);
}

/// Whether a secret can be stored right now, which is a different question.
///
/// Answering both with one enum is the conflation this work removes: a locked
/// keyring reported as `none` is exactly the untrue message the owner met.
#[test]
fn availability_is_a_separate_question_from_where_values_live() {
    assert_eq!(Availability::of("ready"), Some(Availability::Ready));
    assert_eq!(Availability::of("locked"), Some(Availability::Locked));
    assert_eq!(
        Availability::of("unavailable"),
        Some(Availability::Unavailable)
    );
    assert_eq!(Availability::of("none"), None);

    // A locked keyring is not an absent one, on either field.
    assert_ne!(
        Availability::Locked.to_string(),
        Availability::Unavailable.to_string()
    );
}

fn store_sentence(reason: &str) -> Sentence {
    Sentence {
        code: Some("secret_store_failed".into()),
        text: "The key was not saved.".into(),
        reason: Some(reason.into()),
    }
}

/// The engine's own refusal shape, with whatever details the case carries.
fn store_failure(details: serde_json::Value) -> WireError {
    WireError {
        code: "secret_store_failed".into(),
        message: "The key was not saved.".into(),
        sentence: None,
        details: details.as_object().cloned().unwrap_or_default(),
    }
}

/// The ordinary save is unchanged on the wire.
///
/// Both new parameters are optional, so an engine that predates them sees
/// exactly what it saw before. The existing test that pins the default shape
/// is what guards this; this one says why it matters.
#[test]
fn an_ordinary_save_sends_neither_new_parameter() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("realtime"));

    run(model.set_secret("realtime", "openai_api_key", "sk-not-a-real-key".into())).unwrap();

    let sent = peer.last("secret.set").expect("the save was sent");
    assert!(
        sent.get("store").is_none() && sent.get("unlock").is_none(),
        "an ordinary save carries a choice nobody made: {sent}"
    );
}

/// Choosing the file store is carried in the request, per call.
///
/// The consent IS the parameter: there is no persisted flag the app sets and
/// no automatic fallback, so a save that reaches the file store can only have
/// come from an owner who pressed the button that says so.
#[test]
fn storing_on_this_computer_carries_the_consent_in_the_request() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("realtime"));

    run(model.store_secret_on_this_computer(
        "realtime",
        "openai_api_key",
        "sk-not-a-real-key".into(),
    ))
    .unwrap();

    let sent = peer.last("secret.set").expect("the save was sent");
    assert_eq!(sent.get("store").and_then(|v| v.as_str()), Some("file"));
    assert!(
        sent.get("unlock").is_none_or(|v| v == false),
        "the file store does not wait on a keyring: {sent}"
    );
}

/// Retrying asks the engine to wait for the owner to unlock.
#[test]
fn retrying_after_the_unlock_asks_the_engine_to_wait() {
    let (model, peer) = model("default", "active_aligned");
    run(model.refresh_section("realtime"));

    run(model.retry_secret_after_unlock("realtime", "openai_api_key", "sk-not-a-real-key".into()))
        .unwrap();

    let sent = peer.last("secret.set").expect("the retry was sent");
    assert_eq!(sent.get("unlock").and_then(|v| v.as_bool()), Some(true));
    assert!(
        sent.get("store").is_none_or(|v| v == "keyring"),
        "the retry is for the keyring, not the file: {sent}"
    );
}

/// The unlock deadline outlives the engine's cap, with room to spare.
///
/// This is the whole of the agreement with the engine: its cap must expire
/// first, so the owner reads the engine's typed reason rather than this
/// application's timeout. A 20-second write deadline would cut the owner off
/// mid-password, which is worse than the bug being fixed, because they would
/// be doing exactly what they were asked.
#[test]
fn the_unlock_deadline_outlives_the_engines_cap() {
    let cap = Duration::from_secs(90);

    assert!(
        UNLOCK_DEADLINE > cap,
        "the engine's cap would outlive this deadline and the owner would see our timeout"
    );
    assert!(
        UNLOCK_DEADLINE - cap >= Duration::from_secs(10),
        "there is no room between the cap and the deadline for the answer to arrive"
    );
    assert!(
        WRITE_DEADLINE < cap,
        "the ordinary write deadline should stay short; only the unlock waits"
    );
}

/// The way home moves what the engine already holds.
///
/// The owner cannot retype a value the application cannot read, so the verb
/// takes no value at all.
#[test]
fn the_way_back_to_the_keyring_needs_no_secret_from_the_owner() {
    let (model, peer) = model("default", "active_aligned");

    let _ = run(model.migrate_to_keyring(true));

    let sent = peer
        .last("secret.migrate_to_keyring")
        .expect("the verb was sent");
    assert_eq!(sent.get("unlock").and_then(|v| v.as_bool()), Some(true));
    assert!(
        sent.get("value").is_none() && sent.get("id").is_none(),
        "the migration asked for a secret it cannot have: {sent}"
    );
}

/// Consenting to the file store leaves the row telling the truth.
///
/// The owner's one deliberate choice about where their key lives is the worst
/// possible moment for the row to keep its previous answer. The write succeeds
/// and the engine's next snapshot says `file`, so the model has to go and read
/// it: a notify alone redraws the pane from the cached snapshot, which is the
/// old store, and the row then states the opposite of what just happened.
#[test]
fn storing_on_this_computer_leaves_the_row_saying_so() {
    let (model, peer) = model("default", "active_aligned");

    run(model.refresh_setup());
    assert_eq!(model.secret_store_kind(), Some(StoreKind::Keyring));

    // What the engine will say once the value is in the file store.
    let mut state = peer.result("setup.state.get", None);
    state["secrets"] = serde_json::json!({"store": "file", "availability": "ready"});
    peer.set_result("setup.state.get", None, state);

    run(model.store_secret_on_this_computer("providers", "openai_api_key", "sk-x".into()))
        .expect("the file store took it");

    assert_eq!(
        model.secret_store_kind(),
        Some(StoreKind::File),
        "the row kept the old store after the owner chose a new one"
    );
}

/// Taking the way back leaves the row saying the keyring, and drops the offer.
///
/// A successful migration moves every value, so the row that offered the way
/// back has nothing left to offer. Reading the snapshot again is what retires
/// the button; without it the owner is invited to do a thing already done.
#[test]
fn migrating_back_leaves_the_row_saying_the_keyring() {
    let (model, peer) = model("default", "active_aligned");

    let mut state = peer.result("setup.state.get", None);
    state["secrets"] = serde_json::json!({"store": "file", "availability": "ready"});
    peer.set_result("setup.state.get", None, state.clone());
    run(model.refresh_setup());
    assert_eq!(model.secret_store_kind(), Some(StoreKind::File));

    // What the engine will say once the values are back.
    state["secrets"] = serde_json::json!({"store": "keyring", "availability": "ready"});
    peer.set_result("setup.state.get", None, state);

    run(model.migrate_to_keyring(false)).expect("the migration was taken");

    assert_eq!(
        model.secret_store_kind(),
        Some(StoreKind::Keyring),
        "the row still offers a way back that has already been taken"
    );
}

/// The row still ends up correct when the ceiling is already reached.
///
/// The re-read after a write goes through the same gate as every other read,
/// and that gate refuses a fifth concurrent read by returning nothing, which
/// `read_setup` answers with `Ok(())`. A fix that inherits that would be no
/// fix at all: the owner consents to the file store, four reads happen to be
/// in flight, and the row keeps the old answer exactly as before. Worse, a
/// read issued before the write can land after it carrying the old store.
#[test]
fn the_row_is_corrected_even_with_the_read_ceiling_reached() {
    let context = MainContext::new();
    let _guard = context.acquire().expect("the context is free");
    let (model, peer) = model("default", "active_aligned");

    context.block_on(model.refresh_setup());
    assert_eq!(model.secret_store_kind(), Some(StoreKind::Keyring));

    // Four reads that have started and cannot finish: the ceiling, held there
    // for as long as this test needs it.
    peer.hold("setup.state.get");
    let mut in_flight = Vec::new();
    for _ in 0..fermix_desktop::models::api::MAX_CONCURRENT_READS {
        let model = Rc::clone(&model);
        in_flight.push(context.spawn_local(async move { model.refresh_setup().await }));
    }
    // Let each one take its permit and stop at the peer.
    for _ in 0..50 {
        context.iteration(false);
    }
    // The premise of this test, checked rather than assumed: four reads have
    // actually started and are sitting at the peer. Without this the test can
    // pass by never reaching the ceiling it exists to test.
    assert_eq!(
        peer.count("setup.state.get"),
        1 + fermix_desktop::models::api::MAX_CONCURRENT_READS,
        "the reads never started, so the ceiling was never reached"
    );

    // What the engine will say once the value is in the file store.
    let mut state = peer.result("setup.state.get", None);
    state["secrets"] = serde_json::json!({"store": "file", "availability": "ready"});
    peer.set_result("setup.state.get", None, state);

    let write = {
        let model = Rc::clone(&model);
        context.spawn_local(async move {
            model
                .store_secret_on_this_computer("providers", "openai_api_key", "sk-x".into())
                .await
        })
    };

    // The write lands while the four reads are still stuck, so its correcting
    // read meets the ceiling exactly as it would in the application.
    let before = peer.count("setup.state.get");
    for _ in 0..50 {
        context.iteration(false);
    }
    assert_eq!(
        peer.count("setup.state.get"),
        before,
        "a read got through while the ceiling was full, so this proves nothing"
    );

    // Then the ceiling clears, as it always eventually does, and the re-read
    // has to still happen. Giving up at the moment it was refused is the
    // defect; waiting for a slot is the fix.
    peer.release();
    context
        .block_on(write)
        .expect("the spawned write ran")
        .expect("the file store took it");
    for handle in in_flight {
        let _ = context.block_on(handle);
    }
    let after = peer.count("setup.state.get");

    // The call count, not the final row, is what settles this. Reads already
    // in flight answer from the peer's state at the moment they are let go, so
    // they can correct the row by accident here in a way a real read issued
    // before the write never would. What has to be true is that the write
    // ISSUED a read of its own.
    assert_eq!(
        after - before,
        1,
        "the correcting read was never issued: the gate refused it and the \
         refusal was swallowed, so in the application the row keeps the old store"
    );
}

/// Where values live, as the model reads it off the setup snapshot.
#[test]
fn the_model_reads_where_secrets_live_from_the_setup_snapshot() {
    let (model, peer) = model("default", "active_aligned");

    // An engine that predates the row says nothing, and nothing is claimed on
    // its behalf: a missing row is not "no store". The golden now carries the
    // row, so the silence has to be built by taking it away rather than by
    // leaning on a fixture that happened not to have it yet.
    let mut silent = peer.result("setup.state.get", None);
    silent
        .as_object_mut()
        .expect("the setup state is an object")
        .remove("secrets");
    peer.set_result("setup.state.get", None, silent);
    run(model.refresh_setup());
    assert_eq!(model.secret_store_kind(), None);

    for (published, expected) in [("keyring", StoreKind::Keyring), ("file", StoreKind::File)] {
        let mut state = peer.result("setup.state.get", None);
        // Two facts in one row, because one field could not carry both: a
        // locked keyring is store "keyring" with availability "locked", and a
        // home on the file store reports ready whatever the keyring is doing.
        state["secrets"] = serde_json::json!({
            "store": published,
            "availability": "ready",
        });
        peer.set_result("setup.state.get", None, state);
        run(model.refresh_setup());

        assert_eq!(
            model.secret_store_kind(),
            Some(expected),
            "the snapshot published {published}"
        );
    }
}

/// The way home is offered only from the place it leads away from.
///
/// Nothing to migrate means no button: an owner already on the keyring would
/// otherwise be offered a move to where they are.
#[test]
fn the_way_back_is_offered_only_from_the_file_store() {
    assert!(StoreKind::File.offers_return_to_keyring());
    assert!(!StoreKind::Keyring.offers_return_to_keyring());
}
