//! Integrations: the plugin rows decoded from the engine's own fixtures, the
//! plugin writers against a fake daemon, and the pure rules the pane draws by.

use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::{decode_response, CallError, Management};
use fermix_client::model::{Features, JobStatus, JobView};
use fermix_client::plugins::{
    client_answer, client_for, client_secret_id, client_state, client_title, consent_body, count,
    feature_state, job_words, line, matches, needs_operator, plugin_secret_id, reattachable,
    sign_in_provider, verbs, visible, visible_features, ClientAnswer, Filter, OAuthClient,
    PluginAction, PluginList, PluginRow, SettingKind, Verb, CLIENT_ID_MISSING, FEATURES,
    PORT_INVALID, REGION_MISSING, SECRET_FIRST,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::os::unix::net::UnixListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");

fn fixture_result(name: &str) -> Value {
    let found = SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"));
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    decode_response(&serde_json::to_vec(response).unwrap(), id).unwrap()
}

fn decode<T: DeserializeOwned>(name: &str) -> T {
    serde_json::from_value(fixture_result(name))
        .unwrap_or_else(|e| panic!("{name} does not decode: {e}"))
}

fn list() -> PluginList {
    decode("plugins_list")
}

fn plugin(name: &str) -> PluginRow {
    list().plugins.into_iter().find(|p| p.name == name).unwrap()
}

/// A row as the daemon publishes it, with `changes` laid over the fixture's Notion row.
fn row(changes: Value) -> PluginRow {
    let mut base = fixture_result("plugins_list")["plugins"][3].clone();
    for (k, v) in changes.as_object().unwrap() {
        base[k] = v.clone();
    }
    serde_json::from_value(base).unwrap()
}

fn client(changes: Value) -> OAuthClient {
    let mut base = json!({
        "provider": "tesla", "configured": false, "redirect_port": null, "client_id": null,
        "secret_present": true, "region": null, "regions": []
    });
    for (k, v) in changes.as_object().unwrap() {
        base[k] = v.clone();
    }
    serde_json::from_value(base).unwrap()
}

fn job(kind: &str, status: &str) -> JobView {
    serde_json::from_value(json!({
        "job_id": format!("job:{kind}"), "kind": kind, "status": status, "phase": null,
        "progress": null, "budget_ms": 600000, "started_at": "2026-09-25T00:00:00Z",
        "finished_at": null, "result": null, "failure": null
    }))
    .unwrap()
}

// ---- decoding ----

#[test]
fn the_published_list_decodes_rows_and_sign_in_clients() {
    let list = list();
    let names: Vec<&str> = list.plugins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["google_calendar", "acme", "obsidian", "notion"]);
    let calendar = &list.plugins[0];
    assert!(calendar.installed && calendar.enabled);
    assert_eq!(calendar.status_sentence, "Connected as owner@example.com.");
    assert_eq!(calendar.auth_provider.as_deref(), Some("google"));
    assert!(calendar.credential_present);
    assert_eq!(list.oauth_clients.len(), 2);
    assert_eq!(list.oauth_clients[0].redirect_port, Some(1455));
    assert!(list.oauth_clients[0].configured);
}

#[test]
fn a_remote_plugin_carries_its_disclosure_profiles_and_workspaces() {
    let acme = plugin("acme");
    assert_eq!(acme.runtime_kind.as_deref(), Some("remote_mcp"));
    assert_eq!(
        acme.remote_disclosure.as_deref(),
        Some("Your prompt and the content this plugin reads leave this Mac and reach Acme.")
    );
    assert_eq!(acme.access_profiles.len(), 2);
    assert!(!acme.access_profiles[0].write && acme.access_profiles[1].write);
    assert_eq!(acme.workspaces[1].label, "Lab");
    assert_eq!(acme.workspace_label, None);
}

