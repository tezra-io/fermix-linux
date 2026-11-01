//! The Always On pane's state between reads: the last summary answered, the providers and the
//! provisioner's record, what the owner chose and typed, the action on its way and what it
//! answered, and the steps the app keeps itself. Every decision the pane makes is a method here,
//! so the window only draws what this says and runs the calls it hands over.
//!
//! When to read is decided here too, and never by a clock: `home.status` is read when the pane is
//! shown (the `off` line included, which the engine never says changed), after this app's own
//! step, when the engine says the summary changed (`home_status_changed`, and a desktop's server
//! out of reach or no longer knowing it, where no notice can arrive), and when a new engine
//! answers while the pane shows. Each of those marks a read due; the window asks `next_read`
//! until it answers none, so reads land in order and the notices that come while one is out are
//! one more read, not one each. While Always On is off and the pane is closed nothing is read.

use super::actions::{Context, Outcome};
use super::notices::{notice, Notice, Reading};
use super::{
    guide_at, own_server_command, page, serves_provisioner, token_target, Local, Page, Screen,
    ScreenKind, TokenTarget, GUIDE_URL, LINK_FAILED,
};
use crate::home::{
    passphrase, ActionId, Code, HomeStatus, Passphrase, ReadinessId, ReadinessItem, SummaryState,
};
use crate::provision::{Options, Pick, Provider, ProvisionView};
use crate::view::DaemonProblem;

/// How long, in seconds, the app's own moving screen outlasts the engine's silence on it, judged
/// at each read: no timer reads to end it.
pub const OWN_MOVE: i64 = 600;

/// One read the pane makes, as `next_read` hands them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Read {
    /// `hello`'s methods, then `home.status` where it lists it.
    Status,
    /// `home.provision.providers`.
    Providers,
    /// `home.provision.resources`: the provisioner's record, read back from the provider.
    Record,
    /// `home.provision.options` of the provider in use.
    Options(String),
}

/// The reads asked for and not yet handed out.
#[derive(Debug, Default)]
struct Due {
    /// `home.status`, once however many asked for it.
    status: bool,
    /// The pane was shown: once the status is read, the providers where unread and the record
    /// where the summary rests on it.
    pane: bool,
    /// Open console waits on the record and the providers.
    console: bool,
}

