//! Reach from anywhere (M62): the line the Phones step draws under the rail's switch, from
//! `mobile.status.remote`, and how the step follows it while it is open. The sentences are
//! macOS's, word for word. The line says only whether this computer is connected to Fermix's
//! relays, never which one and never that a phone can reach it (M62 §7).

use crate::mobile::{Remote, RemoteState};

/// The rail's switch in `channels.mobile`, under which the line is drawn.
pub const SWITCH_KEY: &str = "mobile_iroh";

pub const HELPER_MISSING: &str = "Not available in this build yet.";
pub const DORMANT: &str = "Starts when you pair your first phone.";
pub const STARTING: &str = "Starting…";
pub const READY: &str = "Connected to Fermix’s relays.";
pub const NOT_CONNECTED: &str = "Not connected to Fermix’s relays right now.";

/// The most reads one follow makes: five minutes at the contract's cadence while the daemon
/// answers. Past the last, the line stays as last read until the switch moves or the Phones step
/// is shown again.
pub const POLLS: u32 = 300;
/// The contract's cadence for reading the channel, and the first wait of a follow.
pub const POLL_MS: u64 = 1_000;
/// The longest wait between reads while the daemon refuses them, as a restarting one does.
pub const BACKOFF_MS: u64 = 30_000;

/// How the line is drawn: a plain note, a start that moves on by itself, or a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Note,
    Progress,
    Failure,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub kind: Kind,
}

impl Line {
    fn new(text: &str, kind: Kind) -> Line {
        Line {
            text: text.to_owned(),
            kind,
        }
    }
}

/// The line for the rail as last read. A rail that is off says nothing beyond its switch, and
/// neither does a state this app does not know or an engine older than the rail. A failure is
/// the daemon's sentence as written; one with no sentence says nothing.
pub fn line(remote: Option<&Remote>) -> Option<Line> {
    let remote = remote?;
    match remote.state {
        RemoteState::HelperMissing => Some(Line::new(HELPER_MISSING, Kind::Note)),
        RemoteState::Dormant => Some(Line::new(DORMANT, Kind::Note)),
        RemoteState::Starting => Some(Line::new(STARTING, Kind::Progress)),
        RemoteState::Ready if remote.connected => Some(Line::new(READY, Kind::Note)),
        RemoteState::Ready => Some(Line::new(NOT_CONNECTED, Kind::Note)),
        RemoteState::Failed => remote
            .failure
            .as_deref()
            .map(|failure| Line::new(failure, Kind::Failure)),
        RemoteState::Off | RemoteState::Unrecognized => None,
    }
}

/// Whether the rail can move while nobody touches its switch, so the line is followed: starting,
/// up and able to lose its relay, retried after a failure, or a state this app does not know.
/// Off, dormant and a build with no rail move only with the switch or a pairing.
pub fn moves(remote: Option<&Remote>) -> bool {
    match remote.map(|remote| remote.state) {
        Some(
            RemoteState::Starting
            | RemoteState::Ready
            | RemoteState::Failed
            | RemoteState::Unrecognized,
        ) => true,
        Some(RemoteState::Off | RemoteState::Dormant | RemoteState::HelperMissing) | None => false,
    }
}

/// One follow of the rail: the reads it made, against `POLLS`, and the refusals in a row since
/// the last answer. The wait doubles with each refusal up to `BACKOFF_MS`, so a restarting daemon
/// is not asked once a second, and only the first refusal of a run and the answer that ends it
/// are worth a log line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Follow {
    reads: u32,
    refusals: u32,
}

impl Follow {
    pub fn exhausted(&self) -> bool {
        self.reads >= POLLS
    }

    pub fn wait_ms(&self) -> u64 {
        let doubled = POLL_MS.saturating_mul(1u64 << self.refusals.min(16));
        doubled.min(BACKOFF_MS)
    }

    /// An answer. Returns the length of the run of refusals it ends, where it ends one.
    pub fn answered(&mut self) -> Option<u32> {
        self.reads += 1;
        let ended = std::mem::take(&mut self.refusals);
        (ended > 0).then_some(ended)
    }

    /// A refusal. True when it starts a run.
    pub fn refused(&mut self) -> bool {
        self.reads += 1;
        self.refusals += 1;
        self.refusals == 1
    }
}

/// Reads of `mobile.status` run side by side and can answer out of order. Each read takes a
/// number when it is asked, and an answer lands only if no later-asked read has landed first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Newest {
    asked: u64,
    landed: u64,
}

impl Newest {
    pub fn ask(&mut self) -> u64 {
        self.asked += 1;
        self.asked
    }

    /// Whether the answer to read `number` lands; if it does, older ones no longer can.
    pub fn lands(&mut self, number: u64) -> bool {
        if number <= self.landed {
            return false;
        }
        self.landed = number;
        true
    }
}