#[test]
fn a_plugin_setting_decodes_its_kind_and_a_missing_kind_means_text() {
    let obsidian = plugin("obsidian");
    let vault = &obsidian.settings[0];
    assert_eq!(vault.key, "OBSIDIAN_VAULT_PATH");
    assert_eq!(vault.kind, SettingKind::Text);
    assert!(vault.required);
    assert_eq!(vault.value, None);
    let settings = json!([
        {"key": "a", "label": "A", "value": "true", "required": false, "kind": "boolean"},
        {"key": "b", "label": "B", "value": "x", "required": false},
        {"key": "c", "label": "C", "value": null, "required": false, "kind": "colour"}
    ]);
    let r = row(json!({ "settings": settings }));
    let kinds: Vec<SettingKind> = r.settings.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            SettingKind::Boolean,
            SettingKind::Text,
            SettingKind::Unknown
        ]
    );
    assert!(r.settings[0].is_on());
    assert!(!r.settings[1].is_on());
}

#[test]
fn a_newer_daemons_row_with_extra_and_missing_optional_fields_still_decodes() {
    let r: PluginRow = serde_json::from_value(json!({
        "name": "future", "title": "Future", "installed": false, "enabled": false,
        "status_sentence": "Not installed.", "consent_sentence": "Runs inside Fermix on this Mac.",
        "logo": {"mime": "image/png"}, "badge": 3
    }))
    .unwrap();
    assert!(r.verbs.is_empty() && r.actions.is_empty());
    assert_eq!(r.summary, None);
    assert_eq!(r.primary_action, None);
}

#[test]
fn every_plugin_writer_answer_decodes() {
    for name in [
        "plugins_enable",
        "plugins_disable",
        "plugins_disconnect",
        "plugins_setting_set",
    ] {
        let answer = fixture_result(name);
        let row: PluginRow = serde_json::from_value(answer["plugin"].clone())
            .unwrap_or_else(|e| panic!("{name} does not decode: {e}"));
        assert!(!row.name.is_empty());
    }
    let answer = fixture_result("plugins_oauth_client_set");
    let client: OAuthClient = serde_json::from_value(answer["oauth_client"].clone()).unwrap();
    assert_eq!(client.provider, "google");
}

#[test]
fn every_plugin_job_start_decodes_as_a_running_job() {
    let starts = [
        ("plugins_install_start", "plugin_install"),
        ("plugins_check_start", "plugin_check"),
        (
            "plugins_workspaces_discover_start",
            "plugin_workspaces_discover",
        ),
        ("plugins_workspace_select_start", "plugin_workspace_select"),
    ];
    for (name, kind) in starts {
        let job: JobView = decode(name);
        assert_eq!(job.kind, kind);
        assert_eq!(job.status, JobStatus::Running);
    }
}

// ---- verbs and actions ----

#[test]
fn a_button_paints_the_daemons_verb_and_routes_on_its_action() {
    let acme = plugin("acme");
    let drawn = verbs(&acme);
    let pairs: Vec<(&str, PluginAction)> =
        drawn.iter().map(|v| (v.label.as_str(), v.action)).collect();
    assert_eq!(
        pairs,
        [
            ("Choose workspace", PluginAction::ChooseWorkspace),
            ("Check again", PluginAction::Check),
            ("Disconnect", PluginAction::Disconnect),
            ("Turn off", PluginAction::Disable),
        ]
    );
    assert!(drawn[0].primary);
    assert!(drawn[1..].iter().all(|v| !v.primary));
}

#[test]
fn an_unknown_action_draws_no_button_and_its_neighbours_keep_their_pairing() {
    let r = row(json!({
        "verbs": ["Teleport", "Sign in again", "Turn off"],
        "actions": ["teleport", "sign_in", "disable"],
        "primary_verb": "Sign in again", "primary_action": "sign_in"
    }));
    assert_eq!(
        verbs(&r),
        [
            Verb {
                label: "Sign in again".into(),
                action: PluginAction::SignIn,
                primary: true
            },
            Verb {
                label: "Turn off".into(),
                action: PluginAction::Disable,
                primary: false
            },
        ]
    );
}

#[test]
fn a_row_with_no_verbs_draws_no_buttons() {
    let r = row(json!({"verbs": [], "actions": [], "primary_verb": null, "primary_action": null}));
    assert!(verbs(&r).is_empty());
}

