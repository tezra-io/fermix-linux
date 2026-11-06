//! What the Cues pane draws, from what the daemon publishes (M60 cues §4): whether there is a
//! pane at all, each row's tone and controls, where a row opens, the filter model's pair rule,
//! and the questions a row asks in place. Controls follow the published action ids, never the
//! words; a state is read only for a tone, which moves no control.

use super::wire::{
    CueState, CueStatus, DoorState, FollowUpState, MandateAction, MandateMode, MandateRow,
    ProposalList, SecretField, SecretSlot, SourceAction, SourceKind, SourceRow, WatcherFilter,
    WatcherRow, WatcherState, Word,
};
use super::words;
use crate::management::CallError;
use crate::model::Capabilities;
use crate::settings::{text_value, ChoiceOption, Listing, Row, Section};
use crate::view::{daemon_problem, feature_problem};
use serde_json::{json, Map, Value};

/// The pane's slug in Settings.
pub const PANE: &str = "cues";
/// The settings section the pane's switches, filter model and door port are in.
pub const SECTION: &str = "cues";
/// The method whose presence in `hello` is the first half of the gate.
pub const STATUS_METHOD: &str = "cues.status";
pub const ENABLED_KEY: &str = "cues_enabled";
pub const DOOR_PORT_KEY: &str = "cues_door_port";
pub const FILTER_PROVIDER_KEY: &str = "filter_provider";
pub const FILTER_MODEL_KEY: &str = "filter_model";

/// Whether the Cues pane exists (§4.1): `hello` lists `cues.status`, the daemon publishes the
/// `cues` section, and `cues.status` has answered. With any of them missing there is no pane,
/// no sidebar row, no search hit and no sentence.
pub fn available(
    capabilities: Option<&Capabilities>,
    sections: Option<&[Section]>,
    status: Option<&CueStatus>,
) -> bool {
    let method = capabilities.is_some_and(|c| c.has_method(STATUS_METHOD));
    method && publishes_section(sections) && status.is_some()
}

/// Whether the gate has said no (§4.1), so a route aimed at the pane lands on Providers: the
/// daemon lacks the method, its sections are read and have no `cues`, or `cues.status` answered
/// that it has no such method. Unread is not shut: a daemon just restarted keeps the pane on
/// screen while it is read again, and a daemon down shows the down page instead.
pub fn shut(
    capabilities: Option<&Capabilities>,
    sections: Option<&[Section]>,
    status_missing: bool,
) -> bool {
    let no_method = capabilities.is_some_and(|c| !c.has_method(STATUS_METHOD));
    let no_section = sections.is_some() && !publishes_section(sections);
    no_method || no_section || status_missing
}

/// Whether the daemon publishes the `cues` section, once its sections are read.
pub fn publishes_section(sections: Option<&[Section]>) -> bool {
    sections.is_some_and(|all| all.iter().any(|s| s.id == SECTION && s.pane == PANE))
}

/// What a cue read that failed leaves: no surface, where the daemon lacks the method, or a
/// sentence drawn in place of what was read (the daemon's own, as memory off gives it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Missing,
    Sentence(String),
}

pub fn failure(err: &CallError) -> Failure {
    match (feature_problem(err), err) {
        (None, _) => Failure::Missing,
        (Some(_), CallError::Refused(refusal)) if refusal.code != "method_not_found" => {
            Failure::Sentence(refusal.sentence.clone())
        }
        (Some(problem), _) => Failure::Sentence(problem.sentence()),
    }
}

/// What a page read for one row says when that read failed: `gone` where the daemon refused the
/// name the page is about (`field`, the row went after its list was drawn), the daemon's own
/// sentence for any other refusal, and otherwise what became of the daemon. The page was opened
/// from a list the daemon answered, so a method it lacks means it is older than this app.
pub fn read_failure(err: &CallError, field: &str, gone: &str) -> String {
    assert!(
        !field.is_empty() && !gone.is_empty(),
        "a page names its row"
    );
    match err {
        CallError::Refused(refusal)
            if refusal.code == "invalid_params" && refusal.field.as_deref() == Some(field) =>
        {
            gone.to_owned()
        }
        CallError::Refused(refusal) if refusal.code != "method_not_found" => {
            refusal.sentence.clone()
        }
        other => daemon_problem(other).sentence(),
    }
}

