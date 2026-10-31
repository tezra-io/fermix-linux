//! The Always On pane (M75 lean v1 §7), worked out from what the engine said and the few things
//! the app keeps itself. `home.status.summary` is the one source: every screen maps to its
//! `state`, every sentence and action label is the engine's, and Details holds its `detail`.
//! The app keeps only what nothing in the engine records: Set up, which opens the token screen;
//! Paste a new token, which opens the field in place; and the restart after its own Move now,
//! while the engine is away. The words here are the app's own labels, the same as the Mac's.
//!
//! `actions` says where each action goes and the calls it makes, `notices` the notifications a
//! change between two reads raises and how often the summary is read, and `model` holds the
//! pane's state between reads.

pub mod actions;
pub mod model;
pub mod notices;

use crate::home::{
    Action, ActionId, Code, Detail, HomeStatus, ItemStatus, ProblemKind, ReadinessItem, Role,
    Summary, SummaryState,
};
use crate::management::CallError;
use crate::provision::{Offer, Options, METHODS};
use crate::view::{daemon_problem, DaemonProblem};

/// The settings pane, and the pane the daemon files the `snapshots` section under.
pub const PANE: &str = "always_on";
/// The Always On docs page Learn more opens.
pub const GUIDE_URL: &str = "https://fermix.ai/docs/always-on/";
/// Hetzner's console, where the token is made.
pub const HETZNER_CONSOLE: &str = "https://console.hetzner.cloud/";
const STATUS_METHOD: &str = "home.status";

pub const NOTICE_TITLE: &str = "Always On";
pub const LEARN_MORE: &str = "Learn more";
pub const LINK_FAILED: &str = "The browser could not open the page.";
pub const DETAILS: &str = "Details";
pub const TOKEN_FIELD: &str = "Hetzner API token";
pub const TOKEN_CHECKING: &str = "Checking the token with Hetzner…";
pub const HOW_TO_GET_ONE: &str = "How to get one";
pub const OPEN_HETZNER: &str = "Open Hetzner";
pub const CHANGE: &str = "Change";
pub const CHANGE_TITLE: &str = "Size and location";
pub const PASSPHRASE_TITLE: &str = "Recovery passphrase";
pub const PASSPHRASE_BODY: &str = "Fermix encrypts every backup of your home to this passphrase \
    and never stores it. Keep it in a password manager: no backup can be restored without it.";
pub const PASSPHRASE_MOVE_BODY: &str = "Type your recovery passphrase. The move encrypts the \
    backups of your home to it, and Fermix never stores it.";
pub const PASSPHRASE: &str = "Passphrase";
pub const PASSPHRASE_AGAIN: &str = "Passphrase again";
pub const SET_PASSPHRASE: &str = "Set passphrase";
pub const MOVE_NOW: &str = "Move now";
pub const COPY_COMMAND: &str = "Copy command";
pub const SERVER_NAME: &str = "Name";
pub const SERVER_SIZE: &str = "Size";
pub const SERVER_LOCATION: &str = "Location";
pub const SERVER_ADDRESS: &str = "IPv4 address";
pub const SERVER_SYSTEM: &str = "System";
pub const SERVER_VERSION: &str = "Version";
pub const COPIES: &str = "Backups on this computer";
pub const DISK: &str = "Server disk in use";
pub const REPLACE_TITLE: &str = "Replace the server";
pub const REPLACE_BODY: &str = "Fermix creates a new server for your home. Once you move to it, \
    Fermix deletes the old one.";
pub const REPLACE_CREATE: &str = "Create server";
pub const CANCEL: &str = "Cancel";
pub const DONE: &str = "Done";
pub const READING: &str = "Reading from Fermix…";
/// What the pane cannot show while Fermix on this computer is not running.
pub const NEEDS_FERMIX: &str = "Always On is read from Fermix on this computer. Start it to see \
    where Fermix runs.";

/// The actions that call the Hetzner provisioner.
const HETZNER_ACTIONS: [ActionId; 9] = [
    ActionId::CreateServer,
    ActionId::Cancel,
    ActionId::TurnOn,
    ActionId::RestartServer,
    ActionId::StartAgain,
    ActionId::CreateIn,
    ActionId::PasteToken,
    ActionId::DeleteServer,
    ActionId::ReplaceServer,
];

