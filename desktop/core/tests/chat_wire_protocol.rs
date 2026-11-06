//! The chat wire's codec beyond the golden lines: what it is lenient about inside a row, what it
//! refuses, and the framing caps.

use fermix_client::chat_wire::protocol::{
    is_media_ref, line_buffer, loggable_id, read_line, Block, ClientEvent, DecodeError,
    ServerEvent, Unreadable, MAX_BLOCK_LISTS, MAX_FRAME_BYTES, MAX_LINE_BYTES, MAX_LIST_ROWS,
    MAX_LIST_TITLE, MAX_LOGGED_BYTES, NOT_LOGGED,
};
use fermix_client::checklists::ListRef;
use fermix_client::realtime::protocol::LineBuffer;
use serde_json::{json, Value};

fn decode(value: Value) -> ServerEvent {
    let event = read_line(value.to_string().as_bytes()).expect("decodes");
    assert!(
        !matches!(event, ServerEvent::Unreadable(_)),
        "a test event reads: {event:?}"
    );
    event
}

fn row(extra: Value) -> Value {
    let mut row = json!({
        "type": "row", "profile_id": "main", "server_seq": 5, "role": "assistant",
        "text": "hi", "ts": "2026-09-25T09:00:00Z", "media_refs": []
    });
    for (key, value) in extra.as_object().unwrap() {
        row[key] = value.clone();
    }
    row
}

#[test]
fn a_message_never_carries_attachments_and_absent_options_are_absent_keys() {
    let line = ClientEvent::msg("linux-1", "hello").line().unwrap();
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["attach_ids"], json!([]));
    let command: Value =
        serde_json::from_str(&ClientEvent::command("linux-2", "new", None).line().unwrap())
            .unwrap();
    assert!(command.get("args").is_none());
    let cancel: Value =
        serde_json::from_str(&ClientEvent::cancel("linux-3").line().unwrap()).unwrap();
    assert!(cancel.get("task_ref").is_none());
}

#[test]
fn a_line_at_the_daemon_cap_is_refused_before_it_is_sent() {
    let text = "x".repeat(MAX_LINE_BYTES);
    assert!(ClientEvent::msg("linux-1", &text).line().is_err());
    assert!(ClientEvent::msg("linux-1", "short").line().is_ok());
}

#[test]
fn a_row_keeps_an_unforeseen_role_and_kind() {
    let ServerEvent::Row(row) = decode(row(json!({"role": "system", "kind": "notice"}))) else {
        panic!("a row");
    };
    assert_eq!(row.role, "system");
    assert_eq!(row.kind.as_deref(), Some("notice"));
}

#[test]
fn a_row_keeps_a_ref_that_is_not_one_for_the_fetcher_to_refuse() {
    let short = "a".repeat(63);
    let media = json!({"media_refs": [{"ref": short, "kind": "image", "mime": "image/png", "size_bytes": 1}]});
    let ServerEvent::Row(row) = decode(row(media)) else {
        panic!("a row");
    };
    assert_eq!(row.media_refs.len(), 1);
    assert!(!is_media_ref(&row.media_refs[0].r#ref));
}

#[test]
fn a_malformed_media_ref_or_preview_is_skipped_and_the_row_stands() {
    let extra = json!({
        "media_refs": [{"ref": "r"}, {"ref": "b", "kind": "file", "mime": "x/y", "size_bytes": 2}],
        "link_previews": [{"url": "https://a.test/"}]
    });
    let ServerEvent::Row(row) = decode(row(extra)) else {
        panic!("a row");
    };
    assert_eq!(row.media_refs.len(), 1);
    assert!(row.link_previews.is_empty());
    assert_eq!(row.skipped, 2, "what was dropped is counted for the log");
}

#[test]
fn an_unknown_block_and_a_place_without_coordinates_are_skipped() {
    let extra = json!({"blocks": [
        {"type": "weather", "days": []},
        {"type": "places", "places": [
            {"name": "No coordinates"},
            {"name": "Here", "location": {"latitude": 1.0, "longitude": 2.0}}
        ]}
    ]});
    let ServerEvent::Row(row) = decode(row(extra)) else {
        panic!("a row");
    };
    let [Block::Places(places)] = row.blocks.as_slice() else {
        panic!("only the places block");
    };
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].name, "Here");
    assert_eq!(row.skipped, 2, "the unknown block and the place");
}

