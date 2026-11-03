//! Meetings and Computer: the notetaker's sign-in state and the computer-use
//! helper's probe. Both are the daemon's answers, shown as they are: an
//! unanswered question is never read as "no" (M38 §5.7, §8.6).

use crate::doctor::Tone;
use crate::management::{CallError, Management};
use crate::model::{DetectRow, JobView};
use crate::update::view::{note_spans, Span};
use serde::Deserialize;
use serde_json::json;

pub const MEETBOT: &str = "meetbot";
pub const COMPUTER_SIDECAR: &str = "computer_use_sidecar";

/// Above the notetaker's Google account. Its sign-in opens the helper's own Chromium on its own
/// profile, never the user's browser, and it joins meetings as whoever signs in there
/// (macOS says the same in settings.meetings.signInNotice).
pub const MEETBOT_SIGN_IN_NOTE: &str = "Signing in opens the notetaker's own browser, separate \
    from yours, so it starts signed out. Use the Google account the notetaker should join \
    meetings as: it appears under that account's name, so an account made for it is best.";

/// On Linux nothing is granted: `screen_capture` and `input_control` say whether the helper can
/// take screenshots, and move the pointer and press keys, in this session.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ComputerPermissions {
    pub installed: bool,
    pub screen_capture: bool,
    pub input_control: bool,
    /// When the helper last probed; null until it has.
    pub probed_at: Option<String>,
    /// `linux` or `macos` once the helper was probed; absent from older daemons.
    #[serde(default)]
    pub platform: Option<String>,
    /// Linux only: the session's display, `x11`, `wayland`, `none`, or another word. It never
    /// names the pixel route: only the `pixel route` row does.
    #[serde(default)]
    pub display_server: Option<String>,
    /// Linux only: the rows `fermix doctor` prints for computer use.
    #[serde(default)]
    pub rows: Option<Vec<ProbeRow>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ProbeRow {
    pub name: String,
    pub status: RowStatus,
    /// A sentence fragment, shown as written; a backticked span is a command.
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    Ok,
    Warn,
    Fail,
    NotApplicable,
    /// A word this app has never seen.
    #[serde(other)]
    Unknown,
}

impl RowStatus {
    /// A word never seen is not vouched for, so it asks for a look too.
    pub fn asks_for_a_look(self) -> bool {
        matches!(self, RowStatus::Warn | RowStatus::Fail | RowStatus::Unknown)
    }
}

/// One thing computer use can do in this session, with its state word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ability {
    pub title: &'static str,
    /// A daemon detail's backticked span in it is inline code, as in a note.
    pub subtitle: Vec<Span>,
    pub state: &'static str,
    pub tone: Tone,
}

/// One of the helper's rows, shown below the abilities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeNote {
    pub title: String,
    pub detail: Vec<Span>,
    /// It carries a command the person may copy. The app never runs it.
    pub selectable: bool,
    pub attention: bool,
}

/// The Linux Computer pane's probe group, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxProbe {
    pub abilities: Vec<Ability>,
    pub notes: Vec<ProbeNote>,
    /// When the helper last checked, once.
    pub checked: Option<String>,
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

/// A probe time as the pane shows it: `2026-08-19T12:00:00Z` reads `2026-08-19 12:00:00`.
pub fn checked_words(at: &str) -> String {
    at.replace('T', " ").trim_end_matches('Z').to_owned()
}

const BUS: &str = "accessibility bus";
const ROUTE: &str = "pixel route";
const PIXELS: &str = "Screenshots, pointer and keys";
const WINDOW: &str = "Work inside one window";
const CONTROLS_ALONE: &str = "Fermix works by controls alone on this desktop.";

/// What a Linux helper can do in this session, drawn from its rows. `None` for a macOS
/// answer, an older daemon's, or one without rows: the pane then shows the verdicts.
pub fn linux_probe(permissions: &ComputerPermissions) -> Option<LinuxProbe> {
    if permissions.platform.as_deref() != Some("linux") {
        return None;
    }
    let rows = permissions.rows.as_deref()?;
    let server = permissions.display_server.as_deref();
    let bus = rows.iter().find(|r| r.name == BUS);
    let route = rows.iter().find(|r| r.name == ROUTE);
    let by_control = bus.is_some_and(|b| b.status == RowStatus::Ok);
    // On Wayland the portal is the route when its row is ok, and may be when an engine from
    // before the route row sends none.
    let portal_route = server == Some("wayland") && route.is_none_or(|r| r.status == RowStatus::Ok);
    Some(LinuxProbe {
        abilities: vec![
            controls(bus),
            pixels(permissions, route, by_control),
            one_window(permissions),
        ],
        notes: rows
            .iter()
            .filter(|r| is_note(r, portal_route))
            .map(note)
            .collect(),
        checked: permissions.probed_at.as_deref().map(checked_words),
    })
}

fn ability(title: &'static str, subtitle: &str, state: &'static str, tone: Tone) -> Ability {
    Ability {
        title,
        subtitle: note_spans(subtitle),
        state,
        tone,
    }
}