/// A row's tone, the dot before its sentence. The sentence carries the state; the dot lets a
/// list be read at a glance and is never the only signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Fail,
    Quiet,
}

/// Off is quiet, a source that stopped working is a failure, one the daemon leads with an action
/// for is amber, and the rest is well.
pub fn source_tone(source: &SourceRow) -> Tone {
    if !source.enabled {
        return Tone::Quiet;
    }
    let failed = [
        crate::cues::wire::SourceState::AuthFailed,
        crate::cues::wire::SourceState::Stopped,
        crate::cues::wire::SourceState::Unavailable,
    ];
    if failed.iter().any(|s| source.state.is(*s)) {
        return Tone::Fail;
    }
    if leads(source.primary_action.as_ref(), &source.actions) {
        return Tone::Warn;
    }
    Tone::Good
}

/// A watcher that needs an update in chat, or whose filter cannot reach its model, asks for
/// attention; a paused one is quiet.
pub fn watcher_tone(watcher: &WatcherRow) -> Tone {
    if !watcher.enabled {
        return Tone::Quiet;
    }
    if watcher_attention(watcher) {
        return Tone::Warn;
    }
    if watcher.task.paused || watcher.state.is(WatcherState::Paused) {
        return Tone::Quiet;
    }
    Tone::Good
}

/// Whether a watcher's sentence is drawn in the attention style.
pub fn watcher_attention(watcher: &WatcherRow) -> bool {
    watcher.state.is(WatcherState::NeedsUpdate) || watcher.state.is(WatcherState::Held)
}

/// A mandate that did not load, or waits for its approval, is amber.
pub fn mandate_tone(mandate: &MandateRow) -> Tone {
    if !mandate.enabled {
        return Tone::Quiet;
    }
    let loaded = mandate
        .load
        .state
        .is(crate::cues::wire::MandateLoad::Active);
    if !loaded || offers_approve(mandate) {
        return Tone::Warn;
    }
    Tone::Good
}

/// The subsystem: running is well, a restart or memory still owed is amber, off is quiet.
pub fn status_tone(status: &CueStatus) -> Tone {
    match status.state.known() {
        Some(CueState::Running) => Tone::Good,
        Some(CueState::NeedsRestart | CueState::NeedsMemory) => Tone::Warn,
        Some(CueState::Off) | None => Tone::Quiet,
    }
}

/// The door: listening is well, a door that could not open amber, one with nothing to take quiet.
pub fn door_tone(status: &CueStatus) -> Tone {
    match status.door.state.known() {
        Some(DoorState::Listening) => Tone::Good,
        Some(DoorState::Unavailable) => Tone::Warn,
        Some(DoorState::NotRunning) | None => Tone::Quiet,
    }
}

/// Whether the door's sentence leaves the folded port for the open, as a warning.
pub fn door_in_the_open(status: &CueStatus) -> bool {
    status.door.state.is(DoorState::Unavailable)
}

/// Whether the daemon leads with an action it also publishes.
fn leads<T: Copy + PartialEq>(primary: Option<&Word<T>>, actions: &[Word<T>]) -> bool {
    primary.is_some_and(|p| matches!(p, Word::Known(_)) && actions.contains(p))
}

/// Whether the daemon publishes `action` among a row's actions, the only thing that draws its
/// control.
pub fn has<T: Copy + PartialEq>(actions: &[Word<T>], action: T) -> bool {
    actions.iter().any(|a| a.is(action))
}

/// The switch is drawn where the daemon publishes a way to throw it.
pub fn source_switch(source: &SourceRow) -> bool {
    has(&source.actions, SourceAction::Enable) || has(&source.actions, SourceAction::Disable)
}

pub fn mandate_switch(mandate: &MandateRow) -> bool {
    has(&mandate.actions, MandateAction::Enable) || has(&mandate.actions, MandateAction::Disable)
}

/// Approve is offered where the daemon leads with it and publishes it. An inactive mandate
/// publishes neither.
pub fn offers_approve(mandate: &MandateRow) -> bool {
    let primary = mandate.primary_action.as_ref();
    primary.is_some_and(|p| p.is(MandateAction::Approve))
        && has(&mandate.actions, MandateAction::Approve)
}

