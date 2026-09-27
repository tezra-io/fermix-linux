//! What the Voice page says before a call can begin (spec_voice §4.3): exactly
//! one sentence, the first that applies, from what the daemon reports, what the
//! last connection attempt found and which microphone the sound server offers.
//! Nothing is probed or inferred here. Also the line under the mascot and the
//! mascot's expression, from the session.

use crate::mascot::Expression;
use crate::model::SetupState;
use crate::overview::RealtimeFacts;
use crate::realtime::client::ConnectError;
use crate::realtime::protocol::Direction;
use crate::realtime::session::{Mode, Palette, Session};

/// The readiness failure a missing OpenAI key raises.
const KEY_FAILURE: &str = "realtime:openai";
/// Said before a call when the sound server offers no microphone, and after one
/// whose recording could find none.
pub const NO_MICROPHONE: &str = "No microphone is connected.";
/// Said when the microphone a call records from went away and another input took its place:
/// the call ends rather than record from an input nobody chose.
pub const MICROPHONE_LOST: &str = "The microphone was disconnected, so the call ended.";

/// What the last attempt to reach `realtime.sock` found.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reach {
    Untried,
    /// ENOENT or ECONNREFUSED: the daemon has not opened its voice socket.
    NoSocket,
    ClientTooOld,
    ClientTooNew,
    /// `max_clients_reached`.
    Busy,
}

impl Reach {
    /// What a failed connection says about reaching voice. Anything the gate
    /// has no row for stays `Untried`: the session's own sentence covers it.
    pub fn from_connect(error: &ConnectError) -> Reach {
        match error {
            ConnectError::NotFound | ConnectError::Refused => Reach::NoSocket,
            ConnectError::Rejected(refusal) => match refusal.direction {
                Some(Direction::ClientTooOld) => Reach::ClientTooOld,
                Some(Direction::ClientTooNew) => Reach::ClientTooNew,
                _ if refusal.reason == "max_clients_reached" => Reach::Busy,
                _ => Reach::Untried,
            },
            ConnectError::Timeout | ConnectError::Io(_) => Reach::Untried,
        }
    }
}

/// One input the sound server lists, as the app's device watch reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    /// A copy of an output (the sound server's "monitor" class), not a microphone.
    pub monitor: bool,
    /// The sound server's default input, which a call records from.
    pub default: bool,
}

/// The microphone a call would record from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Microphone {
    /// No list yet, or the sound server did not give one: nothing is claimed,
    /// and a call's own attempt says what it finds.
    #[default]
    Unknown,
    /// The sound server offers no input but copies of its outputs.
    Missing,
    /// The input a call records from, by the sound server's name for it.
    Named(String),
}

impl Microphone {
    /// From the whole list. A call records from the default input, even when that
    /// is a copy of an output, so that one is named; a list without a default
    /// names its first microphone.
    pub fn from_sources(sources: &[Source]) -> Microphone {
        let mut microphones = sources.iter().filter(|s| !s.monitor);
        let Some(first) = microphones.next() else {
            return Microphone::Missing;
        };
        let chosen = sources.iter().find(|s| s.default).unwrap_or(first);
        Microphone::Named(chosen.name.clone())
    }

    /// The Voice page's Microphone row: the device, or plainly none or unknown.
    pub fn label(&self) -> &str {
        match self {
            Microphone::Named(name) => name,
            Microphone::Missing => "None",
            Microphone::Unknown => "Unknown",
        }
    }

    /// Whether a call recording from this microphone has lost it in `sources`: the named one
    /// is no longer listed, even with another input left, or the list names none at all.
    pub fn lost_in(&self, sources: &[Source]) -> bool {
        match self {
            Microphone::Named(name) => !sources.iter().any(|s| s.name == *name),
            Microphone::Unknown => Microphone::from_sources(sources) == Microphone::Missing,
            Microphone::Missing => false,
        }
    }
}

/// How a call whose microphone was lost ends once the grace period is over: not at all when
/// `sources` lists that microphone again; otherwise with why.
pub fn lost_microphone_sentence(recorded: &Microphone, sources: &[Source]) -> Option<&'static str> {
    if !recorded.lost_in(sources) {
        return None;
    }
    match Microphone::from_sources(sources) {
        Microphone::Missing => Some(NO_MICROPHONE),
        Microphone::Named(_) | Microphone::Unknown => Some(MICROPHONE_LOST),
    }
}

/// What closing the main window does. Windows are the presence model (spec_voice §1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainWindowClose {
    /// The companion is on screen: the main window only hides, and a call carries on.
    Hide,
    /// It was the last window on screen: Fermix quits, which ends any call. The companion's
    /// window is still there while hidden, so closing alone would leave Fermix running unseen.
    Quit,
}

pub fn main_window_close(companion_shown: bool) -> MainWindowClose {
    if companion_shown {
        MainWindowClose::Hide
    } else {
        MainWindowClose::Quit
    }
}

/// The one thing the page offers to fix what it says. Its button says what to
/// do, so the sentence beside it only says what is so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GateAction {
    StartFermix,
    /// Turn `realtime_enabled` on.
    TurnOn,
    /// Store the OpenAI API key.
    AddKey,
    Restart,
}