/// Controls work through the accessibility bus; its row is the only one that says so.
fn controls(bus: Option<&ProbeRow>) -> Ability {
    let Some(bus) = bus else {
        let missing = "The helper did not report its accessibility bus.";
        return ability("Controls", missing, "Unknown", Tone::Quiet);
    };
    let subtitle = format!("Accessibility bus: {}", bus.detail);
    match bus.status {
        RowStatus::Ok => ability("Controls", &subtitle, "Available", Tone::Good),
        _ => ability("Controls", &subtitle, "Not available", Tone::Warn),
    }
}

/// What the helper can do with pixels in this session, as its two booleans say. The route is
/// named only as the `pixel route` row names it, never from the display server.
fn pixels(
    permissions: &ComputerPermissions,
    route: Option<&ProbeRow>,
    by_control: bool,
) -> Ability {
    match (permissions.screen_capture, permissions.input_control) {
        (true, true) => ability(PIXELS, &route_line(route), "Available", Tone::Good),
        (false, false) => no_pixels(route, by_control),
        (true, false) => {
            let half = "Screenshots work, but the pointer and keys do not.";
            ability(PIXELS, half, "Partly", Tone::Warn)
        }
        // A point comes only from a screenshot. No route line: the portal is the route, but a
        // stored no keeps it from any screenshot until it is reset.
        (false, true) => {
            let half = "Keys and typing work, but screenshots do not, so Fermix cannot see \
                        where to click.";
            ability(PIXELS, half, "Partly", Tone::Warn)
        }
    }
}

/// The `pixel route` row as written; an engine from before that row sends none, and then nothing
/// is said.
fn route_line(route: Option<&ProbeRow>) -> String {
    route
        .map(|r| format!("Pixel route: {}", r.detail))
        .unwrap_or_default()
}

/// Working by controls alone is a normal mode, so no pixels is quiet while controls work, and
/// without them nothing works, which is said without claiming controls. A route row that is not
/// ok says why. A route row that is ok names a route the helper took and could not use (on X11,
/// an X server the probe could not reach): a fault either way, and the X11 pixels note says why.
fn no_pixels(route: Option<&ProbeRow>, by_control: bool) -> Ability {
    let tone = if by_control { Tone::Quiet } else { Tone::Warn };
    let nothing = "They do not work in this session.";
    let (line, tone) = match route {
        Some(route) if route.status != RowStatus::Ok => (route_line(Some(route)), tone),
        Some(_) => (nothing.to_owned(), Tone::Warn),
        None if by_control => (CONTROLS_ALONE.to_owned(), tone),
        None => (nothing.to_owned(), tone),
    };
    ability(PIXELS, &line, "Not available", tone)
}

/// Binding one window can happen under X11 alone, with both halves. It also needs its switch and
/// the helper's own support, which this answer does not carry, so X11 says where it can work,
/// never that it does.
fn one_window(permissions: &ComputerPermissions) -> Ability {
    let x11 = permissions.display_server.as_deref() == Some("x11");
    let both = permissions.screen_capture && permissions.input_control;
    match (x11, both) {
        (true, true) => {
            let switch = "Its switch is above. A badge marks the window Fermix works in.";
            ability(WINDOW, switch, "On X11", Tone::Quiet)
        }
        (true, false) => {
            let needs = "It needs the screenshots, pointer and keys above.";
            ability(WINDOW, needs, "Not available", Tone::Quiet)
        }
        (false, _) => ability(WINDOW, "Only on X11.", "Not available", Tone::Quiet),
    }
}

/// The ATK bridge and toolkit notes always; where the portal is or may be the route, the stored
/// screenshot answer, which says whether and when the desktop asks; elsewhere that answer when it
/// carries its reset; any other row that asks for a look. The bus is already the Controls line,
/// and the pixel route the Screenshots line.
fn is_note(row: &ProbeRow, portal_route: bool) -> bool {
    match row.name.as_str() {
        BUS | ROUTE => false,
        "atk bridge" => true,
        "portal screenshot" => {
            portal_route || carries_command(&row.detail) || row.status.asks_for_a_look()
        }
        name => name.starts_with("toolkit ") || row.status.asks_for_a_look(),
    }
}

fn carries_command(detail: &str) -> bool {
    note_spans(detail)
        .iter()
        .any(|span| matches!(span, Span::Code(_)))
}

fn note(row: &ProbeRow) -> ProbeNote {
    ProbeNote {
        title: note_title(&row.name),
        detail: note_spans(&row.detail),
        selectable: carries_command(&row.detail),
        attention: row.status.asks_for_a_look(),
    }
}

/// A row's name as a title; a name this app does not know is shown as given.
fn note_title(name: &str) -> String {
    let title = match name {
        "atk bridge" => "ATK bridge",
        "toolkit qt" => "Qt applications",
        "toolkit chromium" => "Chromium-based applications",
        "portal screenshot" => "Stored screenshot answer",
        "x11 pixels" => "X11 pixels",
        "desktop portal" => "Desktop portal",
        "display server" => "Display server",
        other => other,
    };
    title.to_owned()
}

/// A capability job's phase as the pane shows it. An unknown phase is still work.
pub fn phase_words(phase: Option<&str>) -> &'static str {
    match phase {
        Some("sidecar_downloading") => "Downloading the helper…",
        Some("downloading") => "Downloading the notetaker's browser…",
        Some("awaiting_signin") => "Finish signing in to Google in the notetaker's browser window.",
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
