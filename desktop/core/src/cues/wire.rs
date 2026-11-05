//! The `cues.*` answers as the app reads them (management `PROTOCOL.md`, the cue notes). Field
//! names are the wire's. A null and an absent key decode alike, unknown fields are ignored, and
//! every closed vocabulary decodes to a `Word`, so a word this build does not know is kept as
//! the daemon wrote it rather than refused.
//!
//! No row carries cue text. The values Fermix hands out, a local push token, the secret it makes
//! for a GitHub source and a webhook's address, decode as `HandedOut`, whose `Debug` never shows
//! them.

use crate::settings::Row;
use serde::{Deserialize, Serialize};
use std::fmt;

/// One word of a closed vocabulary: one this build knows, or the daemon's own word for one it
/// does not. An unknown word draws the daemon's sentence and no control.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum Word<T> {
    Known(T),
    Unknown(String),
}

impl<T: Copy + PartialEq> Word<T> {
    pub fn is(&self, known: T) -> bool {
        matches!(self, Word::Known(k) if *k == known)
    }

    pub fn known(&self) -> Option<T> {
        match self {
            Word::Known(k) => Some(*k),
            Word::Unknown(_) => None,
        }
    }
}

/// A value Fermix hands out once or on request: shown or copied, never logged. Its `Debug` says
/// only that one is there.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct HandedOut(String);

impl HandedOut {
    pub fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for HandedOut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("HandedOut(..)")
    }
}

// ---- vocabularies (`x-cue-vocabulary`, and the enums inside the cue rows) ----

/// Where the cue subsystem stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CueState {
    Off,
    NeedsMemory,
    NeedsRestart,
    Running,
}

/// Whether the door that takes pushes and webhooks is listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorState {
    Listening,
    Unavailable,
    NotRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    LocalPush,
    Feed,
    Stream,
    Webhook,
}

/// Published for logs and support: a row draws its sentence. The one thing read from it is the
/// failure tone of a source that stopped working, which moves no control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Off,
    NeedsSecret,
    NotRunning,
    Idle,
    Polling,
    Connected,
    Reconnecting,
    AuthFailed,
    OverBudget,
    Stopped,
    Unavailable,
}

/// Which control a source row draws. Edit and remove are the app's own and never arrive here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceAction {
    Enable,
    Disable,
    SetSecret,
    NewToken,
    GetUrl,
}

/// One secret a source's kind declares: `key` and `secret` are what the owner types, `token` and
/// `path_secret` are Fermix's to make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretField {
    Token,
    Key,
    Secret,
    PathSecret,
}

impl SecretField {
    /// Whether the owner types this value with `secret.set`; the daemon refuses the other two.
    pub fn typed(self) -> bool {
        matches!(self, SecretField::Key | SecretField::Secret)
    }