/// What a source's page says it is and did, each fact a title and its value, in order (M60 cues
/// §4.4): its kind, the host of the feed or stream it reads or an X accounts source's handles
/// (`location`, never its address, which can carry a credential), its daily limit, its last day
/// and the watchers listening. A webhook and local push have no host, and no row for one.
pub fn source_facts(source: &SourceRow) -> Vec<(&'static str, String)> {
    let mut facts = vec![(words::KIND, source.label.clone())];
    let location = source.location.as_deref().map(str::trim);
    if let Some(location) = location.filter(|l| !l.is_empty()) {
        facts.push((location_title(source), location.to_owned()));
    }
    let counts = &source.counts;
    facts.push((words::ITEMS_A_DAY, words::items_a_day(source.daily_budget)));
    facts.push((
        words::LAST_DAY,
        words::source_last_day(counts.received, counts.routed),
    ));
    facts.push((words::LISTENING, words::list(&source.listening)));
    facts
}

/// The title of a source's `location` row: an X accounts source's location is its handles.
fn location_title(source: &SourceRow) -> &'static str {
    if source.preset.as_deref() == Some("x_accounts") {
        words::ACCOUNTS
    } else {
        words::ADDRESS_HOST
    }
}

/// Where a source row opens its page: at the secret the daemon says it waits for, where its
/// primary action points at one, `set_secret` at a secret the owner types and `new_token` at
/// the token only Fermix makes. Read from the published action and field, never the state.
pub fn opening(source: &SourceRow) -> Option<SecretField> {
    if !leads(source.primary_action.as_ref(), &source.actions) {
        return None;
    }
    let field = source.missing_secret.as_ref()?.known()?;
    match source.primary_action.as_ref()?.known()? {
        SourceAction::SetSecret if field.typed() => Some(field),
        SourceAction::NewToken if field == SecretField::Token => Some(field),
        _ => None,
    }
}

/// The secrets a source's page draws, the one it opened at first.
pub fn secrets_first(source: &SourceRow, first: Option<SecretField>) -> Vec<&SecretSlot> {
    let (mut leading, rest): (Vec<&SecretSlot>, Vec<&SecretSlot>) = source
        .secrets
        .iter()
        .partition(|slot| first.is_some_and(|f| slot.field.is(f)));
    leading.extend(rest);
    leading
}

/// The symbolic icon a source is drawn with, by its kind.
pub fn kind_icon(kind: &Word<SourceKind>) -> &'static str {
    match kind.known() {
        Some(SourceKind::Feed) => "application-rss+xml-symbolic",
        Some(SourceKind::Stream) => "network-transmit-receive-symbolic",
        Some(SourceKind::Webhook) => "insert-link-symbolic",
        Some(SourceKind::LocalPush) => "utilities-terminal-symbolic",
        None => "network-server-symbolic",
    }
}

pub const WATCHER_ICON: &str = "view-reveal-symbolic";
pub const MANDATE_ICON: &str = "security-high-symbolic";
pub const WAITING_ICON: &str = "dialog-question-symbolic";

/// The most a Waiting for you list draws, as the daemon lists at most.
const WAITING_MAX: usize = 100;

/// A waiting proposal's row (§4.7), in Fermix's record words only: who wants to run what, and
/// until when. Show in chat opens Chat at it by its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingLine {
    pub proposal_id: String,
    pub title: String,
    /// "Waits until …", or None for a time this app cannot read.
    pub waits: Option<String>,
}

/// The waiting proposals as rows, oldest first as listed, at most 100. `written` writes a
/// daemon time on the reader's clock as Chat does (`chat::written_time`), None for one it
/// cannot read.
pub fn waiting_lines(
    list: &ProposalList,
    written: impl Fn(&str) -> Option<String>,
) -> Vec<WaitingLine> {
    list.proposals
        .iter()
        .take(WAITING_MAX)
        .map(|p| WaitingLine {
            proposal_id: p.proposal_id.clone(),
            title: words::waiting_row(&p.mandate, &p.tool),
            waits: written(&p.expires_at).map(|t| words::waits_until(&t)),
        })
        .collect()
}

/// A mode in the app's words, and whether it is drawn in the warning style. None for a mode
/// this build does not know, which draws no tag.
pub fn mode_tag(mode: &Word<MandateMode>) -> Option<(&'static str, bool)> {
    match mode.known()? {
        MandateMode::Approve => Some((words::MODE_APPROVE, false)),
        MandateMode::Auto => Some((words::MODE_AUTO, true)),
    }
}

