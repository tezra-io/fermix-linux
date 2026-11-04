//! Removal and Undo (M85 lists §3.7, §10; §8 "Undo"): a removal made here is kept for Undo under a
//! token, its toast's Undo and Ctrl+Z take it back once between them, Ctrl+Z reaches 20 back newest
//! first and keeps one removal of a row, a list taken back is restored without a row and shown, a
//! row not named yet or a 65th edit makes no removal, and a list's Undo refused, or an item's after
//! its list was removed elsewhere, says so under the head. Nothing here opens a socket: the
//! library returns what to send.

use fermix_client::chat_wire::protocol::{read_line, ClientEvent, ListOp, ServerEvent};
use fermix_client::checklists::draft;
use fermix_client::checklists::library::{Library, MAX_WAITING};
use fermix_client::checklists::pane::{drawing, Asked, Folds, PaneBody, PaneDrawing};
use fermix_client::checklists::removals::{self, Removals, MAX_KEPT};
use fermix_client::checklists::routing::lists_event;
use fermix_client::checklists::words::TOO_MANY;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const GROCERIES: &str = "ck_4q8m2x7v1d";
const PACKING: &str = "ck_2b7n5r8c3j";
const LEMONS: &str = "ci_0r2y8e6u3g";
const REMOVED_AT: &str = "2031-06-02T15:00:00.000000Z";
const BASIL: &str = "ci_3h9d1v8r2k";
const TAKEN: &str = "Another list is named \"Groceries\" now, so this one cannot come back under \
                     that name; restore it with a new title, or rename the other list first.";
const GONE: &str = "No list has that id; it may have been removed more than 30 days ago.";

fn event(line: Value) -> ServerEvent {
    read_line(line.to_string().as_bytes()).expect("a framed line")
}

/// Feeds the daemon's line to the lists, as the chat session does, and returns what they send.
fn receive(lists: &mut Library, line: Value) -> Vec<ClientEvent> {
    lists_event(lists, &event(line)).expect("the lists take it")
}

/// The index naming `ids` of Groceries and Packing, in no folder, in that order.
fn index(ids: &[&str]) -> Value {
    let entries: Vec<Value> = [(GROCERIES, "Groceries"), (PACKING, "Packing")]
        .into_iter()
        .filter(|(id, _)| ids.contains(id))
        .map(|(id, title)| json!({"list_id": id, "title": title, "open": 1, "done": 1, "rev": 1}))
        .collect();
    json!({"type": "checklists", "profile_id": "main", "lists": entries})
}

/// Groceries at `rev`: Lemons and a done Basil, removed when `removed_at` says.
fn groceries(rev: u64, removed_at: Option<&str>) -> Value {
    json!({"type": "checklist", "profile_id": "main", "list": {
    "list_id": GROCERIES, "title": "Groceries", "rev": rev, "removed_at": removed_at,
    "rows": [
        {"row_id": LEMONS, "kind": "item", "text": "Lemons", "done": false},
        {"row_id": BASIL, "kind": "item", "text": "Basil", "done": true}
    ]}})
}

fn accepted(client_msg_id: &str) -> Value {
    json!({"type": "accepted", "client_msg_id": client_msg_id, "duplicate": false})
}

fn refused(message: &str, client_msg_id: &str) -> Value {
    json!({"type": "error", "reason": "request_failed", "message": message,
        "client_msg_id": client_msg_id})
}

/// Connected, the index naming both lists, and Groceries pulled and answered at rev 7.
fn holding_groceries() -> Library {
    let mut lists = Library::default();
    lists.negotiated(7);
    receive(&mut lists, index(&[GROCERIES, PACKING]));
    lists.draw(GROCERIES, false);
    receive(&mut lists, groceries(7, None));
    lists
}

/// Every restore in `sent`, as (list, row) with `-` for a list's own.
fn restores(sent: &[ClientEvent]) -> Vec<String> {
    sent.iter()
        .filter_map(|event| match event {
            ClientEvent::ChecklistEdit {
                list_id,
                op: ListOp::Restore { row_id, title },
                ..
            } => {
                assert_eq!(*title, None, "taken back under its own title");
                let row = row_id.as_deref().unwrap_or("-");
                Some(format!("{}:{row}", list_id.as_deref().unwrap_or("?")))
            }
            _ => None,
        })
        .collect()
}

/// The list the pane draws for `asked`, and whether its lines are drawn.
fn pane_on(lists: &Library, asked: &mut Asked) -> (String, bool) {
    let drawn = drawing(
        lists,
        asked.now(lists.index()),
        &Folds::default(),
        &BTreeSet::new(),
    );
    match drawn {
        PaneDrawing::List(list) => (list.list_id, matches!(list.body, PaneBody::Lines(_))),
        other => panic!("a list: {other:?}"),
    }
}

