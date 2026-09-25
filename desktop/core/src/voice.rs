//! What the Voice page says before a call can begin (spec_voice §4.3): exactly
//! one row, the first that applies, from what the daemon reports and what the
//! last connection attempt found. Nothing is probed or inferred here. Also the
//! call's word and the mascot's expression, from the session.

use crate::mascot::Expression;
use crate::model::SetupState;
use crate::overview::RealtimeFacts;
use crate::realtime::client::ConnectError;
use crate::realtime::protocol::Direction;
use crate::realtime::session::{Mode, Palette, Session, Status};
use crate::view::fixed_attention_copy;

/// The readiness failure a missing OpenAI key raises.
const KEY_FAILURE: &str = "realtime:openai";

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

/// The one thing the page offers to fix what it says.
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceGate {
    pub sentence: String,
    pub action: Option<GateAction>,
    /// Whether "Begin voice call" may be pressed.
    pub ready: bool,
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
        let off = "Voice is off. Turn it on to talk to Fermix from this app.";
        return blocked(off, Some(GateAction::TurnOn));
    }
    if state
        .readiness
        .failures
        .iter()
        .any(|f| f.component == KEY_FAILURE)
    {
        return blocked(&key_sentence(), Some(GateAction::AddKey));
    }
    if let Some(sentence) = restart_for(state, "providers") {
        return blocked(sentence, Some(GateAction::Restart));
    }
    let degraded = facts.realtime.is_some_and(|r| r.status == "degraded");
    if degraded || facts.reach == Reach::NoSocket {
        let closed = "Voice is on, but Fermix has not opened its voice connection. Restarting \
                      Fermix usually fixes this.";
        return blocked(closed, Some(GateAction::Restart));
    }
    reach_gate(facts.reach)
}

fn reach_gate(reach: Reach) -> VoiceGate {
    let sentence = match reach {
        Reach::ClientTooOld => "Update this app to talk to this version of Fermix.",
        Reach::ClientTooNew => "Update Fermix to talk to this app.",
        Reach::Busy => "Four other voice clients are already connected to Fermix.",
        Reach::Untried | Reach::NoSocket => {
            return VoiceGate {
                sentence: "Ready".into(),
                action: None,
                ready: true,
            }
        }
    };
    blocked(sentence, None)
}

fn restart_for<'a>(state: &'a SetupState, section: &str) -> Option<&'a str> {
    state
        .restart
        .reasons
        .iter()
        .find(|r| r.section.as_deref() == Some(section))
        .map(|r| r.sentence.as_str())
}

/// Home's words for the same failure, and what to do about it.
fn key_sentence() -> String {
    let (_, body) = fixed_attention_copy(KEY_FAILURE).expect("Home has copy for the key failure");
    format!("{body} Add an OpenAI API key to talk to Fermix.")
}

/// The word under the mascot. Where the gate has the reason (`reachable` is
/// false), the word stays short; before a call it rests on the session's ready.
pub fn call_status(session: &Session, reachable: bool) -> Status {
    if !reachable {
        // True of every gate row, and it does not compete with the row's sentence.
        return Status {
            label: "Unavailable".into(),
            icon: "action-unavailable-symbolic",
            palette: Palette::Faint,
        };
    }
    if session.mode() == Mode::Offline {
        return Status {
            label: "Ready".into(),
            icon: "call-start-symbolic",
            palette: Palette::Secondary,
        };
    }
    session.status()
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
