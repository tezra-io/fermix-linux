//! The Always On pane between reads: the last summary answered, what a press is waiting on (the
//! token, the choice, the passphrase or the confirm), the offers read for a choice, the act on its
//! way and what it answered, and the restart after this app's own Move now. Every decision the
//! pane makes is a method here, so the window draws what this says and sends what it hands over.
//!
//! A press collects what the action `needs`, asks its `confirm` (a dialog that collects a need
//! shows the confirm and its answer accepts it), then hands over one `home.act` with the key the
//! action was drawn with. A step held between two reads goes once the summary no longer offers
//! its action under that key, which is the engine's own rule for a press.
//!
//! When to read is decided here too, and never by a clock: `home.status` when the pane is shown,
//! after each act, when the engine says the summary changed (`home_status_changed`), and when a
//! new engine answers while the pane shows; the offers once while a choice shows. Each marks a
//! read due; the window asks `next_read` until it answers none, so the notices that come while a
//! read is out are one more read, not one each.

use super::notices::Posted;
use super::{
    offered, restarting, says_where_the_move_stands, screen, sentence, token_target, Page,
    TokenTarget, LINK_FAILED, NEEDED,
};
use crate::home::{
    passphrase, ActResult, Acted, Action, ActionId, HomeStatus, Input, Need, Notice, OwnServer,
    Passphrase, Summary, SummaryState, Token,
};
use crate::management::CallError;
use crate::provision::{Options, Pick};
use crate::view::DaemonProblem;

/// One read the pane makes, as `next_read` hands them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// `hello`'s methods, then `home.status` where it lists what the pane needs.
    Status,
    /// `home.provision.options`: the provider in use's offers.
    Options,
}

/// Where a press goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// The screen changed: a field, a dialog or a sentence to draw.
    Drawn,
    /// What the press needs is read first: the window asks `next_read`.
    Read,
    /// A page to open in the browser.
    Open(String),
    /// The app's own update.
    Update,
    /// The one `home.act`, off the window's thread.
    Run(Act),
    /// Another act is on its way, or there is nothing to send.
    Ignored,
}

/// One `home.act`: the action as it was drawn, and what its needs collected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Act {
    pub action: ActionId,
    pub key: String,
    pub input: Input,
}

/// The dialog a press waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dialog {
    /// The size and place, the recommendation selected, with the action's confirm.
    Choice(Action),
    /// The recovery passphrase, twice where none is set, with the action's confirm.
    Passphrase { action: Action, twice: bool },
    /// The action's confirm.
    Confirm(Action),
}

/// What a press waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Asking {
    /// The token field, in place of the action.
    Token(Action),
    Choice(Action),
    Passphrase {
        action: Action,
        twice: bool,
    },
    Confirm {
        action: Action,
        input: Input,
    },
}

impl Asking {
    fn action(&self) -> &Action {
        match self {
            Asking::Token(action) | Asking::Choice(action) => action,
            Asking::Passphrase { action, .. } | Asking::Confirm { action, .. } => action,
        }
    }
}

/// This app's own Move now started, and the engine restarts into it.
#[derive(Debug)]
struct Restart {
    /// The engine was seen away, or a new one answered, since.
    away: bool,
}

#[derive(Debug, Default)]
pub struct Model {
    /// The methods `hello` lists, empty until it is read.
    served: Vec<String>,
    /// The last summary answered, by a read or by an act.
    last: Option<Summary>,
    /// The last read's failure, while it is the latest word.
    failure: Option<DaemonProblem>,
    /// The provider picked on the token screen, where the engine lists several.
    picked: Option<String>,
    /// What the provider sells, read while a choice shows.
    options: Option<Result<Options, String>>,
    /// The size and place the owner chose; the recommendation until then.
    chosen: Option<Pick>,
    asking: Option<Asking>,
    /// The act on its way, which holds every press.
    pub working: Option<ActionId>,
    /// The daemon's refusal of the last act, said under the action.
    pub refusal: Option<String>,
    pub token_refusal: Option<String>,
    pub passphrase_refusal: Option<String>,
    /// The code for a server of the owner's own, shown once, while the code is open and the
    /// pane shows it.
    pub handout: Option<OwnServer>,
    restart: Option<Restart>,
    /// The notices raised, kept across launches by the window.
    posted: Posted,
    pub pane_visible: bool,
    due_status: bool,
}

