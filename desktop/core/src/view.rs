//! What each screen says, computed from the daemon's answers and the app's own
//! in-flight work. Pure, so every sentence is tested; the GTK side only draws it.
//! State sentences the daemon writes (failures, restart reasons) pass through
//! untouched; the copy here is labels, verbs and the attention catalogue ported
//! from the macOS app (`AttentionCatalogue`, `ProductStrings`).

use crate::management::{CallError, Refusal, PROTOCOL_VERSION};
use crate::model::{Hello, ProviderRow, ReadinessFailure, SetupState};
use crate::providers::{connection, doors, Connection, Door, ImportSource};
use std::io::ErrorKind;

/// What the app is doing with one provider right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Job {
        job_id: String,
        door: Door,
        phase: Option<String>,
        /// The app holds this sign-in's link. A sign-in found already running
        /// (started before the app did) has none.
        has_link: bool,
        browser_failed: bool,
    },
    Saving,
    SigningOut,
    RemovingKey,
    Switching,
    /// The daemon's own sentence for why the last action failed.
    Failed(String),
    Cancelled,
}

impl Activity {
    /// Work the app is still waiting on. `Failed` and `Cancelled` only explain the
    /// last action, so they must never hold back a refresh.
    pub fn in_flight(&self) -> bool {
        !matches!(
            self,
            Activity::Idle | Activity::Failed(_) | Activity::Cancelled
        )
    }
}

/// A change the app just made, acknowledged for a short while even when the
/// daemon's row reads the same as before (a ChatGPT re-sign-in does).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recent {
    SignedIn,
    Imported(ImportSource),
    KeyAdded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    MakePrimary,
    Door(Door),
    ReplaceKey,
    RemoveKey,
    SignOut,
}

impl MenuItem {
    pub fn label(self) -> String {
        match self {
            MenuItem::MakePrimary => "Make primary".into(),
            MenuItem::Door(door) => door.verb(false),
            MenuItem::ReplaceKey => "Replace key…".into(),
            MenuItem::RemoveKey => "Remove key".into(),
            MenuItem::SignOut => "Sign out".into(),
        }
    }
}

/// Whether a busy row offers the sign-in link, and how loudly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyLink {
    None,
    /// Beside Cancel, for finishing the sign-in in another browser.
    Offered,
    /// The way forward, because the browser did not open.
    Rescue,
}

