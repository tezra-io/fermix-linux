//! The desktop notifications of Always On (M75 lean v1 §7, L18): the server is ready, the move
//! is done, the server has not answered for 10 minutes, a waiting server must be set up again,
//! and the last backup is 7 days old. Each is raised from a change the app saw between two reads
//! of `home.status`, never from a state it read once, so each is raised at most once per
//! occurrence: a launch reads its first summary and raises nothing for it. The body is the
//! engine's own sentence for the state it changed to.
//!
//! And how long the app waits before it reads `home.status` again: briskly while something moves
//! by itself and the pane shows it, slowly while all is set and nothing can change but the
//! server's health, and rarely where Always On is not set up.

use crate::home::{HomeStatus, ProblemKind, SummaryState};
use crate::phone::unix_seconds;
use std::time::Duration;

/// How old the newest backup is before the computer says so, as the server tells the phones.
pub const BACKUP_AGE: i64 = 7 * 86_400;

/// While the pane shows a server being created or a move, and while the engine restarts for
/// this app's own move.
pub const WATCHING: Duration = Duration::from_secs(3);
/// While the pane is open on anything else.
pub const OPEN: Duration = Duration::from_secs(10);
/// While a server is created or a move runs with the pane closed.
pub const MOVING: Duration = Duration::from_secs(15);
/// While Always On is set up: a server that stops answering, one that waits too long, an old
/// backup.
pub const SETTLED: Duration = Duration::from_secs(60);
/// After a read that failed, or while Always On is not set up.
pub const IDLE: Duration = Duration::from_secs(300);

/// Which notification a change raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    ServerReady,
    MoveDone,
    CannotReach,
    SetUpAgain,
    BackupOld,
}

impl NoticeKind {
    /// The notification's id: a newer one of a kind replaces the one before it, and only that.
    pub fn id(self) -> &'static str {
        match self {
            NoticeKind::ServerReady => "always-on-server-ready",
            NoticeKind::MoveDone => "always-on-move-done",
            NoticeKind::CannotReach => "always-on-cannot-reach",
            NoticeKind::SetUpAgain => "always-on-set-up-again",
            NoticeKind::BackupOld => "always-on-backup-old",
        }
    }
}

/// One notification to raise: its kind and the engine's sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub body: String,
}

/// One read of `home.status`, and when it answered, in Unix seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub status: HomeStatus,
    pub at: i64,
}

/// The notification the change from `last` to `current` raises, if any. `own_move` says this
/// app started the move `current` may finish, across the engine's restart, where no read may
/// have seen `moving`.
pub fn notice(last: Option<&Reading>, current: &Reading, own_move: bool) -> Option<Notice> {
    let now = current.status.summary.as_ref()?;
    let before = last?.status.summary.as_ref()?;
    let kind = match now.state {
        SummaryState::Ready => {
            (before.state != SummaryState::Ready).then_some(NoticeKind::ServerReady)
        }
        SummaryState::On => {
            (before.state == SummaryState::Moving || own_move).then_some(NoticeKind::MoveDone)
        }
        SummaryState::Problem => problem_notice(last?, current),
        _ => None,
    }?;
    Some(Notice {
        kind,
        body: now.sentence.clone(),
    })
}

/// A problem raises a notification as it begins, and an old backup once its age passes a week,
/// once for that backup.
fn problem_notice(last: &Reading, current: &Reading) -> Option<NoticeKind> {
    let was = problem_of(&last.status);
    match problem_of(&current.status)? {
        ProblemKind::CannotReach => {
            (was != Some(ProblemKind::CannotReach)).then_some(NoticeKind::CannotReach)
        }
        ProblemKind::WaitedTooLong => {
            (was != Some(ProblemKind::WaitedTooLong)).then_some(NoticeKind::SetUpAgain)
        }
        ProblemKind::BackupOld => passed_a_week(last, current).then_some(NoticeKind::BackupOld),
        _ => None,
    }
}

fn problem_of(status: &HomeStatus) -> Option<ProblemKind> {
    status.summary.as_ref()?.problem()
}

/// Whether the newest backup is a week old now and was not yet at the last read: the same
/// backup, counted from when it was taken, or from the move where none has finished since.
fn passed_a_week(last: &Reading, current: &Reading) -> bool {
    let Some(since) = backup_since(&current.status) else {
        return false;
    };
    if current.at - since < BACKUP_AGE {
        return false;
    }
    let same_backup = problem_of(&last.status) == Some(ProblemKind::BackupOld)
        && backup_since(&last.status) == Some(since);
    !same_backup || last.at - since < BACKUP_AGE
}

/// When the backup an old-backup problem counts from was taken: the problem's own time, or the
/// move's where no backup finished since.
fn backup_since(status: &HomeStatus) -> Option<i64> {
    let seen = status.summary.as_ref()?.detail.seen_at.as_deref();
    let moved = status.moved.as_ref().map(|m| m.moved_at.as_str());
    unix_seconds(seen.or(moved)?)
}

/// The wait before the next read, as the last read leaves it.
pub fn wait(
    status: Option<&HomeStatus>,
    failed: bool,
    pane_visible: bool,
    own_move: bool,
) -> Duration {
    if own_move {
        return WATCHING;
    }
    let state = status.and_then(|s| s.summary.as_ref()).map(|s| s.state);
    let moves = matches!(state, Some(SummaryState::Creating | SummaryState::Moving));
    match (moves, pane_visible, failed) {
        (true, true, _) => WATCHING,
        (true, false, _) => MOVING,
        (false, true, _) => OPEN,
        (false, false, true) => IDLE,
        (false, false, false) => settled_or_idle(state),
    }
}

fn settled_or_idle(state: Option<SummaryState>) -> Duration {
    match state {
        Some(
            SummaryState::Ready | SummaryState::On | SummaryState::Problem | SummaryState::Undone,
        ) => SETTLED,
        _ => IDLE,
    }
}
