//! Always On (M75 §14): where Fermix runs, as `home.status` says it, and the steps the Always On
//! pane takes the owner through before a move. `home.status` is the one source the pane draws;
//! nothing here derives a state the daemon publishes. What the app holds itself is only what the
//! daemon never publishes: the destination the owner picked, the enrollment code it answered once,
//! the candidate under review, and the calls still on their way (`Steps`).
//!
//! After a move this computer's engine runs in the device role and keeps both sockets, so the app
//! changes no transport (M75 B1): the pane reads the same `home.status`, which then names the
//! primary this computer reaches and carries that primary's own picture (`primary_home`). The
//! server's recovery, enrollment and move are then the server's: the same calls reach it through
//! this computer's engine, which is how a desktop replaces the server.

use crate::management::{CallError, Management};
use crate::phone::unix_seconds;
use crate::view::DaemonProblem;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fmt;
use std::time::Duration;

/// The settings pane, and the pane the daemon files the `snapshots` section under.
pub const PANE: &str = "always_on";
/// The section that configures this computer itself (M75 §6, device-class), drawn under the
/// server it reaches; the pane's other sections follow recovery.
pub const THIS_COMPUTER: &str = "engine";
/// The guide that starts the AWS stack (M75 §13).
pub const GUIDE_URL: &str = "https://fermix.ai/docs/always-on";
/// What the owner runs on a Linux host they already have; it asks for the code itself.
pub const HOST_COMMAND: &str = "curl -fsSL https://fermix.ai/always-on | sudo sh";
/// How often the pane reads `home.status` while it is on screen.
pub const POLL_EVERY: Duration = Duration::from_secs(3);
/// The most reads one visit to the pane makes: 30 minutes at `POLL_EVERY`. Past it the pane
/// stops and says so (`STOPPED_CHECKING`); coming back to the pane starts a new visit.
pub const POLLS_PER_VISIT: u32 = 600;
/// How long the pane waits for this computer's engine to come back from its restart into a move
/// before it stops spinning, says so, and offers to start it (M75 review, edges F14).
pub const RESTART_GRACE: Duration = Duration::from_secs(120);
/// How long a replacement of the server may go with neither server answering this computer
/// before the pane says so plainly. Sending a large home takes minutes, so it is longer.
pub const REPLACE_GRACE: Duration = Duration::from_secs(15 * 60);

/// What the pane cannot show while Fermix on this computer is not running.
pub const NEEDS_FERMIX: &str = "Always On is read from Fermix on this computer. Start it to see \
    where Fermix runs.";
pub const EXPLAIN_TITLE: &str = "Keep Fermix running when this computer is off";
pub const EXPLAIN: &str = "Always On moves Fermix to a Linux server you own, so it keeps working \
    while this computer sleeps. Chat here, your phone and your channels then reach Fermix there.";
pub const MOVES_TITLE: &str = "What moves";
pub const WHAT_MOVES: &str = "Memory, chat history, schedules, settings, channel connections and \
    the keys Fermix holds, those in your keyring too. On the server Fermix keeps them in files only \
    its own account can read, which the commands it runs cannot open. Paired phones keep working.";
pub const STAYS_TITLE: &str = "What stays here";
pub const WHAT_STAYS: &str = "This computer's browser sign-ins, its logs and the folders you let \
    Fermix use. Voice calls are not available after the move. This computer then runs actions \
    for the server only while you allow it, and you approve every command.";
pub const CHOOSE_TITLE: &str = "Where Fermix runs";
pub const AWS_TITLE: &str = "A new server on AWS";
pub const AWS_DETAIL: &str =
    "Fermix's stack creates it in your own AWS account, which pays for it.";
pub const HOST_TITLE: &str = "A Linux server you already have";
pub const HOST_DETAIL: &str = "Any supported Linux host where you can run a command with sudo.";
pub const CODE_TITLE: &str = "Enrollment code";
pub const GET_CODE: &str = "Get a code";
pub const NEW_CODE: &str = "Get a new code";
pub const OPENING: &str = "Getting a code";
pub const AWS_STEPS: &str = "Open the guide and start the stack it links. Paste this code into \
    the stack's EnrollmentCode field, named Enrollment code from Fermix.";
pub const OPEN_GUIDE: &str = "Open the guide";
pub const GUIDE_TITLE: &str = "Always On guide";
pub const GUIDE_NOT_OPENED: &str = "Your browser did not open. The guide is at \
    fermix.ai/docs/always-on.";
pub const COMMAND_TITLE: &str = "Command for the server";
pub const HOST_STEPS: &str = "On the server, run this command, and paste this code when it asks \
    for it.";
pub const COPY: &str = "Copy";
pub const COPIED: &str = "Copied";
pub const COPY_CODE: &str = "Copy the code";
pub const COPY_COMMAND: &str = "Copy the command";
pub const CODE_ELSEWHERE: &str = "A code from earlier is still open, and this app no longer \
    shows it. A new code replaces it.";
pub const CODE_EXPIRED: &str = "The code expired before a server used it. Get a new one.";
pub const WAITING: &str = "Waiting for the server to enroll. This page checks every few seconds.";
pub const STOPPED_CHECKING: &str = "This page stopped checking after 30 minutes. Open Always On \
    again to check again.";
pub const CANDIDATES_TITLE: &str = "Servers ready to take over";
pub const REVIEW: &str = "Review…";
pub const REVIEW_TITLE: &str = "Review the server";
pub const UNVERIFIED: &str = "Unverified";
pub const INSTANCE_TITLE: &str = "What the server says about itself";
pub const INSTANCE_NOTE: &str = "Fermix could not check these. Before you move, compare the \
    instance ID with InstanceId in your stack's Outputs. If they differ, do not move.";
