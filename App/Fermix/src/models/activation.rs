//! Bringing the daemon up, as one bounded transaction.
//!
//! Five steps behind the Starting checklist, inside one 90-second budget
//! (M38 section 5.5): refuse before anything is written, run the command line's
//! own install, point the client at the socket under the home it bound,
//! negotiate, prove the daemon's web door answers, and read what is already set
//! up.
//!
//! Every way this can end is one of the named states. Nothing here retries
//! through a second mechanism and nothing invents progress between
//! observations: a step is marked done only by evidence the command line or the
//! daemon published, and a step that cannot succeed reports which step it was
//! and what it saw.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4::gio;
use gtk4::gio::prelude::CancellableExt;
use gtk4::glib;

use crate::copy::{self, Key};
use crate::management::contract;
use crate::management::health::{self, Health, Origin};
use crate::management::ManagementError;
use crate::service::runner::ServiceError;
use crate::service::types::{Alignment, ServiceCode, ServiceStatus};

use super::api::READ_DEADLINE;
use super::settings_model::{Sentence, SettingsModel};

/// The whole transaction's budget. Every named failure is reached inside it.
pub const BUDGET: Duration = Duration::from_secs(90);
/// How often the web door is asked while it is not answering yet.
pub const WEB_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How long one look at the web door may take.
pub const WEB_PROBE_DEADLINE: Duration = Duration::from_secs(2);
/// The restarts that make a start a crash loop rather than a slow one.
pub const CRASH_LOOP_RESTARTS: u32 = 3;

/// One row of the Starting checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    BindHome,
    EnableLinger,
    StartService,
    VerifyDaemon,
    ReadSetupState,
}

impl Step {
    /// The rows, in the order the transaction takes them.
    pub const ALL: &'static [Step] = &[
        Step::BindHome,
        Step::EnableLinger,
        Step::StartService,
        Step::VerifyDaemon,
        Step::ReadSetupState,
    ];

    /// What the row says.
    pub fn key(self) -> Key {
        match self {
            Step::BindHome => Key::SetupStepBindHome,
            Step::EnableLinger => Key::SetupStepEnableLinger,
            Step::StartService => Key::SetupStepStartService,
            Step::VerifyDaemon => Key::SetupStepVerifyDaemon,
            Step::ReadSetupState => Key::SetupStepReadSetupState,
        }
    }

    /// The rows one `service install` answers for.
    ///
    /// The command line runs the whole transaction and answers once: it reports
    /// no per-step progress, so nothing here invents any. A run that returned a
    /// result took every step it names; a run that refused stopped on the row
    /// its own code names, which is `of_code` below.
    const fn installed_by_the_command_line() -> &'static [Step] {
        &[Step::BindHome, Step::EnableLinger, Step::StartService]
    }

    /// The row a refusal stopped on, read from the code the command line
    /// published rather than from where the ladder happened to be.
    fn of_code(code: ServiceCode) -> Step {
        match code {
            ServiceCode::LingerDenied | ServiceCode::LoginctlAbsent => Step::EnableLinger,
            ServiceCode::InvalidHome
            | ServiceCode::HomeChangeRefused
            | ServiceCode::BindingWriteFailed
            | ServiceCode::InvalidPort
            | ServiceCode::ConfigWriteFailed => Step::BindHome,
            ServiceCode::HealthUnavailable => Step::VerifyDaemon,
            _ => Step::StartService,
        }
    }
}

/// How far one row has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Waiting,
    Working,
    Done,
    Failed,
}

impl StepState {
    /// The word a screen reader hears beside the glyph.
    pub fn key(self) -> Key {
        match self {
            StepState::Waiting => Key::SetupStepWaiting,
            StepState::Working => Key::SetupStepWorking,
            StepState::Done => Key::SetupStepDone,
            StepState::Failed => Key::SetupStepFailed,
        }
    }
}

/// What one button on a refusal does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    TryAgain,
    Cancel,
    CopyCommand,
    CopyCommands,
    ShowMeHow,
    RunDoctor,
    ViewLog,
}

impl Action {
    /// The word on the button.
    pub fn key(self) -> Key {
        match self {
            Action::TryAgain => Key::ActionTryAgain,
            Action::Cancel => Key::ActionCancel,
            Action::CopyCommand => Key::ActionCopyCommand,
            Action::CopyCommands => Key::ActionCopyCommands,
            Action::ShowMeHow => Key::ActionShowMeHow,
            Action::RunDoctor => Key::MenuRunDoctor,
            Action::ViewLog => Key::PageLogs,
        }
    }
}