#[test]
fn every_published_action_id_is_known() {
    let ids = [
        ("install", PluginAction::Install),
        ("enable", PluginAction::Enable),
        ("disable", PluginAction::Disable),
        ("sign_in", PluginAction::SignIn),
        ("add_token", PluginAction::AddToken),
        ("replace_token", PluginAction::ReplaceToken),
        ("set_up_client", PluginAction::SetUpClient),
        ("choose_workspace", PluginAction::ChooseWorkspace),
        ("check", PluginAction::Check),
        ("disconnect", PluginAction::Disconnect),
    ];
    for (wire, action) in ids {
        assert_eq!(PluginAction::parse(wire), Some(action), "{wire}");
    }
    assert_eq!(PluginAction::parse("Sign in"), None);
    assert_eq!(PluginAction::parse(""), None);
}

// ---- filters, search and the row's line ----

#[test]
fn each_filter_is_counted_and_features_are_the_two_linux_ones() {
    let list = list();
    let counts: Vec<(Filter, usize)> = Filter::ALL
        .iter()
        .map(|f| (*f, count(*f, &list.plugins)))
        .collect();
    assert_eq!(
        counts,
        [
            (Filter::Installed, 2),
            (Filter::Available, 2),
            (Filter::Mcps, 2),
            (Filter::Features, 2),
        ]
    );
    let titles: Vec<&str> = Filter::ALL.iter().map(|f| f.title()).collect();
    assert_eq!(titles, ["Installed", "Available", "MCPs", "Features"]);
}

#[test]
fn a_filter_is_named_and_found_again_by_its_slug() {
    for filter in Filter::ALL {
        assert_eq!(Filter::from_slug(filter.slug()), Some(filter));
    }
    assert_eq!(Filter::from_slug("everything"), None);
}

#[test]
fn mcps_are_the_local_process_and_remote_server_plugins() {
    let list = list();
    let mcps: Vec<&str> = visible(&list.plugins, Filter::Mcps, "")
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(mcps, ["acme", "obsidian"]);
    assert!(visible(&list.plugins, Filter::Features, "").is_empty());
}

#[test]
fn search_reads_the_name_and_the_description_and_ignores_case() {
    assert!(matches("", "Notion", None));
    assert!(matches("  ", "Notion", None));
    assert!(matches("NOT", "Notion", None));
    assert!(matches(
        "vault",
        "Obsidian",
        Some("Notes in your local Obsidian vault.")
    ));
    assert!(!matches("slack", "Notion", Some("Pages.")));
    let list = list();
    let found: Vec<&str> = visible(&list.plugins, Filter::Installed, "brain")
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(found, ["acme"]);
}

#[test]
fn an_installed_row_shows_its_status_and_an_available_one_its_summary() {
    assert_eq!(
        line(&plugin("google_calendar")),
        "Connected as owner@example.com."
    );
    assert_eq!(
        line(&plugin("notion")),
        "Search, read, create, and update Notion pages and data sources."
    );
    assert_eq!(line(&row(json!({"summary": " "}))), "Not installed.");
    assert_eq!(line(&row(json!({"summary": null}))), "Not installed.");
}

#[test]
fn features_link_to_their_panes_and_say_on_off_or_not_reported() {
    let ids: Vec<(&str, &str)> = FEATURES.iter().map(|f| (f.id, f.pane)).collect();
    assert_eq!(
        ids,
        [("computer_use", "computer"), ("meetings", "meetings")]
    );
    let features = Features {
        voice: true,
        voice_notes: false,
        meetings: true,
        computer_use: false,
    };
    assert_eq!(feature_state(&FEATURES[0], Some(&features)), "Off");
    assert_eq!(feature_state(&FEATURES[1], Some(&features)), "On");
    assert_eq!(feature_state(&FEATURES[1], None), "Not reported");
    let found: Vec<&str> = visible_features("notetaker").iter().map(|f| f.id).collect();
    assert_eq!(found, ["meetings"]);
    assert_eq!(visible_features("").len(), 2);
}

