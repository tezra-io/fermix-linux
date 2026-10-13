//! Logs: `logs.query` pages and the lines the app holds from them. The daemon
//! owns the reads; the app never opens a log file. Each page comes back oldest
//! first, and the lines are held newest first, as the view shows them. A poll
//! repeats the cursorless query and prepends what is new at the head; "Load
//! older" follows the cursor and appends at the tail. Only `backward` paging is
//! used: `forward` overlaps its cursor's page.
//!
//! Messages are log-redacted but not path-scrubbed: they hold `/home/<user>`.
//! They are shown as they are and never logged, so no error built here quotes one.

use crate::management::{CallError, Management};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Entries per page, for the first page, each poll, and each older page.
pub const PAGE_LIMIT: u32 = 200;
/// The daemon refuses a longer search, counted in UTF-8 bytes.
pub const MAX_SEARCH_BYTES: usize = 256;
/// The most lines the view holds; past it the oldest, at the tail, are let go.
pub const MAX_LINES: usize = 5_000;

pub const SEARCH_TOO_LONG: &str = "That search is longer than Fermix accepts, so it was not sent.";
pub const ROTATED: &str = "The log rotated, so the view restarted from the newest entries.";

/// Minimum severities, most severe first, as `level` names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Emergency,
    Alert,
    Critical,
    Error,
    Warning,
    Notice,
    Info,
    Debug,
}

impl Level {
    pub const ALL: [Level; 8] = [
        Level::Emergency,
        Level::Alert,
        Level::Critical,
        Level::Error,
        Level::Warning,
        Level::Notice,
        Level::Info,
        Level::Debug,
    ];

    pub fn wire(self) -> &'static str {
        match self {
            Level::Emergency => "emergency",
            Level::Alert => "alert",
            Level::Critical => "critical",
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Notice => "notice",
            Level::Info => "info",
            Level::Debug => "debug",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Emergency => "Emergency",
            Level::Alert => "Alert",
            Level::Critical => "Critical",
            Level::Error => "Error",
            Level::Warning => "Warning",
            Level::Notice => "Notice",
            Level::Info => "Info",
            Level::Debug => "Debug",
        }
    }
}

/// What the view asks for. A cursor belongs to the filter it was minted under.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    /// The least severe level shown; `None` is every level.
    pub level: Option<Level>,
    /// Checked by `search_term`: never blank, never past `MAX_SEARCH_BYTES`.
    pub search: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LogPage {
    /// Oldest first.
    pub entries: Vec<LogEntry>,
    /// The daemon's size cap dropped this page's oldest rows; the cursor still
    /// points just past the kept ones, so the next older page brings them.
    #[serde(default)]
    pub truncated: bool,
    /// The next older page's anchor, or `None` at the honest end.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LogEntry {
    /// The file's own timestamp with the host's offset. Never sorted on.
    pub time: String,
    pub level: String,
    #[serde(default)]
    pub subsystem: Option<String>,
    pub message: String,
}

impl Management {
    /// One page ending at `cursor`, or the newest page when there is none.
    pub fn logs_query(&self, filter: &Filter, cursor: Option<&str>) -> Result<LogPage, CallError> {
        let page = self.call("logs.query", query_params(filter, cursor))?;
        // Not the serde error: it can quote a message, and messages hold home paths.
        serde_json::from_value(page)
            .map_err(|_| CallError::Protocol("logs.query answered an unexpected shape".into()))
    }
}

/// The params for one page. The search key is `search`; `query` is refused.
pub fn query_params(filter: &Filter, cursor: Option<&str>) -> Value {
    let mut params = Map::new();
    params.insert("limit".into(), json!(PAGE_LIMIT));
    if let Some(level) = filter.level {
        params.insert("level".into(), json!(level.wire()));
    }
    if let Some(search) = &filter.search {
        assert!(
            !search.is_empty() && search.len() <= MAX_SEARCH_BYTES,
            "a search is checked by search_term before it is sent"
        );
        params.insert("search".into(), json!(search));
    }
    if let Some(cursor) = cursor {
        assert!(!cursor.is_empty(), "a cursor is the daemon's, never blank");
        params.insert("cursor".into(), json!(cursor));
    }
    Value::Object(params)
}

/// The search to send for what was typed: trimmed, and `None` when blank,
/// because the daemon refuses an empty one. Too long is refused here, in words,
/// since the daemon's own refusal names no field.
pub fn search_term(typed: &str) -> Result<Option<String>, &'static str> {
    let typed = typed.trim();
    if typed.len() > MAX_SEARCH_BYTES {
        return Err(SEARCH_TOO_LONG);
    }
    Ok((!typed.is_empty()).then(|| typed.to_owned()))
}

type Key<'a> = (&'a str, &'a str, Option<&'a str>, &'a str);

