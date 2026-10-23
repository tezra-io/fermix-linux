//! The chat connection's life, as values: the `client_hello` / `server_hello` handshake, the
//! bounded backoff between attempts, and the sentence the page shows for each state. The app owns
//! the socket and the clock and asks this what to do.
//!
//! It connects when chat is first asked for and then stays connected: a connection that fails
//! is tried again after `delay`, forever, at most twice a minute once the backoff tops out. Only a
//! daemon that cannot speak this build's version ends that, until chat is asked for again
//! (macOS `CompanionSession`).

use crate::chat_wire::protocol::{Direction, ServerEvent, PROTOCOL_VERSION};
use std::time::Duration;

/// How long each consecutive failure waits; the last delay repeats.
pub const RECONNECT_DELAYS_S: [u64; 6] = [1, 2, 4, 8, 16, 30];
/// How long the daemon has to answer `client_hello`.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

/// The error reasons after which the daemon closes the connection (PROTOCOL.md "Errors").
const CLOSING_REASONS: [&str; 15] = [
    "invalid_json",
    "invalid_event",
    "missing_type",
    "unknown_event",
    "missing_field",
    "invalid_field",
    "attachments_unsupported",
    "line_too_large",
    "handshake_required",
    "unexpected_client_hello",
    "missing_protocol_version",
    "invalid_protocol_version",
    "unsupported_protocol_version",
    "max_clients_reached",
    "unidentified_client",
];

// The macOS app's ProductStrings wording (`companion.connection.*`), "this Mac" made "this computer".
const UPDATE_APP: &str =
    "This app is older than the Fermix engine it talks to. Update the app to chat.";
const UPDATE_FERMIX: &str = "The Fermix engine is older than this app. Update the engine to chat.";
/// What the page says for `Shown::NoChat`.
pub const NO_CHAT: &str = "The Fermix engine on this computer has no chat yet";
const RECONNECTING: &str = "Chat cannot reach Fermix right now and is trying again";

/// Whether the daemon closes the connection after an error with this reason.
pub fn closes(reason: &str) -> bool {
    CLOSING_REASONS.contains(&reason)
}

/// How long to wait after `failures` consecutive failures (0 for the first).
pub fn delay(failures: u32) -> Duration {
    let last = RECONNECT_DELAYS_S.len() - 1;
    let index = usize::try_from(failures).map_or(last, |n| n.min(last));
    Duration::from_secs(RECONNECT_DELAYS_S[index])
}

/// What one event read while waiting for `server_hello` means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handshake {
    Ok,
    /// The two sides cannot talk at this version; the sentence says which to update. No retry.
    Refused(String),
    /// Any other refusal: the connection is over, and the next attempt may work.
    Lost(String),
    /// Not a handshake answer: log it and keep waiting, the deadline still running.
    Ignore,
}

/// The handshake outcome of `event`, for this build's `PROTOCOL_VERSION`.
pub fn handshake(event: &ServerEvent) -> Handshake {
    match event {
        ServerEvent::ServerHello {
            min_version,
            max_version,
        } => window(*min_version, *max_version),
        ServerEvent::Error(error) => match &error.direction {
            Some(direction) => Handshake::Refused(refusal(direction)),
            None => Handshake::Lost(error.reason.clone()),
        },
        _ => Handshake::Ignore,
    }
}

fn window(min: u64, max: u64) -> Handshake {
    if PROTOCOL_VERSION < min {
        return Handshake::Refused(refusal(&Direction::ClientTooOld));
    }
    if PROTOCOL_VERSION > max {
        return Handshake::Refused(refusal(&Direction::ClientTooNew));
    }
    Handshake::Ok
}

/// The sentence for a version refusal. A direction this build does not know names both sides.
pub fn refusal(direction: &Direction) -> String {
    match direction {
        Direction::ClientTooOld => UPDATE_APP.into(),
        Direction::ClientTooNew => UPDATE_FERMIX.into(),
        Direction::Other(word) => {
            format!("The daemon reported: {word}")
        }
    }
}

/// Why a connection ended or never began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lost {
    /// No `companion.sock` file: an engine that predates chat.
    NoSocket,
    /// Anything else; the detail is for the log.
    Broken(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    Connecting,
    Handshaking,
    Connected,
    /// Waiting out the backoff before the next attempt.
    Waiting,
    /// The daemon cannot speak this build's version. Nothing is retried.
    Refused(String),
}

/// What the page says about the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    Idle,
    /// The first attempt since chat was asked for.
    Connecting,
    Connected,
    NoChat,
    Disconnected(String),
    Refused(String),
}

/// The connection's phase and its run of failures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    phase: Phase,
    failures: u32,
    /// Why the last attempt failed, kept while later attempts run.
    last_lost: Option<Lost>,
}

impl Session {
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// Begins an attempt; false when one is running or connected. Chat asked for after a
    /// refusal tries again: a new daemon may speak this version.
    pub fn start(&mut self) -> bool {
        if matches!(self.phase, Phase::Refused(_)) {
            *self = Session::default();
        }
        if !matches!(self.phase, Phase::Idle | Phase::Waiting) {
            return false;
        }
        self.phase = Phase::Connecting;
        true
    }

    /// The socket connected and `client_hello` is on its way. False when the attempt ended
    /// while the connect ran: the caller closes that socket.
    pub fn connected(&mut self) -> bool {
        if self.phase != Phase::Connecting {
            return false;
        }
        self.phase = Phase::Handshaking;
        true
    }

    /// An event read during the handshake. `Ok` connects and resets the backoff; `Refused`
    /// ends the attempts. The caller ends a `Lost` connection with `lost`.
    pub fn hello(&mut self, event: &ServerEvent) -> Handshake {
        let outcome = handshake(event);
        match &outcome {
            Handshake::Ok => {
                self.phase = Phase::Connected;
                self.failures = 0;
                self.last_lost = None;
            }
            Handshake::Refused(sentence) => self.phase = Phase::Refused(sentence.clone()),
            Handshake::Lost(_) | Handshake::Ignore => {}
        }
        outcome
    }

    /// The connection ended or never began; returns how long to wait before `start` again.
    /// `None` when no attempt was running: a refusal's own close, or a second report of one
    /// loss, changes nothing.
    pub fn lost(&mut self, why: Lost) -> Option<Duration> {
        let running = matches!(
            self.phase,
            Phase::Connecting | Phase::Handshaking | Phase::Connected
        );
        if !running {
            return None;
        }
        let wait = delay(self.failures);
        self.failures = self.failures.saturating_add(1);
        self.last_lost = Some(why);
        self.phase = Phase::Waiting;
        Some(wait)
    }

    pub fn shown(&self) -> Shown {
        match (&self.phase, &self.last_lost) {
            (Phase::Idle, _) => Shown::Idle,
            (Phase::Connected, _) => Shown::Connected,
            (Phase::Refused(sentence), _) => Shown::Refused(sentence.clone()),
            (_, None) => Shown::Connecting,
            (_, Some(Lost::NoSocket)) => Shown::NoChat,
            (_, Some(Lost::Broken(_))) => Shown::Disconnected(RECONNECTING.into()),
        }
    }
}