pub const NO_INSTANCE: &str = "This server reported no cloud instance.";
pub const PASSPHRASE_TITLE: &str = "Recovery passphrase";
pub const PASSPHRASE_NOTE: &str = "Every snapshot of Fermix is encrypted to this passphrase, and \
    it is the only way to restore one. Fermix never stores it, so keep it somewhere safe.";
pub const PASSPHRASE: &str = "Recovery passphrase";
pub const PASSPHRASE_AGAIN: &str = "Recovery passphrase again";
pub const PASSPHRASE_MISMATCH: &str = "The two passphrases are not the same.";
pub const STARTING: &str = "Starting the move";
pub const READINESS_TITLE: &str = "Before you move";
pub const CHECKING: &str = "Checking what the move needs";
pub const CHECK_AGAIN: &str = "Check again";
pub const READY: &str = "Nothing here stops the move.";
pub const NOT_READY: &str = "The move cannot start while an item here stops it.";
/// The words beside an item, so its status is never a colour or an icon alone.
pub const ITEM_OK: &str = "Checked";
pub const ITEM_WARN: &str = "Worth knowing";
pub const ITEM_BLOCK: &str = "Stops the move";
pub const MOVING_TITLE: &str = "Moving Fermix";
pub const SERVER_TITLE: &str = "Server";
pub const THE_SERVER: &str = "The server";
/// The server with no name, inside a sentence.
const THE_SERVER_INLINE: &str = "the server";
pub const CONNECTED: &str = "Connected";
pub const NOT_CONNECTED: &str = "Not connected";
/// A session opening with nothing known to be wrong, drawn quietly.
pub const RECONNECTING: &str = "Reconnecting";
/// The server no longer knows this computer; its sentence says why.
pub const REMOVED: &str = "Removed";
pub const UNLINKED: &str = "Fermix on this computer runs as a desktop of a server, but it is not \
    linked to one.";
pub const CANDIDATE: &str = "Fermix on this computer waits for a home to move here.";
pub const DESKTOPS_TITLE: &str = "Desktops";
pub const DESKTOPS_NOTE: &str = "Computers that moved Fermix here. Fermix runs actions on one \
    only while its switch is on, and that computer allows it too.";
pub const RECOVERY_TITLE: &str = "Recovery";
pub const BACK_UP_NOW: &str = "Back up now";
pub const BACKING_UP: &str = "Starting a snapshot";
pub const NOTHING_VERIFIED: &str = "Nothing verified yet";
/// While the server's own picture is missing, under the daemon's sentence for why.
pub const SERVER_AWAY: &str = "The server's recovery and settings show here again once it \
    answers.";
pub const SET_PASSPHRASE: &str = "Set recovery passphrase…";
pub const SET: &str = "Set passphrase";
pub const CANCEL: &str = "Cancel";
pub const SECRETS_TITLE: &str = "Secrets to enter again";
pub const SECRETS_HERE: &str =
    "Fermix could not find these secrets on this computer, so it cannot \
    use them. Enter each one again in Settings.";
pub const SECRETS_SERVER: &str = "The server could not find these secrets, so it cannot use them. \
    Enter each one again in Settings.";
pub const GRANTS_TITLE: &str = "Folders the server may read";
pub const REMOVE: &str = "Remove";
pub const REPLACE_TITLE: &str = "Replace the server";
pub const REPLACE: &str = "Replace the server…";
/// Moving Fermix back to a desktop is not in this version (M75 fix plan, decision 4).
pub const MOVE_BACK: &str = "Moving Fermix back to this computer is not in this version. To bring \
    it back, restore the newest snapshot into a fresh home here.";

// ---- The wire --------------------------------------------------------------------------------

/// `home.status.role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Primary,
    Device,
    /// A fresh engine that enrolled and waits for a home to move to it (M75 B4).
    Candidate,
    #[serde(other)]
    Unknown,
}

/// `home.status`: the one picture of this home every surface draws.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HomeStatus {
    pub role: Role,
    pub epoch: Option<u64>,
    pub moved: Option<Moved>,
    /// In the device role after a move, the primary this computer reaches.
    pub primary: Option<Primary>,
    /// On a primary, the desktops that hold a session to it.
    #[serde(default)]
    pub desktops: Vec<Desktop>,
    pub snapshots: Snapshots,
    /// The open enrollment window, without its code.
    #[serde(default)]
    pub enrollment: Option<Enrollment>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default, rename = "move")]
    pub moving: Option<MoveState>,
    /// On a primary, the ids `secret.set` takes of the secrets its settings name and this host
    /// could not resolve: each one to enter again.
    #[serde(default)]
    pub secrets_missing: Vec<String>,
    /// On a linked desktop, the primary's own `home.status`, as it answered within its bound.
    #[serde(default)]
    pub primary_home: Option<Box<HomeStatus>>,
    /// On a linked desktop, the daemon's sentence for why `primary_home` is missing.
    #[serde(default)]
    pub primary_missing: Option<String>,
}

impl HomeStatus {
    /// Whether this is a desktop linked to a primary after a move.
    pub fn linked(&self) -> bool {
        self.role == Role::Device && self.primary.is_some()
    }

    /// The home whose recovery, enrollment and move the pane offers: this one on a primary, the
    /// server's own on a linked desktop while it answers. None anywhere else.
    pub fn home_of_record(&self) -> Option<&HomeStatus> {
        match self.role {
            Role::Primary => Some(self),
            Role::Device => self.primary_home.as_deref(),
            Role::Candidate | Role::Unknown => None,
        }
    }

    /// Where a move started here goes from: on a linked desktop the server, which is replaced
    /// and keeps no desktop of its own; anywhere else this computer.
    pub fn move_from(&self) -> MoveFrom {
        if self.linked() {
            MoveFrom::TheServer
        } else {
            MoveFrom::ThisComputer
        }
    }