fn key(entry: &LogEntry) -> Key<'_> {
    (
        &entry.time,
        &entry.level,
        entry.subsystem.as_deref(),
        &entry.message,
    )
}

/// What one merge changed, as the edit a list model holding the same lines
/// takes: `newer` inserted at the head, then `dropped` rows removed from the
/// tail and `older` added there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Merged {
    /// Rows added at the head, newest first.
    pub newer: Vec<LogEntry>,
    /// Rows let go from the tail, the oldest, to stay under the cap.
    pub dropped: usize,
    /// Rows added at the tail, newest first.
    pub older: Vec<LogEntry>,
}

/// The lines the view holds, newest first, with no gaps and no line twice:
/// past the cap the oldest are let go from the tail and older history ends
/// there, rather than leave a hole.
#[derive(Debug, Clone)]
pub struct LogLines {
    entries: Vec<LogEntry>,
    older: Option<String>,
    capped: bool,
    cap: usize,
}

impl LogLines {
    /// The first page for a filter, which comes oldest first.
    pub fn new(first: LogPage, cap: usize) -> Self {
        assert!(cap > 0, "the view must hold at least one line");
        let mut entries = first.entries;
        entries.reverse();
        let mut lines = LogLines {
            entries,
            older: first.cursor,
            capped: false,
            cap,
        };
        lines.trim_oldest();
        lines
    }

    /// Newest first, as the view shows them.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn older_cursor(&self) -> Option<&str> {
        self.older.as_deref()
    }

    pub fn can_load_older(&self) -> bool {
        self.older.is_some()
    }

    /// Whether the cap has let lines go, which also ends older history.
    pub fn capped(&self) -> bool {
        self.capped
    }

    /// A poll: the newest page again. Its entries already held sit among the
    /// first `page.len()` held rows, so only those are compared.
    pub fn prepend_newer(&mut self, poll: LogPage) -> Merged {
        let end = self.entries.len().min(poll.entries.len());
        let recent: HashSet<Key<'_>> = self.entries[..end].iter().map(key).collect();
        let newer: Vec<LogEntry> = poll
            .entries
            .into_iter()
            .rev()
            .filter(|e| !recent.contains(&key(e)))
            .collect();
        self.entries.splice(0..0, newer.iter().cloned());
        let dropped = self.trim_oldest();
        Merged {
            newer,
            dropped,
            older: Vec::new(),
        }
    }

    /// "Load older". Lines logged since the cursor was minted shift its window
    /// newer, so rows already held come back and are dropped here. Past the cap
    /// only the rows next to the oldest held one are kept.
    pub fn append_older(&mut self, page: LogPage) -> Merged {
        let held: HashSet<Key<'_>> = self.entries.iter().map(key).collect();
        let mut older: Vec<LogEntry> = page
            .entries
            .into_iter()
            .rev()
            .filter(|e| !held.contains(&key(e)))
            .collect();
        self.older = page.cursor;
        let room = self.cap.saturating_sub(self.entries.len());
        if older.len() > room {
            older.truncate(room);
            self.older = None;
            self.capped = true;
        }
        self.entries.extend(older.iter().cloned());
        Merged {
            older,
            ..Merged::default()
        }
    }

    fn trim_oldest(&mut self) -> usize {
        let over = self.entries.len().saturating_sub(self.cap);
        if over > 0 {
            self.entries.truncate(self.cap);
            self.older = None;
            self.capped = true;
        }
        over
    }
}

/// Said at the oldest end once the cap has let lines go.
pub fn capped_note() -> String {
    format!("This view keeps the newest {MAX_LINES} lines.")
}

/// One line as copied: the raw time, the level, and the message, which already
/// begins with its `[subsystem]` tag where it has one.
pub fn line_text(entry: &LogEntry) -> String {
    format!("{} {} {}", entry.time, entry.level, entry.message)
}

/// The lines as copied, one to a line, in the order given.
pub fn copy_text(entries: &[LogEntry]) -> String {
    entries.iter().map(line_text).collect::<Vec<_>>().join("\n")
}

/// `2026-09-24T21:12:34.651701-04:00` shows as `2026-09-24 21:12:34`; a time
/// in any other shape shows as sent.
pub fn shown_time(raw: &str) -> String {
    let shaped = raw.len() >= 19 && raw.is_char_boundary(19) && raw.as_bytes()[10] == b'T';
    if !shaped {
        return raw.to_owned();
    }
    format!("{} {}", &raw[..10], &raw[11..19])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emphasis {
    Alarm,
    Warn,
    Plain,
    Quiet,
}

pub fn emphasis(level: &str) -> Emphasis {
    match level {
        "emergency" | "alert" | "critical" | "error" => Emphasis::Alarm,
        "warning" => Emphasis::Warn,
        "debug" => Emphasis::Quiet,
        _ => Emphasis::Plain,
    }
}
