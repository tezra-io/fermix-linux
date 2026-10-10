//! The phone channel on the wire: `mobile.status`, the pairing window the Phone dialog polls, and
//! the paired phones (PROTOCOL.md). A pairing window is polled rather than run as a job: its view
//! grows while it runs, and the owner decides partway through.

use crate::management::{CallError, Management};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;

/// A pairing window's `state`, which the dialog switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    AwaitingScan,
    AwaitingDecision,
    Approved,
    Denied,
    Expired,
    Cancelled,
    Failed,
    #[serde(other)]
    Unknown,
}

impl SessionState {
    /// Whether the window is over, which is when polling stops. A state this app does not know
    /// may still be open.
    pub fn is_terminal(self) -> bool {
        match self {
            SessionState::AwaitingScan | SessionState::AwaitingDecision => false,
            SessionState::Unknown => false,
            SessionState::Approved
            | SessionState::Denied
            | SessionState::Expired
            | SessionState::Cancelled
            | SessionState::Failed => true,
        }
    }
}

/// Why a finished window did not pair: `outcome.reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeReason {
    Denied,
    Timeout,
    Cancelled,
    #[serde(other)]
    Unknown,
}

/// `listener.status`: whether a phone can reach this computer at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerStatus {
    Ready,
    Down,
    Unavailable,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Listener {
    pub status: ListenerStatus,
}

/// `mobile.status`: the channel as it stands, answered with the channel off too.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MobileStatus {
    pub enabled: bool,
    /// False until the channel runs, which a switch turned on does not do before a restart.
    pub started: bool,
    /// True when the channel could not start this boot.
    pub refused: bool,
    pub listener: Listener,
    pub paired_devices: i64,
    /// The open pairing window, else the newest one the daemon keeps, else none.
    pub pairing: Option<PairingSummary>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PairingSummary {
    pub session_id: String,
    pub state: SessionState,
}

/// A pairing window as `mobile.pair.get`, `.decide` and `.cancel` answer it. `ttl_ms` is relative
/// and absent once the window is over; a start refused for a reason the owner can act on answers
/// `failed` with no `session_id`, since nothing was opened.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PairingSession {
    pub session_id: Option<String>,
    pub state: SessionState,
    pub ttl_ms: Option<i64>,
    pub request: Option<PairingRequest>,
    pub outcome: Option<PairingOutcome>,
    pub failure: Option<PairingFailure>,
}

/// The phone waiting for a decision. `sas` is the six digits the owner compares with the phone.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PairingRequest {
    pub device_name: String,
    pub model: String,
    pub sas: String,
    pub attestation: Attestation,
}

/// What the daemon verified of the phone's secure hardware, in its own words.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Attestation {
    pub sentence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PairingOutcome {
    pub device_id: Option<String>,
    pub reason: Option<OutcomeReason>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PairingFailure {
    pub code: String,
    pub sentence: String,
}

/// `mobile.pair.start`'s answer: the window, and the pairing link the daemon hands back once.
/// The link carries the phone's one-time secret, so no print of this type spells it.
#[derive(Clone, PartialEq, Deserialize)]
pub struct PairingStart {
    #[serde(flatten)]
    pub session: PairingSession,
    uri: Option<String>,
}

impl PairingStart {
    pub fn has_uri(&self) -> bool {
        self.uri.is_some()
    }

    /// The link as the daemon sent it, for the guards to read and nothing else.
    pub fn uri(&self) -> Option<&str> {
        self.uri.as_deref()
    }
}

impl fmt::Debug for PairingStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingStart")
            .field("session", &self.session)
            .field("uri", &self.uri.as_ref().map(|_| "withheld"))
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MobileDevice {
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub last_seen: Option<String>,
}

/// `mobile.devices.list`: every paired phone, oldest first. Empty while the channel is not running.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MobileDevices {
    pub devices: Vec<MobileDevice>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Revoked {
    pub device_id: String,
    pub revoked: bool,
}

impl Management {
    pub fn mobile_status(&self) -> Result<MobileStatus, CallError> {
        decoded("mobile.status", self.call("mobile.status", json!({}))?)
    }

    /// Opens a pairing window. Its answer is never quoted in an error, since it holds the link.
    pub fn mobile_pair_start(&self) -> Result<PairingStart, CallError> {
        let answer = self.call("mobile.pair.start", json!({}))?;
        serde_json::from_value(answer).map_err(|_| {
            CallError::Protocol("mobile.pair.start answered an unexpected shape".into())
        })
    }

    pub fn mobile_pair_get(&self, session_id: &str) -> Result<PairingSession, CallError> {
        assert!(!session_id.is_empty(), "mobile.pair.get needs a session id");
        let params = json!({ "session_id": session_id });
        decoded("mobile.pair.get", self.call("mobile.pair.get", params)?)
    }

    pub fn mobile_pair_decide(
        &self,
        session_id: &str,
        approved: bool,
    ) -> Result<PairingSession, CallError> {
        assert!(
            !session_id.is_empty(),
            "mobile.pair.decide needs a session id"
        );
        let params = json!({ "session_id": session_id, "approved": approved });
        decoded(
            "mobile.pair.decide",
            self.call("mobile.pair.decide", params)?,
        )
    }

    /// Closes a window; on one that already ended it answers that ending.
    pub fn mobile_pair_cancel(&self, session_id: &str) -> Result<PairingSession, CallError> {
        assert!(
            !session_id.is_empty(),
            "mobile.pair.cancel needs a session id"
        );
        let params = json!({ "session_id": session_id });
        decoded(
            "mobile.pair.cancel",
            self.call("mobile.pair.cancel", params)?,
        )
    }

    pub fn mobile_devices_list(&self) -> Result<MobileDevices, CallError> {
        decoded(
            "mobile.devices.list",
            self.call("mobile.devices.list", json!({}))?,
        )
    }

    pub fn mobile_devices_revoke(&self, device_id: &str) -> Result<Revoked, CallError> {
        assert!(
            !device_id.is_empty(),
            "mobile.devices.revoke needs a device id"
        );
        let params = json!({ "device_id": device_id });
        decoded(
            "mobile.devices.revoke",
            self.call("mobile.devices.revoke", params)?,
        )
    }
}

fn decoded<T: DeserializeOwned>(method: &str, answer: Value) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}
