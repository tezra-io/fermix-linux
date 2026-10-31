//! The chat's timeline reducer: every server event and request, as values, against the rules of
//! the vendored PROTOCOL.md ("Keeping a client's timeline", "Delivery and the outbox",
//! "Streaming a turn", "Approvals") and the macOS reducer it ports.

use fermix_client::chat_wire::protocol::{decode_line, ClientEvent, Cursor, ServerEvent};
use fermix_client::chat_wire::timeline::{
    self, route, Chat, ToolState, MAX_HELD, MAX_PENDING, NEWEST, PAGE_LIMIT,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn event(value: Value) -> ServerEvent {
    decode_line(value.to_string().as_bytes()).expect("a test event decodes")
}

fn message(seq: u64, text: &str) -> Value {
    json!({"server_seq": seq, "role": "assistant", "content": text, "ts": "t", "media_refs": []})
}

fn page(messages: Vec<Value>, head: u64, after: Option<u64>, before: Option<u64>) -> ServerEvent {
    let mut page = json!({"type": "history_page", "profile_id": "main",
        "messages": messages, "history_head_seq": head});
    if let Some(after) = after {
        page["next_after_seq"] = json!(after);
    }
    if let Some(before) = before {
        page["next_before_seq"] = json!(before);
    }
    event(page)
}

fn live(seq: u64, text: &str) -> ServerEvent {
    event(
        json!({"type": "row", "profile_id": "main", "server_seq": seq, "role": "assistant",
        "text": text, "ts": "t", "media_refs": []}),
    )
}

fn receive(chat: &mut Chat, event: ServerEvent) -> Vec<ClientEvent> {
    timeline::receive(chat, event, 0)
}

fn seqs(chat: &Chat) -> Vec<u64> {
    chat.rows.iter().map(|held| held.row.server_seq).collect()
}

fn pull(cursor: Cursor, limit: u16) -> ClientEvent {
    ClientEvent::pull(cursor, limit)
}

/// A chat connected and holding rows 1..=n, nothing out.
fn holding(n: u64) -> Chat {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    let messages = (1..=n).map(|seq| message(seq, "row")).collect();
    receive(&mut chat, page(messages, n, None, None));
    chat
}

#[test]
fn a_cold_start_reads_the_newest_page_and_a_warm_one_reads_on_from_its_cursor() {
    let mut cold = Chat::default();
    let sent = timeline::negotiated(&mut cold, &BTreeSet::new());
    assert_eq!(sent, [pull(Cursor::BeforeSeq(NEWEST), PAGE_LIMIT)]);

    let mut warm = holding(3);
    timeline::disconnected(&mut warm);
    let sent = timeline::negotiated(&mut warm, &BTreeSet::new());
    assert_eq!(sent, [pull(Cursor::AfterSeq(3), PAGE_LIMIT)]);
}

#[test]
fn the_outbox_is_resent_in_order_after_the_catch_up_read() {
    let mut chat = holding(1);
    timeline::disconnected(&mut chat);
    timeline::send(&mut chat, "one", "linux-1".into());
    timeline::send(&mut chat, "two", "linux-2".into());
    let sent = timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(
        sent,
        [
            pull(Cursor::AfterSeq(1), PAGE_LIMIT),
            ClientEvent::msg("linux-1", "one"),
            ClientEvent::msg("linux-2", "two"),
        ]
    );
}

#[test]
fn a_send_while_disconnected_waits_and_a_blank_one_is_refused() {
    let mut chat = Chat::default();
    assert!(timeline::send(&mut chat, "hi", "linux-1".into()).is_empty());
    assert_eq!(chat.pending.len(), 1);
    assert!(timeline::send(&mut chat, "  ", "linux-2".into()).is_empty());
    assert_eq!(chat.pending.len(), 1);
}

#[test]
fn the_outbox_is_bounded() {
    let mut chat = holding(0);
    for n in 0..MAX_PENDING {
        timeline::send(&mut chat, "x", format!("linux-{n}"));
    }
    assert!(timeline::send(&mut chat, "x", "linux-over".into()).is_empty());
    assert_eq!(chat.pending.len(), MAX_PENDING);
    assert!(chat.last_error.is_some());
}

#[test]
fn accepted_clears_the_outbox_entry_even_as_a_duplicate() {
    let mut chat = holding(0);
    timeline::send(&mut chat, "hi", "linux-1".into());
    let accepted = json!({"type": "accepted", "client_msg_id": "linux-1",
        "duplicate": true, "server_seq": 4});
    receive(&mut chat, event(accepted));
    assert!(chat.pending.is_empty());
}

#[test]
fn a_live_row_is_shown_when_next_dropped_when_old_and_a_gap_pulls_once() {
    let mut chat = holding(2);
    assert!(receive(&mut chat, live(3, "next")).is_empty());
    assert!(receive(&mut chat, live(3, "again")).is_empty());
    assert_eq!(seqs(&chat), [1, 2, 3]);
    assert_eq!(chat.cursor, 3);

    let sent = receive(&mut chat, live(5, "gap"));
    assert_eq!(sent, [pull(Cursor::AfterSeq(3), PAGE_LIMIT)]);
    assert!(receive(&mut chat, live(6, "gap again")).is_empty());
    assert_eq!(seqs(&chat), [1, 2, 3]);
}

#[test]
fn a_page_merges_by_seq_and_a_changed_row_takes_the_pages_content() {
    let mut chat = holding(2);
    receive(&mut chat, live(4, "gap"));
    let sent = receive(
        &mut chat,
        page(
            vec![
                message(2, "changed"),
                message(3, "three"),
                message(4, "gap"),
            ],
            4,
            Some(4),
            None,
        ),
    );
    assert!(sent.is_empty());
    assert_eq!(seqs(&chat), [1, 2, 3, 4]);
    assert_eq!(chat.rows[1].row.text, "changed");
}

#[test]
fn a_forward_page_below_the_head_reads_on() {
    let mut chat = holding(1);
    receive(&mut chat, live(3, "gap"));
    let sent = receive(&mut chat, page(vec![message(2, "two")], 9, Some(2), None));
    assert_eq!(sent, [pull(Cursor::AfterSeq(2), PAGE_LIMIT)]);
}

#[test]
fn only_a_backward_page_says_whether_older_rows_exist() {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(&mut chat, page(vec![message(70, "x")], 70, None, Some(70)));
    assert!(chat.has_older);
    let sent = timeline::pull_older(&mut chat);
    assert_eq!(sent, [pull(Cursor::BeforeSeq(70), PAGE_LIMIT)]);
    receive(&mut chat, page(vec![message(69, "y")], 70, None, None));
    assert!(!chat.has_older);
    assert_eq!(seqs(&chat), [69, 70]);
}

#[test]
fn an_older_read_asked_while_a_read_is_out_waits_its_turn() {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(
        &mut chat,
        page(vec![message(70, "x")], 72, Some(70), Some(70)),
    );
    assert!(
        timeline::pull_older(&mut chat).is_empty(),
        "the catch-up is out"
    );
    let sent = receive(
        &mut chat,
        page(vec![message(71, "a"), message(72, "b")], 72, Some(72), None),
    );
    assert_eq!(sent, [pull(Cursor::BeforeSeq(70), PAGE_LIMIT)]);
}

#[test]
fn a_row_updated_is_read_once_for_a_held_row_and_dropped_for_another() {
    let mut chat = holding(3);
    let updated =
        |seq: u64| event(json!({"type": "row_updated", "profile_id": "main", "server_seq": seq}));
    assert!(receive(&mut chat, updated(9)).is_empty());
    let sent = receive(&mut chat, updated(2));
    assert_eq!(sent, [pull(Cursor::AfterSeq(1), 1)]);
    assert!(receive(&mut chat, updated(2)).is_empty(), "one read out");
    receive(
        &mut chat,
        page(vec![message(2, "transcript")], 3, Some(2), None),
    );
    assert_eq!(chat.rows[1].row.text, "transcript");
}

#[test]
fn a_rows_read_cut_by_the_page_cap_continues() {
    let mut chat = holding(5);
    let updated =
        |seq: u64| event(json!({"type": "row_updated", "profile_id": "main", "server_seq": seq}));
    receive(&mut chat, updated(2));
    receive(&mut chat, updated(4));
    let sent = receive(&mut chat, page(vec![message(2, "two")], 5, Some(2), None));
    assert_eq!(
        sent,
        [pull(Cursor::AfterSeq(3), 1)],
        "seq 4 was named while a read was out"
    );
    let mut chat = holding(5);
    timeline::disconnected(&mut chat);
    let on_screen = BTreeSet::from([2, 4]);
    timeline::negotiated(&mut chat, &on_screen);
    let sent = receive(&mut chat, page(Vec::new(), 5, Some(5), None));
    assert_eq!(sent, [pull(Cursor::AfterSeq(1), 3)]);
    let sent = receive(&mut chat, page(vec![message(2, "two")], 5, Some(2), None));
    assert_eq!(
        sent,
        [pull(Cursor::AfterSeq(3), 1)],
        "seq 4 was past the cut"
    );
}

#[test]
fn rows_coming_on_screen_after_a_reconnect_are_read_again_once() {
    let mut chat = holding(3);
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(&mut chat, page(Vec::new(), 3, Some(3), None));
    let sent = timeline::rows_on_screen(&mut chat, &BTreeSet::from([3]));
    assert_eq!(sent, [pull(Cursor::AfterSeq(2), 1)]);
    receive(&mut chat, page(vec![message(3, "row")], 3, Some(3), None));
    assert!(timeline::rows_on_screen(&mut chat, &BTreeSet::from([3])).is_empty());
}

#[test]
fn a_disconnect_ends_the_turn_and_frees_the_read() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t1", "text": "Hal"})),
    );
    receive(&mut chat, live(3, "gap"));
    timeline::disconnected(&mut chat);
    assert!(chat.turn.is_none());
    let sent = timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(sent, [pull(Cursor::AfterSeq(1), PAGE_LIMIT)]);
}

