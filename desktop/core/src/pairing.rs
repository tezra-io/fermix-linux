//! The Phone dialog's steps as a reducer over the daemon's answers (M60 §3.3, §3.4), the guards
//! an answer is held to before anything is drawn, and the code drawn from the pairing link.
//!
//! It derives nothing the daemon publishes: which state a window is in, whether the channel runs
//! and how a window ended are all read. The words it adds are the app's over the daemon's facts,
//! switched on the published state and reason and never on a verb word.

use crate::mobile::{
    MobileStatus, OutcomeReason, PairingSession, PairingStart, PairingSummary, SessionState,
};
use crate::phone::{
    ENDED_CANCELLED, ENDED_DENIED, ENDED_ELSEWHERE, ENDED_EXPIRED, ENDED_UNREADABLE, PAIR_AGAIN,
    RESTART, START_OVER, TURN_ON_AND_RESTART,
};
use std::fmt;

/// The pairing link the daemon hands back once, held by the Scan step and nowhere else, so it
/// leaves memory when Scan does. It carries the phone's one-time secret: never logged, never
/// written anywhere, and never an accessible value. No print of it spells it.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingLink(String);

impl PairingLink {
    /// The link as the person reads and pastes it, behind "Can't scan the code?".
    pub fn text(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PairingLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingLink(withheld)")
    }
}

/// What an answer is held to before anything is drawn: the guards `fermix pair` applies (§3.4).
/// An answer that fails one ends the window with the app's sentence rather than drawing a code or
/// a phone this app cannot vouch for.
pub mod guards {
    use super::PairingLink;

    pub const LINK_PREFIX: &str = "fermix://pair?";
    pub const MAX_LINK_BYTES: usize = 2048;
    pub const MAX_TTL_MS: i64 = 120_000;
    pub const DIGITS: usize = 6;
    pub const MAX_FIELD_BYTES: usize = 128;

    /// The link, held to its prefix, its length and its characters. Its version is never read:
    /// the link is the phone's to parse.
    pub fn link(value: &str) -> Option<PairingLink> {
        let shaped = value.starts_with(LINK_PREFIX)
            && value.len() <= MAX_LINK_BYTES
            && !value.chars().any(char::is_control);
        shaped.then(|| PairingLink(value.to_owned()))
    }

    /// The window's lifetime as it opens, which `fermix pair` holds to 1 to 120000 ms.
    pub fn ttl(value: Option<i64>) -> Option<i64> {
        value.filter(|ms| (1..=MAX_TTL_MS).contains(ms))
    }

    /// What is left of an open window on a later read. The daemon counts it down to zero before
    /// it says the window expired, so a read in that last moment can carry zero.
    pub fn remaining(value: Option<i64>) -> Option<i64> {
        value.filter(|ms| (0..=MAX_TTL_MS).contains(ms))
    }

    /// Six ASCII digits, which is what the phone draws.
    pub fn digits(value: &str) -> bool {
        value.len() == DIGITS && value.bytes().all(|b| b.is_ascii_digit())
    }

    /// A phone's name or model: present, inside the pairing intake's bound, and nothing that draws
    /// text of its own. The phone writes both, and Compare draws them above the digits the owner
    /// approves by, so a character that breaks the line, reorders the words around it or hides
    /// itself is refused rather than drawn.
    pub fn field(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= MAX_FIELD_BYTES
            && !value.chars().any(|c| c.is_control() || draws_unseen(c))
    }

    /// A line or paragraph separator, a bidirectional control, or an invisible character that
    /// joins nothing. The zero-width joiner and non-joiner stay allowed: emoji and several
    /// scripts are spelled with them.
    fn draws_unseen(c: char) -> bool {
        matches!(
            c,
            '\u{061c}'
                | '\u{200b}'
                | '\u{200e}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
    }
}

/// A pairing link as a QR code's modules, before anything is scaled (§3.5): the link's bytes at
/// correction level M, the level `fermix pair` draws and the highest a 2048-byte link fits.
/// The modules are the link in another form, so no print of them spells them either.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode {
    width: usize,
    /// Row by row, top first; true where a module is dark.
    modules: Vec<bool>,
}

