//! A source's add and edit form, as a value (M60 cues §4.4). The form is the daemon's: kinds,
//! presets and rows come from `cues.sources.form` on an add and from `cues.sources.get`'s
//! `values` on an edit, and the app writes no field list per kind. What lives here is what the
//! owner set, what Add sends, which typed secrets are stored after it, and where a refusal goes.
//! A typed secret stays here until Add and is never sent with the source.

use super::calls::{FormChoice, SourceChoice};
use super::wire::{FormKind, FormPreset, HandedOut, SecretField, SourceDetail, SourceForm};
use super::words;
use crate::settings::{Kind, Row};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;

/// The name row's key, which Add sends as the source's `ref`.
pub const NAME_KEY: &str = "ref";
/// The check row, whose change asks for the form again.
pub const VERIFY_KEY: &str = "verify";
/// The kind whose check is chosen in the form, and the check it starts with.
const WEBHOOK: &str = "webhook";
const SIGNATURE: &str = "hmac";
/// Where a refusal about the kind or preset pickers points.
const PICKERS: [&str; 2] = ["kind", "preset"];

/// A secret as typed, held until it is stored. Its `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Held(String);

impl Held {
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Held {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Held(..)")
    }
}

/// One secret typed into the add form, stored with `secret.set` once the source exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedSecret {
    pub field: SecretField,
    pub value: Held,
}

/// What Add sends: the name, the preset or the kind, and the values the owner changed.
#[derive(Debug, Clone, PartialEq)]
pub struct AddSpec {
    pub name: String,
    pub choice: SourceChoice,
    pub values: Map<String, Value>,
}

/// The id a source's typed secret is stored under.
pub fn secret_id(source: &str, field: SecretField) -> String {
    assert!(
        !source.is_empty(),
        "a source secret belongs to a named source"
    );
    format!("cue_source:{source}:{}", field.wire())
}

/// What an add leaves for the source's page: a token handed out once, and the typed secret the
/// daemon would not store, with its sentence.
#[derive(Debug, Default)]
pub struct Arrival {
    pub token: Option<HandedOut>,
    pub unstored: Option<(SecretField, String)>,
}

impl Arrival {
    /// The secret the source's page opens at: the one not stored, so its sentence leads.
    pub fn at(&self) -> Option<SecretField> {
        self.unstored.as_ref().map(|(field, _)| *field)
    }
}

/// An add once the daemon took the source: each typed secret is stored after it, one at a time
/// in the form's row order, and the first the daemon refuses ends it there, the rest unsent.
#[derive(Debug)]
pub struct AddFlow {
    name: String,
    /// The secrets still to store, the next first.
    pending: VecDeque<TypedSecret>,
    /// The secret given out to store, until its answer.
    storing: Option<SecretField>,
    arrival: Arrival,
}

impl AddFlow {
    /// The source `name` was added, handing out `token`; `secrets` are what `prepare_add` held.
    pub fn added(name: &str, token: Option<HandedOut>, secrets: Vec<TypedSecret>) -> AddFlow {
        assert!(!name.is_empty(), "an added source is named");
        AddFlow {
            name: name.to_owned(),
            pending: secrets.into(),
            storing: None,
            arrival: Arrival {
                token,
                unstored: None,
            },
        }
    }

    /// The next secret to store, with the id it goes under; None once each is stored or one
    /// was refused.
    pub fn next_secret(&mut self) -> Option<(String, Held)> {
        assert!(self.storing.is_none(), "one secret is stored at a time");
        let secret = self.pending.pop_front()?;
        self.storing = Some(secret.field);
        Some((secret_id(&self.name, secret.field), secret.value))
    }

    /// The daemon's answer to the secret last given out: a refusal ends the add at it, and the
    /// secrets after it are dropped unsent.
    pub fn stored(&mut self, refusal: Option<String>) {
        let field = self
            .storing
            .take()
            .expect("an answer to a secret given out");
        if let Some(sentence) = refusal {
            self.arrival.unstored = Some((field, sentence));
            self.pending.clear();
        }
    }

    /// What the source's page opens with, once nothing is left to store.
    pub fn arrival(self) -> Arrival {
        assert!(
            self.storing.is_none() && self.pending.is_empty(),
            "each secret was stored or one refused"
        );
        self.arrival
    }
}