/// One named refusal, with everything the screen that renders it needs.
///
/// The title and the body are the catalogue's, because these are states this
/// application names; the sentence is always somebody else's words, and the
/// evidence is the offline log tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// Whether this was refused before anything was written.
    pub before_mutation: bool,
    pub title: Key,
    pub body: String,
    /// The one next action, as a sentence, where the design fixes one.
    pub next_action: Option<Key>,
    /// A command a person runs, with the row that labels it.
    pub command: Option<(Key, String)>,
    /// The command line's or the daemon's own words.
    pub sentence: Option<String>,
    /// The offline boot evidence, where the command line could collect it.
    pub evidence: Vec<String>,
    pub actions: Vec<Action>,
}

impl Refusal {
    /// One named state: a title from the catalogue and the buttons it offers.
    fn named(before_mutation: bool, title: Key, actions: Vec<Action>) -> Self {
        Self {
            before_mutation,
            title,
            body: String::new(),
            next_action: None,
            command: None,
            sentence: None,
            evidence: Vec::new(),
            actions,
        }
    }

    /// The paragraph under the title, where this state has one of its own. A
    /// state whose only explanation is somebody else's sentence carries none.
    fn body(mut self, body: Key) -> Self {
        self.body = copy::text(body);
        self
    }

    fn with_sentence(mut self, sentence: Option<String>) -> Self {
        self.sentence = sentence;
        self
    }
}

/// How the transaction ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The daemon is up, it negotiated, its web door answered, and what is
    /// already set up has been read.
    Activated,
    /// A named refusal. Nothing was written where `before_mutation` holds.
    Refused(Box<Refusal>),
}

/// One activation, as a value the assistant drives.
pub struct Activation {
    settings: Rc<SettingsModel>,
    budget: Duration,
    cancellable: RefCell<Option<gio::Cancellable>>,
}

impl Activation {
    /// An activation over the one settings model, inside the published budget.
    pub fn new(settings: Rc<SettingsModel>) -> Self {
        Self::with_budget(settings, BUDGET)
    }

    /// The same transaction inside a different budget.
    ///
    /// The product uses the published one; this exists so the wait for a web
    /// door that never answers can be proven in seconds rather than in ninety.
    /// It changes a value, never a code path.
    pub fn with_budget(settings: Rc<SettingsModel>, budget: Duration) -> Self {
        Self {
            settings,
            budget,
            cancellable: RefCell::new(None),
        }
    }

    /// Stop whatever is running. The command line's child is force-exited and
    /// reaped by the runner that owns it.
    pub fn cancel(&self) {
        if let Some(cancellable) = self.cancellable.borrow().as_ref() {
            cancellable.cancel();
        }
    }

    /// Whether a transaction is in flight.
    pub fn is_running(&self) -> bool {
        self.cancellable.borrow().is_some()
    }

    /// Run the whole transaction, reporting each row as it moves.
    pub async fn run(&self, home: Option<PathBuf>, progress: &dyn Fn(Step, StepState)) -> Outcome {
        let deadline = Instant::now() + self.budget;
        let cancellable = gio::Cancellable::new();
        self.cancellable.replace(Some(cancellable.clone()));

        let outcome = self
            .transaction(home, deadline, &cancellable, progress)
            .await;

        self.cancellable.replace(None);
        outcome
    }

    async fn transaction(
        &self,
        home: Option<PathBuf>,
        deadline: Instant,
        cancellable: &gio::Cancellable,
        progress: &dyn Fn(Step, StepState),
    ) -> Outcome {
        if let Some(refusal) = self.preflight().await {
            return self.refused(refusal, cancellable).await;
        }

        for step in [Step::BindHome, Step::EnableLinger, Step::StartService] {
            progress(step, StepState::Working);
        }
        if let Some(refusal) = self.install(home, deadline, cancellable, progress).await {
            return self.refused(refusal, cancellable).await;
        }

        progress(Step::VerifyDaemon, StepState::Working);
        if let Some(refusal) = self.verify(deadline, cancellable).await {
            progress(Step::VerifyDaemon, StepState::Failed);
            return self.refused(refusal, cancellable).await;
        }
        progress(Step::VerifyDaemon, StepState::Done);

        progress(Step::ReadSetupState, StepState::Working);
        if let Some(refusal) = self.read_what_is_set_up().await {
            progress(Step::ReadSetupState, StepState::Failed);
            return self.refused(refusal, cancellable).await;
        }
        progress(Step::ReadSetupState, StepState::Done);

        Outcome::Activated
    }

