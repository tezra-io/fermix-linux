//! What each action of the summary does (management PROTOCOL.md, *The Always On summary*), in
//! the order each call is made. An action runs as one blocking piece off the window's thread, as
//! every call does: `perform` makes its calls, follows a job it started to its end within the
//! job's budget, and answers what the pane shows next. Every write is followed by the pane's
//! read of the summary, which says where things stand; nothing here guesses it.

use super::{move_target, sentence};
use crate::home::{
    unconfirmed, ActionId, HomeStatus, ItemStatus, MoveFrom, MoveStarted, Passphrase, Readiness,
    ReadinessId, ReadinessItem, Role, SnapshotRun,
};
use crate::job::{outcome, Outcome as JobEnd};
use crate::management::{CallError, Management};
use crate::model::{JobList, JobStatus, JobView};
use crate::provision::{Options, Pick, PowerAction, ProvisionView};
use std::time::Duration;

/// A job is polled this far apart.
pub const JOB_POLL: Duration = Duration::from_secs(2);
/// The most polls one job is followed for: the provisioner's budget is fifteen minutes.
pub const JOB_POLLS: u64 = 450;
pub const JOB_OVERDUE: &str = "This took too long, so Fermix stopped waiting.";
const PROVISION_JOB: &str = "home_provision";

/// The daemon calls the actions make, and the wait between two polls of a job.
pub trait HomeDaemon {
    fn provision_options(&self) -> Result<Options, CallError>;
    fn provision_create(&self, pick: &Pick, replace: bool) -> Result<JobView, CallError>;
    fn provision_power(&self, action: PowerAction) -> Result<JobView, CallError>;
    fn provision_resources(&self) -> Result<ProvisionView, CallError>;
    fn provision_delete(&self) -> Result<JobView, CallError>;
    fn job_list(&self) -> Result<JobList, CallError>;
    fn job_cancel(&self, job_id: &str) -> Result<JobView, CallError>;
    fn job_get(&self, job_id: &str) -> Result<JobView, CallError>;
    fn home_move_readiness(&self, candidate_id: &str) -> Result<Readiness, CallError>;
    fn home_move_start(
        &self,
        candidate_id: &str,
        passphrase: &Passphrase,
        from: MoveFrom,
    ) -> Result<MoveStarted, CallError>;
    fn home_snapshots_run(&self) -> Result<SnapshotRun, CallError>;
    fn pause(&self, wait: Duration);
}

impl HomeDaemon for Management {
    fn provision_options(&self) -> Result<Options, CallError> {
        Management::provision_options(self)
    }

    fn provision_create(&self, pick: &Pick, replace: bool) -> Result<JobView, CallError> {
        Management::provision_create(self, pick, replace)
    }

    fn provision_power(&self, action: PowerAction) -> Result<JobView, CallError> {
        Management::provision_power(self, action)
    }

    fn provision_resources(&self) -> Result<ProvisionView, CallError> {
        Management::provision_resources(self)
    }

    fn provision_delete(&self) -> Result<JobView, CallError> {
        Management::provision_delete(self)
    }

    fn job_list(&self) -> Result<JobList, CallError> {
        Management::job_list(self)
    }

    fn job_cancel(&self, job_id: &str) -> Result<JobView, CallError> {
        Management::job_cancel(self, job_id)
    }

    fn job_get(&self, job_id: &str) -> Result<JobView, CallError> {
        Management::job_get(self, job_id)
    }

    fn home_move_readiness(&self, candidate_id: &str) -> Result<Readiness, CallError> {
        Management::home_move_readiness(self, candidate_id)
    }

    fn home_move_start(
        &self,
        candidate_id: &str,
        passphrase: &Passphrase,
        from: MoveFrom,
    ) -> Result<MoveStarted, CallError> {
        Management::home_move_start(self, candidate_id, passphrase, from)
    }

    fn home_snapshots_run(&self) -> Result<SnapshotRun, CallError> {
        Management::home_snapshots_run(self)
    }

    fn pause(&self, wait: Duration) {
        std::thread::sleep(wait);
    }
}

/// What one action needs from the pane: the summary it was pressed on, the options read for
/// Create server, Change and Replace, the owner's choice, the passphrase held for the move, and
/// the warnings the screen showed.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    pub action: ActionId,
    pub status: Option<HomeStatus>,
    pub options: Option<Options>,
    pub chosen: Option<Pick>,
    pub passphrase: Option<Passphrase>,
    pub shown: Vec<ReadinessId>,
}