/// One row of a list as drawn: its icon, its name, what qualifies it, a tag, the daemon's
/// sentence with its tone, and the controls it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    pub icon: &'static str,
    pub qualifier: Option<String>,
    /// The mode's words, and whether they are a warning.
    pub tag: Option<(&'static str, bool)>,
    pub sentence: String,
    pub tone: Tone,
    /// Whether the sentence is set in the attention style.
    pub attention: bool,
    /// The switch, and whether it is on, where the daemon publishes one.
    pub switch: Option<bool>,
    pub approve: bool,
}

pub fn source_line(source: &SourceRow) -> Line {
    let tone = source_tone(source);
    Line {
        name: source.name.clone(),
        icon: kind_icon(&source.kind),
        qualifier: None,
        tag: None,
        sentence: source.status_sentence.clone(),
        tone,
        attention: matches!(tone, Tone::Fail | Tone::Warn),
        switch: source_switch(source).then_some(source.enabled),
        approve: false,
    }
}

/// A watcher: its name, the sources it listens to and the daemon's sentence. Never anything it
/// read; its day's counts are on its page.
pub fn watcher_line(watcher: &WatcherRow) -> Line {
    Line {
        name: watcher.name.clone(),
        icon: WATCHER_ICON,
        qualifier: (!watcher.sources.is_empty()).then(|| watcher.sources.join(", ")),
        tag: None,
        sentence: watcher.status_sentence.clone(),
        tone: watcher_tone(watcher),
        attention: watcher_attention(watcher),
        switch: None,
        approve: false,
    }
}

pub fn mandate_line(mandate: &MandateRow) -> Line {
    Line {
        name: mandate.name.clone(),
        icon: MANDATE_ICON,
        qualifier: Some(task_name(mandate)),
        tag: mode_tag(&mandate.mode),
        sentence: mandate.status_sentence.clone(),
        tone: mandate_tone(mandate),
        attention: false,
        switch: mandate_switch(mandate).then_some(mandate.enabled),
        approve: offers_approve(mandate),
    }
}

/// A mandate's task by its name, or its job id where it has none.
pub fn task_name(mandate: &MandateRow) -> String {
    mandate
        .job
        .name
        .clone()
        .unwrap_or_else(|| mandate.job.id.clone())
}

/// Where Approval would be, in the daemon's words alone: the load check's sentence for a mandate
/// that did not load, the approval's own detail where it has one, else the status sentence.
pub fn approval_line(mandate: &MandateRow) -> String {
    mandate
        .load
        .sentence
        .clone()
        .or_else(|| mandate.approval.detail.clone())
        .unwrap_or_else(|| mandate.status_sentence.clone())
}

/// The Cues filter model a watcher filters with: the model in use, the main model where none is
/// chosen, or no filter. A filter this build does not know draws no row.
pub fn watcher_filter(watcher: &WatcherRow) -> Option<String> {
    match watcher.filter.known()? {
        WatcherFilter::None => Some(words::FILTER_NONE.to_owned()),
        WatcherFilter::Llm => Some(
            watcher
                .filter_model
                .as_ref()
                .map_or_else(|| words::FILTER_MAIN_MODEL.to_owned(), |m| m.model.clone()),
        ),
    }
}

/// "Release watch · keeps no notes", and paused where it is.
pub fn watcher_task(watcher: &WatcherRow) -> String {
    let name = watcher
        .task
        .name
        .clone()
        .unwrap_or_else(|| watcher.task.job_id.clone());
    let notes = if watcher.task_notes {
        words::KEEPS_NOTES
    } else {
        words::KEEPS_NO_NOTES
    };
    let mut parts = vec![name, notes.to_owned()];
    if watcher.task.paused {
        parts.push(words::PAUSED.to_owned());
    }
    words::middot(&parts)
}

// ---- a watcher's follow-up ----

/// A watcher's follow-up as its page draws it: its name, else its job; the daemon's sentence,
/// which says in words what holds it; whether it is held, drawn in the warning tone; and whether
/// Ask first again is offered, only while Always allow is on. Granting is a card's alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FollowUpLine {
    pub name: String,
    pub sentence: String,
    pub held: bool,
    pub ask_first_again: bool,
}

