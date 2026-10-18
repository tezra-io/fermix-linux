//! The Chat page's entries, read off the chat wire's timeline: held rows, questions still
//! sending, the running turn with its tools, and the last failure.

use fermix_client::chat::{media_line, view, Entry, Key, RunState};
use fermix_client::chat_wire::protocol::{decode_line, MediaRef, ServerEvent};
use fermix_client::chat_wire::timeline::{self, Chat};
use fermix_client::places::Card;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

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

/// A reply with no places.
fn reply(text: &str) -> Entry {
    Entry::Assistant {
        text: text.into(),
        places: Vec::new(),
    }
}

/// The places a reply's entry carries, by name.
fn place_names(entry: &Entry) -> Vec<&str> {
    let Entry::Assistant { places, .. } = entry else {
        panic!("a reply: {entry:?}");
    };
    places
        .iter()
        .map(|card: &Card| card.name.as_str())
        .collect()
}

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");

/// The vendored places block of the reply to `turn`, as JSON.
fn fixture_blocks(turn: &str) -> Value {
    for line in SERVER.lines() {
        let event: Value = serde_json::from_str(line).expect("a fixture line is JSON");
        if event["type"] == "text_done" && event["turn_id"] == turn {
            return event["blocks"].clone();
        }
    }
    panic!("no text_done for {turn} in the fixtures");
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
        [Entry::User("Hi".into()), reply("Hello **there**")]
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
    assert_eq!(shown.entries[1], reply("Here"));
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

fn receive(chat: &mut Chat, value: Value) {
    timeline::receive(chat, event(value), 0);
}

fn page_of(seqs: impl IntoIterator<Item = u64>, head: u64, older: Option<u64>) -> Value {
    let messages: Vec<Value> = seqs
        .into_iter()
        .map(|seq| {
            json!({"server_seq": seq, "role": "user", "content": format!("Message {seq}"),
            "kind": "text", "ts": "2026-09-25T09:00:00Z", "media_refs": []})
        })
        .collect();
    let mut page = json!({"type": "history_page", "profile_id": "main", "messages": messages,
        "history_head_seq": head});
    if let Some(before) = older {
        page["next_before_seq"] = json!(before);
    }
    page
}

/// The page keeps each entry's widget by its key, and reads which held row each entry draws.
#[test]
fn every_entry_has_a_key_of_its_own_and_names_the_row_it_draws() {
    let mut chat = connected();
    let voice = json!({"client_msg_id": "linux-1", "media_refs": [
        {"ref": "a", "kind": "audio", "mime": "audio/ogg", "size_bytes": 1}]});
    row(&mut chat, 1, "user", "Hi", voice);
    row(&mut chat, 2, "assistant", "Hello", json!({}));
    timeline::send(&mut chat, "And?", "linux-2".into());
    receive(
        &mut chat,
        json!({"type": "turn_error", "turn_id": "t0", "code": "interrupted", "message": "x"}),
    );
    receive(
        &mut chat,
        json!({"type": "turn_started", "profile_id": "main", "turn_id": "t1",
        "in_reply_to": "linux-2"}),
    );
    receive(
        &mut chat,
        json!({"type": "tool_event", "turn_id": "t1", "tool": "web_search", "phase": "start"}),
    );
    receive(
        &mut chat,
        json!({"type": "text_delta", "turn_id": "t1", "text": "So"}),
    );
    let shown = view(&chat);
    assert_eq!(
        shown.keys,
        [
            Key::Row(1),
            Key::Media(1, 0),
            Key::Row(2),
            Key::Sending("linux-2".into()),
            Key::Tools("t1".into()),
            Key::Reply("t1".into(), 0),
            Key::Failure,
        ]
    );
    assert_eq!(shown.keys.len(), shown.entries.len());
    assert_eq!(
        shown.seqs,
        [Some(1), Some(1), Some(2), None, None, None, None]
    );
}

/// The streamed reply and the row it becomes are one widget, also after the row is read again.
#[test]
fn a_reply_keeps_its_key_from_its_first_words_to_the_row_it_becomes() {
    let mut chat = connected();
    row(
        &mut chat,
        1,
        "user",
        "Hi",
        json!({"client_msg_id": "linux-1"}),
    );
    receive(
        &mut chat,
        json!({"type": "turn_started", "profile_id": "main", "turn_id": "t1",
        "in_reply_to": "linux-1"}),
    );
    receive(
        &mut chat,
        json!({"type": "text_delta", "turn_id": "t1", "text": "Hel"}),
    );
    let streaming = view(&chat);
    assert_eq!(streaming.keys, [Key::Row(1), Key::Reply("t1".into(), 0)]);
    assert_eq!(streaming.seqs, [Some(1), None]);
    receive(
        &mut chat,
        json!({"type": "text_done", "turn_id": "t1", "server_seq": 2, "text": "Hello"}),
    );
    let held = view(&chat);
    assert_eq!(held.keys, streaming.keys);
    assert_eq!(held.seqs, [Some(1), Some(2)]);
    // Away and back: the row on screen is read again and keeps its key.
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::from([2]));
    receive(&mut chat, page_of([], 2, None));
    let mut reread = page_of([2], 2, None);
    reread["messages"][0]["role"] = json!("assistant");
    reread["messages"][0]["content"] = json!("Hello again");
    receive(&mut chat, reread);
    let again = view(&chat);
    assert_eq!(again.entries[1], reply("Hello again"));
    assert_eq!(again.keys, streaming.keys);
}