impl Model {
    /// A model that knows the notices earlier runs raised.
    pub fn with_posted(posted: Posted) -> Model {
        Model {
            posted,
            ..Model::default()
        }
    }

    /// The notices raised, for the window to keep.
    pub fn posted(&self) -> &Posted {
        &self.posted
    }

    pub fn page(&self) -> Page {
        let lacks =
            !self.served.is_empty() && !NEEDED.iter().all(|m| self.served.iter().any(|s| s == m));
        if lacks || matches!(self.failure, Some(DaemonProblem::UpdateNeeded(_))) {
            return Page::NeedsUpdate;
        }
        let Some(summary) = &self.last else {
            return self.failure.clone().map_or(Page::Reading, Page::Unreadable);
        };
        if self.restart.is_some() && !says_where_the_move_stands(summary.state) {
            return Page::Screen(Box::new(restarting()));
        }
        let mut shown = screen(summary);
        if let Some(Asking::Token(asked)) = &self.asking {
            shown.asks_token = true;
            shown.action = shown.action.filter(|a| a != asked);
        }
        Page::Screen(Box::new(shown))
    }

    pub fn summary(&self) -> Option<&Summary> {
        self.last.as_ref()
    }

    /// The last read's failure under a screen drawn from an earlier answer.
    pub fn read_note(&self) -> Option<String> {
        self.last.as_ref()?;
        self.failure.as_ref().map(DaemonProblem::sentence)
    }

    /// Whose token the field takes, while one is asked for: of the providers the summary lists.
    pub fn token_target(&self) -> Option<TokenTarget> {
        let Some(Asking::Token(_)) = &self.asking else {
            return None;
        };
        let providers = &self.last.as_ref()?.detail.providers;
        token_target(providers, self.picked.as_deref())
    }

    pub fn dialog(&self) -> Option<Dialog> {
        match self.asking.as_ref()? {
            Asking::Token(_) => None,
            Asking::Choice(action) => self
                .loaded_options()
                .map(|_| Dialog::Choice(action.clone())),
            Asking::Passphrase { action, twice } => Some(Dialog::Passphrase {
                action: action.clone(),
                twice: *twice,
            }),
            Asking::Confirm { action, .. } => Some(Dialog::Confirm(action.clone())),
        }
    }

    pub fn options(&self) -> Option<&Result<Options, String>> {
        self.options.as_ref()
    }

    pub fn loaded_options(&self) -> Option<&Options> {
        self.options.as_ref()?.as_ref().ok()
    }

    pub fn chosen(&self) -> Option<&Pick> {
        self.chosen.as_ref()
    }

    /// The size and place selected in a list of offers: the owner's, else the recommendation.
    pub fn pick(&self) -> Option<Pick> {
        let recommended = || self.loaded_options()?.recommended.clone();
        self.chosen.clone().or_else(recommended)
    }

    // ---- Reads -------------------------------------------------------------------------

    pub fn served(&mut self, methods: Vec<String>) {
        self.served = methods;
    }

    /// A read answered: the notice to raise, if any.
    pub fn answered(&mut self, status: HomeStatus) -> Option<Notice> {
        self.failure = None;
        let back = self.restart.as_ref().is_some_and(|r| r.away);
        if back || says_where_the_move_stands(status.summary.state) {
            self.restart = None;
        }
        self.take(status.summary)
    }

    /// A read was not answered: the screen stays as it was and says so.
    pub fn unanswered(&mut self, problem: DaemonProblem) {
        if let Some(restart) = &mut self.restart {
            restart.away = true;
        }
        self.failure = Some(problem);
    }

    /// A newer summary: what a press waited on goes once it no longer offers that action under
    /// its key, the code once it is no longer open, the offers once no choice shows.
    fn take(&mut self, summary: Summary) -> Option<Notice> {
        if self
            .asking
            .as_ref()
            .is_some_and(|a| !summary.offers(a.action()))
        {
            self.asking = None;
            self.token_refusal = None;
            self.passphrase_refusal = None;
        }
        if summary.state != SummaryState::Connect {
            self.handout = None;
        }
        let fresh = summary.notice.clone().filter(|n| self.posted.fresh(n));
        self.last = Some(summary);
        if !self.choosing() {
            self.options = None;
            self.chosen = None;
        }
        fresh
    }

    /// The pane appeared: it reads the summary.
    pub fn pane_shown(&mut self) {
        self.pane_visible = true;
        self.due_status = true;
    }