    /// On a linked desktop whose server answers and holds the candidate `candidate_id`, the
    /// replacement of that server by it, as the pane follows it: both names, and the epoch the
    /// server has now.
    pub fn replacement_by(&self, candidate_id: &str) -> Option<Replacing> {
        let server = self.primary_home.as_deref().filter(|_| self.linked())?;
        let to = server
            .candidates
            .iter()
            .find(|c| c.candidate_id == candidate_id)?;
        let from = self.primary.as_ref().and_then(|p| p.name.clone());
        Some(Replacing {
            to: to.name.clone(),
            from: from.unwrap_or_else(|| THE_SERVER_INLINE.to_owned()),
            epoch: server.epoch,
        })
    }
}

/// The marker that retired this copy of the home.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Moved {
    pub to: String,
    pub moved_at: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Primary {
    pub name: Option<String>,
    pub connected: bool,
    /// The link's state in one word.
    #[serde(default)]
    pub status: LinkStatus,
    pub since: Option<String>,
    pub epoch: Option<u64>,
    pub sentence: String,
}

/// `home.status.primary.status`: the link to the primary in one closed word.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    /// A session is up.
    Connected,
    /// A session opening with nothing known to be wrong: drawn quietly, never as an outage.
    Renewing,
    /// An outage, retried with a backoff.
    Unreachable,
    /// The primary no longer knows this computer; not retried until the engine restarts.
    Revoked,
    /// The primary answered as an older copy of itself; retried.
    Stale,
    /// A word this app does not know, or an engine that sends none: `connected` says the rest.
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Desktop {
    pub device_id: String,
    pub name: String,
    pub connected: bool,
    pub since: Option<String>,
    /// Whether this primary lets the desktop run actions; null where it has no switch.
    pub executor: Option<bool>,
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
    /// A desktop asleep or away is no failure: the daemon's sentence that the snapshot waits.
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
    /// The recovery key the point was written for.
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Enrollment {
    pub state: EnrollmentState,
    pub expires_at: String,
}

/// A server that enrolled and waits for the move. Its instance facts are its own report, which
/// nobody verified (M75 B6).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Candidate {
    pub candidate_id: String,
    pub name: String,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub instance: Option<Instance>,
    #[serde(default)]
    pub snapshots: Option<CandidateSnapshots>,
    #[serde(default)]
    pub enrolled_at: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Instance {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CandidateSnapshots {
    #[serde(default)]
    pub s3_bucket: Option<String>,
    #[serde(default)]
    pub s3_region: Option<String>,
}

/// The move in progress, or how it ended: its phase and the daemon's sentence for it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MoveState {
    pub phase: String,
    pub candidate: Option<String>,
    pub sentence: String,
}

impl MoveState {
    /// Whether the move is over: done, or undone. Only the engine that came back after the
    /// restart writes either, so `committed` and `undoing` (about to restart) are not endings, and
    /// a phase this app does not know is still running.
    pub fn ended(&self) -> bool {
        matches!(self.phase.as_str(), "done" | "aborted")
    }

    pub fn undone(&self) -> bool {
        self.phase == "aborted"
    }

    /// `committed` and `undoing`: the engine is about to restart, or, when the engine that came
    /// back could not record the ending, says so in its sentence and tries again at each read.
    /// Either way there is nothing to wait on but the next read, so it is shown without a
    /// spinner.
    pub fn restarting(&self) -> bool {
        matches!(self.phase.as_str(), "committed" | "undoing")
    }
}

/// `home.enroll.open`'s answer. The code is answered once and never in a status, so no print of
/// this type spells it.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Code {
    code: String,
    pub expires_at: String,
}

impl Code {
    /// The code as the daemon answered it, for the label and the clipboard and nothing else.
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

/// `home.move.readiness`: what the owner reviews before the move, item by item.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Readiness {
    /// False while any item blocks; the daemon refuses the move until it is true.
    pub ready: bool,
    pub items: Vec<ReadinessItem>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ReadinessItem {
    pub id: String,
    pub status: ItemStatus,
    pub sentence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Ok,
    Warn,
    Block,
    /// A status this app does not know: its sentence is shown as it is.
    #[serde(other)]
    Unknown,
}

impl ItemStatus {
    /// The words beside the item.
    pub fn mark(self) -> Option<&'static str> {
        match self {
            ItemStatus::Ok => Some(ITEM_OK),
            ItemStatus::Warn => Some(ITEM_WARN),
            ItemStatus::Block => Some(ITEM_BLOCK),
            ItemStatus::Unknown => None,
        }
    }
}

/// Where a move starts from, which decides what becomes of the home it leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveFrom {
    /// This computer's home moves, and this computer stays a desktop of the server (the engine's
    /// default, so the request leaves `keep_as_desktop` out).
    ThisComputer,
    /// The server is replaced: it keeps no desktop key, row or link (`keep_as_desktop: false`).
    TheServer,
}

/// `home.move.start`'s answer: the move was written and the engine that holds the home restarts
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

/// `device.grants.list`: the folders this computer lets its primary read beyond its own sandbox,
/// oldest first, and the daemon's sentence about them.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DeviceGrants {
    pub grants: Vec<Grant>,
    pub sentence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Grant {
    pub path: String,
    pub granted_at: String,
}

/// `device.grants.remove`'s answer: the folder let go, the grants left, and the daemon's sentence.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct GrantRemoved {
    pub removed: String,
    pub grants: Vec<Grant>,
    pub sentence: String,
}

/// The sentence of a `home.move.start` the daemon sent on but could not confirm: the server had
/// it and its answer was lost (`unconfirmed`). It is no failure: the move is followed through
/// `home.status`. None for any other answer.
pub fn unconfirmed(error: &CallError) -> Option<&str> {
    match error {
        CallError::Refused(refusal) if refusal.code == "unconfirmed" => Some(&refusal.sentence),
        _ => None,
    }
}