#[test]
fn a_reads_asked_while_disconnected_go_out_after_the_handshake_read() {
    let mut chat = holding(1);
    timeline::disconnected(&mut chat);
    assert!(timeline::pull_older(&mut chat).is_empty());
    let sent = timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(sent, [pull(Cursor::AfterSeq(1), PAGE_LIMIT)]);
}

#[test]
fn a_turn_streams_its_draft_and_its_text_done_becomes_the_row() {
    let mut chat = holding(1);
    timeline::send(&mut chat, "Hi", "linux-1".into());
    receive(
        &mut chat,
        event(json!({"type": "turn_started", "profile_id": "main",
        "turn_id": "t1", "in_reply_to": "linux-1"})),
    );
    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t1", "text": "Hel"})),
    );
    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t1", "text": "lo "})),
    );
    let turn = chat.turn.as_ref().expect("a turn");
    assert_eq!(turn.draft, "Hello ");
    assert!(timeline::can_stop(&chat));
    receive(
        &mut chat,
        event(json!({"type": "text_done", "turn_id": "t1",
        "server_seq": 2, "text": "Hello."})),
    );
    assert!(chat.turn.is_none());
    let reply = &chat.rows[1];
    assert_eq!(reply.row.text, "Hello.");
    assert_eq!(reply.row.in_reply_to.as_deref(), Some("linux-1"));
    assert_eq!(reply.turn_id.as_deref(), Some("t1"));
}

