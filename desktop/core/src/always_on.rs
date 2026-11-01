//! The Always On pane (Always On setup §8.4): a renderer of the engine's summary. Every screen is
//! `home.status.summary` as the engine words it: its sentence, the step in hand, one main action
//! under its label, the others beside it, the review's items, and Details from the engine's
//! lines. Every press is one `home.act` with the key the action was drawn with and what its
//! `needs` collected, after its `confirm`; the engine runs the steps. The app keeps three things
//! of its own: how it asks for a secret, how it draws a choice of size and place, and that it is
//! restarting after its own Move now. The words here are those, and nothing the engine words.
//!
//! `model` holds the pane between reads and says what a press does and when to read, never on a
//! timer; `notices` keeps which of the engine's notices were raised already.

pub mod model;
pub mod notices;

use crate::home::{Action, ActionId, Line, Need, Provider, ReadinessItem, Summary, SummaryState};
use crate::management::CallError;
use crate::provision::{Offer, Options};
use crate::view::daemon_problem;
use crate::view::DaemonProblem;

/// The settings pane, and the pane the daemon files the `snapshots` section under.
pub const PANE: &str = "always_on";
/// What the pane cannot be without: the summary and the one step method.
pub const NEEDED: [&str; 2] = ["home.status", "home.act"];
/// The method whose listing in `hello` says the engine serves Always On.
const STEP: &str = "home.act";

pub const NOTICE_TITLE: &str = "Always On";
/// The desktop notification's id: a newer notice replaces the one before it.
pub const NOTICE_ID: &str = "always-on";
pub const LEARN_MORE: &str = "Learn more";
pub const LINK_FAILED: &str = "The browser could not open the page.";
pub const DETAILS: &str = "Details";
/// Over the providers' names, where the engine offers more than one.
pub const PROVIDER_CHOOSE: &str = "Choose where Fermix creates your server.";
pub const HOW_TO_GET_ONE: &str = "How to get one";
pub const CHANGE: &str = "Change";
pub const CHANGE_TITLE: &str = "Size and location";
pub const PASSPHRASE_TITLE: &str = "Recovery passphrase";
/// Where none is set yet, so it is typed twice.
pub const PASSPHRASE_NEW_BODY: &str = "Fermix encrypts every backup of your home to this \
    passphrase and never stores it. Keep it in a password manager: no backup can be restored \
    without it.";
/// Where one is set: Fermix checks it.
pub const PASSPHRASE_BODY: &str = "Type the recovery passphrase you set. Fermix never stores it.";
pub const PASSPHRASE: &str = "Passphrase";
pub const PASSPHRASE_AGAIN: &str = "Passphrase again";
pub const COPY_COMMAND: &str = "Copy command";
pub const COPY_CODE: &str = "Copy code";
pub const CANCEL: &str = "Cancel";
pub const DONE: &str = "Done";
pub const READING: &str = "Reading from Fermix…";
/// What the pane cannot show while Fermix on this computer was never read.
pub const NEEDS_FERMIX: &str = "Always On is read from Fermix on this computer. Start it to see \
    where Fermix runs.";
/// While Fermix restarts into the move this app started, and says nothing of it yet.
pub const RESTARTING: &str = "Fermix restarts to move to your server…";

/// What the pane shows as a whole.
#[derive(Debug, Clone, PartialEq)]
pub enum Page {
    /// The first read is out.
    Reading,
    /// Nothing was ever answered: why.
    Unreadable(DaemonProblem),
    /// The engine does not serve what the pane needs.
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

/// One line of Details: a sentence of the engine's, or one of its labelled lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    Sentence(String),
    Line(Line),
}

/// One screen, as the summary has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub state: SummaryState,
    pub sentence: String,
    /// The engine's sentence for the step in hand.
    pub progress: Option<String>,
    /// The bar that claims no percentage shows.
    pub shows_progress: bool,
    /// The one main action, or none: all is well, or its token is asked for in its place.
    pub action: Option<Action>,
    /// The other actions, beside the main one.
    pub others: Vec<Action>,
    /// The review's warnings and blocks.
    pub readiness: Vec<ReadinessItem>,
    pub details: Vec<Fact>,
    /// The main action needs a choice: the screen draws it, the recommendation first.
    pub choice: bool,
    /// The token field shows, in place of the main action.
    pub asks_token: bool,
}

/// The screen `summary` is: every action this app has a way to ask for, under the engine's
/// words.
pub fn screen(summary: &Summary) -> Screen {
    let detail = &summary.detail;
    let action = summary.action.clone().filter(offered);
    let others = detail
        .alternatives
        .iter()
        .chain(&detail.actions)
        .filter(|a| offered(a))
        .cloned()
        .collect();
    Screen {
        state: summary.state,
        sentence: summary.sentence.clone(),
        progress: detail.progress.clone(),
        shows_progress: matches!(summary.state, SummaryState::Creating | SummaryState::Moving),
        choice: action
            .as_ref()
            .is_some_and(|a| a.needs.contains(&Need::Choice)),
        action,
        others,
        readiness: detail.readiness.clone(),
        details: details(summary),
        asks_token: false,
    }
}

