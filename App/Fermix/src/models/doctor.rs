//! Doctor, as data.
//!
//! One session at a time: started on entering the surface at local scope,
//! polled until it is terminal, cancelled on leaving. The eight statuses, the
//! summaries, the remediations and the evidence are all the daemon's; what this
//! file owns is the polling, the bound on it, and the reading of a check's name
//! out of the evidence the daemon published beside it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use crate::copy::Key;
use crate::management::types::{
    CheckStatus, DoctorScope, DoctorSession, DoctorSessionParams, DoctorSessionStatus,
    DoctorStartParams, Remediation,
};

use super::api::{accept, ask, READ_DEADLINE};
use super::settings_model::{Sentence, SettingsModel};
use super::{spawn, Observers, Poller};

/// How often a running session is re-read.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// The hard ceiling on polls, whatever the daemon's budget says.
pub const POLL_CAP: u32 = 2_000;
/// The polls added on top of the daemon's own budget, so a run that finishes at
/// its deadline is still read once after it does.
const POLL_SLACK: u32 = 20;

/// What a check is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckName {
    /// The daemon's own name for the check, from the evidence it published.
    Words(String),
    /// The check's identifier, where the daemon published no name. Shown as an
    /// identifier rather than dressed up as a sentence.
    Identifier(String),
}

/// One check, as a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRow {
    pub id: String,
    pub name: CheckName,
    pub summary: String,
    pub status: CheckStatus,
    pub remediation: Option<Remediation>,
    /// The evidence, flattened to labelled lines.
    pub evidence: Vec<(String, String)>,
}

impl CheckRow {
    /// Whether this row carries something to act on.
    pub fn needs_action(&self) -> bool {
        matches!(
            self.status,
            CheckStatus::Failed | CheckStatus::Warning | CheckStatus::Unavailable
        )
    }
}

/// The eight status words the catalogue renders for the eight wire statuses.
///
/// An atom is not a word, and one has to exist somewhere: the daemon publishes
/// `passed`, and the word "Passed" is the product's rendering of it.
pub fn status_word(status: CheckStatus) -> Key {
    match status {
        CheckStatus::Passed => Key::DoctorStatusPassed,
        CheckStatus::Warning => Key::DoctorStatusWarning,
        CheckStatus::Failed => Key::DoctorStatusFailed,
        CheckStatus::NotApplicable => Key::DoctorStatusNotApplicable,
        CheckStatus::Unavailable => Key::DoctorStatusUnavailable,
        CheckStatus::Skipped => Key::DoctorStatusSkipped,
        CheckStatus::Cancelled => Key::DoctorStatusCancelled,
        CheckStatus::TimedOut => Key::DoctorStatusTimedOut,
        // A status a newer daemon mints has no word here. The pill and the word
        // both say so rather than picking a neighbour's.
        CheckStatus::Unrecognized => Key::HomeRuntimeUnavailable,
    }
}

/// The one-letter pill beside the word, so status is never colour alone.
pub fn status_pill(status: CheckStatus) -> Key {
    match status {
        CheckStatus::Passed => Key::DoctorPillPassed,
        CheckStatus::Warning => Key::DoctorPillWarning,
        CheckStatus::Failed => Key::DoctorPillFailed,
        CheckStatus::NotApplicable => Key::DoctorPillNotApplicable,
        CheckStatus::Unavailable => Key::DoctorPillUnavailable,
        CheckStatus::Skipped => Key::DoctorPillSkipped,
        CheckStatus::Cancelled => Key::DoctorPillCancelled,
        CheckStatus::TimedOut => Key::DoctorPillTimedOut,
        CheckStatus::Unrecognized => Key::DoctorPillUnavailable,
    }
}

/// Doctor's own model.
pub struct DoctorModel {
    settings: Rc<SettingsModel>,
    session: RefCell<Option<DoctorSession>>,
    refusal: RefCell<Option<Sentence>>,
    poller: Poller,
    starting: Cell<bool>,
    observers: Observers,
}