/// None for a watcher with no follow-up, and from a daemon older than follow-ups. A state this
/// build does not know is drawn as the daemon's sentence alone, not held.
pub fn follow_up_line(watcher: &WatcherRow) -> Option<FollowUpLine> {
    let follow_up = watcher.follow_up.as_ref()?;
    let name = follow_up
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| follow_up.job_id.clone());
    Some(FollowUpLine {
        name,
        sentence: follow_up.status_sentence.clone(),
        held: follow_up.state.is(FollowUpState::Held),
        ask_first_again: follow_up.always_allowed,
    })
}

/// Ask first again on a watcher's page: one revoke out at a time, its button off while it is,
/// and the daemon's refusal said until the next press.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskFirstAgain {
    sending: bool,
    refusal: Option<String>,
}

impl AskFirstAgain {
    /// A press: whether a revoke goes now. None goes while one is out. It takes the last
    /// refusal away.
    pub fn press(&mut self) -> bool {
        if self.sending {
            return false;
        }
        self.sending = true;
        self.refusal = None;
        true
    }

    /// The daemon answered the revoke; a refusal is said until the next press.
    pub fn answered(&mut self, refusal: Option<String>) {
        assert!(self.sending, "an answer comes to a press");
        assert!(refusal.as_deref() != Some(""), "a refusal says why");
        self.sending = false;
        self.refusal = refusal;
    }

    pub fn sending(&self) -> bool {
        self.sending
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }
}

// ---- the filter model (§4.3) ----

/// The two keys the daemon takes together only: a provider and a model, or both cleared, which
/// is the main model. Choosing a provider holds it here until a model is chosen; nothing is
/// written before then.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterDraft {
    provider: Option<String>,
}

impl FilterDraft {
    /// A provider chosen: held, not written. Choosing the main model is a write, never a hold.
    pub fn hold(&mut self, provider: &str) {
        assert!(
            !provider.is_empty(),
            "the main model is written, never held"
        );
        self.provider = Some(provider.to_owned());
    }

    /// The provider the rows show: the one held, else the one the daemon has in force.
    pub fn provider<'a>(&'a self, saved: &'a str) -> &'a str {
        self.provider.as_deref().unwrap_or(saved)
    }

    /// Whether a provider is held that the daemon does not have yet.
    pub fn holds(&self, saved: &str) -> bool {
        self.provider.as_deref().is_some_and(|p| p != saved)
    }

    pub fn drop_held(&mut self) {
        self.provider = None;
    }

    /// The write for a model chosen, both keys in one apply, or None while the model is blank
    /// or the main model is in force.
    pub fn choose_model(&self, saved: &str, model: &str) -> Option<Map<String, Value>> {
        let provider = self.provider(saved);
        let model = model.trim();
        if provider.is_empty() || model.is_empty() {
            return None;
        }
        Some(pair(provider, model))
    }
}

/// The main model: both keys cleared in one apply.
pub fn main_model() -> Map<String, Value> {
    pair("", "")
}

fn pair(provider: &str, model: &str) -> Map<String, Value> {
    let mut values = Map::new();
    values.insert(FILTER_PROVIDER_KEY.into(), json!(provider));
    values.insert(FILTER_MODEL_KEY.into(), json!(model));
    values
}

/// What the model row suggests: the daemon's catalog, which is the saved provider's, while the
/// rows show the saved provider; a held provider's own list once `providers.models.list` gave
/// it; else nothing, and the model is typed. `listed` names the provider its list is for.
pub fn model_suggestions(
    model_row: &Row,
    draft: &FilterDraft,
    saved: &str,
    listed: Option<(&str, &Listing)>,
) -> Vec<ChoiceOption> {
    let provider = draft.provider(saved);
    if provider == saved {
        return model_row.options.clone();
    }
    let Some((_, Ok(models))) = listed.filter(|(listed_for, _)| *listed_for == provider) else {
        return Vec::new();
    };
    models
        .iter()
        .map(|m| ChoiceOption {
            value: m.id.clone(),
            label: m.label.clone(),
            hint: None,
            disabled: false,
        })
        .collect()
}