    pub fn wire(self) -> &'static str {
        match self {
            SecretField::Token => "token",
            SecretField::Key => "key",
            SecretField::Secret => "secret",
            SecretField::PathSecret => "path_secret",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherState {
    On,
    Paused,
    /// Saved before a rule every save now keeps: it gets nothing until it is changed in chat.
    NeedsUpdate,
    /// Its filter cannot reach its model.
    Held,
}

/// Whether a watcher's follow-up can run now. Published for logs and support: the row draws its
/// sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FollowUpState {
    Ready,
    /// It cannot run: watchers may not act, or a tool or channel it needs is not set up.
    Held,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WatcherFilter {
    Llm,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateMode {
    Approve,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateAction {
    Approve,
    Enable,
    Disable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MandateLoad {
    Active,
    Inactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    Unapproved,
    Approved,
    Changed,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Waiting,
    Confirmed,
    Denied,
    Expired,
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecidedFrom {
    Telegram,
    Signal,
    Whatsapp,
    Imessage,
    Companion,
    Mobile,
    Management,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus {
    Ok,
    Error,
    Unknown,
}

// ---- status ----

/// `cues.status`. Every build has cues, so `available` is always true and is not read.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CueStatus {
    pub enabled: bool,
    pub running: bool,
    pub memory_enabled: bool,
    pub mandates_enabled: bool,
    pub state: Word<CueState>,
    pub status_sentence: String,
    pub door: Door,
    pub counts: StatusCounts,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Door {
    pub state: Word<DoorState>,
    pub port: u16,
    #[serde(default)]
    pub reason: Option<String>,
    pub sentence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct StatusCounts {
    pub sources: u64,
    pub sources_attention: u64,
    pub watchers: u64,
    pub watchers_held: u64,
    pub mandates: u64,
    pub mandates_attention: u64,
    pub proposals_waiting: u64,
}

// ---- sources ----

/// One secret a source's kind declares, and whether it is stored. Never a value.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SecretSlot {
    pub field: Word<SecretField>,
    pub label: String,
    pub present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct SourceCounts {
    pub window_s: u64,
    pub received: u64,
    pub routed: u64,
    pub not_routed: u64,
    pub open_gaps: u64,
}

/// One source in the daemon's own words. Never a payload, an item's text, a cursor, a secret's
/// value or a webhook's address.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceRow {
    #[serde(rename = "ref")]
    pub name: String,
    pub kind: Word<SourceKind>,
    #[serde(default)]
    pub preset: Option<String>,
    /// The preset's or the kind's name.
    pub label: String,
    pub enabled: bool,
    pub state: Word<SourceState>,
    pub status_sentence: String,
    #[serde(default)]
    pub primary_action: Option<Word<SourceAction>>,
    #[serde(default)]
    pub actions: Vec<Word<SourceAction>>,
    /// A feed's or stream's host, all a source's page draws of its address, or an X accounts
    /// source's handles. A row never decodes the address itself (`url`): it can carry a
    /// credential, in its user, its path or its query (M60 cues §4.4). The owner edits it through
    /// the form's values.
    #[serde(default)]
    pub location: Option<String>,
    /// Items a UTC day; 0 is no limit.
    pub daily_budget: u64,
    #[serde(default)]
    pub secrets: Vec<SecretSlot>,
    /// The field `set_secret` or `new_token` points at.
    #[serde(default)]
    pub missing_secret: Option<Word<SecretField>>,
    #[serde(default)]
    pub has_url: bool,
    pub counts: SourceCounts,
    #[serde(default)]
    pub last_good_at: Option<String>,
    #[serde(default)]
    pub last_accepted_at: Option<String>,
    /// The enabled watchers that listen to it.
    #[serde(default)]
    pub listening: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceList {
    pub sources: Vec<SourceRow>,
    pub can_add: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Gap {
    pub reason: String,
    pub count: u64,
    pub sentence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Rejected {
    pub reason: String,
    pub count: u64,
    pub sentence: String,
}

/// `cues.sources.get`: the row, its editable fields as settings rows, its open gaps and its
/// refused requests, each with the daemon's sentence.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceDetail {
    pub source: SourceRow,
    #[serde(default)]
    pub values: Vec<Row>,
    #[serde(default)]
    pub gaps: Vec<Gap>,
    #[serde(default)]
    pub rejected: Vec<Rejected>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FormKind {
    pub id: String,
    pub label: String,
    pub sentence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FormPreset {
    pub id: String,
    pub label: String,
    pub kind: String,
}

/// `cues.sources.form`: the add form for one choice of kind, preset and check. The rows are the
/// daemon's; the app writes no field list per kind.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceForm {
    pub kinds: Vec<FormKind>,
    pub presets: Vec<FormPreset>,
    pub rows: Vec<Row>,
    #[serde(default)]
    pub consent_sentence: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    /// What the choice reads with (X accounts: the X sign-in), ready or not, in the daemon's
    /// words. None for every other choice, and from a daemon older than v0.18.
    #[serde(default)]
    pub requirement: Option<Requirement>,
}

/// Whether what a choice reads with is set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Ready,
    NeedsSignIn,
}

/// A form's requirement: the daemon's sentence, drawn as written, and whether it is met; a state
/// this build does not know is drawn as not met. It never stops the add.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Requirement {
    pub state: Word<RequirementState>,
    pub sentence: String,
}

/// `cues.sources.add`: the new row under the name in force, and what Fermix made for it, each
/// handed out once: a new local push source's first token, or a new GitHub source's secret.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceAdded {
    pub source: SourceRow,
    #[serde(default)]
    pub token: Option<HandedOut>,
    #[serde(default)]
    pub secret: Option<HandedOut>,
}

/// The answer of `cues.sources.update`, `.enable` and `.disable`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceAnswer {
    pub source: SourceRow,
}

/// `cues.sources.remove`: any secret that could not be deleted, as sentences, and the watchers
/// that listened to it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SourceRemoved {
    #[serde(rename = "ref")]
    pub name: String,
    #[serde(default)]
    pub left: Vec<String>,
    #[serde(default)]
    pub watchers: Vec<String>,
}

/// `cues.sources.token.new`: a fresh token, handed out once.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewToken {
    #[serde(rename = "ref")]
    pub name: String,
    pub token: HandedOut,
}

/// `cues.sources.url.get`: a webhook's address, handed out on every ask and never drawn.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Address {
    #[serde(rename = "ref")]
    pub name: String,
    pub url: HandedOut,
    pub path: HandedOut,
}

// ---- watchers ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FilterModel {
    #[serde(default)]
    pub provider: Option<String>,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WatcherTask {
    pub job_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub paused: bool,
}

/// The job a passing item sets off, handed nothing from the item. It is made in chat; only its
/// Always allow is taken back here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct FollowUp {
    pub job_id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Whether the owner's Always allow lets it run with no card. A daemon older than Always allow
    /// sends none, which reads as asking first.
    #[serde(default)]
    pub always_allowed: bool,
    pub state: Word<FollowUpState>,
    pub status_sentence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct WatcherCounts {
    /// Every item its words to listen for let through, kept or not.
    pub offered: u64,
    pub waiting: u64,
    pub passed: u64,
    pub dropped: u64,
    pub acted: u64,
    pub failed: u64,
}

/// One watcher, read-only: watchers are made and changed in chat. Never an item's text, a reason
/// or a hint.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WatcherRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub filter: Word<WatcherFilter>,
    /// The one Cues filter model every `llm` watcher filters with; None for the main model.
    #[serde(default)]
    pub filter_model: Option<FilterModel>,
    pub state: Word<WatcherState>,
    pub status_sentence: String,
    #[serde(default)]
    pub sources: Vec<String>,
    pub task: WatcherTask,
    /// None for a watcher with none, and from a daemon older than follow-ups.
    #[serde(default)]
    pub follow_up: Option<FollowUp>,
    pub task_notes: bool,
    /// 0 is none.
    pub daily_cap: u64,
    pub counts: WatcherCounts,
    #[serde(default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WatcherList {
    pub watchers: Vec<WatcherRow>,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Instruction {
    pub number: u64,
    pub text: String,
    #[serde(default)]
    pub added_at: Option<String>,
}

/// How many of a watcher's examples are marked each way. Examples are counted, never shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Examples {
    pub pass: u64,
    pub drop: u64,
}

/// `cues.watchers.get`: the row plus the owner's own words, its purpose, its words to listen for
/// and its instructions.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WatcherDetail {
    #[serde(flatten)]
    pub row: WatcherRow,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub listen_for: Vec<String>,
    #[serde(default)]
    pub instructions: Vec<Instruction>,
    pub examples: Examples,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WatcherAnswer {
    pub watcher: WatcherDetail,
}

/// `cues.watchers.always_allow.revoke`: the watcher's row once its follow-up asks first again.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlwaysAllowRevoked {
    pub watcher: WatcherRow,
}

// ---- mandates ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MandateJob {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Validator {
    pub path: String,
    #[serde(default)]
    pub interpreter: Option<String>,
    /// The environment variables it is given, by name.
    #[serde(default)]
    pub env: Vec<String>,
}