/// `home.snapshots.passphrase`'s answer: the recovery key now set, and the daemon's sentence about
/// the snapshots written before it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PassphraseSet {
    pub key_id: String,
    pub snapshots: Snapshots,
    pub sentence: String,
}

/// The recovery passphrase, held only between the two entries and the call that carries it. No
/// print of it spells it, and nothing keeps it.
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

impl Management {
    pub fn home_status(&self) -> Result<HomeStatus, CallError> {
        decoded("home.status", self.call("home.status", json!({}))?)
    }

    /// Opens the one enrollment window, replacing an open one. Its answer is never quoted in an
    /// error, since it holds the code.
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
    /// restarts into it: this computer's, so its next reads may find it down for a moment, or,
    /// replacing the server, the server's.
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

    /// The folders this computer lets the server read, answered here even while the server is
    /// unreachable.
    pub fn device_grants_list(&self) -> Result<DeviceGrants, CallError> {
        decoded(
            "device.grants.list",
            self.call("device.grants.list", json!({}))?,
        )
    }

    /// Takes back the server's read of `path` on this computer; it holds for the next read.
    pub fn device_grants_remove(&self, path: &str) -> Result<GrantRemoved, CallError> {
        assert!(
            path.starts_with('/'),
            "a grant is removed by its absolute path"
        );
        decoded(
            "device.grants.remove",
            self.call("device.grants.remove", json!({ "path": path }))?,
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

// ---- What the app holds itself -------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Aws,
    ExistingHost,
}

/// A replacement of the server this app started from a desktop: the new server's name, the old
/// server's name, and the epoch the server had then. The server the desktop reaches afterwards
/// with a later epoch is the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacing {
    pub to: String,
    pub from: String,
    pub epoch: Option<u64>,
}

/// The pane's own steps. Each field is something the daemon does not publish.
#[derive(Debug, Default)]
pub struct Steps {
    pub destination: Option<Destination>,
    /// The code `home.enroll.open` answered, while its window may still be open.
    code: Option<Code>,
    /// The last code ran out before a server used it.
    code_expired: bool,
    pub opening: bool,
    /// The candidate under review, by id.
    reviewing: Option<String>,
    /// The readiness report for the candidate under review: its id and the answer, or the
    /// sentence for why it was not read.
    readiness: Option<(String, Result<Readiness, String>)>,
    /// A readiness read is on its way.
    pub checking: bool,
    pub starting: bool,
    /// The candidate's name this app started a move of this computer's home to, until the daemon
    /// says the move ended.
    pub started: Option<String>,
    /// On a desktop, the steps that replace the server are open.
    pub replace_open: bool,
    /// On a desktop, the replacement of the server this app started, until it ended.
    pub replacing: Option<Replacing>,
    /// The daemon's sentence for a start it could not confirm, shown while the move is followed
    /// and until the engine that holds the home answers again.
    unconfirmed: Option<String>,
    /// Since when the move this app follows has gone unanswered (Unix seconds): this computer's
    /// engine since its restart into the move, or, replacing the server, both servers.
    quiet_since: Option<i64>,
    /// Counts the moves this app started, so a read asked before one says nothing about it.
    moves: u64,
    pub backing_up: bool,
    /// A desktop's executor switch on its way: the desktop and the value asked for.
    pub executor: Option<(String, bool)>,
    /// The daemon's sentence for the last step it refused.
    pub notice: Option<String>,
}

impl Steps {
    pub fn code_opened(&mut self, code: Code) {
        self.code = Some(code);
        self.code_expired = false;
        self.opening = false;
        self.notice = None;
    }

    /// The code to show, if any.
    pub fn code(&self) -> Option<&Code> {
        self.code.as_ref()
    }

    pub fn reviewing(&self) -> Option<&str> {
        self.reviewing.as_deref()
    }

    /// The owner reviews `candidate_id`: what an earlier review read is let go, and its readiness
    /// is to be read (`checking`).
    pub fn review(&mut self, candidate_id: &str) {
        assert!(!candidate_id.is_empty(), "a review names its candidate");
        self.reviewing = Some(candidate_id.to_owned());
        self.readiness = None;
        self.checking = true;
    }

    /// A readiness read for `candidate_id` came back. An answer for a candidate no longer under
    /// review is dropped.
    pub fn readiness_read(&mut self, candidate_id: &str, answer: Result<Readiness, String>) {
        if self.reviewing.as_deref() != Some(candidate_id) {
            return;
        }
        self.checking = false;
        self.readiness = Some((candidate_id.to_owned(), answer));
    }

    /// Opens or closes the steps that replace the server; closing lets go of what they held.
    pub fn open_replace(&mut self, open: bool) {
        self.replace_open = open;
        if !open {
            self.destination = None;
            self.code = None;
            self.code_expired = false;
            self.let_go_of_review();
        }
    }

    fn let_go_of_review(&mut self) {
        self.reviewing = None;
        self.readiness = None;
        self.checking = false;
    }

    /// What `settle` and `unanswered` take with them: reads asked before the move this app
    /// started last are dropped.
    pub fn ticket(&self) -> u64 {
        self.moves
    }

    /// This computer's home started moving to `name`; the engine restarts into it.
    pub fn move_started(&mut self, name: &str) {
        self.started = Some(name.to_owned());
        self.moves += 1;
        self.quiet_since = None;
        self.let_go_of_review();
        self.notice = None;
    }

    /// A start the daemon sent on but could not confirm (`unconfirmed`): followed like one it
    /// confirmed, the replacement by `plan` on a desktop or this computer's move to `name`, with
    /// the daemon's sentence beside it until the home answers again.
    pub fn start_unconfirmed(&mut self, plan: Option<Replacing>, name: &str, sentence: &str) {
        match plan {
            Some(replacing) => self.replace_started(replacing),
            None => self.move_started(name),
        }
        self.unconfirmed = Some(sentence.to_owned());
    }