// ---- after a switch-on, consent, and the ids a plugin is addressed by ----

#[test]
fn only_a_step_the_person_must_take_opens_the_detail_after_an_enable() {
    for action in ["sign_in", "add_token", "set_up_client", "choose_workspace"] {
        assert!(
            needs_operator(&row(json!({"primary_action": action}))),
            "{action}"
        );
    }
    for action in [
        json!("check"),
        json!("enable"),
        json!("install"),
        json!("teleport"),
        Value::Null,
    ] {
        assert!(
            !needs_operator(&row(json!({"primary_action": action}))),
            "{action}"
        );
    }
}

#[test]
fn consent_states_where_it_runs_and_what_leaves_the_machine() {
    assert_eq!(
        consent_body(&plugin("notion")),
        "Runs inside Fermix on this Mac."
    );
    assert_eq!(
        consent_body(&plugin("acme")),
        "Runs on the plugin's own servers, not on this Mac.\n\n\
         Your prompt and the content this plugin reads leave this Mac and reach Acme."
    );
}

#[test]
fn a_plugin_is_addressed_by_its_name_and_a_client_by_its_provider() {
    assert_eq!(sign_in_provider("notion"), "plugin:notion");
    assert_eq!(plugin_secret_id("discord"), "plugin:discord");
    assert_eq!(client_secret_id("tesla"), "oauth_client:tesla");
}

// ---- sign-in clients ----

#[test]
fn a_blank_port_is_left_out_and_never_sent_as_zero() {
    let answer = client_answer(&client(json!({})), " id-1 ", " ", None).unwrap();
    assert_eq!(
        serde_json::to_value(&answer).unwrap(),
        json!({"provider": "tesla", "client_id": "id-1"})
    );
    let answer = client_answer(&client(json!({})), "id-1", "8123", None).unwrap();
    assert_eq!(answer.redirect_port, Some(8123));
    for bad in ["0", "65536", "-1", "80a"] {
        assert_eq!(
            client_answer(&client(json!({})), "id-1", bad, None),
            Err(PORT_INVALID),
            "{bad}"
        );
    }
}

#[test]
fn a_region_is_sent_exactly_when_the_provider_offers_regions() {
    let regions = json!([{"id": "na", "label": "North America"}, {"id": "eu", "label": "Europe"}]);
    let regional = client(json!({ "regions": regions }));
    assert_eq!(
        client_answer(&regional, "id", "", None),
        Err(REGION_MISSING)
    );
    assert_eq!(
        client_answer(&regional, "id", "", Some("mars")),
        Err(REGION_MISSING)
    );
    let answer = client_answer(&regional, "id", "", Some("eu")).unwrap();
    assert_eq!(
        serde_json::to_value(&answer).unwrap(),
        json!({"provider": "tesla", "client_id": "id", "region": "eu"})
    );
    let single = client_answer(&client(json!({})), "id", "", Some("eu")).unwrap();
    assert_eq!(single.region, None);
}

#[test]
fn a_client_needs_its_secret_stored_first_and_an_identifier() {
    let no_secret = client(json!({"secret_present": false}));
    assert_eq!(client_answer(&no_secret, "id", "", None), Err(SECRET_FIRST));
    assert_eq!(
        client_answer(&client(json!({})), "  ", "", None),
        Err(CLIENT_ID_MISSING)
    );
}

#[test]
fn a_client_says_whether_it_is_configured_and_its_region() {
    assert_eq!(client_state(&client(json!({}))), "Not configured");
    let regions = json!([{"id": "eu", "label": "Europe, Middle East and Africa"}]);
    let set = client(json!({"configured": true, "region": "eu", "regions": regions}));
    assert_eq!(
        client_state(&set),
        "Configured, Europe, Middle East and Africa"
    );
}

#[test]
fn a_client_takes_the_daemons_spelling_of_its_name_where_a_plugin_has_it() {
    let mut list = list();
    assert_eq!(client_title(&list, "notion"), "Notion");
    list.plugins[3].auth_provider = Some("github".into());
    list.plugins[3].title = "GitHub".into();
    assert_eq!(client_title(&list, "github"), "GitHub");
    assert_eq!(
        client_title(&list, "google"),
        "Google",
        "not Google Calendar"
    );
    assert_eq!(client_title(&list, "x"), "X");
}