    // ---- The steps ------------------------------------------------------

    /// What the command line says about this account, before anything is
    /// written. Six named states and the command line's own refusal.
    async fn preflight(&self) -> Option<Refusal> {
        self.settings.refresh_service().await;

        let state = self.settings.state();
        if let Some(refusal) = state.service_refusal.as_ref() {
            return Some(preflight_of_refusal(refusal));
        }

        preflight_of_status(state.service.as_ref()?)
    }

    /// `fermix service install --json [--home PATH]`, inside what is left of the
    /// budget.
    async fn install(
        &self,
        home: Option<PathBuf>,
        deadline: Instant,
        cancellable: &gio::Cancellable,
        progress: &dyn Fn(Step, StepState),
    ) -> Option<Refusal> {
        let Some(remaining) = remaining(deadline) else {
            return Some(timed_out());
        };

        let budget = Budget::arm(cancellable, remaining);
        let outcome = self
            .settings
            .service()
            .install(home.as_deref(), None, cancellable)
            .await;
        drop(budget);

        match outcome {
            Ok(_installed) => {
                for step in Step::installed_by_the_command_line() {
                    progress(*step, StepState::Done);
                }
                None
            }
            Err(error) => {
                let refusal = activation_of_service_error(&error);
                if let ServiceError::Refused { code, .. } = &error {
                    progress(Step::of_code(ServiceCode::of(code)), StepState::Failed);
                } else {
                    progress(Step::StartService, StepState::Failed);
                }
                Some(refusal)
            }
        }
    }

    /// The daemon answers its socket, and its web door answers 200.
    async fn verify(&self, deadline: Instant, cancellable: &gio::Cancellable) -> Option<Refusal> {
        // The socket lives inside the home the command line just bound, so the
        // client is pointed at it before anything is asked of it.
        self.settings.refresh_service().await;

        let hello = match self.settings.api().hello(READ_DEADLINE).await.value {
            Ok(hello) => hello,
            Err(error) => return Some(self.negotiation_refusal(&error).await),
        };

        let Some(origin) = Origin::parse(&hello.setup.origin) else {
            return Some(web_unavailable().with_sentence(Some(hello.setup.origin.clone())));
        };

        self.wait_for_web(&origin, deadline, cancellable).await
    }

    /// Ask the web door until it answers or the budget runs out.
    ///
    /// Bounded by the budget: every turn either ends the wait or waits one
    /// interval, and the interval is a fraction of what is left.
    async fn wait_for_web(
        &self,
        origin: &Origin,
        deadline: Instant,
        cancellable: &gio::Cancellable,
    ) -> Option<Refusal> {
        let mut last = Health::Unreachable;

        while let Some(remaining) = remaining(deadline) {
            if cancellable.is_cancelled() {
                return Some(cancelled());
            }

            last = health::probe(origin, WEB_PROBE_DEADLINE.min(remaining)).await;
            if last == Health::Live {
                return None;
            }

            glib::timeout_future(WEB_POLL_INTERVAL).await;
        }

        // Something is accepting on the origin the daemon needs and it is not
        // answering the daemon's own health door, which is the port being held
        // by something else. Nothing accepting at all is the daemon's web door
        // simply not being up.
        match last {
            Health::Occupied(status_line) => Some(
                Refusal::named(
                    false,
                    Key::ActivationBindFailureTitle,
                    vec![Action::TryAgain, Action::RunDoctor, Action::ViewLog],
                )
                .body(Key::ActivationBindFailureBody)
                .with_sentence(Some(status_line).filter(|line| !line.is_empty())),
            ),
            _ => Some(web_unavailable()),
        }
    }

    /// What this home already has, which is what decides where the assistant
    /// opens rather than re-asking a home that is already configured.
    async fn read_what_is_set_up(&self) -> Option<Refusal> {
        if let Err(error) = self.settings.read_setup().await {
            return Some(read_refusal(&error));
        }

        self.settings.refresh_overview().await;
        self.settings.refresh_sections().await;
        None
    }

    /// Why the negotiation failed: an empty version intersection is the one
    /// that names both ranges, and everything else is asked of the unit.
    async fn negotiation_refusal(&self, error: &ManagementError) -> Refusal {
        if let Some(refusal) = protocol_refusal(error) {
            return refusal;
        }

        self.settings.refresh_service().await;
        let state = self.settings.state();
        let crash_loop = state.service.as_ref().is_some_and(is_crash_loop);
        drop(state);

        let sentence = Some(Sentence::of(error).text);
        if crash_loop {
            return Refusal::named(
                false,
                Key::ActivationCrashLoopTitle,
                vec![Action::TryAgain, Action::ViewLog],
            )
            .body(Key::ActivationCrashLoopBody)
            .with_sentence(sentence);
        }

        unit_inactive().with_sentence(sentence)
    }

