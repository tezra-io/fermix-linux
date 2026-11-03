//! A mandate's proposals and a watcher's follow-ups in Chat (M60 cues §5, §6, §9 "Chat" and
//! "Outside text"; PROTOCOL.md "Version 5"): the card on the wire, held by id in the timeline,
//! answered once through the outbox by its id and the word alone, expired on this computer's
//! clock, dropped and held again at a handshake, resolved by the daemon, placed among the
//! messages by when it was made, and its outside text kept plain.

use fermix_client::chat::{view, Entry, Key};
use fermix_client::chat_wire::proposals::{
    self, card, facts, item_header, outside, CardAsk, Footer, Item, ALWAYS_ALLOW, CONFIRM, DENY,
    OUTSIDE_LINES, REASON, RUN, SKIP, STRINGS,
};
use fermix_client::chat_wire::protocol::{
    read_line, Ask, ClientEvent, Decision, ProposalState, ServerEvent, Unreadable,
};
use fermix_client::chat_wire::timeline::{self, Chat, Standing};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// 2026-10-22T14:00:00Z, the clock most cases stand at.
const NOON: i64 = 1_792_677_600;
const ARGS: &str = "{\n  \"repo\": \"ops\",\n  \"title\": \"Phoenix 2.0\"\n}";

fn event(value: Value) -> ServerEvent {
    let event = read_line(value.to_string().as_bytes()).expect("a test event decodes");
    assert!(
        !matches!(event, ServerEvent::Unreadable(_)),
        "a test event reads: {event:?}"
    );
    event
}

/// A card as the engine writes it, waiting, made at 14:00 and waiting until 14:30.
fn card_json(id: &str) -> Value {
    json!({
        "proposal_id": id, "state": "waiting", "mandate": "release-issues",
        "tool": "github_create_issue", "task": {"job_id": "job_7f3a", "name": "Release watch"},
        "watcher": "release-watch", "source_ref": "eng-blogs",
        "created_at": "2026-10-22T14:00:00Z", "expires_at": "2026-10-22T14:30:00Z",
        "received_at": "2026-10-22T13:59:00Z",
        "status_sentence": "Waiting for your answer; it runs only if you confirm.",
        "summary": "A task wants to run github_create_issue under your mandate release-issues.",
        "outside": {"args_text": ARGS, "cue": {"type": "release", "text": "Phoenix 2.0 is out."}}
    })
}

fn with(mut card: Value, key: &str, value: Value) -> Value {
    card[key] = value;
    card
}

fn proposal(card: Value) -> ServerEvent {
    event(json!({"type": "proposal", "profile_id": "main", "proposal": card}))
}

fn resolved(id: &str, state: &str, sentence: &str, result: Option<&str>) -> ServerEvent {
    let mut line = json!({"type": "proposal_resolved", "profile_id": "main",
        "proposal_id": id, "state": state, "sentence": sentence});
    if let Some(result) = result {
        line["result_text"] = json!(result);
    }
    event(line)
}

fn accepted(id: &str) -> ServerEvent {
    event(json!({"type": "accepted", "client_msg_id": id, "duplicate": false}))
}

fn refused(id: &str, message: &str) -> ServerEvent {
    event(
        json!({"type": "error", "reason": "request_failed", "message": message,
        "client_msg_id": id}),
    )
}

/// A chat that has said hello and holds the cards the daemon sent, at noon.
fn holding(cards: Vec<Value>) -> Chat {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    for card in cards {
        timeline::receive(&mut chat, proposal(card), NOON);
    }
    chat
}

fn standing(chat: &Chat, id: &str) -> Standing {
    chat.proposals
        .iter()
        .find(|held| held.proposal.proposal_id == id)
        .map(|held| held.standing.clone())
        .unwrap_or_else(|| panic!("{id} is held"))
}

fn ids(chat: &Chat) -> Vec<&str> {
    chat.proposals
        .iter()
        .map(|held| held.proposal.proposal_id.as_str())
        .collect()
}

fn answer(chat: &mut Chat, id: &str, decision: Decision, msg: &str) -> Vec<ClientEvent> {
    timeline::answer_proposal(chat, id, decision, msg.into())
}

/// The cards a view draws, by id, with their footers.
fn drawn_cards(chat: &Chat) -> Vec<(String, Footer)> {
    view(chat)
        .entries
        .into_iter()
        .filter_map(|entry| match entry {
            Entry::Proposal(card) => Some((card.id, card.footer)),
            _ => None,
        })
        .collect()
}

/// "14:30" for any RFC 3339 time, as the app's clock would show a time today.
fn clock(at: &str) -> Option<String> {
    at.get(11..16).map(str::to_owned)
}

// The wire.

#[test]
fn a_card_decodes_with_nulls_and_with_absent_keys_alike() {
    let mut nulls = card_json("p1");
    for key in ["watcher", "source_ref", "received_at", "decided_at"] {
        nulls[key] = Value::Null;
    }
    nulls["task"]["name"] = Value::Null;
    nulls["outside"]["cue"] = Value::Null;
    let mut absent = card_json("p1");
    for key in ["watcher", "source_ref", "received_at"] {
        absent.as_object_mut().unwrap().remove(key);
    }
    absent["task"].as_object_mut().unwrap().remove("name");
    absent["outside"].as_object_mut().unwrap().remove("cue");
    assert_eq!(proposal(nulls), proposal(absent));
}

