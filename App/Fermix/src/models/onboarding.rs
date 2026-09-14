//! The Setup assistant, as data.
//!
//! Seven screens, two ladders, one finish gate. The machine here has no socket,
//! no clock and no window: the activation transaction is
//! [`super::activation`], the daemon's answers arrive through the one settings
//! model, and every screen draws what this value says.
//!
//! The finish gate is the daemon's, in one place (M38 section 5.5): the daemon
//! is live, no gating readiness failure remains, and no restart is pending.
//! Closing the assistant never marks completion, so Home goes on saying setup
//! is required until the daemon itself says otherwise.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gtk4::glib;

use crate::copy::Key;
use crate::management::types::{SettingValue, SettingsPane, SettingsRow};
use crate::management::vocabulary::Readiness;

use super::activation::{Activation, Outcome, Refusal, Step, StepState};
use super::settings_model::{Sentence, SettingsModel};
use super::{spawn, Observers};

/// The section About you writes, and the four keys it writes into.
///
/// Named here and nowhere else. This is the one hand-built form in the
/// application that does not read its shape off `settings.get`: the values it
/// collects are exactly the ones whose absence keeps the assistant open, and
/// the assistant exists before the pane that publishes them is worth showing.
pub const PERSONALIZATION: &str = "personalization";
/// The person's own name.
pub const NAME_KEY: &str = "user_name";
/// Their time zone.
pub const TIMEZONE_KEY: &str = "timezone";
/// How Fermix answers.
pub const STYLE_KEY: &str = "communication_style";
/// What they call the assistant. It is a personalization row, not a second
/// section: there is no `agent` section on the wire.
pub const ASSISTANT_KEY: &str = "bot_name";

/// The screens, in the order a person meets them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Welcome,
    Starting,
    ConnectAi,
    AboutYou,
    Applying,
    Ready,
    /// Replaces Starting when the transaction ends in one of its named causes.
    BootFailed,
}

impl Stage {
    /// The page title.
    pub fn title(self) -> Key {
        match self {
            Stage::Welcome => Key::SetupWelcomeTitle,
            Stage::Starting => Key::SetupStartingTitle,
            Stage::ConnectAi => Key::SetupConnectAiTitle,
            Stage::AboutYou => Key::SetupAboutYouTitle,
            Stage::Applying => Key::SetupApplyingTitle,
            Stage::Ready => Key::SetupReadyTitle,
            Stage::BootFailed => Key::SetupBootFailedTitle,
        }
    }

    /// The stack page's name.
    pub fn slug(self) -> &'static str {
        match self {
            Stage::Welcome => "welcome",
            Stage::Starting => "starting",
            Stage::ConnectAi => "connect-ai",
            Stage::AboutYou => "about-you",
            Stage::Applying => "applying",
            Stage::Ready => "ready",
            Stage::BootFailed => "boot-failed",
        }
    }

    /// Which of the four dots is lit, where this screen draws them.
    ///
    /// Four, not seven: the two mechanical stages inherit the step they run
    /// inside, and the failure screen carries no dots at all.
    pub fn progress(self) -> Option<usize> {
        match self {
            Stage::Welcome | Stage::Starting => Some(0),
            Stage::ConnectAi => Some(1),
            Stage::AboutYou | Stage::Applying => Some(2),
            Stage::Ready => Some(3),
            Stage::BootFailed => None,
        }
    }

    /// Whether the dots are drawn at all. The two ladders show their own
    /// progress row by row, and a second progress indicator over one is two
    /// answers to one question.
    pub fn draws_progress(self) -> bool {
        !matches!(self, Stage::Starting | Stage::Applying | Stage::BootFailed)
    }

    /// How many dots there are.
    pub const PROGRESS_STEPS: usize = 4;
}

/// The bar's leading control, which is a different thing on the one screen with
/// a transaction running behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leading {
    /// Step back one screen.
    Back,
    /// Leave the assistant. Nothing is marked complete by leaving.
    Leave(Key),
    /// Stop what is running, which is the only way off the ladder.
    Cancel,
}