    /// The pane went: the code it showed, what was asked on it and the offers go with it.
    pub fn pane_hidden(&mut self) {
        self.pane_visible = false;
        self.handout = None;
        self.asking = None;
        self.options = None;
        self.chosen = None;
        self.picked = None;
        self.token_refusal = None;
        self.passphrase_refusal = None;
    }

    /// The engine said the summary changed: one read of it.
    pub fn home_status_changed(&mut self) {
        self.due_status = true;
    }

    /// A new engine answers here (a restart, this app's own move, Fermix started again): it is
    /// read now only while the pane shows.
    pub fn engine_restarted(&mut self) {
        if let Some(restart) = &mut self.restart {
            restart.away = true;
        }
        if self.pane_visible {
            self.due_status = true;
        }
    }

    /// The next read due, if any: the summary first, then the offers a choice shows. Each is
    /// handed out once; what a later trigger asks for is due again.
    pub fn next_read(&mut self) -> Option<Read> {
        if std::mem::take(&mut self.due_status) {
            return Some(Read::Status);
        }
        let wanted = self.pane_visible && self.options.is_none() && self.choosing();
        wanted.then_some(Read::Options)
    }

    /// What the provider sells, or why not. A choice's dialog does not open on a failed read,
    /// which is said under the action.
    pub fn options_read(&mut self, answer: Result<Options, String>) {
        if let (Err(why), Some(Asking::Choice(_))) = (&answer, &self.asking) {
            self.refusal = Some(why.clone());
            self.asking = None;
        }
        self.options = Some(answer);
    }

    /// Whether a choice shows: the main action needs one, or a dialog asks for it.
    fn choosing(&self) -> bool {
        let main = self.last.as_ref().and_then(|s| s.action.as_ref());
        matches!(self.asking, Some(Asking::Choice(_)))
            || main.is_some_and(|a| offered(a) && a.needs.contains(&Need::Choice))
    }

    // ---- Presses -----------------------------------------------------------------------

    /// One action the screen offers was pressed.
    pub fn press(&mut self, action: &Action) -> Press {
        if self.working.is_some() || !offered(action) {
            return Press::Ignored;
        }
        self.refusal = None;
        match action.id {
            ActionId::Update => return Press::Update,
            ActionId::LearnMore | ActionId::OpenConsole => return self.open(action),
            _ => {}
        }
        match action.needs.first() {
            Some(Need::Token) => {
                self.asking = Some(Asking::Token(action.clone()));
                self.token_refusal = None;
                Press::Drawn
            }
            Some(Need::RecoveryPassphrase) => self.ask_passphrase(action, false),
            Some(Need::NewRecoveryPassphrase) => self.ask_passphrase(action, true),
            Some(Need::Choice) => self.ask_choice(action),
            Some(Need::Unknown) => unreachable!("an action with a need it cannot ask is not drawn"),
            None => self.confirm_or_run(action, Input::none()),
        }
    }

    fn open(&mut self, action: &Action) -> Press {
        match &action.url {
            Some(url) => Press::Open(url.clone()),
            None => {
                self.refusal = Some(LINK_FAILED.to_owned());
                Press::Drawn
            }
        }
    }

    fn ask_passphrase(&mut self, action: &Action, twice: bool) -> Press {
        self.asking = Some(Asking::Passphrase {
            action: action.clone(),
            twice,
        });
        self.passphrase_refusal = None;
        Press::Drawn
    }

    /// The main action's choice is drawn on the screen, so its press goes to the confirm; any
    /// other opens its dialog once the offers are read, a failed read read again.
    fn ask_choice(&mut self, action: &Action) -> Press {
        let main = self.last.as_ref().and_then(|s| s.action.as_ref());
        if main == Some(action) {
            return self.confirm_or_run(action, Input::choice(self.chosen.clone()));
        }
        self.asking = Some(Asking::Choice(action.clone()));
        if matches!(self.options, Some(Err(_))) {
            self.options = None;
        }
        if self.loaded_options().is_some() {
            Press::Drawn
        } else {
            Press::Read
        }
    }

    fn confirm_or_run(&mut self, action: &Action, input: Input) -> Press {
        if action.confirm.is_some() {
            self.asking = Some(Asking::Confirm {
                action: action.clone(),
                input,
            });
            return Press::Drawn;
        }
        self.run(action, input)
    }

