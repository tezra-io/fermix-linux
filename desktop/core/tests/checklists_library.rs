//! The lists over a connection (M85 lists §5.2 to §5.4; PROTOCOL.md "Lists"): the index pulled at
//! each handshake, a list pulled when it is drawn, the engine's holding mirrored, edits answered
//! and sent again, creates named, refusals kept apart from the chat's, and every queue bounded.
//! A port of the macOS app's ChecklistLibraryTests, with this app's caps. Nothing here opens a
//! socket: the library returns what to send.

use fermix_client::chat_wire::protocol::{
    read_line, ClientEvent, List, ListEntry, ListOp, ListRow, ServerError, ServerEvent,
};
use fermix_client::checklists::draft;
use fermix_client::checklists::library::{Library, MAX_HELD, MAX_WAITING};
use fermix_client::checklists::words::TOO_MANY;
use fermix_client::checklists::ITEM;
use serde_json::json;
use std::collections::BTreeMap;

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");
const GROCERIES: &str = "ck_4q8m2x7v1d";
const PACKING: &str = "ck_2b7n5r8c3j";
const LEMONS: &str = "ci_0r2y8e6u3g";
const BASIL: &str = "ci_3h9d1v8r2k";
const GONE: &str = "That row is not in this list any more; pull the list again.";

fn golden(index: usize) -> ServerEvent {
    let line = SERVER
        .lines()
        .nth(index)
        .expect("the fixture has this line");
    read_line(line.as_bytes()).expect("a golden line decodes")
}

fn groceries(rev: u64) -> List {
    let ServerEvent::Checklist { list, .. } = golden(53) else {
        panic!("line 53 is the Groceries list");
    };
    List { rev, ..*list }
}

fn entry(list_id: &str, title: &str) -> ListEntry {
    ListEntry {
        list_id: list_id.into(),
        title: title.into(),
        folder: None,
        open: 0,
        done: 0,
        rev: 1,
    }
}

fn index_of(ids: &[&str]) -> Vec<ListEntry> {
    ids.iter().map(|id| entry(id, id)).collect()
}

fn ids(range: std::ops::RangeInclusive<usize>) -> Vec<String> {
    range.map(|n| format!("ck_{n:010}")).collect()
}

fn index_pull() -> ClientEvent {
    ClientEvent::ChecklistsPull {
        profile_id: "main".into(),
    }
}

fn pull(list_id: &str, removed: Option<bool>) -> ClientEvent {
    ClientEvent::ChecklistPull {
        profile_id: "main".into(),
        list_id: list_id.into(),
        removed,
    }
}

fn edit(client_msg_id: &str, list_id: Option<&str>, op: ListOp) -> ClientEvent {
    ClientEvent::ChecklistEdit {
        client_msg_id: client_msg_id.into(),
        profile_id: "main".into(),
        list_id: list_id.map(str::to_owned),
        op,
    }
}

fn pulls(sent: &[ClientEvent]) -> Vec<String> {
    sent.iter()
        .filter_map(|event| match event {
            ClientEvent::ChecklistPull { list_id, .. } => Some(list_id.clone()),
            _ => None,
        })
        .collect()
}

fn check(row_id: &str) -> ListOp {
    ListOp::Check {
        row_id: row_id.into(),
    }
}

fn add(text: &str) -> ListOp {
    ListOp::Add {
        text: text.into(),
        note: None,
        section_id: None,
        after_id: None,
    }
}

fn refusal(message: &str, client_msg_id: Option<&str>) -> ServerError {
    let mut line = json!({"type": "error", "reason": "request_failed", "message": message});
    if let Some(id) = client_msg_id {
        line["client_msg_id"] = json!(id);
    }
    match read_line(line.to_string().as_bytes()).expect("a refusal decodes") {
        ServerEvent::Error(error) => error,
        other => panic!("an error: {other:?}"),
    }
}

/// A library past a version-7 handshake, with what it sent.
fn connected() -> (Library, Vec<ClientEvent>) {
    let mut lists = Library::default();
    let sent = lists.negotiated(7);
    (lists, sent)
}