/// A place off the globe would open a map of nowhere: its coordinates are refused at decode, so
/// it is skipped and counted like any other place that does not fit.
#[test]
fn a_place_off_the_globe_is_skipped_and_counted() {
    let at = |latitude: f64, longitude: f64| {
        json!({"name": format!("{latitude},{longitude}"),
               "location": {"latitude": latitude, "longitude": longitude}})
    };
    let extra = json!({"blocks": [{"type": "places", "places": [
        at(90.0, 180.0), at(-90.0, -180.0),
        at(90.000001, 0.0), at(-90.000001, 0.0), at(0.0, 180.000001), at(0.0, -180.000001),
        at(1e300, 0.0), at(0.0, 1e300)
    ]}]});
    let ServerEvent::Row(row) = decode(row(extra)) else {
        panic!("a row");
    };
    let [Block::Places(places)] = row.blocks.as_slice() else {
        panic!("the places block");
    };
    let names: Vec<&str> = places.iter().map(|place| place.name.as_str()).collect();
    assert_eq!(
        names,
        ["90,180", "-90,-180"],
        "the bounds themselves are on the globe"
    );
    assert_eq!(
        row.skipped, 6,
        "each place just past a bound, and 1e300 either way"
    );
}

#[test]
fn a_rating_count_written_as_a_float_keeps_its_place() {
    let extra = json!({"blocks": [{"type": "places", "places": [
        {"name": "Here", "location": {"latitude": 1.0, "longitude": 2.0},
         "rating": {"value": 4.5, "count": 321.0}}
    ]}]});
    let ServerEvent::Row(row) = decode(row(extra)) else {
        panic!("a row");
    };
    let [Block::Places(places)] = row.blocks.as_slice() else {
        panic!("the places block");
    };
    assert_eq!(places[0].rating.as_ref().and_then(|r| r.count), Some(321.0));
}

/// A list a `checklists` block names, as the engine writes it.
fn list_ref(list_id: &str, title: &str, rows: &[&str]) -> Value {
    json!({"list_id": list_id, "title": title, "rows": rows})
}

/// The lists of the one `checklists` block among `blocks`.
fn named_lists(blocks: &[Block]) -> &[ListRef] {
    let [Block::Checklists(lists)] = blocks else {
        panic!("one checklists block: {blocks:?}");
    };
    lists
}

/// A reply's `checklists` block reads the same from its `text_done`, its `row` and its history
/// message (PROTOCOL.md "Blocks under a reply"), beside a `places` block.
#[test]
fn a_checklists_block_decodes_from_text_done_a_row_and_a_history_message() {
    let blocks = json!([
        {"type": "places", "places": [
            {"name": "Here", "location": {"latitude": 1.0, "longitude": 2.0}}
        ]},
        {"type": "checklists", "lists": [
            list_ref("ck_9t3k6w0p5h", "Fermix", &["ci_0a1b2c3d4e", "ci_5f6g7h8i9j"]),
            list_ref("ck_4q8m2x7v1d", "Groceries", &[])
        ]}
    ]);
    let wanted = [
        ListRef {
            list_id: "ck_9t3k6w0p5h".into(),
            title: "Fermix".into(),
            rows: vec!["ci_0a1b2c3d4e".into(), "ci_5f6g7h8i9j".into()],
        },
        ListRef {
            list_id: "ck_4q8m2x7v1d".into(),
            title: "Groceries".into(),
            rows: Vec::new(),
        },
    ];
    let done = json!({"type": "text_done", "turn_id": "t", "server_seq": 2, "text": "x",
        "blocks": blocks});
    let ServerEvent::TextDone(reply) = decode(done) else {
        panic!("a reply");
    };
    assert_eq!(named_lists(&reply.blocks[1..]), wanted);
    assert_eq!(reply.skipped, 0);
    let ServerEvent::Row(row) = decode(row(json!({"blocks": blocks}))) else {
        panic!("a row");
    };
    assert_eq!(named_lists(&row.blocks[1..]), wanted);
    let page = json!({"type": "history_page", "profile_id": "main", "history_head_seq": 5,
        "messages": [{"server_seq": 5, "role": "assistant", "content": "x", "ts": "t",
            "media_refs": [], "blocks": blocks}]});
    let ServerEvent::HistoryPage(page) = decode(page) else {
        panic!("a page");
    };
    assert_eq!(named_lists(&page.messages[0].blocks[1..]), wanted);
}

