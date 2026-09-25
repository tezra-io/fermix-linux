//! Meetings and Computer: the notetaker's sign-in state and the computer-use
//! helper's probe. Both are the daemon's answers, shown as they are: an
//! unanswered question is never read as "no" (M38 §5.7, §8.6).

use crate::management::{CallError, Management};
use crate::model::{DetectRow, JobView};
use serde::Deserialize;
use serde_json::json;

pub const MEETBOT: &str = "meetbot";
pub const COMPUTER_SIDECAR: &str = "computer_use_sidecar";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ComputerPermissions {
    pub installed: bool,
    pub screen_capture: bool,
    pub input_control: bool,
    /// When the helper last probed; null until it has.
    pub probed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeetbotState {
    /// The notetaker is not installed.
    Absent,
    SignedIn(String),
    /// The detail, where the daemon gave one.
    SignedOut(Option<String>),
    /// No answer: not detected yet, or the probe did not say.
    Unanswered,
}

pub fn meetbot_state(row: Option<&DetectRow>) -> MeetbotState {
    let Some(row) = row else {
        return MeetbotState::Unanswered;
    };
    match (row.present, row.signed_in) {
        (false, _) => MeetbotState::Absent,
        (true, Some(true)) => MeetbotState::SignedIn(row.detail.clone().unwrap_or_default()),
        (true, Some(false)) => MeetbotState::SignedOut(row.detail.clone()),
        (true, None) => MeetbotState::Unanswered,
    }
}

/// One probed right in words. Before the first probe there is no verdict.
pub fn verdict(granted: bool, probed_at: Option<&str>) -> &'static str {
    match (probed_at, granted) {
        (None, _) => "Not checked yet",
        (Some(_), true) => "Available",
        (Some(_), false) => "Not available",
    }
}

/// A capability job's phase as the pane shows it. An unknown phase is still work.
pub fn phase_words(phase: Option<&str>) -> &'static str {
    match phase {
        Some("sidecar_downloading") => "Downloading the helper…",
        Some("downloading") => "Downloading the notetaker's browser…",
        Some("awaiting_signin") => "Finish signing in to Google in the window that opened.",
        _ => "Working…",
    }
}

impl Management {
    /// Installs a capability's helper as a job; idempotent when it is already installed.
    pub fn capability_install(&self, target: &str) -> Result<JobView, CallError> {
        assert!(!target.is_empty(), "an install names its target");
        let answer = self.call("capabilities.install.start", json!({ "target": target }))?;
        decoded("capabilities.install.start", answer)
    }

    pub fn meetings_sign_in(&self) -> Result<JobView, CallError> {
        let answer = self.call("meetings.signin.start", json!({}))?;
        decoded("meetings.signin.start", answer)
    }

    /// Reads the helper's last probe. Never prompts.
    pub fn computer_permissions(&self) -> Result<ComputerPermissions, CallError> {
        let answer = self.call("computer_use.permissions.get", json!({}))?;
        decoded("computer_use.permissions.get", answer)
    }
}

fn decoded<T: serde::de::DeserializeOwned>(
    method: &str,
    answer: serde_json::Value,
) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}
