//! How a turn's work reads on the Chat page: each tool's name, what the live
//! line says while it runs, and the line a finished group folds into.

use fermix_client::acp::ToolKind;
use fermix_client::activity::{elapsed, live_meta, live_phrase, summary, tool_words};
use fermix_client::chat::{RunState, ToolRun};

fn run(name: &str, state: RunState, started: i64, ended: Option<i64>) -> ToolRun {
    ToolRun {
        id: format!("{name}-{started}"),
        name: name.into(),
        kind: ToolKind::Other,
        state,
        started,
        ended,
    }
}

#[test]
fn an_engine_tool_has_a_name_a_phrase_and_an_icon() {
    let search = tool_words("web_search", ToolKind::Fetch);
    assert_eq!(search.name, "Web search");
    assert_eq!(search.detail, None);
    assert_eq!(search.phrase, "Searching the web");
    assert_eq!(search.icon, "system-search-symbolic");
    let shell = tool_words("shell", ToolKind::Execute);
    assert_eq!(
        (shell.name.as_str(), shell.phrase.as_str()),
        ("Terminal", "Running a command")
    );
    assert_eq!(shell.icon, "utilities-terminal-symbolic");
    let memory = tool_words("memory_recall", ToolKind::Read);
    assert_eq!(
        (memory.name.as_str(), memory.phrase.as_str()),
        ("Memory recall", "Searching memory")
    );
}

#[test]
fn a_plugin_tool_is_named_for_its_plugin_and_what_it_did() {
    let events = tool_words("google_calendar_search_events", ToolKind::Fetch);
    assert_eq!(events.name, "Google Calendar");
    assert_eq!(events.detail.as_deref(), Some("Search events"));
    assert_eq!(events.phrase, "Checking Google Calendar");
    assert_eq!(events.icon, "x-office-calendar-symbolic");
    let send = tool_words("gmail_send_message", ToolKind::Execute);
    assert_eq!(send.name, "Gmail");
    assert_eq!(send.detail.as_deref(), Some("Send message"));
    assert_eq!(send.phrase, "Using Gmail");
    assert_eq!(send.icon, "mail-unread-symbolic");
}

#[test]
fn an_unknown_tool_reads_as_words_with_its_kinds_icon() {
    let github = tool_words("mcp_github_create_issue", ToolKind::Execute);
    assert_eq!(github.name, "Github create issue");
    assert_eq!(github.phrase, "Using Github create issue");
    assert_eq!(github.icon, "utilities-terminal-symbolic");
    for (kind, icon) in [
        (ToolKind::Read, "document-open-symbolic"),
        (ToolKind::Fetch, "web-browser-symbolic"),
        (ToolKind::Execute, "utilities-terminal-symbolic"),
        (ToolKind::Other, "system-run-symbolic"),
    ] {
        assert_eq!(tool_words("frobnicate.widget", kind).icon, icon, "{kind:?}");
    }
    assert_eq!(
        tool_words("frobnicate.widget", ToolKind::Other).name,
        "Frobnicate widget"
    );
    let blank = tool_words("", ToolKind::Other);
    assert_eq!(
        (blank.name.as_str(), blank.phrase.as_str()),
        ("Tool", "Using a tool")
    );
}

#[test]
fn the_live_line_says_what_is_running_or_that_fermix_is_thinking() {
    assert_eq!(live_phrase(&[], false), "Thinking");
    let calendar = run("google_calendar_search_events", RunState::Running, 1, None);
    let search = run("web_search", RunState::Done, 0, Some(1));
    assert_eq!(
        live_phrase(&[search.clone(), calendar.clone()], false),
        "Checking Google Calendar"
    );
    assert_eq!(
        live_phrase(std::slice::from_ref(&search), false),
        "Thinking",
        "between tools, Fermix is thinking again"
    );
    assert_eq!(live_phrase(&[search, calendar], true), "Stopping…");
}

#[test]
fn elapsed_time_is_short_and_waits_a_moment_before_it_shows() {
    assert_eq!(elapsed(0), "0s");
    assert_eq!(elapsed(12), "12s");
    assert_eq!(elapsed(60), "1m");
    assert_eq!(elapsed(65), "1m 5s");
    assert_eq!(live_meta(0, 2), None, "a quick answer shows no clock");
    assert_eq!(live_meta(0, 3).as_deref(), Some("3s"));
    assert_eq!(live_meta(2, 1).as_deref(), Some("2 done · 1s"));
    assert_eq!(live_meta(2, 14).as_deref(), Some("2 done · 14s"));
}

#[test]
fn a_finished_group_folds_into_one_line() {
    let runs = [
        run("web_search", RunState::Done, 10, Some(12)),
        run("shell", RunState::Failed, 12, Some(15)),
        run("file_read", RunState::Done, 15, Some(19)),
    ];
    let folded = summary(&runs);
    assert_eq!(folded.text, "Used 3 tools · 9s");
    assert_eq!(folded.failed.as_deref(), Some("1 failed"));
    assert_eq!(folded.ended, None);
    assert_eq!(folded.label, "Used 3 tools, 1 failed, 9 seconds");
    let one = summary(&[run("web_search", RunState::Done, 10, Some(10))]);
    assert_eq!(
        one.text, "Used 1 tool",
        "no time when it took under a second"
    );
    assert_eq!(one.failed, None);
    assert_eq!(one.label, "Used 1 tool");
    let stopped = summary(&[
        run("shell", RunState::Stopped, 10, Some(75)),
        run("web_search", RunState::Unfinished, 11, Some(75)),
    ]);
    assert_eq!(stopped.text, "Used 2 tools · 1m 5s");
    assert_eq!(stopped.ended.as_deref(), Some("stopped"));
    assert_eq!(stopped.label, "Used 2 tools, stopped, 1 minute 5 seconds");
    let cut = summary(&[run("shell", RunState::Unfinished, 10, Some(11))]);
    assert_eq!(cut.ended.as_deref(), Some("didn't finish"));
}
