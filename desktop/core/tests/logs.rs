//! Logs: pages decoded from the engine's own fixtures, the query the app sends,
//! and the merge that keeps the held lines newest first with no duplicates.

use fermix_client::frame::{read_frame, write_frame};
use fermix_client::logs::{
    capped_note, copy_text, emphasis, line_text, query_params, search_term, shown_time, Emphasis,
    Filter, Level, LogEntry, LogLines, LogPage, Merged, MAX_SEARCH_BYTES, PAGE_LIMIT,
    SEARCH_TOO_LONG,
};
use fermix_client::management::{decode_response, CallError, Management};
use serde_json::{json, Value};
use std::os::unix::net::UnixListener;
use std::thread;
use std::time::Duration;

const SUCCESS: &str = include_str!("../contracts/management/fixtures/success.jsonl");
const ERRORS: &str = include_str!("../contracts/management/fixtures/errors.jsonl");

fn answer(name: &str) -> Result<Value, CallError> {
    let found = SUCCESS
        .lines()
        .chain(ERRORS.lines())
        .map(|line| serde_json::from_str::<Value>(line).expect("fixture line is JSON"))
        .find(|v| v["name"] == name)
        .unwrap_or_else(|| panic!("no fixture named {name}"));
    let response = &found["response"];
    let id = response["request_id"].as_str().unwrap();
    decode_response(&serde_json::to_vec(response).unwrap(), id)
}

fn entry(n: u32) -> LogEntry {
    LogEntry {
        time: format!("2026-09-24T21:00:{n:02}.000000-04:00"),
        level: "info".into(),
        subsystem: None,
        message: format!("line {n}"),
    }
}

/// A page of entries `from..to`, oldest first, as the daemon sends it.
fn page(from: u32, to: u32, cursor: Option<&str>) -> LogPage {
    LogPage {
        entries: (from..to).map(entry).collect(),
        truncated: false,
        cursor: cursor.map(str::to_owned),
    }
}

fn numbers_of(entries: &[LogEntry]) -> Vec<u32> {
    entries
        .iter()
        .map(|e| e.message.trim_start_matches("line ").parse().unwrap())
        .collect()
}

/// The held lines' numbers, in the order the view shows them.
fn numbers(lines: &LogLines) -> Vec<u32> {
    numbers_of(lines.entries())
}

/// A list model taking a merge as the app's store does: `newer` at the head,
/// then `dropped` let go from the tail and `older` added there.
fn take(model: &mut Vec<LogEntry>, merged: Merged) {
    model.splice(0..0, merged.newer);
    model.truncate(model.len() - merged.dropped);
    model.extend(merged.older);
}

#[test]
fn the_published_page_decodes_oldest_first_with_its_older_cursor() {
    let page: LogPage = serde_json::from_value(answer("logs_query").unwrap()).unwrap();
    assert_eq!(page.entries.len(), 2);
    assert!(page.entries[0].time < page.entries[1].time, "oldest first");
    assert_eq!(page.entries[0].subsystem.as_deref(), Some("realtime"));
    assert_eq!(page.entries[1].subsystem, None);
    assert_eq!(page.entries[1].level, "warning");
    assert!(!page.truncated);
    assert!(page.cursor.is_some());
}

#[test]
fn an_empty_log_is_an_empty_page_with_no_cursor() {
    let page: LogPage = serde_json::from_value(json!({
        "entries": [], "count": 0, "truncated": false, "direction": "backward", "cursor": null,
        "someday": "a newer field"
    }))
    .unwrap();
    let lines = LogLines::new(page, 100);
    assert!(lines.entries().is_empty());
    assert!(!lines.can_load_older());
}

#[test]
fn a_rotated_cursor_is_a_refusal_with_the_daemons_sentence() {
    let Err(CallError::Refused(expired)) = answer("cursor_expired") else {
        panic!("cursor_expired is a refusal");
    };
    assert_eq!(expired.code, "cursor_expired");
    assert_eq!(
        expired.sentence,
        "The log cursor predates a rotation and cannot be resumed."
    );
}

#[test]
fn the_query_sends_search_never_query_and_never_a_direction() {
    let plain = query_params(&Filter::default(), None);
    assert_eq!(plain, json!({"limit": PAGE_LIMIT}));
    let filter = Filter {
        level: Some(Level::Warning),
        search: Some("timeout".into()),
    };
    assert_eq!(
        query_params(&filter, Some("abc")),
        json!({"limit": 200, "level": "warning", "search": "timeout", "cursor": "abc"})
    );
}

