//! The Cues pane's rules (M60 cues §4, §9): whether there is a pane at all, what each row draws
//! from what the daemon publishes, the filter model's pair rule, the questions asked in place,
//! and the pane's own words. Every row comes from the vendored goldens, changed only where a test
//! needs another state.

use fermix_client::chat::{written_time, LocalTime};
use fermix_client::cues::view::{
    approval_line, available, door_in_the_open, failure, mandate_line, model_shown,
    model_suggestions, opening, secrets_first, shows_model_row, shut, source_facts, source_line,
    status_tone, waiting_lines, watcher_filter, watcher_line, watcher_task, Asking, Failure,
    FilterDraft, Question, Tone,
};
use fermix_client::cues::wire::{
    CueStatus, MandateRow, ProposalList, SecretField, SourceList, SourceRow, WatcherDetail,
    WatcherRow,
};
use fermix_client::cues::words::{self, STRINGS};
use fermix_client::management::{decode_response, CallError};
use fermix_client::model::{Capabilities, Hello, ListedModel};
use fermix_client::settings::{
    matching_panes, pane, ChoiceOption, Listing, Row, Section, SectionRows, Sections, GROUPS, PANES,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const ERRORS: &str = include_str!("../contracts/management/fixtures/errors.jsonl");

fn line_named(lines: &str, name: &str) -> Value {
    lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"))
}

fn result(name: &str) -> Value {
    line_named(SUCCESS, name)["response"]["result"].clone()
}

fn decode<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("decodes")
}

/// The error golden `name`, as the client turns it into a `CallError`.
fn refused(name: &str) -> CallError {
    let response = &line_named(ERRORS, name)["response"];
    let id = response["request_id"].as_str().unwrap_or("x");
    decode_response(&serde_json::to_vec(response).expect("JSON"), id)
        .expect_err("an error golden is a refusal")
}

fn status() -> CueStatus {
    decode(result("cues_status"))
}

fn sources() -> Vec<SourceRow> {
    decode::<SourceList>(result("cues_sources_list")).sources
}