impl Leading {
    /// The word on the control.
    pub fn key(self) -> Key {
        match self {
            Leading::Back => Key::ActionBack,
            Leading::Leave(key) => key,
            Leading::Cancel => Key::SetupCancel,
        }
    }

    /// Whether Escape reaches it. Stopping a transaction is a decision a person
    /// takes deliberately, so the ladder's way out is a press and nothing else.
    pub fn answers_escape(self) -> bool {
        !matches!(self, Leading::Cancel)
    }
}

/// The one suggested action, where a screen has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    Begin,
    Continue,
    Finish,
    Retry,
}

impl Primary {
    /// The word on the button.
    pub fn key(self) -> Key {
        match self {
            Primary::Begin => Key::SetupWelcomeStart,
            Primary::Continue | Primary::Finish => Key::ActionContinue,
            Primary::Retry => Key::ActionTryAgain,
        }
    }
}

/// Why an advance or a finish was refused.
///
/// A fact about the last attempt rather than a latch: readiness landing
/// afterwards clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Provider,
    Personalization,
    /// Settings changed and the daemon has not taken them yet. Applying is the
    /// screen that takes it, so the block keeps the person there.
    Restart,
    /// Nothing is answering at all, which the assistant cannot clear from any
    /// of its screens.
    DaemonNotLive,
    /// A gating failure on a pane the assistant has no screen for. It carries
    /// the pane the daemon named, because the sentence alone would be a dead
    /// end.
    Elsewhere(SettingsPane),
}

impl Block {
    /// What the screen says about it.
    pub fn key(&self) -> Key {
        match self {
            Block::Provider => Key::SetupBlockedProvider,
            Block::Personalization => Key::SetupBlockedPersonalization,
            Block::Restart => Key::AttentionRestartPendingTitle,
            Block::DaemonNotLive => Key::HomeStatusNotRunning,
            Block::Elsewhere(_) => Key::SetupBlockedElsewhere,
        }
    }

    /// The pane this block sends a person to, where it names one.
    pub fn pane(&self) -> Option<SettingsPane> {
        match self {
            Block::Elsewhere(pane) if *pane != SettingsPane::Unrecognized => Some(*pane),
            _ => None,
        }
    }

    /// The screen that can clear it, where the assistant has one.
    fn stage(&self) -> Option<Stage> {
        match self {
            Block::Provider => Some(Stage::ConnectAi),
            Block::Personalization => Some(Stage::AboutYou),
            Block::Restart | Block::DaemonNotLive | Block::Elsewhere(_) => None,
        }
    }
}

/// One row of the Applying ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyStep {
    Save,
    Restart,
}

impl ApplyStep {
    /// What the row says.
    pub fn key(self) -> Key {
        match self {
            ApplyStep::Save => Key::SetupStepSaveSetup,
            ApplyStep::Restart => Key::SetupStepRestart,
        }
    }
}

/// The four answers About you collects.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answers {
    pub name: String,
    pub timezone: String,
    pub style: String,
    pub assistant: String,
}

impl Answers {
    /// One write, four keys: the daemon takes them whole or refuses them whole.
    pub fn values(&self) -> BTreeMap<String, SettingValue> {
        BTreeMap::from([
            (NAME_KEY.to_string(), SettingValue::Text(self.name.clone())),
            (
                TIMEZONE_KEY.to_string(),
                SettingValue::Text(self.timezone.clone()),
            ),
            (
                STYLE_KEY.to_string(),
                SettingValue::Text(self.style.clone()),
            ),
            (
                ASSISTANT_KEY.to_string(),
                SettingValue::Text(self.assistant.clone()),
            ),
        ])
    }
}