/// What the pane shows after an action.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// The daemon's refusal or a job's failure, said under the action.
    pub refusal: Option<String>,
    /// A word that is no failure: a move's start whose answer was lost.
    pub notice: Option<String>,
    /// What Hetzner sells, where the action read it.
    pub options: Option<Result<Options, String>>,
    /// The review's warnings and blocks, where the move stopped at one the screen did not show.
    pub reviewed: Option<Vec<ReadinessItem>>,
    /// The move this app started, in the engine's sentence.
    pub move_started: Option<String>,
    /// Move now waits for the recovery passphrase, which none was held for.
    pub asks_passphrase: bool,
    /// The move took the passphrase, which leaves memory now.
    pub passphrase_spent: bool,
}

/// Runs one daemon action of the summary.
pub fn perform(daemon: &impl HomeDaemon, context: &Context) -> Outcome {
    let mut out = Outcome::default();
    let done = match context.action {
        ActionId::CreateServer => create_server(daemon, context, &mut out),
        ActionId::Cancel => cancel(daemon),
        ActionId::MoveNow | ActionId::TryAgain => move_now(daemon, context, &mut out),
        ActionId::CheckAgain => check_again(daemon, context),
        ActionId::TurnOn => power(daemon, PowerAction::On),
        ActionId::RestartServer => power(daemon, PowerAction::Reset),
        ActionId::StartAgain => start_again(daemon, context, &mut out),
        ActionId::CreateIn => create_in(daemon, context, &mut out),
        ActionId::DeleteServer => delete(daemon).map(|_| ()),
        ActionId::BackUpNow => daemon
            .home_snapshots_run()
            .map(|_| ())
            .map_err(|e| sentence(&e)),
        ActionId::ReplaceServer => replace(daemon, context),
        other => panic!("{other:?} does not run on the daemon"),
    };
    if let Err(refused) = done {
        out.refusal = Some(refused);
    }
    out
}

/// Creates the server: the owner's choice, else the recommendation.
fn create_server(daemon: &impl HomeDaemon, context: &Context, out: &mut Outcome) -> Step {
    let options = known_options(daemon, context, out)?;
    let pick = context
        .chosen
        .clone()
        .or_else(|| options.recommended.clone())
        .ok_or_else(|| options.sentence.clone())?;
    create(daemon, &pick, false)
}

/// Creates the size asked for where Hetzner has room next. A create Hetzner had no room for
/// left no server to name it, so it is the owner's choice, else the recommendation.
fn create_in(daemon: &impl HomeDaemon, context: &Context, out: &mut Outcome) -> Step {
    let Some(location) = summary_of(context).and_then(|s| s.detail.next_location) else {
        return Ok(());
    };
    let size = match &context.chosen {
        Some(chosen) => chosen.size.clone(),
        None => recommendation(daemon, context, out)?.size,
    };
    on_its_desktop(daemon, context, &Pick { size, location })
}

/// Deletes the server, then creates one of the same size in the same place, as the record
/// names them; a create that stopped before any server stood names none, and the owner's
/// choice, else the recommendation, is made again.
fn start_again(daemon: &impl HomeDaemon, context: &Context, out: &mut Outcome) -> Step {
    let view = daemon.provision_resources().map_err(|e| sentence(&e))?;
    let made = view.server.map(|s| Pick {
        size: s.size,
        location: s.location,
    });
    if !delete(daemon)? {
        return Ok(());
    }
    let again = match made.or_else(|| context.chosen.clone()) {
        Some(pick) => pick,
        None => recommendation(daemon, context, out)?,
    };
    on_its_desktop(daemon, context, &again)
}

/// The recommendation, or the sentence that says nothing is in stock.
fn recommendation(
    daemon: &impl HomeDaemon,
    context: &Context,
    out: &mut Outcome,
) -> Result<Pick, String> {
    let options = known_options(daemon, context, out)?;
    options.recommended.ok_or(options.sentence)
}

/// On the desktop whose home lives on the server, a server it creates is always the
/// replacement of that one.
fn on_its_desktop(daemon: &impl HomeDaemon, context: &Context, pick: &Pick) -> Step {
    let desktop = context
        .status
        .as_ref()
        .is_some_and(|s| s.role == Role::Device);
    create(daemon, pick, desktop)
}

/// Creates the server that replaces the one the home lives on, from Replace's choice; the move
/// to it is the same Move now.
fn replace(daemon: &impl HomeDaemon, context: &Context) -> Step {
    let pick = context
        .chosen
        .as_ref()
        .expect("Replace creates the size and place its sheet chose");
    create(daemon, pick, true)
}

/// Starts the create as a job the summary follows from here.
fn create(daemon: &impl HomeDaemon, pick: &Pick, replace: bool) -> Step {
    daemon
        .provision_create(pick, replace)
        .map(|_| ())
        .map_err(|e| sentence(&e))
}

/// What Hetzner sells: as the pane read it, else read now and kept.
fn known_options(
    daemon: &impl HomeDaemon,
    context: &Context,
    out: &mut Outcome,
) -> Result<Options, String> {
    if let Some(options) = &context.options {
        return Ok(options.clone());
    }
    let read = daemon.provision_options().map_err(|e| sentence(&e));
    out.options = Some(read.clone());
    read
}

