//! The Cues forms (M60 cues §4.4, §4.6, §7 core): the source form's state (which choice is asked
//! for, whose rows are drawn, what was set and typed, what Add sends and stores, where a refusal
//! goes) and the mandate form (what an add sends, what an edit sends, pins keeping their type).
//! Every form and row comes from the vendored goldens.

use fermix_client::cues::calls::{FormChoice, SourceChoice};
use fermix_client::cues::form::{secret_id, SourceFormState, NAME_KEY, VERIFY_KEY};
use fermix_client::cues::mandate::{MandateForm, PinDraft};
use fermix_client::cues::wire::{
    MandateChoices, MandateList, MandateRow, PinValue, SecretField, SourceDetail, SourceForm,
};
use fermix_client::cues::words;
use fermix_client::settings::Kind;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");

fn result(name: &str) -> Value {
    SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"))["response"]["result"]
        .clone()
}

fn decode<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("decodes")
}

/// The webhook form the golden answers (GitHub preset), the stream form and the X accounts form.
fn webhook_form() -> SourceForm {
    decode(result("cues_sources_form"))
}

fn stream_form() -> SourceForm {
    decode(result("cues_sources_form_stream"))
}

fn x_accounts_form() -> SourceForm {
    decode(result("cues_sources_form_x_accounts"))
}

fn choice(kind: Option<&str>, preset: Option<&str>, verify: Option<&str>) -> FormChoice {
    FormChoice {
        kind: kind.map(str::to_owned),
        preset: preset.map(str::to_owned),
        verify: verify.map(str::to_owned),
    }
}

/// A form opened and answered for the GitHub webhook preset.
fn github() -> SourceFormState {
    let mut form = SourceFormState::default();
    form.take(&webhook_form(), &FormChoice::default());
    form.choose_kind("webhook");
    assert_eq!(form.request(), choice(None, Some("github"), None));
    form.take(&webhook_form(), &form.request());
    form
}

// ---- the source form ----

#[test]
fn the_first_answer_opens_on_the_first_kind_and_its_first_preset() {
    let mut form = SourceFormState::default();
    assert_eq!(
        form.request(),
        FormChoice::default(),
        "asked with no choice"
    );
    form.take(&webhook_form(), &FormChoice::default());
    assert_eq!(form.kind(), Some("feed"));
    assert_eq!(form.preset(), Some("rss"));
    assert_eq!(form.kinds().len(), 4);
    assert!(
        form.rows().is_empty(),
        "an answer to another choice draws no rows"
    );
    assert_eq!(form.request(), choice(None, Some("rss"), None));
    let presets: Vec<&str> = form.kind_presets().iter().map(|p| p.id.as_str()).collect();
    let answer = webhook_form();
    let feeds: Vec<&str> = answer
        .presets
        .iter()
        .filter(|p| p.kind == "feed")
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(presets, feeds, "the feed's presets, in the answer's order");
}

#[test]
fn x_accounts_is_a_feed_preset_asked_for_by_its_preset() {
    let mut form = SourceFormState::default();
    form.take(&x_accounts_form(), &FormChoice::default());
    form.choose_preset(Some("x_accounts"));
    assert_eq!(form.kind(), Some("feed"));
    assert_eq!(form.request(), choice(None, Some("x_accounts"), None));
    form.take(&x_accounts_form(), &form.request());
    let handles = form.rows().iter().find(|r| r.key == "handles");
    assert!(
        handles.is_some_and(|r| r.kind == Kind::List),
        "the handles are a list"
    );
}

#[test]
fn rows_come_only_from_the_answer_to_the_choice_held_now() {
    let mut form = SourceFormState::default();
    form.take(&webhook_form(), &FormChoice::default());
    let feed = form.request();
    form.choose_kind("webhook");
    form.take(&webhook_form(), &feed);
    assert!(form.rows().is_empty(), "the feed's answer came late");
    form.take(&webhook_form(), &form.request());
    assert_eq!(form.rows()[0].key, NAME_KEY);
    let answer = webhook_form();
    assert!(answer.consent_sentence.is_some(), "a consent line");
    let lines: Vec<String> = answer
        .consent_sentence
        .into_iter()
        .chain(answer.notes)
        .collect();
    assert!(lines.len() > 2, "a consent line and notes");
    assert_eq!(
        form.about(),
        Some(lines.join("\n\n")),
        "the consent line leads, then the notes, in order"
    );
}

