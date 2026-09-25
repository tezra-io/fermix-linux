//! Settings: the daemon's rows decoded from its own fixtures, and the pure rules
//! that turn a row into what a control shows and what a control sends back.

use fermix_client::management::decode_response;
use fermix_client::settings::{
    channel_word, choice_items, list_with, list_without, matching_panes, number_answer,
    number_view, pane, placeholder, sections_for, text_answer, ApplyResult, Kind, ReloadResult,
    Row, SectionRows, Sections, GROUPS, PANES,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;

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
fn the_thirteen_panes_sit_in_four_groups() {
    assert_eq!(PANES.len(), 13);
    assert_eq!(GROUPS.len(), 4);
    let grouped: usize = GROUPS.iter().map(|(_, panes)| panes.len()).sum();
    assert_eq!(grouped, 13);
    for (_, panes) in GROUPS {
        for slug in panes {
            assert!(pane(slug).is_some(), "{slug} is a pane");
        }
    }
    assert_eq!(pane("coding").unwrap().title, "Coding agents");
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
    assert!(matching_panes("  ", &sections.sections, &loaded).len() == 13);
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
