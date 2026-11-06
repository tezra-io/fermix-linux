//! The Chat page's entries, read off the chat wire's timeline: held rows, questions still
//! sending, the running turn with its tools, and the last failure.

use fermix_client::chat::{view, Entry, Key, ListsCard, RunState};
use fermix_client::chat_wire::protocol::{read_line, ServerEvent};
use fermix_client::chat_wire::timeline::{self, Chat};
use fermix_client::media::{Strip, TileKind};
use fermix_client::places::Card;
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashSet};

fn event(value: Value) -> ServerEvent {
    let event = read_line(value.to_string().as_bytes()).expect("a test event decodes");
    assert!(
        !matches!(event, ServerEvent::Unreadable(_)),
        "a test event reads: {event:?}"
    );
    event
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

/// A reply with no places and nothing under it.
fn reply(text: &str) -> Entry {
    Entry::Assistant {
        text: text.into(),
        places: Vec::new(),
        media: Strip::default(),
        lists: None,
    }
}

/// The person's words with nothing under them.
fn said(text: &str) -> Entry {
    Entry::User {
        text: text.into(),
        media: Strip::default(),
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
    assert_eq!(shown.entries, [said("Hi"), reply("Hello **there**")]);
    assert_eq!(shown.times[0].as_deref(), Some("2026-09-25T09:00:00Z"));
    assert_eq!(shown.first_seq, Some(1));
}

/// A time this app cannot read is drawn as no time: the page never tries, and logs, the daemon's
/// text again at each draw. The row itself is drawn.
#[test]
fn a_row_whose_time_cannot_be_read_is_drawn_with_none() {
    let mut chat = connected();
    row(
        &mut chat,
        1,
        "assistant",
        "Hello",
        json!({"ts": "tomorrow-ish"}),
    );
    row(&mut chat, 2, "assistant", "Again", json!({}));
    let shown = view(&chat);
    assert_eq!(shown.entries, [reply("Hello"), reply("Again")]);
    assert_eq!(shown.times[0], None);
    assert_eq!(shown.times[1].as_deref(), Some("2026-09-25T09:00:00Z"));
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
    assert_eq!(view(&chat).entries, [said("What is on today?")]);
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

/// A row of files is one entry: its strip under no words, keyed by its row.
#[test]
fn a_row_of_files_is_one_entry_with_its_strip() {
    let mut chat = connected();
    let media = json!({"media_refs": [
        {"ref": "a", "kind": "audio", "mime": "audio/ogg", "size_bytes": 1},
        {"ref": "b", "kind": "image", "mime": "image/jpeg", "size_bytes": 1, "filename": "lamp.jpg"}
    ]});
    row(&mut chat, 1, "user", "", media.clone());
    row(&mut chat, 2, "assistant", "lamp.jpg", media);
    let shown = view(&chat);
    assert_eq!(shown.keys, [Key::Row(1), Key::Row(2)]);
    let Entry::User { text, media } = &shown.entries[0] else {
        panic!("the person's row: {:?}", shown.entries[0]);
    };
    assert_eq!(text, "");
    assert_eq!(
        media.voice.as_ref().map(|note| note.r#ref.as_str()),
        Some("a")
    );
    let tiles: Vec<(&str, TileKind)> = media
        .tiles
        .iter()
        .map(|tile| (tile.name.as_str(), tile.kind))
        .collect();
    assert_eq!(tiles, [("lamp.jpg", TileKind::Picture)]);
    let Entry::Assistant { text, media, .. } = &shown.entries[1] else {
        panic!("Fermix's row: {:?}", shown.entries[1]);
    };
    assert_eq!(text, "", "the line naming the tile went to the tile");
    assert_eq!(media.tiles.len(), 1);
}

/// A row with nothing to say and nothing to show draws nothing.
#[test]
fn a_row_with_no_words_and_no_files_draws_nothing() {
    let mut chat = connected();
    row(&mut chat, 1, "user", "  ", json!({}));
    assert!(view(&chat).entries.is_empty());
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
            Key::Row(2),
            Key::Sending("linux-2".into()),
            Key::Tools("t1".into()),
            Key::Reply("t1".into(), 0),
            Key::Failure,
        ]
    );
    assert_eq!(shown.keys.len(), shown.entries.len());
    assert_eq!(shown.seqs, [Some(1), Some(2), None, None, None, None]);
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

/// A `checklists` block naming `lists`, each by id and title, with no rows.
fn lists_block(lists: &[&str]) -> Value {
    let lists: Vec<Value> = lists
        .iter()
        .map(|id| json!({"list_id": id, "title": format!("List {id}"), "rows": []}))
        .collect();
    json!({"blocks": [{"type": "checklists", "lists": lists}]})
}

/// Each reply's lists card, by its seq: the lists it names, and whether it is live.
fn lists_cards(chat: &Chat) -> Vec<(Option<u64>, Vec<String>, bool)> {
    let shown = view(chat);
    let cards = shown.entries.iter().zip(&shown.seqs);
    cards
        .filter_map(|(entry, seq)| match entry {
            Entry::Assistant {
                lists: Some(ListsCard { lists, live }),
                ..
            } => Some((
                *seq,
                lists.iter().map(|l| l.list_id.clone()).collect(),
                *live,
            )),
            _ => None,
        })
        .collect()
}

/// A reply whose turn read or changed lists carries their card, and only the newest card that
/// draws a list is live (M85 lists §2.1). A card of several lists counts only its first three, a
/// person's row never counts, and a reply that names no list has no card. A streamed draft has
/// none until its row.
#[test]
fn only_the_newest_card_drawing_a_list_is_live() {
    const A: &str = "ck_aaaaaaaaaa";
    const B: &str = "ck_bbbbbbbbbb";
    const C: &str = "ck_cccccccccc";
    const D: &str = "ck_dddddddddd";
    let mut chat = connected();
    row(&mut chat, 1, "assistant", "A", lists_block(&[A]));
    row(&mut chat, 2, "assistant", "B", lists_block(&[B]));
    row(
        &mut chat,
        3,
        "assistant",
        "Four",
        lists_block(&[C, A, B, D]),
    );
    row(&mut chat, 4, "assistant", "No lists", json!({}));
    row(&mut chat, 5, "user", "B, from me", lists_block(&[B]));
    row(&mut chat, 6, "assistant", "D", lists_block(&[D]));
    let ids = |lists: &[&str]| lists.iter().map(|id| id.to_string()).collect::<Vec<_>>();
    assert_eq!(
        lists_cards(&chat),
        [
            (Some(1), ids(&[A]), false),
            (Some(2), ids(&[B]), false),
            (Some(3), ids(&[C, A, B, D]), true),
            (Some(6), ids(&[D]), true),
        ],
        "the card of four draws C, A and B, which no newer reply draws; D is not drawn there"
    );
    let shown = view(&chat);
    assert_eq!(shown.entries[3], reply("No lists"));
    receive(
        &mut chat,
        json!({"type": "text_delta", "turn_id": "t7", "text": "Ticked it"}),
    );
    assert_eq!(view(&chat).entries.last(), Some(&reply("Ticked it")));
    receive(
        &mut chat,
        json!({"type": "text_done", "turn_id": "t7", "server_seq": 7, "text": "Ticked it",
        "blocks": lists_block(&[A])["blocks"]}),
    );
    let after: Vec<(Option<u64>, bool)> = lists_cards(&chat)
        .into_iter()
        .map(|(seq, _, live)| (seq, live))
        .collect();
    assert_eq!(
        after,
        [
            (Some(1), false),
            (Some(2), false),
            (Some(3), false),
            (Some(6), true),
            (Some(7), true)
        ],
        "the card of four folds: the newest card draws A, one of the lists it draws"
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
    assert_eq!(view(&chat).entries, [said("Here")]);
}
