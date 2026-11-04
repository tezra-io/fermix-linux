//! The Always On pane between reads: the last summary answered, what a press is waiting on (the
//! passphrase or the confirm), the act on its way and what it answered, and the restart after
//! this app's own Move now. Every decision the pane makes is a method here, so the window draws
//! what this says and sends what it hands over.
//!
//! A press collects what the action `needs`, asks its `confirm` (a dialog that collects a need
//! shows the confirm and its answer accepts it), then hands over one `home.act` with the key the
//! action was drawn with. A step held between two reads goes once the summary no longer offers
//! its action under that key, which is the engine's own rule for a press.
//!
//! When to read is decided here too, and never by a clock: `home.status` when the pane is shown,
//! after each act, when the engine says the summary changed (`home_status_changed`), and when a
//! new engine answers while the pane shows. Each marks a read due; the window asks `take_read`
//! until it answers false, so the notices that come while a read is out are one more read, not
//! one each.

use super::notices::Posted;
use super::{
    offered, restarting, says_where_the_move_stands, screen, sentence, Page, LINK_FAILED, NEEDED,
};
use crate::home::{
    passphrase, ActResult, Acted, Action, ActionId, HomeStatus, Input, Need, Notice, OwnServer,
    Passphrase, Summary, SummaryState,
};
use crate::management::CallError;
use crate::view::DaemonProblem;

/// Where a press goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Press {
    /// The screen changed: a dialog or a sentence to draw.
    Drawn,
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
    /// The recovery passphrase, twice where none is set, with the action's confirm.
    Passphrase { action: Action, twice: bool },
    /// The action's confirm.
    Confirm(Action),
}

/// What a press waits on.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Asking {
    Passphrase {
        action: Action,
        twice: bool,
    },
    /// An action that needs nothing, asked first.
    Confirm(Action),
}

impl Asking {
    fn action(&self) -> &Action {
        match self {
            Asking::Passphrase { action, .. } | Asking::Confirm(action) => action,
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
    asking: Option<Asking>,
    /// The act on its way, which holds every press.
    pub working: Option<ActionId>,
    /// The daemon's refusal of the last act, said under the action.
    pub refusal: Option<String>,
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
        Page::Screen(Box::new(screen(summary)))
    }

    pub fn summary(&self) -> Option<&Summary> {
        self.last.as_ref()
    }

    /// The last read's failure under a screen drawn from an earlier answer.
    pub fn read_note(&self) -> Option<String> {
        self.last.as_ref()?;
        self.failure.as_ref().map(DaemonProblem::sentence)
    }

    pub fn dialog(&self) -> Option<Dialog> {
        match self.asking.as_ref()? {
            Asking::Passphrase { action, twice } => Some(Dialog::Passphrase {
                action: action.clone(),
                twice: *twice,
            }),
            Asking::Confirm(action) => Some(Dialog::Confirm(action.clone())),
        }
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
    /// its key, and the code once it is no longer open.
    fn take(&mut self, summary: Summary) -> Option<Notice> {
        if self
            .asking
            .as_ref()
            .is_some_and(|a| !summary.offers(a.action()))
        {
            self.asking = None;
            self.passphrase_refusal = None;
        }
        if summary.state != SummaryState::Connect {
            self.handout = None;
        }
        let fresh = summary.notice.clone().filter(|n| self.posted.fresh(n));
        self.last = Some(summary);
        fresh
    }

    /// The pane appeared: it reads the summary.
    pub fn pane_shown(&mut self) {
        self.pane_visible = true;
        self.due_status = true;
    }

    /// The pane went: the code it showed and what was asked on it go with it.
    pub fn pane_hidden(&mut self) {
        self.pane_visible = false;
        self.handout = None;
        self.asking = None;
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

    /// Whether a read of the summary is due: handed out once, and due again at a later trigger.
    pub fn take_read(&mut self) -> bool {
        std::mem::take(&mut self.due_status)
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
            ActionId::LearnMore => return self.open(action),
            _ => {}
        }
        match action.needs.first() {
            Some(Need::RecoveryPassphrase) => self.ask_passphrase(action, false),
            Some(Need::NewRecoveryPassphrase) => self.ask_passphrase(action, true),
            Some(Need::Unknown) => unreachable!("an action with a need it cannot ask is not drawn"),
            None => self.confirm_or_run(action),
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

    /// An action that needs nothing: its confirm first where it has one, else its act now.
    fn confirm_or_run(&mut self, action: &Action) -> Press {
        if action.confirm.is_some() {
            self.asking = Some(Asking::Confirm(action.clone()));
            return Press::Drawn;
        }
        self.run(action, Input::none())
    }

    /// Holds every press for `action`, and hands over its act; a dialog's ask is answered.
    fn run(&mut self, action: &Action, input: Input) -> Press {
        self.working = Some(action.id);
        self.asking = None;
        Press::Run(Act {
            action: action.id,
            key: action.key.clone(),
            input,
        })
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
        let Some(Asking::Confirm(action)) = self.asking.clone() else {
            return Press::Ignored;
        };
        if self.working.is_some() {
            return Press::Ignored;
        }
        self.run(&action, Input::none())
    }

    /// A dialog closed unanswered: nothing is sent.
    pub fn dismissed(&mut self) {
        self.asking = None;
        self.passphrase_refusal = None;
    }

    /// The act answered: the summary after it is drawn and read again, its result kept, and its
    /// notice raised once. A refusal, `changed` among them, is said under the action.
    pub fn acted(&mut self, answer: Result<Acted, CallError>) -> Option<Notice> {
        let acting = self.working.take();
        assert!(acting.is_some(), "an act was on its way");
        self.due_status = true;
        let acted = match answer {
            Ok(acted) => acted,
            Err(error) => {
                self.refusal = Some(sentence(&error));
                return None;
            }
        };
        self.asking = None;
        self.refusal = None;
        let fresh = self.take(acted.summary);
        match acted.result {
            Some(ActResult::OwnServer(code)) => self.handout = Some(code),
            Some(ActResult::Moved { .. }) => self.restart = Some(Restart { away: false }),
            None => {}
        }
        fresh
    }
}