/// Everything one assistant screen draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub stage: Stage,
    pub leading: Option<Leading>,
    pub primary: Option<Primary>,
    pub progress: Option<usize>,
    pub steps: Vec<(Step, StepState)>,
    pub applying: Vec<(ApplyStep, StepState)>,
    pub failure: Option<Refusal>,
    pub block: Option<Block>,
    /// The daemon's own sentence for a refused write.
    pub refusal: Option<Sentence>,
    /// The home a person chose on Welcome, held here and written by nothing but
    /// the command line's own install.
    pub home: Option<PathBuf>,
    /// The advisory failures, which Ready renders as one row.
    pub advisory: usize,
    /// Whether the daemon still reports no provider it can answer with. Connect
    /// your AI draws its rows on this and never on a word of its own.
    pub provider_gap: bool,
}

/// The assistant's own model.
pub struct OnboardingModel {
    settings: Rc<SettingsModel>,
    activation: Activation,
    stage: Cell<Stage>,
    home: RefCell<Option<PathBuf>>,
    steps: RefCell<Vec<(Step, StepState)>>,
    applying: RefCell<Vec<(ApplyStep, StepState)>>,
    failure: RefCell<Option<Refusal>>,
    block: RefCell<Option<Block>>,
    answers: RefCell<Answers>,
    refusal: RefCell<Option<Sentence>>,
    /// Called when the assistant is done with the window, however it ended.
    on_leave: RefCell<Option<Box<dyn Fn()>>>,
    observers: Observers,
}