#[test]
fn the_link_reads_a_card_without_its_arguments_as_unreadable_by_its_id() {
    let mut card = card_json("p1");
    card["outside"].as_object_mut().unwrap().remove("args_text");
    let line = json!({"type": "proposal", "profile_id": "main", "proposal": card});
    assert_eq!(
        read_line(line.to_string().as_bytes()),
        Ok(ServerEvent::Unreadable(Unreadable {
            kind: "proposal".into(),
            card_id: Some("p1".into()),
            ..Unreadable::default()
        }))
    );
}

/// A watcher's follow-up as the engine writes it (PROTOCOL.md "Version 5"): no call, the
/// follow-up named instead, and outside only the filter's reason and the cue.
fn follow_up_json(id: &str) -> Value {
    json!({
        "proposal_id": id, "state": "waiting", "task": {"job_id": "codex_review"},
        "created_at": "2026-10-22T14:00:00Z", "expires_at": "2026-10-22T14:30:00Z",
        "status_sentence": "Waiting for your answer; it runs only if you confirm.",
        "summary": "Watcher codex-limits wants to run its follow-up \"Codex review\".",
        "outside": {"reason": "The post says usage limits reset."},
        "follow_up": {"asks": "run", "offers_always_allow": true, "name": "Codex review",
            "instructions": "Review the last day's commits.", "tools": ["codex"],
            "reports_to": "companion"}
    })
}

/// A call is its arguments: one whose arguments are blank would draw an empty block and ask the
/// owner to confirm nothing they can see, so it is unreadable by its id.
#[test]
fn a_call_with_blank_arguments_is_unreadable_by_its_id() {
    for blank in ["", "  "] {
        let mut card = card_json("p1");
        card["outside"]["args_text"] = json!(blank);
        let line = json!({"type": "proposal", "profile_id": "main", "proposal": card});
        assert_eq!(
            read_line(line.to_string().as_bytes()),
            Ok(ServerEvent::Unreadable(Unreadable {
                kind: "proposal".into(),
                card_id: Some("p1".into()),
                ..Unreadable::default()
            })),
            "{blank:?}"
        );
    }
}

#[test]
fn a_card_that_is_both_a_call_and_a_follow_up_is_unreadable_by_its_id() {
    let both = with(
        card_json("p1"),
        "follow_up",
        follow_up_json("p1")["follow_up"].clone(),
    );
    let line = json!({"type": "proposal", "profile_id": "main", "proposal": both});
    assert_eq!(
        read_line(line.to_string().as_bytes()),
        Ok(ServerEvent::Unreadable(Unreadable {
            kind: "proposal".into(),
            card_id: Some("p1".into()),
            ..Unreadable::default()
        }))
    );
}

/// A follow-up's card is held like a call's, by its id: it is drawn, it takes an answer, and its
/// resolution ends it. No placeholder stands for it.
#[test]
fn a_follow_up_is_held_as_its_card_and_resolved_by_its_id() {
    let mut chat = holding(vec![follow_up_json("cft_1")]);
    assert_eq!(ids(&chat), ["cft_1"]);
    assert!(chat.unshown.is_empty(), "no placeholder stands for it");
    assert_eq!(drawn_cards(&chat), [("cft_1".to_owned(), Footer::Ask)]);
    let sent = answer(&mut chat, "cft_1", Decision::Confirm, "linux-1");
    assert_eq!(
        sent,
        [ClientEvent::proposal_answer(
            "linux-1",
            "cft_1",
            Decision::Confirm
        )]
    );
    let starting = "Confirmed; the follow-up is starting.";
    timeline::receive(
        &mut chat,
        resolved("cft_1", "confirmed", starting, None),
        NOON,
    );
    assert_eq!(
        drawn_cards(&chat),
        [(
            "cft_1".to_owned(),
            Footer::Resolved {
                sentence: starting.into(),
                result: None
            }
        )]
    );
}

/// A call's arguments are part of what it asks, so its card always has them. A follow-up asks no
/// call: its card says what it will do, where it reports and its tools, in Fermix's and the
/// owner's words, and has no arguments rather than empty ones.
#[test]
fn a_calls_card_carries_its_arguments_and_a_follow_ups_its_instructions() {
    let chat = holding(vec![card_json("p1"), follow_up_json("cft_1")]);
    let held = &chat.proposals[0];
    let Ask::Call { args_text, .. } = &held.proposal.ask else {
        panic!("a call: {:?}", held.proposal.ask);
    };
    assert_eq!(args_text, ARGS);
    assert_eq!(card(held).ask, CardAsk::Call { call: ARGS.into() });
    assert_eq!(
        card(&chat.proposals[1]).ask,
        CardAsk::FollowUp {
            instructions: "It will: Review the last day's commits.".into(),
            facts: "Reports to companion · Tools: codex".into(),
        }
    );
}

