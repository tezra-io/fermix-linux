//! Doctor: the daemon's health checks, run as a session the app starts and polls.
//! The daemon owns every status and sentence; this module only names the rows,
//! orders them, and says which remediations this app can act on. Sessions decode
//! tolerantly, so a newer daemon's statuses, kinds and fields never break it.

use crate::job;
use crate::management::{CallError, Management};
use crate::settings;
use serde::Deserialize;
use serde_json::{json, Value};

/// macOS polls a run every half second.
pub const POLL_MS: u64 = 500;
/// 40 s of polling: past the 30 s network budget, and never longer.
pub const MAX_POLLS: u64 = 80;
pub const STALLED: &str = "Fermix stopped reporting on this run.";

/// The one `instructions` target this app shows steps for. The other one the
/// engine publishes, `external_config_change.recovery`, is the Recovery screen,
/// which the Linux app does not have yet.
const SERVICE_REMOVAL: &str = "legacy_service_unit.removal";

/// What a service remediation asks for on Linux: the package's own command
/// adopts or migrates another Fermix service unit (M38 §4.5, §5.8).
pub const SERVICE_STEPS_BODY: &str =
    "Run this in a terminal. It adopts the other service, or moves it over to this one.";
pub const SERVICE_STEPS_COMMAND: &str = "fermix service install";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Local,
    /// Probes providers and channels for real; one probe is a metered call.
    Network,
}

impl Scope {
    pub fn wire(self) -> &'static str {
        match self {
            Scope::Local => "local",
            Scope::Network => "network",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Running,
    Completed,
    Cancelled,
    TimedOut,
    Failed,
    #[serde(other)]
    Unknown,
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
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    Warning,
    Info,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    SettingsPane,
    SystemSettings,
    Job,
    Restart,
    Reload,
    Instructions,
    None,
    #[serde(other)]
    Other,
}

/// The one view `doctor.start`, `doctor.get` and `doctor.cancel` all answer with.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Session {
    pub session_id: String,
    pub scope: String,
    pub status: SessionStatus,
    pub budget_ms: u64,
    pub total: u32,
    pub completed_count: u32,
    pub summary: Summary,
    /// While running, only the rows that have landed; every catalogued row once it ends.
    #[serde(default)]
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub passed: u32,
    pub warning: u32,
    pub failed: u32,
    pub not_applicable: u32,
    pub unavailable: u32,
    pub skipped: u32,
    pub cancelled: u32,
    pub timed_out: u32,
}

/// One check. There is no name on the wire: the title comes from `id`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Check {
    pub id: String,
    pub severity: Severity,
    pub status: CheckStatus,
    /// The daemon's sentence, scrubbed and at most 256 bytes.
    pub summary: String,
    /// Absent even where a remediation code is set: that row has no action.
    #[serde(default)]
    pub remediation: Option<Remediation>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Remediation {
    pub title: String,
    pub body: String,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Action {
    pub kind: ActionKind,
    #[serde(default)]
    pub target: Option<String>,
}

impl Management {
    pub fn doctor_start(&self, scope: Scope) -> Result<Session, CallError> {
        let view = self.call("doctor.start", json!({ "scope": scope.wire() }))?;
        session("doctor.start", view)
    }

    pub fn doctor_get(&self, session_id: &str) -> Result<Session, CallError> {
        assert!(!session_id.is_empty(), "doctor.get needs a session id");
        let view = self.call("doctor.get", json!({ "session_id": session_id }))?;
        session("doctor.get", view)
    }

    /// Stops a running session; on one that already ended it answers that ending.
    pub fn doctor_cancel(&self, session_id: &str) -> Result<Session, CallError> {
        assert!(!session_id.is_empty(), "doctor.cancel needs a session id");
        let view = self.call("doctor.cancel", json!({ "session_id": session_id }))?;
        session("doctor.cancel", view)
    }
}

fn session(method: &str, view: Value) -> Result<Session, CallError> {
    serde_json::from_value(view)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}

/// The most polls one session may take: its budget, a margin for the daemon to
/// record the ending, and never more than `MAX_POLLS`.
pub fn poll_cap(budget_ms: u64) -> u64 {
    job::poll_cap(budget_ms, POLL_MS).min(MAX_POLLS)
}

/// `auth_token_expiry` reads "Auth token expiry": underscores opened, first letter raised.
pub fn check_title(id: &str) -> String {
    let words = id.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Attention,
    Passed,
    NotRun,
}

impl Group {
    pub fn title(self) -> &'static str {
        match self {
            Group::Attention => "Needs attention",
            Group::Passed => "Passed",
            Group::NotRun => "Not run",
        }
    }
}

/// How a status reads beside the row's title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Good,
    Warn,
    Bad,
    Quiet,
}