/// The steps the app keeps itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Local {
    /// Set up was pressed on `off`: the token screen shows until a token or a code exists.
    pub setting_up: bool,
    /// Paste a new token was pressed on a problem that offers it.
    pub pasting_token: bool,
    /// The engine's sentence for the move this app started, while the engine restarts to carry
    /// it out and until it says where the move stands.
    pub move_started: Option<String>,
}

/// The screen a summary maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Off,
    /// The paste field, "How to get one" and the own-server link: opened by Set up, and the
    /// engine's `connect`.
    Token,
    Choose,
    Creating,
    Ready,
    Moving,
    On,
    Undone,
    Retired,
    /// The server's own page, before its move.
    Candidate,
    /// The server's own page, after its move.
    Primary,
    Problem(ProblemKind),
    /// A state this app does not know: its sentence, and nothing more.
    Unknown,
}

/// One screen: one sentence, at most one main action, and what Details and its links hold.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub kind: ScreenKind,
    pub sentence: String,
    /// The one main action, or none when all is well.
    pub action: Option<Action>,
    /// The token screen's "How to get one" steps.
    pub steps: Vec<String>,
    /// The other actions: Details' on a problem and on `on`, the own-server link on the token
    /// screen.
    pub alternatives: Vec<Action>,
    /// The paste field shows (the token screen, or Paste a new token).
    pub token_field: bool,
    /// The bar that claims no percentage shows (creating and moving).
    pub shows_progress: bool,
    /// The recovery passphrase fields sit below while none is set.
    pub asks_passphrase: bool,
    pub detail: Detail,
}

/// What the pane shows as a whole.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// The first read is out.
    Reading,
    /// `home.status` was not answered, and nothing was before: why.
    Unreadable(DaemonProblem),
    /// The engine answers no summary, or does not serve what the screen needs.
    NeedsUpdate,
    Screen(Box<Screen>),
}

impl Page {
    pub fn screen(self) -> Option<Screen> {
        match self {
            Page::Screen(screen) => Some(*screen),
            _ => None,
        }
    }
}

/// The page for what the reads answered: `status` is the last `home.status` answered, kept
/// while a later read fails; `failure` the last read's, while it is the latest word; `served`
/// the methods `hello` lists, empty until it is read.
pub fn page(
    status: Option<&HomeStatus>,
    failure: Option<&DaemonProblem>,
    local: &Local,
    served: &[String],
) -> Page {
    if let Some(started) = own_move(status, failure, local) {
        return Page::Screen(Box::new(Screen::moving(started)));
    }
    let lacks_status = !served.is_empty() && !served.iter().any(|m| m == STATUS_METHOD);
    if lacks_status || matches!(failure, Some(DaemonProblem::UpdateNeeded(_))) {
        return Page::NeedsUpdate;
    }
    if let Some(DaemonProblem::NotRunning) = failure {
        return Page::Unreadable(DaemonProblem::NotRunning);
    }
    let Some(status) = status else {
        return failure.map_or(Page::Reading, |p| Page::Unreadable(p.clone()));
    };
    let Some(summary) = &status.summary else {
        return Page::NeedsUpdate;
    };
    let screen = Screen::of(summary, status, local);
    if screen.needs_hetzner() && !serves_hetzner(served) {
        return Page::NeedsUpdate;
    }
    Page::Screen(Box::new(screen))
}

/// The sentence of this app's own move while the engine has not answered for it: it is away
/// while it restarts, or still says what it said before the move began.
fn own_move<'a>(
    status: Option<&HomeStatus>,
    failure: Option<&DaemonProblem>,
    local: &'a Local,
) -> Option<&'a str> {
    let started = local.move_started.as_deref()?;
    let state = status.and_then(|s| s.summary.as_ref()).map(|s| s.state);
    (failure.is_some() || !says_where_the_move_stands(state)).then_some(started)
}

/// Whether a summary in this state says where a move stands: under way, done, undone, or gone
/// wrong.
pub fn says_where_the_move_stands(state: Option<SummaryState>) -> bool {
    matches!(
        state,
        Some(
            SummaryState::Moving
                | SummaryState::On
                | SummaryState::Undone
                | SummaryState::Retired
                | SummaryState::Problem
        )
    )
}