/// Connected, the index naming Groceries, and Groceries pulled and answered at rev 7.
fn holding_groceries() -> (Library, Vec<ClientEvent>) {
    let (mut lists, mut sent) = connected();
    sent.extend(lists.received_index(index_of(&[GROCERIES])));
    sent.extend(lists.draw(GROCERIES, false));
    lists.received_list(groceries(7), &BTreeMap::new());
    (lists, sent)
}

fn shown(lists: &Library, list_id: &str) -> List {
    lists.shown(list_id).expect("the list is drawn")
}

#[test]
fn a_daemon_that_serves_lists_is_asked_for_the_index_and_the_index_is_kept() {
    let (mut lists, sent) = connected();

    assert!(lists.index().served());
    assert_eq!(sent, [index_pull()]);
    let ServerEvent::Checklists { lists: golden } = golden(50) else {
        panic!("line 50 is the index");
    };
    assert!(lists.received_index(golden.clone()).is_empty());
    assert_eq!(lists.index().entries(), Some(golden.as_slice()));
    assert!(!lists.index().lacks(GROCERIES));
    assert!(lists.index().lacks(PACKING));
}

#[test]
fn nothing_is_sent_on_a_handshake_whose_version_does_not_bring_lists_and_edits_wait_for_one_that_does(
) {
    let mut lists = Library::default();
    assert!(lists.negotiated(5).is_empty());
    assert!(!lists.index().served());
    assert!(lists.draw(GROCERIES, false).is_empty());
    assert!(lists
        .edit(GROCERIES, check(LEMONS), "linux-1".into())
        .is_empty());

    let sent = lists.negotiated(7);
    assert_eq!(
        sent,
        [
            edit("linux-1", Some(GROCERIES), check(LEMONS)),
            index_pull()
        ]
    );
}

#[test]
fn a_list_is_pulled_once_when_it_is_first_drawn_and_its_answer_lands_in_its_one_state() {
    let (mut lists, mut sent) = connected();
    sent.extend(lists.received_index(index_of(&[GROCERIES])));

    sent.extend(lists.draw(GROCERIES, false));
    sent.extend(lists.draw(GROCERIES, false));
    lists.received_list(groceries(7), &BTreeMap::new());

    assert_eq!(sent, [index_pull(), pull(GROCERIES, None)]);
    assert_eq!(lists.shown(GROCERIES), Some(groceries(7)));
    assert_eq!(lists.draw(GROCERIES, true), [pull(GROCERIES, Some(true))]);
    assert!(
        lists.draw(GROCERIES, true).is_empty(),
        "held with its removed rows"
    );
}

#[test]
fn past_sixteen_lists_the_oldest_pulled_is_let_go_and_drawing_it_again_pulls_it_again() {
    let (mut lists, _) = connected();
    let all = ids(0..=16);
    let named: Vec<&str> = all.iter().map(String::as_str).collect();
    lists.received_index(index_of(&named));

    let mut sent = Vec::new();
    for id in &all {
        sent.extend(lists.draw(id, false));
    }
    sent.extend(lists.draw(&all[1], false));
    sent.extend(lists.draw(&all[0], false));

    assert_eq!(MAX_HELD, 16);
    let mut expected = all.clone();
    expected.push(all[0].clone());
    assert_eq!(
        pulls(&sent),
        expected,
        "the first was let go; the second is held"
    );
}

#[test]
fn a_list_pulled_again_after_a_refused_edit_is_the_newest_held_as_the_engine_holds_it() {
    let (mut lists, _) = connected();
    let all = ids(1..=17);
    let named: Vec<&str> = all.iter().map(String::as_str).collect();
    lists.received_index(index_of(&named));
    let mut sent = Vec::new();
    for id in &all[..16] {
        sent.extend(lists.draw(id, false));
    }
    let rename = ListOp::Rename {
        text: "Weekly".into(),
    };
    sent.extend(lists.edit(&all[0], rename, "linux-1".into()));
    let again = lists.refusal(&refusal("That list is gone.", Some("linux-1")));
    sent.extend(again.expect("the lists' refusal"));
    for id in [&all[16], &all[1], &all[0]] {
        sent.extend(lists.draw(id, false));
    }

    let mut expected: Vec<String> = all[..16].to_vec();
    expected.extend([all[0].clone(), all[16].clone(), all[1].clone()]);
    assert_eq!(pulls(&sent), expected, "the engine let the second go");
}