    /// A refusal, with the offline boot evidence attached where the command
    /// line could collect it. A refusal before any mutation has no boot to
    /// explain, so none is collected for one.
    async fn refused(&self, mut refusal: Refusal, cancellable: &gio::Cancellable) -> Outcome {
        if !refusal.before_mutation && !cancellable.is_cancelled() {
            let cancellable = gio::Cancellable::new();
            if let Ok(export) = self
                .settings
                .service()
                .export_diagnostics(&cancellable)
                .await
            {
                refusal.evidence = super::recovery::log_lines(&export);
            }
        }

        Outcome::Refused(Box::new(refusal))
    }
}

// ---------------------------------------------------------------------------
// The named states
// ---------------------------------------------------------------------------

/// The preflight state one command line refusal names.
fn preflight_of_refusal(refusal: &Sentence) -> Refusal {
    let code = refusal
        .code
        .as_deref()
        .map(ServiceCode::of)
        .unwrap_or(ServiceCode::Unrecognized);

    let named = match code {
        ServiceCode::UserManagerUnreachable => Refusal::named(
            true,
            Key::PreflightNoUserManagerTitle,
            vec![Action::TryAgain, Action::Cancel],
        )
        .body(Key::PreflightNoUserManagerBody),
        ServiceCode::ForeignDistribution => Refusal::named(
            true,
            Key::PreflightForeignDistributionTitle,
            vec![Action::Cancel],
        )
        .body(Key::PreflightForeignDistributionBody),
        ServiceCode::LoginctlAbsent => login_manager_absent(true),
        // A refusal this build has no state for is still a refusal before
        // anything was written. Its own sentence is the whole explanation.
        _ => Refusal::named(
            true,
            Key::SetupPreflightRefusedTitle,
            vec![Action::TryAgain, Action::Cancel],
        ),
    };

    named.with_sentence(Some(refusal.text.clone()))
}

/// The preflight state a typed status names, in the order the six are asked.
fn preflight_of_status(status: &ServiceStatus) -> Option<Refusal> {
    if let Some(distribution) = status.foreign_distribution() {
        return Some(
            Refusal::named(
                true,
                Key::PreflightForeignDistributionTitle,
                vec![Action::Cancel],
            )
            .body(Key::PreflightForeignDistributionBody)
            .with_sentence(Some(distribution.to_string())),
        );
    }

    // A unit Fermix did not write is a published fact rather than a refused
    // read: the status names it and the verbs that would rewrite it refuse. The
    // path is the whole of what a person can act on, so it travels with the
    // state.
    if status.unit.foreign {
        return Some(
            Refusal::named(
                true,
                Key::PreflightForeignDaemonTitle,
                vec![Action::TryAgain, Action::Cancel],
            )
            .body(Key::PreflightForeignDaemonBody)
            .with_sentence(status.unit.effective_path.clone()),
        );
    }

    if status.answers_nothing() {
        return Some(
            Refusal::named(
                true,
                Key::PreflightPreManagementDaemonTitle,
                vec![Action::TryAgain, Action::Cancel],
            )
            .body(Key::PreflightPreManagementDaemonBody),
        );
    }

    if status.alignment == Alignment::PendingRestart {
        return Some(
            Refusal::named(
                true,
                Key::PreflightEngineSkewTitle,
                vec![Action::TryAgain, Action::Cancel],
            )
            .body(Key::PreflightEngineSkewBody),
        );
    }

    None
}

/// The activation state one command line refusal names.
fn activation_of_service_error(error: &ServiceError) -> Refusal {
    let ServiceError::Refused { code, sentence } = error else {
        return unit_inactive().with_sentence(Some(error.to_string()));
    };

    let sentence = Some(sentence.clone());
    match ServiceCode::of(code) {
        ServiceCode::LingerDenied => linger_denied().with_sentence(sentence),
        ServiceCode::LoginctlAbsent => login_manager_absent(false).with_sentence(sentence),
        ServiceCode::ActivationTimeout => unit_inactive().with_sentence(sentence),
        ServiceCode::HealthUnavailable => web_unavailable().with_sentence(sentence),
        // A code this build has no state for still stopped the boot, and its
        // own sentence says what happened.
        _ => Refusal::named(
            false,
            Key::SetupBootFailedTitle,
            vec![Action::TryAgain, Action::ViewLog],
        )
        .with_sentence(sentence),
    }
}