/// Whether `hello` lists every `home.provision.*` method, which the Hetzner path needs.
pub fn serves_hetzner(served: &[String]) -> bool {
    METHODS.iter().all(|m| served.iter().any(|s| s == m))
}

impl Screen {
    fn of(summary: &Summary, home: &HomeStatus, local: &Local) -> Screen {
        match summary.state {
            SummaryState::Off if local.setting_up => Screen::token_step(summary),
            SummaryState::Connect => Screen::token(summary),
            SummaryState::Problem => Screen::problem(summary, local),
            state => Screen::plain(summary, kind_of(state), home),
        }
    }

    /// The app's own Set up step: `off`'s token words.
    fn token_step(summary: &Summary) -> Screen {
        let connect = summary.detail.connect.as_ref();
        Screen {
            sentence: connect.map_or_else(|| summary.sentence.clone(), |c| c.sentence.clone()),
            steps: connect.map(|c| c.steps.clone()).unwrap_or_default(),
            alternatives: offered_all(connect.map(|c| c.alternatives.as_slice())),
            ..Screen::token(summary)
        }
    }

    fn token(summary: &Summary) -> Screen {
        Screen {
            kind: ScreenKind::Token,
            sentence: summary.sentence.clone(),
            action: None,
            steps: summary.detail.steps.clone().unwrap_or_default(),
            alternatives: offered_all(summary.detail.alternatives.as_deref()),
            token_field: true,
            shows_progress: false,
            asks_passphrase: false,
            detail: summary.detail.clone(),
        }
    }

    fn problem(summary: &Summary, local: &Local) -> Screen {
        let pasting = local.pasting_token
            && summary.action.as_ref().map(|a| a.id) == Some(ActionId::PasteToken);
        Screen {
            kind: ScreenKind::Problem(summary.detail.kind.unwrap_or(ProblemKind::Unknown)),
            sentence: summary.sentence.clone(),
            action: offered(summary.action.as_ref()).filter(|_| !pasting),
            steps: Vec::new(),
            alternatives: offered_all(summary.detail.alternatives.as_deref()),
            token_field: pasting,
            shows_progress: false,
            asks_passphrase: false,
            detail: summary.detail.clone(),
        }
    }

    fn plain(summary: &Summary, kind: ScreenKind, home: &HomeStatus) -> Screen {
        let asks = kind == ScreenKind::Creating
            && home.role == Role::Primary
            && !home.snapshots.recipient_set;
        Screen {
            kind,
            sentence: summary.sentence.clone(),
            action: offered(summary.action.as_ref()),
            steps: Vec::new(),
            alternatives: offered_all(summary.detail.actions.as_deref()),
            token_field: false,
            shows_progress: matches!(kind, ScreenKind::Creating | ScreenKind::Moving),
            asks_passphrase: asks,
            detail: summary.detail.clone(),
        }
    }

    /// The moving screen while the engine restarts for this app's own move.
    fn moving(sentence: &str) -> Screen {
        Screen {
            kind: ScreenKind::Moving,
            sentence: sentence.to_owned(),
            action: None,
            steps: Vec::new(),
            alternatives: Vec::new(),
            token_field: false,
            shows_progress: true,
            asks_passphrase: false,
            detail: Detail::default(),
        }
    }

    /// The review's warnings and blocks the summary carries, listed only when there are some.
    pub fn warnings(&self) -> Vec<ReadinessItem> {
        let items = self.detail.readiness.as_deref().unwrap_or_default();
        items
            .iter()
            .filter(|i| i.status != ItemStatus::Ok)
            .cloned()
            .collect()
    }

    /// Whether this screen calls the Hetzner provisioner.
    pub fn needs_hetzner(&self) -> bool {
        let action = self.action.as_ref().map(|a| a.id);
        self.kind == ScreenKind::Choose || action.is_some_and(|id| HETZNER_ACTIONS.contains(&id))
    }

    /// Whether the screen has Details at all.
    pub fn has_details(&self) -> bool {
        matches!(
            self.kind,
            ScreenKind::Choose | ScreenKind::Ready | ScreenKind::On | ScreenKind::Problem(_)
        )
    }
}

