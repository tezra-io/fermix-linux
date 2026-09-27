//! How a turn's work reads on the Chat page: each tool's name, what the live
//! line says while Fermix works, and the one line a finished group folds into.
//! The wire gives only a tool's engine name, its coarse kind and its status,
//! so the words come from the engine's own tool names (`builtin.ex`, the
//! bundled plugins, MCP's `mcp_` prefix), with a plain fallback for the rest.

use crate::acp::ToolKind;
use crate::chat::{RunState, ToolRun};

/// A tool as a person reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolWords {
    /// The row's name: "Web search", "Google Calendar".
    pub name: String,
    /// What a plugin's tool did, beside its name: "Search events".
    pub detail: Option<String>,
    /// The live line while it runs: "Searching the web".
    pub phrase: String,
    /// A symbolic icon from the system theme.
    pub icon: &'static str,
}

/// The engine's built-in tools: name, live phrase, and an icon when the
/// kind's own icon says less.
const BUILTIN: &[(&str, &str, &str, Option<&str>)] = &[
    ("shell", "Terminal", "Running a command", None),
    ("file_read", "File read", "Reading a file", None),
    (
        "view_image",
        "View image",
        "Looking at an image",
        Some("image-x-generic-symbolic"),
    ),
    ("file_write", "File write", "Writing a file", None),
    ("file_edit", "File edit", "Editing a file", None),
    ("glob_search", "File search", "Finding files", Some(SEARCH)),
    (
        "content_search",
        "Content search",
        "Searching files",
        Some(SEARCH),
    ),
    ("git_read", "Git read", "Reading the repository", None),
    ("git_write", "Git write", "Updating the repository", None),
    ("web_fetch", "Web page", "Reading a web page", None),
    (
        "web_search",
        "Web search",
        "Searching the web",
        Some(SEARCH),
    ),
    (
        "place_search",
        "Place search",
        "Looking up places",
        Some(SEARCH),
    ),
    (
        "recall_activity",
        "Recent activity",
        "Recalling recent activity",
        None,
    ),
    ("skill_create", "Skill create", "Creating a skill", None),
    ("skill_reload", "Skill reload", "Reloading skills", None),
    ("skill_view", "Skill view", "Reading a skill", None),
    ("skill_run", "Skill run", "Running a skill", None),
    ("skill_list", "Skill list", "Checking skills", None),
    ("subagents", "Helper agents", "Asking helper agents", None),
    ("tool_help", "Tool lookup", "Looking up tools", None),
    (
        "tool_search",
        "Tool lookup",
        "Looking up tools",
        Some(SEARCH),
    ),
    ("tool_describe", "Tool lookup", "Looking up tools", None),
    ("tool_call", "Tool call", "Calling a tool", None),
    (
        "memory_recall",
        "Memory recall",
        "Searching memory",
        Some(SEARCH),
    ),
    ("memory_store", "Memory store", "Saving to memory", None),
    ("schedule_job", "Schedule job", "Scheduling a job", None),
    ("update_job", "Update job", "Updating a job", None),
    (
        "list_jobs",
        "Scheduled jobs",
        "Checking scheduled jobs",
        None,
    ),
    ("pause_job", "Pause job", "Pausing a job", None),
    ("resume_job", "Resume job", "Resuming a job", None),
    ("remove_job", "Remove job", "Removing a job", None),
    ("run_job_now", "Run job", "Running a job", None),
    ("list_job_runs", "Job runs", "Checking job runs", None),
    ("get_job_run", "Job run", "Checking a job run", None),
    ("browser", "Browser", "Using the browser", None),
    ("react", "Reaction", "Reacting", None),
    ("computer_use", "Computer use", "Using the computer", None),
    ("codex_run", "Codex", "Running Codex", None),
    (
        "claude_code_run",
        "Claude Code",
        "Running Claude Code",
        None,
    ),
    (
        "codex_cloud_run",
        "Codex Cloud",
        "Running Codex Cloud",
        None,
    ),
    (
        "get_coding_run",
        "Coding run",
        "Checking a coding run",
        None,
    ),
    (
        "event_store",
        "Event store",
        "Saving an event",
        Some(CALENDAR),
    ),
    ("event_list", "Events", "Checking events", Some(CALENDAR)),
    (
        "event_update",
        "Event update",
        "Updating an event",
        Some(CALENDAR),
    ),
    (
        "event_remove",
        "Event remove",
        "Removing an event",
        Some(CALENDAR),
    ),
];

/// The bundled plugins, whose tools are named `<plugin>_<action>`.
const PLUGINS: &[(&str, &str, &str)] = &[
    ("google_calendar_", "Google Calendar", CALENDAR),
    ("google_drive_", "Google Drive", "folder-remote-symbolic"),
    ("gmail_", "Gmail", "mail-unread-symbolic"),
];

const SEARCH: &str = "system-search-symbolic";
const CALENDAR: &str = "x-office-calendar-symbolic";
/// A plugin action that only looks: "Checking Gmail" rather than "Using Gmail".
const LOOKING: &[&str] = &["search", "get", "list", "find", "read"];