/// Linger refused, which is the Linux moment M38 section 6.5 fixes: what
/// happened, what is untouched, and the one command that grants it.
fn linger_denied() -> Refusal {
    let mut refusal = Refusal::named(
        false,
        Key::LingerDeniedTitle,
        vec![Action::CopyCommand, Action::TryAgain],
    )
    .body(Key::LingerDeniedBody);

    refusal.command = Some((Key::LingerDeniedCommandRow, linger_command()));
    refusal
}

/// The other linger moment: a host with no login manager cannot be helped by
/// any command, so none is offered.
fn login_manager_absent(before_mutation: bool) -> Refusal {
    let mut refusal = Refusal::named(
        before_mutation,
        Key::LoginManagerAbsentTitle,
        vec![Action::TryAgain, Action::Cancel],
    )
    .body(Key::LoginManagerAbsentBody);

    refusal.next_action = Some(Key::LoginManagerAbsentNextAction);
    refusal
}

/// The service was switched on and is not running.
fn unit_inactive() -> Refusal {
    Refusal::named(
        false,
        Key::ActivationUnitInactiveTitle,
        vec![Action::TryAgain, Action::ViewLog],
    )
    .body(Key::ActivationUnitInactiveBody)
}

/// The daemon answers its socket and its web door does not.
fn web_unavailable() -> Refusal {
    Refusal::named(
        false,
        Key::ActivationWebUnavailableTitle,
        vec![Action::TryAgain, Action::RunDoctor, Action::ViewLog],
    )
    .body(Key::ActivationWebUnavailableBody)
}

/// The command the design prints, with this account's own name in it.
///
/// The name comes from the platform's own answer rather than from an
/// environment variable, which can name a different account than the one this
/// process runs as.
pub fn linger_command() -> String {
    copy::fill(
        Key::LingerDeniedCommandValue,
        &[("<user>", &glib::user_name().to_string_lossy())],
    )
}

/// The row that says what the linger command is, with the same account in it.
pub fn linger_command_row() -> String {
    copy::fill(
        Key::LingerDeniedCommandRow,
        &[("<user>", &glib::user_name().to_string_lossy())],
    )
}

/// The two halves share no version, or a read this assistant makes needs one
/// the daemon does not speak. Both name both ranges and neither retries on an
/// older wire.
fn protocol_refusal(error: &ManagementError) -> Option<Refusal> {
    let window = contract::supported_range();
    let daemon = match error {
        ManagementError::VersionsDoNotOverlap { daemon, .. } => *daemon,
        ManagementError::MethodNeedsNewerDaemon { negotiated, .. } => (window.min, *negotiated),
        ManagementError::Wire(wire)
            if matches!(wire.code.as_str(), "client_too_old" | "daemon_too_old") =>
        {
            (
                wire.detail_u32("minimum_version").unwrap_or(window.min),
                wire.detail_u32("maximum_version").unwrap_or(window.max),
            )
        }
        _ => return None,
    };

    let mut refusal = Refusal::named(
        false,
        Key::ActivationProtocolMismatchTitle,
        vec![Action::TryAgain, Action::ViewLog],
    );
    refusal.body = copy::fill(
        Key::ActivationProtocolMismatchBody,
        &[
            ("{app_range}", &range(window.min, window.max)),
            ("{daemon_range}", &range(daemon.0, daemon.1)),
        ],
    );
    refusal.sentence = Some(Sentence::of(error).text);
    Some(refusal)
}

/// The read that says what is already set up was refused, in the daemon's own
/// words.
fn read_refusal(error: &ManagementError) -> Refusal {
    if let Some(refusal) = protocol_refusal(error) {
        return refusal;
    }

    Refusal::named(
        false,
        Key::SetupBootFailedTitle,
        vec![Action::TryAgain, Action::RunDoctor, Action::ViewLog],
    )
    .with_sentence(Some(Sentence::of(error).text))
}

fn timed_out() -> Refusal {
    unit_inactive()
}

fn cancelled() -> Refusal {
    Refusal::named(
        false,
        Key::SetupBootFailedTitle,
        vec![Action::TryAgain, Action::ViewLog],
    )
}

/// Whether the service manager is restarting a unit that keeps dying.
fn is_crash_loop(status: &ServiceStatus) -> bool {
    status.is_restarting() || status.restart_count >= CRASH_LOOP_RESTARTS
}