impl OnboardingModel {
    /// An assistant over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        Self::with_activation(Rc::clone(&settings), Activation::new(settings))
    }

    /// The same assistant over an activation with a different budget, which is
    /// what lets the ladder's own waits be proven in seconds.
    pub fn with_activation(settings: Rc<SettingsModel>, activation: Activation) -> Rc<Self> {
        Rc::new(Self {
            activation,
            settings,
            stage: Cell::new(Stage::Welcome),
            home: RefCell::new(None),
            steps: RefCell::new(fresh_steps()),
            applying: RefCell::new(Vec::new()),
            failure: RefCell::new(None),
            block: RefCell::new(None),
            answers: RefCell::new(Answers::default()),
            refusal: RefCell::new(None),
            on_leave: RefCell::new(None),
            observers: Observers::default(),
        })
    }

    /// Tell me when the assistant moves.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// Tell me when the assistant is done with the window.
    pub fn on_leave(&self, leave: impl Fn() + 'static) {
        self.on_leave.replace(Some(Box::new(leave)));
    }

    /// Which screen is showing.
    pub fn stage(&self) -> Stage {
        self.stage.get()
    }

    /// Everything the screen draws.
    pub fn snapshot(&self) -> Snapshot {
        let stage = self.stage.get();

        Snapshot {
            stage,
            leading: self.leading(stage),
            primary: self.primary(stage),
            progress: stage.draws_progress().then(|| stage.progress()).flatten(),
            steps: self.steps.borrow().clone(),
            applying: self.applying.borrow().clone(),
            failure: self.failure.borrow().clone(),
            block: self.block.borrow().clone(),
            refusal: self.refusal.borrow().clone(),
            home: self.home.borrow().clone(),
            advisory: self.advisory().len(),
            provider_gap: self.gaps().contains(&Block::Provider),
        }
    }

    /// The answers as they stand, which a screen edits row by row.
    pub fn answers(&self) -> Answers {
        self.answers.borrow().clone()
    }

    /// Put one answer in. Nothing is written until Applying.
    pub fn set_answers(&self, answers: Answers) {
        self.answers.replace(answers);
        self.observers.notify();
    }

    /// Fill the four answers from what the daemon already holds, and from this
    /// account where it holds nothing. Zero typing is a valid answer.
    pub fn prefill(&self, rows: &[SettingsRow]) {
        let daemon = |key: &str| -> String {
            rows.iter()
                .find(|row| row.key == key)
                .map(|row| super::super::ui::settings::descriptor_row::as_text(&row.value))
                .unwrap_or_default()
        };

        let mut answers = self.answers.borrow_mut();
        answers.name = first_of(daemon(NAME_KEY), account_name());
        answers.timezone = first_of(daemon(TIMEZONE_KEY), local_timezone());
        answers.style = first_of(daemon(STYLE_KEY), default_style(rows));
        answers.assistant = daemon(ASSISTANT_KEY);
    }

    /// The home a person chose on Welcome, where they chose one.
    pub fn home(&self) -> Option<PathBuf> {
        self.home.borrow().clone()
    }

    /// Hold one home. It is an argument to the command line's own install and
    /// nothing else: this application validates nothing about it and parses
    /// nothing inside it.
    pub fn choose_home(&self, home: &Path) {
        self.home.replace(Some(home.to_path_buf()));
        self.observers.notify();
    }

    // ---- Moving ---------------------------------------------------------

    /// The Welcome action, and the one the Boot failed card offers again.
    pub fn begin(self: &Rc<Self>) {
        if self.activation.is_running() {
            return;
        }

        self.failure.replace(None);
        self.block.replace(None);
        self.steps.replace(fresh_steps());
        self.show(Stage::Starting);

        let model = Rc::clone(self);
        spawn(async move {
            model.activate().await;
        });
    }

    /// Stop the transaction. The only way off the ladder, and it leaves the
    /// assistant rather than stranding a person on a ladder that is not
    /// running.
    pub fn cancel(&self) {
        self.activation.cancel();
        self.leave();
    }

    /// Leave the assistant. Nothing is marked complete by leaving.
    pub fn leave(&self) {
        if let Some(leave) = self.on_leave.borrow().as_ref() {
            leave();
        }
    }

    /// The bar's leading control.
    pub fn back(self: &Rc<Self>) {
        match self.leading(self.stage.get()) {
            Some(Leading::Back) => {
                self.block.replace(None);
                self.show(Stage::ConnectAi);
            }
            Some(Leading::Leave(_)) => self.leave(),
            Some(Leading::Cancel) | None => {}
        }
    }

    /// The bar's one suggested action.
    pub fn advance(self: &Rc<Self>) {
        match self.stage.get() {
            Stage::Welcome => self.begin(),
            Stage::ConnectAi => self.leave_connect_ai(),
            Stage::AboutYou => self.start_applying(true),
            Stage::Ready => self.finish(),
            Stage::BootFailed => self.begin(),
            Stage::Starting | Stage::Applying => {}
        }
    }

    /// Open the assistant at the screen that answers the first gap, which is
    /// what Home's Continue setup does.
    pub fn resume(self: &Rc<Self>) {
        self.block.replace(None);
        self.refusal.replace(None);
        self.enter(self.first_incomplete());
    }

    /// Show one screen, and start what that screen exists to run.
    ///
    /// Applying is the one screen with a transaction behind it rather than a
    /// decision on it: a home that owes nothing but a restart lands there and
    /// the restart is taken. It is not written again — a restart-only entry
    /// keeps the personalization the daemon already has.
    fn enter(self: &Rc<Self>, stage: Stage) {
        if stage == Stage::Applying {
            self.start_applying(false);
            return;
        }
        self.show(stage);
    }

    /// The screen the daemon's own readiness says is the first one still owed.
    pub fn first_incomplete(&self) -> Stage {
        if self.settings.state().reachable != Some(true) {
            return Stage::Welcome;
        }

        match self.gaps().into_iter().find_map(|gap| gap.stage()) {
            Some(stage) => stage,
            None if self.settings.state().restart.required => Stage::Applying,
            None => Stage::Ready,
        }
    }

    /// Connecting an AI is the one required decision, so nothing walks past
    /// this screen without it.
    fn leave_connect_ai(self: &Rc<Self>) {
        if self.gaps().contains(&Block::Provider) {
            self.block.replace(Some(Block::Provider));
            self.observers.notify();
            return;
        }

        self.block.replace(None);
        self.show(Stage::AboutYou);
    }

    /// The three-part gate. A refusal names the missing half and sends the
    /// person to the screen that can clear it.
    pub fn finish(self: &Rc<Self>) {
        let blocked = self.blocked();
        self.block.replace(blocked.clone());

        match blocked {
            None => {
                self.show(Stage::Ready);
                self.leave();
            }
            Some(block) => match block.stage() {
                Some(stage) => self.show(stage),
                None => self.observers.notify(),
            },
        }
    }

    // ---- The two ladders ------------------------------------------------

    async fn activate(self: &Rc<Self>) {
        let home = self.home.borrow().clone();
        let model = Rc::downgrade(self);
        let progress = move |step: Step, state: StepState| {
            if let Some(model) = model.upgrade() {
                model.mark(step, state);
            }
        };

        let outcome = self.activation.run(home, &progress).await;

        match outcome {
            Outcome::Activated => {
                self.block.replace(None);
                self.enter(self.landing());
            }
            Outcome::Refused(refusal) => {
                self.failure.replace(Some(*refusal));
                self.show(Stage::BootFailed);
            }
        }
    }

    /// Where a finished boot lands: the first gap with a screen, a home that
    /// only needs a restart on the screen that takes it, and a configured home
    /// straight on Ready.
    fn landing(&self) -> Stage {
        if let Some(stage) = self.gaps().into_iter().find_map(|gap| gap.stage()) {
            return stage;
        }
        if !self.gaps().is_empty() {
            return Stage::Ready;
        }
        if self.settings.state().restart.required {
            return Stage::Applying;
        }

        Stage::Ready
    }

    /// Save the four answers, restart where the daemon says one is owed, then
    /// run the finish gate on what it reports afterwards.
    pub fn start_applying(self: &Rc<Self>, save: bool) {
        self.refusal.replace(None);
        self.applying
            .replace(applying_steps(save, self.restart_required()));
        self.show(Stage::Applying);

        let model = Rc::clone(self);
        spawn(async move {
            model.apply(save).await;
        });
    }

    async fn apply(self: &Rc<Self>, save: bool) {
        if save {
            self.mark_apply(ApplyStep::Save, StepState::Working);
            let values = self.answers.borrow().values();
            if let Err(sentence) = self.settings.apply_values(PERSONALIZATION, values).await {
                self.mark_apply(ApplyStep::Save, StepState::Failed);
                self.refusal.replace(Some(sentence));
                self.block.replace(None);
                self.show(Stage::AboutYou);
                return;
            }
            self.mark_apply(ApplyStep::Save, StepState::Done);
        }

        if self.restart_required() {
            self.add_restart_step();
            self.mark_apply(ApplyStep::Restart, StepState::Working);
            if let Err(sentence) = self.settings.restart(false).await {
                self.mark_apply(ApplyStep::Restart, StepState::Failed);
                self.refusal.replace(Some(sentence));
                self.observers.notify();
                return;
            }
            self.mark_apply(ApplyStep::Restart, StepState::Done);
        }

        self.settings.refresh_setup().await;
        self.settings.refresh_overview().await;
        self.finish();
    }

    /// The Applying ladder's own retry, which re-runs what is left rather than
    /// re-writing what already landed.
    pub fn retry_applying(self: &Rc<Self>) {
        let save = self
            .applying
            .borrow()
            .iter()
            .any(|(step, state)| *step == ApplyStep::Save && *state != StepState::Done);

        self.start_applying(save);
    }

    // ---- What the daemon says -------------------------------------------

    /// The gating failures, in the daemon's own order.
    pub fn gaps(&self) -> Vec<Block> {
        self.failures(true)
    }

    /// The advisory failures, which never stop the assistant finishing.
    pub fn advisory(&self) -> Vec<SettingsPane> {
        let state = self.settings.state();
        let Some(setup) = state.setup.as_ref() else {
            return Vec::new();
        };

        setup
            .readiness
            .failures
            .iter()
            .filter(|failure| !failure.gating)
            .map(|failure| failure.pane)
            .collect()
    }

    fn failures(&self, gating: bool) -> Vec<Block> {
        let state = self.settings.state();
        let Some(setup) = state.setup.as_ref() else {
            return Vec::new();
        };

        setup
            .readiness
            .failures
            .iter()
            .filter(|failure| failure.gating == gating)
            .map(|failure| match failure.pane {
                SettingsPane::Providers => Block::Provider,
                SettingsPane::Personality => Block::Personalization,
                pane => Block::Elsewhere(pane),
            })
            .collect()
    }

    /// Which half of the finish gate is missing, or nothing once it holds.
    pub fn blocked(&self) -> Option<Block> {
        if self.settings.state().reachable != Some(true) {
            return Some(Block::DaemonNotLive);
        }

        if let Some(gap) = self.gaps().into_iter().next() {
            return Some(gap);
        }
        if self.restart_required() {
            return Some(Block::Restart);
        }

        None
    }

    /// Whether the daemon says a restart is owed.
    pub fn restart_required(&self) -> bool {
        self.settings.state().restart.required
    }

    /// Whether the daemon reports this home as ready.
    pub fn is_ready(&self) -> bool {
        let state = self.settings.state();
        state
            .setup
            .as_ref()
            .map(|setup| Readiness::parse(setup.readiness.status.as_deref()) == Readiness::Ready)
            .unwrap_or(false)
    }

    // ---- Bookkeeping ----------------------------------------------------

    fn show(&self, stage: Stage) {
        self.stage.set(stage);
        self.observers.notify();
    }

    fn mark(&self, step: Step, state: StepState) {
        {
            let mut steps = self.steps.borrow_mut();
            if let Some(row) = steps.iter_mut().find(|(row, _)| *row == step) {
                row.1 = state;
            }
        }
        self.observers.notify();
    }

    /// The restart row joins a ladder that was built without one, because a
    /// write is one of the things that makes a restart owed.
    ///
    /// Added rather than rebuilt: rebuilding forgets the step that has already
    /// finished, which leaves a row that never becomes a tick and a retry that
    /// writes again what already landed.
    fn add_restart_step(&self) {
        let mut applying = self.applying.borrow_mut();
        if !applying.iter().any(|(step, _)| *step == ApplyStep::Restart) {
            applying.push((ApplyStep::Restart, StepState::Waiting));
        }
    }

    fn mark_apply(&self, step: ApplyStep, state: StepState) {
        {
            let mut applying = self.applying.borrow_mut();
            if let Some(row) = applying.iter_mut().find(|(row, _)| *row == step) {
                row.1 = state;
            }
        }
        self.observers.notify();
    }

    /// The bar's leading control, per screen.
    fn leading(&self, stage: Stage) -> Option<Leading> {
        match stage {
            // There is nothing behind Connect your AI but a transaction that has
            // already run, so its way out is out of the assistant.
            Stage::ConnectAi => Some(Leading::Leave(Key::SetupConnectAiSkip)),
            Stage::Welcome | Stage::BootFailed => Some(Leading::Leave(Key::ActionBack)),
            Stage::AboutYou => Some(Leading::Back),
            Stage::Starting => Some(Leading::Cancel),
            Stage::Applying | Stage::Ready => None,
        }
    }

    /// The bar's one suggested action, per screen.
    fn primary(&self, stage: Stage) -> Option<Primary> {
        match stage {
            Stage::Welcome => Some(Primary::Begin),
            Stage::ConnectAi | Stage::AboutYou => Some(Primary::Continue),
            Stage::Ready => Some(Primary::Finish),
            Stage::BootFailed => None,
            Stage::Applying => self
                .applying
                .borrow()
                .iter()
                .any(|(_, state)| *state == StepState::Failed)
                .then_some(Primary::Retry),
            Stage::Starting => None,
        }
    }
}

