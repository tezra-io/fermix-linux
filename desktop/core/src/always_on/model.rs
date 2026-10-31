//! The Always On pane's state between reads: the last summary answered, what the owner chose and
//! typed, the action on its way and what it answered, and the steps the app keeps itself. Every
//! decision the pane makes is a method here, so the window only draws what this says and runs
//! the calls it hands over.

use super::actions::{Context, Outcome};
use super::notices::{notice, wait, Notice, Reading, SETTLED};
use super::{
    own_server_command, page, serves_hetzner, Local, Page, Screen, ScreenKind, GUIDE_URL,
    HETZNER_CONSOLE,
};
use crate::home::{
    passphrase, ActionId, Code, HomeStatus, Passphrase, ReadinessId, ReadinessItem, SummaryState,
};
use crate::provision::{Options, Pick, ProvisionView};
use crate::view::DaemonProblem;
use std::time::Duration;

/// How many answered reads the app's own moving screen outlasts the engine's silence on it:
/// about ten minutes at the brisk pace.
pub const OWN_MOVE_READS: u32 = 200;
const STATUS_METHOD: &str = "home.status";

/// Where an action goes once pressed.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
    /// A step the app keeps itself changed the screen.
    Drawn,
    /// A page to open in the browser.
    Open(&'static str),
    /// The app's own update.
    Update,
    /// The code for a server of the owner's own, `home.enroll.open`.
    GetCode,
    /// The calls `actions::perform` makes, off the window's thread.
    Run(Box<Context>),
    /// Another action is on its way, or this app has no button for it.
    Ignored,
}

#[derive(Debug, Default)]
pub struct Model {
    /// The methods `hello` lists, empty until it is read.
    served: Vec<String>,
    /// The last answered read, which the next one is compared with.
    last: Option<Reading>,
    /// The last read's failure, while it is the latest word.
    failure: Option<DaemonProblem>,
    pub local: Local,
    /// What Hetzner sells, read for Create server, Change and Replace.
    pub options: Option<Result<Options, String>>,
    /// The size and location the owner chose; the recommendation until then.
    chosen: Option<Pick>,
    /// The action on its way, which holds the screen's buttons.
    pub working: Option<ActionId>,
    /// The daemon's refusal of the last action, said under it.
    pub refusal: Option<String>,
    /// A word that is no failure: a move's start whose answer was lost.
    pub notice: Option<String>,
    pub checking_token: bool,
    pub token_refusal: Option<String>,
    pub setting_passphrase: bool,
    pub passphrase_refusal: Option<String>,
    /// The warnings and blocks of the last review this app asked for, which the ready screen
    /// lists where the summary carries none: during a replace the review is the old server's.
    pub reviewed: Vec<ReadinessItem>,
    /// Move now waits for the recovery passphrase, which a dialog asks for.
    pub asks_move_passphrase: bool,
    /// Replace the server's dialog is up.
    pub replacing: bool,
    /// The code for a server of the owner's own, shown once, while the pane shows it.
    pub handout: Option<Code>,
    pub getting_code: bool,
    pub code_refusal: Option<String>,
    pub pane_visible: bool,
    /// The recovery passphrase typed for this move, held only until the move starts:
    /// `home.move.start` takes it again, and the engine never stores it.
    passphrase: Option<Passphrase>,
    /// Reads answered since this app's own move started.
    own_move_reads: u32,
}

impl Model {
    pub fn page(&self) -> Page {
        page(
            self.status(),
            self.failure.as_ref(),
            &self.local,
            &self.served,
        )
    }

    pub fn status(&self) -> Option<&HomeStatus> {
        self.last.as_ref().map(|r| &r.status)
    }

    pub fn serves_hetzner(&self) -> bool {
        serves_hetzner(&self.served)
    }

    /// Whether `hello` lists `home.status`, so the follow reads it.
    pub fn serves_status(&self) -> bool {
        self.served.iter().any(|m| m == STATUS_METHOD)
    }

    /// The last read's failure under a screen drawn from an earlier answer.
    pub fn read_note(&self) -> Option<String> {
        self.last.as_ref()?;
        self.failure.as_ref().map(DaemonProblem::sentence)
    }

    // ---- Reads -------------------------------------------------------------------------

    pub fn served(&mut self, methods: Vec<String>) {
        self.served = methods;
    }

    /// A read answered at `at` (Unix seconds): the notification the change calls for, if any.
    pub fn answered(&mut self, status: HomeStatus, at: i64) -> Option<Notice> {
        let current = Reading { status, at };
        let told = notice(
            self.last.as_ref(),
            &current,
            self.local.move_started.is_some(),
        );
        self.settle(&current.status);
        self.last = Some(current);
        self.failure = None;
        told
    }