/// A pinned argument's value, which keeps its own type: text, a number, or true or false.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PinValue {
    Flag(bool),
    Number(serde_json::Number),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Pin {
    pub key: String,
    pub value: PinValue,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Load {
    pub state: Word<MandateLoad>,
    #[serde(default)]
    pub sentence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Approval {
    /// None where memory is off and the approval cannot be read.
    #[serde(default)]
    pub state: Option<Word<ApprovalState>>,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct MandateCounts {
    pub window_s: u64,
    pub allowed: u64,
    pub proposed: u64,
    pub refused: u64,
}

/// One mandate. Durations are whole seconds; paths are the owner's own configuration, published
/// as written.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MandateRow {
    pub name: String,
    pub enabled: bool,
    pub job: MandateJob,
    pub mode: Word<MandateMode>,
    pub approve_within_s: u64,
    /// 0 is no limit.
    pub max_cue_age_s: u64,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub read_tools: Vec<String>,
    #[serde(default)]
    pub validator: Option<Validator>,
    #[serde(default)]
    pub idempotency_arg: Option<String>,
    #[serde(default)]
    pub unpinned: Vec<String>,
    #[serde(default)]
    pub pin_args: Vec<Pin>,
    pub load: Load,
    pub approval: Approval,
    pub counts: MandateCounts,
    /// What the mode lets the task do.
    pub consent_sentence: String,
    pub status_sentence: String,
    #[serde(default)]
    pub primary_action: Option<Word<MandateAction>>,
    #[serde(default)]
    pub actions: Vec<Word<MandateAction>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MandateList {
    pub mandates: Vec<MandateRow>,
    pub can_add: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChoiceJob {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub watcher: Option<String>,
    #[serde(default)]
    pub mandate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChoiceTool {
    pub name: String,
    pub plugin: String,
    #[serde(default)]
    pub title: Option<String>,
    pub read_only: bool,
}

/// `cues.mandates.choices`: the jobs that run on cues and the installed, enabled `http` plugin
/// tools a mandate may name.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MandateChoices {
    pub jobs: Vec<ChoiceJob>,
    pub tools: Vec<ChoiceTool>,
}

/// The answer of every mandate write but remove.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MandateAnswer {
    pub mandate: MandateRow,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MandateRemoved {
    pub name: String,
}

// ---- proposals ----

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposalTask {
    pub job_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// Fermix's record of a proposal: ids, names from the owner's configuration, times and Fermix's
/// sentence. Never an argument or cue text.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposalRecord {
    pub proposal_id: String,
    pub state: Word<ProposalState>,
    pub mandate: String,
    pub tool: String,
    pub task: ProposalTask,
    #[serde(default)]
    pub watcher: Option<String>,
    #[serde(default)]
    pub source_ref: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub decided_at: Option<String>,
    #[serde(default)]
    pub decided_from: Option<Word<DecidedFrom>>,
    #[serde(default)]
    pub result_status: Option<Word<ResultStatus>>,
    pub status_sentence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposalList {
    pub proposals: Vec<ProposalRecord>,
    pub waiting: u64,
}

/// `cues.proposals.get`'s card, Fermix's half of it. The outside text it also carries is not
/// read here at all: only Chat's card draws it, as outside text (M60 cues §5.5).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposalCard {
    #[serde(flatten)]
    pub record: ProposalRecord,
    #[serde(default)]
    pub received_at: Option<String>,
    /// Fermix's sentence, with no outside text in it.
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposalCardAnswer {
    pub proposal: ProposalCard,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProposalAnswer {
    pub proposal: ProposalRecord,
}

/// Which proposals `cues.proposals.list` reads: the waiting ones, or the last seven days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalFilter {
    Waiting,
    Recent,
}

/// The owner's answer to a proposal, in the words `cues.proposals.decide` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Confirm,
    Deny,
}
