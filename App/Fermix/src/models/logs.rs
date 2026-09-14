//! Logs, as data.
//!
//! A bounded tail of the file the daemon writes, re-read every two seconds
//! while the surface is in front of someone and not paused, plus the older
//! pages behind it. Nothing here reads the file: `logs.query` is the only door,
//! and the cursor, the filters and the bounds are all the protocol's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use crate::copy::Key;
use crate::management::types::{
    LogDirection, LogEntry, LogLevel, LogsQueryParams, LogsQueryResult,
};
use crate::management::vocabulary::Refusal;
use crate::management::{ManagementError, TransportError};

use super::api::{accept, ask, READ_DEADLINE};
use super::settings_model::SettingsModel;
use super::{spawn, Observers, Poller};

/// How often the tail is re-read while the surface is visible and not paused.
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// The most polls one visible stretch performs: six hours at the interval
/// above. Reaching it stops the poll; showing the surface again starts it.
pub const POLL_CAP: u32 = 10_800;
/// The tail this surface opens on, which is the protocol's own initial tail.
pub const TAIL_LIMIT: u32 = 200;
/// The most entries held at once. Older ones are dropped as newer arrive, so a
/// window left open for a day holds a page, not a file.
pub const HELD_ENTRIES: usize = 2_000;

/// The eight levels, in the order the filter lists them.
pub const LEVELS: &[LogLevel] = &[
    LogLevel::Emergency,
    LogLevel::Alert,
    LogLevel::Critical,
    LogLevel::Error,
    LogLevel::Warning,
    LogLevel::Notice,
    LogLevel::Info,
    LogLevel::Debug,
];

/// The word for one level. The wire publishes the atom; the catalogue owns how
/// it reads.
pub fn level_word(level: LogLevel) -> Key {
    match level {
        LogLevel::Emergency => Key::LogLevelEmergency,
        LogLevel::Alert => Key::LogLevelAlert,
        LogLevel::Critical => Key::LogLevelCritical,
        LogLevel::Error => Key::LogLevelError,
        LogLevel::Warning => Key::LogLevelWarning,
        LogLevel::Notice => Key::LogLevelNotice,
        LogLevel::Info => Key::LogLevelInfo,
        LogLevel::Debug => Key::LogLevelDebug,
    }
}

/// Logs' own model.
pub struct LogsModel {
    settings: Rc<SettingsModel>,
    entries: RefCell<Vec<LogEntry>>,
    /// The cursor for the page *older* than what is held, where there is one.
    older: RefCell<Option<String>>,
    level: Cell<Option<LogLevel>>,
    search: RefCell<Option<String>>,
    paused: Cell<bool>,
    /// The daemon's own sentence about why this view started again.
    notice: RefCell<Option<String>>,
    unavailable: Cell<bool>,
    poller: Poller,
    observers: Observers,
}

/// What one read did to the held entries, so a view knows whether its scroll
/// position still means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landed {
    /// Nothing changed.
    Unchanged,
    /// Entries were added at the end.
    Appended,
    /// Entries were added at the start.
    Prepended,
    /// The whole window moved, so the list was replaced.
    Replaced,
}