/// Stops the `home_provision` job still running, wherever it was started, then deletes what it
/// made: delete is refused while a job runs.
fn cancel(daemon: &impl HomeDaemon) -> Step {
    let jobs = daemon.job_list().map_err(|e| sentence(&e))?.jobs;
    let running = jobs
        .into_iter()
        .find(|j| j.kind == PROVISION_JOB && j.status == JobStatus::Running);
    if let Some(running) = running {
        let cancelled = daemon
            .job_cancel(&running.job_id)
            .map_err(|e| sentence(&e))?;
        follow(daemon, cancelled)?;
    }
    delete(daemon).map(|_| ())
}

/// Deletes what Fermix created, as a job followed to its end. Answers whether it was deleted.
fn delete(daemon: &impl HomeDaemon) -> Result<bool, String> {
    let job = daemon.provision_delete().map_err(|e| sentence(&e))?;
    follow(daemon, job)
}

/// Turns the server on or resets it, as a job followed to its end, then reads Hetzner back so
/// the summary says what it found, whether the job ended well or not.
fn power(daemon: &impl HomeDaemon, action: PowerAction) -> Step {
    let powered = daemon
        .provision_power(action)
        .map_err(|e| sentence(&e))
        .and_then(|job| follow(daemon, job));
    let read = daemon.provision_resources().map_err(|e| sentence(&e));
    powered?;
    read.map(|_| ())
}

/// Follows a job until it ends, up to its budget: whether it completed, or why not.
fn follow(daemon: &impl HomeDaemon, started: JobView) -> Result<bool, String> {
    let budget_polls = started.budget_ms / 1_000 / JOB_POLL.as_secs() + 1;
    let mut job = started;
    for _ in 0..JOB_POLLS.min(budget_polls) {
        if job.status != JobStatus::Running {
            break;
        }
        daemon.pause(JOB_POLL);
        job = daemon.job_get(&job.job_id).map_err(|e| sentence(&e))?;
    }
    match outcome(&job) {
        JobEnd::Completed => Ok(true),
        JobEnd::Cancelled => Ok(false),
        JobEnd::Failed(why) => Err(why),
        JobEnd::Running(_) => Err(JOB_OVERDUE.to_owned()),
    }
}

/// The review, then the start. A review that blocks, or warns of something the screen did not
/// show, stops here and the screen lists it; the next Move now starts the move.
fn move_now(daemon: &impl HomeDaemon, context: &Context, out: &mut Outcome) -> Step {
    let Some(status) = &context.status else {
        return Ok(());
    };
    let Some(candidate) = move_target(status) else {
        return Ok(());
    };
    let Some(passphrase) = &context.passphrase else {
        out.asks_passphrase = true;
        return Ok(());
    };
    let review = daemon
        .home_move_readiness(&candidate)
        .map_err(|e| sentence(&e))?;
    let warnings: Vec<ReadinessItem> = review
        .items
        .into_iter()
        .filter(|i| i.status != ItemStatus::Ok)
        .collect();
    let unseen = warnings.iter().any(|i| !context.shown.contains(&i.id));
    if !review.ready || unseen {
        out.reviewed = Some(warnings);
        return Ok(());
    }
    start(daemon, &candidate, passphrase, from(status), out)
}

/// A server being replaced keeps nothing once the new one runs; every other move keeps this
/// computer as a desktop.
fn from(status: &HomeStatus) -> MoveFrom {
    match status.role {
        Role::Device => MoveFrom::TheServer,
        _ => MoveFrom::ThisComputer,
    }
}

fn start(
    daemon: &impl HomeDaemon,
    candidate: &str,
    passphrase: &Passphrase,
    from: MoveFrom,
    out: &mut Outcome,
) -> Step {
    let started = match daemon.home_move_start(candidate, passphrase, from) {
        Ok(started) => started.sentence,
        // No failure: the move may have started, and the summary says so once the server
        // answers again.
        Err(e) => {
            let lost = unconfirmed(&e).ok_or_else(|| sentence(&e))?.to_owned();
            out.notice = Some(lost.clone());
            lost
        }
    };
    out.move_started = Some(started);
    out.passphrase_spent = true;
    Ok(())
}

fn check_again(daemon: &impl HomeDaemon, context: &Context) -> Step {
    let Some(candidate) = context.status.as_ref().and_then(move_target) else {
        return Ok(());
    };
    daemon
        .home_move_readiness(&candidate)
        .map(|_| ())
        .map_err(|e| sentence(&e))
}

fn summary_of(context: &Context) -> Option<&crate::home::Summary> {
    context.status.as_ref()?.summary.as_ref()
}

/// One step of an action: done, or the sentence that says why not.
type Step = Result<(), String>;
