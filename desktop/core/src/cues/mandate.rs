//! The mandate form, as a value (M60 cues §4.6, L5). The engine publishes no mandate form, so the
//! form is written from the mandate's value keys and fed by `cues.mandates.choices` and, on an
//! edit, the mandate's own row. An add sends every field, options only where set; an edit sends
//! only what changed from the form it opened on, so a duration rounded to minutes is never
//! rewritten by an edit that did not touch it.

use super::wire::{MandateChoices, MandateMode, MandateRow, PinValue, Word};
use super::words;
use crate::settings::{ChoiceOption, Kind, Row};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// Every key a mandate write carries, which is what a refusal can name.
pub const FIELDS: [&str; 13] = [
    "name",
    "job",
    "mode",
    "tools",
    "read_tools",
    "approve_within_s",
    "max_cue_age_s",
    "validator",
    "interpreter",
    "validator_env",
    "idempotency_arg",
    "unpinned",
    "pin_args",
];

/// The keys an add sends only where they hold something.
const OPTIONS: [&str; 6] = [
    "validator",
    "interpreter",
    "validator_env",
    "idempotency_arg",
    "unpinned",
    "pin_args",
];

/// The mode words the form sends.
pub const APPROVE: &str = "approve";
pub const AUTO: &str = "auto";

/// A tool's switch is the row `tool:<name>`.
const TOOL: &str = "tool:";
/// A pinned argument as one item of its list.
pub const PIN_EXAMPLE: &str = "key=value";

/// One pinned argument as typed. A value the mandate already held keeps its own type until its
/// text changes; a typed value is text.
#[derive(Debug, Clone, PartialEq)]
pub struct PinDraft {
    pub key: String,
    pub text: String,
    original: Option<PinValue>,
}

impl PinDraft {
    pub fn new(key: &str, text: &str) -> PinDraft {
        PinDraft {
            key: key.to_owned(),
            text: text.to_owned(),
            original: None,
        }
    }

    pub fn value(&self) -> PinValue {
        match &self.original {
            Some(original) if pin_text(original) == self.text => original.clone(),
            _ => PinValue::Text(self.text.clone()),
        }
    }
}

