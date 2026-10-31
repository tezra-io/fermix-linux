//! The home on the wire (M75): `home.status`, the one picture of this home every surface draws,
//! with the Always On summary the engine works out once (`summary`), recovery snapshots,
//! enrollment, the candidates and the move; and the calls the Always On pane makes of them.
//! Decoding and the calls only; what a screen shows of it is `always_on.rs`'s.
//!
//! After a move this computer's engine runs in the device role and keeps its socket, so nothing
//! here changes transport: `home.status` then names the server this computer reaches and carries
//! that server's own picture (`primary_home`), and the enrollment and the move are forwarded to
//! it, which is how a desktop replaces the server.

use crate::management::{CallError, Management};
use crate::provision::Location;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;

pub const PASSPHRASE_MISMATCH: &str = "The two passphrases are not the same.";

/// `home.status.role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Primary,
    /// A computer whose home moved, linked to the server it reaches.
    Device,
    /// A fresh engine that enrolled and waits for a home to move to it.
    Candidate,
    #[serde(other)]
    Unknown,
}

/// `home.status`. `summary`, `disk` and `version` came after the method: an engine that sends no
/// `summary` is one that needs updating.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HomeStatus {
    pub role: Role,
    pub epoch: Option<u64>,
    pub moved: Option<Moved>,
    /// In the device role after a move, the server this computer reaches.
    pub primary: Option<Primary>,
    #[serde(default)]
    pub desktops: Vec<Desktop>,
    pub snapshots: Snapshots,
    /// On a primary, its enrollment window, never its code.
    pub enrollment: Option<Enrollment>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(rename = "move")]
    pub moving: Option<MoveState>,
    #[serde(default)]
    pub secrets_missing: Vec<String>,
    /// On a linked desktop, the server's own `home.status`.
    #[serde(default)]
    pub primary_home: Option<Box<HomeStatus>>,
    /// On a linked desktop, the daemon's sentence for why `primary_home` is missing.
    #[serde(default)]
    pub primary_missing: Option<String>,
    #[serde(default)]
    pub summary: Option<Summary>,
    #[serde(default)]
    pub disk: Option<Disk>,
    #[serde(default)]
    pub version: Option<String>,
}

impl HomeStatus {
    /// Where the enrollment and the candidates live: the server's own picture on a linked
    /// desktop, this home anywhere else.
    pub fn home_of_record(&self) -> Option<&HomeStatus> {
        match self.role {
            Role::Device => self.primary_home.as_deref(),
            _ => Some(self),
        }
    }
}

/// The marker that retired this copy of the home.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Moved {
    pub to: String,
    pub moved_at: String,
    pub epoch: u64,
}

/// The server a desktop reaches.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Primary {
    pub name: Option<String>,
    pub connected: bool,
    #[serde(default)]
    pub status: LinkStatus,
    pub since: Option<String>,
    pub epoch: Option<u64>,
    pub sentence: String,
}

/// The link to the server in one closed word.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Connected,
    Renewing,
    Unreachable,
    Revoked,
    Stale,
    #[default]
    #[serde(other)]
    Unknown,
}

/// A desktop of the primary.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Desktop {
    pub device_id: String,
    pub name: String,
    pub connected: bool,
    pub since: Option<String>,
}

/// `home.snapshots.status`, and the `snapshots` place of `home.status`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Snapshots {
    pub configured: bool,
    pub recipient_set: bool,
    pub interval_hours: u64,
    pub running: bool,
    pub next_run_at: Option<String>,
    pub targets: Vec<Target>,
    pub sentence: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Target {
    pub id: String,
    pub label: String,
    pub configured: bool,
    pub destination: Option<String>,
    pub last_attempt_at: Option<String>,
    pub last_result: Option<String>,
    pub last_verified: Option<Point>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub waiting: Option<String>,
}

/// The newest snapshot a target proved it holds.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Point {
    pub at: String,
    pub name: Option<String>,
    pub epoch: Option<u64>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub location: Option<String>,
    #[serde(default)]
    pub key_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentState {
    Open,
    Used,
    Expired,
    #[serde(other)]
    Unknown,
}

/// Which code a window is: one a person copies, or the one inside a server Fermix created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentKind {
    Manual,
    Provisioned,
    #[serde(other)]
    Unknown,
}

/// The primary's enrollment window. `kind` and `candidate_id` came after the window; an engine
/// from before them names neither.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Enrollment {
    pub state: EnrollmentState,
    pub expires_at: String,
    #[serde(default)]
    pub kind: Option<EnrollmentKind>,
    /// The candidate that enrolled with this window: how the server Fermix created is known.
    #[serde(default)]
    pub candidate_id: Option<String>,
}

/// A server that enrolled and waits for its move, never its keys.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Candidate {
    pub candidate_id: String,
    pub name: String,
    pub os: String,
    pub arch: String,
    pub version: String,
    pub enrolled_at: String,
    pub expires_at: String,
}