#[test]
fn a_follow_up_with_no_tools_says_only_where_it_reports() {
    let mut bare = follow_up_json("cft_1");
    bare["follow_up"]["tools"] = json!([]);
    let chat = holding(vec![bare]);
    let CardAsk::FollowUp { facts, .. } = card(&chat.proposals[0]).ask else {
        panic!("a follow-up");
    };
    assert_eq!(facts, "Reports to companion");
}

/// The filter's reason is outside text on either kind of card; a call's arguments one by one
/// change nothing drawn, since the card shows them whole.
#[test]
fn the_filters_reason_shows_on_either_card_and_the_arguments_one_by_one_change_nothing() {
    let mut listed = card_json("p1");
    listed["outside"]["args"] = json!([{"name": "repo", "value": "ops"},
        {"name": "title", "value": "Phoenix 2.0"}]);
    let reasoned = with(
        card_json("p1"),
        "outside",
        json!({"args_text": ARGS, "reason": "A Phoenix release.",
            "cue": {"type": "release", "text": "Phoenix 2.0 is out."}}),
    );
    let plain = card(&holding(vec![card_json("p1")]).proposals[0]);
    assert_eq!(plain.reason, None);
    assert_eq!(card(&holding(vec![listed]).proposals[0]), plain);
    let with_reason = card(&holding(vec![reasoned]).proposals[0]);
    assert_eq!(with_reason.reason.as_deref(), Some("A Phoenix release."));
    let follow_up = card(&holding(vec![follow_up_json("cft_1")]).proposals[0]);
    assert_eq!(
        follow_up.reason.as_deref(),
        Some("The post says usage limits reset.")
    );
}

/// The buttons, left to right: a call's Deny and Confirm; a follow-up's Skip and Run, with Always
/// allow before them only where its card offers it. The answer that runs is last.
#[test]
fn a_follow_up_answers_always_allow_skip_and_run_and_a_call_deny_and_confirm() {
    let mut once = follow_up_json("cft_2");
    once["follow_up"]["offers_always_allow"] = json!(false);
    let mut unsaid = follow_up_json("cft_3");
    unsaid["follow_up"]
        .as_object_mut()
        .unwrap()
        .remove("offers_always_allow");
    let chat = holding(vec![card_json("p1"), follow_up_json("cft_1"), once, unsaid]);
    let answers: Vec<_> = chat
        .proposals
        .iter()
        .map(|held| card(held).answers)
        .collect();
    let skip_run = vec![(SKIP, Decision::Deny), (RUN, Decision::Confirm)];
    assert_eq!(
        answers,
        [
            vec![(DENY, Decision::Deny), (CONFIRM, Decision::Confirm)],
            vec![
                (ALWAYS_ALLOW, Decision::AlwaysAllow),
                (SKIP, Decision::Deny),
                (RUN, Decision::Confirm)
            ],
            skip_run.clone(),
            skip_run,
        ]
    );
}

#[test]
fn a_state_this_build_does_not_know_keeps_its_word() {
    let ServerEvent::Proposal { proposal } =
        proposal(with(card_json("p1"), "state", json!("held")))
    else {
        panic!("a proposal");
    };
    assert_eq!(proposal.state, ProposalState::Unknown("held".into()));
}

/// §6 rule 2: the app confirms exactly the recorded call: the id and the word, never arguments.
#[test]
fn an_answer_carries_the_request_the_profile_the_id_and_the_word_alone() {
    for (decision, word) in [(Decision::Confirm, "confirm"), (Decision::Deny, "deny")] {
        let line = ClientEvent::proposal_answer("linux-1", "p1", decision)
            .line()
            .unwrap();
        let written: Value = serde_json::from_str(&line).unwrap();
        let expected = json!({"type": "proposal_answer", "client_msg_id": "linux-1",
            "profile_id": "main", "proposal_id": "p1", "decision": word});
        assert_eq!(written, expected);
    }
}

// The timeline.

#[test]
fn a_card_is_held_once_by_its_id_and_one_sent_again_replaces_it() {
    let mut chat = holding(vec![card_json("p1"), card_json("p2")]);
    let again = with(card_json("p1"), "summary", json!("Sent again."));
    timeline::receive(&mut chat, proposal(again), NOON);
    assert_eq!(ids(&chat), ["p1", "p2"]);
    assert_eq!(chat.proposals[0].proposal.summary, "Sent again.");
    assert!(chat.proposals.iter().all(|held| held.asks()));
}

#[test]
fn one_answer_goes_on_the_wire_and_both_buttons_wait() {
    let mut chat = holding(vec![card_json("p1")]);
    let sent = answer(&mut chat, "p1", Decision::Confirm, "linux-1");
    assert_eq!(
        sent,
        [ClientEvent::proposal_answer(
            "linux-1",
            "p1",
            Decision::Confirm
        )]
    );
    assert_eq!(standing(&chat, "p1"), Standing::Answered(Decision::Confirm));
    assert_eq!(
        drawn_cards(&chat),
        [("p1".to_owned(), Footer::Answered { running: true })]
    );
    assert!(answer(&mut chat, "p1", Decision::Deny, "linux-2").is_empty());
    assert_eq!(chat.pending.len(), 1, "a card is never answered twice");
}