/// A list a block names off the contract is skipped and counted, alone: ids not the engine's
/// (`ck_` or `ci_` and ten lowercase letters or digits), a blank or overlong title, or a field
/// missing, its rows included. Past `MAX_BLOCK_LISTS`, the rest are skipped too.
#[test]
fn a_list_off_the_contract_is_skipped_and_counted_and_a_block_names_at_most_eight() {
    let good = |n: usize| list_ref(&format!("ck_{n:010}"), "List", &["ci_0a1b2c3d4e"]);
    let long_title = "x".repeat(MAX_LIST_TITLE + 1);
    let full_title = "é".repeat(MAX_LIST_TITLE);
    let off = [
        list_ref("CK_9t3k6w0p5h", "Upper prefix", &[]),
        list_ref("ck_9T3K6W0P5H", "Upper id", &[]),
        list_ref("ck_9t3k6w0p5", "Nine", &[]),
        list_ref("ck_9t3k6w0p5h0", "Eleven", &[]),
        list_ref("ci_9t3k6w0p5h", "A row's prefix", &[]),
        list_ref("ck_9t3k6w0p5h", "A list's id as a row", &["ck_0a1b2c3d4e"]),
        list_ref("ck_9t3k6w0p5h", "A row with a dash", &["ci_0a1b2-3d4e"]),
        list_ref("ck_9t3k6w0p5h", "", &[]),
        list_ref("ck_9t3k6w0p5h", &long_title, &[]),
        json!({"list_id": "ck_9t3k6w0p5h", "rows": []}),
        json!({"title": "No id", "rows": []}),
        json!({"list_id": "ck_9t3k6w0p5h", "title": "No rows"}),
    ];
    let mut lists: Vec<Value> = off.to_vec();
    lists.push(list_ref("ck_9t3k6w0p5h", &full_title, &[]));
    lists.extend((0..MAX_BLOCK_LISTS).map(good));
    let ServerEvent::Row(row) = decode(row(json!({"blocks": [
        {"type": "checklists", "lists": lists}
    ]}))) else {
        panic!("a row");
    };
    let kept = named_lists(&row.blocks);
    assert_eq!(kept.len(), MAX_BLOCK_LISTS);
    assert_eq!(
        kept[0].title, full_title,
        "80 characters is a title, whatever its bytes"
    );
    assert_eq!(kept[MAX_BLOCK_LISTS - 1].list_id, "ck_0000000006");
    assert_eq!(
        row.skipped,
        off.len() + 1,
        "each list off the contract, and the ninth"
    );
}

/// A list naming more rows than a list holds keeps its first `MAX_LIST_ROWS` and is read: the
/// contract bounds a list's rows, not a block's, and a turn that removes many rows and adds many
/// can name more.
#[test]
fn a_list_naming_more_rows_than_a_list_holds_keeps_the_first_500() {
    let rows: Vec<String> = (0..=MAX_LIST_ROWS).map(|n| format!("ci_{n:010}")).collect();
    let named: Vec<&str> = rows.iter().map(String::as_str).collect();
    let ServerEvent::Row(row) = decode(row(json!({"blocks": [
        {"type": "checklists", "lists": [list_ref("ck_9t3k6w0p5h", "Many rows", &named)]}
    ]}))) else {
        panic!("a row");
    };
    let [list] = named_lists(&row.blocks) else {
        panic!("the list read");
    };
    assert_eq!(list.rows, rows[..MAX_LIST_ROWS]);
    assert_eq!(row.skipped, 0);
}