impl LogsModel {
    /// A Logs model over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        Rc::new(Self {
            settings,
            entries: RefCell::new(Vec::new()),
            older: RefCell::new(None),
            level: Cell::new(None),
            search: RefCell::new(None),
            paused: Cell::new(false),
            notice: RefCell::new(None),
            unavailable: Cell::new(false),
            poller: Poller::new(),
            observers: Observers::default(),
        })
    }

    /// Tell me when the entries move.
    pub fn observe(&self, observer: impl Fn() + 'static) {
        self.observers.add(observer);
    }

    /// The entries held, oldest first.
    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.borrow().clone()
    }

    /// How many are held.
    pub fn count(&self) -> usize {
        self.entries.borrow().len()
    }

    /// The daemon's own sentence about the last thing that happened to this
    /// view, where there is one to show.
    pub fn notice(&self) -> Option<String> {
        self.notice.borrow().clone()
    }

    /// Forget the notice, once it has been shown.
    pub fn clear_notice(&self) {
        self.notice.replace(None);
    }

    /// Whether the daemon is answering.
    pub fn is_unavailable(&self) -> bool {
        self.unavailable.get()
    }

    /// Whether there is an older page to ask for.
    pub fn has_older(&self) -> bool {
        self.older.borrow().is_some()
    }

    /// Whether the tail is paused.
    pub fn is_paused(&self) -> bool {
        self.paused.get()
    }

    /// Whether the tail is being polled.
    pub fn is_polling(&self) -> bool {
        self.poller.is_running()
    }

    /// The level filter, where one is set.
    pub fn level(&self) -> Option<LogLevel> {
        self.level.get()
    }

    /// The search, where one is set.
    pub fn search(&self) -> Option<String> {
        self.search.borrow().clone()
    }

    /// Pause or resume the tail. A paused view stays exactly where it is.
    pub fn set_paused(self: &Rc<Self>, paused: bool) {
        self.paused.set(paused);
        if paused {
            self.poller.stop();
        } else {
            self.start_polling();
        }
        self.observers.notify();
    }

    /// Filter by level. Changing a filter starts the window again, because the
    /// entries behind it are a different set.
    pub fn set_level(self: &Rc<Self>, level: Option<LogLevel>) {
        if self.level.get() == level {
            return;
        }
        self.level.set(level);
        self.restart();
    }

    /// Filter by text. The protocol caps the query, and so does this.
    pub fn set_search(self: &Rc<Self>, search: Option<String>) {
        let search = search
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .map(|text| text.chars().take(MAX_SEARCH).collect::<String>());

        if *self.search.borrow() == search {
            return;
        }
        self.search.replace(search);
        self.restart();
    }

    fn restart(self: &Rc<Self>) {
        self.entries.borrow_mut().clear();
        self.older.replace(None);
        self.observers.notify();

        let model = Rc::clone(self);
        spawn(async move {
            model.refresh().await;
        });
    }

    /// Start the tail poll.
    pub fn start_polling(self: &Rc<Self>) {
        if self.paused.get() {
            return;
        }

        let model = Rc::clone(self);
        self.poller.start(POLL_INTERVAL, POLL_CAP, move || {
            let model = Rc::clone(&model);
            spawn(async move {
                model.refresh().await;
            });
            true
        });
    }

    /// Stop it.
    pub fn stop_polling(&self) {
        self.poller.stop();
    }

    /// Read the newest page and merge it with what is held.
    pub async fn refresh(&self) -> Landed {
        let params = LogsQueryParams {
            limit: Some(TAIL_LIMIT),
            level: self.level.get(),
            subsystem: None,
            search: self.search.borrow().clone(),
            direction: Some(LogDirection::Backward),
            cursor: None,
        };

        let Some(result) = self.query(params).await else {
            return Landed::Unchanged;
        };

        // The newest page carries the cursor for the page behind it. Held only
        // while nothing older has been asked for yet, so paging back does not
        // lose its place on the next tick.
        if self.older.borrow().is_none() {
            self.older.replace(result.cursor.clone());
        }

        let landed = self.merge_newer(result.entries);
        self.observers.notify();
        landed
    }

    /// Read the page older than what is held.
    pub async fn load_older(&self) -> Landed {
        let Some(cursor) = self.older.borrow().clone() else {
            return Landed::Unchanged;
        };

        let params = LogsQueryParams {
            limit: Some(TAIL_LIMIT),
            level: self.level.get(),
            subsystem: None,
            search: self.search.borrow().clone(),
            direction: Some(LogDirection::Backward),
            cursor: Some(cursor),
        };

        let Some(result) = self.query(params).await else {
            return Landed::Unchanged;
        };

        self.older.replace(result.cursor.clone());
        let landed = self.merge_older(result.entries);
        self.observers.notify();
        landed
    }

    async fn query(&self, params: LogsQueryParams) -> Option<LogsQueryResult> {
        let api = self.settings.api();
        let issued =
            ask::<_, LogsQueryResult>(api.as_ref(), "logs.query", &params, READ_DEADLINE).await;

        match accept(api.as_ref(), issued) {
            None => None,
            Some(Ok(result)) => {
                self.unavailable.set(false);
                Some(result)
            }
            Some(Err(error)) => {
                self.record(&error);
                None
            }
        }
    }

    /// A refusal, read.
    ///
    /// A cursor that predates a rotation is the one refusal this surface
    /// recovers from on its own: it drops what it held, says why in the
    /// daemon's own words, and starts again at the newest entries.
    fn record(&self, error: &ManagementError) {
        if let ManagementError::Wire(wire) = error {
            if Refusal::of(wire) == Refusal::CursorExpired {
                self.entries.borrow_mut().clear();
                self.older.replace(None);
                self.notice.replace(Some(wire.rendered().to_string()));
                self.observers.notify();
                return;
            }
        }

        if matches!(
            error,
            ManagementError::Transport(TransportError::NotRunning)
        ) {
            self.unavailable.set(true);
            self.observers.notify();
        }
    }

    /// Lay a newer page over what is held: the entries after the last one held
    /// are appended, and a window that has moved past everything held replaces
    /// it.
    fn merge_newer(&self, page: Vec<LogEntry>) -> Landed {
        let mut entries = self.entries.borrow_mut();

        if entries.is_empty() {
            if page.is_empty() {
                return Landed::Unchanged;
            }
            *entries = page;
            return Landed::Replaced;
        }

        let last = identity(entries.last().expect("the list is not empty"));
        match page.iter().position(|entry| identity(entry) == last) {
            Some(at) if at + 1 >= page.len() => Landed::Unchanged,
            Some(at) => {
                entries.extend_from_slice(&page[at + 1..]);
                let over = entries.len().saturating_sub(HELD_ENTRIES);
                if over > 0 {
                    entries.drain(..over);
                }
                Landed::Appended
            }
            None if page.is_empty() => Landed::Unchanged,
            None => {
                *entries = page;
                Landed::Replaced
            }
        }
    }

    /// Put an older page in front of what is held.
    fn merge_older(&self, page: Vec<LogEntry>) -> Landed {
        if page.is_empty() {
            return Landed::Unchanged;
        }

        let mut entries = self.entries.borrow_mut();
        let first = entries.first().map(identity);
        let kept: Vec<LogEntry> = match first {
            Some(first) => page
                .into_iter()
                .take_while(|entry| identity(entry) != first)
                .collect(),
            None => page,
        };

        if kept.is_empty() {
            return Landed::Unchanged;
        }

        entries.splice(0..0, kept);
        entries.truncate(HELD_ENTRIES);
        Landed::Prepended
    }
}