pub struct VoiceFacts<'a> {
    /// `setup.state.get`, or `None` when the daemon does not answer.
    pub state: Option<&'a SetupState>,
    /// `overview.realtime`, when the overview answered and reported it.
    pub realtime: Option<&'a RealtimeFacts>,
    pub reach: Reach,
    pub microphone: &'a Microphone,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceGate {
    /// One short sentence, said once, under the mascot.
    pub sentence: String,
    pub action: Option<GateAction>,
    /// Whether "Begin voice call" may be pressed.
    pub ready: bool,
}

/// The line under the mascot: a mode's word beside its icon, or a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    /// The mode's icon. A sentence (what stands in the way, or why the last
    /// call failed) has none: its words say it.
    pub icon: Option<&'static str>,
    pub palette: Palette,
}

fn blocked(sentence: &str, action: Option<GateAction>) -> VoiceGate {
    VoiceGate {
        sentence: sentence.to_owned(),
        action,
        ready: false,
    }
}

/// The first row that applies, in the order spec_voice §4.3 gives. A voice
/// change waiting for a restart comes before "Voice is off", because the daemon
/// only reads voice settings as it boots.
pub fn voice_gate(facts: &VoiceFacts<'_>) -> VoiceGate {
    let Some(state) = facts.state else {
        return blocked("Fermix is not running.", Some(GateAction::StartFermix));
    };
    if let Some(sentence) = restart_for(state, "realtime") {
        return blocked(sentence, Some(GateAction::Restart));
    }
    let enabled = facts.realtime.map_or(state.features.voice, |r| r.enabled);
    if !enabled {
        return blocked("Voice is off.", Some(GateAction::TurnOn));
    }
    if state
        .readiness
        .failures
        .iter()
        .any(|f| f.component == KEY_FAILURE)
    {
        return blocked(KEY_SENTENCE, Some(GateAction::AddKey));
    }
    if let Some(sentence) = restart_for(state, "providers") {
        return blocked(sentence, Some(GateAction::Restart));
    }
    let degraded = facts.realtime.is_some_and(|r| r.status == "degraded");
    if degraded || facts.reach == Reach::NoSocket {
        let closed = "Fermix has not opened its voice connection.";
        return blocked(closed, Some(GateAction::Restart));
    }
    if let Some(sentence) = reach_sentence(facts.reach) {
        return blocked(sentence, None);
    }
    // Only the person can plug one in; the row updates when they do.
    if *facts.microphone == Microphone::Missing {
        return blocked(NO_MICROPHONE, None);
    }
    VoiceGate {
        sentence: "Ready".into(),
        action: None,
        ready: true,
    }
}

fn reach_sentence(reach: Reach) -> Option<&'static str> {
    match reach {
        Reach::ClientTooOld => Some("Update this app to talk to this version of Fermix."),
        Reach::ClientTooNew => Some("Update Fermix to talk to this app."),
        Reach::Busy => Some("Four other voice clients are already connected to Fermix."),
        Reach::Untried | Reach::NoSocket => None,
    }
}

fn restart_for<'a>(state: &'a SetupState, section: &str) -> Option<&'a str> {
    state
        .restart
        .reasons
        .iter()
        .find(|r| r.section.as_deref() == Some(section))
        .map(|r| r.sentence.as_str())
}

/// What the key is for, in short; Home's attention row for the same failure
/// explains at length. The caveat stays: a sign-in looks like it should cover voice.
const KEY_SENTENCE: &str =
    "Voice needs an OpenAI API key. A Codex or Claude sign-in does not cover it.";

/// The line under the mascot. Before a call, what stands in the way is said
/// here and nowhere else, as a plain sentence in the faint palette: a setup
/// state, not an alarm. A call that is up shows what it is doing, whatever the
/// gate says; a failed one shows why, in the session's own sentence. A call
/// that ended for want of a microphone stops saying so once the list names one
/// again, since the next call would record from it.
pub fn call_status(session: &Session, gate: &VoiceGate, microphone: &Microphone) -> StatusLine {
    if !gate.ready && !session.in_call() {
        return StatusLine {
            text: gate.sentence.clone(),
            icon: None,
            palette: Palette::Faint,
        };
    }
    let microphone_back =
        session.error() == Some(NO_MICROPHONE) && matches!(microphone, Microphone::Named(_));
    if session.mode() == Mode::Offline || microphone_back {
        return StatusLine {
            text: "Ready".into(),
            icon: Some("call-start-symbolic"),
            palette: Palette::Secondary,
        };
    }
    let status = session.status();
    // In the error mode the word is a whole sentence.
    let icon = (status.palette != Palette::Error).then_some(status.icon);
    StatusLine {
        text: status.label,
        icon,
        palette: status.palette,
    }
}

/// The mascot's face for what voice is doing.
pub fn expression(mode: Mode) -> Expression {
    match mode {
        Mode::Listening => Expression::Listening,
        Mode::Thinking | Mode::ToolUse | Mode::Connecting | Mode::Reconnecting => {
            Expression::Thinking
        }
        Mode::Speaking => Expression::Speaking,
        Mode::Offline | Mode::Idle | Mode::Muted | Mode::Error => Expression::Idle,
    }
}