    /// The server started moving to a new server; this computer's engine runs on.
    pub fn replace_started(&mut self, replacing: Replacing) {
        self.replacing = Some(replacing);
        self.moves += 1;
        self.quiet_since = None;
        self.replace_open = false;
        self.destination = None;
        self.code = None;
        self.let_go_of_review();
        self.notice = None;
    }

    /// A read asked with `ticket` failed at `now`: the start of the wait, if this is its first.
    /// False for a read asked before the last move started, which the pane drops.
    pub fn unanswered(&mut self, ticket: u64, now: i64) -> bool {
        if ticket != self.moves {
            return false;
        }
        self.quiet_since.get_or_insert(now);
        true
    }

    /// Brings the steps in line with a fresh `home.status`, asked with `ticket`, at `now` (Unix
    /// seconds). The code is let go once the daemon says its window was used or ran out, or its own
    /// expiry passed; a candidate under review that is no longer held is let go; a move this app
    /// started is forgotten once the engine that came back after the restart says it ended
    /// (`done` or `aborted`); a replacement once the server's move was undone or the server this
    /// computer reaches has a later epoch. Until then a read that fails is the engine restarting
    /// into the move, not a stopped Fermix. A read asked before the last move started says nothing
    /// about it: false, and the pane drops it.
    pub fn settle(&mut self, status: &HomeStatus, ticket: u64, now: i64) -> bool {
        if ticket != self.moves {
            return false;
        }
        if let Some(home) = status.home_of_record() {
            self.settle_code(home, now);
            let held = |id: &String| home.candidates.iter().any(|c| &c.candidate_id == id);
            if !self.reviewing.as_ref().is_some_and(held) {
                self.let_go_of_review();
            }
        }
        if status.moving.as_ref().is_some_and(MoveState::ended) {
            self.started = None;
        }
        self.settle_replacing(status);
        let quiet = self.replacing.is_some() && status.primary_home.is_none();
        if !quiet {
            self.unconfirmed = None;
        }
        self.quiet_since = if quiet {
            Some(self.quiet_since.unwrap_or(now))
        } else {
            None
        };
        true
    }

    fn settle_code(&mut self, home: &HomeStatus, now: i64) {
        let window = home.enrollment.as_ref().map(|e| e.state);
        let lapsed = self
            .code
            .as_ref()
            .and_then(|c| unix_seconds(&c.expires_at))
            .is_some_and(|expires| now >= expires);
        let expired = lapsed || window == Some(EnrollmentState::Expired);
        if self.code.is_some() && (expired || window == Some(EnrollmentState::Used)) {
            self.code_expired = expired && home.candidates.is_empty();
            self.code = None;
        }
    }

    fn settle_replacing(&mut self, status: &HomeStatus) {
        let (Some(replacing), Some(server)) = (&self.replacing, status.primary_home.as_deref())
        else {
            return;
        };
        // The server answers with no move at all after a start it could not confirm: none ran.
        let none = server.moving.is_none();
        let undone = server.moving.as_ref().is_some_and(MoveState::undone);
        let later = server
            .epoch
            .zip(replacing.epoch)
            .is_some_and(|(now, then)| now > then);
        if none || undone || later {
            self.replacing = None;
        }
    }

    /// How long the move this app follows has gone unanswered at `now`, in seconds.
    fn quiet_for(&self, now: i64) -> Option<i64> {
        self.quiet_since.map(|since| now - since)
    }
}

// ---- What the pane draws -------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub stage: Stage,
    /// The recovery group, once `home.status` answered.
    pub recovery: Option<Recovery>,
    /// The desktops of a primary.
    pub desktops: Vec<DesktopRow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    /// Nothing read yet.
    Reading,
    /// `home.status` could not be read, and no move of this app's explains it.
    Down(DaemonProblem),
    /// This home is the primary and has not moved: the steps before a move.
    Setup(Setup),
    /// A move is under way.
    Moving(Moving),
    /// After the move: the primary this computer reaches, and the steps that replace it.
    Desktop(Box<DesktopStage>),
    /// The device role with no link to a primary.
    Unlinked,
    /// A candidate: the daemon's sentence for where its enrollment and move stand.
    Candidate(String),
}

/// A move under way: the daemon's sentence for its phase, or the app's own while the engine that
/// holds the home cannot be read.
#[derive(Debug, Clone, PartialEq)]
pub struct Moving {
    pub sentence: String,
    /// While a replacement waits on the servers, the daemon's sentence for why the server cannot
    /// be read.
    pub detail: Option<String>,
    /// Something is on its way that the pane waits on: it spins. Not past a grace, and not while
    /// the engine is about to restart or could not record the ending.
    pub waiting: bool,
    /// The wait went past its grace: the pane stops spinning and says so.
    pub stalled: bool,
    /// Once the wait for this computer's engine is stalled, what became of it: the pane offers
    /// what the down page would, to start or restart it.
    pub problem: Option<DaemonProblem>,
}