#[test]
fn a_second_text_done_of_a_turn_still_lands() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "text_done", "turn_id": "t1", "server_seq": 2, "text": "a"})),
    );
    receive(
        &mut chat,
        event(json!({"type": "text_done", "turn_id": "t1", "server_seq": 3, "text": "b"})),
    );
    assert_eq!(seqs(&chat), [1, 2, 3]);
}

#[test]
fn a_text_done_of_another_turn_leaves_the_running_turn() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t2", "text": "x"})),
    );
    receive(
        &mut chat,
        event(json!({"type": "text_done", "turn_id": "t1", "server_seq": 2, "text": "a"})),
    );
    assert_eq!(
        chat.turn.as_ref().map(|turn| turn.turn_id.as_str()),
        Some("t2")
    );
}

#[test]
fn a_turn_joined_midway_has_no_stop() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t9", "text": "so far"})),
    );
    assert_eq!(chat.turn.as_ref().unwrap().draft, "so far");
    assert!(!timeline::can_stop(&chat));
    assert!(timeline::stop(&mut chat).is_empty());
}

#[test]
fn stop_cancels_the_request_the_turn_answers_and_never_a_task() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "turn_started", "profile_id": "main",
        "turn_id": "t1", "in_reply_to": "linux-1"})),
    );
    let sent = timeline::stop(&mut chat);
    assert_eq!(sent, [ClientEvent::cancel("linux-1")]);
    let ClientEvent::Cancel { task_ref, .. } = &sent[0] else {
        panic!("a cancel");
    };
    assert!(task_ref.is_none());
    assert!(
        chat.turn.as_ref().unwrap().stopping,
        "the turn shows it is stopping"
    );
}