#[test]
fn a_plugin_finds_the_sign_in_client_its_provider_names() {
    let list = list();
    let calendar = &list.plugins[0];
    assert_eq!(client_for(&list, calendar).unwrap().provider, "google");
    let obsidian = &list.plugins[2];
    assert_eq!(client_for(&list, obsidian), None, "no auth provider");
    let orphan = row(json!({"auth_provider": "github"}));
    assert_eq!(client_for(&list, &orphan), None, "no client published");
}

// ---- jobs ----

#[test]
fn a_running_plugin_job_nobody_follows_is_reattached() {
    let listed: fermix_client::model::JobList = decode("job_list");
    assert!(reattachable(&listed.jobs, &[]).is_empty(), "no plugin jobs");
    let jobs = [
        job("plugin_install", "running"),
        job("plugin_check", "completed"),
        job("auth", "running"),
        job("plugin_workspaces_discover", "running"),
    ];
    let ids: Vec<String> = reattachable(&jobs, &[])
        .into_iter()
        .map(|j| j.job_id)
        .collect();
    assert_eq!(
        ids,
        ["job:plugin_install", "job:plugin_workspaces_discover"]
    );
    let followed = ["job:plugin_install".to_owned()];
    assert_eq!(reattachable(&jobs, &followed).len(), 1);
}

#[test]
fn a_job_is_described_by_its_kind_never_its_phase() {
    assert_eq!(job_words("plugin_install"), "Installing…");
    assert_eq!(job_words("plugin_check"), "Checking…");
    assert_eq!(
        job_words("plugin_workspaces_discover"),
        "Finding workspaces…"
    );
    assert_eq!(
        job_words("plugin_workspace_select"),
        "Saving the workspace…"
    );
    assert_eq!(job_words("auth"), "Waiting for the browser…");
    assert_eq!(job_words("something_new"), "Working…");
}

// ---- the writers on the wire ----

/// Answers each connection with the next envelope and hands back what was asked.
fn fake_daemon(replies: Vec<Value>) -> (tempfile::TempDir, Management, JoinHandle<Vec<Value>>) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let handle = thread::spawn(move || {
        replies
            .into_iter()
            .map(|reply| {
                let (mut stream, _) = listener.accept().unwrap();
                let request: Value =
                    serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
                let text = serde_json::to_string(&reply)
                    .unwrap()
                    .replace("$id", request["request_id"].as_str().unwrap());
                write_frame(&mut stream, text.as_bytes()).unwrap();
                request
            })
            .collect()
    });
    let client = Management::new(socket, Duration::from_millis(500));
    (dir, client, handle)
}

fn ok(result: Value) -> Value {
    json!({"request_id": "$id", "result": result})
}

#[test]
fn the_list_is_asked_for_with_empty_params() {
    let (_dir, client, handle) = fake_daemon(vec![ok(fixture_result("plugins_list"))]);
    let list = client.plugins_list().unwrap();
    assert_eq!(list.plugins.len(), 4);
    let asked = handle.join().unwrap();
    assert_eq!(asked[0]["method"], "plugins.list");
    assert_eq!(asked[0]["params"], json!({}));
}

#[test]
fn a_row_whose_verbs_and_actions_do_not_pair_is_a_protocol_error() {
    let mut broken = fixture_result("plugins_list");
    broken["plugins"][0]["verbs"] = json!(["Check again"]);
    let (_dir, client, handle) = fake_daemon(vec![ok(broken)]);
    let err = client.plugins_list().unwrap_err();
    assert!(matches!(err, CallError::Protocol(_)), "got {err:?}");
    handle.join().unwrap();
}