/// A `checklists` block without its array is a block this build cannot read: skipped and
/// counted, as an unknown one is.
#[test]
fn a_checklists_block_without_its_lists_is_skipped_and_counted() {
    let ServerEvent::Row(row) = decode(row(json!({"blocks": [
        {"type": "checklists"},
        {"type": "checklists", "lists": "Groceries"}
    ]}))) else {
        panic!("a row");
    };
    assert!(row.blocks.is_empty());
    assert_eq!(row.skipped, 2);
}

#[test]
fn a_reply_counts_the_blocks_it_dropped() {
    let done = json!({"type": "text_done", "turn_id": "t", "server_seq": 2, "text": "x",
        "blocks": [{"type": "weather"}]});
    let ServerEvent::TextDone(reply) = decode(done) else {
        panic!("a reply");
    };
    assert!(reply.blocks.is_empty());
    assert_eq!(reply.skipped, 1);
}

#[test]
fn a_page_with_one_malformed_message_keeps_the_rest_and_counts_it() {
    let page = json!({
        "type": "history_page", "profile_id": "main", "history_head_seq": 9,
        "next_after_seq": 9,
        "messages": [
            {"server_seq": 8, "role": "user", "content": "a", "ts": "t", "media_refs": []},
            {"server_seq": "nine"},
            {"server_seq": 9, "role": "assistant", "content": "b", "ts": "t", "media_refs": []}
        ]
    });
    let ServerEvent::HistoryPage(page) = decode(page) else {
        panic!("a page");
    };
    assert_eq!(page.skipped, 1);
    let seqs: Vec<u64> = page.messages.iter().map(|m| m.server_seq).collect();
    assert_eq!(seqs, [8, 9]);
}

#[test]
fn an_approval_never_prints_its_token_or_routes() {
    let approval = json!({
        "type": "approval", "approval_id": "a1", "kind": "sandbox", "text": "Allow?",
        "token": "secret-token", "ttl_s": 60,
        "approve_command": "/confirm secret-token", "deny_command": "/deny secret-token"
    });
    let printed = format!("{:?}", decode(approval));
    assert!(!printed.contains("secret-token"), "{printed}");
    assert!(printed.contains("a1"));
}

