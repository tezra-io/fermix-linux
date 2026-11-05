//! The 25 `cues.*` methods as typed calls. Add, update, `token.new` and `url.get` never carry a
//! secret: a typed secret goes through `secret.set` alone. An answer that hands a value out is
//! never quoted in an error, since it holds that value.

use super::wire::{
    Address, AlwaysAllowRevoked, CueStatus, Decision, MandateAnswer, MandateChoices, MandateList,
    MandateRemoved, NewToken, ProposalAnswer, ProposalCardAnswer, ProposalFilter, ProposalList,
    SourceAdded, SourceAnswer, SourceDetail, SourceForm, SourceList, SourceRemoved, WatcherAnswer,
    WatcherList,
};
use crate::management::{CallError, Management};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

/// What `cues.sources.form` is asked for: a kind, or a preset (which names its kind), and the
/// check a webhook verifies its deliveries with. None of them asks for the default form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormChoice {
    pub kind: Option<String>,
    pub preset: Option<String>,
    pub verify: Option<String>,
}

impl FormChoice {
    fn params(&self) -> Value {
        let mut params = Map::new();
        let fields = [
            ("kind", &self.kind),
            ("preset", &self.preset),
            ("verify", &self.verify),
        ];
        for (key, value) in fields {
            if let Some(value) = value {
                params.insert(key.into(), json!(value));
            }
        }
        Value::Object(params)
    }
}

/// How a new source names what it is: by its preset, or by its kind where it has none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChoice {
    Preset(String),
    Kind(String),
}

impl Management {
    pub fn cues_status(&self) -> Result<CueStatus, CallError> {
        decoded("cues.status", self.call("cues.status", json!({}))?)
    }

    pub fn cue_sources(&self) -> Result<SourceList, CallError> {
        decoded(
            "cues.sources.list",
            self.call("cues.sources.list", json!({}))?,
        )
    }

    pub fn cue_source(&self, name: &str) -> Result<SourceDetail, CallError> {
        named("cues.sources.get", name);
        let answer = self.call("cues.sources.get", json!({ "ref": name }))?;
        decoded("cues.sources.get", answer)
    }

    /// The add form for one choice. It writes nothing.
    pub fn cue_source_form(&self, choice: &FormChoice) -> Result<SourceForm, CallError> {
        let answer = self.call("cues.sources.form", choice.params())?;
        decoded("cues.sources.form", answer)
    }

    /// Saves a new source under `name`, or with None under a name the daemon makes. `values` are
    /// the non-secret fields that differ from the form's. The answer names the source as it was
    /// saved, and carries what Fermix made for it once: a local push token or a GitHub secret.
    pub fn cue_source_add(
        &self,
        name: Option<&str>,
        choice: &SourceChoice,
        values: Map<String, Value>,
    ) -> Result<SourceAdded, CallError> {
        let (key, id) = match choice {
            SourceChoice::Preset(id) => ("preset", id),
            SourceChoice::Kind(id) => ("kind", id),
        };
        assert!(!id.is_empty(), "a new source names its {key}");
        let mut params = Map::new();
        if let Some(name) = name {
            named("cues.sources.add", name);
            params.insert("ref".into(), json!(name));
        }
        params.insert(key.into(), json!(id));
        params.insert("values".into(), Value::Object(values));
        let answer = self.call("cues.sources.add", Value::Object(params))?;
        handed("cues.sources.add", answer)
    }

    /// Changes the fields `values` names. The name, kind and preset cannot change.
    pub fn cue_source_update(
        &self,
        name: &str,
        values: Map<String, Value>,
    ) -> Result<SourceAnswer, CallError> {
        named("cues.sources.update", name);
        assert!(!values.is_empty(), "an update changes at least one field");
        let params = json!({ "ref": name, "values": values });
        decoded(
            "cues.sources.update",
            self.call("cues.sources.update", params)?,
        )
    }

    pub fn cue_source_remove(&self, name: &str) -> Result<SourceRemoved, CallError> {
        self.by_ref("cues.sources.remove", name)
    }

    /// The source's switch, either way.
    pub fn cue_source_switch(&self, name: &str, on: bool) -> Result<SourceAnswer, CallError> {
        let method = if on {
            "cues.sources.enable"
        } else {
            "cues.sources.disable"
        };
        self.by_ref(method, name)
    }

    /// A fresh token for a local push source, handed out once. The old one stops within a minute.
    pub fn cue_source_token_new(&self, name: &str) -> Result<NewToken, CallError> {
        named("cues.sources.token.new", name);
        let answer = self.call("cues.sources.token.new", json!({ "ref": name }))?;
        handed("cues.sources.token.new", answer)
    }