#[test]
fn levels_are_the_daemons_eight_most_severe_first() {
    let wire: Vec<&str> = Level::ALL.iter().map(|l| l.wire()).collect();
    assert_eq!(
        wire,
        [
            "emergency",
            "alert",
            "critical",
            "error",
            "warning",
            "notice",
            "info",
            "debug"
        ]
    );
    assert_eq!(Level::Warning.label(), "Warning");
    assert_eq!(Level::Emergency.label(), "Emergency");
}

#[test]
fn a_search_is_trimmed_blank_is_no_search_and_the_limit_counts_bytes() {
    assert_eq!(search_term(""), Ok(None));
    assert_eq!(search_term("   "), Ok(None));
    assert_eq!(search_term(" timeout "), Ok(Some("timeout".into())));
    let longest = "a".repeat(MAX_SEARCH_BYTES);
    assert_eq!(search_term(&longest), Ok(Some(longest.clone())));
    assert_eq!(search_term(&format!("{longest}a")), Err(SEARCH_TOO_LONG));
    // 129 characters, 258 bytes: short by count, too long on the wire.
    assert_eq!(search_term(&"é".repeat(129)), Err(SEARCH_TOO_LONG));
    assert_eq!(MAX_SEARCH_BYTES, 256);
}

#[test]
fn the_first_page_is_held_newest_first_with_its_cursor_for_older() {
    let lines = LogLines::new(page(0, 5, Some("c1")), 100);
    assert_eq!(numbers(&lines), [4, 3, 2, 1, 0]);
    assert_eq!(lines.older_cursor(), Some("c1"));
    assert!(lines.can_load_older());
    assert!(!lines.capped());
}

#[test]
fn a_poll_prepends_only_the_newer_lines_and_keeps_the_older_cursor() {
    let mut lines = LogLines::new(page(0, 5, Some("c1")), 100);
    let merged = lines.prepend_newer(page(2, 8, Some("poll")));
    assert_eq!(numbers_of(&merged.newer), [7, 6, 5], "newest first");
    assert_eq!(merged.dropped, 0);
    assert!(merged.older.is_empty());
    assert_eq!(numbers(&lines), [7, 6, 5, 4, 3, 2, 1, 0]);
    assert_eq!(lines.older_cursor(), Some("c1"), "a poll never moves it");
    let again = lines.prepend_newer(page(2, 8, Some("poll")));
    assert_eq!(
        again,
        Merged::default(),
        "the same poll twice changes nothing"
    );
    assert_eq!(numbers(&lines), [7, 6, 5, 4, 3, 2, 1, 0]);
}

#[test]
fn a_poll_with_nothing_in_common_prepends_it_all() {
    let mut lines = LogLines::new(page(0, 3, None), 100);
    let merged = lines.prepend_newer(page(10, 12, None));
    assert_eq!(numbers_of(&merged.newer), [11, 10]);
    assert_eq!(numbers(&lines), [11, 10, 2, 1, 0]);
}

#[test]
fn load_older_appends_replaces_the_cursor_and_drops_the_shifted_overlap() {
    let mut lines = LogLines::new(page(10, 15, Some("c1")), 100);
    lines.prepend_newer(page(12, 17, None));
    // Two lines were logged since c1 was minted, so the older page overlaps by two.
    let merged = lines.append_older(page(7, 12, Some("c2")));
    assert!(merged.newer.is_empty());
    assert_eq!(merged.dropped, 0);
    assert_eq!(numbers_of(&merged.older), [9, 8, 7], "newest first");
    assert_eq!(numbers(&lines), [16, 15, 14, 13, 12, 11, 10, 9, 8, 7]);
    assert_eq!(lines.older_cursor(), Some("c2"));
    lines.append_older(page(5, 7, None));
    assert_eq!(numbers(&lines)[8..], [8, 7, 6, 5]);
    assert!(!lines.can_load_older(), "a null cursor is the honest end");
}

#[test]
fn past_the_cap_a_poll_lets_the_oldest_go_from_the_tail_and_ends_older_history() {
    let mut lines = LogLines::new(page(0, 4, Some("c1")), 5);
    let merged = lines.prepend_newer(page(4, 7, None));
    assert_eq!(numbers_of(&merged.newer), [6, 5, 4]);
    assert_eq!(merged.dropped, 2);
    assert_eq!(numbers(&lines), [6, 5, 4, 3, 2]);
    assert!(lines.capped());
    assert_eq!(lines.older_cursor(), None, "older lines would leave a gap");
    assert!(!lines.can_load_older());
}