/// The ladder, with nothing done yet.
fn fresh_steps() -> Vec<(Step, StepState)> {
    Step::ALL
        .iter()
        .map(|step| (*step, StepState::Waiting))
        .collect()
}

/// The Applying ladder: the steps this run actually takes, and no others.
///
/// A row shown for work nobody does is a promise the assistant cannot keep, and
/// it is a promise in both directions: a restart-only entry writes nothing, so
/// it draws no write row to leave waiting forever.
fn applying_steps(save: bool, restart: bool) -> Vec<(ApplyStep, StepState)> {
    let mut steps = Vec::with_capacity(2);
    if save {
        steps.push((ApplyStep::Save, StepState::Waiting));
    }
    if restart {
        steps.push((ApplyStep::Restart, StepState::Waiting));
    }
    steps
}

/// The first of two answers that says something.
fn first_of(daemon: String, platform: String) -> String {
    if daemon.is_empty() {
        platform
    } else {
        daemon
    }
}

/// This account's own name, as the platform reports it. Never an environment
/// variable: one can name a different account than the one this process runs
/// as.
fn account_name() -> String {
    let real = glib::real_name().to_string_lossy().to_string();
    if real.is_empty() || real == "Unknown" {
        return glib::user_name().to_string_lossy().to_string();
    }
    real
}

