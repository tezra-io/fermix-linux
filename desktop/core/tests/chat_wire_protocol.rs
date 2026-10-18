//! The chat wire's codec beyond the golden lines: what it is lenient about inside a row, what it
//! refuses, and the framing caps.

use fermix_client::chat_wire::protocol::{
    decode_line, is_media_ref, line_buffer, Block, ClientEvent, DecodeError, ServerEvent,
    MAX_FRAME_BYTES, MAX_LINE_BYTES,
};
use fermix_client::realtime::protocol::LineBuffer;
use serde_json::{json, Value};

fn decode(value: Value) -> ServerEvent {
    decode_line(value.to_string().as_bytes()).expect("decodes")
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
    assert!(matches!(decode_line(b"nope"), Err(DecodeError::NotJson(_))));
    assert!(matches!(decode_line(b"[1]"), Err(DecodeError::NotAnObject)));
    assert!(matches!(decode_line(b"{}"), Err(DecodeError::MissingType)));
    assert!(matches!(
        decode_line(br#"{"type":"accepted"}"#),
        Err(DecodeError::BadEvent { .. })
    ));
    assert_eq!(
        decode_line(br#"{"type":"later_event"}"#).unwrap(),
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