/// The one way in a resting row shows as a button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lead {
    pub door: Door,
    pub verb: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Suffix {
    /// At most one visible button, and everything else behind the ⋮ menu
    /// (drawn only when `more` has something in it).
    Resting {
        lead: Option<Lead>,
        more: Vec<MenuItem>,
    },
    Busy {
        copy_link: CopyLink,
        cancel: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub subtitle: String,
    pub is_error: bool,
    pub suffix: Suffix,
}

pub fn row_view(row: &ProviderRow, activity: &Activity, recent: Option<Recent>) -> RowView {
    let resting = |subtitle: String, is_error: bool| RowView {
        subtitle,
        is_error,
        suffix: idle_suffix(row),
    };
    match activity {
        Activity::Job {
            job_id,
            door,
            phase,
            has_link,
            browser_failed,
        } => {
            let view = job_view(*door, phase.as_deref(), *has_link, *browser_failed);
            // Nothing can be cancelled until the daemon has named the job.
            match view.suffix {
                Suffix::Busy { copy_link, .. } if job_id.is_empty() => {
                    busy(&view.subtitle, copy_link, false)
                }
                _ => view,
            }
        }
        Activity::Saving => busy("Adding the key", CopyLink::None, false),
        Activity::SigningOut => busy("Signing out", CopyLink::None, false),
        Activity::RemovingKey => busy("Removing the key", CopyLink::None, false),
        Activity::Switching => busy("Switching", CopyLink::None, false),
        Activity::Failed(sentence) => resting(sentence.clone(), true),
        Activity::Cancelled => resting("Sign-in cancelled".into(), false),
        Activity::Idle => resting(with_primary(row, resting_sentence(row, recent)), false),
    }
}

fn busy(subtitle: &str, copy_link: CopyLink, cancel: bool) -> RowView {
    RowView {
        subtitle: subtitle.into(),
        is_error: false,
        suffix: Suffix::Busy { copy_link, cancel },
    }
}

fn job_view(door: Door, phase: Option<&str>, has_link: bool, browser_failed: bool) -> RowView {
    if let Door::Import(source) = door {
        return match phase {
            Some("verifying") => busy("Checking the login", CopyLink::None, false),
            _ => busy(
                &format!("Reading your {} login", source.product()),
                CopyLink::None,
                true,
            ),
        };
    }
    match phase {
        Some("verifying") => busy("Checking the sign-in", CopyLink::None, false),
        _ if browser_failed => busy("Your browser did not open", CopyLink::Rescue, true),
        Some("awaiting_browser") if has_link => {
            busy("Continue in your browser", CopyLink::Offered, true)
        }
        Some("awaiting_browser") => busy("A sign-in is already running", CopyLink::None, true),
        _ => busy("Starting sign-in", CopyLink::None, true),
    }
}

/// "Just now" is only said while the row still reads connected, so a sign-out
/// or a key removed elsewhere is never covered up.
fn resting_sentence(row: &ProviderRow, recent: Option<Recent>) -> String {
    if row.auth_mode.as_deref() == Some("none") {
        return "No sign-in needed".into();
    }
    let uses_key = row.auth_mode.as_deref() == Some("api_key");
    match (connection(row), recent) {
        (Connection::Connected, Some(recent)) => recent_sentence(recent),
        (Connection::Connected, None) if uses_key => "Key added".into(),
        (Connection::Connected, None) => "Signed in".into(),
        (Connection::Expired, _) => "Sign-in expired".into(),
        (Connection::NotConnected, _) if doors(row) == [Door::ApiKey] => "No API key".into(),
        (Connection::NotConnected, _) => "Not signed in".into(),
    }
}

fn recent_sentence(recent: Recent) -> String {
    match recent {
        Recent::SignedIn => "Signed in just now".into(),
        Recent::Imported(source) => {
            format!("Signed in just now with your {} login", source.product())
        }
        Recent::KeyAdded => "Key added just now".into(),
    }
}

fn with_primary(row: &ProviderRow, sentence: String) -> String {
    if row.primary {
        format!("{sentence} · Primary")
    } else {
        sentence
    }
}

/// Sign-in is the primary way in, so the row leads with its first way in even
/// once connected; the rest waits behind ⋮ (owner, 2026-09-25). A key is only
/// the lead while none is stored: a stored one is replaced from the menu.
fn idle_suffix(row: &ProviderRow) -> Suffix {
    let ways_in = doors(row);
    let link = connection(row);
    let lead_door = ways_in
        .first()
        .copied()
        .filter(|door| !(link == Connection::Connected && *door == Door::ApiKey));
    let others = ways_in.iter().filter(|door| Some(**door) != lead_door);
    let more = if link == Connection::Connected {
        connected_menu(row, others)
    } else {
        others.map(|door| MenuItem::Door(*door)).collect()
    };
    let lead = lead_door.map(|door| Lead {
        door,
        verb: lead_verb(row, door, link),
    });
    Suffix::Resting { lead, more }
}

/// A sign-in that already holds a token is "again", whether it works or not.
fn lead_verb(row: &ProviderRow, door: Door, link: Connection) -> String {
    let signed_in_this_way = row.auth_mode.as_deref() == Some("oauth");
    match door {
        Door::ApiKey => "Add key…".into(),
        _ => door.verb(link != Connection::NotConnected && signed_in_this_way),
    }
}

fn connected_menu<'a>(row: &ProviderRow, others: impl Iterator<Item = &'a Door>) -> Vec<MenuItem> {
    let mut items = Vec::new();
    if !row.primary {
        items.push(MenuItem::MakePrimary);
    }
    let uses_key = row.auth_mode.as_deref() == Some("api_key");
    let other_doors = others.filter(|d| !(uses_key && **d == Door::ApiKey));
    items.extend(other_doors.map(|d| MenuItem::Door(*d)));
    match row.auth_mode.as_deref() {
        Some("api_key") => items.extend([MenuItem::ReplaceKey, MenuItem::RemoveKey]),
        Some("oauth") => items.push(MenuItem::SignOut),
        _ => {}
    }
    items
}

// ---- Home ----------------------------------------------------------------

pub fn status_word(state: &SetupState) -> &'static str {
    if state.readiness.status != "ready" {
        "Setup required"
    } else if state.restart.required {
        "Restart to finish updating"
    } else {
        "Ready"
    }
}