#[test]
fn a_deny_or_a_skip_waits_without_saying_it_runs() {
    let mut chat = holding(vec![card_json("p1"), follow_up_json("cft_1")]);
    answer(&mut chat, "p1", Decision::Deny, "linux-1");
    answer(&mut chat, "cft_1", Decision::Deny, "linux-2");
    assert_eq!(
        drawn_cards(&chat),
        [
            ("p1".to_owned(), Footer::Answered { running: false }),
            ("cft_1".to_owned(), Footer::Answered { running: false })
        ]
    );
}

/// Always allow runs the follow-up as Run does, so the card says it is running, and every button
/// waits; it goes on the wire as the id and the word alone.
#[test]
fn always_allow_goes_on_the_wire_runs_and_every_button_waits() {
    let mut chat = holding(vec![follow_up_json("cft_1")]);
    let sent = answer(&mut chat, "cft_1", Decision::AlwaysAllow, "linux-1");
    assert_eq!(
        sent,
        [ClientEvent::proposal_answer(
            "linux-1",
            "cft_1",
            Decision::AlwaysAllow
        )]
    );
    let line: Value = serde_json::from_str(&sent[0].line().unwrap()).unwrap();
    assert_eq!(line["decision"], "always_allow");
    assert_eq!(
        drawn_cards(&chat),
        [("cft_1".to_owned(), Footer::Answered { running: true })]
    );
    for decision in [Decision::Confirm, Decision::Deny, Decision::AlwaysAllow] {
        assert!(answer(&mut chat, "cft_1", decision, "linux-2").is_empty());
    }
    assert_eq!(chat.pending.len(), 1, "a card is never answered twice");
}

/// A card takes Always allow only where it offers it (PROTOCOL.md "Version 5"): a call's card
/// would refuse it, and a follow-up that does not offer it never said it may. Nothing is sent,
/// and the card still asks.
#[test]
fn always_allow_on_a_card_that_does_not_offer_it_sends_nothing() {
    let mut once = follow_up_json("cft_1");
    once["follow_up"]["offers_always_allow"] = json!(false);
    let mut chat = holding(vec![card_json("p1"), once]);
    for id in ["p1", "cft_1"] {
        let sent = answer(&mut chat, id, Decision::AlwaysAllow, "linux-1");
        assert!(sent.is_empty(), "{id} takes no Always allow");
        assert_eq!(standing(&chat, id), Standing::Open);
    }
    assert!(chat.pending.is_empty());
    assert!(chat.proposals.iter().all(|held| held.asks()));
}

#[test]
fn only_a_waiting_card_draws_buttons_and_takes_an_answer() {
    let mut chat = holding(vec![
        with(card_json("p1"), "state", json!("confirmed")),
        with(card_json("p2"), "state", json!("withdrawn")),
    ]);
    for id in ["p1", "p2", "unheld"] {
        assert!(answer(&mut chat, id, Decision::Confirm, "linux-1").is_empty());
    }
    assert!(chat.pending.is_empty());
    let status = "Waiting for your answer; it runs only if you confirm.";
    let footers: Vec<Footer> = drawn_cards(&chat).into_iter().map(|(_, f)| f).collect();
    assert_eq!(
        footers,
        [Footer::Status(status.into()), Footer::Status(status.into())]
    );
}

#[test]
fn accepted_clears_the_outbox_and_the_card_stays_answered() {
    let mut chat = holding(vec![card_json("p1")]);
    answer(&mut chat, "p1", Decision::Confirm, "linux-1");
    timeline::receive(&mut chat, accepted("linux-1"), NOON);
    assert!(chat.pending.is_empty());
    assert_eq!(standing(&chat, "p1"), Standing::Answered(Decision::Confirm));
}

#[test]
fn a_resolution_replaces_the_buttons_and_one_for_an_unknown_card_changes_nothing() {
    let mut chat = holding(vec![card_json("p1"), card_json("p2")]);
    timeline::receive(
        &mut chat,
        resolved("elsewhere", "denied", "Denied.", None),
        NOON,
    );
    assert_eq!(ids(&chat), ["p1", "p2"]);
    assert!(chat.proposals.iter().all(|held| held.asks()));
    let ran = "Confirmed. github_create_issue ran.";
    timeline::receive(
        &mut chat,
        resolved("p1", "confirmed", ran, Some("{\"number\":42}")),
        NOON,
    );
    assert_eq!(
        drawn_cards(&chat)[0].1,
        Footer::Resolved {
            sentence: ran.into(),
            result: Some("{\"number\":42}".into())
        }
    );
    assert_eq!(drawn_cards(&chat)[1].1, Footer::Ask);
}