pub fn tool_words(name: &str, kind: ToolKind) -> ToolWords {
    if let Some(&(_, label, phrase, icon)) = BUILTIN.iter().find(|(raw, ..)| *raw == name) {
        return ToolWords {
            name: label.into(),
            detail: None,
            phrase: phrase.into(),
            icon: icon.unwrap_or(kind_icon(kind)),
        };
    }
    if let Some(words) = plugin_words(name) {
        return words;
    }
    let plain = words(name.strip_prefix("mcp_").unwrap_or(name));
    let phrase = match &plain {
        Some(plain) => format!("Using {plain}"),
        None => "Using a tool".into(),
    };
    ToolWords {
        name: plain.unwrap_or_else(|| "Tool".into()),
        detail: None,
        phrase,
        icon: kind_icon(kind),
    }
}

fn plugin_words(name: &str) -> Option<ToolWords> {
    let (action, title, icon) = PLUGINS
        .iter()
        .find_map(|&(prefix, title, icon)| Some((name.strip_prefix(prefix)?, title, icon)))?;
    let looks = LOOKING
        .iter()
        .any(|verb| action.split('_').next() == Some(*verb));
    let verb = if looks { "Checking" } else { "Using" };
    Some(ToolWords {
        name: title.into(),
        detail: words(action),
        phrase: format!("{verb} {title}"),
        icon,
    })
}

fn kind_icon(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "document-open-symbolic",
        ToolKind::Fetch => "web-browser-symbolic",
        ToolKind::Execute => "utilities-terminal-symbolic",
        ToolKind::Other => "system-run-symbolic",
    }
}

/// "memory.recall" as "Memory recall"; `None` when there is no word in it.
fn words(name: &str) -> Option<String> {
    let words: Vec<&str> = name
        .split(|c: char| c == '_' || c == '.' || c == '-' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    let joined = words.join(" ");
    let mut chars = joined.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// What the live line says: the tool running now (the latest, when several
/// are), "Thinking" between tools and before any, "Stopping…" once asked to.
pub fn live_phrase(runs: &[ToolRun], stopping: bool) -> String {
    if stopping {
        return "Stopping…".into();
    }
    match runs.iter().rev().find(|run| run.state == RunState::Running) {
        Some(run) => tool_words(&run.name, run.kind).phrase,
        None => "Thinking".into(),
    }
}

/// Seconds since a run started, "12s" or "1m 5s".
pub fn elapsed(seconds: i64) -> String {
    let seconds = seconds.max(0);
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s}s"),
        (m, 0) => format!("{m}m"),
        (m, s) => format!("{m}m {s}s"),
    }
}

/// The small text after the live phrase: nothing for the first moments, then
/// the time, and how many tools have finished once one has.
pub fn live_meta(finished: usize, seconds: i64) -> Option<String> {
    match finished {
        0 if seconds < 3 => None,
        0 => Some(elapsed(seconds)),
        n => Some(format!("{n} done · {}", elapsed(seconds))),
    }
}

/// A finished group's one line, with its parts drawn apart: the failures in
/// the warning colour, how it was cut short in dim text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// "Used 3 tools · 9s"; no time under a second.
    pub text: String,
    /// "1 failed".
    pub failed: Option<String>,
    /// "stopped" or "didn't finish".
    pub ended: Option<String>,
    /// All of it for a screen reader, in words.
    pub label: String,
}

pub fn summary(runs: &[ToolRun]) -> Summary {
    assert!(!runs.is_empty(), "a group holds at least one run");
    let used = match runs.len() {
        1 => "Used 1 tool".to_owned(),
        n => format!("Used {n} tools"),
    };
    let seconds = took(runs);
    let failed = runs.iter().filter(|r| r.state == RunState::Failed).count();
    let failed = (failed > 0).then(|| format!("{failed} failed"));
    let ended = if runs.iter().any(|r| r.state == RunState::Stopped) {
        Some("stopped".to_owned())
    } else if runs.iter().any(|r| r.state == RunState::Unfinished) {
        Some("didn't finish".to_owned())
    } else {
        None
    };
    let text = match seconds {
        0 => used.clone(),
        s => format!("{used} · {}", elapsed(s)),
    };
    let parts = [Some(used), failed.clone(), ended.clone(), spoken(seconds)];
    Summary {
        text,
        failed,
        ended,
        label: parts.into_iter().flatten().collect::<Vec<_>>().join(", "),
    }
}

/// From the first start to the last end.
fn took(runs: &[ToolRun]) -> i64 {
    let first = runs.iter().map(|r| r.started).min().unwrap_or_default();
    let last = runs.iter().filter_map(|r| r.ended).max().unwrap_or(first);
    (last - first).max(0)
}

fn spoken(seconds: i64) -> Option<String> {
    let unit = |n: i64, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
    match (seconds / 60, seconds % 60) {
        (0, 0) => None,
        (0, s) => Some(unit(s, "second")),
        (m, 0) => Some(unit(m, "minute")),
        (m, s) => Some(format!("{} {}", unit(m, "minute"), unit(s, "second"))),
    }
}