/// A page read backward lands above: every entry already drawn keeps its key and its order.
#[test]
fn a_page_read_backward_adds_keys_above_and_keeps_every_key_below() {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(&mut chat, page_of(51..=100, 100, Some(51)));
    let before = view(&chat).keys;
    assert_eq!(before.first(), Some(&Key::Row(51)));
    assert!(!timeline::pull_older(&mut chat).is_empty());
    receive(&mut chat, page_of(1..=50, 100, None));
    let after = view(&chat).keys;
    assert_eq!(after.len(), 100);
    assert_eq!(after.first(), Some(&Key::Row(1)));
    assert!(after.ends_with(&before));
}

/// A turn may send `text_done` more than once (PROTOCOL.md): each of its replies keeps a key of
/// its own, and the next one streaming takes the key its row will have.
#[test]
fn every_reply_of_a_turn_has_a_key_of_its_own() {
    let unique = |keys: &[Key]| keys.iter().collect::<HashSet<_>>().len() == keys.len();
    let mut chat = connected();
    row(
        &mut chat,
        1,
        "user",
        "Hi",
        json!({"client_msg_id": "linux-1"}),
    );
    receive(
        &mut chat,
        json!({"type": "turn_started", "profile_id": "main", "turn_id": "t1",
        "in_reply_to": "linux-1"}),
    );
    receive(
        &mut chat,
        json!({"type": "text_done", "turn_id": "t1", "server_seq": 2, "text": "First"}),
    );
    receive(
        &mut chat,
        json!({"type": "text_delta", "turn_id": "t1", "text": "Sec"}),
    );
    let streaming = view(&chat);
    assert!(unique(&streaming.keys), "{:?}", streaming.keys);
    assert_eq!(
        streaming.keys,
        [
            Key::Row(1),
            Key::Reply("t1".into(), 0),
            Key::Reply("t1".into(), 1)
        ]
    );
    receive(
        &mut chat,
        json!({"type": "text_done", "turn_id": "t1", "server_seq": 3, "text": "Second"}),
    );
    let held = view(&chat);
    assert!(unique(&held.keys), "{:?}", held.keys);
    assert_eq!(held.keys, streaming.keys, "the draft's key is its row's");
}

/// A reply's places come with its row: none while it streams, its cards from `text_done` on, in
/// the same entry under the same key, so the reply grows its cards in place.
#[test]
fn a_streamed_reply_gains_its_places_at_text_done_under_its_key() {
    let mut chat = connected();
    row(
        &mut chat,
        1,
        "user",
        "Coffee?",
        json!({"client_msg_id": "linux-1"}),
    );
    receive(
        &mut chat,
        json!({"type": "turn_started", "profile_id": "main", "turn_id": "t1",
        "in_reply_to": "linux-1"}),
    );
    receive(
        &mut chat,
        json!({"type": "text_delta", "turn_id": "t1", "text": "Two quiet ones"}),
    );
    let streaming = view(&chat);
    assert_eq!(streaming.entries[1], reply("Two quiet ones"));
    receive(
        &mut chat,
        json!({"type": "text_done", "turn_id": "t1", "server_seq": 2,
        "text": "Two quiet ones", "blocks": fixture_blocks("turn-mac-6")}),
    );
    let done = view(&chat);
    assert_eq!(done.keys, streaming.keys);
    assert_eq!(
        place_names(&done.entries[1]),
        ["Example Coffee", "Quiet Grounds"]
    );
}

#[test]
fn a_row_read_from_history_or_read_again_carries_its_places() {
    let mut chat = connected();
    row(
        &mut chat,
        1,
        "assistant",
        "Two in Suwanee",
        json!({"blocks": fixture_blocks("turn-mac-7")}),
    );
    let first = view(&chat);
    assert_eq!(
        place_names(&first.entries[0]),
        ["Example Taqueria", "Second Cantina"]
    );
    // Away and back: the row read again with other places changes under the same key.
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::from([1]));
    receive(&mut chat, page_of([], 1, None));
    let mut reread = page_of([1], 1, None);
    reread["messages"][0]["role"] = json!("assistant");
    reread["messages"][0]["content"] = json!("Two in Suwanee");
    reread["messages"][0]["blocks"] = fixture_blocks("turn-mac-6");
    receive(&mut chat, reread);
    let again = view(&chat);
    assert_eq!(again.keys, first.keys);
    assert_eq!(
        place_names(&again.entries[0]),
        ["Example Coffee", "Quiet Grounds"]
    );
}

#[test]
fn a_person_s_row_never_draws_places() {
    let mut chat = connected();
    row(
        &mut chat,
        1,
        "user",
        "Here",
        json!({"blocks": fixture_blocks("turn-mac-6")}),
    );
    assert_eq!(view(&chat).entries, [Entry::User("Here".into())]);
}