#[test]
fn a_frame_that_cannot_mean_anything_is_refused() {
    assert!(matches!(read_line(b"nope"), Err(DecodeError::NotJson(_))));
    assert!(matches!(read_line(b"[1]"), Err(DecodeError::NotAnObject)));
    assert!(matches!(read_line(b"{}"), Err(DecodeError::MissingType)));
    assert_eq!(
        read_line(br#"{"type":"accepted"}"#),
        Ok(ServerEvent::Unreadable(Unreadable {
            kind: "accepted".into(),
            ..Unreadable::default()
        })),
        "a known event that does not fit costs only itself"
    );
    assert_eq!(
        read_line(br#"{"type":"later_event"}"#).unwrap(),
        ServerEvent::Unknown("later_event".into())
    );
}

#[test]
fn a_media_ref_is_64_lowercase_hex() {
    assert!(is_media_ref(&"0a".repeat(32)));
    assert!(!is_media_ref(&"0A".repeat(32)));
    assert!(!is_media_ref(&"a".repeat(65)));
    assert!(!is_media_ref(&"g".repeat(64)));
}

#[test]
fn the_line_buffer_takes_a_page_past_the_voice_caps_and_stops_a_runaway() {
    let long = vec![b'a'; 3 * 1024 * 1024];
    let mut voice = LineBuffer::new();
    assert!(voice.push(&long).is_err(), "the voice wire caps at 1 MiB");
    let mut chat = line_buffer();
    let mut bytes = long.clone();
    bytes.push(b'\n');
    assert_eq!(chat.push(&bytes).unwrap().len(), 1);
    let mut runaway = line_buffer();
    let over = vec![b'a'; MAX_FRAME_BYTES + 1];
    assert!(matches!(
        runaway.push(&over),
        Err(DecodeError::FrameTooLarge(_))
    ));
}

/// JSON may carry a NUL (`\u0000`), a GTK label cannot: GLib panics on one in a debug build and
/// cuts the text at it in a release one. Every string the daemon sends arrives with each NUL
/// made U+FFFD, so no label downstream ever sees one.
#[test]
fn a_nul_in_any_text_the_daemon_sends_arrives_as_a_replacement_character() {
    let ServerEvent::Row(held) = decode(row(json!({"text": "before\u{0}after"}))) else {
        panic!("a row");
    };
    assert_eq!(held.text, "before\u{FFFD}after");
    let delta = json!({"type": "text_delta", "turn_id": "t\u{0}1", "text": "so\u{0}far"});
    let ServerEvent::TextDelta { turn_id, text } = decode(delta) else {
        panic!("a delta");
    };
    assert_eq!(
        (turn_id.as_str(), text.as_str()),
        ("t\u{FFFD}1", "so\u{FFFD}far")
    );
    let page = json!({"type": "history_page", "profile_id": "main", "history_head_seq": 1,
        "messages": [{"server_seq": 1, "role": "user", "content": "\u{0}", "media_refs": []}]});
    let ServerEvent::HistoryPage(page) = decode(page) else {
        panic!("a page");
    };
    assert_eq!(page.messages[0].text, "\u{FFFD}");
}

/// A NUL inside a fence, in its tag and in its code, reaches the code block replaced too.
#[test]
fn a_nul_in_a_fence_reaches_the_code_block_replaced() {
    use fermix_client::markdown::{highlight, render, Block as Drawn};
    let fence = "```py\u{0}\nx = 1\u{0}\n```";
    let ServerEvent::Row(held) = decode(row(json!({"text": fence}))) else {
        panic!("a row");
    };
    let blocks = render(&held.text);
    let [Drawn::Code { language, text }] = blocks.as_slice() else {
        panic!("one code block");
    };
    assert_eq!(language.as_deref(), Some("py\u{FFFD}"));
    assert_eq!(text, "x = 1\u{FFFD}");
    let inks = highlight::Inks {
        keyword: "#aa00aa".into(),
        string: "#00aa00".into(),
        number: "#aa5500".into(),
    };
    assert!(!highlight::markup(text, language.as_deref(), &inks).contains('\0'));
}

/// An event's type for a log line is the contract's word for it, never the daemon's text: a type
/// this build does not know is "unknown", and one whose fields did not fit keeps its known type.
#[test]
fn an_events_type_for_the_log_is_the_contracts_word() {
    let unknown = read_line(br#"{"type":"a password: hunter2"}"#).unwrap();
    assert_eq!(unknown.kind(), "unknown");
    let unreadable = read_line(br#"{"type":"accepted","client_msg_id":5}"#).unwrap();
    assert!(matches!(unreadable, ServerEvent::Unreadable(_)));
    assert_eq!(unreadable.kind(), "accepted");
    let done = decode(json!({"type": "turn_done", "turn_id": "t1"}));
    assert_eq!(done.kind(), "turn_done");
}

/// An id or a code word the daemon names goes into a log line only when it is in the contract's
/// id alphabet and length (the schema's `attach_id`, `^[0-9A-Za-z_-]{1,64}$`): anything else is
/// said without its text.
#[test]
fn only_a_short_plain_id_is_logged_as_it_is() {
    let longest = "a".repeat(MAX_LOGGED_BYTES);
    let media_ref = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";
    let plain = [
        "mac-att-9d04e2",
        "turn-linux-0a1b2c3d4e5f6071",
        "client_too_new",
        media_ref,
        longest.as_str(),
    ];
    for id in plain {
        assert_eq!(loggable_id(id), id);
    }
    let long = "a".repeat(MAX_LOGGED_BYTES + 1);
    let odd = [
        long.as_str(),
        "two words",
        "line\nbreak",
        "",
        "caf\u{e9}",
        "token=sk-live-1",
        "/home/me/tax.pdf",
        "a:b",
        "{x}",
        "\"q\"",
    ];
    for id in odd {
        assert_eq!(loggable_id(id), NOT_LOGGED, "{id:?}");
    }
}