#[test]
fn a_kind_without_presets_asks_with_the_kind_alone() {
    let mut form = SourceFormState::default();
    form.take(&webhook_form(), &FormChoice::default());
    form.choose_kind("stream");
    assert_eq!(form.preset(), None);
    assert_eq!(form.request(), choice(Some("stream"), None, None));
    form.choose_kind("local_push");
    assert_eq!(form.request(), choice(Some("local_push"), None, None));
    form.choose_kind("webhook");
    form.choose_preset(None);
    assert_eq!(
        form.request(),
        choice(Some("webhook"), None, Some("hmac")),
        "None is the kind, checked by a signature"
    );
}

#[test]
fn a_check_chosen_asks_again_and_a_new_kind_or_preset_resets_it() {
    let mut form = github();
    form.set(VERIFY_KEY, json!("standard_webhooks"));
    assert_eq!(
        form.request(),
        choice(None, Some("github"), Some("standard_webhooks"))
    );
    form.choose_preset(None);
    assert_eq!(
        form.request(),
        choice(Some("webhook"), None, Some("hmac")),
        "a webhook with no preset starts checked by a signature"
    );
    form.set(VERIFY_KEY, json!("shared_secret"));
    assert_eq!(
        form.request(),
        choice(Some("webhook"), None, Some("shared_secret"))
    );
    form.choose_kind("feed");
    assert_eq!(form.request(), choice(None, Some("rss"), None));
}

#[test]
fn what_was_set_for_a_field_the_new_form_lacks_goes_with_it() {
    let mut form = github();
    form.set(NAME_KEY, json!("gh-hooks"));
    form.set("header", json!("X-Signature"));
    form.hold_secret("secret", "typed-signing-secret");
    form.choose_kind("stream");
    form.take(&stream_form(), &form.request());
    assert_eq!(form.name(), "gh-hooks", "the name is on every form");
    assert!(!form.changed_values().contains_key("header"));
    let held: Vec<SecretField> = form.typed_secrets().iter().map(|s| s.field).collect();
    assert_eq!(held, [SecretField::Secret], "the stream has a secret row");
    form.hold_secret("secret", "typed-stream-secret");
    form.choose_kind("webhook");
    form.take(&webhook_form(), &form.request());
    form.choose_kind("stream");
    form.take(&stream_form(), &form.request());
    assert!(
        form.typed_secrets().is_empty(),
        "GitHub's form has no secret row, so the secret went with it"
    );
}

#[test]
fn add_sends_the_name_the_choice_and_the_changed_values_only() {
    let mut form = github();
    assert_eq!(form.spec(), None, "no name yet");
    form.set(NAME_KEY, json!("  gh-hooks "));
    form.set("header", json!("X-Signature"));
    form.set("algorithm", json!("sha256"));
    let spec = form.spec().expect("a name and a choice");
    assert_eq!(spec.name, "gh-hooks");
    assert_eq!(spec.choice, SourceChoice::Preset("github".into()));
    assert_eq!(
        Value::Object(spec.values),
        json!({"header": "X-Signature"}),
        "the name travels as ref; an unchanged value is not sent"
    );
    form.choose_preset(None);
    form.take(&webhook_form(), &form.request());
    let spec = form.spec().expect("still named");
    assert_eq!(spec.choice, SourceChoice::Kind("webhook".into()));
    assert_eq!(
        spec.values.get(VERIFY_KEY),
        Some(&json!("hmac")),
        "the check asked for goes with the add, whatever its row holds"
    );
}