#[test]
fn what_is_drawn_before_the_handshake_is_pulled_once_the_index_answers_and_only_if_it_names_it() {
    let mut lists = Library::default();
    assert!(lists.draw(GROCERIES, false).is_empty());
    assert!(lists.draw("ck_0000000000", false).is_empty());

    assert_eq!(
        lists.negotiated(7),
        [index_pull()],
        "no list before the index"
    );
    assert_eq!(
        lists.received_index(index_of(&[GROCERIES])),
        [pull(GROCERIES, None)]
    );
}

#[test]
fn an_edit_is_drawn_at_once_and_sent_its_accepted_is_the_lists_and_the_push_confirms_it() {
    let (mut lists, _) = holding_groceries();

    let sent = lists.edit(GROCERIES, check(LEMONS), "linux-1".into());

    assert_eq!(sent, [edit("linux-1", Some(GROCERIES), check(LEMONS))]);
    assert_eq!(shown(&lists, GROCERIES).open_count(), 2);
    assert!(lists.accepted("linux-1", false));
    let state = lists.state(GROCERIES).expect("held");
    assert!(state.pending()[0].accepted);
    lists.received_list(groceries(8), &BTreeMap::new());
    assert!(lists.state(GROCERIES).expect("held").pending().is_empty());
}

#[test]
fn a_refused_edit_puts_the_row_back_with_the_engines_sentence_and_pulls_the_list_again() {
    let (mut lists, _) = holding_groceries();
    let uncheck = ListOp::Uncheck {
        row_id: BASIL.into(),
    };
    lists.edit(GROCERIES, uncheck, "linux-1".into());

    let sent = lists.refusal(&refusal(GONE, Some("linux-1")));

    assert_eq!(sent, Some(vec![pull(GROCERIES, None)]));
    let state = lists.state(GROCERIES).expect("held");
    assert_eq!(state.refusal(), Some(GONE));
    assert_eq!(state.shown(), Some(groceries(7)));
}

#[test]
fn a_refusal_or_an_accepted_that_names_no_edit_sent_here_is_the_chats() {
    let (mut lists, _) = holding_groceries();
    lists.edit(GROCERIES, check(LEMONS), "linux-1".into());

    assert_eq!(lists.refusal(&refusal("history is unreadable", None)), None);
    assert_eq!(lists.refusal(&refusal(GONE, Some("linux-chat"))), None);
    assert!(!lists.accepted("linux-chat", false));
    let state = lists.state(GROCERIES).expect("held");
    assert_eq!(state.refusal(), None);
    assert_eq!(state.pending().len(), 1);
    assert!(!state.is_removed());
    assert_eq!(lists.index().refusal(), None);
}

#[test]
fn a_refusal_with_no_sentence_says_the_daemons_word() {
    let (mut lists, _) = holding_groceries();
    lists.edit(GROCERIES, check(LEMONS), "linux-1".into());
    let line =
        json!({"type": "error", "reason": "client_message_conflict", "client_msg_id": "linux-1"});
    let ServerEvent::Error(error) = read_line(line.to_string().as_bytes()).expect("decodes") else {
        panic!("an error");
    };

    assert!(lists.refusal(&error).is_some());
    let state = lists.state(GROCERIES).expect("held");
    assert_eq!(
        state.refusal(),
        Some("The daemon reported: client_message_conflict")
    );
}