#[test]
fn a_refused_answer_resolves_its_card_in_the_daemons_sentence_and_is_no_chat_failure() {
    let mut chat = holding(vec![card_json("p1")]);
    answer(&mut chat, "p1", Decision::Confirm, "linux-1");
    let sentence = "That call was already confirmed; it runs at most once.";
    timeline::receive(&mut chat, refused("linux-1", sentence), NOON);
    assert!(chat.pending.is_empty());
    assert_eq!(chat.last_error, None);
    assert_eq!(
        standing(&chat, "p1"),
        Standing::Refused {
            sentence: sentence.into()
        }
    );
    assert_eq!(
        drawn_cards(&chat),
        [(
            "p1".to_owned(),
            Footer::Resolved {
                sentence: sentence.into(),
                result: None
            }
        )]
    );
}

/// A refusal can leave the proposal waiting (engine `cues/proposals.ex`: stale, disabled, a read
/// that failed), so the daemon's own word wins: a card it sends again takes the refused card's
/// place and asks again, here and after a handshake. A card `proposal_resolved` closed stays a
/// record whatever comes after.
#[test]
fn a_card_refused_here_asks_again_when_the_daemon_sends_it_and_a_resolved_one_stays_a_record() {
    let mut chat = holding(vec![card_json("refused"), card_json("resolved")]);
    answer(&mut chat, "refused", Decision::Confirm, "linux-1");
    let sentence = "The task changed since this was proposed; nothing ran.";
    timeline::receive(&mut chat, refused("linux-1", sentence), NOON);
    timeline::receive(
        &mut chat,
        resolved("resolved", "denied", "Denied.", None),
        NOON,
    );
    timeline::receive(&mut chat, proposal(card_json("refused")), NOON);
    timeline::receive(&mut chat, proposal(card_json("resolved")), NOON);
    assert_eq!(standing(&chat, "refused"), Standing::Open);
    assert!(chat.proposals[0].asks());
    let record = Footer::Resolved {
        sentence: "Denied.".into(),
        result: None,
    };
    assert_eq!(
        drawn_cards(&chat),
        [
            ("refused".to_owned(), Footer::Ask),
            ("resolved".to_owned(), record.clone())
        ]
    );

    answer(&mut chat, "refused", Decision::Deny, "linux-2");
    timeline::receive(&mut chat, refused("linux-2", sentence), NOON);
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(
        ids(&chat),
        ["refused", "resolved"],
        "a refusal is kept over a handshake until the daemon speaks"
    );
    timeline::receive(&mut chat, proposal(card_json("refused")), NOON);
    timeline::receive(&mut chat, proposal(card_json("resolved")), NOON);
    assert_eq!(
        drawn_cards(&chat),
        [
            ("refused".to_owned(), Footer::Ask),
            ("resolved".to_owned(), record)
        ]
    );
}

#[test]
fn a_handshake_drops_the_cards_not_resolved_and_holds_the_resend_once() {
    let mut chat = holding(vec![
        card_json("open"),
        card_json("answered"),
        with(
            card_json("expired"),
            "expires_at",
            json!("2026-10-22T13:00:00Z"),
        ),
        card_json("resolved"),
    ]);
    answer(&mut chat, "answered", Decision::Deny, "linux-1");
    timeline::receive(&mut chat, accepted("linux-1"), NOON);
    timeline::receive(
        &mut chat,
        resolved("resolved", "denied", "Denied.", None),
        NOON,
    );
    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(ids(&chat), ["resolved"]);
    timeline::receive(&mut chat, proposal(card_json("open")), NOON);
    timeline::receive(&mut chat, proposal(card_json("open")), NOON);
    assert_eq!(ids(&chat), ["resolved", "open"]);
    assert_eq!(standing(&chat, "open"), Standing::Open);
}

#[test]
fn a_card_sent_again_while_its_answer_waits_in_the_outbox_stays_answered() {
    let mut chat = holding(vec![card_json("p1")]);
    timeline::disconnected(&mut chat);
    assert!(
        answer(&mut chat, "p1", Decision::Confirm, "linux-1").is_empty(),
        "nothing is sent while disconnected"
    );
    let resent = timeline::negotiated(&mut chat, &BTreeSet::new());
    let sent = ClientEvent::proposal_answer("linux-1", "p1", Decision::Confirm);
    assert!(resent.contains(&sent), "{resent:?}");
    timeline::receive(&mut chat, proposal(card_json("p1")), NOON);
    assert_eq!(standing(&chat, "p1"), Standing::Answered(Decision::Confirm));
    assert!(answer(&mut chat, "p1", Decision::Confirm, "linux-2").is_empty());
}