#[test]
fn a_typed_secret_is_held_in_row_order_and_never_sent_with_the_source() {
    let mut form = SourceFormState::default();
    form.take(&stream_form(), &FormChoice::default());
    form.choose_kind("stream");
    form.take(&stream_form(), &form.request());
    form.set(NAME_KEY, json!("news"));
    form.hold_secret("secret", "the-api-secret");
    form.hold_secret("key", "the-api-key");
    let (spec, secrets) = form.prepare_add().expect("ready");
    assert!(!spec.values.contains_key("key") && !spec.values.contains_key("secret"));
    let fields: Vec<SecretField> = secrets.iter().map(|s| s.field).collect();
    assert_eq!(fields, [SecretField::Key, SecretField::Secret], "row order");
    assert_eq!(secrets[0].value.reveal(), "the-api-key");
    let shown = format!("{form:?} {secrets:?}");
    assert!(!shown.contains("the-api-key") && !shown.contains("the-api-secret"));
    assert_eq!(secret_id("news", SecretField::Key), "cue_source:news:key");
}

#[test]
fn a_secret_this_build_cannot_store_is_refused_under_its_row_and_nothing_is_sent() {
    let mut answer = stream_form();
    let secret_row = answer.rows.iter().find(|r| r.kind == Kind::Secret);
    let mut extra = secret_row.expect("a secret row").clone();
    extra.key = "token".into();
    answer.rows.push(extra);
    let mut form = SourceFormState::default();
    form.take(&answer, &FormChoice::default());
    form.choose_kind("stream");
    form.take(&answer, &form.request());
    form.set(NAME_KEY, json!("news"));
    form.hold_secret("token", "pasted");
    assert!(form.prepare_add().is_none());
    assert_eq!(form.refusal_for("token"), Some(words::SECRET_UNSTORABLE));
    form.hold_secret("token", "");
    assert!(form.prepare_add().is_some(), "cleared, it adds");
}

#[test]
fn a_refusal_goes_under_its_row_the_pickers_or_the_form_and_clears_with_its_field() {
    let mut form = github();
    form.refuse(Some("header"), "That header is not allowed.");
    assert_eq!(
        form.refusal_for("header"),
        Some("That header is not allowed.")
    );
    assert_eq!(form.form_refusal(), None);
    assert!(form.refused_folded(), "header is under More options");
    form.set("header", json!("X-Other"));
    assert_eq!(form.refusal_for("header"), None, "changing it clears it");
    form.refuse(Some("preset"), "No such preset.");
    assert_eq!(form.refusal_for("preset"), Some("No such preset."));
    assert_eq!(form.form_refusal(), None, "said under the pickers");
    form.refuse(Some("bogus"), "Something else.");
    assert_eq!(form.form_refusal(), Some("Something else."));
    form.refuse(None, "Fermix could not save it.");
    assert_eq!(form.form_refusal(), Some("Fermix could not save it."));
    assert!(!form.refused_folded());
}

#[test]
fn an_edit_is_the_sources_own_fields_without_its_secrets_and_sends_what_changed() {
    let detail: SourceDetail = decode(result("cues_sources_get"));
    let mut form = SourceFormState::editing(&detail);
    assert_eq!(form.editing_name(), Some("gh-hooks"));
    assert!(form.rows().iter().all(|r| r.key != "secret"));
    assert!(form.spec().is_none(), "an edit adds nothing");
    assert!(form.changed_values().is_empty());
    form.set("verify", json!("shared_secret"));
    assert_eq!(
        Value::Object(form.changed_values()),
        json!({"verify": "shared_secret"})
    );
}

// ---- the mandate form ----

fn choices() -> MandateChoices {
    decode(result("cues_mandates_choices"))
}

fn release_issues() -> MandateRow {
    decode::<MandateList>(result("cues_mandates_list")).mandates[0].clone()
}