impl PairingCode {
    pub const QUIET_ZONE: usize = 4;
    pub const MIN_MODULE_PX: usize = 2;
    /// The side a short link's card is drawn at, near enough.
    pub const PREFERRED_SIDE: usize = 240;
    /// The widest card: a version 40 code at the smallest module.
    pub const MAX_SIDE: usize = (177 + 2 * Self::QUIET_ZONE) * Self::MIN_MODULE_PX;

    /// None only where the link does not fit a code, which a link the guards passed always does.
    pub fn make(link: &PairingLink) -> Option<PairingCode> {
        let code =
            qrcode::QrCode::with_error_correction_level(link.text(), qrcode::EcLevel::M).ok()?;
        let modules = code
            .to_colors()
            .into_iter()
            .map(|c| c == qrcode::Color::Dark)
            .collect();
        Some(PairingCode {
            width: code.width(),
            modules,
        })
    }

    /// Modules on a side, without the quiet zone.
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn dark(&self, row: usize, column: usize) -> bool {
        assert!(
            row < self.width && column < self.width,
            "a module inside the code"
        );
        self.modules[row * self.width + column]
    }

    /// Modules on a side, quiet zone included.
    pub fn span(&self) -> usize {
        self.width + 2 * Self::QUIET_ZONE
    }

    /// Whole pixels per module, never fewer than two.
    pub fn module_px(&self) -> usize {
        (Self::PREFERRED_SIDE / self.span()).max(Self::MIN_MODULE_PX)
    }

    /// The card's side in pixels.
    pub fn side(&self) -> usize {
        self.span() * self.module_px()
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PairingCode({} modules, withheld)", self.width)
    }
}

/// The one step the Phone dialog shows, which changes in place (§3.3).
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Waiting on the daemon: reading the channel, opening a window, or resuming the window
    /// `session` names, which a poll then reads.
    Waiting {
        session: Option<String>,
    },
    TurnOn(TurnOn),
    Scan(Scan),
    Compare(Compare),
    Paired {
        name: String,
    },
    Ended(Ending),
    Phones,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Waiting,
    TurnOn,
    Scan,
    Compare,
    Paired,
    Ended,
    Phones,
}

impl Step {
    pub fn kind(&self) -> StepKind {
        match self {
            Step::Waiting { .. } => StepKind::Waiting,
            Step::TurnOn(_) => StepKind::TurnOn,
            Step::Scan(_) => StepKind::Scan,
            Step::Compare(_) => StepKind::Compare,
            Step::Paired { .. } => StepKind::Paired,
            Step::Ended(_) => StepKind::Ended,
            Step::Phones => StepKind::Phones,
        }
    }

    /// The window this step shows, which a poll reads and closing the dialog cancels. None once
    /// the window has ended, so a window that is over is never cancelled.
    pub fn open_session(&self) -> Option<&str> {
        match self {
            Step::Waiting { session } => session.as_deref(),
            Step::Scan(scan) => Some(&scan.session),
            Step::Compare(compare) => Some(&compare.session),
            Step::TurnOn(_) | Step::Paired { .. } | Step::Ended(_) | Step::Phones => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Idle,
    /// The switch is being written.
    Applying,
    /// The app's one restart is running.
    Restarting,
}

/// Turn on: shown while the channel is not running.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOn {
    /// Whether this step throws the switch, or the switch is on and only the restart is owed.
    pub throws_switch: bool,
    pub progress: Progress,
    /// What refused it, as written, where something did.
    pub refusal: Option<String>,
}

impl TurnOn {
    pub fn new(throws_switch: bool) -> TurnOn {
        TurnOn {
            throws_switch,
            progress: Progress::Idle,
            refusal: None,
        }
    }

    pub fn action_title(&self) -> &'static str {
        if self.throws_switch {
            TURN_ON_AND_RESTART
        } else {
            RESTART
        }
    }
}