/// A source row from the list golden, with some of its fields changed.
fn source(name: &str, changes: Value) -> SourceRow {
    let list = result("cues_sources_list");
    let mut row = list["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .find(|s| s["ref"] == name)
        .expect("a source of that name")
        .clone();
    for (key, value) in changes.as_object().expect("changes") {
        row[key] = value.clone();
    }
    decode(row)
}

fn mandate(changes: Value) -> MandateRow {
    let mut row = result("cues_mandates_list")["mandates"][0].clone();
    for (key, value) in changes.as_object().expect("changes") {
        row[key] = value.clone();
    }
    decode(row)
}

fn watcher(changes: Value) -> WatcherRow {
    let mut row = result("cues_watchers_list")["watchers"][0].clone();
    for (key, value) in changes.as_object().expect("changes") {
        row[key] = value.clone();
    }
    decode(row)
}

fn capabilities() -> Capabilities {
    decode::<Hello>(result("hello")).capabilities
}

fn sections() -> Vec<Section> {
    decode::<Sections>(result("settings_sections")).sections
}

// ---- the gate (§4.1) ----

#[test]
fn the_pane_exists_with_the_method_the_section_and_an_answered_status() {
    let (caps, sections, status) = (capabilities(), sections(), status());
    assert!(caps.has_method("cues.status"));
    assert!(available(Some(&caps), Some(&sections), Some(&status)));
    let mut without = caps.clone();
    without.methods.retain(|m| m != "cues.status");
    assert!(!without.has_method("cues.status"));
    assert!(!available(Some(&without), Some(&sections), Some(&status)));
    let others: Vec<Section> = sections
        .iter()
        .filter(|s| s.id != "cues")
        .cloned()
        .collect();
    assert!(!available(Some(&caps), Some(&others), Some(&status)));
    assert!(
        !available(Some(&caps), Some(&sections), None),
        "not asked yet"
    );
    assert!(
        !available(Some(&caps), None, Some(&status)),
        "sections unread"
    );
    assert!(
        !available(None, Some(&sections), Some(&status)),
        "daemon down"
    );
}

#[test]
fn a_route_leaves_the_pane_only_once_the_gate_has_said_no() {
    let (caps, sections) = (capabilities(), sections());
    assert!(!shut(Some(&caps), Some(&sections), false), "open");
    assert!(!shut(Some(&caps), None, false), "sections still to read");
    assert!(!shut(None, None, false), "down: the down page stands in");
    let mut without = caps.clone();
    without.methods.retain(|m| m != "cues.status");
    assert!(shut(Some(&without), None, false), "no method");
    let others: Vec<Section> = sections
        .iter()
        .filter(|s| s.id != "cues")
        .cloned()
        .collect();
    assert!(shut(Some(&caps), Some(&others), false), "no section");
    assert!(
        shut(Some(&caps), Some(&sections), true),
        "cues.status missing"
    );
}

#[test]
fn the_pane_answers_its_keywords_and_hides_from_search_with_the_gate() {
    let (sections, loaded) = (sections(), HashMap::new());
    for word in ["feeds", "webhooks", "Mandates", "door", "cues"] {
        assert_eq!(matching_panes(word, &sections, &loaded, &[]), ["cues"]);
    }
    assert!(matching_panes("feeds", &sections, &loaded, &["cues"]).is_empty());
    let all = matching_panes("", &sections, &loaded, &["cues"]);
    assert_eq!(all.len(), PANES.len() - 1);
    assert!(!all.contains(&"cues"));
}

#[test]
fn sixteen_panes_sit_in_four_groups_with_cues_after_integrations() {
    assert_eq!(PANES.len(), 16);
    let grouped: usize = GROUPS.iter().map(|(_, panes)| panes.len()).sum();
    assert_eq!(grouped, 16);
    assert_eq!(
        GROUPS[1],
        ("Connections", &["channels", "integrations", "cues"][..])
    );
    let cues = pane("cues").expect("a pane");
    assert_eq!(cues.title, "Cues");
    assert_eq!(cues.keywords, words::KEYWORDS);
}

/// A cue method the daemon does not have hides the pane; one it has at a newer version is the
/// daemon-too-old state; memory off is the daemon's own sentence in place of a list.
#[test]
fn a_failed_cue_read_hides_the_pane_says_update_or_says_the_daemons_words() {
    assert_eq!(failure(&refused("method_not_found")), Failure::Missing);
    let too_old = failure(&refused("method_requires_newer_engine"));
    assert_eq!(
        too_old,
        Failure::Sentence("Fermix is older than this app. Update the fermix package.".into())
    );
    assert_eq!(
        failure(&refused("unavailable_cues_memory_off")),
        Failure::Sentence("Cues keep their records in memory, which is off.".into())
    );
    let CallError::Refused(refusal) = refused("method_requires_newer_engine") else {
        panic!("a refusal");
    };
    assert_eq!(refusal.requires, Some(2));
    let CallError::Refused(plain) = refused("method_not_found") else {
        panic!("a refusal");
    };
    assert_eq!(plain.requires, None);
}

// ---- rows (§4.2, §4.4 to §4.6) ----

#[test]
fn every_list_golden_decodes_and_nulls_read_as_absent_keys() {
    assert_eq!(sources().len(), 3);
    let mut nulls = result("cues_sources_list")["sources"][1].clone();
    let keys = [
        "preset",
        "primary_action",
        "location",
        "url",
        "missing_secret",
        "last_good_at",
    ];
    for key in keys {
        nulls[key] = Value::Null;
    }
    let mut absent = nulls.clone();
    for key in keys {
        absent.as_object_mut().expect("a row").remove(key);
    }
    let (with_nulls, without): (SourceRow, SourceRow) = (decode(nulls), decode(absent));
    assert_eq!(with_nulls, without);
    assert_eq!(with_nulls.preset, None);
}

#[test]
fn a_source_is_quiet_off_red_when_it_stopped_amber_when_it_owes_a_step_and_well_otherwise() {
    assert_eq!(
        source_line(&source("eng-blogs", json!({}))).tone,
        Tone::Good
    );
    let off = source(
        "eng-blogs",
        json!({"enabled": false, "state": "auth_failed"}),
    );
    assert_eq!(source_line(&off).tone, Tone::Quiet);
    let failed = source_line(&source("eng-blogs", json!({"state": "auth_failed"})));
    assert_eq!((failed.tone, failed.attention), (Tone::Fail, true));
    let owed = source(
        "gh-hooks",
        json!({"primary_action": "set_secret", "actions": ["disable", "set_secret"]}),
    );
    assert_eq!(source_line(&owed).tone, Tone::Warn);
}

#[test]
fn a_switch_is_drawn_only_where_the_daemon_publishes_one() {
    let line = source_line(&source("home-alerts", json!({})));
    assert_eq!(line.switch, Some(true));
    let none = source_line(&source("home-alerts", json!({"actions": ["new_token"]})));
    assert_eq!(none.switch, None);
    let m = mandate_line(&mandate(json!({"actions": []})));
    assert_eq!(m.switch, None);
    assert_eq!(mandate_line(&mandate(json!({}))).switch, Some(true));
}

#[test]
fn an_unknown_word_draws_the_daemons_sentence_and_no_control() {
    let row = source(
        "eng-blogs",
        json!({"state": "teleporting", "primary_action": "teleport", "actions": ["teleport"]}),
    );
    let line = source_line(&row);
    assert_eq!(line.switch, None);
    assert_eq!(
        line.tone,
        Tone::Good,
        "an unknown action leads with nothing"
    );
    assert_eq!(line.sentence, row.status_sentence);
    assert_eq!(opening(&row), None);
    let m = mandate(json!({"primary_action": null, "actions": ["promote"], "mode": "yolo"}));
    let line = mandate_line(&m);
    assert!(!line.approve);
    assert_eq!(line.switch, None);
    assert_eq!(line.tag, None, "a mode this build does not know has no tag");
}

#[test]
fn a_row_opens_its_page_at_the_secret_or_the_token_the_daemon_leads_with() {
    let webhook = source(
        "gh-hooks",
        json!({"primary_action": "set_secret", "actions": ["disable", "set_secret"],
               "missing_secret": "secret"}),
    );
    assert_eq!(opening(&webhook), Some(SecretField::Secret));
    let first = secrets_first(&webhook, opening(&webhook));
    assert!(first[0].field.is(SecretField::Secret));
    let push = source(
        "home-alerts",
        json!({"primary_action": "new_token", "actions": ["new_token"], "missing_secret": "token"}),
    );
    assert_eq!(opening(&push), Some(SecretField::Token));
    let unpublished = source(
        "gh-hooks",
        json!({"primary_action": "set_secret", "actions": ["disable"], "missing_secret": "secret"}),
    );
    assert_eq!(
        opening(&unpublished),
        None,
        "a lead not in actions opens plain"
    );
    let mismatched = source(
        "gh-hooks",
        json!({"primary_action": "set_secret", "actions": ["set_secret"],
               "missing_secret": "path_secret"}),
    );
    assert_eq!(opening(&mismatched), None, "Fermix makes the address");
    assert_eq!(opening(&source("eng-blogs", json!({}))), None);
}

#[test]
fn a_watcher_row_carries_its_name_its_sources_and_the_daemons_sentence_only() {
    let line = watcher_line(&watcher(json!({})));
    assert_eq!(line.name, "release-watch");
    assert_eq!(line.qualifier.as_deref(), Some("eng-blogs"));
    assert_eq!(line.sentence, "Filtering normally.");
    assert_eq!((line.tone, line.attention), (Tone::Good, false));
    assert_eq!(line.switch, None, "watchers are changed in chat");
    for state in ["needs_update", "held"] {
        let line = watcher_line(&watcher(json!({"state": state})));
        assert_eq!((line.tone, line.attention), (Tone::Warn, true), "{state}");
    }
    let paused = watcher(json!({"task": {"job_id": "j", "name": null, "paused": true}}));
    assert_eq!(watcher_line(&paused).tone, Tone::Quiet);
    assert_eq!(watcher_task(&paused), "j · keeps no notes · paused");
}

#[test]
fn a_watchers_filter_names_the_model_in_use_or_draws_no_row_for_an_unknown_one() {
    assert_eq!(
        watcher_filter(&watcher(json!({}))).as_deref(),
        Some("Your main model")
    );
    let chosen = watcher(json!({"filter_model": {"provider": "openai", "model": "gpt-5-mini"}}));
    assert_eq!(watcher_filter(&chosen).as_deref(), Some("gpt-5-mini"));
    let none = watcher(json!({"filter": "none"}));
    assert_eq!(watcher_filter(&none).as_deref(), Some("No filter"));
    assert_eq!(watcher_filter(&watcher(json!({"filter": "oracle"}))), None);
    let detail: WatcherDetail = decode(result("cues_watchers_get")["watcher"].clone());
    assert_eq!(detail.row.name, "release-watch");
}

#[test]
fn approve_is_offered_only_where_the_daemon_leads_with_it() {
    let approved = mandate_line(&mandate(json!({})));
    assert!(!approved.approve);
    assert_eq!(approved.tone, Tone::Good);
    assert_eq!(approved.tag, Some(("Ask me first", false)));
    assert_eq!(approved.qualifier.as_deref(), Some("Release watch"));
    let unapproved =
        mandate(json!({"primary_action": "approve", "actions": ["disable", "approve"]}));
    let line = mandate_line(&unapproved);
    assert!(line.approve);
    assert_eq!(line.tone, Tone::Warn);
    let inactive = mandate(json!({
        "primary_action": null, "actions": ["disable"],
        "load": {"state": "inactive", "sentence": "No Fermix app can ask you on this computer."}
    }));
    let line = mandate_line(&inactive);
    assert!(!line.approve);
    assert_eq!(line.tone, Tone::Warn);
    let auto = mandate_line(&mandate(json!({"mode": "auto"})));
    assert_eq!(auto.tag, Some(("Act without asking", true)));
}

#[test]
fn the_approval_line_is_the_daemons_words() {
    let inactive = mandate(json!({
        "load": {"state": "inactive", "sentence": "Its validator is not owned by root."},
        "approval": {"state": "approved", "detail": "Approved from the Fermix app.",
                     "approved_at": null, "approved_from": null, "disabled_at": null,
                     "disabled_reason": null}
    }));
    assert_eq!(
        approval_line(&inactive),
        "Its validator is not owned by root."
    );
    let detailed = mandate(json!({
        "approval": {"state": "disabled", "detail": "Turned off after a change.",
                     "approved_at": null, "approved_from": null, "disabled_at": null,
                     "disabled_reason": null}
    }));
    assert_eq!(approval_line(&detailed), "Turned off after a change.");
    let plain = mandate(json!({}));
    assert_eq!(approval_line(&plain), plain.status_sentence);
}

#[test]
fn the_status_and_the_door_take_their_tones_from_the_daemon() {
    let running = status();
    assert_eq!(status_tone(&running), Tone::Good);
    assert!(!door_in_the_open(&running));
    let mut value = result("cues_status");
    value["state"] = json!("needs_restart");
    value["door"]["state"] = json!("unavailable");
    let restart: CueStatus = decode(value.clone());
    assert_eq!(status_tone(&restart), Tone::Warn);
    assert!(door_in_the_open(&restart));
    value["state"] = json!("off");
    assert_eq!(status_tone(&decode(value.clone())), Tone::Quiet);
    value["state"] = json!("dreaming");
    assert_eq!(status_tone(&decode(value)), Tone::Quiet);
}

// ---- the filter model (§4.3) ----

#[test]
fn a_provider_is_held_until_a_model_is_chosen_and_the_pair_goes_in_one_write() {
    let mut draft = FilterDraft::default();
    assert_eq!(draft.provider("anthropic"), "anthropic");
    assert!(!draft.holds("anthropic"));
    draft.hold("openai");
    assert!(draft.holds("anthropic"));
    assert_eq!(draft.provider("anthropic"), "openai");
    assert_eq!(
        draft.choose_model("anthropic", "  "),
        None,
        "a blank model writes nothing"
    );
    let pair = draft
        .choose_model("anthropic", " gpt-5-mini ")
        .expect("a write");
    assert_eq!(
        Value::Object(pair),
        json!({"filter_provider": "openai", "filter_model": "gpt-5-mini"})
    );
    draft.drop_held();
    assert_eq!(draft.provider("anthropic"), "anthropic");
    let saved = draft
        .choose_model("anthropic", "claude-haiku-5-5")
        .expect("a write");
    assert_eq!(saved["filter_provider"], json!("anthropic"));
    assert_eq!(
        draft.choose_model("", "claude-haiku-5-5"),
        None,
        "main model"
    );
}

#[test]
fn the_main_model_clears_both_keys_in_one_write() {
    assert_eq!(
        Value::Object(fermix_client::cues::view::main_model()),
        json!({"filter_provider": "", "filter_model": ""})
    );
}

fn filter_model_row() -> Row {
    let section: SectionRows = decode(result("settings_get_cues"));
    section
        .rows
        .into_iter()
        .find(|r| r.key == "filter_model")
        .expect("the golden has the model row")
}

fn values(options: &[ChoiceOption]) -> Vec<&str> {
    options.iter().map(|o| o.value.as_str()).collect()
}

#[test]
fn the_model_row_suggests_the_saved_catalog_or_the_held_providers_own_list() {
    let row = filter_model_row();
    let mut draft = FilterDraft::default();
    let catalog = model_suggestions(&row, &draft, "anthropic", None);
    assert_eq!(catalog.len(), 9);
    assert_eq!(catalog[0].value, "claude-sonnet-4-6");
    draft.hold("openai");
    assert!(
        model_suggestions(&row, &draft, "anthropic", None).is_empty(),
        "the saved provider's catalog is never offered for another"
    );
    let listed: Listing = Ok(vec![ListedModel {
        id: "gpt-5.5-mini".into(),
        label: "GPT-5.5 mini".into(),
    }]);
    let held = model_suggestions(&row, &draft, "anthropic", Some(("openai", &listed)));
    assert_eq!(values(&held), ["gpt-5.5-mini"]);
    assert_eq!(held[0].label, "GPT-5.5 mini");
    let elsewhere = model_suggestions(&row, &draft, "anthropic", Some(("google", &listed)));
    assert!(elsewhere.is_empty(), "another provider's list");
    let failed: Listing = Err("Fermix could not list them.".into());
    let none = model_suggestions(&row, &draft, "anthropic", Some(("openai", &failed)));
    assert!(none.is_empty(), "a failed list leaves a field to type in");
    draft.hold("anthropic");
    assert_eq!(
        model_suggestions(&row, &draft, "anthropic", Some(("openai", &listed))).len(),
        9,
        "back on the saved provider"
    );
}

#[test]
fn the_model_row_shows_the_saved_model_until_another_provider_is_held() {
    let row = filter_model_row();
    let mut draft = FilterDraft::default();
    assert_eq!(model_shown(&row, &draft, "anthropic"), "claude-sonnet-4-6");
    draft.hold("openai");
    assert_eq!(
        model_shown(&row, &draft, "anthropic"),
        "",
        "a held provider's model is new"
    );
    assert!(shows_model_row(&draft, "anthropic"));
    draft.drop_held();
    assert!(
        !shows_model_row(&draft, ""),
        "the main model has no model row"
    );
    draft.hold("openai");
    assert!(
        shows_model_row(&draft, ""),
        "a provider held from the main model"
    );
}

// ---- questions asked in place ----

#[test]
fn a_question_is_asked_then_answered_by_a_second_press_only() {
    let mut asking = Asking::default();
    assert_eq!(asking.confirm(), None, "nothing asked, nothing done");
    asking.ask("release-issues", Question::Approve);
    assert_eq!(asking.asking("release-issues"), Some(Question::Approve));
    asking.ask("other", Question::RemoveMandate);
    assert_eq!(
        asking.asking("release-issues"),
        None,
        "one question at a time"
    );
    asking.withdraw();
    assert_eq!(asking.confirm(), None);
    asking.ask("release-issues", Question::Approve);
    let asked = asking.confirm();
    assert_eq!(asked, Some(("release-issues".into(), Question::Approve)));
    assert!(asking.acting("release-issues"));
    asking.ask("other", Question::Approve);
    assert_eq!(
        asking.asking("other"),
        None,
        "nothing is asked while one runs"
    );
    asking.finished(
        "release-issues",
        Some("Its validator is not owned by root.".into()),
    );
    assert!(!asking.acting("release-issues"));
    assert_eq!(
        asking.refusal("release-issues"),
        Some("Its validator is not owned by root.")
    );
    asking.ask("release-issues", Question::Approve);
    assert_eq!(
        asking.refusal("release-issues"),
        None,
        "asking again clears it"
    );
}

#[test]
fn a_question_keeps_its_place_while_it_is_being_answered() {
    let mut asking = Asking::default();
    asking.ask("github", Question::RemoveSource);
    assert_eq!(asking.shown("github"), Some(Question::RemoveSource));
    asking.confirm();
    assert_eq!(asking.asking("github"), None);
    assert_eq!(
        asking.shown("github"),
        Some(Question::RemoveSource),
        "its buttons stay, still, until the daemon answers"
    );
    assert_eq!(asking.shown("other"), None);
    asking.finished("github", None);
    assert_eq!(asking.shown("github"), None);
}

// ---- words (§8) ----

#[test]
fn the_panes_words_are_sentence_case_free_of_mac_and_of_wire_tokens() {
    for text in STRINGS {
        let first = text.chars().next().expect("not empty");
        let lower_starts = ["keeps", "paused", "items", "minutes"];
        assert!(
            first.is_uppercase() || lower_starts.iter().any(|w| text.starts_with(w)),
            "{text} starts a sentence"
        );
        assert!(!text.contains("Mac"), "{text} names the Mac");
        assert!(!text.contains("Keychain"), "{text} names the keychain");
        assert!(!text.contains('_'), "{text} carries a wire token");
        assert!(!text.contains("  "), "{text} has a double space");
    }
}

#[test]
fn the_pages_compose_their_lines_from_published_counts() {
    assert_eq!(
        words::source_last_day(3, 2),
        "3 received · 2 reached a watcher"
    );
    assert_eq!(words::items_a_day(0), "No limit");
    assert_eq!(words::items_a_day(500), "500");
    assert_eq!(words::waits(1800), "30 minutes, then the proposal expires");
    assert_eq!(words::acts_on(0), "items of any age");
    assert_eq!(words::acts_on(5400), "items at most 1 hour, 30 minutes old");
    assert_eq!(
        words::remove_source_question("eng-blogs", &["release-watch".into()]),
        "Remove eng-blogs? Its stored secrets are deleted. These watchers listen to it: \
         release-watch."
    );
    assert_eq!(
        words::push_example(4032, "home-alerts"),
        "curl -sS http://127.0.0.1:4032/cues/v1/home-alerts -H 'Authorization: Bearer <token>' \
         --data @item.json"
    );
    assert_eq!(
        words::token_label("home-alerts"),
        "Token for home-alerts, shown once"
    );
    assert_eq!(words::switch_label("gh-hooks"), "Turn gh-hooks on or off");
}

/// 2026-10-17 14:20 on the reader's clock.
const NOW: LocalTime = LocalTime {
    year: 2026,
    month: 10,
    day: 17,
    hour: 14,
    minute: 20,
};

fn at(year: i32, month: i32, day: i32, hour: i32, minute: i32) -> LocalTime {
    LocalTime {
        year,
        month,
        day,
        hour,
        minute,
    }
}

#[test]
fn an_instructions_day_is_the_day_alone_with_its_year_only_beyond_the_readers() {
    assert_eq!(words::day(at(2026, 10, 12, 9, 0), NOW), "Oct 12");
    assert_eq!(words::day(at(2026, 10, 17, 23, 59), NOW), "Oct 17");
    assert_eq!(words::day(at(2025, 12, 31, 23, 30), NOW), "Dec 31, 2025");
}

#[test]
fn a_waiting_proposal_reads_as_who_wants_to_run_what_and_until_when() {
    let mut list: ProposalList = decode(result("cues_proposals_list"));
    // The golden expires at 2026-10-17T12:20:00Z, which the app reads as 14:20 on a clock two
    // hours east of UTC; anything else is a time it cannot read.
    let expires = list.proposals[0].expires_at.clone();
    let written =
        |time: &str| (time == expires).then(|| written_time(at(2026, 10, 17, 14, 20), NOW));
    let lines = waiting_lines(&list, written);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].proposal_id, "cpr_9f2c41d07a5e6b38c1d2e4f0");
    assert_eq!(
        lines[0].title,
        "release-issues wants to run github_create_issue"
    );
    assert_eq!(lines[0].waits.as_deref(), Some("Waits until 14:20"));
    let tomorrow = |_: &str| Some(written_time(at(2026, 10, 18, 9, 5), NOW));
    assert_eq!(
        waiting_lines(&list, tomorrow)[0].waits.as_deref(),
        Some("Waits until Oct 18, 09:05"),
        "Chat's time rule"
    );
    list.proposals[0].expires_at = "soon".into();
    assert_eq!(
        waiting_lines(&list, written)[0].waits,
        None,
        "a time this app cannot read is not drawn"
    );
    let many = list.proposals[0].clone();
    list.proposals = vec![many; 150];
    assert_eq!(waiting_lines(&list, written).len(), 100, "at most 100");
}