fn kind_of(state: SummaryState) -> ScreenKind {
    match state {
        SummaryState::Off => ScreenKind::Off,
        SummaryState::Connect => ScreenKind::Token,
        SummaryState::Choose => ScreenKind::Choose,
        SummaryState::Creating => ScreenKind::Creating,
        SummaryState::Ready => ScreenKind::Ready,
        SummaryState::Moving => ScreenKind::Moving,
        SummaryState::On => ScreenKind::On,
        SummaryState::Undone => ScreenKind::Undone,
        SummaryState::Retired => ScreenKind::Retired,
        SummaryState::Candidate => ScreenKind::Candidate,
        SummaryState::Primary => ScreenKind::Primary,
        SummaryState::Problem => ScreenKind::Problem(ProblemKind::Unknown),
        SummaryState::Unknown => ScreenKind::Unknown,
    }
}

/// An action this app knows, or none: a button whose id it does not know would do nothing.
fn offered(action: Option<&Action>) -> Option<Action> {
    action.filter(|a| a.id != ActionId::Unknown).cloned()
}

/// The alternatives this app knows. "Use my own Linux server" is left out while a server Fermix
/// created stands (Details then offers to delete it): only one code is open at a time, and a new
/// one would strand that server (L15).
fn offered_all(actions: Option<&[Action]>) -> Vec<Action> {
    let known: Vec<Action> = actions
        .unwrap_or_default()
        .iter()
        .filter(|a| a.id != ActionId::Unknown)
        .cloned()
        .collect();
    let stands = known.iter().any(|a| a.id == ActionId::DeleteServer);
    known
        .into_iter()
        .filter(|a| !(stands && a.id == ActionId::OwnServer))
        .collect()
}