#[test]
fn a_list_the_loaded_index_does_not_name_is_never_pulled_and_is_pulled_once_an_index_names_it() {
    let (mut lists, _) = connected();
    lists.received_index(Vec::new());

    assert!(lists.draw(GROCERIES, false).is_empty());
    assert!(lists.index().lacks(GROCERIES), "drawn as removed");

    let sent = lists.received_index(index_of(&[GROCERIES]));
    assert_eq!(sent, [pull(GROCERIES, None)]);
    assert!(!lists.index().lacks(GROCERIES));
}

#[test]
fn an_edit_never_answered_is_sent_again_at_the_next_handshake_and_the_held_list_pulled_after_the_index(
) {
    let (mut lists, _) = holding_groceries();
    lists.edit(GROCERIES, check(LEMONS), "linux-1".into());
    let rename = ListOp::Rename {
        text: "Weekly".into(),
    };
    lists.edit(GROCERIES, rename.clone(), "linux-2".into());
    assert!(lists.accepted("linux-1", false));

    lists.disconnected();
    assert!(lists.draw(GROCERIES, false).is_empty(), "held, and offline");
    let sent = lists.negotiated(7);
    assert_eq!(
        sent,
        [edit("linux-2", Some(GROCERIES), rename), index_pull()]
    );
    let sent = lists.received_index(index_of(&[GROCERIES]));
    assert_eq!(sent, [pull(GROCERIES, None)]);
    assert_eq!(
        shown(&lists, GROCERIES).title,
        "Weekly",
        "kept across the drop"
    );
}

#[test]
fn an_add_the_engine_applied_before_the_drop_is_answered_duplicate_and_drawn_once() {
    let (mut lists, _) = holding_groceries();
    lists.edit(GROCERIES, add("Milk"), "linux-1".into());
    lists.disconnected();

    let sent = lists.negotiated(7);
    assert_eq!(
        sent,
        [edit("linux-1", Some(GROCERIES), add("Milk")), index_pull()],
        "sent again before anything is pulled"
    );
    assert!(lists.accepted("linux-1", true));
    lists.received_index(index_of(&[GROCERIES]));
    let mut list = groceries(8);
    list.rows.insert(
        1,
        ListRow {
            row_id: "ci_9x9x9x9x9x".into(),
            kind: ITEM.into(),
            text: "Milk".into(),
            note: None,
            done: Some(false),
            done_at: None,
        },
    );
    lists.received_list(list, &BTreeMap::new());

    let milk = shown(&lists, GROCERIES)
        .rows
        .iter()
        .filter(|row| row.text == "Milk")
        .count();
    assert_eq!(milk, 1);
}

#[test]
fn a_create_says_what_it_made_once_the_engine_pushes_the_new_list_which_is_held_from_then_on() {
    let (mut lists, _) = connected();

    let sent = lists.create("Packing", Some("Trips"), "mac-20".into());
    let ServerEvent::Checklist { list, made } = golden(55) else {
        panic!("line 55 is the new Packing list");
    };
    let create = ListOp::Create {
        title: "Packing".into(),
        folder: Some("Trips".into()),
    };
    assert_eq!(sent, Some(vec![edit("mac-20", None, create)]));
    assert!(lists.accepted("mac-20", false));
    lists.received_list(*list.clone(), &made);

    let created = lists.index().created().expect("the create named its list");
    assert_eq!(
        (created.client_msg_id.as_str(), created.list_id.as_str()),
        ("mac-20", PACKING)
    );
    assert_eq!(lists.shown(PACKING), Some(*list));
    assert_eq!(lists.creates_waiting(), 0);
    lists.received_index(index_of(&[PACKING]));
    assert!(
        lists.draw(PACKING, false).is_empty(),
        "the engine holds what its create made"
    );
}

/// A library whose create of Packing was sent, and the connection dropped and came back.
fn reconnected_after_create(accepted_before_drop: bool) -> Library {
    let (mut lists, _) = connected();
    lists.received_index(Vec::new());
    lists.create("Packing", Some("Trips"), "linux-1".into());
    if accepted_before_drop {
        assert!(lists.accepted("linux-1", false));
    }
    lists.disconnected();
    lists.negotiated(7);
    lists
}