/// The move under way, or how it ended: its phase and the daemon's sentence for it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MoveState {
    pub phase: String,
    pub candidate: Option<String>,
    pub sentence: String,
}

// ---- The summary -----------------------------------------------------------------------------

/// The screen a summary maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryState {
    Off,
    Connect,
    Choose,
    Creating,
    Ready,
    Moving,
    On,
    Undone,
    Retired,
    Candidate,
    Primary,
    Problem,
    #[serde(other)]
    Unknown,
}

/// What one summary action does (PROTOCOL.md, *The Always On summary*, says which calls).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionId {
    SetUp,
    CreateServer,
    Cancel,
    MoveNow,
    TryAgain,
    LearnMore,
    TurnOn,
    RestartServer,
    StartAgain,
    CreateIn,
    OpenHetzner,
    PasteToken,
    Update,
    BackUpNow,
    ReplaceServer,
    CheckAgain,
    OwnServer,
    DeleteServer,
    /// An action a newer engine offers that this app has no button for.
    #[serde(other)]
    Unknown,
}

/// What went wrong, on a `problem` summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    TurnedOff,
    CannotReach,
    Deleted,
    NotSetUp,
    WaitedTooLong,
    NoCapacity,
    AccountLimit,
    ReadOnlyToken,
    TokenRefused,
    CreateFailed,
    Update,
    DiskFull,
    BackupOld,
    NotReady,
    Unreadable,
    DeleteFailed,
    #[serde(other)]
    Unknown,
}

/// One action a summary offers: its id and the engine's label for it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
}

/// `off`'s token step, which its Set up opens.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Connect {
    pub sentence: String,
    pub steps: Vec<String>,
    pub alternatives: Vec<Action>,
}

/// The server a summary names. Hetzner's facts are null for a server of the owner's own.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SummaryServer {
    pub name: Option<String>,
    #[serde(default, rename = "type")]
    pub size: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub ipv4: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

/// The newest verified backup in one place.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Backup {
    pub place: String,
    pub at: String,
}

/// What Details holds for the state, and nothing else. Every field is absent where the state
/// has none.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Detail {
    pub connect: Option<Connect>,
    pub steps: Option<Vec<String>>,
    pub alternatives: Option<Vec<Action>>,
    pub actions: Option<Vec<Action>>,
    pub billing: Option<String>,
    pub server: Option<SummaryServer>,
    pub readiness: Option<Vec<ReadinessItem>>,
    pub waits_until: Option<String>,
    pub backups: Option<Vec<Backup>>,
    pub copies_bytes: Option<u64>,
    pub disk_used: Option<f64>,
    pub data: Option<String>,
    pub kind: Option<ProblemKind>,
    pub note: Option<String>,
    pub seen_at: Option<String>,
    pub next_location: Option<Location>,
}

/// `home.status.summary`: where Always On stands, in one sentence and at most one action.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Summary {
    pub state: SummaryState,
    pub sentence: String,
    pub action: Option<Action>,
    pub detail: Detail,
}

impl Summary {
    /// The problem's kind, on a `problem` summary.
    pub fn problem(&self) -> Option<ProblemKind> {
        (self.state == SummaryState::Problem)
            .then_some(self.detail.kind)
            .flatten()
    }
}

/// On the primary, how full its disk is.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Disk {
    pub used: f64,
    pub nearly_full: bool,
}

// ---- The review and the move ----------------------------------------------------------------

/// The id of one fact the review checks. One this app does not know keeps its word, and its
/// sentence is shown as it is.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(from = "String")]
pub enum ReadinessId {
    KeychainSecrets,
    EngineVersion,
    Disk,
    S3Size,
    Plugins,
    StaysHere,
    Whatsapp,
    Slack,
    CodingAgents,
    CutoverGap,
    ServesNothing,
    Other(String),
}

impl From<String> for ReadinessId {
    fn from(word: String) -> ReadinessId {
        match word.as_str() {
            "keychain_secrets" => ReadinessId::KeychainSecrets,
            "engine_version" => ReadinessId::EngineVersion,
            "disk" => ReadinessId::Disk,
            "s3_size" => ReadinessId::S3Size,
            "plugins" => ReadinessId::Plugins,
            "stays_here" => ReadinessId::StaysHere,
            "whatsapp" => ReadinessId::Whatsapp,
            "slack" => ReadinessId::Slack,
            "coding_agents" => ReadinessId::CodingAgents,
            "cutover_gap" => ReadinessId::CutoverGap,
            "serves_nothing" => ReadinessId::ServesNothing,
            _ => ReadinessId::Other(word),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Ok,
    Warn,
    Block,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReadinessItem {
    pub id: ReadinessId,
    pub status: ItemStatus,
    pub sentence: String,
}

/// `home.move.readiness`: what the owner reviews before the move.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Readiness {
    /// False while any item blocks; the daemon refuses the move until it is true.
    pub ready: bool,
    pub items: Vec<ReadinessItem>,
}

/// What becomes of the home a move leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFrom {
    /// This computer's home moves, and this computer stays a desktop of the server (the
    /// engine's default, so the request leaves `keep_as_desktop` out).
    ThisComputer,
    /// The server is replaced: it keeps no desktop of its own (`keep_as_desktop: false`).
    TheServer,
}

/// `home.move.start`'s answer: the move was written, and the engine holding the home restarts
/// into it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MoveStarted {
    pub move_id: String,
    pub candidate: Candidate,
    pub keep_as_desktop: bool,
    pub sentence: String,
}