#[test]
fn tools_pair_by_name_and_a_stray_stop_changes_nothing() {
    let mut chat = holding(1);
    let tool = |phase: &str, name: &str| {
        event(json!({"type": "tool_event", "turn_id": "t1", "tool": name, "phase": phase}))
    };
    timeline::receive(&mut chat, tool("start", "web_search"), 10);
    timeline::receive(&mut chat, tool("start", "web_search"), 11);
    timeline::receive(&mut chat, tool("stop", "web_search"), 15);
    let turn = chat.turn.as_ref().unwrap();
    assert_eq!(turn.started, 10, "the turn began with its first event");
    assert_eq!((turn.tools[0].started, turn.tools[0].ended), (10, Some(15)));
    assert_eq!((turn.tools[1].started, turn.tools[1].ended), (11, None));
    receive(&mut chat, tool("stop", "read_file"));
    let states: Vec<ToolState> = chat
        .turn
        .as_ref()
        .unwrap()
        .tools
        .iter()
        .map(|run| run.state)
        .collect();
    assert_eq!(states, [ToolState::Done, ToolState::Running]);
}

#[test]
fn turn_done_and_turn_error_end_the_turn_and_keep_nothing_of_the_draft() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t1", "text": "x"})),
    );
    receive(
        &mut chat,
        event(json!({"type": "turn_done", "turn_id": "t1"})),
    );
    assert!(chat.turn.is_none());
    assert_eq!(seqs(&chat), [1]);
    assert!(chat.last_error.is_none());

    receive(
        &mut chat,
        event(json!({"type": "text_delta", "turn_id": "t2", "text": "x"})),
    );
    receive(
        &mut chat,
        event(json!({"type": "turn_error", "turn_id": "t2",
        "code": "cancelled", "message": "cancelled"})),
    );
    assert!(chat.turn.is_none());
    assert_eq!(chat.last_error.as_deref(), Some("The reply was stopped"));
}

#[test]
fn a_failed_request_leaves_the_outbox_with_a_sentence() {
    let mut chat = holding(0);
    timeline::send(&mut chat, "hi", "linux-4".into());
    receive(
        &mut chat,
        event(json!({"type": "error", "reason": "request_failed",
        "message": ":timeout", "client_msg_id": "linux-4"})),
    );
    assert!(chat.pending.is_empty());
    assert_eq!(
        chat.last_error.as_deref(),
        Some("The daemon reported: request_failed")
    );
}

#[test]
fn a_media_failure_is_not_the_timelines() {
    let mut chat = holding(0);
    receive(
        &mut chat,
        event(json!({"type": "error", "reason": "request_failed",
        "message": "enospc", "ref": "a".repeat(64)})),
    );
    assert!(chat.last_error.is_none());
}

#[test]
fn approvals_replace_by_id_clear_on_hello_and_expire_by_their_own_ttl() {
    let mut chat = holding(0);
    let card = |ttl: u64, text: &str| {
        event(
            json!({"type": "approval", "approval_id": "a1", "kind": "sandbox", "text": text,
            "token": "tok", "ttl_s": ttl, "approve_command": "/confirm tok",
            "deny_command": "/deny tok"}),
        )
    };
    timeline::receive(&mut chat, card(60, "first"), 100);
    timeline::receive(&mut chat, card(30, "again"), 110);
    assert_eq!(chat.approvals.len(), 1);
    assert_eq!(chat.approvals[0].approval.text, "again");
    timeline::expire_approvals(&mut chat, 139);
    assert_eq!(chat.approvals.len(), 1);
    timeline::expire_approvals(&mut chat, 140);
    assert!(chat.approvals.is_empty());

    timeline::receive(&mut chat, card(60, "back"), 200);
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert!(
        chat.approvals.is_empty(),
        "the daemon sends the waiting ones again"
    );
}