#[test]
fn the_row_writers_send_the_plugin_name_and_answer_with_the_row() {
    let replies = vec![
        ok(fixture_result("plugins_enable")),
        ok(fixture_result("plugins_disable")),
        ok(fixture_result("plugins_disconnect")),
        ok(fixture_result("plugins_setting_set")),
    ];
    let (_dir, client, handle) = fake_daemon(replies);
    assert!(client.plugins_enable("google_calendar").unwrap().enabled);
    assert!(!client.plugins_disable("google_calendar").unwrap().enabled);
    assert!(
        !client
            .plugins_disconnect("google_calendar")
            .unwrap()
            .credential_present
    );
    let set = client
        .plugins_setting_set("obsidian", "OBSIDIAN_VAULT_PATH", "/vault")
        .unwrap();
    assert_eq!(set.settings[0].value.as_deref(), Some("/Users/owner/Vault"));
    let asked = handle.join().unwrap();
    let methods: Vec<&str> = asked
        .iter()
        .map(|r| r["method"].as_str().unwrap())
        .collect();
    assert_eq!(
        methods,
        [
            "plugins.enable",
            "plugins.disable",
            "plugins.disconnect",
            "plugins.setting.set"
        ]
    );
    assert_eq!(asked[0]["params"], json!({"name": "google_calendar"}));
    assert_eq!(
        asked[3]["params"],
        json!({"name": "obsidian", "key": "OBSIDIAN_VAULT_PATH", "value": "/vault"})
    );
}

#[test]
fn a_sign_in_client_is_sent_without_the_keys_left_blank() {
    let (_dir, client, handle) = fake_daemon(vec![ok(fixture_result("plugins_oauth_client_set"))]);
    let answer = ClientAnswer {
        provider: "google".into(),
        client_id: "1042.apps.googleusercontent.com".into(),
        redirect_port: None,
        region: None,
    };
    let stored = client.plugins_oauth_client_set(&answer).unwrap();
    assert!(stored.configured);
    let asked = handle.join().unwrap();
    assert_eq!(asked[0]["method"], "plugins.oauth_client.set");
    assert_eq!(
        asked[0]["params"],
        json!({"provider": "google", "client_id": "1042.apps.googleusercontent.com"})
    );
}

#[test]
fn the_plugin_jobs_start_with_their_own_params() {
    let replies = vec![
        ok(fixture_result("plugins_install_start")),
        ok(fixture_result("plugins_check_start")),
        ok(fixture_result("plugins_workspaces_discover_start")),
        ok(fixture_result("plugins_workspace_select_start")),
    ];
    let (_dir, client, handle) = fake_daemon(replies);
    assert_eq!(
        client.plugins_install_start("obsidian").unwrap().kind,
        "plugin_install"
    );
    assert_eq!(
        client.plugins_check_start("acme").unwrap().kind,
        "plugin_check"
    );
    let discover = client.plugins_workspaces_discover_start("acme").unwrap();
    assert_eq!(discover.budget_ms, 60_000);
    let select = client
        .plugins_workspace_select_start("acme", "retrieval", "ws_lab", "Lab")
        .unwrap();
    assert_eq!(select.phase.as_deref(), Some("binding"));
    let asked = handle.join().unwrap();
    assert_eq!(asked[0]["method"], "plugins.install.start");
    assert_eq!(asked[0]["params"], json!({"name": "obsidian"}));
    assert_eq!(asked[1]["method"], "plugins.check.start");
    assert_eq!(asked[2]["method"], "plugins.workspaces.discover.start");
    assert_eq!(asked[3]["method"], "plugins.workspace.select.start");
    assert_eq!(
        asked[3]["params"],
        json!({"name": "acme", "profile": "retrieval", "workspace_id": "ws_lab", "label": "Lab"})
    );
}

#[test]
fn a_job_start_that_failed_without_a_reason_is_a_protocol_error() {
    let mut start = fixture_result("plugins_install_start");
    start["status"] = json!("failed");
    let (_dir, client, handle) = fake_daemon(vec![ok(start)]);
    let err = client.plugins_install_start("obsidian").unwrap_err();
    assert!(matches!(err, CallError::Protocol(_)), "got {err:?}");
    handle.join().unwrap();
}
