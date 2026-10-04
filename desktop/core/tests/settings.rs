//! Settings: the daemon's rows decoded from its own fixtures, and the pure rules
//! that turn a row into what a control shows and what a control sends back.

use fermix_client::management::{decode_response, CallError, Refusal};
use fermix_client::model::ModelPage;
use fermix_client::settings::{
    channel_word, choice_items, list_with, list_without, matching_panes, number_answer,
    number_view, pane, placeholder, read_failure, sections_for, text_answer, unlisted_models,
    value_width, with_listing, ApplyResult, Kind, ReloadResult, Row, SectionRows, Sections,
    FIELD_CHARS, GROUPS, KEPT_VALUE_CHARS, MODELS_UNLISTED, PANES, WIDEST_VALUE_CHARS,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::ErrorKind;

const SUCCESS: &str = include_str!("fixtures/management/success.jsonl");

fn fixtures() -> impl Iterator<Item = Value> {
    SUCCESS
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
}

fn decode<T: DeserializeOwned>(name: &str) -> T {
    let found = fixtures()
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"));
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    let result = decode_response(&serde_json::to_vec(response).unwrap(), id).unwrap();
    serde_json::from_value(result).unwrap_or_else(|e| panic!("{name} does not decode: {e}"))
}

fn row(value: Value) -> Row {
    let mut base = json!({
        "key": "k", "kind": "text", "label": "Label", "footer": null, "info": null,
        "value": null, "present": null, "options": [], "min": null, "max": null,
        "step": null, "restart": false, "read_only": false, "suggestions": false,
        "unit": null, "format": null
    });
    for (k, v) in value.as_object().unwrap() {
        base[k] = v.clone();
    }
    serde_json::from_value(base).unwrap()
}

#[test]
fn every_published_section_fixture_decodes() {
    let names: Vec<String> = fixtures()
        .filter_map(|v| v["name"].as_str().map(str::to_owned))
        .filter(|n| n.starts_with("settings_get"))
        .collect();
    assert!(names.len() >= 20, "the fixtures cover the sections");
    for name in names {
        let section: SectionRows = decode(&name);
        assert!(!section.rows.is_empty(), "{name} has rows");
    }
}

#[test]
fn a_newer_daemons_row_with_unknown_fields_and_kind_still_decodes() {
    let r = row(json!({"kind": "colour", "shade": "teal", "value": "#fff"}));
    assert_eq!(r.kind, Kind::Unknown);
    let old = row(json!({"kind": "toggle", "value": true}));
    assert_eq!(old.kind, Kind::Toggle);
    assert_eq!(old.info, None);
}

#[test]
fn sections_keep_the_daemons_order_and_linux_drops_computer_history() {
    let sections: Sections = decode("settings_sections");
    let computer: Vec<&str> = sections_for("computer", &sections.sections)
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(computer, ["computer_use"]);
    let providers: Vec<&str> = sections_for("providers", &sections.sections)
        .iter()
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(providers.first(), Some(&"providers.openai_codex"));
    assert_eq!(providers.last(), Some(&"routing"));
}

#[test]
fn the_fourteen_panes_sit_in_four_groups() {
    assert_eq!(PANES.len(), 14);
    assert_eq!(GROUPS.len(), 4);
    let grouped: usize = GROUPS.iter().map(|(_, panes)| panes.len()).sum();
    assert_eq!(grouped, 14);
    for (_, panes) in GROUPS {
        for slug in panes {
            assert!(pane(slug).is_some(), "{slug} is a pane");
        }
    }
    assert_eq!(pane("coding").unwrap().title, "Coding agents");
    // Where secrets are kept leads the System group.
    assert_eq!(
        GROUPS[3],
        ("System", &["secrets", "sandbox", "permissions"][..])
    );
    assert_eq!(pane("secrets").unwrap().title, "Secrets");
}

#[test]
fn a_percent_shows_as_a_whole_percentage_and_goes_back_as_a_fraction() {
    let r = row(json!({"kind": "number", "format": "percent", "value": 0.85,
        "min": 0.1, "max": 1.0, "step": 0.01}));
    let view = number_view(&r);
    assert_eq!(
        (view.value, view.min, view.max, view.step),
        (85.0, 10.0, 100.0, 1.0)
    );
    assert_eq!(view.digits, 0);
    assert_eq!(number_answer(&r, 70.0), json!(0.7));
    assert_eq!(number_answer(&r, 5.0), json!(0.1), "clamped to the minimum");
}

#[test]
fn a_whole_number_goes_back_as_a_json_integer_never_a_float() {
    let r = row(
        json!({"kind": "number", "format": "minutes", "unit": "minutes",
        "value": 30, "min": 1, "max": 240, "step": 1}),
    );
    let sent = number_answer(&r, 15.0);
    assert_eq!(serde_json::to_string(&sent).unwrap(), "15");
    assert_eq!(number_answer(&r, 15.4), json!(15), "snapped to the step");
    assert_eq!(
        number_answer(&r, 999.0),
        json!(240),
        "clamped to the maximum"
    );
    assert_eq!(number_view(&r).text(15.0), "15 minutes");
}

#[test]
fn an_open_ended_number_invents_no_ceiling() {
    let r = row(json!({"kind": "number", "format": "hours", "unit": "hours",
        "value": 24, "min": 0, "step": 1}));
    let view = number_view(&r);
    assert!(view.max >= 1.0e9, "no upper bound means effectively none");
    assert_eq!(number_answer(&r, 5000.0), json!(5000));
}

#[test]
fn cents_show_as_dollars_and_go_back_as_whole_cents() {
    let r = row(json!({"kind": "number", "format": "currency_cents",
        "value": 100, "min": 1, "step": 1}));
    let view = number_view(&r);
    assert_eq!(
        (view.value, view.min, view.step, view.digits),
        (1.0, 0.01, 0.01, 2)
    );
    assert_eq!(view.text(2.5), "$2.50");
    assert_eq!(view.parse("$3.25"), Some(3.25));
    assert_eq!(number_answer(&r, 2.5), json!(250));
}

#[test]
fn a_number_reads_with_its_unit_beside_it_and_types_back_with_or_without_it() {
    let percent = number_view(&row(json!({"kind": "number", "format": "percent",
        "value": 0.85, "min": 0.1, "max": 1.0, "step": 0.01})));
    assert_eq!(percent.text(85.0), "85%");
    assert_eq!(percent.parse("70 %"), Some(70.0));
    let hours = number_view(&row(json!({"kind": "number", "format": "hours",
        "unit": "hours", "value": 24, "min": 0, "step": 1})));
    assert_eq!(hours.text(24.0), "24 hours");
    assert_eq!(
        hours.text(1.0),
        "1 hours",
        "the daemon's unit word is shown as it is"
    );
    assert_eq!(hours.parse("12 hours"), Some(12.0));
    assert_eq!(hours.parse("12"), Some(12.0));
    assert_eq!(hours.parse("twelve"), None);
}

#[test]
fn a_closed_choice_marks_the_value_and_keeps_disabled_options_with_their_reason() {
    let r = row(json!({"kind": "choice", "value": "standard", "options": [
        {"value": "strict", "label": "Strict", "hint": "Reads only what you name", "disabled": false},
        {"value": "standard", "label": "Standard", "hint": null, "disabled": false},
        {"value": "local", "label": "On this device", "hint": "Not offered here", "disabled": true}
    ]}));
    let (items, selected) = choice_items(&r);
    assert_eq!(items.len(), 3);
    assert_eq!(selected, 1);
    assert!(items[2].disabled);
    assert_eq!(items[2].hint.as_deref(), Some("Not offered here"));
}

#[test]
fn a_value_off_the_list_shows_as_not_set_and_sends_nothing() {
    let r = row(json!({"kind": "choice", "value": "", "options": [
        {"value": "codex", "label": "Codex", "hint": null, "disabled": false}
    ]}));
    let (items, selected) = choice_items(&r);
    assert_eq!(selected, 0);
    assert_eq!(items[0].label, "Not set");
    assert_eq!(items[0].value, None);
}

#[test]
fn an_empty_text_shows_the_inherit_label_or_not_set() {
    let inherit = row(
        json!({"kind": "choice", "suggestions": true, "value": "", "options": [
            {"value": "", "label": "Same as main model", "hint": null, "disabled": false}
        ]}),
    );
    assert_eq!(placeholder(&inherit), "Same as main model");
    assert_eq!(placeholder(&row(json!({"value": ""}))), "Not set");
}

#[test]
fn text_is_sent_only_when_it_changed_and_blank_clears_it() {
    let r = row(json!({"value": "Fermix Notetaker"}));
    assert_eq!(text_answer(&r, "Fermix Notetaker"), None);
    assert_eq!(text_answer(&r, " Notes "), Some(json!("Notes")));
    assert_eq!(
        text_answer(&r, ""),
        Some(json!("")),
        "blank clears; null never does"
    );
    let unset = row(json!({"value": null}));
    assert_eq!(
        text_answer(&unset, "  "),
        None,
        "blank over unset changes nothing"
    );
}

#[test]
fn a_value_keeps_its_whole_text_up_to_the_kept_width_and_asks_for_the_rest() {
    let model = "gpt-realtime-2 · Realtime, integrated tools";
    let width = value_width(model, 1);
    assert_eq!(width.min_chars, KEPT_VALUE_CHARS);
    assert_eq!(
        width.natural_chars, 43,
        "the whole value, counted in characters"
    );
    assert!(!width.whole());

    let short = value_width("Low", 1);
    assert_eq!((short.min_chars, short.natural_chars), (3, 3));
    assert!(short.whole());
    let kept = value_width(&"x".repeat(KEPT_VALUE_CHARS as usize), 1);
    assert!(
        kept.whole(),
        "a value of exactly the kept width is never cut"
    );

    let style = "Answer in a few sentences, and go longer when the question needs it. ".repeat(2);
    let long = value_width(&style, 1);
    assert_eq!(long.natural_chars, WIDEST_VALUE_CHARS);
    assert!(long.min_chars < long.natural_chars);
}

#[test]
fn a_field_keeps_room_to_type_however_short_its_value() {
    let empty = value_width("", FIELD_CHARS);
    assert_eq!(
        (empty.min_chars, empty.natural_chars),
        (FIELD_CHARS, FIELD_CHARS)
    );
    let name = value_width("Fermix", FIELD_CHARS);
    assert_eq!(
        (name.min_chars, name.natural_chars),
        (FIELD_CHARS, FIELD_CHARS)
    );
    let zone = value_width("America/Argentina/Buenos_Aires", FIELD_CHARS);
    assert_eq!((zone.min_chars, zone.natural_chars), (KEPT_VALUE_CHARS, 30));
}

#[test]
#[should_panic(expected = "floor")]
fn a_floor_wider_than_the_kept_width_is_a_bug() {
    value_width("", KEPT_VALUE_CHARS + 1);
}

#[test]
fn a_list_adds_trimmed_new_names_and_removes_one() {
    let items = vec!["EXA_API_KEY".to_owned(), "BRAVE_API_KEY".to_owned()];
    assert_eq!(list_with(&items, "  "), None);
    assert_eq!(
        list_with(&items, "EXA_API_KEY"),
        None,
        "a duplicate adds nothing"
    );
    assert_eq!(
        list_with(&items, " TAVILY_API_KEY "),
        Some(vec![
            "EXA_API_KEY".into(),
            "BRAVE_API_KEY".into(),
            "TAVILY_API_KEY".into()
        ])
    );
    assert_eq!(
        list_without(&items, "EXA_API_KEY"),
        vec!["BRAVE_API_KEY".to_owned()]
    );
}

#[test]
fn search_finds_panes_by_title_and_by_rows_already_read() {
    let sections: Sections = decode("settings_sections");
    let memory: SectionRows = decode("settings_get_memory");
    let loaded = HashMap::from([(memory.id.clone(), memory)]);
    assert_eq!(
        matching_panes("sand", &sections.sections, &loaded),
        ["sandbox"]
    );
    assert_eq!(
        matching_panes("compact", &sections.sections, &loaded),
        ["memory"]
    );
    assert_eq!(
        matching_panes("TELEGRAM", &sections.sections, &loaded),
        ["channels"]
    );
    assert!(matching_panes("  ", &sections.sections, &loaded).len() == PANES.len());
    assert!(matching_panes("zzz", &sections.sections, &loaded).is_empty());
}

#[test]
fn a_channel_says_off_needs_setup_or_connected() {
    assert_eq!(channel_word(false, None), "Off");
    assert_eq!(channel_word(true, Some("setup_required")), "Needs setup");
    assert_eq!(channel_word(true, Some("ok")), "Connected");
}

#[test]
fn apply_and_reload_answers_decode() {
    let applied: ApplyResult = decode("settings_apply");
    assert_eq!(applied.applied, ["realtime_enabled"]);
    assert!(applied.restart.required);
    assert!(applied.side_effects.is_empty());
    let reloaded: ReloadResult = decode("settings_reload");
    assert_eq!(reloaded.config_state, "clear");
}

#[test]
fn a_section_that_could_not_be_read_says_why() {
    let refused = CallError::Refused(Refusal {
        code: "unknown_section".into(),
        sentence: "Fermix has no settings section called voices.".into(),
        field: None,
        reason: None,
    });
    assert_eq!(
        read_failure(&refused),
        "Fermix has no settings section called voices."
    );
    assert_eq!(
        read_failure(&CallError::Timeout),
        "Fermix is not responding."
    );
    assert_eq!(
        read_failure(&CallError::DaemonDown(ErrorKind::NotFound)),
        "Fermix is not running."
    );
    assert_eq!(
        read_failure(&CallError::Protocol("no rows".into())),
        "Fermix answered in a way this app does not understand."
    );
}

fn model_row(section: &SectionRows) -> &Row {
    section
        .rows
        .iter()
        .find(|r| r.key == "default_model")
        .expect("a provider section has a model row")
}

fn all_but_the_model_row(section: &SectionRows) -> Vec<Row> {
    section
        .rows
        .iter()
        .filter(|r| r.key != "default_model")
        .cloned()
        .collect()
}

#[test]
fn only_a_providers_empty_model_row_waits_for_its_models_to_be_listed() {
    let codex: SectionRows = decode("settings_get_providers_openai_codex");
    assert_eq!(unlisted_models(&codex), Some("openai_codex"));
    let anthropic: SectionRows = decode("settings_get_providers_anthropic");
    assert_eq!(
        unlisted_models(&anthropic),
        None,
        "a shipped catalog is the list"
    );
    let elsewhere = SectionRows {
        id: "voice".into(),
        title: "Voice".into(),
        rows: vec![row(
            json!({"key": "default_model", "kind": "choice", "suggestions": true}),
        )],
    };
    assert_eq!(
        unlisted_models(&elsewhere),
        None,
        "only a provider's own section is listed"
    );
}

#[test]
fn a_listing_fills_the_model_row_and_leaves_the_rest() {
    let codex: SectionRows = decode("settings_get_providers_openai_codex");
    let page: ModelPage = decode("providers_models_list");
    let drawn = with_listing(&codex, &Ok(page.models));
    let model = model_row(&drawn);
    let offered: Vec<(&str, &str)> = model
        .options
        .iter()
        .map(|o| (o.value.as_str(), o.label.as_str()))
        .collect();
    assert_eq!(
        offered,
        [
            ("claude-opus-5", "Claude Opus 5"),
            ("claude-sonnet-5", "Claude Sonnet 5")
        ]
    );
    assert!(model
        .options
        .iter()
        .all(|o| !o.disabled && o.hint.is_none()));
    assert_eq!(model.footer, None);
    assert_eq!(all_but_the_model_row(&drawn), all_but_the_model_row(&codex));
}

#[test]
fn a_failed_listing_says_why_under_the_model_row_and_offers_nothing() {
    let codex: SectionRows = decode("settings_get_providers_openai_codex");
    let drawn = with_listing(&codex, &Err(MODELS_UNLISTED.to_owned()));
    let model = model_row(&drawn);
    assert!(model.options.is_empty());
    assert_eq!(model.footer.as_deref(), Some(MODELS_UNLISTED));
    assert_eq!(all_but_the_model_row(&drawn), all_but_the_model_row(&codex));
}

#[test]
fn a_listing_never_replaces_a_shipped_catalog() {
    let anthropic: SectionRows = decode("settings_get_providers_anthropic");
    let page: ModelPage = decode("providers_models_list");
    assert_eq!(with_listing(&anthropic, &Ok(page.models)), anthropic);
    assert_eq!(
        with_listing(&anthropic, &Err(MODELS_UNLISTED.to_owned())),
        anthropic
    );
}
