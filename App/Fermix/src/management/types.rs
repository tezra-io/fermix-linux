//! The shapes the daemon publishes, decoded.
//!
//! Two rules run through the whole file and they point in opposite directions
//! on purpose.
//!
//! **Results are permissive.** No result struct denies an unknown field, and
//! every field a newer daemon may add carries `#[serde(default)]` with a
//! declared absent rendering. The direction that breaks is a new client talking
//! to an old daemon, so an old client meeting a new field must keep working.
//!
//! **Requests are strict.** Every params object is a typed struct, so the
//! application cannot send a key the method does not publish; the daemon
//! refuses unknown params rather than ignoring them, and building them by
//! construction is what keeps that refusal out of the product.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// hello
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct HelloResult {
    pub protocol: ProtocolRange,
    pub capabilities: HelloCapabilities,
    pub engine: EngineIdentity,
    pub setup: SetupEndpoint,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ProtocolRange {
    pub current_version: u32,
    pub minimum_version: u32,
    pub maximum_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HelloCapabilities {
    pub methods: Vec<String>,
    pub minimum_versions: std::collections::BTreeMap<String, u32>,
}

/// One engine's identity.
///
/// `hello` publishes it, and so does the command line's `running`, because the
/// command line asks the daemon the same question and copies its answer. One
/// concept, one type. It carries `Serialize` so `tests/contract.rs` can
/// re-encode a decoded golden and prove no published field was dropped on the
/// way in.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineIdentity {
    pub engine_id: String,
    pub product_version: String,
    pub build_id: Option<String>,
    pub source_commit: Option<String>,
    pub distribution_identity: String,
    pub artifact_target: Option<String>,
    pub architecture: String,
    pub pid: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupEndpoint {
    pub origin: String,
    pub path: String,
}

// ---------------------------------------------------------------------------
// overview.get
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewResult {
    pub generated_at: Option<String>,
    pub readiness: OverviewReadiness,
    pub health: OverviewHealth,
    pub daemon: OverviewDaemon,
    pub provider: OverviewProvider,
    pub channels: Vec<OverviewChannel>,
    pub memory: OverviewMemory,
    pub jobs: OverviewJobs,
    pub agents: OverviewAgents,
    pub realtime: OverviewRealtime,
    pub capabilities: OverviewCapabilities,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewReadiness {
    pub status: Option<String>,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewHealth {
    pub status: Option<String>,
    pub restart_required: bool,
    /// Absent on an N-1 daemon: "restart to apply" with no reason list.
    #[serde(default)]
    pub restart_reasons: Vec<String>,
    pub providers: Vec<OverviewHealthProvider>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewHealthProvider {
    pub name: Option<String>,
    pub status: Option<String>,
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub primary: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewDaemon {
    pub status: Option<String>,
    pub version: Option<String>,
    pub uptime_ms: Option<u64>,
    pub pid: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewProvider {
    pub active: Option<String>,
    pub model: Option<String>,
    pub auth_mode: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewChannel {
    pub name: String,
    pub status: Option<String>,
    pub enabled: bool,
    pub mode: Option<String>,
    pub process_alive: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewMemory {
    pub repo: Option<String>,
    pub conversation_store: Option<String>,
    pub store: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewJobs {
    pub scheduled: u32,
    pub running: u32,
    pub paused: u32,
    pub failed_recent: u32,
    pub next: Option<serde_json::Value>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewAgents {
    pub main: OverviewMainAgent,
    pub skill_workers: u32,
    pub running_skill_workers: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewMainAgent {
    pub health: Option<String>,
    pub activity: Option<String>,
    pub status: Option<String>,
    pub active_conversations: u32,
    pub pending_conversations: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverviewRealtime {
    pub enabled: bool,
    pub status: Option<String>,
    pub provider: Option<String>,
    /// Null while voice is disabled, and absent on a daemon that predates the
    /// two voice engines.
    #[serde(default)]
    pub engine: Option<String>,
    pub model: Option<String>,
    pub socket_alive: Option<bool>,
    pub active_sessions: u32,
    pub active_clients: u32,
    pub companion_connected: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct OverviewCapabilities {
    pub builtin: u32,
    pub skill: u32,
    pub mcp: u32,
    pub total: u32,
}

// ---------------------------------------------------------------------------
// setup.state.get
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SetupStateResult {
    pub readiness: SetupReadiness,
    pub restart: RestartState,
    pub providers: Vec<SetupProviderRow>,
    pub channels: Vec<SetupChannelRow>,
    pub personalization: SetupPersonalization,
    pub features: SetupFeatures,
    pub profile: Option<String>,
    pub coexistence: SetupCoexistence,
    /// Where secrets live and whether one can be stored right now.
    ///
    /// Absent on an engine that predates the row, which is not the same as
    /// knowing there is no store: one says nothing has been said, the other
    /// would be a claim nobody made.
    #[serde(default)]
    pub secrets: Option<SetupSecrets>,
}

/// The two secret-store facts, which one field could not carry.
///
/// A locked keyring is `store: "keyring"` with `availability: "locked"`, and
/// a home on the file store is `availability: "ready"` whatever the keyring
/// is doing, because that is what consenting to it bought. Answering both
/// with one enum is the collapse this work exists to undo.
#[derive(Debug, Clone, Deserialize)]
pub struct SetupSecrets {
    /// Where values live. `keyring` or `file`, never null and never "none".
    pub store: String,
    /// Whether a secret can be stored now: `ready`, `locked` or
    /// `unavailable`.
    pub availability: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupReadiness {
    pub status: Option<String>,
    pub failures: Vec<ReadinessFailure>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadinessFailure {
    pub component: String,
    pub gating: bool,
    pub pane: SettingsPane,
    /// The closed-set copy key. Home renders one of four templates from its
    /// family; `pane` is only the deep link.
    pub detail_key: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RestartState {
    pub required: bool,
    #[serde(default)]
    pub reasons: Vec<RestartReason>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RestartReason {
    pub section: String,
    /// The daemon's own sentence. Never composed here.
    pub sentence: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupProviderRow {
    pub id: String,
    pub label: String,
    pub auth_modes: Vec<String>,
    pub auth_mode: Option<String>,
    pub configured: bool,
    pub primary: bool,
    pub present_key: bool,
    pub default_model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub fast: Option<bool>,
    pub account_label: Option<String>,
    pub token_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupChannelRow {
    pub name: String,
    pub enabled: bool,
    pub configured: bool,
    pub status: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupPersonalization {
    pub present: SetupPersonalizationPresent,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SetupPersonalizationPresent {
    pub user_name: bool,
    pub timezone: bool,
    pub communication_style: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SetupFeatures {
    pub voice: bool,
    pub voice_notes: bool,
    pub meetings: bool,
    pub computer_use: bool,
    pub computer_history: SetupComputerHistory,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct SetupComputerHistory {
    pub enabled: bool,
    pub installed: bool,
    pub ready: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupCoexistence {
    pub legacy_service_unit: LegacyServiceUnit,
    pub config_state: ConfigState,
    pub secret_acl_restricted: SecretAclRestricted,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LegacyServiceUnit {
    pub present: bool,
    pub scope: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecretAclRestricted {
    /// Null until Doctor has run, which is not the same answer as false.
    pub present: Option<bool>,
    #[serde(default)]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigState {
    Clear,
    ExternalChange,
    ConfigUnreadable,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsPane {
    /// The pane a window with nothing remembered opens on.
    #[default]
    Providers,
    Personality,
    Memory,
    Channels,
    Integrations,
    Voice,
    Meetings,
    Computer,
    Coding,
    Search,
    Images,
    Sandbox,
    Permissions,
    /// A pane a newer daemon publishes and this build cannot route to. The row
    /// still renders; only its deep link is withheld.
    #[serde(other)]
    Unrecognized,
}

// ---------------------------------------------------------------------------
// settings.*
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsSectionsResult {
    pub sections: Vec<SettingsSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsSection {
    pub id: String,
    pub pane: SettingsPane,
    pub title: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsGetResult {
    pub id: String,
    pub title: String,
    pub rows: Vec<SettingsRow>,
}

/// One settings row. Every field is present on every row, `null` where it does
/// not apply, so one shape decodes six kinds of control.
#[derive(Debug, Clone, Deserialize)]
pub struct SettingsRow {
    pub key: String,
    pub kind: SettingsRowKind,
    pub label: String,
    pub footer: Option<String>,
    pub value: SettingValue,
    /// Meaningful on a secret row only. A secret row never carries a value.
    pub present: Option<bool>,
    #[serde(default)]
    pub options: Vec<SettingsOption>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub restart: bool,
    pub read_only: bool,
    /// On a choice row, whether an off-list value is accepted.
    pub suggestions: bool,
    /// Meaningful on a number row only.
    pub unit: Option<String>,
    /// Meaningful on a number row only.
    pub format: Option<SettingsNumberFormat>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsRowKind {
    Toggle,
    Choice,
    Text,
    Number,
    Secret,
    List,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsNumberFormat {
    Integer,
    Percent,
    CurrencyCents,
    Minutes,
    Hours,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsOption {
    pub value: String,
    pub label: String,
    pub hint: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

/// A row's value: a scalar, a list of strings, or nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    List(Vec<String>),
    Toggle(bool),
    Number(f64),
    Text(String),
    Absent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsApplyResult {
    /// Every key the daemon wrote, including keys it derived and the operator
    /// never typed.
    pub applied: Vec<String>,
    pub restart: RestartState,
    pub readiness: ReadinessSummary,
    /// One sentence per derived change. The daemon's words.
    #[serde(default)]
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadinessSummary {
    pub status: Option<String>,
    pub failure_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SettingsReloadResult {
    pub reloaded: bool,
    pub restart: RestartState,
    pub readiness: ReadinessSummary,
    pub config_state: ConfigState,
}

// ---------------------------------------------------------------------------
// secret.*
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SecretResult {
    pub id: String,
    /// Presence, never a value. Secrets travel inbound only.
    pub present: bool,
    pub restart: RestartState,
    /// Which store took it, so a client never infers that from what it asked
    /// for. Absent on an engine that predates the field.
    ///
    /// `secret.clear` carries it too, and reports rather than decides: the
    /// store a home uses should not appear and disappear between two calls
    /// about the same secret, and forgetting one secret is not a decision
    /// about where the next one goes.
    #[serde(default)]
    pub store: Option<String>,
}

// ---------------------------------------------------------------------------
// doctor.*
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct DoctorSession {
    pub session_id: String,
    pub scope: DoctorScope,
    pub status: DoctorSessionStatus,
    pub budget_ms: u64,
    pub duration_ms: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub total: u32,
    pub completed_count: u32,
    pub summary: DoctorSummary,
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorScope {
    Local,
    Network,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSessionStatus {
    Running,
    Completed,
    Cancelled,
    TimedOut,
    Failed,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct DoctorSummary {
    pub passed: u32,
    pub warning: u32,
    pub failed: u32,
    pub not_applicable: u32,
    pub unavailable: u32,
    pub skipped: u32,
    pub cancelled: u32,
    pub timed_out: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DoctorCheck {
    pub id: String,
    pub category: DoctorCategory,
    pub severity: DoctorSeverity,
    pub applicability: DoctorApplicability,
    pub origin: String,
    pub status: CheckStatus,
    /// The daemon's own sentence about this check.
    pub summary: String,
    #[serde(default)]
    pub evidence: serde_json::Map<String, serde_json::Value>,
    pub remediation_code: Option<String>,
    pub duration_ms: u64,
    pub finished_at: String,
    /// Absent on an N-1 daemon: the summary renders with no action button.
    #[serde(default)]
    pub remediation: Option<Remediation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Warning,
    Failed,
    NotApplicable,
    Unavailable,
    Skipped,
    Cancelled,
    TimedOut,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCategory {
    Runtime,
    Configuration,
    Security,
    Capability,
    Connectivity,
    Distribution,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    Critical,
    Warning,
    Info,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorApplicability {
    Always,
    Configured,
    Platform,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Remediation {
    /// The daemon's own title. Never composed here.
    pub title: String,
    pub body: String,
    pub action: RemediationAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RemediationAction {
    pub kind: RemediationKind,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationKind {
    SettingsPane,
    SystemSettings,
    Job,
    Restart,
    Reload,
    Instructions,
    None,
    #[serde(other)]
    Unrecognized,
}

// ---------------------------------------------------------------------------
// logs.query
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct LogsQueryResult {
    pub entries: Vec<LogEntry>,
    pub count: u32,
    pub truncated: bool,
    pub direction: LogDirection,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LogEntry {
    pub time: String,
    pub level: LogLevel,
    pub subsystem: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Emergency,
    Alert,
    Critical,
    Error,
    Warning,
    Notice,
    Info,
    Debug,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogDirection {
    Backward,
    Forward,
}

// ---------------------------------------------------------------------------
// lifecycle.* and diagnostics.build
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct LifecyclePrepareResult {
    pub lease_id: String,
    /// Relative, deliberately: the daemon's expiry runs on monotonic time, so a
    /// client starts its own timer on receipt.
    pub ttl_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LifecycleLeaseResult {
    pub lease_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsBuildResult {
    pub schema_version: u32,
    pub generated_at: String,
    pub engine: EngineIdentity,
    pub protocol: ProtocolRange,
    pub service: DiagnosticsService,
    pub doctor: Option<serde_json::Value>,
    pub logs: DiagnosticsLogs,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsService {
    pub scope: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiagnosticsLogs {
    pub count: u32,
    pub truncated: bool,
    pub entries: Vec<LogEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetupSessionResult {
    pub url: String,
    pub expires_at_ms: u64,
}

// ---------------------------------------------------------------------------
// setup.detect
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct SetupDetectResult {
    pub results: Vec<DetectResult>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DetectResult {
    pub target: DetectTarget,
    pub present: bool,
    pub detail: Option<String>,
    /// Null while the notetaker is absent, which is not the same answer as
    /// signed out.
    #[serde(default)]
    pub signed_in: Option<bool>,
    #[serde(default)]
    pub vendors: Vec<HarnessVendor>,
    #[serde(default)]
    pub guidance: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectTarget {
    ExistingPrimary,
    ClaudeCode,
    CodexCli,
    Ollama,
    HarnessVendors,
    Meetbot,
    #[serde(other)]
    #[serde(skip_serializing)]
    Unrecognized,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HarnessVendor {
    pub vendor: String,
    pub installed: bool,
    pub version: Option<String>,
    pub auth: String,
}

// ---------------------------------------------------------------------------
// providers.*
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersSetPrimaryResult {
    pub restart: RestartState,
    #[serde(default)]
    pub side_effects: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProvidersModelsListResult {
    pub models: Vec<ProviderModel>,
    pub cursor: Option<String>,
    /// Where the rows came from. A live listing never degrades to the catalog.
    pub source: ModelSource,
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderModel {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    Catalog,
    Live,
    #[serde(other)]
    Unrecognized,
}

// ---------------------------------------------------------------------------
// jobs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct JobView {
    pub job_id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    /// Display copy from a closed per-kind vocabulary, never a state.
    pub phase: Option<String>,
    pub progress: Option<JobProgress>,
    pub budget_ms: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub result: Option<serde_json::Value>,
    /// Carries the daemon's own sentence.
    pub failure: Option<JobFailure>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobListResult {
    pub jobs: Vec<JobView>,
}

/// `auth.start` answers a job view plus the authorize url, returned once.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthStartResult {
    #[serde(flatten)]
    pub job: JobView,
    #[serde(default)]
    pub authorize_url: Option<String>,
    #[serde(default)]
    pub expires_in_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthLogoutResult {
    pub restart: RestartState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    ProviderProbe,
    Auth,
    AuthImport,
    PluginInstall,
    PluginCheck,
    PluginWorkspacesDiscover,
    PluginWorkspaceSelect,
    CapabilityInstall,
    MeetingsSignin,
    ComputerUseGrant,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobProgress {
    pub done: u64,
    pub total: Option<u64>,
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JobFailure {
    pub code: JobFailureCode,
    /// The daemon's own sentence. A terminal status word is not a diagnosis.
    pub sentence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobFailureCode {
    Unavailable,
    Refused,
    TimedOut,
    InternalError,
    #[serde(other)]
    Unrecognized,
}

// ---------------------------------------------------------------------------
// plugins.*
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct PluginsListResult {
    pub plugins: Vec<PluginRow>,
    #[serde(default)]
    pub oauth_clients: Vec<PluginOAuthClient>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginRowResult {
    pub plugin: PluginRow,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PluginOAuthClientResult {
    pub oauth_client: PluginOAuthClient,
}

/// One shape for two halves: an installed plugin and a catalog entry that has
/// never been fetched publish the same fields, and `installed` separates them.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginRow {
    pub name: String,
    pub title: String,
    pub version: Option<String>,
    /// Null for a plugin that runs inside Fermix itself.
    pub runtime_kind: Option<PluginRuntimeKind>,
    pub auth_kind: Option<String>,
    pub auth_provider: Option<String>,
    pub installed: bool,
    pub enabled: bool,
    /// Published for logs and support. Nothing routes on it.
    pub status: String,
    /// The daemon's own sentence.
    pub status_sentence: String,
    /// The word the row leads with, or null where the next step is not a button
    /// this surface owns.
    pub primary_verb: Option<String>,
    /// The method that word runs. Null exactly when `primary_verb` is.
    pub primary_action: Option<PluginAction>,
    /// The words to draw.
    #[serde(default)]
    pub verbs: Vec<String>,
    /// The method each word runs, in the same order.
    #[serde(default)]
    pub actions: Vec<PluginAction>,
    #[serde(default)]
    pub settings: Vec<PluginSetting>,
    pub account_label: Option<String>,
    pub credential_present: bool,
    /// Always present, and it names where the code runs.
    pub consent_sentence: String,
    pub remote_disclosure: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub access_profiles: Vec<PluginAccessProfile>,
    #[serde(default)]
    pub workspaces: Vec<PluginWorkspace>,
    pub workspace_id: Option<String>,
    pub workspace_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeKind {
    LocalStdio,
    RemoteMcp,
    #[serde(other)]
    Unrecognized,
}

/// The closed routing ids. A word is not a routing key: the row paints
/// `verbs[i]` and routes on `actions[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAction {
    Install,
    Enable,
    Disable,
    SignIn,
    AddToken,
    ReplaceToken,
    SetUpClient,
    ChooseWorkspace,
    Check,
    Disconnect,
    /// An id a newer daemon publishes and this build cannot route. Its word is
    /// not painted, because a button that routes nowhere is worse than none.
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginSetting {
    pub key: String,
    pub label: String,
    pub value: Option<String>,
    pub required: bool,
    /// Optional for older protocol-2 engines; an absent one reads as text.
    #[serde(default)]
    pub kind: Option<PluginSettingKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginSettingKind {
    Text,
    Boolean,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginAccessProfile {
    pub id: String,
    pub label: String,
    pub write: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginWorkspace {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginOAuthClient {
    pub provider: String,
    pub configured: bool,
    pub redirect_port: Option<u32>,
    /// Optional for older protocol-2 engines.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Reports the stored secret independently of `configured`.
    #[serde(default)]
    pub secret_present: bool,
    #[serde(default)]
    pub region: Option<String>,
    /// An absent list reads as a provider with one region, never as a picker
    /// that failed to arrive.
    #[serde(default)]
    pub regions: Vec<PluginOAuthRegion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PluginOAuthRegion {
    pub id: String,
    pub label: String,
}

// ---------------------------------------------------------------------------
// computer_use.permissions.get
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct ComputerUsePermissions {
    /// From the installer, not from the probe: the feature being switched off
    /// says nothing about whether the helper is on disk.
    pub installed: bool,
    pub screen_capture: bool,
    pub input_control: bool,
    pub probed_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Request parameters, one typed struct per method that takes any
// ---------------------------------------------------------------------------

/// The params of a method that takes none. Serializes to `{}`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct NoParams {}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorStartParams {
    pub scope: DoctorScope,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorSessionParams {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogsQueryParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<LogLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<LogDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LifecycleLeaseParams {
    pub lease_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsGetParams {
    pub section: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettingsApplyParams {
    pub section: String,
    pub values: std::collections::BTreeMap<String, SettingValue>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretSetParams {
    pub id: String,
    /// Inbound only, and never blank: a blank value is refused before it is
    /// sent, in the one dialog that owns a secret entry.
    pub value: String,
    /// Which store to write to. Absent means the keyring.
    ///
    /// Sending `file` IS the owner's consent, carried per call: there is no
    /// persisted flag this application sets and no automatic fallback, so a
    /// value can only reach the file store from a button that said so.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<&'static str>,
    /// Whether the engine should wait for the owner to finish unlocking.
    ///
    /// Absent rather than `false` when it was not asked for, so an engine that
    /// predates the parameter sees exactly the request it saw before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unlock: Option<bool>,
}

/// `secret.migrate_to_keyring`: move every file-stored secret back.
///
/// It takes no value and names no secret, which is the point: the owner cannot
/// retype what this application cannot read, so the way home moves what the
/// engine already holds.
#[derive(Debug, Clone, Serialize)]
pub struct SecretMigrateParams {
    pub unlock: bool,
}

/// What the migration moved.
#[derive(Debug, Clone, Deserialize)]
pub struct SecretMigrateResult {
    /// The secrets that reached the keyring, by id. Empty when none had to.
    #[serde(default)]
    pub moved: Vec<String>,
    /// Where values live now.
    pub store: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SecretClearParams {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupDetectParams {
    pub targets: Vec<DetectTarget>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderParams {
    pub provider: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvidersModelsListParams {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobParams {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthImportParams {
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginNameParams {
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginWorkspaceSelectParams {
    pub name: String,
    pub profile: String,
    pub workspace_id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginOAuthClientSetParams {
    pub provider: String,
    pub client_id: String,
    pub redirect_port: u32,
    /// Required exactly where the client row publishes a non-empty `regions`,
    /// and refused where it publishes none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginSettingSetParams {
    pub name: String,
    pub key: String,
    /// Always a string. A boolean setting takes only "true" or "false".
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilitiesInstallParams {
    pub target: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_restart_reason_list_decodes_as_empty() {
        let health: OverviewHealth = serde_json::from_value(serde_json::json!({
            "status": "ok",
            "restart_required": true,
            "providers": []
        }))
        .expect("decodes without restart_reasons");

        assert!(health.restart_required);
        assert!(health.restart_reasons.is_empty());
    }

    #[test]
    fn a_field_a_newer_daemon_adds_does_not_break_an_older_client() {
        let summary: ReadinessSummary = serde_json::from_value(serde_json::json!({
            "status": "ready",
            "failure_count": 0,
            "a_field_from_the_future": {"nested": true}
        }))
        .expect("an unknown field is tolerated on a result");

        assert_eq!(summary.failure_count, 0);
    }

    #[test]
    fn an_unknown_enum_member_is_kept_rather_than_refused() {
        let pane: SettingsPane =
            serde_json::from_value(serde_json::json!("a_pane_from_the_future")).expect("decodes");
        assert_eq!(pane, SettingsPane::Unrecognized);
    }

    #[test]
    fn a_setting_value_decodes_every_shape_it_can_take() {
        let cases = [
            (
                serde_json::json!("a string"),
                SettingValue::Text("a string".into()),
            ),
            (serde_json::json!(true), SettingValue::Toggle(true)),
            (serde_json::json!(7), SettingValue::Number(7.0)),
            (
                serde_json::json!(["one", "two"]),
                SettingValue::List(vec!["one".into(), "two".into()]),
            ),
            (serde_json::json!(null), SettingValue::Absent),
        ];

        for (json, expected) in cases {
            let decoded: SettingValue = serde_json::from_value(json).expect("decodes");
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn params_carry_only_the_keys_the_method_publishes() {
        let params = SettingsGetParams {
            section: "memory".into(),
        };
        assert_eq!(
            serde_json::to_value(&params).expect("serializes"),
            serde_json::json!({"section": "memory"})
        );
    }

    #[test]
    fn a_method_with_no_params_serializes_to_an_empty_object() {
        assert_eq!(
            serde_json::to_value(NoParams {}).expect("serializes"),
            serde_json::json!({})
        );
    }
}