/// The model the row shows: the saved one, or none while another provider is held.
pub fn model_shown(model_row: &Row, draft: &FilterDraft, saved: &str) -> String {
    if draft.holds(saved) {
        return String::new();
    }
    text_value(model_row)
}

/// The main model has no model row; any provider, saved or held, has one.
pub fn shows_model_row(draft: &FilterDraft, saved: &str) -> bool {
    !draft.provider(saved).is_empty()
}

// ---- questions asked in place ----

/// A question a row or a page asks in place of its buttons (§4.4 to §4.6), as the phone's Forget
/// does: the first press asks, Cancel withdraws, only the second press acts. A popup is never
/// raised over the pane or a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Question {
    Approve,
    RemoveSource,
    RemoveMandate,
    NewToken,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asking {
    asking: Option<(String, Question)>,
    acting: Option<(String, Question)>,
    /// The daemon's sentence where it refused, by what it was about.
    refusals: Vec<(String, String)>,
}

impl Asking {
    /// The first press: `target` asks. Asking one question withdraws any other.
    pub fn ask(&mut self, target: &str, question: Question) {
        assert!(!target.is_empty(), "a question is about something named");
        if self.acting.is_some() {
            return;
        }
        self.asking = Some((target.to_owned(), question));
        self.refusals.retain(|(t, _)| t != target);
    }

    pub fn withdraw(&mut self) {
        self.asking = None;
    }

    /// The second press: what was asked, which is then being done.
    pub fn confirm(&mut self) -> Option<(String, Question)> {
        if self.acting.is_some() {
            return None;
        }
        let asked = self.asking.take()?;
        self.acting = Some(asked.clone());
        Some(asked)
    }

    /// The daemon answered. A refusal stays with what it refused.
    pub fn finished(&mut self, target: &str, refusal: Option<String>) {
        if self.acting.as_ref().map(|(t, _)| t.as_str()) != Some(target) {
            return;
        }
        self.acting = None;
        if let Some(sentence) = refusal {
            self.refusals.push((target.to_owned(), sentence));
        }
    }

    /// The question `target` asks now, if any.
    pub fn asking(&self, target: &str) -> Option<Question> {
        self.asking
            .as_ref()
            .filter(|(t, _)| t == target)
            .map(|(_, q)| *q)
    }

    /// The question `target` shows: the one it asks, or the one being answered, whose buttons
    /// stay in its place until the daemon answers.
    pub fn shown(&self, target: &str) -> Option<Question> {
        let acting = self.acting.as_ref().filter(|(t, _)| t == target);
        self.asking(target).or(acting.map(|(_, q)| *q))
    }

    /// Whether `target`'s action is running.
    pub fn acting(&self, target: &str) -> bool {
        self.acting.as_ref().is_some_and(|(t, _)| t == target)
    }

    pub fn refusal(&self, target: &str) -> Option<&str> {
        self.refusals
            .iter()
            .find(|(t, _)| t == target)
            .map(|(_, sentence)| sentence.as_str())
    }

    pub fn note_refusal(&mut self, target: &str, sentence: &str) {
        self.refusals.retain(|(t, _)| t != target);
        self.refusals.push((target.to_owned(), sentence.to_owned()));
    }

    pub fn clear_refusal(&mut self, target: &str) {
        self.refusals.retain(|(t, _)| t != target);
    }

    /// A write on `target` that asked nothing first (a switch, a secret stored or cleared, a
    /// copy) was answered: its refusal stays with it until its next action, and an answer takes
    /// away the last one.
    pub fn answered(&mut self, target: &str, refusal: Option<&str>) {
        match refusal {
            Some(sentence) => self.note_refusal(target, sentence),
            None => self.clear_refusal(target),
        }
    }
}

/// The question's sentence about `target`.
pub fn question_text(question: Question, target: &str, listening: &[String]) -> String {
    match question {
        Question::Approve => words::approve_question(target),
        Question::RemoveSource => words::remove_source_question(target, listening),
        Question::RemoveMandate => words::remove_mandate_question(target),
        Question::NewToken => words::new_token_question(target),
    }
}

/// The word on the button that answers the question.
pub fn question_verb(question: Question) -> &'static str {
    match question {
        Question::Approve => words::APPROVE,
        Question::RemoveSource | Question::RemoveMandate => words::REMOVE,
        Question::NewToken => words::MAKE_TOKEN,
    }
}