#[test]
fn past_the_cap_load_older_keeps_the_lines_next_to_the_oldest_held() {
    let mut lines = LogLines::new(page(10, 13, Some("c1")), 5);
    let merged = lines.append_older(page(5, 10, Some("c2")));
    assert_eq!(merged.dropped, 0);
    assert_eq!(numbers_of(&merged.older), [9, 8]);
    assert_eq!(numbers(&lines), [12, 11, 10, 9, 8]);
    assert!(lines.capped());
    assert!(!lines.can_load_older());
}

#[test]
fn a_first_page_larger_than_the_cap_keeps_its_newest_lines() {
    let lines = LogLines::new(page(0, 8, Some("c1")), 5);
    assert_eq!(numbers(&lines), [7, 6, 5, 4, 3]);
    assert!(lines.capped());
    assert!(!lines.can_load_older());
}

#[test]
fn a_list_model_taking_each_merge_mirrors_the_held_lines() {
    let mut lines = LogLines::new(page(10, 13, Some("c1")), 8);
    let mut model = lines.entries().to_vec();
    let check = |lines: &LogLines, model: &[LogEntry]| {
        assert_eq!(numbers_of(model), numbers(lines));
    };
    take(&mut model, lines.prepend_newer(page(11, 15, None)));
    check(&lines, &model);
    take(&mut model, lines.append_older(page(8, 12, Some("c2"))));
    check(&lines, &model);
    take(&mut model, lines.prepend_newer(page(14, 17, None)));
    check(&lines, &model);
    assert_eq!(numbers(&lines), [16, 15, 14, 13, 12, 11, 10, 9]);
    // More new lines than the cap holds: some of this poll's own oldest go too.
    take(&mut model, lines.prepend_newer(page(17, 27, None)));
    check(&lines, &model);
    assert_eq!(numbers(&lines), [26, 25, 24, 23, 22, 21, 20, 19]);
    assert!(lines.capped());
}

#[test]
fn copy_reads_as_shown_newest_first() {
    let lines = LogLines::new(page(0, 3, None), 100);
    let copied = copy_text(lines.entries());
    let shown: Vec<&str> = copied
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(shown, ["2", "1", "0"]);
}

#[test]
fn the_cap_note_names_the_cap() {
    assert_eq!(capped_note(), "This view keeps the newest 5000 lines.");
}

#[test]
fn a_line_reads_time_level_and_message_and_copies_the_raw_time() {
    let e = LogEntry {
        time: "2026-09-24T21:12:34.651701-04:00".into(),
        level: "warning".into(),
        subsystem: Some("realtime".into()),
        message: "[realtime] socket at /home/someone/.fermix listening".into(),
    };
    assert_eq!(
        line_text(&e),
        "2026-09-24T21:12:34.651701-04:00 warning [realtime] socket at /home/someone/.fermix listening"
    );
    assert_eq!(shown_time(&e.time), "2026-09-24 21:12:34");
    assert_eq!(
        shown_time("yesterday"),
        "yesterday",
        "an odd time shows as sent"
    );
    assert_eq!(copy_text(&[entry(1), entry(2)]).lines().count(), 2);
    assert!(copy_text(&[]).is_empty());
}

#[test]
fn severe_levels_stand_out_and_debug_recedes() {
    for level in ["emergency", "alert", "critical", "error"] {
        assert_eq!(emphasis(level), Emphasis::Alarm, "{level}");
    }
    assert_eq!(emphasis("warning"), Emphasis::Warn);
    assert_eq!(emphasis("notice"), Emphasis::Plain);
    assert_eq!(emphasis("info"), Emphasis::Plain);
    assert_eq!(emphasis("debug"), Emphasis::Quiet);
    assert_eq!(emphasis("something-new"), Emphasis::Plain);
}

#[test]
fn logs_query_sends_the_filter_and_decodes_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let result = answer("logs_query").unwrap();
    let daemon = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
        let reply = json!({"request_id": request["request_id"], "result": result});
        write_frame(&mut stream, &serde_json::to_vec(&reply).unwrap()).unwrap();
        request
    });
    let client = Management::new(socket, Duration::from_millis(500));
    let filter = Filter {
        level: Some(Level::Error),
        search: Some("accept".into()),
    };
    let page = client.logs_query(&filter, None).unwrap();
    assert_eq!(page.entries.len(), 2);
    let sent = daemon.join().unwrap();
    assert_eq!(sent["method"], "logs.query");
    assert_eq!(
        sent["params"],
        json!({"limit": 200, "level": "error", "search": "accept"})
    );
}