    /// A webhook's address, made on the first ask and the same on every later one.
    pub fn cue_source_address(&self, name: &str) -> Result<Address, CallError> {
        named("cues.sources.url.get", name);
        let answer = self.call("cues.sources.url.get", json!({ "ref": name }))?;
        handed("cues.sources.url.get", answer)
    }

    pub fn cue_watchers(&self) -> Result<WatcherList, CallError> {
        decoded(
            "cues.watchers.list",
            self.call("cues.watchers.list", json!({}))?,
        )
    }

    pub fn cue_watcher(&self, name: &str) -> Result<WatcherAnswer, CallError> {
        self.by_name("cues.watchers.get", name)
    }

    /// Takes back the owner's Always allow of the watcher's follow-up, so each of its runs asks
    /// first again on a card. Granting it is the owner's answer on a card alone.
    pub fn cue_watcher_always_allow_revoke(
        &self,
        name: &str,
    ) -> Result<AlwaysAllowRevoked, CallError> {
        self.by_name("cues.watchers.always_allow.revoke", name)
    }

    pub fn cue_mandates(&self) -> Result<MandateList, CallError> {
        decoded(
            "cues.mandates.list",
            self.call("cues.mandates.list", json!({}))?,
        )
    }

    pub fn cue_mandate_choices(&self) -> Result<MandateChoices, CallError> {
        decoded(
            "cues.mandates.choices",
            self.call("cues.mandates.choices", json!({}))?,
        )
    }

    /// Saves a new mandate, or changes one: durations travel as whole seconds.
    pub fn cue_mandate_write(
        &self,
        name: &str,
        values: Map<String, Value>,
        add: bool,
    ) -> Result<MandateAnswer, CallError> {
        let method = if add {
            "cues.mandates.add"
        } else {
            "cues.mandates.update"
        };
        named(method, name);
        assert!(!values.is_empty(), "{method} sends at least one field");
        let params = json!({ "name": name, "values": values });
        decoded(method, self.call(method, params)?)
    }

    pub fn cue_mandate_remove(&self, name: &str) -> Result<MandateRemoved, CallError> {
        self.by_name("cues.mandates.remove", name)
    }

    pub fn cue_mandate_switch(&self, name: &str, on: bool) -> Result<MandateAnswer, CallError> {
        let method = if on {
            "cues.mandates.enable"
        } else {
            "cues.mandates.disable"
        };
        self.by_name(method, name)
    }

    /// Records the owner's approval of a mandate as it is now.
    pub fn cue_mandate_approve(&self, name: &str) -> Result<MandateAnswer, CallError> {
        self.by_name("cues.mandates.approve", name)
    }

    pub fn cue_proposals(&self, filter: ProposalFilter) -> Result<ProposalList, CallError> {
        let params = match filter {
            ProposalFilter::Waiting => json!({}),
            ProposalFilter::Recent => json!({ "state": "recent" }),
        };
        decoded(
            "cues.proposals.list",
            self.call("cues.proposals.list", params)?,
        )
    }

    pub fn cue_proposal(&self, proposal_id: &str) -> Result<ProposalCardAnswer, CallError> {
        named("cues.proposals.get", proposal_id);
        let params = json!({ "proposal_id": proposal_id });
        decoded(
            "cues.proposals.get",
            self.call("cues.proposals.get", params)?,
        )
    }

    /// The contract's way to answer a proposal. This app answers one only on its card in Chat
    /// (M60 cues L1, §6 rule 3); nothing in it calls this.
    pub fn cue_proposal_decide(
        &self,
        proposal_id: &str,
        decision: Decision,
    ) -> Result<ProposalAnswer, CallError> {
        named("cues.proposals.decide", proposal_id);
        let word = match decision {
            Decision::Confirm => "confirm",
            Decision::Deny => "deny",
        };
        let params = json!({ "proposal_id": proposal_id, "decision": word });
        decoded(
            "cues.proposals.decide",
            self.call("cues.proposals.decide", params)?,
        )
    }

    fn by_ref<T: DeserializeOwned>(&self, method: &str, name: &str) -> Result<T, CallError> {
        named(method, name);
        decoded(method, self.call(method, json!({ "ref": name }))?)
    }

    fn by_name<T: DeserializeOwned>(&self, method: &str, name: &str) -> Result<T, CallError> {
        named(method, name);
        decoded(method, self.call(method, json!({ "name": name }))?)
    }
}

fn named(method: &str, name: &str) {
    assert!(!name.is_empty(), "{method} names what it is about");
}

fn decoded<T: DeserializeOwned>(method: &str, answer: Value) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}

/// Decodes an answer that hands a value out. The decoder's own words can quote what it read, so
/// they are left out.
fn handed<T: DeserializeOwned>(method: &str, answer: Value) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|_| CallError::Protocol(format!("{method} answered an unexpected shape")))
}