/// The zone this computer is set to.
fn local_timezone() -> String {
    glib::TimeZone::local().identifier().to_string()
}

/// The style a fresh home starts on: the middle option the daemon published,
/// where it published any.
fn default_style(rows: &[SettingsRow]) -> String {
    let options = rows
        .iter()
        .find(|row| row.key == STYLE_KEY)
        .map(|row| row.options.clone())
        .unwrap_or_default();

    options
        .get(options.len() / 2)
        .map(|option| option.value.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_dots_are_the_four_steps_and_the_ladders_inherit_theirs() {
        assert_eq!(Stage::Welcome.progress(), Some(0));
        assert_eq!(Stage::Starting.progress(), Some(0));
        assert_eq!(Stage::ConnectAi.progress(), Some(1));
        assert_eq!(Stage::AboutYou.progress(), Some(2));
        assert_eq!(Stage::Applying.progress(), Some(2));
        assert_eq!(Stage::Ready.progress(), Some(3));
        assert_eq!(Stage::BootFailed.progress(), None);
    }

    #[test]
    fn the_mechanical_screens_draw_no_dots_over_their_own_ladder() {
        assert!(Stage::Welcome.draws_progress());
        assert!(Stage::ConnectAi.draws_progress());
        assert!(Stage::AboutYou.draws_progress());
        assert!(Stage::Ready.draws_progress());
        assert!(!Stage::Starting.draws_progress());
        assert!(!Stage::Applying.draws_progress());
        assert!(!Stage::BootFailed.draws_progress());
    }

    #[test]
    fn escape_reaches_every_leading_control_but_the_one_that_stops_a_transaction() {
        assert!(Leading::Back.answers_escape());
        assert!(Leading::Leave(Key::ActionBack).answers_escape());
        assert!(!Leading::Cancel.answers_escape());
    }

    #[test]
    fn one_write_carries_all_four_answers() {
        let answers = Answers {
            name: "Ada".into(),
            timezone: "Europe/London".into(),
            style: "Answer in as few words as the question allows.".into(),
            assistant: "Fermix".into(),
        };

        let values = answers.values();
        assert_eq!(values.len(), 4);
        assert_eq!(
            values.get(NAME_KEY),
            Some(&SettingValue::Text("Ada".to_string()))
        );
        assert_eq!(
            values.get(ASSISTANT_KEY),
            Some(&SettingValue::Text("Fermix".to_string()))
        );
    }

    #[test]
    fn the_ladder_is_the_steps_this_run_takes_and_no_others() {
        assert_eq!(
            applying_steps(true, false),
            vec![(ApplyStep::Save, StepState::Waiting)]
        );
        assert_eq!(
            applying_steps(false, true),
            vec![(ApplyStep::Restart, StepState::Waiting)]
        );
        assert_eq!(applying_steps(true, true).len(), 2);
        assert_eq!(applying_steps(true, true)[1].0, ApplyStep::Restart);
        assert!(applying_steps(false, false).is_empty());
    }

    #[test]
    fn an_answer_the_daemon_already_holds_wins_over_this_computers_own() {
        assert_eq!(first_of("Ada".into(), "ada".into()), "Ada");
        assert_eq!(first_of(String::new(), "ada".into()), "ada");
    }
}
