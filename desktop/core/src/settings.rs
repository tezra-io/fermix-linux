//! Settings as the daemon publishes them: sections of typed rows. The daemon
//! owns every label, value and rule; this module only turns a row into what a
//! control shows and a control's value back into what `settings.apply` takes.
//! Rows decode tolerantly, so a newer daemon's fields and kinds never break it.

use crate::model::RestartState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// Sections the daemon publishes that this platform has no feature for.
const HIDDEN_ON_LINUX: [&str; 1] = ["computer_history"];

/// Stands in for an upper bound the daemon leaves open.
const NO_CEILING: f64 = 1.0e12;

pub const NOT_SET: &str = "Not set";
pub const UNKNOWN_KIND: &str = "This copy of Fermix is older than the daemon and has no control \
    for this setting. Update Fermix to change it here.";

pub struct Pane {
    pub slug: &'static str,
    pub title: &'static str,
    pub icon: &'static str,
}

pub const PANES: [Pane; 13] = [
    pane_of("providers", "Providers", "dialog-password-symbolic"),
    pane_of("personality", "Personality", "avatar-default-symbolic"),
    pane_of("memory", "Memory", "document-open-recent-symbolic"),
    pane_of("channels", "Channels", "mail-send-symbolic"),
    pane_of(
        "integrations",
        "Integrations",
        "application-x-addon-symbolic",
    ),
    pane_of("voice", "Voice", "audio-input-microphone-symbolic"),
    pane_of("meetings", "Meetings", "x-office-calendar-symbolic"),
    pane_of("computer", "Computer", "video-display-symbolic"),
    pane_of("coding", "Coding agents", "utilities-terminal-symbolic"),
    pane_of("search", "Search", "system-search-symbolic"),
    pane_of("images", "Images", "image-x-generic-symbolic"),
    pane_of("sandbox", "Sandbox", "security-medium-symbolic"),
    pane_of("permissions", "Permissions", "security-high-symbolic"),
];

const fn pane_of(slug: &'static str, title: &'static str, icon: &'static str) -> Pane {
    Pane { slug, title, icon }
}

pub const GROUPS: [(&str, &[&str]); 4] = [
    ("Assistant", &["providers", "personality", "memory"]),
    ("Connections", &["channels", "integrations"]),
    (
        "Capabilities",
        &[
            "voice", "meetings", "computer", "coding", "search", "images",
        ],
    ),
    ("System", &["sandbox", "permissions"]),
];