#[test]
fn a_card_expires_on_this_clock_and_only_one_nobody_answered() {
    let mut chat = holding(vec![
        card_json("open"),
        card_json("answered"),
        with(
            card_json("later"),
            "expires_at",
            json!("2026-10-22T15:00:00Z"),
        ),
    ]);
    answer(&mut chat, "answered", Decision::Confirm, "linux-1");
    let half_past = NOON + 30 * 60;
    timeline::expire_proposals(&mut chat, half_past - 1);
    assert_eq!(standing(&chat, "open"), Standing::Open);
    timeline::expire_proposals(&mut chat, half_past);
    assert_eq!(standing(&chat, "open"), Standing::Expired);
    assert_eq!(
        standing(&chat, "answered"),
        Standing::Answered(Decision::Confirm)
    );
    assert_eq!(standing(&chat, "later"), Standing::Open);
    assert!(answer(&mut chat, "open", Decision::Confirm, "linux-2").is_empty());
    assert_eq!(drawn_cards(&chat)[0].1, Footer::Expired);
    let ended = "That proposal expired; nothing ran.";
    timeline::receive(
        &mut chat,
        resolved("open", "expired", ended, None),
        half_past,
    );
    assert!(matches!(standing(&chat, "open"), Standing::Resolved { .. }));
}

#[test]
fn a_card_can_arrive_expired_and_one_whose_expiry_cannot_be_read_never_expires_here() {
    let past = with(
        card_json("past"),
        "expires_at",
        json!("2026-10-22T13:00:00Z"),
    );
    let unreadable = with(card_json("unreadable"), "expires_at", json!("soon"));
    let mut chat = holding(vec![past, unreadable]);
    assert_eq!(standing(&chat, "past"), Standing::Expired);
    timeline::expire_proposals(&mut chat, i64::MAX);
    assert_eq!(standing(&chat, "unreadable"), Standing::Open);
}

#[test]
fn the_one_timer_waits_for_the_soonest_expiry_of_the_cards_that_ask() {
    let mut chat = holding(vec![
        with(
            card_json("late"),
            "expires_at",
            json!("2026-10-22T15:00:00Z"),
        ),
        card_json("soon"),
        with(card_json("unreadable"), "expires_at", json!("soon")),
    ]);
    assert_eq!(timeline::next_expiry(&chat), Some(NOON + 30 * 60));
    answer(&mut chat, "soon", Decision::Deny, "linux-1");
    assert_eq!(timeline::next_expiry(&chat), Some(NOON + 60 * 60));
    answer(&mut chat, "late", Decision::Deny, "linux-2");
    assert_eq!(timeline::next_expiry(&chat), None);
}

#[test]
fn a_reveal_is_published_once_and_taken_back_when_served() {
    let mut chat = holding(vec![card_json("p1")]);
    timeline::reveal(&mut chat, "p1");
    assert_eq!(chat.reveal.as_deref(), Some("p1"));
    timeline::revealed(&mut chat, "p2");
    assert_eq!(
        chat.reveal.as_deref(),
        Some("p1"),
        "another card's serving takes nothing back"
    );
    timeline::revealed(&mut chat, "p1");
    assert_eq!(chat.reveal, None);
}

// The view.

#[test]
fn cards_stand_among_the_messages_by_when_each_was_made_and_last_when_that_cannot_be_read() {
    let mut chat = Chat::default();
    timeline::negotiated(&mut chat, &BTreeSet::new());
    let message = |seq: u64, role: &str, ts: &str| {
        json!({"server_seq": seq, "role": role, "content": format!("row {seq}"), "ts": ts,
            "media_refs": []})
    };
    let page = json!({"type": "history_page", "profile_id": "main", "history_head_seq": 2,
        "messages": [message(1, "user", "2026-10-22T13:00:00Z"),
                     message(2, "assistant", "2026-10-22T15:00:00Z")]});
    timeline::receive(&mut chat, event(page), NOON);
    let unreadable = with(card_json("unreadable"), "created_at", json!("today"));
    let late = with(
        card_json("late"),
        "created_at",
        json!("2026-10-22T16:00:00Z"),
    );
    for card in [unreadable, late, card_json("between")] {
        timeline::receive(&mut chat, proposal(card), NOON);
    }
    let keys = view(&chat).keys;
    let proposal_key = |id: &str| Key::Proposal(id.into());
    assert_eq!(
        keys,
        [
            Key::Row(1),
            proposal_key("between"),
            Key::Row(2),
            proposal_key("late"),
            proposal_key("unreadable"),
        ]
    );
}

#[test]
fn a_card_stands_apart_right_after_the_owners_own_message() {
    let mut chat = holding(vec![]);
    let row = json!({"type": "history_page", "profile_id": "main", "history_head_seq": 1,
        "messages": [{"server_seq": 1, "role": "user", "content": "Watch the releases",
            "ts": "2026-10-22T13:59:30Z", "media_refs": []}]});
    timeline::receive(&mut chat, event(row), NOON);
    timeline::receive(&mut chat, proposal(card_json("p1")), NOON);
    let shown = view(&chat);
    assert_eq!(shown.keys, [Key::Row(1), Key::Proposal("p1".into())]);
    assert!(matches!(shown.entries[1], Entry::Proposal(_)));
    assert_eq!(shown.times[1], None, "a card carries no time of a message");
    assert_eq!(shown.seqs[1], None, "a card is no held row");
}