impl DoctorModel {
    /// A Doctor model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        Rc::new(Self {
            settings,
            session: RefCell::new(None),
            refusal: RefCell::new(None),
            poller: Poller::new(),
            starting: Cell::new(false),
            observers: Observers::default(),
        })
    }

    /// Tell me when the session moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// The session as it was last read.
    pub fn session(&self) -> Option<DoctorSession> {
        self.session.borrow().clone()
    }

    /// The daemon's own refusal, where the last start was refused.
    pub fn refusal(&self) -> Option<Sentence> {
        self.refusal.borrow().clone()
    }

    /// Whether a run is in progress.
    pub fn is_running(&self) -> bool {
        self.session
            .borrow()
            .as_ref()
            .map(|session| session.status == DoctorSessionStatus::Running)
            .unwrap_or(false)
            || self.starting.get()
    }

    /// Whether the session is being polled.
    pub fn is_polling(&self) -> bool {
        self.poller.is_running()
    }

    /// The checks, as rows.
    pub fn rows(&self) -> Vec<CheckRow> {
        let session = self.session.borrow();
        let Some(session) = session.as_ref() else {
            return Vec::new();
        };

        session
            .checks
            .iter()
            .map(|check| CheckRow {
                id: check.id.clone(),
                name: name_of(check),
                summary: check.summary.clone(),
                status: check.status,
                remediation: check.remediation.clone(),
                evidence: evidence_lines(&check.evidence),
            })
            .collect()
    }

    /// How many checks failed, for the summary line.
    pub fn failed(&self) -> u32 {
        self.session
            .borrow()
            .as_ref()
            .map(|session| session.summary.failed)
            .unwrap_or(0)
    }

    /// Start a run, unless one is already in flight.
    ///
    /// Local scope on entering the surface; network scope only ever from the
    /// explicit toolbar action, because those checks call the services the
    /// person has configured.
    pub async fn start(&self, scope: DoctorScope) {
        if self.starting.get() {
            return;
        }
        self.starting.set(true);
        self.refusal.replace(None);

        let api = self.settings.api();
        let issued = ask::<_, DoctorSession>(
            api.as_ref(),
            "doctor.start",
            &DoctorStartParams { scope },
            READ_DEADLINE,
        )
        .await;

        self.starting.set(false);

        match accept(api.as_ref(), issued) {
            None => {}
            Some(Ok(session)) => {
                self.session.replace(Some(session));
            }
            Some(Err(error)) => {
                self.refusal.replace(Some(Sentence::of(&error)));
            }
        }

        self.observers.notify();
    }

    /// Start a run and poll it until it is terminal.
    pub fn run(self: &Rc<Self>, scope: DoctorScope) {
        let model = Rc::clone(self);
        spawn(async move {
            model.start(scope).await;
            model.poll_until_terminal();
        });
    }

    /// Poll the running session until it stops running.
    pub fn poll_until_terminal(self: &Rc<Self>) {
        let Some(session) = self.session() else {
            return;
        };
        if session.status != DoctorSessionStatus::Running {
            return;
        }

        let model = Rc::clone(self);
        self.poller
            .start(POLL_INTERVAL, poll_cap(session.budget_ms), move || {
                if !model.is_running() {
                    return false;
                }
                let model = Rc::clone(&model);
                spawn(async move {
                    model.read_once().await;
                });
                true
            });
    }

    /// One `doctor.get`.
    pub async fn read_once(&self) {
        let Some(session_id) = self.session.borrow().as_ref().map(|s| s.session_id.clone()) else {
            return;
        };

        let api = self.settings.api();
        let issued = ask::<_, DoctorSession>(
            api.as_ref(),
            "doctor.get",
            &DoctorSessionParams { session_id },
            READ_DEADLINE,
        )
        .await;

        match accept(api.as_ref(), issued) {
            Some(Ok(session)) => {
                self.session.replace(Some(session));
            }
            Some(Err(error)) => {
                self.refusal.replace(Some(Sentence::of(&error)));
            }
            None => return,
        }

        self.observers.notify();
    }

    /// Stop polling, and ask the daemon to stop the run.
    ///
    /// Leaving the surface cancels: a Doctor run holds a session on the daemon,
    /// and at most two exist at once.
    pub async fn cancel(&self) {
        self.poller.stop();

        let Some(session_id) = self.session.borrow().as_ref().map(|s| s.session_id.clone()) else {
            return;
        };
        if !self.is_running() {
            return;
        }

        let api = self.settings.api();
        let issued = ask::<_, DoctorSession>(
            api.as_ref(),
            "doctor.cancel",
            &DoctorSessionParams { session_id },
            READ_DEADLINE,
        )
        .await;

        if let Some(Ok(session)) = accept(api.as_ref(), issued) {
            self.session.replace(Some(session));
        }

        self.observers.notify();
    }

    /// Stop polling without cancelling the run.
    pub fn stop_polling(&self) {
        self.poller.stop();
    }
}