/// The secret field a secret row stands for, by its key; None for one this build cannot name.
pub fn secret_field(key: &str) -> Option<SecretField> {
    [
        SecretField::Token,
        SecretField::Key,
        SecretField::Secret,
        SecretField::PathSecret,
    ]
    .into_iter()
    .find(|f| f.wire() == key)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Refused {
    field: Option<String>,
    sentence: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceFormState {
    /// The source an edit changes; None on an add.
    editing: Option<String>,
    kind: Option<String>,
    preset: Option<String>,
    kinds: Vec<FormKind>,
    presets: Vec<FormPreset>,
    rows: Vec<Row>,
    consent: Option<String>,
    notes: Vec<String>,
    /// What the owner set, by row key. Never a secret.
    values: BTreeMap<String, Value>,
    /// What the owner typed into a secret row, by row key, until Add.
    secrets: BTreeMap<String, Held>,
    refusal: Option<Refused>,
}

impl SourceFormState {
    /// An edit: the source's own fields, its name and kind fixed. Its secrets are stored from
    /// its page, so they are not on this form.
    pub fn editing(detail: &SourceDetail) -> SourceFormState {
        SourceFormState {
            editing: Some(detail.source.name.clone()),
            rows: detail
                .values
                .iter()
                .filter(|r| r.kind != Kind::Secret)
                .cloned()
                .collect(),
            ..SourceFormState::default()
        }
    }

    pub fn editing_name(&self) -> Option<&str> {
        self.editing.as_deref()
    }

    pub fn kind(&self) -> Option<&str> {
        self.kind.as_deref()
    }

    pub fn preset(&self) -> Option<&str> {
        self.preset.as_deref()
    }

    pub fn kinds(&self) -> &[FormKind] {
        &self.kinds
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The presets of the kind chosen.
    pub fn kind_presets(&self) -> Vec<&FormPreset> {
        self.presets
            .iter()
            .filter(|p| Some(&p.kind) == self.kind.as_ref())
            .collect()
    }

    /// The choice the form is asked for: the preset where one is chosen, else the kind, and
    /// the check where the owner set one.
    pub fn request(&self) -> FormChoice {
        let verify = self
            .values
            .get(VERIFY_KEY)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        match &self.preset {
            Some(preset) => FormChoice {
                kind: None,
                preset: Some(preset.clone()),
                verify,
            },
            None => FormChoice {
                kind: self.kind.clone(),
                preset: None,
                verify,
            },
        }
    }

    /// The daemon's answer to `asked`. The first answer opens on the first kind and that kind's
    /// first preset. Rows are taken only from an answer to the choice held now, so one choice's
    /// fields never show under another's; what was set or typed for a field the new form lacks
    /// goes with the field.
    pub fn take(&mut self, form: &SourceForm, asked: &FormChoice) {
        self.kinds = form.kinds.clone();
        self.presets = form.presets.clone();
        if self.kind.is_none() {
            self.kind = form.kinds.first().map(|k| k.id.clone());
            self.preset = self.first_preset();
            self.default_check();
        }
        if *asked != self.request() {
            return;
        }
        self.rows = form.rows.clone();
        self.consent = form.consent_sentence.clone();
        self.notes = form.notes.clone();
        let keys: Vec<&str> = form.rows.iter().map(|r| r.key.as_str()).collect();
        self.values.retain(|key, _| keys.contains(&key.as_str()));
        self.secrets.retain(|key, _| keys.contains(&key.as_str()));
    }

    /// A kind chosen: its first preset ("None" stays in the menu) and no check of another's.
    pub fn choose_kind(&mut self, kind: &str) {
        assert!(!kind.is_empty(), "a kind has an id");
        if self.kind.as_deref() == Some(kind) {
            return;
        }
        self.kind = Some(kind.to_owned());
        self.preset = self.first_preset();
        self.values.remove(VERIFY_KEY);
        self.refusal = None;
        self.default_check();
    }

    pub fn choose_preset(&mut self, preset: Option<&str>) {
        self.preset = preset.map(str::to_owned);
        self.values.remove(VERIFY_KEY);
        self.refusal = None;
        self.default_check();
    }

    /// A webhook with no preset is checked by a signature until another check is chosen, so
    /// the form asked for carries the signature's fields (§4.4 step 5).
    fn default_check(&mut self) {
        let bare_webhook = self.kind.as_deref() == Some(WEBHOOK) && self.preset.is_none();
        if bare_webhook && !self.values.contains_key(VERIFY_KEY) {
            self.values
                .insert(VERIFY_KEY.to_owned(), Value::from(SIGNATURE));
        }
    }

    fn first_preset(&self) -> Option<String> {
        self.presets
            .iter()
            .find(|p| Some(&p.kind) == self.kind.as_ref())
            .map(|p| p.id.clone())
    }

    /// A field changed. Its refusal goes with the value it was about.
    pub fn set(&mut self, key: &str, value: Value) {
        self.values.insert(key.to_owned(), value);
        self.clear_refusal(key);
    }

    /// A secret row typed in; blank forgets it.
    pub fn hold_secret(&mut self, key: &str, text: &str) {
        if text.is_empty() {
            self.secrets.remove(key);
        } else {
            self.secrets.insert(key.to_owned(), Held(text.to_owned()));
        }
        self.clear_refusal(key);
    }

    fn clear_refusal(&mut self, key: &str) {
        if self.refusal.as_ref().and_then(|r| r.field.as_deref()) == Some(key) {
            self.refusal = None;
        }
    }

    /// What a row holds now: what the owner set, else the daemon's value.
    pub fn value_of(&self, row: &Row) -> Value {
        self.values
            .get(&row.key)
            .cloned()
            .unwrap_or_else(|| row.value.clone())
    }

    /// The name typed, trimmed.
    pub fn name(&self) -> String {
        self.values
            .get(NAME_KEY)
            .and_then(Value::as_str)
            .map(|n| n.trim().to_owned())
            .unwrap_or_default()
    }

    /// What Add sends, or None while there is nothing to add: an edit, no name yet, or no form.
    pub fn spec(&self) -> Option<AddSpec> {
        let name = self.name();
        if self.editing.is_some() || name.is_empty() {
            return None;
        }
        let choice = match (&self.preset, &self.kind) {
            (Some(preset), _) => SourceChoice::Preset(preset.clone()),
            (None, Some(kind)) => SourceChoice::Kind(kind.clone()),
            (None, None) => return None,
        };
        let mut values = self.changed_values();
        values.remove(NAME_KEY);
        // The check chosen is part of what was asked for, so the answer's row already holds it;
        // the add carries it all the same, or the source would have no check.
        if let Some(verify) = self
            .values
            .get(VERIFY_KEY)
            .filter(|v| v.as_str() != Some(""))
        {
            values.insert(VERIFY_KEY.to_owned(), verify.clone());
        }
        Some(AddSpec {
            name,
            choice,
            values,
        })
    }

    /// The fields the owner changed from the form's own values, secrets aside: what Add sends
    /// beside the name, and what an edit sends.
    pub fn changed_values(&self) -> Map<String, Value> {
        self.rows
            .iter()
            .filter(|row| row.kind != Kind::Secret)
            .filter_map(|row| {
                let value = self.values.get(&row.key)?;
                (*value != row.value).then(|| (row.key.clone(), value.clone()))
            })
            .collect()
    }

    /// The secrets typed, in the form's order, each one the owner may store.
    pub fn typed_secrets(&self) -> Vec<TypedSecret> {
        self.rows
            .iter()
            .filter(|row| row.kind == Kind::Secret)
            .filter_map(|row| {
                let value = self.secrets.get(&row.key)?.clone();
                let field = secret_field(&row.key).filter(|f| f.typed())?;
                Some(TypedSecret { field, value })
            })
            .collect()
    }

    /// What Add sends and stores, or None with a refusal under the row of a secret typed for a
    /// field this build cannot store: sending the source without it would drop what was typed
    /// with nothing said.
    pub fn prepare_add(&mut self) -> Option<(AddSpec, Vec<TypedSecret>)> {
        let unstorable = self.rows.iter().find(|row| {
            row.kind == Kind::Secret
                && self.secrets.contains_key(&row.key)
                && !secret_field(&row.key).is_some_and(SecretField::typed)
        });
        if let Some(row) = unstorable {
            let field = row.key.clone();
            self.refuse(Some(&field), words::SECRET_UNSTORABLE);
            return None;
        }
        Some((self.spec()?, self.typed_secrets()))
    }

    /// What the info beside the kind's line holds, as on the Mac: the daemon's consent line, then
    /// its notes.
    pub fn about(&self) -> Option<String> {
        let lines: Vec<&str> = self
            .consent
            .iter()
            .chain(&self.notes)
            .map(String::as_str)
            .filter(|l| !l.is_empty())
            .collect();
        (!lines.is_empty()).then(|| lines.join("\n\n"))
    }

    /// The daemon refused, about `field` where it named one.
    pub fn refuse(&mut self, field: Option<&str>, sentence: &str) {
        assert!(!sentence.is_empty(), "a refusal says why");
        self.refusal = Some(Refused {
            field: field.map(str::to_owned),
            sentence: sentence.to_owned(),
        });
    }

    /// The refusal under the row, or the pickers, it names.
    pub fn refusal_for(&self, key: &str) -> Option<&str> {
        let refused = self.refusal.as_ref()?;
        (refused.field.as_deref() == Some(key)).then_some(refused.sentence.as_str())
    }

    /// A refusal that names no row of the form and no picker, said under the form.
    pub fn form_refusal(&self) -> Option<&str> {
        let refused = self.refusal.as_ref()?;
        let placed = refused
            .field
            .as_deref()
            .is_some_and(|f| PICKERS.contains(&f) || self.rows.iter().any(|r| r.key == f));
        (!placed).then_some(refused.sentence.as_str())
    }

    /// Whether the refusal is under a row folded under More options, which then opens.
    pub fn refused_folded(&self) -> bool {
        let Some(field) = self.refusal.as_ref().and_then(|r| r.field.as_deref()) else {
            return false;
        };
        self.rows.iter().any(|r| r.key == field && r.advanced)
    }
}