#[test]
fn an_answer_sends_the_route_once_and_resolution_removes_the_card() {
    let mut chat = holding(0);
    timeline::receive(
        &mut chat,
        event(json!({"type": "approval", "approval_id": "s1",
        "kind": "soul", "text": "Apply?", "token": "tok", "ttl_s": 60,
        "approve_command": "/soul apply tok", "deny_command": "/soul discard tok"})),
        0,
    );
    let sent = timeline::answer(&mut chat, "s1", true, "linux-7".into());
    assert_eq!(
        sent,
        [ClientEvent::command("linux-7", "soul", Some("apply tok"))]
    );
    assert!(timeline::answering(&chat, "s1"));
    assert!(timeline::answer(&mut chat, "s1", false, "linux-8".into()).is_empty());
    receive(
        &mut chat,
        event(json!({"type": "approval_resolved", "approval_id": "s1",
        "outcome": "approved"})),
    );
    assert!(chat.approvals.is_empty());
}

#[test]
fn a_route_splits_into_its_command_and_arguments() {
    assert_eq!(
        route("/confirm TOKEN"),
        Some(("confirm".into(), Some("TOKEN".into())))
    );
    assert_eq!(
        route("/soul apply TOKEN"),
        Some(("soul".into(), Some("apply TOKEN".into())))
    );
    assert_eq!(route("/stop"), Some(("stop".into(), None)));
    assert_eq!(route("confirm"), None);
    assert_eq!(route("/"), None);
}

#[test]
fn the_read_frontier_is_sent_once_per_new_row_and_follows_the_daemon() {
    let mut chat = holding(3);
    assert_eq!(
        timeline::newest_seen(&mut chat),
        [ClientEvent::read_state(3)]
    );
    assert!(timeline::newest_seen(&mut chat).is_empty());
    receive(
        &mut chat,
        event(json!({"type": "read_state", "profile_id": "main", "read_up_to_seq": 5})),
    );
    assert_eq!(chat.read_up_to, 5);
}

#[test]
fn a_row_with_an_unforeseen_role_is_held() {
    let mut chat = holding(1);
    receive(
        &mut chat,
        event(json!({"type": "row", "profile_id": "main", "server_seq": 2,
        "role": "system", "text": "note", "ts": "t", "media_refs": []})),
    );
    assert_eq!(chat.rows[1].row.role, "system");
}

#[test]
fn a_page_with_a_malformed_message_still_merges_and_does_not_loop() {
    let mut chat = holding(1);
    receive(&mut chat, live(3, "gap"));
    let sent = receive(
        &mut chat,
        page(
            vec![message(2, "ok"), json!({"server_seq": "bad"})],
            2,
            Some(2),
            None,
        ),
    );
    assert!(sent.is_empty());
    assert_eq!(seqs(&chat), [1, 2]);
}

#[test]
fn live_growth_past_the_cap_forgets_the_oldest_row() {
    let mut chat = holding(MAX_HELD as u64);
    assert!(!chat.has_older);
    receive(&mut chat, live(MAX_HELD as u64 + 1, "new"));
    assert_eq!(chat.rows.len(), MAX_HELD);
    assert_eq!(chat.rows[0].row.server_seq, 2);
    assert!(chat.has_older);
}

#[test]
fn an_unknown_event_changes_nothing() {
    let mut chat = holding(1);
    let before = chat.clone();
    receive(&mut chat, ServerEvent::Unknown("later".into()));
    assert_eq!(chat, before);
}

#[test]
fn a_message_too_long_for_one_line_never_enters_the_outbox() {
    let mut chat = holding(0);
    let sent = timeline::send(&mut chat, &"x".repeat(70_000), "linux-big".into());
    assert!(sent.is_empty());
    assert!(chat.pending.is_empty());
    assert_eq!(
        chat.last_error.as_deref(),
        Some("The message is too long to send")
    );
}