fn packing_index(title: &str) -> Vec<ListEntry> {
    vec![ListEntry {
        folder: Some("Trips".into()),
        ..entry(PACKING, title)
    }]
}

#[test]
fn a_create_accepted_just_before_the_connection_dropped_is_found_in_the_next_index_by_its_title() {
    let mut lists = reconnected_after_create(true);

    let sent = lists.received_index(packing_index("Packing"));

    let created = lists.index().created().expect("named by its title");
    assert_eq!(created.list_id, PACKING);
    assert_eq!(pulls(&sent), [PACKING]);
    assert_eq!(lists.creates_waiting(), 0);
}

#[test]
fn a_create_whose_accepted_was_lost_comes_back_duplicate_and_the_index_names_it_case_blind() {
    let (mut lists, _) = connected();
    lists.received_index(Vec::new());
    lists.create("packing", Some("Trips"), "linux-1".into());
    lists.disconnected();
    lists.negotiated(7);
    assert!(lists.accepted("linux-1", true));

    let sent = lists.received_index(packing_index("Packing"));

    let created = lists.index().created().expect("named by its title");
    assert_eq!(created.list_id, PACKING);
    assert_eq!(pulls(&sent), [PACKING]);
    assert_eq!(lists.creates_waiting(), 0);
}

#[test]
fn a_create_the_next_connections_index_cannot_name_is_let_go_and_never_shown_later() {
    let mut lists = reconnected_after_create(true);

    lists.received_index(Vec::new());
    assert_eq!(lists.creates_waiting(), 0);

    lists.received_index(packing_index("Packing"));
    assert_eq!(lists.index().created(), None);
}

#[test]
fn a_create_the_engine_applied_counts_among_the_sixteen_held_before_its_push_names_it() {
    let (mut lists, _) = connected();
    let all = ids(1..=16);
    let named: Vec<&str> = all.iter().map(String::as_str).collect();
    lists.received_index(index_of(&named));
    let mut sent = Vec::new();
    for id in &all {
        sent.extend(lists.draw(id, false));
    }

    lists.create("Packing", None, "linux-1".into());
    assert!(lists.accepted("linux-1", false));
    sent.extend(lists.draw(&all[0], false));
    let ServerEvent::Checklist { list, .. } = golden(55) else {
        panic!("line 55 is the new Packing list");
    };
    let made = BTreeMap::from([("linux-1".to_owned(), PACKING.to_owned())]);
    lists.received_list(*list, &made);
    sent.extend(lists.draw(PACKING, false));

    let mut expected = all.clone();
    expected.push(all[0].clone());
    assert_eq!(
        pulls(&sent),
        expected,
        "the first was let go for the new list"
    );
}

#[test]
fn an_op_that_names_a_row_the_engine_has_not_named_yet_is_neither_drawn_nor_sent() {
    let (mut lists, _) = holding_groceries();
    lists.edit(GROCERIES, add("Milk"), "linux-1".into());
    let local = draft::local_row_id("linux-1");
    let in_list = shown(&lists, GROCERIES);
    assert!(in_list.rows.iter().any(|row| row.row_id == local));

    let ops = [
        check(&local),
        ListOp::Uncheck {
            row_id: local.clone(),
        },
        ListOp::Edit {
            row_id: local.clone(),
            text: Some("Oat milk".into()),
            note: None,
        },
        ListOp::Remove {
            row_id: local.clone(),
        },
        ListOp::Move {
            row_id: "ci_6j4c7x0m5s".into(),
            section_id: None,
            after_id: Some(local.clone()),
            to_list_id: None,
        },
        ListOp::Add {
            text: "Eggs".into(),
            note: None,
            section_id: Some(local.clone()),
            after_id: None,
        },
        ListOp::Section {
            text: "Dairy".into(),
            after_id: Some(local.clone()),
        },
    ];
    for (n, op) in ops.into_iter().enumerate() {
        let sent = lists.edit(GROCERIES, op, format!("linux-{}", n + 2));
        assert!(sent.is_empty());
    }

    assert_eq!(lists.state(GROCERIES).expect("held").pending().len(), 1);
}