pub fn answers_with(state: &SetupState) -> String {
    let Some(primary) = state.providers.iter().find(|p| p.primary) else {
        return "No provider yet".into();
    };
    match &primary.default_model {
        Some(model) => format!("{} · {model}", primary.label),
        None => primary.label.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttentionAction {
    /// Start this provider's first way in (`win.door` with `target`).
    Door {
        target: String,
        verb: String,
    },
    /// The settings pane the daemon says the gap is in.
    OpenPane(String),
    Restart,
    /// A pane this app does not know: open the daemon's own setup page in the browser.
    OpenSetupPage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionRow {
    pub title: String,
    pub body: String,
    pub action: Option<AttentionAction>,
}

/// Gating gaps first, then advisory ones, then one row for a pending restart.
pub fn attention_rows(state: &SetupState) -> Vec<AttentionRow> {
    let mut failures: Vec<&ReadinessFailure> = state
        .readiness
        .failures
        .iter()
        .filter(|f| !(state.restart.required && f.detail_key == "restart_pending"))
        .collect();
    failures.sort_by_key(|f| !f.gating);
    let mut rows: Vec<AttentionRow> = failures.iter().map(|f| attention_row(f, state)).collect();
    if state.restart.required {
        rows.push(restart_row(state));
    }
    rows
}

fn restart_row(state: &SetupState) -> AttentionRow {
    let reasons: Vec<&str> = state
        .restart
        .reasons
        .iter()
        .map(|r| r.sentence.as_str())
        .collect();
    let body = if reasons.is_empty() {
        "A saved change is waiting for Fermix to restart.".to_owned()
    } else {
        reasons.join("\n")
    };
    AttentionRow {
        title: "Restart to apply your changes".into(),
        body,
        action: Some(AttentionAction::Restart),
    }
}

fn attention_row(failure: &ReadinessFailure, state: &SetupState) -> AttentionRow {
    let (title, body) = attention_copy(&failure.detail_key, state);
    AttentionRow {
        title,
        body: body.into(),
        action: Some(attention_action(failure, state)),
    }
}

fn attention_action(failure: &ReadinessFailure, state: &SetupState) -> AttentionAction {
    if failure.detail_key == "restart_pending" {
        return AttentionAction::Restart;
    }
    if let Some(action) = door_action(&failure.detail_key, state) {
        return action;
    }
    match failure.pane.as_deref().and_then(crate::settings::pane) {
        Some(pane) => AttentionAction::OpenPane(pane.slug.to_owned()),
        None => AttentionAction::OpenSetupPage,
    }
}

/// A missing credential offers the provider's first way in, the same button its
/// Providers row shows, so one click on Home starts it.
fn door_action(key: &str, state: &SetupState) -> Option<AttentionAction> {
    let provider = key.strip_prefix(CREDENTIALS_PREFIX)?;
    let row = state.providers.iter().find(|p| p.id == provider)?;
    let main = *doors(row).first()?;
    Some(AttentionAction::Door {
        target: main.target(&row.id),
        verb: main.verb(connection(row) == Connection::Expired),
    })
}

/// The daemon titles its channels in `settings.sections`, which Home does not read;
/// these are the same spellings. An unknown channel shows its own name.
pub fn channel_title(name: &str) -> &str {
    match name {
        "acp" => "Editors",
        "telegram" => "Telegram",
        "whatsapp" => "WhatsApp",
        "discord" => "Discord",
        "slack" => "Slack",
        "signal" => "Signal",
        other => other,
    }
}

const CREDENTIALS_PREFIX: &str = "provider:missing_credentials:";
const CHANNEL_PREFIX: &str = "channel:";
const UNRECOGNIZED_BODY: &str =
    "This Fermix build has no description for that gap, so the daemon's own name for it is shown.";

fn attention_copy(key: &str, state: &SetupState) -> (String, &'static str) {
    if let Some(provider) = key
        .strip_prefix(CREDENTIALS_PREFIX)
        .filter(|p| !p.is_empty())
    {
        let label = state
            .providers
            .iter()
            .find(|p| p.id == provider)
            .map_or(provider, |p| p.label.as_str());
        return (
            format!("Connect {label}"),
            "Fermix has no working credential for this provider yet.",
        );
    }
    if let Some(channel) = key.strip_prefix(CHANNEL_PREFIX).filter(|c| !c.is_empty()) {
        return (
            format!("{} is on but not finished", channel_title(channel)),
            "The channel is enabled and still missing something it needs, so it will not answer.",
        );
    }
    match fixed_attention_copy(key) {
        Some((title, body)) => (title.into(), body),
        None => (key.into(), UNRECOGNIZED_BODY),
    }
}

pub(crate) fn fixed_attention_copy(key: &str) -> Option<(&'static str, &'static str)> {
    let copy = match key {
        "personalization" => ("Tell Fermix about you", "Fermix keeps your name, time zone and style so it can address you and keep time straight."),
        "provider:unknown_configured" => ("The configured provider is not one Fermix knows", "Choose a provider Fermix can reach, or the assistant has nothing to answer with."),
        "provider:multiple_primary" => ("More than one provider is marked primary", "Exactly one provider answers each conversation, so pick which of them it is."),
        "provider:invalid_auth_mode" => ("A provider is set to sign in a way it does not offer", "Pick an authentication mode that provider supports, or it will refuse every call."),
        "realtime:openai" => ("The voice companion needs an OpenAI key", "Voice uses the OpenAI Realtime API, which a Codex or Claude sign-in does not authorize."),
        "sandbox:env_missing" => ("Some allowed environment variables are not set", "Sandboxed commands run without them until each value is stored or its name is removed from the allowed list."),
        "sandbox:env_helper_failed" => ("A helper command for an environment variable failed", "Sandboxed commands run without that value until the helper command in the settings file works again."),
        "restart_pending" => ("Restart to apply your changes", "A saved change is waiting for Fermix to restart."),
        "external_config_change" => ("Settings changed outside Fermix", "Something else edited the settings file. Fermix will not save changes until it reloads it."),
        "config_unreadable" => ("The settings file cannot be read", "Fermix cannot parse the settings file, so it will not change it."),
        "legacy_service_unit" => ("Another Fermix service is registered on this computer", "Two services sharing one home will fight over it."),
        "secret_acl_restricted" => ("Some stored keys cannot be read", "The keyring will not hand these to Fermix, so the providers that use them cannot sign in."),
        _ => return None,
    };
    Some(copy)
}

// ---- The daemon itself ----------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonProblem {
    /// No socket: the service is stopped.
    NotRunning,
    /// A socket, but nothing answers in time.
    NotResponding,
    /// It answered, but not in a way this app understands.
    Broken(String),
    /// One side is too old for the other; the sentence says which to update.
    UpdateNeeded(String),
}

const APP_TOO_OLD: &str = "This app is older than Fermix. Update the Fermix app.";
const DAEMON_TOO_OLD: &str = "Fermix is older than this app. Update the fermix package.";

impl DaemonProblem {
    pub fn status_word(&self) -> &'static str {
        match self {
            DaemonProblem::NotRunning => "Not running",
            DaemonProblem::NotResponding => "Not responding",
            DaemonProblem::Broken(_) => "Needs attention",
            DaemonProblem::UpdateNeeded(_) => "Update needed",
        }
    }

    /// A whole sentence for a dialog or a status page.
    pub fn sentence(&self) -> String {
        match self {
            DaemonProblem::NotRunning => "Fermix is not running.".into(),
            DaemonProblem::NotResponding => "Fermix is not responding.".into(),
            DaemonProblem::Broken(_) => {
                "Fermix answered in a way this app does not understand.".into()
            }
            DaemonProblem::UpdateNeeded(sentence) => sentence.clone(),
        }
    }
}

pub fn daemon_problem(err: &CallError) -> DaemonProblem {
    match err {
        CallError::DaemonDown(ErrorKind::NotFound) => DaemonProblem::NotRunning,
        CallError::DaemonDown(_) | CallError::Timeout | CallError::Io(_) => {
            DaemonProblem::NotResponding
        }
        CallError::Refused(refusal) => refusal_problem(refusal),
        CallError::Protocol(detail) => DaemonProblem::Broken(detail.clone()),
    }
}

fn refusal_problem(refusal: &Refusal) -> DaemonProblem {
    match refusal.code.as_str() {
        "client_too_old" => DaemonProblem::UpdateNeeded(APP_TOO_OLD.into()),
        "daemon_too_old" | "method_not_found" => DaemonProblem::UpdateNeeded(DAEMON_TOO_OLD.into()),
        _ => DaemonProblem::Broken(refusal.sentence.clone()),
    }
}

/// Whether this daemon can serve this app at all, read from `hello` before
/// anything else is asked of it.
pub fn hello_problem(hello: &Hello) -> Option<DaemonProblem> {
    let too_old = |sentence: &str| Some(DaemonProblem::UpdateNeeded(sentence.into()));
    if PROTOCOL_VERSION < hello.protocol.minimum_version {
        return too_old(APP_TOO_OLD);
    }
    if PROTOCOL_VERSION > hello.protocol.maximum_version {
        return too_old(DAEMON_TOO_OLD);
    }
    match hello.capabilities.minimum_versions.get("setup.state.get") {
        None => too_old(DAEMON_TOO_OLD),
        Some(needs) if *needs > PROTOCOL_VERSION => too_old(APP_TOO_OLD),
        Some(_) => None,
    }
}