/// Pinboard's private feed carries its secret in the path, a user-data stream its key; any
/// address can carry a user and password.
const SECRET_ADDRESS: &str =
    "https://reader:hunter2@feeds.pinboard.in/rss/secret:0123abcd/u:me/?token=abc";

/// M60 cues §4.4: a source's page names the feed or stream it reads by its host (`location`),
/// never by its address, which can carry a credential.
#[test]
fn a_sources_page_names_its_feed_by_its_host_and_never_its_address() {
    let feed = source(
        "eng-blogs",
        json!({"url": SECRET_ADDRESS, "location": "feeds.pinboard.in"}),
    );
    let facts = source_facts(&feed);
    let titles: Vec<&str> = facts.iter().map(|(title, _)| *title).collect();
    assert_eq!(
        titles,
        [
            words::KIND,
            words::ADDRESS_HOST,
            words::ITEMS_A_DAY,
            words::LAST_DAY,
            words::LISTENING
        ]
    );
    assert_eq!(facts[1].1, "feeds.pinboard.in");
    for (title, value) in &facts {
        for part in ["hunter2", "secret:0123abcd", "token", "://", "/rss/"] {
            assert!(!value.contains(part), "{title} draws {part}: {value}");
        }
    }
}

#[test]
fn a_source_without_a_host_draws_no_address_row() {
    let listed = sources();
    for webhook_or_push in &listed[1..] {
        let facts = source_facts(webhook_or_push);
        assert!(
            facts.iter().all(|(title, _)| *title != words::ADDRESS_HOST),
            "{}",
            webhook_or_push.name
        );
    }
    let blank = source("eng-blogs", json!({"location": ""}));
    assert!(source_facts(&blank)
        .iter()
        .all(|(title, _)| *title != words::ADDRESS_HOST));
}

/// The page is handed a row that does not hold the address at all, so no drawing of it can
/// reach the address: it is never decoded.
#[test]
fn a_source_row_never_holds_its_address() {
    let feed = source("eng-blogs", json!({"url": SECRET_ADDRESS}));
    let held = format!("{feed:?}");
    for part in ["hunter2", "secret:0123abcd", "token=abc"] {
        assert!(!held.contains(part), "the row holds {part}: {held}");
    }
}
