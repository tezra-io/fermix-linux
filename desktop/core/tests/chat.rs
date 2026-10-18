//! The Chat page's entries, read off the chat wire's timeline: held rows, questions still
//! sending, the running turn with its tools, and the last failure.

use fermix_client::chat::{media_line, view, Entry, RunState};
use fermix_client::chat_wire::protocol::{decode_line, MediaRef, ServerEvent};
use fermix_client::chat_wire::timeline::{self, Chat};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn event(value: Value) -> ServerEvent {
    decode_line(value.to_string().as_bytes()).expect("a test event decodes")
}

fn connected() -> Chat {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    timeline::receive(
        &mut chat,
        event(
            json!({"type": "history_page", "profile_id": "main", "history_head_seq": 0,
            "messages": []}),
        ),
        0,
    );
    chat
}

fn row(chat: &mut Chat, seq: u64, role: &str, text: &str, extra: Value) {
    let mut row = json!({"type": "row", "profile_id": "main", "server_seq": seq, "role": role,
        "text": text, "ts": "2026-09-25T09:00:00Z", "media_refs": []});
    for (key, value) in extra.as_object().unwrap() {
        row[key] = value.clone();
    }
    timeline::receive(chat, event(row), 0);
}

#[test]
fn an_empty_chat_has_no_entries_and_no_turn() {
    let shown = view(&Chat::default());
    assert!(shown.entries.is_empty());
    assert!(shown.turn.is_none());
    assert!(!shown.thinking);
}

#[test]
fn rows_read_as_the_person_and_fermix_with_their_times() {
    let mut chat = connected();
    row(&mut chat, 1, "user", "Hi", json!({}));
    row(&mut chat, 2, "assistant", "Hello **there**", json!({}));
    let shown = view(&chat);
    assert_eq!(
        shown.entries,
        [
            Entry::User("Hi".into()),
            Entry::Assistant("Hello **there**".into())
        ]
    );
    assert_eq!(shown.times[0].as_deref(), Some("2026-09-25T09:00:00Z"));
    assert_eq!(shown.first_seq, Some(1));
}

#[test]
fn a_question_shows_as_sending_until_its_row_arrives() {
    let mut chat = connected();
    timeline::send(&mut chat, "What is on today?", "linux-1".into());
    assert_eq!(
        view(&chat).entries,
        [Entry::Sending("What is on today?".into())]
    );
    row(
        &mut chat,
        1,
        "user",
        "What is on today?",
        json!({"client_msg_id": "linux-1"}),
    );
    assert_eq!(
        view(&chat).entries,
        [Entry::User("What is on today?".into())]
    );
}

#[test]
fn a_turn_thinks_then_shows_its_tools_and_its_draft() {
    let mut chat = connected();
    timeline::receive(
        &mut chat,
        event(json!({"type": "turn_started", "profile_id": "main",
        "turn_id": "t1", "in_reply_to": "linux-1"})),
        100,
    );
    let shown = view(&chat);
    assert!(shown.thinking);
    let turn = shown.turn.expect("a live turn");
    assert_eq!(turn.started, 100);
    assert!(turn.can_stop);
    timeline::receive(
        &mut chat,
        event(json!({"type": "tool_event", "turn_id": "t1",
        "tool": "web_search", "phase": "start"})),
        101,
    );
    timeline::receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t1",
        "text": "Here"})),
        102,
    );
    let shown = view(&chat);
    assert!(!shown.thinking);
    let Entry::Tools(runs) = &shown.entries[0] else {
        panic!("the tools first");
    };
    assert_eq!(runs[0].name, "web_search");
    assert_eq!(runs[0].state, RunState::Running);
    assert_eq!(runs[0].started, 101);
    assert_eq!(shown.entries[1], Entry::Assistant("Here".into()));
}

#[test]
fn stop_marks_the_turn_stopping() {
    let mut chat = connected();
    timeline::receive(
        &mut chat,
        event(json!({"type": "turn_started", "profile_id": "main",
        "turn_id": "t1", "in_reply_to": "linux-1"})),
        0,
    );
    timeline::stop(&mut chat);
    assert!(view(&chat).turn.unwrap().stopping);
}

#[test]
fn a_joined_turn_cannot_be_stopped() {
    let mut chat = connected();
    timeline::receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t9",
        "text": "so far"})),
        0,
    );
    assert!(!view(&chat).turn.unwrap().can_stop);
}

#[test]
fn the_last_failure_comes_last() {
    let mut chat = connected();
    timeline::receive(
        &mut chat,
        event(json!({"type": "turn_error", "turn_id": "t1",
        "code": "interrupted", "message": "lost"})),
        0,
    );
    assert_eq!(
        view(&chat).entries,
        [Entry::Failure("The reply was interrupted".into())]
    );
}

#[test]
fn a_row_of_files_names_each_file_in_the_macs_words() {
    let mut chat = connected();
    let media = json!({"media_refs": [
        {"ref": "a", "kind": "audio", "mime": "audio/ogg", "size_bytes": 1},
        {"ref": "b", "kind": "image", "mime": "image/jpeg", "size_bytes": 1, "filename": "lamp.jpg"}
    ]});
    row(&mut chat, 1, "user", "", media);
    assert_eq!(
        view(&chat).entries,
        [
            Entry::Media {
                line: "Voice note".into(),
                person: true,
            },
            Entry::Media {
                line: "Picture · lamp.jpg".into(),
                person: true,
            }
        ]
    );
}

#[test]
fn every_file_kind_has_a_word() {
    let media = |kind: &str, mime: &str| MediaRef {
        r#ref: "r".into(),
        kind: kind.into(),
        mime: mime.into(),
        size_bytes: 1,
        sha256: None,
        filename: None,
        caption: None,
    };
    assert_eq!(media_line(&media("video", "video/mp4")), "Video");
    assert_eq!(media_line(&media("audio", "audio/mp4")), "Sound");
    assert_eq!(media_line(&media("file", "application/pdf")), "PDF");
    assert_eq!(media_line(&media("file", "text/plain")), "File");
}

#[test]
fn a_picture_is_drawn_no_wider_than_fits_its_box_and_never_enlarged() {
    use fermix_client::chat::picture_width;
    assert_eq!(picture_width(800, 400, 440), 440, "wide: the box's width");
    assert_eq!(picture_width(400, 800, 440), 220, "tall: the box's height");
    assert_eq!(picture_width(100, 50, 440), 100, "small: its own size");
    assert_eq!(picture_width(1, 10_000, 440), 1, "never nothing");
}

#[test]
#[should_panic(expected = "has no size")]
fn a_picture_without_a_size_is_a_bug() {
    fermix_client::chat::picture_width(0, 10, 440);
}

#[test]
fn a_saved_picture_is_named_for_its_type() {
    use fermix_client::chat::picture_file_name;
    assert_eq!(picture_file_name("image/png"), "Fermix image.png");
    assert_eq!(picture_file_name("image/jpeg"), "Fermix image.jpg");
    assert_eq!(picture_file_name("image/svg+xml"), "Fermix image.svg");
    assert_eq!(picture_file_name("image/webp"), "Fermix image.webp");
    assert_eq!(
        picture_file_name("image/../../x"),
        "Fermix image",
        "a type that is not a plain word names no extension"
    );
}