/// Where an action goes once pressed.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
    /// A step the app keeps itself changed the screen.
    Drawn,
    /// What the step needs is read first: the window asks `next_read`.
    Read,
    /// A page to open in the browser.
    Open(String),
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
    /// The providers the token screen draws, read as the pane shows, forgotten as it goes.
    providers: Option<Result<Vec<Provider>, String>>,
    /// The provisioner's record as connect or resources last answered it, which names the
    /// provider in use; forgotten as the pane goes.
    record: Option<Result<ProvisionView, String>>,
    /// What the provider in use sells, read for Create server, Change and Replace.
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
    /// When this app's own move started, in Unix seconds.
    own_move_since: Option<i64>,
    due: Due,
    /// A console Open console waited on the reads for, which the window opens.
    opening: Option<String>,
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

    pub fn serves_provisioner(&self) -> bool {
        serves_provisioner(&self.served)
    }

    /// The provider the provisioner's record names, where one was answered.
    pub fn provider_in_use(&self) -> Option<&str> {
        match &self.record {
            Some(Ok(view)) => Some(&view.provider),
            _ => None,
        }
    }

    /// Which provider's token `screen`'s field takes: the providers as read, the one picked, and
    /// on a problem the one in use.
    pub fn token_target(&self, screen: &Screen) -> TokenTarget {
        let providers = match &self.providers {
            None => return TokenTarget::Unknown(None),
            Some(Err(why)) => return TokenTarget::Unknown(Some(why.clone())),
            Some(Ok(providers)) => providers,
        };
        if let (ScreenKind::Problem(_), Some(Err(why))) = (screen.kind, &self.record) {
            return TokenTarget::Unknown(Some(why.clone()));
        }
        let picked = self.local.provider.as_deref();
        token_target(screen.kind, providers, picked, self.provider_in_use())
    }

    /// The token target of the screen shown now, where it has the field.
    pub fn token_target_now(&self) -> Option<TokenTarget> {
        let screen = self.page().screen().filter(|s| s.token_field)?;
        Some(self.token_target(&screen))
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
        self.settle(&current.status, at);
        self.last = Some(current);
        self.failure = None;
        told
    }

    /// A read was not answered: the screen stays as it was and says so.
    pub fn unanswered(&mut self, problem: DaemonProblem) {
        self.failure = Some(problem);
    }

    /// Lets go of each step the app keeps once the engine's summary has moved past it. The
    /// app's own moving screen goes when the engine says where the move is, or at the first read
    /// past its bound.
    fn settle(&mut self, status: &HomeStatus, at: i64) {
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
        let past = self
            .own_move_since
            .is_some_and(|since| at - since > OWN_MOVE);
        if super::says_where_the_move_stands(state) || past {
            self.local.move_started = None;
            self.own_move_since = None;
        }
    }

    /// Whether the summary rests on what the provider last said of the server, so opening the
    /// pane reads it back first.
    pub fn needs_reconcile(&self) -> bool {
        let state = self
            .status()
            .and_then(|s| s.summary.as_ref())
            .map(|s| s.state);
        let rests = matches!(
            state,
            Some(SummaryState::Problem | SummaryState::Creating | SummaryState::Ready)
        );
        rests && self.serves_provisioner()
    }

    /// Whether the choose screen shows, which reads what the provider sells.
    pub fn choosing(&self) -> bool {
        let summary = self.status().and_then(|s| s.summary.as_ref());
        summary.is_some_and(|s| s.state == SummaryState::Choose)
    }

    /// The pane appeared: it reads the summary, the providers and, where the summary rests on
    /// it, the record.
    pub fn pane_shown(&mut self) {
        self.pane_visible = true;
        self.due.status = true;
        self.due.pane = true;
    }

    /// The pane went: the code it showed and the provider picked on it leave memory with it,
    /// and what it read of the provider is read again as it shows again.
    pub fn pane_hidden(&mut self) {
        self.pane_visible = false;
        self.handout = None;
        self.local.provider = None;
        self.providers = None;
        self.record = None;
        self.options = None;
        self.opening = None;
        self.due.pane = false;
        self.due.console = false;
    }

    /// The engine said the summary changed, or a desktop's server cannot be reached or no
    /// longer knows it: one read of it.
    pub fn home_status_changed(&mut self) {
        self.due.status = true;
    }

    /// A new engine answers here (a restart, this app's own move, Fermix started again). Its
    /// providers and record may differ, so they go; it is read now only while the pane shows.
    pub fn engine_restarted(&mut self) {
        self.providers = None;
        self.record = None;
        if self.pane_visible {
            self.due.status = true;
            self.due.pane = true;
        }
    }

    /// The next read due, if any, in the order the pane needs them: the summary first, then
    /// what the screen shown rests on. Each one is handed out once; what a later trigger asks
    /// for is due again.
    pub fn next_read(&mut self) -> Option<Read> {
        if std::mem::take(&mut self.due.status) {
            return Some(Read::Status);
        }
        if !self.pane_visible {
            return None;
        }
        if self.due.pane {
            if self.serves_provisioner() && self.providers.is_none() {
                return Some(Read::Providers);
            }
            self.due.pane = false;
            if self.needs_reconcile() {
                self.due.status = true;
                return Some(Read::Record);
            }
        }
        self.screen_read()
    }

    /// What the screen shown waits on: the record where a new token or Open console takes the
    /// provider in use, or the offers are read; the providers for Open console; and what the
    /// provider in use sells on choose and for Replace. A failed read ends what waited on it.
    fn screen_read(&mut self) -> Option<Read> {
        if !self.serves_provisioner() {
            return None;
        }
        let offers = self.wants_options();
        let wants_record = offers || self.local.pasting_token || self.due.console;
        if wants_record && self.record.is_none() {
            return Some(Read::Record);
        }
        if self.due.console {
            if self.providers.is_none() {
                return Some(Read::Providers);
            }
            self.due.console = false;
            self.console_read();
        }
        if !offers {
            return None;
        }
        match &self.record {
            Some(Ok(view)) => Some(Read::Options(view.provider.clone())),
            Some(Err(why)) => {
                let why = why.clone();
                self.options_read(Err(why));
                None
            }
            None => unreachable!("the record is read before the offers"),
        }
    }

    fn wants_options(&self) -> bool {
        (self.choosing() || self.replacing) && self.options.is_none()
    }

    pub fn providers_read(&mut self, answer: Result<Vec<Provider>, String>) {
        self.providers = Some(answer);
    }

    pub fn record_read(&mut self, answer: Result<ProvisionView, String>) {
        self.record = Some(answer);
    }

    /// What the provider sells, or why not; Replace's dialog does not open on a failed read,
    /// which is said under the action.
    pub fn options_read(&mut self, answer: Result<Options, String>) {
        if let (Err(why), true) = (&answer, self.replacing) {
            self.replacing = false;
            self.refusal = Some(why.clone());
        }
        self.options = Some(answer);
    }

    /// The console of the provider in use, which the record names and the providers give the
    /// address of; None while either is unread.
    fn console(&self) -> Option<Result<String, String>> {
        let in_use = match self.record.as_ref()? {
            Ok(view) => view.provider.as_str(),
            Err(why) => return Some(Err(why.clone())),
        };
        let providers = match self.providers.as_ref()? {
            Ok(providers) => providers,
            Err(why) => return Some(Err(why.clone())),
        };
        let console = providers.iter().find(|p| p.id == in_use);
        Some(
            console
                .map(|p| p.console_url.clone())
                .ok_or_else(|| LINK_FAILED.to_owned()),
        )
    }

    /// Open console's reads are in: the console to open, or why not.
    fn console_read(&mut self) {
        match self.console() {
            Some(Ok(url)) => self.opening = Some(url),
            Some(Err(why)) => self.refusal = Some(why),
            None => unreachable!("the console is judged once both are read"),
        }
    }

    /// The console Open console waited on the reads for, once.
    pub fn take_opening(&mut self) -> Option<String> {
        self.opening.take()
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
            ActionId::PasteToken => return self.paste_token(),
            ActionId::ReplaceServer => return self.open_replace(),
            ActionId::LearnMore => return Press::Open(self.guide()),
            ActionId::OpenConsole => return self.open_console(),
            ActionId::Update => return Press::Update,
            ActionId::OwnServer => return self.get_code(),
            ActionId::Unknown => return Press::Ignored,
            daemon => return Press::Run(Box::new(self.run(daemon, None))),
        }
        Press::Drawn
    }

    /// Paste a new token opens the field in place, for the provider in use, whose record is read
    /// where none was.
    fn paste_token(&mut self) -> Press {
        self.local.pasting_token = true;
        match self.record {
            Some(_) => Press::Drawn,
            None => Press::Read,
        }
    }

    /// Replace's dialog opens once what the provider sells is read.
    fn open_replace(&mut self) -> Press {
        self.replacing = true;
        match self.loaded_options() {
            Some(_) => Press::Drawn,
            None => Press::Read,
        }
    }

    /// The docs page, at the provider's own section on its token screen.
    fn guide(&self) -> String {
        match self.token_target_now() {
            Some(TokenTarget::Provider(provider)) => guide_at(&provider.docs_anchor),
            _ => GUIDE_URL.to_owned(),
        }
    }

    /// Opens the console of the provider in use, once the record and the providers are read.
    fn open_console(&mut self) -> Press {
        match self.console() {
            Some(Ok(url)) => Press::Open(url),
            Some(Err(why)) => {
                self.refusal = Some(why);
                Press::Drawn
            }
            None => {
                self.due.console = true;
                Press::Read
            }
        }
    }

    /// The provider whose token screen shows, where the engine offers more than one.
    pub fn pick_provider(&mut self, id: &str) {
        assert!(!id.is_empty(), "a provider is picked by its id");
        self.local.provider = Some(id.to_owned());
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
            provider: self.provider_in_use().map(str::to_owned),
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

    /// An action ended at `at` (Unix seconds): what it answered, and the read of what it changed.
    pub fn finished(&mut self, outcome: Outcome, at: i64) {
        self.working = None;
        self.due.status = true;
        self.refusal = outcome.refusal;
        self.notice = outcome.notice;
        if let Some(options) = outcome.options {
            self.options = Some(options);
        }
        if let Some(view) = outcome.record {
            self.record = Some(Ok(view));
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
            self.own_move_since = Some(at);
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
        self.due.status = true;
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

    /// `home.provision.connect` answered: the token works, and its answer is the record that
    /// names the provider from then on; or the daemon's refusal goes under the field.
    pub fn token_checked(&mut self, answer: Result<ProvisionView, String>) {
        self.checking_token = false;
        self.due.status = true;
        match answer {
            Ok(view) => {
                self.token_refusal = None;
                self.local.pasting_token = false;
                self.record = Some(Ok(view));
                self.options = None;
            }
            Err(refused) => self.token_refusal = Some(refused),
        }
    }

    /// `home.enroll.open` answered: the code for the owner's own server, or why not. A code no
    /// shell line can carry is a broken answer, said as one.
    pub fn code_answered(&mut self, answer: Result<Code, String>) {
        self.getting_code = false;
        self.due.status = true;
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