/// One line of Details: the engine's words and values, under the app's own labels.
#[derive(Debug, Clone, PartialEq)]
pub enum Fact {
    /// One line as it is.
    Line(String),
    /// A labelled value as the engine gave it.
    Value(&'static str, String),
    /// A labelled time, drawn in this computer's own style.
    Time(String, String),
    Bytes(&'static str, u64),
    /// A fraction, drawn as a percentage.
    Share(&'static str, f64),
    /// Until when a ready server waits for its move.
    WaitsUntil(String),
    /// When Fermix last saw what a problem rests on.
    Seen(String),
    /// A warning or block of the review.
    Item(ReadinessItem),
    /// Another action the screen offers.
    Act(Action),
}

/// What Details holds (§7): on `choose` the server's shape and the price with its IPv4 address
/// and VAT, and how Hetzner bills; on `ready` the server's facts and until when it waits; on
/// `on` the server, each backup, the space this computer's copies take, the server's disk, what
/// lives on the server, and the other actions; on a problem what Fermix last saw and when, and
/// the other actions. `offer` is the size and place Create server makes, in `currency`.
pub fn details(screen: &Screen, offer: Option<(&Offer, &str)>) -> Vec<Fact> {
    let detail = &screen.detail;
    let mut facts = Vec::new();
    match screen.kind {
        ScreenKind::Choose => {
            if let Some((offer, currency)) = offer {
                facts.push(Fact::Line(shape(offer)));
                facts.push(Fact::Line(price(offer, currency)));
            }
            facts.extend(detail.billing.clone().map(Fact::Line));
        }
        ScreenKind::Ready => facts.extend(server_facts(detail)),
        ScreenKind::On => facts.extend(on_facts(detail)),
        ScreenKind::Problem(_) => {
            facts.extend(detail.note.clone().map(Fact::Line));
            facts.extend(detail.seen_at.clone().map(Fact::Seen));
            facts.extend(screen.warnings().into_iter().map(Fact::Item));
        }
        _ => return facts,
    }
    let acts = matches!(screen.kind, ScreenKind::On | ScreenKind::Problem(_));
    if acts {
        facts.extend(screen.alternatives.iter().cloned().map(Fact::Act));
    }
    facts
}

/// The server's facts as Hetzner reports them, each only where the engine gave it, and until
/// when a ready one waits.
fn server_facts(detail: &Detail) -> Vec<Fact> {
    let mut facts = Vec::new();
    if let Some(server) = &detail.server {
        let given = [
            (SERVER_NAME, &server.name),
            (SERVER_SIZE, &server.size),
            (SERVER_LOCATION, &server.location),
            (SERVER_ADDRESS, &server.ipv4),
            (SERVER_SYSTEM, &server.system),
            (SERVER_VERSION, &server.version),
        ];
        let values = given
            .into_iter()
            .filter_map(|(label, value)| Some(Fact::Value(label, value.clone()?)));
        facts.extend(values);
    }
    facts.extend(detail.waits_until.clone().map(Fact::WaitsUntil));
    facts
}

fn on_facts(detail: &Detail) -> Vec<Fact> {
    let mut facts = server_facts(detail);
    let backups = detail.backups.as_deref().unwrap_or_default();
    facts.extend(
        backups
            .iter()
            .map(|b| Fact::Time(backup_label(&b.place), b.at.clone())),
    );
    facts.extend(detail.copies_bytes.map(|b| Fact::Bytes(COPIES, b)));
    facts.extend(detail.disk_used.map(|used| Fact::Share(DISK, used)));
    facts.extend(detail.data.clone().map(Fact::Line));
    facts
}

/// The server a move goes to: the one the code Fermix put inside the created server enrolled,
/// else the newest one held, the engine's own rule for the server its summary is ready for. On a
/// linked desktop replacing its server, the server's own picture holds them.
pub fn move_target(status: &HomeStatus) -> Option<String> {
    let home = status.home_of_record()?;
    let bound = home
        .enrollment
        .as_ref()
        .and_then(|e| e.candidate_id.as_ref());
    if let Some(bound) = bound.filter(|b| home.candidates.iter().any(|c| &c.candidate_id == *b)) {
        return Some(bound.clone());
    }
    home.candidates
        .iter()
        .max_by(|a, b| a.enrolled_at.cmp(&b.enrolled_at))
        .map(|c| c.candidate_id.clone())
}

/// What a person is told for a call that did not land: the daemon's sentence for a refusal, and
/// the app's for a daemon that is not there.
pub fn sentence(error: &CallError) -> String {
    match error {
        CallError::Refused(refusal) => refusal.sentence.clone(),
        other => daemon_problem(other).sentence(),
    }
}

// ---- The words around the engine's values ---------------------------------------------------

/// An amount Hetzner gave, in its currency, never rounded or summed here.
pub fn money(amount: &str, currency: &str) -> String {
    match currency {
        "EUR" => format!("€{amount}"),
        "USD" => format!("${amount}"),
        _ => format!("{amount} {currency}"),
    }
}

/// The owner's own choice, in the words the recommendation is said in.
pub fn choice_sentence(offer: &Offer, options: &Options) -> String {
    let city = options
        .city(offer.location)
        .or(offer.location.wire())
        .unwrap_or_default();
    let total = money(&offer.total_monthly, &options.currency);
    format!("{} in {city}, {total} a month.", offer.description)
}

pub fn monthly(offer: &Offer, currency: &str) -> String {
    format!("{} a month", money(&offer.total_monthly, currency))
}

pub fn shape(offer: &Offer) -> String {
    format!(
        "{} vCPU, {} GB memory, {} GB disk",
        offer.cores, offer.memory_gb, offer.disk_gb
    )
}

pub fn price(offer: &Offer, currency: &str) -> String {
    format!(
        "{} a month, and {} for its IPv4 address. Prices include VAT.",
        money(&offer.monthly, currency),
        money(&offer.ipv4_monthly, currency)
    )
}

pub fn waits_until(time: &str) -> String {
    format!("It waits for the move until {time}.")
}

pub fn backup_label(place: &str) -> String {
    format!("Last backup in {place}")
}

pub fn seen_line(time: &str) -> String {
    format!("Fermix last checked {time}.")
}

pub fn own_server_line(until: &str) -> String {
    format!("Run this on your Linux server. The code inside works once, until {until}.")
}

/// The one command an owner's own Linux server runs, with the code inside (§7): the installer
/// reads the code from `FERMIX_ENROLL_CODE` rather than asking for it. The code goes into a
/// shell line, so one outside the contract's shape (`fxenroll1.` and base64url) is no command.
pub fn own_server_command(code: &Code) -> Option<String> {
    let text = code.text();
    let body = text.strip_prefix("fxenroll1.")?;
    let shaped = !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    shaped.then(|| {
        format!("curl -fsSL https://fermix.ai/always-on | sudo FERMIX_ENROLL_CODE={text} sh")
    })
}