pub fn pane(slug: &str) -> Option<&'static Pane> {
    PANES.iter().find(|p| p.slug == slug)
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Section {
    pub id: String,
    pub pane: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Sections {
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SectionRows {
    pub id: String,
    pub title: String,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Toggle,
    Choice,
    Text,
    Number,
    Secret,
    List,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Row {
    pub key: String,
    pub kind: Kind,
    pub label: String,
    pub footer: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub present: Option<bool>,
    #[serde(default)]
    pub options: Vec<ChoiceOption>,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub suggestions: bool,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChoiceOption {
    pub value: String,
    pub label: String,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ApplyResult {
    pub applied: Vec<String>,
    pub restart: RestartState,
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReloadResult {
    pub restart: RestartState,
    pub config_state: String,
}

/// The sections a pane draws, in the daemon's order, minus those this platform lacks.
pub fn sections_for<'a>(pane: &str, sections: &'a [Section]) -> Vec<&'a Section> {
    sections
        .iter()
        .filter(|s| s.pane == pane && !HIDDEN_ON_LINUX.contains(&s.id.as_str()))
        .collect()
}

/// A number as its control shows it: a fraction as a percentage, cents as dollars.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberView {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub digits: u32,
    pub affix: Affix,
}

/// What sits beside the figure.
#[derive(Debug, Clone, PartialEq)]
pub enum Affix {
    None,
    Percent,
    Dollars,
    /// The daemon's unit word, shown as it is.
    Unit(String),
}

impl NumberView {
    pub fn text(&self, shown: f64) -> String {
        let digits = usize::try_from(self.digits).expect("small");
        let figure = format!("{shown:.digits$}");
        match &self.affix {
            Affix::None => figure,
            Affix::Percent => format!("{figure}%"),
            Affix::Dollars => format!("${figure}"),
            Affix::Unit(unit) => format!("{figure} {unit}"),
        }
    }

    /// What was typed, with or without its affix; `None` when it is not a number.
    pub fn parse(&self, typed: &str) -> Option<f64> {
        let mut text = typed.trim();
        let affix = match &self.affix {
            Affix::None => "",
            Affix::Percent => "%",
            Affix::Dollars => "$",
            Affix::Unit(unit) => unit.as_str(),
        };
        if !affix.is_empty() {
            text = text
                .trim_start_matches(affix)
                .trim_end_matches(affix)
                .trim();
        }
        text.parse::<f64>().ok().filter(|n| n.is_finite())
    }
}

/// How many shown units one wire unit is, and what the control shows beside it.
fn measure(row: &Row) -> (f64, Affix) {
    match (row.format.as_deref(), &row.unit) {
        (Some("percent"), _) => (100.0, Affix::Percent),
        (Some("currency_cents"), _) => (0.01, Affix::Dollars),
        (_, Some(unit)) if !unit.is_empty() => (1.0, Affix::Unit(unit.clone())),
        _ => (1.0, Affix::None),
    }
}

/// Whether the wire value is a whole number, which the daemon insists go as an integer.
fn whole(row: &Row) -> bool {
    let whole_format = matches!(
        row.format.as_deref(),
        Some("integer" | "minutes" | "hours" | "currency_cents")
    );
    whole_format || row.step.is_some_and(|s| s.fract() == 0.0)
}

pub fn number_view(row: &Row) -> NumberView {
    assert_eq!(row.kind, Kind::Number, "{} is not a number row", row.key);
    let (scale, affix) = measure(row);
    let step = row.step.filter(|s| *s > 0.0).unwrap_or(1.0) * scale;
    NumberView {
        value: row.value.as_f64().unwrap_or(0.0) * scale,
        min: row.min.map_or(-NO_CEILING, |m| m * scale),
        max: row.max.map_or(NO_CEILING, |m| m * scale),
        step,
        digits: decimals(step),
        affix,
    }
}

/// Decimal places needed to show a step exactly, at most 4.
fn decimals(step: f64) -> u32 {
    (0..4)
        .find(|d| {
            let scaled = step * 10f64.powi(*d);
            (scaled - scaled.round()).abs() < 1e-9
        })
        .map_or(4, |d| u32::try_from(d).expect("small"))
}

/// The wire value for what the control shows: clamped, snapped to the step, and
/// an integer whenever the setting counts whole things.
pub fn number_answer(row: &Row, shown: f64) -> Value {
    assert!(shown.is_finite(), "a control never holds a non-number");
    let (scale, _) = measure(row);
    let step = row.step.filter(|s| *s > 0.0).unwrap_or(1.0);
    let raw = shown / scale;
    let snapped = (raw / step).round() * step;
    let low = row.min.unwrap_or(f64::MIN);
    let high = row.max.unwrap_or(f64::MAX);
    let clamped = snapped.clamp(low, high);
    if whole(row) {
        #[allow(clippy::cast_possible_truncation)]
        return json!(clamped.round() as i64);
    }
    let places = 10f64.powi(i32::try_from(decimals(step)).expect("small"));
    json!((clamped * places).round() / places)
}

/// One entry of a choice control. `value: None` is the "Not set" stand-in for a
/// value the options do not hold; choosing it sends nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceItem {
    pub value: Option<String>,
    pub label: String,
    pub hint: Option<String>,
    pub disabled: bool,
}

/// The items a closed choice offers, and which one is selected.
pub fn choice_items(row: &Row) -> (Vec<ChoiceItem>, usize) {
    let current = row.value.as_str().unwrap_or("");
    let mut items: Vec<ChoiceItem> = row
        .options
        .iter()
        .map(|o| ChoiceItem {
            value: Some(o.value.clone()),
            label: o.label.clone(),
            hint: o.hint.clone(),
            disabled: o.disabled,
        })
        .collect();
    if let Some(index) = row.options.iter().position(|o| o.value == current) {
        return (items, index);
    }
    items.insert(
        0,
        ChoiceItem {
            value: None,
            label: NOT_SET.into(),
            hint: None,
            disabled: true,
        },
    );
    (items, 0)
}

/// What an empty field says: the label of the row's inherit option, else "Not set".
pub fn placeholder(row: &Row) -> String {
    row.options
        .iter()
        .find(|o| o.value.is_empty())
        .map_or_else(|| NOT_SET.to_owned(), |o| o.label.clone())
}

pub fn text_value(row: &Row) -> String {
    row.value.as_str().unwrap_or("").to_owned()
}

/// The value to send for what was typed, or `None` when nothing changed.
/// Blank clears a text setting; `null` is never sent, as the daemon refuses it.
pub fn text_answer(row: &Row, typed: &str) -> Option<Value> {
    let typed = typed.trim();
    (typed != text_value(row).trim()).then(|| json!(typed))
}

pub fn list_items(row: &Row) -> Vec<String> {
    row.value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The whole list with one more entry, or `None` when the entry is blank or already there.
pub fn list_with(items: &[String], new: &str) -> Option<Vec<String>> {
    let new = new.trim();
    if new.is_empty() || items.iter().any(|i| i == new) {
        return None;
    }
    let mut next = items.to_vec();
    next.push(new.to_owned());
    Some(next)
}

pub fn list_without(items: &[String], gone: &str) -> Vec<String> {
    items.iter().filter(|i| *i != gone).cloned().collect()
}

/// Panes whose title, section titles or already-read row labels hold `query`.
/// A blank query matches every pane. Footers are not searched, and nothing is read for it.
pub fn matching_panes(
    query: &str,
    sections: &[Section],
    loaded: &HashMap<String, SectionRows>,
) -> Vec<&'static str> {
    let query = query.trim().to_lowercase();
    let holds = |text: &str| text.to_lowercase().contains(&query);
    PANES
        .iter()
        .filter(|p| {
            holds(p.title)
                || sections_for(p.slug, sections).iter().any(|s| {
                    holds(&s.title)
                        || loaded
                            .get(&s.id)
                            .is_some_and(|rows| rows.rows.iter().any(|r| holds(&r.label)))
                })
        })
        .map(|p| p.slug)
        .collect()
}

/// A channel's state in words. The daemon publishes only a status atom.
pub fn channel_word(enabled: bool, status: Option<&str>) -> &'static str {
    match (enabled, status) {
        (false, _) => "Off",
        (true, Some("ok")) => "Connected",
        (true, _) => "Needs setup",
    }
}
