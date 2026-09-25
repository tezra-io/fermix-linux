//! The daemon's answers, as the app reads them. Field names are the wire's.
//! Unknown fields are ignored so a newer daemon's additions never break an older app.

use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SetupState {
    pub readiness: Readiness,
    pub restart: RestartState,
    pub providers: Vec<ProviderRow>,
    pub channels: Vec<ChannelRow>,
    pub features: Features,
    pub coexistence: Coexistence,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChannelRow {
    pub name: String,
    pub enabled: bool,
    pub configured: bool,
    /// `ok`, `setup_required`, or null when the channel is off. Never shown as is.
    pub status: Option<String>,
    pub mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Features {
    pub voice: bool,
    pub voice_notes: bool,
    pub meetings: bool,
    pub computer_use: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Coexistence {
    /// `clear`, `external_change` or `config_unreadable`.
    pub config_state: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Readiness {
    pub status: String,
    pub failures: Vec<ReadinessFailure>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReadinessFailure {
    pub component: String,
    pub gating: bool,
    pub pane: Option<String>,
    pub detail_key: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RestartState {
    pub required: bool,
    pub reasons: Vec<RestartReason>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RestartReason {
    pub section: Option<String>,
    pub sentence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProviderRow {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JobView {
    pub job_id: String,
    pub kind: String,
    pub status: JobStatus,
    pub phase: Option<String>,
    pub budget_ms: u64,
    pub result: Option<Map<String, Value>>,
    pub failure: Option<JobFailure>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JobFailure {
    pub code: String,
    pub sentence: String,
}

/// `auth.start`: the job, plus the authorize url the daemon hands out exactly once.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AuthStart {
    #[serde(flatten)]
    pub job: JobView,
    pub authorize_url: Option<String>,
    pub expires_in_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct JobList {
    pub jobs: Vec<JobView>,
}

/// Answers that carry only the restart state (`auth.logout`, `providers.set_primary`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RestartOnly {
    pub restart: RestartState,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SecretSetResult {
    pub id: String,
    pub present: bool,
    pub restart: RestartState,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DetectResult {
    pub results: Vec<DetectRow>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DetectRow {
    pub target: String,
    pub present: bool,
    pub detail: Option<String>,
    /// Only the notetaker answers this; null or absent means unanswered, not signed out.
    #[serde(default)]
    pub signed_in: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Hello {
    pub protocol: ProtocolRange,
    pub capabilities: Capabilities,
    pub engine: EngineIdentity,
}

/// The management protocol versions this daemon serves.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ProtocolRange {
    pub current_version: u64,
    pub minimum_version: u64,
    pub maximum_version: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Capabilities {
    pub methods: Vec<String>,
    /// The lowest protocol version each method answers at.
    pub minimum_versions: HashMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EngineIdentity {
    pub product_version: String,
    pub pid: Option<String>,
    /// `x86_64` or `aarch64`; older daemons may not say.
    #[serde(default)]
    pub architecture: Option<String>,
    /// How this engine was installed; the app manages only `linux_package`.
    #[serde(default)]
    pub distribution_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Lease {
    pub lease_id: String,
}

/// A one-use link to the daemon's own setup page, for panes this app has not built.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SetupSession {
    pub url: String,
}