/// `home.snapshots.run`'s answer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SnapshotRun {
    pub started: bool,
    pub snapshots: Snapshots,
}

/// `home.snapshots.passphrase`'s answer: the recovery key now set.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PassphraseSet {
    pub key_id: String,
    pub snapshots: Snapshots,
    pub sentence: String,
}

/// `home.enroll.open`'s answer. The code is answered once and never in a status, so no print of
/// this type spells it.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Code {
    code: String,
    pub expires_at: String,
}

impl Code {
    /// The code as the daemon answered it, for the command and the clipboard and nothing else.
    pub fn text(&self) -> &str {
        &self.code
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Code")
            .field("code", &"withheld")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// The recovery passphrase, held only between the two entries and the call that carries it. No
/// print of it spells it.
#[derive(Clone, PartialEq, Eq)]
pub struct Passphrase(String);

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(withheld)")
    }
}

/// The passphrase the two entries agree on. `Err(None)` while either is empty, which says
/// nothing yet; `Err(Some(sentence))` once both have words and they differ. How long it must be
/// is the daemon's rule, and its refusal says so.
pub fn passphrase(first: &str, second: &str) -> Result<Passphrase, Option<&'static str>> {
    if first.is_empty() || second.is_empty() {
        return Err(None);
    }
    if first != second {
        return Err(Some(PASSPHRASE_MISMATCH));
    }
    Ok(Passphrase(first.to_owned()))
}

/// The sentence of a `home.move.start` the server had but could not confirm (`unconfirmed`): no
/// failure, since the move may have started. None for any other answer.
pub fn unconfirmed(error: &CallError) -> Option<&str> {
    match error {
        CallError::Refused(refusal) if refusal.code == "unconfirmed" => Some(&refusal.sentence),
        _ => None,
    }
}

impl Management {
    pub fn home_status(&self) -> Result<HomeStatus, CallError> {
        decoded("home.status", self.call("home.status", json!({}))?)
    }

    /// Opens the one enrollment window for a server of the owner's own. Its answer is never
    /// quoted in an error, since it holds the code.
    pub fn home_enroll_open(&self) -> Result<Code, CallError> {
        let answer = self.call("home.enroll.open", json!({}))?;
        serde_json::from_value(answer).map_err(|_| {
            CallError::Protocol("home.enroll.open answered an unexpected shape".into())
        })
    }

    /// What the move to `candidate_id` needs and what it leaves behind, before it starts.
    pub fn home_move_readiness(&self, candidate_id: &str) -> Result<Readiness, CallError> {
        assert!(
            !candidate_id.is_empty(),
            "home.move.readiness needs a candidate"
        );
        let params = json!({ "candidate_id": candidate_id });
        decoded(
            "home.move.readiness",
            self.call("home.move.readiness", params)?,
        )
    }

    /// Starts the move to `candidate_id`. The engine that holds the home writes the move and
    /// restarts into it.
    pub fn home_move_start(
        &self,
        candidate_id: &str,
        passphrase: &Passphrase,
        from: MoveFrom,
    ) -> Result<MoveStarted, CallError> {
        assert!(
            !candidate_id.is_empty(),
            "home.move.start needs a candidate"
        );
        let mut params =
            json!({ "candidate_id": candidate_id, "recovery_passphrase": passphrase.0 });
        if from == MoveFrom::TheServer {
            params["keep_as_desktop"] = json!(false);
        }
        decoded("home.move.start", self.call("home.move.start", params)?)
    }

    pub fn home_snapshots_run(&self) -> Result<SnapshotRun, CallError> {
        decoded(
            "home.snapshots.run",
            self.call("home.snapshots.run", json!({}))?,
        )
    }

    /// Derives the recovery key from the passphrase; the daemon keeps only its public half.
    pub fn home_snapshots_passphrase(
        &self,
        passphrase: &Passphrase,
    ) -> Result<PassphraseSet, CallError> {
        let params = json!({ "recovery_passphrase": passphrase.0 });
        decoded(
            "home.snapshots.passphrase",
            self.call("home.snapshots.passphrase", params)?,
        )
    }
}

fn decoded<T: DeserializeOwned>(method: &str, answer: Value) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}