/// The protocol's own search bound.
const MAX_SEARCH: usize = 256;

/// One entry's identity, for merging. The pair is what the daemon wrote: two
/// entries at the same instant with the same text are the same line.
fn identity(entry: &LogEntry) -> (String, String) {
    (entry.time.clone(), entry.message.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(time: &str, message: &str) -> LogEntry {
        LogEntry {
            time: time.to_string(),
            level: LogLevel::Info,
            subsystem: None,
            message: message.to_string(),
        }
    }

    fn model() -> Rc<LogsModel> {
        let peer = Rc::new(super::super::peer::FixturePeer::new("default"));
        let service = Rc::new(crate::service::runner::ServiceRunner::new(
            "/nonexistent/fermix",
        ));
        LogsModel::new(SettingsModel::new(peer, service))
    }

    #[test]
    fn the_first_page_becomes_the_list() {
        let model = model();
        let landed = model.merge_newer(vec![entry("1", "a"), entry("2", "b")]);

        assert_eq!(landed, Landed::Replaced);
        assert_eq!(model.count(), 2);
    }

    #[test]
    fn a_page_that_overlaps_appends_only_what_is_new() {
        let model = model();
        model.merge_newer(vec![entry("1", "a"), entry("2", "b")]);

        let landed = model.merge_newer(vec![entry("1", "a"), entry("2", "b"), entry("3", "c")]);

        assert_eq!(landed, Landed::Appended);
        assert_eq!(model.count(), 3);
    }

    #[test]
    fn a_page_with_nothing_new_leaves_the_list_alone() {
        let model = model();
        model.merge_newer(vec![entry("1", "a")]);

        assert_eq!(
            model.merge_newer(vec![entry("1", "a")]),
            Landed::Unchanged,
            "a poll that finds nothing new must not move the list"
        );
        assert_eq!(model.count(), 1);
    }

    #[test]
    fn a_window_that_has_moved_past_everything_held_replaces_it() {
        let model = model();
        model.merge_newer(vec![entry("1", "a")]);

        let landed = model.merge_newer(vec![entry("9", "z")]);

        assert_eq!(landed, Landed::Replaced);
        assert_eq!(model.entries()[0].message, "z");
    }

    #[test]
    fn an_older_page_goes_in_front_without_repeating_what_is_held() {
        let model = model();
        model.merge_newer(vec![entry("5", "e")]);

        let landed = model.merge_older(vec![entry("3", "c"), entry("4", "d"), entry("5", "e")]);

        assert_eq!(landed, Landed::Prepended);
        let held: Vec<String> = model
            .entries()
            .iter()
            .map(|entry| entry.message.clone())
            .collect();
        assert_eq!(held, vec!["c", "d", "e"]);
    }

    #[test]
    fn the_held_window_is_bounded() {
        let model = model();
        let page: Vec<LogEntry> = (0..HELD_ENTRIES + 10)
            .map(|index| entry(&index.to_string(), "line"))
            .collect();

        model.merge_newer(page);
        assert!(model.count() <= HELD_ENTRIES + 10);

        let next: Vec<LogEntry> = (HELD_ENTRIES..HELD_ENTRIES + 20)
            .map(|index| entry(&index.to_string(), "line"))
            .collect();
        model.merge_newer(next);

        assert!(model.count() <= HELD_ENTRIES, "the window is bounded");
    }

    #[test]
    fn every_level_has_a_word() {
        for level in LEVELS {
            assert!(!crate::copy::text(level_word(*level)).is_empty());
        }
        assert_eq!(LEVELS.len(), 8);
    }
}