/// Scan: the code, the link it was drawn from, and the daemon's own clock. Both leave memory with
/// the step, since no later answer carries the link again.
#[derive(Debug, Clone, PartialEq)]
pub struct Scan {
    pub session: String,
    pub link: PairingLink,
    pub code: PairingCode,
    /// The daemon's `ttl_ms`, as the last answer gave it.
    pub ttl_ms: i64,
}

/// Compare: the phone waiting for a decision, as the daemon reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Compare {
    pub session: String,
    pub device_name: String,
    pub model: String,
    pub digits: String,
    /// What the daemon says of the phone's hardware, drawn as written.
    pub hardware: String,
    pub ttl_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EndAction {
    PairAgain,
    /// Cancels the window open somewhere else, where one is named, and opens a new one.
    StartOver {
        session: Option<String>,
    },
}

/// Ended: one sentence and the one way on.
#[derive(Debug, Clone, PartialEq)]
pub struct Ending {
    pub sentence: String,
    pub action: EndAction,
}

impl Ending {
    fn pair_again(sentence: &str) -> Ending {
        Ending {
            sentence: sentence.to_owned(),
            action: EndAction::PairAgain,
        }
    }

    pub fn action_title(&self) -> &'static str {
        match self.action {
            EndAction::PairAgain => PAIR_AGAIN,
            EndAction::StartOver { .. } => START_OVER,
        }
    }
}

/// One answer from the daemon, as the dialog takes it.
#[derive(Debug)]
pub enum Answer {
    /// `mobile.status`, read as pairing begins.
    Status(MobileStatus),
    Started(PairingStart),
    /// `mobile.pair.start` refused `busy`, with the window `mobile.status` named afterwards.
    Busy(Option<PairingSummary>),
    /// A window's view: `mobile.pair.get`, `.decide` or `.cancel`.
    Session(PairingSession),
    /// Any other refusal, in the daemon's own words.
    Refused(String),
}

/// The step an answer leads to, and the window it leaves open that nothing will show, which the
/// dialog cancels so no window waits for a scan (§3.4).
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub step: Step,
    pub abandons: Option<String>,
}

impl Transition {
    fn to(step: Step) -> Transition {
        Transition {
            step,
            abandons: None,
        }
    }
}

pub fn reduce(step: &Step, answer: Answer) -> Transition {
    match answer {
        Answer::Status(status) => opening(&status),
        Answer::Started(start) => started(&start),
        Answer::Busy(pairing) => busy(pairing),
        Answer::Session(session) => read(&session, step),
        Answer::Refused(sentence) => Transition::to(Step::Ended(Ending::pair_again(&sentence))),
    }
}

/// Pairing begins on the channel as it stands: Turn on while it is not running.
fn opening(status: &MobileStatus) -> Transition {
    if !status.started {
        return Transition::to(Step::TurnOn(TurnOn::new(!status.enabled)));
    }
    Transition::to(Step::Waiting { session: None })
}

/// The window the start opened, or the daemon's refusal of it. The link is read here and nowhere
/// else, since no later answer carries it.
fn started(start: &PairingStart) -> Transition {
    let session = &start.session;
    match session.state {
        SessionState::AwaitingScan => match scan_of(start) {
            Some(scan) => Transition::to(Step::Scan(scan)),
            None => unreadable(Some(session)),
        },
        SessionState::Failed => failed(session),
        _ => unreadable(Some(session)),
    }
}

fn scan_of(start: &PairingStart) -> Option<Scan> {
    let session = start.session.session_id.clone()?;
    let ttl_ms = guards::ttl(start.session.ttl_ms)?;
    let link = guards::link(start.uri()?)?;
    let code = PairingCode::make(&link)?;
    Some(Scan {
        session,
        link,
        code,
        ttl_ms,
    })
}