#[test]
fn an_add_sends_every_field_and_its_options_only_when_set() {
    let mut form = MandateForm::add();
    assert!(!form.is_complete(), "no name or task");
    form.name = "release-issues".into();
    form.job = "release_watch_2".into();
    form.tools = ["github_create_issue", "github_search_issues"]
        .map(str::to_owned)
        .into();
    assert!(form.is_complete());
    assert_eq!(
        Value::Object(form.values(Some(&choices()))),
        json!({
            "job": "release_watch_2",
            "mode": "approve",
            "tools": ["github_create_issue"],
            "read_tools": ["github_search_issues"],
            "approve_within_s": 1800,
            "max_cue_age_s": 900,
        }),
        "the golden add, read-only tools apart, minutes as seconds"
    );
    form.validator = " /usr/local/bin/check ".into();
    form.pins.push(PinDraft::new("repo", "fermix"));
    let values = form.values(Some(&choices()));
    assert_eq!(values["validator"], json!("/usr/local/bin/check"));
    assert_eq!(
        values["pin_args"],
        json!([{"key": "repo", "value": "fermix"}])
    );
    form.waits = "half an hour".into();
    assert!(!form.is_complete(), "minutes are whole numbers");
}

#[test]
fn an_edit_sends_only_what_changed_from_the_form_it_opened_on() {
    let mut row = release_issues();
    row.approve_within_s = 1830;
    let mut form = MandateForm::editing(&row);
    assert_eq!(form.waits, "30");
    assert!(
        form.values(Some(&choices())).is_empty(),
        "an untouched rounded duration is not rewritten"
    );
    form.acts_on = "60".into();
    form.mode = "auto".into();
    assert_eq!(
        Value::Object(form.values(Some(&choices()))),
        json!({"max_cue_age_s": 3600, "mode": "auto"})
    );
    assert!(!form.more_options, "no option was set");
}

#[test]
fn a_tool_no_longer_listed_keeps_its_side_and_a_pin_keeps_its_type() {
    let mut row = release_issues();
    row.read_tools.push("old_search".into());
    row.pin_args = vec![
        decode(json!({"key": "limit", "value": 5})),
        decode(json!({"key": "draft", "value": true})),
    ];
    let mut form = MandateForm::editing(&row);
    assert!(form.more_options, "pins are options");
    assert_eq!(form.pins[0].text, "5");
    form.name = "renamed".into();
    let values = form.values(Some(&choices()));
    assert!(
        !values.contains_key("read_tools") && !values.contains_key("pin_args"),
        "{values:?}"
    );
    form.pins[1].text = "false".into();
    let values = form.values(Some(&choices()));
    assert_eq!(
        values["pin_args"],
        json!([{"key": "limit", "value": 5}, {"key": "draft", "value": "false"}]),
        "an untouched pin keeps its number; a retyped one is text"
    );
    let mut only_unlisted = MandateForm::editing(&row);
    only_unlisted.tools.remove("github_search_issues");
    let values = only_unlisted.values(Some(&choices()));
    assert_eq!(values["read_tools"], json!(["old_search"]));
    assert_eq!(
        PinDraft::new("x", "1").value(),
        PinValue::Text("1".into()),
        "typed is text"
    );
}

#[test]
fn a_mandate_refusal_goes_under_the_key_it_names_or_under_the_form() {
    let mut form = MandateForm::editing(&release_issues());
    form.refuse(Some("approve_within_s"), "Too long.");
    assert_eq!(form.refusal_for("approve_within_s"), Some("Too long."));
    assert_eq!(form.form_refusal(), None);
    form.refuse(Some("elsewhere"), "Refused.");
    assert_eq!(form.form_refusal(), Some("Refused."));
    form.refuse(None, "Refused.");
    assert_eq!(form.form_refusal(), Some("Refused."));
}

fn keys(form: &MandateForm) -> Vec<String> {
    form.rows(Some(&choices()))
        .into_iter()
        .map(|r| r.key)
        .collect()
}