/// The cap for one run: the daemon's own budget in ticks, plus a little, and
/// never more than the published ceiling.
pub fn poll_cap(budget_ms: u64) -> u32 {
    let ticks = (budget_ms / POLL_INTERVAL.as_millis() as u64) as u32;
    ticks.saturating_add(POLL_SLACK).min(POLL_CAP)
}

/// The daemon's own name for a check.
///
/// The wire carries an identifier, a status and a summary, and the name sits in
/// the evidence the engine publishes beside them. Where it is absent the row
/// leads with the identifier, visibly an identifier, rather than with a name
/// this application made up for it.
fn name_of(check: &crate::management::types::DoctorCheck) -> CheckName {
    match check
        .evidence
        .get("source_name")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        Some(name) => CheckName::Words(name.to_string()),
        None => CheckName::Identifier(check.id.clone()),
    }
}

/// The evidence, as labelled lines. A nested value keeps its own shape rather
/// than being flattened into something that reads like a sentence.
fn evidence_lines(evidence: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    evidence
        .iter()
        .map(|(key, value)| (key.clone(), scalar(value)))
        .collect()
}

fn scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_follows_the_daemons_own_budget_and_never_passes_the_ceiling() {
        assert_eq!(poll_cap(10_000), 40);
        assert_eq!(poll_cap(30_000), 80);
        assert_eq!(poll_cap(u64::MAX), POLL_CAP);
    }

    #[test]
    fn every_published_status_has_a_word_and_a_pill() {
        for status in [
            CheckStatus::Passed,
            CheckStatus::Warning,
            CheckStatus::Failed,
            CheckStatus::NotApplicable,
            CheckStatus::Unavailable,
            CheckStatus::Skipped,
            CheckStatus::Cancelled,
            CheckStatus::TimedOut,
        ] {
            assert!(!crate::copy::text(status_word(status)).is_empty());
            assert_eq!(
                crate::copy::text(status_pill(status)).chars().count(),
                1,
                "a pill is one letter"
            );
        }
    }

    #[test]
    fn a_check_is_named_by_the_daemon_where_the_daemon_named_it() {
        let check: crate::management::types::DoctorCheck =
            serde_json::from_value(serde_json::json!({
                "id": "daemon_socket",
                "category": "runtime",
                "severity": "critical",
                "applicability": "always",
                "origin": "engine",
                "status": "warning",
                "summary": "not running",
                "evidence": {"source_name": "daemon socket", "source_status": "warn"},
                "remediation_code": null,
                "duration_ms": 11,
                "finished_at": "2026-08-19T12:00:00Z"
            }))
            .expect("decodes");

        assert_eq!(name_of(&check), CheckName::Words("daemon socket".into()));
        assert_eq!(
            evidence_lines(&check.evidence),
            vec![
                ("source_name".to_string(), "daemon socket".to_string()),
                ("source_status".to_string(), "warn".to_string()),
            ]
        );
    }

    #[test]
    fn a_check_with_no_published_name_leads_with_its_identifier() {
        let check: crate::management::types::DoctorCheck =
            serde_json::from_value(serde_json::json!({
                "id": "home_permissions",
                "category": "security",
                "severity": "critical",
                "applicability": "always",
                "origin": "engine",
                "status": "passed",
                "summary": "fine",
                "evidence": {},
                "remediation_code": null,
                "duration_ms": 1,
                "finished_at": "2026-08-19T12:00:00Z"
            }))
            .expect("decodes");

        assert_eq!(
            name_of(&check),
            CheckName::Identifier("home_permissions".into())
        );
    }
}