/// A window is open somewhere else. A phone waiting there is resumed here, so it can be compared;
/// a code waiting for a scan cannot be drawn again, since the link is given once.
fn busy(pairing: Option<PairingSummary>) -> Transition {
    match pairing {
        Some(p) if p.state == SessionState::AwaitingDecision => Transition::to(Step::Waiting {
            session: Some(p.session_id),
        }),
        other => elsewhere(other.map(|p| p.session_id)),
    }
}

fn elsewhere(session: Option<String>) -> Transition {
    Transition::to(Step::Ended(Ending {
        sentence: ENDED_ELSEWHERE.to_owned(),
        action: EndAction::StartOver { session },
    }))
}

/// A read of the window this step shows. An answer about any other window is one this step has
/// already left, and changes nothing.
fn read(session: &PairingSession, step: &Step) -> Transition {
    let Some(id) = session.session_id.as_deref() else {
        return Transition::to(step.clone());
    };
    if step.open_session() != Some(id) {
        return Transition::to(step.clone());
    }
    match session.state {
        SessionState::AwaitingScan => scanning(session, id, step),
        SessionState::AwaitingDecision => comparing(session, id),
        SessionState::Approved => paired(session),
        SessionState::Denied | SessionState::Expired | SessionState::Cancelled => {
            ended_by(session.outcome.as_ref().and_then(|o| o.reason))
        }
        SessionState::Failed => failed(session),
        SessionState::Unknown => unreadable(Some(session)),
    }
}

/// Still waiting for a scan: the countdown moves on the daemon's clock. A window this dialog did
/// not open has no code to draw.
fn scanning(session: &PairingSession, id: &str, step: &Step) -> Transition {
    let Step::Scan(scan) = step else {
        return elsewhere(Some(id.to_owned()));
    };
    let Some(ttl_ms) = guards::remaining(session.ttl_ms) else {
        return unreadable(Some(session));
    };
    Transition::to(Step::Scan(Scan {
        ttl_ms,
        ..scan.clone()
    }))
}

fn comparing(session: &PairingSession, id: &str) -> Transition {
    let Some(request) = session.request.as_ref() else {
        return unreadable(Some(session));
    };
    let ttl_ms = guards::remaining(session.ttl_ms);
    let fields = guards::field(&request.device_name) && guards::field(&request.model);
    let (Some(ttl_ms), true, true) = (ttl_ms, guards::digits(&request.sas), fields) else {
        return unreadable(Some(session));
    };
    Transition::to(Step::Compare(Compare {
        session: id.to_owned(),
        device_name: request.device_name.clone(),
        model: request.model.clone(),
        digits: request.sas.clone(),
        hardware: request.attestation.sentence.clone(),
        ttl_ms,
    }))
}

fn paired(session: &PairingSession) -> Transition {
    match session.request.as_ref().map(|r| &r.device_name) {
        Some(name) if guards::field(name) => Transition::to(Step::Paired { name: name.clone() }),
        _ => unreadable(None),
    }
}

/// The three endings the daemon reports by reason alone.
fn ended_by(reason: Option<OutcomeReason>) -> Transition {
    let sentence = match reason {
        Some(OutcomeReason::Timeout) => ENDED_EXPIRED,
        Some(OutcomeReason::Denied) => ENDED_DENIED,
        Some(OutcomeReason::Cancelled) => ENDED_CANCELLED,
        Some(OutcomeReason::Unknown) | None => return unreadable(None),
    };
    Transition::to(Step::Ended(Ending::pair_again(sentence)))
}

/// A failed window ends with the daemon's own sentence.
fn failed(session: &PairingSession) -> Transition {
    match &session.failure {
        Some(failure) => Transition::to(Step::Ended(Ending::pair_again(&failure.sentence))),
        None => unreadable(None),
    }
}

/// An answer this app cannot show. A window it leaves open is cancelled.
fn unreadable(session: Option<&PairingSession>) -> Transition {
    let open = session
        .filter(|s| !s.state.is_terminal())
        .and_then(|s| s.session_id.clone());
    Transition {
        step: Step::Ended(Ending::pair_again(ENDED_UNREADABLE)),
        abandons: open,
    }
}