/// One version window, as a person reads it.
fn range(minimum: u32, maximum: u32) -> String {
    if minimum == maximum {
        return minimum.to_string();
    }
    format!("{minimum}\u{2013}{maximum}")
}

/// What is left of the budget, or nothing when it is spent.
fn remaining(deadline: Instant) -> Option<Duration> {
    deadline.checked_duration_since(Instant::now())
}

/// A one-shot timer that ends an operation at the budget.
///
/// It is a future on this thread's own main context rather than a source on the
/// process's default one: a timer added to the default context takes ownership
/// of it for the length of the call, and two threads doing that at once is a
/// panic rather than a wait. Everything else here already runs on the context
/// the caller is on, and this is the one thing that did not.
///
/// It is ended on every path out, including the one where it fired: aborting a
/// task that has already run is not an error.
struct Budget {
    task: glib::JoinHandle<()>,
}

impl Budget {
    fn arm(cancellable: &gio::Cancellable, remaining: Duration) -> Self {
        let cancellable = cancellable.clone();

        Self {
            task: glib::spawn_future_local(async move {
                glib::timeout_future(remaining).await;
                cancellable.cancel();
            }),
        }
    }
}

impl Drop for Budget {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_of_the_ladder_says_something() {
        for step in Step::ALL {
            assert!(!copy::text(step.key()).is_empty());
        }
        for state in [
            StepState::Waiting,
            StepState::Working,
            StepState::Done,
            StepState::Failed,
        ] {
            assert!(!copy::text(state.key()).is_empty());
        }
    }

    #[test]
    fn a_refusal_stops_on_the_row_its_own_code_names() {
        assert_eq!(Step::of_code(ServiceCode::LingerDenied), Step::EnableLinger);
        assert_eq!(Step::of_code(ServiceCode::InvalidHome), Step::BindHome);
        assert_eq!(
            Step::of_code(ServiceCode::HealthUnavailable),
            Step::VerifyDaemon
        );
        assert_eq!(
            Step::of_code(ServiceCode::SystemctlFailed),
            Step::StartService
        );
    }

    #[test]
    fn a_version_window_reads_as_one_number_when_both_ends_agree() {
        assert_eq!(range(2, 2), "2");
        assert_eq!(range(1, 2), "1\u{2013}2");
    }

    /// One published status, with the two fields this reads set to the values
    /// the case is about.
    ///
    /// The state itself has no golden: the engine publishes `sub_state`
    /// verbatim from the service manager and `restart_count` as a number, so a
    /// unit that keeps dying is a pair of values rather than a shape anybody
    /// wrote down. The shape is the golden's; only those two values move.
    fn status_with(sub_state: Option<&str>, restart_count: u32) -> ServiceStatus {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/cli/fixtures/service_status/bound_disabled.json");
        let bytes = std::fs::read(&path).expect("the golden is readable");
        let envelope: serde_json::Value = serde_json::from_slice(&bytes).expect("parses");

        let mut status: ServiceStatus =
            serde_json::from_value(envelope["result"].clone()).expect("decodes");
        status.sub_state = sub_state.map(str::to_string);
        status.restart_count = restart_count;
        status
    }

    #[test]
    fn a_crash_loop_is_a_restarting_unit_or_a_spent_start_budget() {
        assert!(is_crash_loop(&status_with(Some("auto-restart"), 0)));
        assert!(is_crash_loop(&status_with(
            Some("dead"),
            CRASH_LOOP_RESTARTS
        )));
        assert!(!is_crash_loop(&status_with(Some("running"), 0)));
        assert!(!is_crash_loop(&status_with(None, 0)));
    }

    #[test]
    fn an_install_that_answered_marks_every_row_it_answers_for() {
        // The command line reports no per-step progress, so the rows a returned
        // install marks are exactly the ones it ran, named here rather than
        // inferred from a phase list the engine does not publish.
        assert_eq!(
            Step::installed_by_the_command_line(),
            &[Step::BindHome, Step::EnableLinger, Step::StartService]
        );
        for step in Step::installed_by_the_command_line() {
            assert!(Step::ALL.contains(step), "{step:?} is a row of the ladder");
        }
    }

    #[test]
    fn the_linger_command_names_this_account_rather_than_a_placeholder() {
        let command = linger_command();
        assert!(!command.contains("<user>"), "{command}");
        assert!(command.starts_with("sudo loginctl enable-linger "));
    }
}