/// A removal made here, which must be made: its token.
fn removed(
    removals: &mut Removals,
    lists: &mut Library,
    list_id: &str,
    row_id: Option<&str>,
    client_msg_id: &str,
) -> u64 {
    let text = row_id.unwrap_or(list_id);
    let made = removals::remove(removals, lists, list_id, row_id, text, client_msg_id.into());
    made.expect("a removal made").0
}

/// A removal made here is taken back once, by its toast's Undo or by Ctrl+Z, whichever comes
/// first: the second sends nothing, either way round, and so does the second Undo of a removed
/// list's page and its toast (§3.7, M `048260a`).
#[test]
fn the_toast_and_ctrl_z_take_a_removal_back_once_between_them() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let mut sent = Vec::new();

    let lemons = removed(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(LEMONS),
        "linux-1",
    );
    let toast = removals::undo(&mut removals, &mut lists, Some(lemons), "linux-2".into());
    sent.extend(toast.expect("the toast takes it back").1);
    let ctrl_z = removals::undo(&mut removals, &mut lists, None, "linux-3".into());
    assert_eq!(ctrl_z, None, "Ctrl+Z after the toast's Undo");
    assert_eq!(restores(&sent), [format!("{GROCERIES}:{LEMONS}")]);

    let basil = removed(&mut removals, &mut lists, GROCERIES, Some(BASIL), "linux-4");
    let ctrl_z = removals::undo(&mut removals, &mut lists, None, "linux-5".into());
    let (taken, out) = ctrl_z.expect("Ctrl+Z takes it back");
    assert_eq!(taken.token, basil);
    sent.extend(out);
    let toast = removals::undo(&mut removals, &mut lists, Some(basil), "linux-6".into());
    assert_eq!(toast, None, "the toast's Undo after Ctrl+Z");

    let list = removed(&mut removals, &mut lists, GROCERIES, None, "linux-7");
    let (taken, out) =
        removals::restore(&mut removals, &mut lists, GROCERIES, None, "linux-8".into());
    assert_eq!(taken.map(|removal| removal.token), Some(list));
    sent.extend(out);
    for (token, id) in [(Some(list), "linux-9"), (None, "linux-10")] {
        let again = removals::undo(&mut removals, &mut lists, token, id.into());
        assert_eq!(again, None, "after the page's Undo");
    }
    let (none, out) = removals::restore(
        &mut removals,
        &mut lists,
        GROCERIES,
        None,
        "linux-11".into(),
    );
    assert_eq!(none, None, "a second page Undo takes nothing back");
    assert_eq!(
        restores(&out),
        [format!("{GROCERIES}:-")],
        "and still restores"
    );

    assert_eq!(
        restores(&sent),
        [
            format!("{GROCERIES}:{LEMONS}"),
            format!("{GROCERIES}:{BASIL}"),
            format!("{GROCERIES}:-"),
        ]
    );
}

/// Ctrl+Z takes back the newest removal still kept, then the one before it, up to 20 back across
/// lists: the 21st press finds the oldest let go and sends nothing (§3.7, §10).
#[test]
fn ctrl_z_goes_up_to_twenty_back_newest_first_and_the_twenty_first_is_gone() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let rows: Vec<String> = (1..=21).map(|n| format!("ci_row{n:07}")).collect();
    for (n, row) in rows.iter().enumerate() {
        let list = if n % 2 == 0 { GROCERIES } else { PACKING };
        let made = removals::remove(
            &mut removals,
            &mut lists,
            list,
            Some(row),
            row,
            format!("linux-r{n}"),
        );
        assert!(made.is_some(), "{row} removed");
    }
    assert_eq!(MAX_KEPT, 20);

    let mut taken = Vec::new();
    for n in 0..21 {
        match removals::undo(&mut removals, &mut lists, None, format!("linux-u{n}")) {
            Some((removal, sent)) => {
                assert_eq!(restores(&sent).len(), 1);
                taken.push(removal.row_id.expect("a row"));
            }
            None => taken.push("nothing".into()),
        }
    }

    let mut expected: Vec<String> = rows[1..].iter().rev().cloned().collect();
    expected.push("nothing".into());
    assert_eq!(taken, expected);
}