#[test]
fn the_editor_draws_the_mandates_keys_and_its_tools_as_rows() {
    let form = MandateForm::add();
    assert_eq!(
        keys(&form),
        [
            "name",
            "job",
            "mode",
            "tools",
            "tool:github_create_issue",
            "tool:github_search_issues",
            "approve_within_s",
            "max_cue_age_s",
            "validator",
            "interpreter",
            "validator_env",
            "idempotency_arg",
            "unpinned",
            "pin_args",
        ]
    );
    let rows = form.rows(Some(&choices()));
    assert!(!rows[0].read_only, "a new mandate is named");
    assert_eq!(rows[1].options[0].label, words::CHOOSE_TASK);
    assert!(rows[1].options[0].disabled, "choosing no task is no choice");
    assert_eq!(rows[3].value, json!(words::NONE), "no tool chosen yet");
    assert_eq!(rows[4].footer.as_deref(), Some(words::CHANGES_THINGS));
    assert_eq!(rows[5].footer.as_deref(), Some(words::READ_ONLY));
    assert_eq!(
        (rows[6].value.clone(), rows[7].value.clone()),
        (json!(30), json!(15))
    );
    assert!(rows[8..].iter().all(|r| r.advanced) && !rows[..8].iter().any(|r| r.advanced));
    let nothing = MandateForm::add().rows(None);
    assert_eq!(nothing[3].value, json!(words::NO_TOOLS));

    let mut row = release_issues();
    row.tools.push("retired_tool".into());
    row.job.id = "gone_job".into();
    let edit = MandateForm::editing(&row);
    let rows = edit.rows(Some(&choices()));
    assert!(rows[0].read_only, "a mandate's name is fixed");
    assert_eq!(
        rows[1].options[0].value, "gone_job",
        "an unknown task is kept"
    );
    assert!(!rows[1].options[0].disabled);
    assert!(
        keys(&edit).contains(&"tool:retired_tool".to_owned()),
        "kept, unasked"
    );
}

#[test]
fn each_row_writes_its_field_back_and_clears_its_refusal() {
    let mut form = MandateForm::add();
    assert!(form.set("name", &json!("release-issues")));
    assert!(form.set("job", &json!("release_watch_2")));
    assert!(form.set("mode", &json!("auto")));
    assert!(form.set("tool:github_search_issues", &json!(true)));
    assert!(form.set("approve_within_s", &json!(45.0)));
    assert!(form.set("validator_env", &json!(["HOME"])));
    assert!(!form.set("elsewhere", &json!(1)), "no such row");
    assert!(!form.set("tool:", &json!(true)), "a tool is named");
    assert_eq!(form.waits, "45");
    assert!(form.is_complete());
    let values = form.values(Some(&choices()));
    assert_eq!(values["mode"], json!("auto"));
    assert_eq!(values["read_tools"], json!(["github_search_issues"]));
    assert_eq!(values["approve_within_s"], json!(2700));
    assert_eq!(values["validator_env"], json!(["HOME"]));
    form.set("max_cue_age_s", &json!(2.5));
    assert!(!form.is_complete(), "minutes are whole");

    form.refuse(Some("read_tools"), "Not read only.");
    assert_eq!(form.row_refusal("tools"), Some("Not read only."));
    assert_eq!(form.form_refusal(), None, "said under the tools");
    form.set("tool:github_search_issues", &json!(false));
    assert_eq!(form.row_refusal("tools"), None, "a tool's change clears it");
    form.refuse(Some("tools"), "Too many.");
    form.set("tool:github_create_issue", &json!(true));
    assert_eq!(form.row_refusal("tools"), None);
    form.refuse(Some("job"), "No such task.");
    form.set("mode", &json!("approve"));
    assert_eq!(
        form.row_refusal("job"),
        Some("No such task."),
        "another field keeps it"
    );
    form.set("job", &json!("release_watch_2"));
    assert_eq!(form.row_refusal("job"), None);
}

#[test]
fn pins_are_listed_as_key_and_value_and_keep_their_type_while_unchanged() {
    let mut row = release_issues();
    row.pin_args = vec![decode(json!({"key": "limit", "value": 5}))];
    let mut form = MandateForm::editing(&row);
    let listed = form.rows(Some(&choices())).pop().expect("pins last");
    assert_eq!(listed.value, json!(["limit=5"]));
    assert_eq!(listed.example.as_deref(), Some("key=value"));
    form.set("pin_args", &json!(["limit=5", "label=a=b", "bare"]));
    assert_eq!(
        Value::Object(form.values(Some(&choices()))),
        json!({"pin_args": [
            {"key": "limit", "value": 5},
            {"key": "label", "value": "a=b"},
            {"key": "bare", "value": ""},
        ]}),
        "the number stays a number; the value is all after the first ="
    );
}