impl CheckStatus {
    pub fn verdict(self) -> &'static str {
        match self {
            CheckStatus::Passed => "Passed",
            CheckStatus::Warning => "Warning",
            CheckStatus::Failed => "Failed",
            CheckStatus::NotApplicable => "Not applicable",
            CheckStatus::Unavailable => "Unavailable",
            CheckStatus::Skipped => "Skipped",
            CheckStatus::Cancelled => "Cancelled",
            CheckStatus::TimedOut => "Timed out",
            CheckStatus::Unknown => "Unknown",
        }
    }

    pub fn tone(self) -> Tone {
        match self {
            CheckStatus::Passed => Tone::Good,
            CheckStatus::Warning | CheckStatus::Unknown => Tone::Warn,
            CheckStatus::Failed | CheckStatus::Unavailable => Tone::Bad,
            CheckStatus::NotApplicable
            | CheckStatus::Skipped
            | CheckStatus::Cancelled
            | CheckStatus::TimedOut => Tone::Quiet,
        }
    }

    /// A status this app has never seen is not vouched for, so it asks for a look.
    pub fn group(self) -> Group {
        match self.tone() {
            Tone::Good => Group::Passed,
            Tone::Warn | Tone::Bad => Group::Attention,
            Tone::Quiet => Group::NotRun,
        }
    }

    /// Worst first within "Needs attention"; every other group keeps catalog order.
    fn rank(self) -> u8 {
        match self {
            CheckStatus::Failed => 0,
            CheckStatus::Unavailable => 1,
            CheckStatus::Warning => 2,
            CheckStatus::Unknown => 3,
            _ => 4,
        }
    }
}

impl Severity {
    fn rank(self) -> u8 {
        match self {
            Severity::Critical => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
            Severity::Unknown => 3,
        }
    }
}

/// The rows in groups, problems first and the worst of them first, then passed,
/// then not run. Ties keep the daemon's catalog order; empty groups are left out.
pub fn grouped(checks: &[Check]) -> Vec<(Group, Vec<&Check>)> {
    let mut ordered: Vec<&Check> = checks.iter().collect();
    ordered.sort_by_key(|c| {
        let attention = c.status.group() == Group::Attention;
        let severity = if attention { c.severity.rank() } else { 0 };
        (c.status.group(), c.status.rank(), severity)
    });
    let mut groups: Vec<(Group, Vec<&Check>)> = Vec::new();
    for check in ordered {
        match groups.last_mut() {
            Some((group, rows)) if *group == check.status.group() => rows.push(check),
            _ => groups.push((check.status.group(), vec![check])),
        }
    }
    groups
}

/// The banner, from the daemon's counts and never re-derived from the rows.
pub fn summary_sentence(summary: &Summary) -> String {
    match (summary.failed, summary.warning) {
        (1, _) => "One check failed".into(),
        (n, _) if n > 1 => format!("{n} checks failed"),
        (_, 1) => "Healthy, with one thing to look at".into(),
        (_, n) if n > 1 => format!("Healthy, with {n} things to look at"),
        _ => "Everything checks out".into(),
    }
}

pub fn headline(session: &Session) -> String {
    match session.status {
        SessionStatus::Running => "Running checks".into(),
        SessionStatus::Cancelled => "Checks cancelled".into(),
        SessionStatus::TimedOut => "The checks ran out of time".into(),
        SessionStatus::Failed => "The checks stopped early".into(),
        SessionStatus::Completed | SessionStatus::Unknown => summary_sentence(&session.summary),
    }
}

/// The line under the headline: progress while running, where the answers came
/// from once complete, and what a cut-short run found before it stopped.
pub fn detail(session: &Session) -> String {
    let problems = session.summary.failed + session.summary.warning;
    match session.status {
        SessionStatus::Running => format!(
            "{} of {} checks done",
            session.completed_count, session.total
        ),
        SessionStatus::Completed | SessionStatus::Unknown => {
            "Answers come from the running daemon, not from this app.".into()
        }
        _ if problems > 0 => summary_sentence(&session.summary),
        _ => "Nothing that ran needs attention.".into(),
    }
}

/// `(done, total)`, and only while running: at the end every row is filled.
pub fn progress(session: &Session) -> Option<(u32, u32)> {
    (session.status == SessionStatus::Running).then_some((session.completed_count, session.total))
}

/// What a row's one button does. Only what this app can carry out is minted;
/// every other remediation shows its title and body with no button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// A Settings pane this app has, by its slug.
    OpenPane {
        slug: &'static str,
        title: &'static str,
    },
    /// The one Restart confirmation.
    Restart,
    /// `settings.reload`, then the checks again so the row redraws.
    Reload,
    /// The terminal command that adopts or replaces another Fermix service.
    ServiceSteps,
}

impl Fix {
    pub fn label(&self) -> String {
        match self {
            Fix::OpenPane { title, .. } => format!("Open {title}"),
            Fix::Restart => "Restart Fermix…".into(),
            Fix::Reload => "Reload settings".into(),
            Fix::ServiceSteps => "Show how".into(),
        }
    }
}

impl Check {
    /// A target this app cannot place gets no button, never a neighbouring one.
    /// No system settings deep link is portable on Linux, and a bare job target
    /// cannot start a job.
    pub fn fix(&self) -> Option<Fix> {
        let action = &self.remediation.as_ref()?.action;
        let target = action.target.as_deref();
        match action.kind {
            ActionKind::SettingsPane => {
                let pane = settings::pane(target?)?;
                Some(Fix::OpenPane {
                    slug: pane.slug,
                    title: pane.title,
                })
            }
            ActionKind::Restart => Some(Fix::Restart),
            ActionKind::Reload => Some(Fix::Reload),
            ActionKind::Instructions if target == Some(SERVICE_REMOVAL) => Some(Fix::ServiceSteps),
            ActionKind::Instructions
            | ActionKind::SystemSettings
            | ActionKind::Job
            | ActionKind::None
            | ActionKind::Other => None,
        }
    }
}