/// Remove list sends `remove_list`, and the pane turns to the first list once the index drops it.
/// With that list picked since, Undo restores the removed one with no row and under its own title,
/// and the pane shows it once the index names it again; an item taken back leaves the pane where
/// it is (§3.7, §10).
#[test]
fn a_list_taken_back_is_restored_without_a_row_and_the_pane_shows_it() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let mut asked = Asked::default();
    asked.ask(GROCERIES);
    assert_eq!(pane_on(&lists, &mut asked), (GROCERIES.into(), true));

    let (_, sent) = removals::remove(
        &mut removals,
        &mut lists,
        GROCERIES,
        None,
        "Groceries",
        "linux-1".into(),
    )
    .expect("Groceries removed");
    let removing = sent.iter().any(|event| {
        matches!(event, ClientEvent::ChecklistEdit { list_id: Some(id), op: ListOp::RemoveList, .. }
            if id == GROCERIES)
    });
    assert!(removing, "remove_list sent: {sent:?}");
    receive(&mut lists, accepted("linux-1"));
    receive(&mut lists, groceries(8, Some(REMOVED_AT)));
    receive(&mut lists, index(&[PACKING]));
    assert_eq!(pane_on(&lists, &mut asked), (PACKING.into(), false));
    asked.ask(PACKING);

    let (removal, sent) = removals::undo(&mut removals, &mut lists, None, "linux-2".into())
        .expect("Ctrl+Z takes the list back");
    asked.taken_back(&removal);
    assert_eq!(restores(&sent), [format!("{GROCERIES}:-")]);
    assert_eq!(pane_on(&lists, &mut asked).0, PACKING, "until the index");
    receive(&mut lists, accepted("linux-2"));
    receive(&mut lists, groceries(9, None));
    receive(&mut lists, index(&[GROCERIES, PACKING]));
    assert_eq!(pane_on(&lists, &mut asked), (GROCERIES.into(), true));

    asked.ask(PACKING);
    let token = removed(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(LEMONS),
        "linux-3",
    );
    let (removal, _) = removals::undo(&mut removals, &mut lists, Some(token), "linux-4".into())
        .expect("its toast takes it back");
    asked.taken_back(&removal);
    assert_eq!(pane_on(&lists, &mut asked).0, PACKING, "not for an item");
}

/// A row added here that the engine has not named yet cannot be removed, and neither can anything
/// past 64 edits waiting: no removal is kept, so there is no token, no toast and nothing for
/// Ctrl+Z (§5.3).
#[test]
fn a_row_not_named_yet_or_a_sixty_fifth_edit_makes_no_removal_and_no_token() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    lists.edit(
        GROCERIES,
        ListOp::Add {
            text: "Milk".into(),
            note: None,
            section_id: None,
            after_id: None,
        },
        "linux-1".into(),
    );
    let sending = draft::local_row_id("linux-1");

    let made = removals::remove(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(&sending),
        "Milk",
        "linux-2".into(),
    );
    assert_eq!(made, None, "a sending row");
    assert_eq!(removals.newest(), None);

    lists.disconnected();
    for n in 1..MAX_WAITING {
        lists.edit(GROCERIES, check(LEMONS, n), format!("linux-w{n}"));
    }
    let made = removals::remove(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(BASIL),
        "Basil",
        "linux-3".into(),
    );
    assert_eq!(made, None, "the 65th edit");
    assert_eq!(removals.newest(), None);
    let state = lists.state(GROCERIES).expect("held");
    assert_eq!(state.refusal(), Some(TOO_MANY));
    assert_eq!(state.pending().len(), MAX_WAITING);
    assert_eq!(
        removals::undo(&mut removals, &mut lists, None, "linux-4".into()),
        None
    );
}

/// A tick or an untick of `row_id`, by `n`.
fn check(row_id: &str, n: usize) -> ListOp {
    match n % 2 {
        0 => ListOp::Check {
            row_id: row_id.into(),
        },
        _ => ListOp::Uncheck {
            row_id: row_id.into(),
        },
    }
}