/// A pinned value as the owner reads and types it.
pub fn pin_text(value: &PinValue) -> String {
    match value {
        PinValue::Text(text) => text.clone(),
        PinValue::Flag(flag) => flag.to_string(),
        PinValue::Number(number) => number.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MandateForm {
    /// The mandate an edit changes; None on an add.
    original: Option<MandateRow>,
    pub name: String,
    pub job: String,
    /// The mode's word: `approve` or `auto`, or an unknown one an edit keeps.
    pub mode: String,
    /// Every tool chosen, whether it reads or changes things.
    pub tools: BTreeSet<String>,
    /// Whole minutes, as typed.
    pub waits: String,
    pub acts_on: String,
    pub validator: String,
    pub interpreter: String,
    pub validator_env: Vec<String>,
    pub idempotency_arg: String,
    pub unpinned: Vec<String>,
    pub pins: Vec<PinDraft>,
    /// More options starts open on an edit of a mandate that sets any.
    pub more_options: bool,
    refusal: Option<(Option<String>, String)>,
}

impl MandateForm {
    pub fn add() -> MandateForm {
        MandateForm {
            original: None,
            name: String::new(),
            job: String::new(),
            mode: APPROVE.to_owned(),
            tools: BTreeSet::new(),
            waits: "30".to_owned(),
            acts_on: "15".to_owned(),
            validator: String::new(),
            interpreter: String::new(),
            validator_env: Vec::new(),
            idempotency_arg: String::new(),
            unpinned: Vec::new(),
            pins: Vec::new(),
            more_options: false,
            refusal: None,
        }
    }

    pub fn editing(row: &MandateRow) -> MandateForm {
        let validator = row.validator.as_ref();
        MandateForm {
            original: Some(row.clone()),
            name: row.name.clone(),
            job: row.job.id.clone(),
            mode: mode_word(&row.mode),
            tools: row.tools.iter().chain(&row.read_tools).cloned().collect(),
            waits: (row.approve_within_s / 60).to_string(),
            acts_on: (row.max_cue_age_s / 60).to_string(),
            validator: validator.map(|v| v.path.clone()).unwrap_or_default(),
            interpreter: validator
                .and_then(|v| v.interpreter.clone())
                .unwrap_or_default(),
            validator_env: validator.map(|v| v.env.clone()).unwrap_or_default(),
            idempotency_arg: row.idempotency_arg.clone().unwrap_or_default(),
            unpinned: row.unpinned.clone(),
            pins: row
                .pin_args
                .iter()
                .map(|p| PinDraft {
                    key: p.key.clone(),
                    text: pin_text(&p.value),
                    original: Some(p.value.clone()),
                })
                .collect(),
            more_options: validator.is_some()
                || row.idempotency_arg.is_some()
                || !row.unpinned.is_empty()
                || !row.pin_args.is_empty(),
            refusal: None,
        }
    }

    /// The mandate an edit changes.
    pub fn original(&self) -> Option<&MandateRow> {
        self.original.as_ref()
    }

    /// Whether the form holds enough to send: a name, a task and two whole numbers of minutes.
    pub fn is_complete(&self) -> bool {
        !self.name.trim().is_empty()
            && !self.job.is_empty()
            && minutes(&self.waits).is_some()
            && minutes(&self.acts_on).is_some()
    }

    /// What a write sends: every field on an add, options only where set; on an edit, the
    /// fields that differ from the form it opened on.
    pub fn values(&self, choices: Option<&MandateChoices>) -> Map<String, Value> {
        let every = self.all_values(choices);
        let Some(original) = &self.original else {
            return every
                .into_iter()
                .filter(|(key, value)| !OPTIONS.contains(&key.as_str()) || !is_empty(value))
                .collect();
        };
        let opened = MandateForm::editing(original).all_values(choices);
        every
            .into_iter()
            .filter(|(key, value)| opened.get(key) != Some(value))
            .collect()
    }

    /// A tool goes to `read_tools` where the choices mark it read only, or where the mandate
    /// already had it there and the choices no longer list it.
    fn all_values(&self, choices: Option<&MandateChoices>) -> Map<String, Value> {
        let tools = choices.map_or(&[][..], |c| c.tools.as_slice());
        let was_read = |name: &str| {
            self.original
                .as_ref()
                .is_some_and(|o| o.read_tools.iter().any(|t| t == name))
        };
        let reads: BTreeSet<&String> = self
            .tools
            .iter()
            .filter(|name| match tools.iter().find(|t| &t.name == *name) {
                Some(tool) => tool.read_only,
                None => was_read(name),
            })
            .collect();
        let changes: Vec<&String> = self.tools.iter().filter(|t| !reads.contains(t)).collect();
        let pins: Vec<Value> = self
            .pins
            .iter()
            .filter(|p| !p.key.is_empty())
            .map(|p| json!({"key": p.key, "value": p.value()}))
            .collect();
        let mut values = Map::new();
        values.insert("job".into(), json!(self.job));
        values.insert("mode".into(), json!(self.mode));
        values.insert("tools".into(), json!(changes));
        values.insert("read_tools".into(), json!(reads));
        values.insert("approve_within_s".into(), json!(seconds(&self.waits)));
        values.insert("max_cue_age_s".into(), json!(seconds(&self.acts_on)));
        values.insert("validator".into(), json!(self.validator.trim()));
        values.insert("interpreter".into(), json!(self.interpreter.trim()));
        values.insert("validator_env".into(), json!(self.validator_env));
        values.insert("idempotency_arg".into(), json!(self.idempotency_arg.trim()));
        values.insert("unpinned".into(), json!(self.unpinned));
        values.insert("pin_args".into(), Value::Array(pins));
        values
    }

    /// The editor's rows (§4.6): the name (fixed on an edit), the task, the mode, the tools with
    /// a switch each, the two durations in minutes, and the options under More options. The
    /// mode row is drawn by the app as a toggle.
    pub fn rows(&self, choices: Option<&MandateChoices>) -> Vec<Row> {
        let mut rows = vec![self.name_row(), self.job_row(choices), self.mode_row()];
        rows.push(self.tools_row(choices));
        rows.extend(self.tool_rows(choices));
        rows.push(minutes_row(
            "approve_within_s",
            words::WAITS,
            words::WAITS_INFO,
            &self.waits,
        ));
        let acts_on = minutes_row(
            "max_cue_age_s",
            words::ACTS_ON,
            words::ACTS_ON_INFO,
            &self.acts_on,
        );
        rows.push(acts_on);
        rows.extend(self.option_rows());
        rows
    }

    /// One row's value as it changed. False for a key that is none of the editor's rows. A
    /// refusal clears when the field it named changes.
    pub fn set(&mut self, key: &str, value: &Value) -> bool {
        let text = || value.as_str().unwrap_or_default().to_owned();
        match key {
            "name" => self.name = text(),
            "job" => self.job = text(),
            "mode" => self.mode = text(),
            "approve_within_s" => self.waits = whole_minutes(value),
            "max_cue_age_s" => self.acts_on = whole_minutes(value),
            "validator" => self.validator = text(),
            "interpreter" => self.interpreter = text(),
            "idempotency_arg" => self.idempotency_arg = text(),
            "validator_env" => self.validator_env = items(value),
            "unpinned" => self.unpinned = items(value),
            "pin_args" => self.pins = self.pins_from(&items(value)),
            _ => return self.set_tool(key, value),
        }
        self.clear_refusal_on(key);
        true
    }

    fn set_tool(&mut self, key: &str, value: &Value) -> bool {
        let Some(name) = key.strip_prefix(TOOL).filter(|n| !n.is_empty()) else {
            return false;
        };
        if value.as_bool().unwrap_or(false) {
            self.tools.insert(name.to_owned());
        } else {
            self.tools.remove(name);
        }
        self.clear_refusal_on("tools");
        self.clear_refusal_on("read_tools");
        true
    }

    /// The refusal drawn under the row `key`. The tools' row carries what was said of the
    /// tools and of the read tools.
    pub fn row_refusal(&self, key: &str) -> Option<&str> {
        match key {
            "tools" => self.refusal_for("tools").or(self.refusal_for("read_tools")),
            _ => self.refusal_for(key),
        }
    }

    fn clear_refusal_on(&mut self, key: &str) {
        if self.refusal_for(key).is_some() {
            self.refusal = None;
        }
    }

    pub fn refuse(&mut self, field: Option<&str>, sentence: &str) {
        assert!(!sentence.is_empty(), "a refusal says why");
        self.refusal = Some((field.map(str::to_owned), sentence.to_owned()));
    }

    pub fn clear_refusal(&mut self) {
        self.refusal = None;
    }

    /// The daemon's refusal under the field it names.
    pub fn refusal_for(&self, key: &str) -> Option<&str> {
        let (field, sentence) = self.refusal.as_ref()?;
        (field.as_deref() == Some(key)).then_some(sentence.as_str())
    }

    /// A refusal that names none of the form's fields, said under the form.
    pub fn form_refusal(&self) -> Option<&str> {
        let (field, sentence) = self.refusal.as_ref()?;
        let placed = field.as_deref().is_some_and(|f| FIELDS.contains(&f));
        (!placed).then_some(sentence.as_str())
    }
}

// ---- the editor's rows ----

impl MandateForm {
    fn name_row(&self) -> Row {
        let mut row = row("name", Kind::Text, words::NAME, json!(self.name));
        row.read_only = self.original.is_some();
        row.info = (!row.read_only).then(|| words::NAME_INFO.to_owned());
        row
    }

    /// The tasks that run on cues, the one chosen kept where the choices no longer list it,
    /// and Choose a task, unselectable, while none is chosen.
    fn job_row(&self, choices: Option<&MandateChoices>) -> Row {
        let jobs = choices.map_or(&[][..], |c| c.jobs.as_slice());
        let mut options = Vec::new();
        if !jobs.iter().any(|j| j.id == self.job) {
            let label = if self.job.is_empty() {
                words::CHOOSE_TASK
            } else {
                &self.job
            };
            options.push(option(&self.job, label, self.job.is_empty()));
        }
        options.extend(jobs.iter().map(|j| option(&j.id, &j.name, false)));
        let mut row = row("job", Kind::Choice, words::TASK, json!(self.job));
        row.options = options;
        row
    }

    /// The mode's two words, and an unknown one an edit keeps.
    fn mode_row(&self) -> Row {
        let mut options = vec![
            option(APPROVE, words::MODE_APPROVE, false),
            option(AUTO, words::MODE_AUTO, false),
        ];
        if self.mode != APPROVE && self.mode != AUTO {
            options.push(option(&self.mode, &self.mode, false));
        }
        let mut row = row("mode", Kind::Choice, words::MODE, json!(self.mode));
        row.options = options;
        row
    }

    /// The tools chosen, or that no plugin offers one: the line the tools' refusals sit under.
    fn tools_row(&self, choices: Option<&MandateChoices>) -> Row {
        let offered = choices.is_some_and(|c| !c.tools.is_empty());
        let chosen: Vec<String> = self.tools.iter().cloned().collect();
        let shown = if !offered && chosen.is_empty() {
            words::NO_TOOLS.to_owned()
        } else {
            words::list(&chosen)
        };
        let mut row = row("tools", Kind::Text, words::TOOLS, json!(shown));
        row.read_only = true;
        row
    }

    /// A switch for each tool offered, tagged with what it does, then one for each tool chosen
    /// that the choices no longer list, so an edit never drops it unasked.
    fn tool_rows(&self, choices: Option<&MandateChoices>) -> Vec<Row> {
        let offered = choices.map_or(&[][..], |c| c.tools.as_slice());
        let mut rows: Vec<Row> = offered
            .iter()
            .map(|tool| {
                let label = tool.title.as_deref().unwrap_or(&tool.name);
                let chosen = json!(self.tools.contains(&tool.name));
                let mut row = row(&format!("{TOOL}{}", tool.name), Kind::Toggle, label, chosen);
                let does = if tool.read_only {
                    words::READ_ONLY
                } else {
                    words::CHANGES_THINGS
                };
                row.footer = Some(does.to_owned());
                row
            })
            .collect();
        let unlisted = self
            .tools
            .iter()
            .filter(|t| !offered.iter().any(|o| o.name == **t));
        rows.extend(unlisted.map(|t| row(&format!("{TOOL}{t}"), Kind::Toggle, t, json!(true))));
        rows
    }

    /// Under More options: the validator, its interpreter and environment names, the call id
    /// argument, the skill paths and the pinned arguments.
    fn option_rows(&self) -> Vec<Row> {
        let pins: Vec<String> = self
            .pins
            .iter()
            .map(|p| format!("{}={}", p.key, p.text))
            .collect();
        let mut rows = vec![
            row(
                "validator",
                Kind::Text,
                words::VALIDATOR,
                json!(self.validator),
            ),
            row(
                "interpreter",
                Kind::Text,
                words::INTERPRETER,
                json!(self.interpreter),
            ),
            row(
                "validator_env",
                Kind::List,
                words::ENVIRONMENT_NAMES,
                json!(self.validator_env),
            ),
            row(
                "idempotency_arg",
                Kind::Text,
                words::CALL_ID_ARGUMENT,
                json!(self.idempotency_arg),
            ),
            row(
                "unpinned",
                Kind::List,
                words::SKILL_PATHS,
                json!(self.unpinned),
            ),
            row("pin_args", Kind::List, words::PINNED_ARGUMENTS, json!(pins)),
        ];
        rows[0].info = Some(words::VALIDATOR_INFO.to_owned());
        rows[3].info = Some(words::CALL_ID_INFO.to_owned());
        rows[5].example = Some(PIN_EXAMPLE.to_owned());
        for row in &mut rows {
            row.advanced = true;
        }
        rows
    }

    /// The pins as listed. A pin whose key and text are unchanged keeps its own type.
    fn pins_from(&self, listed: &[String]) -> Vec<PinDraft> {
        listed
            .iter()
            .map(|item| {
                let (key, text) = item.split_once('=').unwrap_or((item.as_str(), ""));
                let key = key.trim();
                let kept = self.pins.iter().find(|p| p.key == key && p.text == text);
                kept.cloned().unwrap_or_else(|| PinDraft::new(key, text))
            })
            .collect()
    }
}

fn row(key: &str, kind: Kind, label: &str, value: Value) -> Row {
    Row {
        key: key.to_owned(),
        kind,
        label: label.to_owned(),
        footer: None,
        info: None,
        example: None,
        advanced: false,
        value,
        present: None,
        options: Vec::new(),
        min: None,
        max: None,
        step: None,
        restart: false,
        read_only: false,
        suggestions: false,
        unit: None,
        format: None,
    }
}

fn option(value: &str, label: &str, disabled: bool) -> ChoiceOption {
    ChoiceOption {
        value: value.to_owned(),
        label: label.to_owned(),
        hint: None,
        disabled,
    }
}

/// Whole minutes, from zero up, typed as a number.
fn minutes_row(key: &str, label: &str, info: &str, typed: &str) -> Row {
    let shown = minutes(typed).map_or(Value::Null, |m| json!(m));
    let mut row = row(key, Kind::Number, label, shown);
    row.info = Some(info.to_owned());
    row.unit = Some(words::MINUTES.to_owned());
    (row.min, row.step) = (Some(0.0), Some(1.0));
    row
}

/// A number as typed minutes: whole and not below zero, or text that is not a number of
/// minutes, so the form is not complete.
fn whole_minutes(value: &Value) -> String {
    match value.as_f64() {
        Some(n) if n >= 0.0 && n.fract() == 0.0 => format!("{n:.0}"),
        _ => value.to_string(),
    }
}

fn items(value: &Value) -> Vec<String> {
    let listed = value.as_array().map_or(&[][..], Vec::as_slice);
    listed
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn mode_word(mode: &Word<MandateMode>) -> String {
    match mode {
        Word::Known(MandateMode::Approve) => APPROVE.to_owned(),
        Word::Known(MandateMode::Auto) => AUTO.to_owned(),
        Word::Unknown(word) => word.clone(),
    }
}

/// Whole minutes as typed, or None for anything else.
fn minutes(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

fn seconds(text: &str) -> u64 {
    minutes(text).unwrap_or(0).saturating_mul(60)
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}