#[test]
fn a_streamed_reply_stands_where_it_arrived_among_the_cards() {
    let mut chat = holding(vec![card_json("p1")]);
    let start = json!({"type": "turn_started", "profile_id": "main", "turn_id": "t1",
        "in_reply_to": "linux-9"});
    timeline::receive(&mut chat, event(start), NOON);
    let done = json!({"type": "text_done", "turn_id": "t1", "server_seq": 1, "text": "Done."});
    timeline::receive(&mut chat, event(done), NOON + 60);
    assert_eq!(view(&chat).keys[0], Key::Proposal("p1".into()));
    let early = with(card_json("p2"), "created_at", json!("2026-10-22T14:05:00Z"));
    timeline::receive(&mut chat, proposal(early), NOON + 300);
    assert_eq!(
        view(&chat).keys,
        [
            Key::Proposal("p1".into()),
            Key::Reply("t1".into(), 0),
            Key::Proposal("p2".into()),
        ]
    );
}

#[test]
fn an_answer_is_never_drawn_as_a_message() {
    let mut chat = holding(vec![card_json("p1")]);
    timeline::disconnected(&mut chat);
    answer(&mut chat, "p1", Decision::Confirm, "linux-1");
    assert!(view(&chat)
        .entries
        .iter()
        .all(|entry| !matches!(entry, Entry::Sending(_))));
}

#[test]
fn the_card_says_its_task_watcher_expiry_and_the_items_source() {
    let chat = holding(vec![card_json("p1")]);
    let shown = card(&chat.proposals[0]);
    assert_eq!(
        facts(&shown, clock),
        "Task: Release watch · Watcher: release-watch · Waits until 14:30"
    );
    let item = shown.item.as_ref().expect("the card keeps its item");
    assert_eq!(item_header(item, clock), "From eng-blogs, received 13:59");
    assert_eq!(item.facts, ["Type: release"], "a missing field is left out");
    assert_eq!(item.text, "Phoenix 2.0 is out.");
    assert_eq!(shown.ask, CardAsk::Call { call: ARGS.into() });
}

#[test]
fn a_card_with_no_task_name_names_its_job_and_an_item_with_no_source_says_so() {
    let mut bare = card_json("p1");
    bare["task"] = json!({"job_id": "job_7f3a"});
    bare.as_object_mut().unwrap().remove("watcher");
    bare.as_object_mut().unwrap().remove("source_ref");
    let chat = holding(vec![bare]);
    let shown = card(&chat.proposals[0]);
    let unreadable = |_: &str| None;
    assert_eq!(
        facts(&shown, unreadable),
        "Task: job_7f3a · Waits until 2026-10-22T14:30:00Z"
    );
    let item = shown.item.expect("the cue is kept");
    assert_eq!(item_header(&item, clock), proposals::ITEM_UNSOURCED);
}

#[test]
fn a_screen_reader_hears_the_expiry_as_the_card_appears_and_a_minute_before_it() {
    let chat = holding(vec![card_json("p1")]);
    let shown = card(&chat.proposals[0]);
    assert_eq!(
        proposals::arrival(&shown, clock),
        "A task wants to act, Waits until 14:30"
    );
    assert_eq!(proposals::last_minute_in(&shown, NOON), Some(29 * 60));
    assert_eq!(proposals::last_minute_in(&shown, NOON + 29 * 60), None);
    let unreadable = with(card_json("p2"), "expires_at", json!("soon"));
    let chat = holding(vec![unreadable]);
    assert_eq!(
        proposals::last_minute_in(&card(&chat.proposals[0]), NOON),
        None
    );
}

#[test]
fn a_card_whose_cue_is_gone_has_no_item() {
    let mut card_json = card_json("p1");
    card_json["outside"] = json!({"args_text": ARGS});
    let chat = holding(vec![card_json]);
    assert_eq!(card(&chat.proposals[0]).item, None);
    let item = Item {
        source: None,
        received_at: None,
        facts: Vec::new(),
        text: String::new(),
    };
    assert_eq!(proposals::joined_facts(&item), None);
}

// Outside text.

#[test]
fn outside_text_comes_back_plain_however_it_is_written() {
    let text = "See [the notes](https://evil.example/x), https://bare.example, \
                <b>bold</b> <a href=\"https://x\">x</a> &amp; call +1 555 0100 **now**";
    let shown = outside(text);
    assert_eq!(shown.text, text);
    assert!(!shown.cut);
}

#[test]
fn outside_text_is_cut_at_twelve_lines_with_show_all() {
    let twelve: Vec<String> = (1..=OUTSIDE_LINES).map(|n| format!("line {n}")).collect();
    let whole = outside(&twelve.join("\n"));
    assert!(!whole.cut);
    let thirteen = format!("{}\nline 13", twelve.join("\n"));
    let cut = outside(&thirteen);
    assert!(cut.cut);
    assert_eq!(cut.text, format!("{}…", twelve.join("\n")));
    let long = "word ".repeat(400);
    let wrapped = outside(&long);
    assert!(wrapped.cut, "a long line counts as the lines it wraps to");
    assert!(wrapped.text.chars().count() <= OUTSIDE_LINES * proposals::WRAP_COLUMNS + 1);
}