    /// A read was not answered: the screen stays as it was and says so.
    pub fn unanswered(&mut self, problem: DaemonProblem) {
        self.failure = Some(problem);
    }

    /// Lets go of each step the app keeps once the engine's summary has moved past it. The
    /// app's own moving screen goes when the engine says where the move is, or past its bound.
    fn settle(&mut self, status: &HomeStatus) {
        let summary = status.summary.as_ref();
        let state = summary.map(|s| s.state);
        if state != Some(SummaryState::Off) {
            self.local.setting_up = false;
        }
        if state != Some(SummaryState::Ready) {
            self.reviewed.clear();
        }
        let offers_paste = summary
            .and_then(|s| s.action.as_ref())
            .is_some_and(|a| a.id == ActionId::PasteToken);
        if !offers_paste {
            self.local.pasting_token = false;
        }
        if self.local.move_started.is_none() {
            return;
        }
        self.own_move_reads += 1;
        if super::says_where_the_move_stands(state) || self.own_move_reads > OWN_MOVE_READS {
            self.local.move_started = None;
            self.own_move_reads = 0;
        }
    }

    /// The wait before the next read. Until `hello` answers, which fails while the daemon
    /// starts, the next read comes at the settled pace, so a launch during a create is followed
    /// within a minute.
    pub fn next_wait(&self) -> Duration {
        if self.served.is_empty() {
            return SETTLED;
        }
        let serves = self.serves_status();
        wait(
            self.status().filter(|_| serves),
            self.failure.is_some() || !serves,
            self.pane_visible,
            self.local.move_started.is_some(),
        )
    }

    /// Whether the summary rests on what Hetzner last said of the server, so opening the pane
    /// reads it back first.
    pub fn needs_reconcile(&self) -> bool {
        let state = self
            .status()
            .and_then(|s| s.summary.as_ref())
            .map(|s| s.state);
        let rests = matches!(
            state,
            Some(SummaryState::Problem | SummaryState::Creating | SummaryState::Ready)
        );
        rests && self.serves_hetzner()
    }

    /// Whether the choose screen shows, which reads what Hetzner sells.
    pub fn choosing(&self) -> bool {
        let summary = self.status().and_then(|s| s.summary.as_ref());
        summary.is_some_and(|s| s.state == SummaryState::Choose)
    }

    pub fn pane_shown(&mut self) {
        self.pane_visible = true;
    }

    /// The pane went: the code it showed leaves memory with it.
    pub fn pane_hidden(&mut self) {
        self.pane_visible = false;
        self.handout = None;
    }

    pub fn options_read(&mut self, answer: Result<Options, String>) {
        self.options = Some(answer);
    }

    // ---- The choice ------------------------------------------------------------------

    /// The size and location Create server makes: the owner's choice, else the recommendation.
    pub fn pick(&self) -> Option<Pick> {
        let recommended = || self.loaded_options()?.recommended.clone();
        self.chosen.clone().or_else(recommended)
    }

    pub fn choose(&mut self, pick: Pick) {
        self.chosen = Some(pick);
    }

    pub fn loaded_options(&self) -> Option<&Options> {
        self.options.as_ref()?.as_ref().ok()
    }

    /// The choose screen says the choice: the engine's recommendation, or the owner's own once
    /// changed. Every other screen says the engine's sentence.
    pub fn choose_sentence(&self, screen: &Screen) -> String {
        let Some(options) = self
            .loaded_options()
            .filter(|_| screen.kind == ScreenKind::Choose)
        else {
            return screen.sentence.clone();
        };
        let changed = self
            .chosen
            .as_ref()
            .filter(|c| Some(*c) != options.recommended.as_ref());
        match changed.and_then(|pick| options.offer(pick)) {
            Some(offer) => super::choice_sentence(offer, options),
            None => options.sentence.clone(),
        }
    }

    /// The warnings and blocks the ready screen lists: the summary's, else those of the review
    /// this app asked for.
    pub fn readiness(&self, screen: &Screen) -> Vec<ReadinessItem> {
        let warnings = screen.warnings();
        if warnings.is_empty() {
            self.reviewed.clone()
        } else {
            warnings
        }
    }

    // ---- Actions ---------------------------------------------------------------------

    /// One action the screen offers was pressed.
    pub fn press(&mut self, id: ActionId) -> Press {
        if self.working.is_some() {
            return Press::Ignored;
        }
        self.refusal = None;
        self.notice = None;
        match id {
            ActionId::SetUp => self.local.setting_up = true,
            ActionId::PasteToken => self.local.pasting_token = true,
            ActionId::ReplaceServer => self.replacing = true,
            ActionId::LearnMore => return Press::Open(GUIDE_URL),
            ActionId::OpenHetzner => return Press::Open(HETZNER_CONSOLE),
            ActionId::Update => return Press::Update,
            ActionId::OwnServer => return self.get_code(),
            ActionId::Unknown => return Press::Ignored,
            daemon => return Press::Run(Box::new(self.run(daemon, None))),
        }
        Press::Drawn
    }