/// A list's Undo refused, by the engine or past 64 edits waiting, says its sentence under the head
/// and not over the list's Add an item, the engine's until the next index, even while the index
/// still names the list and the pane shows its removed page (§10).
#[test]
fn a_lists_undo_refused_says_its_sentence_under_the_head_until_the_next_index() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let mut asked = Asked::default();
    removed(&mut removals, &mut lists, GROCERIES, None, "linux-1");
    receive(&mut lists, accepted("linux-1"));
    receive(&mut lists, groceries(8, Some(REMOVED_AT)));
    let (taken, _) =
        removals::restore(&mut removals, &mut lists, GROCERIES, None, "linux-2".into());
    assert!(taken.is_some(), "the page's Undo takes the removal back");

    receive(&mut lists, refused(TAKEN, "linux-2"));

    assert_eq!(lists.index().refusal(), Some(TAKEN));
    assert_eq!(lists.state(GROCERIES).expect("held").refusal(), None);
    let drawn = drawing(
        &lists,
        asked.now(lists.index()),
        &Folds::default(),
        &BTreeSet::new(),
    );
    let page = matches!(&drawn, PaneDrawing::List(list) if list.body == PaneBody::Removed);
    assert!(page, "the removed page: {drawn:?}");
    assert_eq!(drawn.index_refusal(), Some(TAKEN));
    receive(&mut lists, index(&[PACKING]));
    assert_eq!(lists.index().refusal(), None, "the next index");

    removed(&mut removals, &mut lists, PACKING, None, "linux-3");
    lists.disconnected();
    for n in 1..MAX_WAITING {
        lists.edit(GROCERIES, check(LEMONS, n), format!("linux-w{n}"));
    }
    let undone = removals::undo(&mut removals, &mut lists, None, "linux-4".into());
    assert!(undone.is_some_and(|(_, sent)| sent.is_empty()));
    assert_eq!(lists.index().refusal(), Some(TOO_MANY));
    assert_eq!(lists.state(PACKING).and_then(|state| state.refusal()), None);
}

/// A row or a list removed again after it came back some other way, by another app's Recently
/// removed, is kept once, by its newest removal: Ctrl+Z takes that back, and the next press reaches
/// the removal before it, not the older removal of the same row or list (§3.7).
#[test]
fn a_row_or_list_removed_again_is_kept_once_and_ctrl_z_reaches_the_removal_before() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let mut remove =
        |list_id, row_id, id: &str| removed(&mut removals, &mut lists, list_id, row_id, id);
    let basil = remove(GROCERIES, Some(BASIL), "linux-1");
    let lemons = remove(GROCERIES, Some(LEMONS), "linux-2");
    let packing = remove(PACKING, None, "linux-3");
    // Lemons and Packing come back elsewhere, and are removed here again.
    let lemons_again = remove(GROCERIES, Some(LEMONS), "linux-4");
    let packing_again = remove(PACKING, None, "linux-5");
    assert_ne!((lemons, packing), (lemons_again, packing_again));

    let mut taken = Vec::new();
    for n in 6..=9 {
        let undone = removals::undo(&mut removals, &mut lists, None, format!("linux-{n}"));
        taken.push(undone.map(|(removal, sent)| (removal.token, restores(&sent))));
    }

    assert_eq!(
        taken,
        [
            Some((packing_again, vec![format!("{PACKING}:-")])),
            Some((lemons_again, vec![format!("{GROCERIES}:{LEMONS}")])),
            Some((basil, vec![format!("{GROCERIES}:{BASIL}")])),
            None,
        ]
    );
}

/// An item's Undo refused after its list was removed elsewhere, by the engine or past 64 edits
/// waiting: the pane cannot draw that list, so its sentence is the index's, shown under the head,
/// and not the list's (§10).
#[test]
fn an_items_undo_refused_after_its_list_was_removed_says_its_sentence_under_the_head() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    removed(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(LEMONS),
        "linux-1",
    );
    removed(&mut removals, &mut lists, GROCERIES, Some(BASIL), "linux-2");
    receive(&mut lists, accepted("linux-1"));
    receive(&mut lists, accepted("linux-2"));
    receive(&mut lists, groceries(9, Some(REMOVED_AT)));
    receive(&mut lists, index(&[PACKING]));

    let (_, sent) = removals::undo(&mut removals, &mut lists, None, "linux-3".into())
        .expect("Ctrl+Z takes Basil back");
    assert_eq!(restores(&sent), [format!("{GROCERIES}:{BASIL}")]);
    receive(&mut lists, refused(GONE, "linux-3"));
    assert_eq!(lists.index().refusal(), Some(GONE));
    assert_eq!(lists.state(GROCERIES).expect("held").refusal(), None);
    assert!(lists.state(GROCERIES).expect("held").pending().is_empty());

    receive(&mut lists, index(&[PACKING]));
    lists.disconnected();
    for n in 0..MAX_WAITING {
        lists.edit(PACKING, check(LEMONS, n), format!("linux-w{n}"));
    }
    let undone = removals::undo(&mut removals, &mut lists, None, "linux-4".into());
    assert!(undone.is_some_and(|(_, sent)| sent.is_empty()));
    assert_eq!(lists.index().refusal(), Some(TOO_MANY));
    assert_eq!(lists.state(GROCERIES).expect("held").refusal(), None);
}