/// Every file under the app's sources, by its path there, with its text.
fn app_sources() -> Vec<(String, String)> {
    let app = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../app/src"));
    let mut sources = Vec::new();
    let mut waiting = vec![app.to_path_buf()];
    while let Some(dir) = waiting.pop() {
        for entry in std::fs::read_dir(&dir).expect("the app's sources") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                waiting.push(path);
                continue;
            }
            let name = path
                .strip_prefix(app)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            sources.push((name, std::fs::read_to_string(&path).unwrap_or_default()));
        }
    }
    sources
}

/// §6 rule 1: only the card reads a proposal's outside text, and only through the outside-text
/// widget; no tray line, toast, title or notification can carry it. Read from the app's sources.
#[test]
fn only_the_card_reads_outside_text_and_never_through_markdown() {
    let readers = ["chat/proposals.rs", "chat/outside_text.rs"];
    let reads = [
        "args_text",
        "result_text",
        "CardAsk::Call",
        "card.reason",
        "item.text",
        "Footer::Resolved",
    ];
    let renders = ["markdown::", "blocks::", "set_markup", "use_markup(true)"];
    let sources = app_sources();
    let mut found = Vec::new();
    for (name, source) in &sources {
        let scanned = if readers.contains(&name.as_str()) {
            &renders[..]
        } else {
            &reads[..]
        };
        found.extend(
            scanned
                .iter()
                .filter(|token| source.contains(*token))
                .map(|token| format!("{name}: {token}")),
        );
    }
    assert!(found.is_empty(), "outside text read elsewhere: {found:?}");
    let core = include_str!("../src/chat_wire/proposals.rs");
    assert!(
        !core.contains("crate::markdown"),
        "outside text is prepared plain"
    );

    // The positive control: the scan reads both readers, the card holds the reads the scan looks
    // for everywhere else, each of which names a field core really has (of a struct, or of an
    // enum's variant), and the widget the card hands the text to holds the plain calls whose
    // markup forms the scan looks for in it.
    let source = |wanted: &str| {
        let held = sources.iter().find(|(name, _)| name == wanted);
        held.map(|(_, source)| source.as_str())
            .unwrap_or_else(|| panic!("{wanted} is among the app's sources"))
    };
    let card = source("chat/proposals.rs");
    assert!(
        reads.iter().any(|token| card.contains(token)),
        "the card holds none of {reads:?}"
    );
    let protocol = include_str!("../src/chat_wire/protocol.rs");
    for token in reads {
        let field = token
            .rsplit(['.', ':'])
            .next()
            .expect("a token names its field");
        assert!(
            card.contains(token) || protocol.contains(&format!(" {field}: ")),
            "{token} is read nowhere and is no field of the wire"
        );
    }
    let widget = source("chat/outside_text.rs");
    assert!(widget.contains("set_use_markup(false)") && widget.contains("set_text("));
}

// Copy.

#[test]
fn every_card_sentence_is_in_sentence_case_with_no_mac_and_no_wire_token() {
    for word in [REASON, SKIP, RUN, ALWAYS_ALLOW] {
        assert!(STRINGS.contains(&word), "{word} is checked");
    }
    for text in STRINGS {
        let first = text.chars().next().expect("not empty");
        assert!(first.is_uppercase(), "{text}");
        assert!(!text.contains("Mac"), "{text}");
        assert!(!text.contains('_'), "{text}");
    }
}

/// The happy path through every state of a confirmed proposal, as the page would walk it.
#[test]
fn a_proposal_is_shown_answered_accepted_and_resolved_end_to_end() {
    let mut chat = Chat::default();
    let hello = timeline::negotiated(&mut chat, &BTreeSet::new());
    assert!(hello
        .iter()
        .all(|event| !matches!(event, ClientEvent::ProposalAnswer { .. })));
    timeline::receive(&mut chat, proposal(card_json("p1")), NOON);
    assert_eq!(drawn_cards(&chat), [("p1".to_owned(), Footer::Ask)]);
    assert_eq!(timeline::next_expiry(&chat), Some(NOON + 30 * 60));

    let sent = answer(&mut chat, "p1", Decision::Confirm, "linux-1");
    assert_eq!(sent.len(), 1);
    assert_eq!(
        drawn_cards(&chat),
        [("p1".to_owned(), Footer::Answered { running: true })]
    );
    assert_eq!(
        timeline::next_expiry(&chat),
        None,
        "an answered card's timer stops"
    );

    timeline::receive(&mut chat, accepted("linux-1"), NOON + 5);
    assert!(chat.pending.is_empty());

    let ran = "Confirmed. github_create_issue ran.";
    timeline::receive(
        &mut chat,
        resolved("p1", "confirmed", ran, Some("{\"number\":42}")),
        NOON + 9,
    );
    let done = Footer::Resolved {
        sentence: ran.into(),
        result: Some("{\"number\":42}".into()),
    };
    assert_eq!(drawn_cards(&chat), [("p1".to_owned(), done.clone())]);

    timeline::disconnected(&mut chat);
    timeline::negotiated(&mut chat, &BTreeSet::new());
    assert_eq!(
        drawn_cards(&chat),
        [("p1".to_owned(), done)],
        "a resolved card stays a record"
    );
}