    fn get_code(&mut self) -> Press {
        if self.getting_code {
            return Press::Ignored;
        }
        self.getting_code = true;
        self.code_refusal = None;
        Press::GetCode
    }

    /// Holds the buttons for `action`, and hands over what it needs.
    fn run(&mut self, action: ActionId, chosen: Option<Pick>) -> Context {
        self.working = Some(action);
        Context {
            action,
            status: self.status().cloned(),
            options: self.loaded_options().cloned(),
            chosen: chosen.or_else(|| self.chosen.clone()),
            passphrase: self.passphrase.clone(),
            shown: self.shown(),
        }
    }

    /// The ids of the warnings the screen shows now.
    fn shown(&self) -> Vec<ReadinessId> {
        let Some(screen) = self.page().screen() else {
            return Vec::new();
        };
        self.readiness(&screen).into_iter().map(|i| i.id).collect()
    }

    /// An action ended: what it answered.
    pub fn finished(&mut self, outcome: Outcome) {
        self.working = None;
        self.refusal = outcome.refusal;
        self.notice = outcome.notice;
        if let Some(options) = outcome.options {
            self.options = Some(options);
        }
        if let Some(reviewed) = outcome.reviewed {
            self.reviewed = reviewed;
        }
        self.asks_move_passphrase |= outcome.asks_passphrase;
        if outcome.passphrase_spent {
            self.passphrase = None;
        }
        if let Some(started) = outcome.move_started {
            self.local.move_started = Some(started);
            self.own_move_reads = 0;
        }
    }

    /// Creates the new server for the one the home lives on, from Replace's choice; the move to
    /// it is the same Move now (L15).
    pub fn replace_with(&mut self, pick: Pick) -> Option<Context> {
        if self.working.is_some() {
            return None;
        }
        self.replacing = false;
        self.refusal = None;
        Some(self.run(ActionId::ReplaceServer, Some(pick)))
    }

    pub fn close_replace(&mut self) {
        self.replacing = false;
    }

    // ---- The passphrase, the token and the code -----------------------------------------

    /// The passphrase the two entries agree on, which the pane then sets; the mismatch is said
    /// under the fields.
    pub fn passphrase_typed(&mut self, first: &str, second: &str) -> Option<Passphrase> {
        match passphrase(first, second) {
            Ok(typed) => {
                self.passphrase_refusal = None;
                Some(typed)
            }
            Err(said) => {
                self.passphrase_refusal = said.map(str::to_owned);
                None
            }
        }
    }

    /// `home.snapshots.passphrase` answered. A set passphrase is held for the move, which takes
    /// it again.
    pub fn passphrase_set(&mut self, typed: Passphrase, answer: Result<(), String>) {
        self.setting_passphrase = false;
        match answer {
            Ok(()) => {
                self.passphrase_refusal = None;
                self.passphrase = Some(typed);
            }
            Err(refused) => self.passphrase_refusal = Some(refused),
        }
    }

    /// The passphrase Move now asked for, then the move.
    pub fn move_with_passphrase(&mut self, first: &str, second: &str) -> Option<Context> {
        let typed = self.passphrase_typed(first, second)?;
        if self.working.is_some() {
            return None;
        }
        self.refusal = None;
        self.notice = None;
        self.passphrase = Some(typed);
        self.asks_move_passphrase = false;
        Some(self.run(ActionId::MoveNow, None))
    }

    pub fn dismiss_move_passphrase(&mut self) {
        self.asks_move_passphrase = false;
        self.passphrase_refusal = None;
    }

    /// `home.provision.connect` answered: the token works, or the daemon's refusal goes under
    /// the field.
    pub fn token_checked(&mut self, answer: Result<ProvisionView, String>) {
        self.checking_token = false;
        match answer {
            Ok(_) => {
                self.token_refusal = None;
                self.local.pasting_token = false;
            }
            Err(refused) => self.token_refusal = Some(refused),
        }
    }

    /// `home.enroll.open` answered: the code for the owner's own server, or why not. A code no
    /// shell line can carry is a broken answer, said as one.
    pub fn code_answered(&mut self, answer: Result<Code, String>) {
        self.getting_code = false;
        match answer {
            Ok(code) if own_server_command(&code).is_some() => {
                self.code_refusal = None;
                self.handout = Some(code);
            }
            Ok(_) => {
                let broken = DaemonProblem::Broken("home.enroll.open answered an odd code".into());
                self.code_refusal = Some(broken.sentence());
            }
            Err(refused) => self.code_refusal = Some(refused),
        }
    }
}