#[test]
fn a_blank_text_is_no_edit_and_no_new_list() {
    let (mut lists, _) = holding_groceries();

    assert!(lists
        .edit(GROCERIES, add("  "), "linux-1".into())
        .is_empty());
    assert_eq!(lists.create(" ", None, "linux-2".into()), None);

    assert!(lists.state(GROCERIES).expect("held").pending().is_empty());
}

#[test]
fn the_sixty_fifth_waiting_edit_is_not_drawn_and_the_list_says_too_many_are_waiting() {
    let (mut lists, _) = holding_groceries();
    lists.disconnected();
    for n in 0..MAX_WAITING {
        let note = ListOp::Edit {
            row_id: LEMONS.into(),
            text: None,
            note: Some(format!("note {n}")),
        };
        lists.edit(GROCERIES, note, format!("linux-{n}"));
    }
    assert_eq!(MAX_WAITING, 64);

    lists.edit(GROCERIES, check(LEMONS), "linux-65".into());

    let state = lists.state(GROCERIES).expect("held");
    assert_eq!(state.pending().len(), 64);
    assert_eq!(state.refusal(), Some(TOO_MANY));
    assert!(
        !state.shown().expect("drawn").rows[2].is_done(),
        "not drawn"
    );
    assert_eq!(lists.create("Packing", None, "linux-66".into()), None);
    assert_eq!(lists.index().refusal(), Some(TOO_MANY));
    assert_eq!(
        lists.negotiated(7).len(),
        65,
        "the 64 edits, then the index pull"
    );
}

#[test]
fn a_refused_create_says_its_sentence_in_the_index_until_the_next_create() {
    let (mut lists, _) = connected();
    let taken = "A list named Groceries already exists.";
    lists.create("Groceries", None, "linux-1".into());

    assert_eq!(
        lists.refusal(&refusal(taken, Some("linux-1"))),
        Some(Vec::new())
    );
    assert_eq!(lists.index().refusal(), Some(taken));
    lists.create("Weekly", None, "linux-2".into());
    assert_eq!(lists.index().refusal(), None);
}

#[test]
fn lists_drawn_before_an_index_names_them_wait_at_most_sixty_four_the_oldest_dropped() {
    let mut lists = Library::default();
    let all = ids(0..=MAX_WAITING);
    for id in &all {
        lists.draw(id, false);
    }
    lists.negotiated(7);

    let named: Vec<&str> = all.iter().map(String::as_str).collect();
    let sent = lists.received_index(index_of(&named));

    assert_eq!(
        pulls(&sent),
        all[1..].to_vec(),
        "the first drawn was dropped"
    );
}

/// At a reconnect, the lists held when the connection dropped wait ahead of those drawn since,
/// and the queue stays at 64: past it, the oldest drawn since drop (M85 lists §5.2, §5.4).
#[test]
fn a_reconnect_with_sixteen_held_and_sixty_four_drawn_keeps_the_held_and_drops_the_oldest_drawn() {
    let mut lists = Library::default();
    lists.negotiated(7);
    let held = ids(1..=MAX_HELD);
    let named: Vec<&str> = held.iter().map(String::as_str).collect();
    lists.received_index(index_of(&named));
    for id in &held {
        lists.draw(id, false);
    }
    lists.disconnected();
    let drawn = ids(MAX_HELD + 1..=MAX_HELD + MAX_WAITING);
    for id in &drawn {
        lists.draw(id, false);
    }

    lists.negotiated(7);
    let all: Vec<&str> = held.iter().chain(&drawn).map(String::as_str).collect();
    let sent = lists.received_index(index_of(&all));

    let room = MAX_WAITING - MAX_HELD;
    let wanted: Vec<String> = held
        .iter()
        .chain(&drawn[MAX_WAITING - room..])
        .cloned()
        .collect();
    assert_eq!(
        pulls(&sent),
        wanted,
        "the held first, then the newest drawn, 64 in all"
    );
}