    /// Holds every press for `action`, and hands over its act. The token field stays while its
    /// check runs; a dialog's ask is answered.
    fn run(&mut self, action: &Action, input: Input) -> Press {
        self.working = Some(action.id);
        if !matches!(self.asking, Some(Asking::Token(_))) {
            self.asking = None;
        }
        Press::Run(Act {
            action: action.id,
            key: action.key.clone(),
            input,
        })
    }

    /// The provider whose token screen shows, where the engine lists several.
    pub fn pick_provider(&mut self, id: &str) {
        assert!(!id.is_empty(), "a provider is picked by its id");
        self.picked = Some(id.to_owned());
    }

    /// A token was pasted into the field: the act for the provider it is for, naming it where
    /// the engine lists several.
    pub fn token_entered(&mut self, text: &str) -> Press {
        let Some(Asking::Token(action)) = self.asking.clone() else {
            return Press::Ignored;
        };
        let (Some(token), Some(TokenTarget::Provider(provider))) =
            (Token::pasted(text), self.token_target())
        else {
            return Press::Ignored;
        };
        if self.working.is_some() {
            return Press::Ignored;
        }
        let several = self
            .last
            .as_ref()
            .is_some_and(|s| s.detail.providers.len() > 1);
        let named = several.then_some(provider.id.as_str());
        self.token_refusal = None;
        self.confirm_or_run(&action, Input::token(token, named))
    }

    /// A size and place picked in a list of offers.
    pub fn choose(&mut self, pick: Pick) {
        self.chosen = Some(pick);
    }

    /// The choice dialog was answered: it showed the confirm, so this is its act.
    pub fn choice_answered(&mut self) -> Press {
        let Some(Asking::Choice(action)) = self.asking.clone() else {
            return Press::Ignored;
        };
        if self.working.is_some() {
            return Press::Ignored;
        }
        self.run(&action, Input::choice(self.chosen.clone()))
    }

    /// The passphrase dialog was answered: `again` is the second entry, where none is set. It
    /// showed the confirm, so this is its act; two entries that differ are said in it.
    pub fn passphrase_entered(&mut self, first: &str, again: Option<&str>) -> Press {
        let Some(Asking::Passphrase { action, twice }) = self.asking.clone() else {
            return Press::Ignored;
        };
        assert_eq!(twice, again.is_some(), "twice where none is set, else once");
        if self.working.is_some() {
            return Press::Ignored;
        }
        let typed = match again {
            Some(again) => passphrase(first, again).map_err(|said| said.map(str::to_owned)),
            None => Passphrase::typed(first).ok_or(None),
        };
        match typed {
            Ok(typed) => {
                self.passphrase_refusal = None;
                self.run(&action, Input::passphrase(typed))
            }
            Err(said) => {
                self.passphrase_refusal = said;
                Press::Ignored
            }
        }
    }

    /// The confirm was accepted.
    pub fn confirmed(&mut self) -> Press {
        let Some(Asking::Confirm { action, input }) = self.asking.clone() else {
            return Press::Ignored;
        };
        if self.working.is_some() {
            return Press::Ignored;
        }
        self.run(&action, input)
    }

    /// A dialog closed unanswered: nothing is sent.
    pub fn dismissed(&mut self) {
        if !matches!(self.asking, Some(Asking::Token(_))) {
            self.asking = None;
        }
        self.passphrase_refusal = None;
    }

    /// The act answered: the summary after it is drawn and read again, its result kept, and its
    /// notice raised once. A refusal is said under the action, or under the field for a token
    /// the engine did not take; `changed` is said where the press was.
    pub fn acted(&mut self, answer: Result<Acted, CallError>) -> Option<Notice> {
        let action = self.working.take().expect("an act was on its way");
        self.due_status = true;
        let acted = match answer {
            Ok(acted) => acted,
            Err(error) => {
                self.refused(action, &error);
                return None;
            }
        };
        self.asking = None;
        self.refusal = None;
        self.token_refusal = None;
        let fresh = self.take(acted.summary);
        match acted.result {
            Some(ActResult::OwnServer(code)) => self.handout = Some(code),
            Some(ActResult::Moved { .. }) => self.restart = Some(Restart { away: false }),
            None => {}
        }
        fresh
    }

    fn refused(&mut self, action: ActionId, error: &CallError) {
        let said = sentence(error);
        let changed = matches!(error, CallError::Refused(r) if r.code == "changed");
        let field = matches!(self.asking, Some(Asking::Token(_))) && action == ActionId::PasteToken;
        if field && !changed {
            self.token_refusal = Some(said);
        } else {
            self.refusal = Some(said);
        }
    }
}