impl Moving {
    /// The daemon's own sentence for the move's phase.
    fn said(moving: &MoveState) -> Moving {
        Moving {
            sentence: moving.sentence.clone(),
            detail: None,
            waiting: !moving.restarting(),
            stalled: false,
            problem: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesktopStage {
    pub server: ServerRow,
    /// What replacing the server does, named.
    pub explain: String,
    /// Replacing the server is offered: the server answers, as the primary.
    pub can_replace: bool,
    /// The steps that replace the server, drawn from its own picture, while they are open.
    pub replace: Option<Setup>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    /// The daemon's sentence for an undone move or for the last refused step.
    pub notice: Option<String>,
    pub destination: Option<Destination>,
    pub code: Option<CodeView>,
    /// Set while the owner may ask for a code: the label of the button that does.
    pub get_code: Option<&'static str>,
    pub opening: bool,
    /// A line about the code when there is none to show.
    pub code_note: Option<&'static str>,
    pub waiting: bool,
    pub candidates: Vec<CandidateRow>,
    pub review: Option<Review>,
    pub starting: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeView {
    pub text: String,
    pub expiry: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateRow {
    pub id: String,
    pub title: String,
    pub subtitle: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Review {
    pub id: String,
    pub name: String,
    /// (label, value) facts about the host.
    pub host: Vec<(&'static str, String)>,
    /// (label, value) facts the server reported about its cloud instance, every one unverified.
    pub instance: Vec<(&'static str, String)>,
    /// What the move needs and leaves behind, as `home.move.readiness` answered.
    pub readiness: Checked,
    /// Move applies: the readiness report answered with nothing that blocks, and no start is on
    /// its way. The two passphrase entries must agree as well, which the pane checks.
    pub can_start: bool,
}

/// Where the readiness report of the candidate under review stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Checked {
    Checking,
    /// It could not be read: the daemon's sentence, or what became of it.
    Unread(String),
    Answered(Readiness),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerRow {
    pub name: String,
    pub state: &'static str,
    /// The link needs the owner's eye (revoked, or a stale primary answering): the row stands out,
    /// and its sentence says why.
    pub alarm: bool,
    pub sentence: String,
    /// "Moved here …" facts of the marker, where the daemon has one.
    pub moved: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DesktopRow {
    pub device_id: String,
    pub name: String,
    pub state: &'static str,
    /// The switch as it should show: the value on its way while one is, else the daemon's.
    pub executor: Option<bool>,
    pub switching: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    pub sentence: String,
    pub targets: Vec<TargetRow>,
    pub next: Option<String>,
    /// Set while "Back up now" applies: snapshots are configured and none is running.
    pub can_run: bool,
    pub backing_up: bool,
    /// The primary has no recovery key yet: "Set recovery passphrase…" applies.
    pub can_set_passphrase: bool,
    /// The secrets the primary could not resolve, by the id `secret.set` takes.
    pub secrets: Vec<String>,
    /// Where those secrets are missing and what to do, shown above them.
    pub secrets_note: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TargetRow {
    pub title: String,
    pub verified: String,
    pub destination: Option<String>,
    pub error: Option<String>,
    /// The daemon's sentence that the snapshot waits for a desktop asleep or away.
    pub waiting: Option<String>,
}

/// The pane from its last read and its own steps, at `now` (Unix seconds). `read` is None before
/// the first answer.
pub fn view(read: Option<Result<&HomeStatus, &DaemonProblem>>, steps: &Steps, now: i64) -> View {
    let status = match read {
        None => return bare(Stage::Reading),
        Some(Err(problem)) => return bare(unread(problem, steps, now)),
        Some(Ok(status)) => status,
    };
    View {
        stage: stage(status, steps, now),
        recovery: Some(recovery(status, steps, now)),
        desktops: status.desktops.iter().map(|d| desktop(d, steps)).collect(),
    }
}

fn bare(stage: Stage) -> View {
    View {
        stage,
        recovery: None,
        desktops: Vec::new(),
    }
}

/// A read that failed: the restart into a move this app started, or what became of Fermix. Past
/// `RESTART_GRACE` the restart is no longer only a restart, and the pane says so.
fn unread(problem: &DaemonProblem, steps: &Steps, now: i64) -> Stage {
    let Some(name) = &steps.started else {
        return Stage::Down(problem.clone());
    };
    let stalled = past(steps, RESTART_GRACE, now);
    Stage::Moving(Moving {
        sentence: if stalled {
            not_back(name)
        } else {
            restarting(name)
        },
        detail: steps.unconfirmed.clone(),
        waiting: !stalled,
        stalled,
        problem: stalled.then(|| problem.clone()),
    })
}

fn past(steps: &Steps, grace: Duration, now: i64) -> bool {
    let grace = i64::try_from(grace.as_secs()).expect("a grace in seconds fits");
    steps.quiet_for(now).is_some_and(|quiet| quiet >= grace)
}

/// The button that starts the move to `name`.
pub fn move_to(name: &str) -> String {
    format!("Move Fermix to {name}")
}

/// The app's own words while the engine restarts into the move it asked for.
pub fn restarting(name: &str) -> String {
    format!("Fermix is restarting to move this home to {name}.")
}

/// The app's own words once that restart went past its grace.
pub fn not_back(name: &str) -> String {
    format!(
        "Fermix did not come back after restarting to move this home to {name}. Once it runs \
         again, the move goes on from where it stopped."
    )
}

/// What replacing the server `name` does, above its steps.
pub fn replace_explain(name: &str) -> String {
    format!(
        "Fermix moves from {name} to a new server you choose, and {name} serves nothing once the \
         move is done. This computer stays a desktop, of the new server."
    )
}

/// The app's own words while neither server answers during a replacement.
pub fn moving_between(from: &str, to: &str) -> String {
    format!("Fermix is moving from {from} to {to}. This computer reaches it again once {to} takes over.")
}

/// The app's own words once a replacement went past its grace with neither server answering.
pub fn not_taken_over(to: &str) -> String {
    format!(
        "{to} has not taken over yet, and neither server answers this computer. Sending a large \
         home takes longer; this page keeps checking."
    )
}

fn stage(status: &HomeStatus, steps: &Steps, now: i64) -> Stage {
    if status.role == Role::Candidate {
        let sentence = status.moving.as_ref().map(|m| m.sentence.clone());
        return Stage::Candidate(sentence.unwrap_or_else(|| CANDIDATE.to_owned()));
    }
    if let Some(moving) = status.moving.as_ref().filter(|m| !m.ended()) {
        return Stage::Moving(Moving::said(moving));
    }
    match (status.role, &status.primary) {
        (Role::Primary, _) => Stage::Setup(setup(status, steps, now)),
        (_, Some(primary)) => match &steps.replacing {
            Some(replacing) => Stage::Moving(replacement(status, replacing, steps, now)),
            None => Stage::Desktop(Box::new(desktop_stage(status, primary, steps, now))),
        },
        (_, None) => Stage::Unlinked,
    }
}

/// A replacement this app started, as the desktop sees it: the server's own sentence while it
/// answers, else the app's, and past `REPLACE_GRACE` the app's plain word that it waits.
fn replacement(status: &HomeStatus, replacing: &Replacing, steps: &Steps, now: i64) -> Moving {
    let running = status
        .primary_home
        .as_ref()
        .and_then(|server| server.moving.as_ref())
        .filter(|m| !m.ended());
    if let Some(moving) = running {
        return Moving::said(moving);
    }
    let stalled = past(steps, REPLACE_GRACE, now);
    Moving {
        sentence: if stalled {
            not_taken_over(&replacing.to)
        } else {
            moving_between(&replacing.from, &replacing.to)
        },
        detail: steps
            .unconfirmed
            .clone()
            .or_else(|| status.primary_missing.clone()),
        waiting: !stalled,
        stalled,
        problem: None,
    }
}

fn desktop_stage(status: &HomeStatus, primary: &Primary, steps: &Steps, now: i64) -> DesktopStage {
    let server_home = status
        .primary_home
        .as_deref()
        .filter(|server| server.role == Role::Primary);
    let name = primary.name.as_deref().unwrap_or(THE_SERVER_INLINE);
    DesktopStage {
        server: server(primary, status.moved.as_ref(), now),
        explain: replace_explain(name),
        can_replace: server_home.is_some(),
        replace: server_home
            .filter(|_| steps.replace_open)
            .map(|server| setup(server, steps, now)),
    }
}

fn server(primary: &Primary, moved: Option<&Moved>, now: i64) -> ServerRow {
    let moved = moved.map(|m| match unix_seconds(&m.moved_at) {
        Some(at) => format!("Fermix moved to {} {} ago.", m.to, span(now - at)),
        None => format!("Fermix moved to {}.", m.to),
    });
    ServerRow {
        name: primary
            .name
            .clone()
            .unwrap_or_else(|| THE_SERVER.to_owned()),
        state: link_word(primary),
        alarm: matches!(primary.status, LinkStatus::Revoked | LinkStatus::Stale),
        sentence: primary.sentence.clone(),
        moved,
    }
}

/// The link in one word, from `status`; an engine that sends no word is read by `connected`.
fn link_word(primary: &Primary) -> &'static str {
    match primary.status {
        LinkStatus::Connected => CONNECTED,
        LinkStatus::Renewing => RECONNECTING,
        LinkStatus::Unreachable | LinkStatus::Stale => NOT_CONNECTED,
        LinkStatus::Revoked => REMOVED,
        LinkStatus::Unknown if primary.connected => CONNECTED,
        LinkStatus::Unknown => NOT_CONNECTED,
    }
}

/// A folder the server may read here, as the pane lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRow {
    pub path: String,
    /// "Allowed 3 hours ago".
    pub granted: String,
}

/// The read grants as rows, at `now` (Unix seconds).
pub fn grant_rows(grants: &DeviceGrants, now: i64) -> Vec<GrantRow> {
    grants
        .grants
        .iter()
        .map(|grant| GrantRow {
            path: grant.path.clone(),
            granted: match unix_seconds(&grant.granted_at) {
                Some(at) => format!("Allowed {} ago", span(now - at)),
                None => format!("Allowed {}", grant.granted_at),
            },
        })
        .collect()
}

/// The steps before a move of `home`: this computer's on a primary, the server's when a desktop
/// replaces it.
fn setup(home: &HomeStatus, steps: &Steps, now: i64) -> Setup {
    let undone = home
        .moving
        .as_ref()
        .filter(|m| m.undone())
        .map(|m| m.sentence.clone());
    let code = steps.code().map(|code| CodeView {
        text: code.text().to_owned(),
        expiry: expiry(&code.expires_at, now),
    });
    let open_elsewhere = matches!(
        home.enrollment.as_ref().map(|e| e.state),
        Some(EnrollmentState::Open)
    );
    let code_note = match &code {
        Some(_) => None,
        None if open_elsewhere => Some(CODE_ELSEWHERE),
        None if steps.code_expired => Some(CODE_EXPIRED),
        None => None,
    };
    let chosen = steps.destination.is_some();
    let get_code = match (&code, chosen, steps.opening) {
        (_, false, _) | (_, _, true) => None,
        (Some(_), true, false) => Some(NEW_CODE),
        (None, true, false) if open_elsewhere || steps.code_expired => Some(NEW_CODE),
        (None, true, false) => Some(GET_CODE),
    };
    Setup {
        notice: steps.notice.clone().or(undone),
        destination: steps.destination,
        waiting: code.is_some() && home.candidates.is_empty(),
        code,
        get_code,
        opening: steps.opening,
        code_note,
        candidates: home.candidates.iter().map(candidate_row).collect(),
        review: steps
            .reviewing()
            .and_then(|id| home.candidates.iter().find(|c| c.candidate_id == id))
            .map(|c| review(c, steps, now)),
        starting: steps.starting,
    }
}

/// "Use it within 9 minutes. It works once." from the code's expiry.
pub fn expiry(expires_at: &str, now: i64) -> String {
    match unix_seconds(expires_at).map(|at| at - now) {
        Some(left) if left >= 60 => format!("Use it within {}. It works once.", span(left)),
        Some(left) if left > 0 => "Use it within a minute. It works once.".to_owned(),
        Some(_) => "It has expired.".to_owned(),
        None => "It works once.".to_owned(),
    }
}

/// A length of time in its largest whole unit: "3 hours", "1 minute", "less than a minute".
pub fn span(seconds: i64) -> String {
    let units = [(86_400, "day"), (3_600, "hour"), (60, "minute")];
    let Some((size, unit)) = units.into_iter().find(|(size, _)| seconds >= *size) else {
        return "less than a minute".to_owned();
    };
    let count = seconds / size;
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural}")
}

fn system(candidate: &Candidate) -> Option<String> {
    let parts: Vec<&str> = [&candidate.os, &candidate.arch]
        .into_iter()
        .filter_map(|p| p.as_deref().filter(|s| !s.is_empty()))
        .collect();
    (!parts.is_empty()).then(|| parts.join(", "))
}

fn candidate_row(candidate: &Candidate) -> CandidateRow {
    let version = candidate.version.as_deref().map(|v| format!("Fermix {v}"));
    let subtitle: Vec<String> = [system(candidate), version].into_iter().flatten().collect();
    CandidateRow {
        id: candidate.candidate_id.clone(),
        title: candidate.name.clone(),
        subtitle: subtitle.join(", "),
    }
}

fn review(candidate: &Candidate, steps: &Steps, now: i64) -> Review {
    let mut host: Vec<(&'static str, String)> = vec![("Name", candidate.name.clone())];
    host.extend(system(candidate).map(|s| ("System", s)));
    host.extend(candidate.version.clone().map(|v| ("Fermix", v)));
    let enrolled = candidate.enrolled_at.as_deref().and_then(unix_seconds);
    host.extend(enrolled.map(|at| ("Enrolled", format!("{} ago", span(now - at)))));
    let bucket = candidate
        .snapshots
        .as_ref()
        .and_then(|s| s.s3_bucket.clone());
    host.extend(bucket.map(|b| ("Snapshot bucket", b)));
    let instance = candidate.instance.as_ref().map_or_else(Vec::new, |i| {
        [
            ("Provider", &i.provider),
            ("Instance ID", &i.instance_id),
            ("Region", &i.region),
            ("Account", &i.account_id),
        ]
        .into_iter()
        .filter_map(|(label, value)| value.clone().map(|v| (label, v)))
        .collect()
    });
    let readiness = match &steps.readiness {
        Some((id, Ok(report))) if *id == candidate.candidate_id => {
            Checked::Answered(report.clone())
        }
        Some((id, Err(sentence))) if *id == candidate.candidate_id => {
            Checked::Unread(sentence.clone())
        }
        _ => Checked::Checking,
    };
    let ready = matches!(&readiness, Checked::Answered(report) if report.ready);
    Review {
        id: candidate.candidate_id.clone(),
        name: candidate.name.clone(),
        host,
        instance,
        can_start: ready && !steps.starting && !steps.checking,
        readiness,
    }
}

fn desktop(desktop: &Desktop, steps: &Steps) -> DesktopRow {
    let pending = steps
        .executor
        .as_ref()
        .filter(|(id, _)| *id == desktop.device_id)
        .map(|(_, on)| *on);
    DesktopRow {
        device_id: desktop.device_id.clone(),
        name: desktop.name.clone(),
        state: if desktop.connected {
            CONNECTED
        } else {
            NOT_CONNECTED
        },
        executor: pending.or(desktop.executor),
        switching: pending.is_some(),
    }
}

/// Recovery is the primary's: this home's on a primary (and on any engine that is not linked),
/// the server's own on a linked desktop, or, while the server cannot be read, why.
fn recovery(status: &HomeStatus, steps: &Steps, now: i64) -> Recovery {
    if !status.linked() {
        return recovery_of(status, SECRETS_HERE, steps, now);
    }
    match status.primary_home.as_deref() {
        Some(server) => recovery_of(server, SECRETS_SERVER, steps, now),
        None => server_away(status.primary_missing.as_deref(), steps),
    }
}

fn recovery_of(home: &HomeStatus, secrets_note: &'static str, steps: &Steps, now: i64) -> Recovery {
    let snapshots = &home.snapshots;
    let next = snapshots
        .next_run_at
        .as_deref()
        .filter(|_| !snapshots.running)
        .and_then(unix_seconds)
        .map(|at| match at - now {
            left if left > 0 => format!("Next snapshot in {}", span(left)),
            _ => "Next snapshot due now".to_owned(),
        });
    Recovery {
        sentence: snapshots.sentence.clone(),
        targets: snapshots.targets.iter().map(|t| target(t, now)).collect(),
        next,
        can_run: snapshots.configured && !snapshots.running,
        backing_up: steps.backing_up,
        can_set_passphrase: home.role == Role::Primary && !snapshots.recipient_set,
        secrets: home.secrets_missing.clone(),
        secrets_note,
    }
}

fn server_away(why: Option<&str>, steps: &Steps) -> Recovery {
    let sentence = match why {
        Some(why) => format!("{why}\n{SERVER_AWAY}"),
        None => SERVER_AWAY.to_owned(),
    };
    Recovery {
        sentence,
        targets: Vec::new(),
        next: None,
        can_run: false,
        backing_up: steps.backing_up,
        can_set_passphrase: false,
        secrets: Vec::new(),
        secrets_note: SECRETS_SERVER,
    }
}

fn target(target: &Target, now: i64) -> TargetRow {
    let verified = target.last_verified.as_ref().map_or_else(
        || NOTHING_VERIFIED.to_owned(),
        |point| match unix_seconds(&point.at) {
            Some(at) => format!("Last verified {} ago", span(now - at)),
            None => format!("Last verified {}", point.at),
        },
    );
    TargetRow {
        title: target.label.clone(),
        verified,
        destination: target.destination.clone(),
        error: target.last_error.clone(),
        waiting: target.waiting.clone(),
    }
}