#[test]
fn a_rows_read_answered_by_an_empty_page_is_not_asked_again() {
    let mut chat = holding(3);
    let updated =
        |seq: u64| event(json!({"type": "row_updated", "profile_id": "main", "server_seq": seq}));
    assert_eq!(
        receive(&mut chat, updated(2)),
        [pull(Cursor::AfterSeq(1), 1)]
    );
    let sent = receive(&mut chat, page(Vec::new(), 3, Some(1), None));
    assert!(sent.is_empty(), "nothing is left to read from 2 on");
}

#[test]
fn asking_for_the_page_already_being_read_asks_nothing_more() {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(&mut chat, page(vec![message(70, "x")], 70, None, Some(70)));
    assert_eq!(
        timeline::pull_older(&mut chat),
        [pull(Cursor::BeforeSeq(70), PAGE_LIMIT)]
    );
    assert!(timeline::pull_older(&mut chat).is_empty());
    let sent = receive(&mut chat, page(vec![message(69, "y")], 70, None, Some(69)));
    assert!(sent.is_empty(), "the page before 70 is held");
}

#[test]
fn an_older_read_asked_while_disconnected_is_not_kept_for_later() {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(&mut chat, page(vec![message(70, "x")], 70, None, Some(70)));
    timeline::disconnected(&mut chat);
    assert!(timeline::pull_older(&mut chat).is_empty());
    let sent = timeline::negotiated(&mut chat, &BTreeSet::new());
    let sent = [
        sent,
        receive(&mut chat, page(Vec::new(), 70, Some(70), None)),
    ]
    .concat();
    assert_eq!(sent, [pull(Cursor::AfterSeq(70), PAGE_LIMIT)]);
}

#[test]
fn a_page_that_brought_a_row_again_spares_it_the_re_read_on_screen() {
    let mut chat = holding(3);
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(
        &mut chat,
        page(vec![message(2, "row"), message(3, "row")], 3, Some(3), None),
    );
    assert!(timeline::rows_on_screen(&mut chat, &BTreeSet::from([3])).is_empty());
    assert_eq!(
        timeline::rows_on_screen(&mut chat, &BTreeSet::from([1])),
        [pull(Cursor::AfterSeq(0), 1)]
    );
}

#[test]
fn a_forgotten_row_leaves_no_read_behind() {
    let mut chat = holding(MAX_HELD as u64);
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    receive(
        &mut chat,
        page(Vec::new(), MAX_HELD as u64, Some(MAX_HELD as u64), None),
    );
    receive(&mut chat, live(MAX_HELD as u64 + 1, "new"));
    assert!(timeline::rows_on_screen(&mut chat, &BTreeSet::from([1])).is_empty());
}

/// A card of kind `device` (M75 §11) is the agent asking to run a command on one of the owner's
/// desktops. It is held and answered like a sandbox card, and the words it draws keep the
/// computer and the exact command the daemon wrote, line for line.
#[test]
fn a_device_card_names_the_computer_and_the_command_and_answers_like_sandbox() {
    let text = "Run this command on your Studio Mac? It runs there, inside that computer's own \
        sandbox.\nComputer: Studio Mac\nCommand: ls -la ~/Projects";
    let mut chat = holding(0);
    timeline::receive(
        &mut chat,
        event(
            json!({"type": "approval", "approval_id": "device-d1", "kind": "device",
        "text": text, "token": "tok", "ttl_s": 120,
        "approve_command": "/confirm tok", "deny_command": "/deny tok"}),
        ),
        0,
    );
    assert_eq!(chat.approvals.len(), 1);
    let card = &chat.approvals[0].approval;
    assert_eq!(card.kind, "device");
    assert_eq!(card.text, text);
    assert!(card.text.contains("Computer: Studio Mac"));
    assert!(card.text.contains("Command: ls -la ~/Projects"));
    assert!(!card.text.contains("tok"), "the token is never drawn");
    let sent = timeline::answer(&mut chat, "device-d1", true, "linux-9".into());
    assert_eq!(
        sent,
        [ClientEvent::command("linux-9", "confirm", Some("tok"))]
    );
}