/// The screen while Fermix restarts into this app's own move: the bar and nothing to press.
pub fn restarting() -> Screen {
    Screen {
        state: SummaryState::Moving,
        sentence: RESTARTING.to_owned(),
        progress: None,
        shows_progress: true,
        action: None,
        others: Vec::new(),
        readiness: Vec::new(),
        details: Vec::new(),
        choice: false,
        asks_token: false,
    }
}

/// Details: the engine's note and billing, its lines, and what lives on the server.
fn details(summary: &Summary) -> Vec<Fact> {
    let detail = &summary.detail;
    let lines = detail.lines.iter().cloned().map(Fact::Line);
    let first = [&detail.note, &detail.billing].into_iter().flatten();
    first
        .cloned()
        .map(Fact::Sentence)
        .chain(lines)
        .chain(detail.data.clone().map(Fact::Sentence))
        .collect()
}

/// Whether the engine serves Always On: `hello` lists `home.act`. A release build lists none of
/// its methods (and an engine from before Always On none either), so no Always On shows there.
pub fn served(methods: &[String]) -> bool {
    methods.iter().any(|m| m == STEP)
}

/// The settings panes to list: `panes`, without Always On where the engine does not serve it.
pub fn listed(panes: Vec<&'static str>, served: bool) -> Vec<&'static str> {
    panes
        .into_iter()
        .filter(|pane| served || *pane != PANE)
        .collect()
}

/// Whether this app has a way to ask for `action`: an id it knows, and at most one need, one it
/// knows. Any other would be a button that does nothing.
pub fn offered(action: &Action) -> bool {
    action.id != ActionId::Unknown
        && action.needs.len() <= 1
        && !action.needs.contains(&Need::Unknown)
}

/// Whether a summary in this state says where a move stands: under way, done, undone, or gone
/// wrong.
pub fn says_where_the_move_stands(state: SummaryState) -> bool {
    matches!(
        state,
        SummaryState::Moving
            | SummaryState::On
            | SummaryState::Undone
            | SummaryState::Retired
            | SummaryState::Problem
    )
}

/// A step whose confirm deletes something: Return never takes it for yes.
pub fn destructive(id: ActionId) -> bool {
    matches!(
        id,
        ActionId::DeleteServer | ActionId::Cancel | ActionId::StartAgain
    )
}

/// Which provider's token a token field takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenTarget {
    /// The engine lists more than one and none is picked: the owner picks first, from every one
    /// listed, in the engine's order.
    Pick(Vec<Provider>),
    Provider(Provider),
}

/// Which of the `providers` the summary lists a token is for: the one `picked`, or the only one.
/// None while the summary lists none.
pub fn token_target(providers: &[Provider], picked: Option<&str>) -> Option<TokenTarget> {
    if let Some(chosen) = providers.iter().find(|p| Some(p.id.as_str()) == picked) {
        return Some(TokenTarget::Provider(chosen.clone()));
    }
    match providers {
        [] => None,
        [only] => Some(TokenTarget::Provider(only.clone())),
        many => Some(TokenTarget::Pick(many.to_vec())),
    }
}

/// What a person is told for a call that did not land: the daemon's sentence for a refusal, and
/// the app's for a daemon that is not there.
pub fn sentence(error: &CallError) -> String {
    match error {
        CallError::Refused(refusal) => refusal.sentence.clone(),
        other => daemon_problem(other).sentence(),
    }
}

// ---- The words of the app's own asking ----------------------------------------------------------

/// What the field says while the engine checks a pasted token with the provider.
pub fn token_checking(provider: &str) -> String {
    format!("Checking the token with {provider}…")
}

/// The link to a provider's console, after its steps.
pub fn console_label(provider: &str) -> String {
    format!("Open {provider}")
}

/// The offers grouped by the city of their place, in the engine's order of places and of
/// offers; a place with none in stock is left out.
pub fn by_city(options: &Options) -> Vec<(&str, Vec<&Offer>)> {
    options
        .locations
        .iter()
        .map(|place| {
            let here = options.offers.iter().filter(|o| o.location == place.id);
            (place.city.as_str(), here.collect::<Vec<_>>())
        })
        .filter(|(_, offers)| !offers.is_empty())
        .collect()
}

/// Over the command and the code for a server of the owner's own.
pub fn own_server_line(until: &str) -> String {
    format!(
        "Run this on your Linux server, then type the code below when it asks. The code works \
         once, until {until}."
    )
}
