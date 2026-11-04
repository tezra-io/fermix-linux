//! Recently removed and Restore (M85 lists §3.4, §3.7, §7, §10): Recently removed is each list's
//! own, off at start and kept while the app runs; turned on, the list is pulled with its removed
//! rows, turned off it pulls nothing, and with no item removed it draws nothing and stays checked.
//! Restore sends the row's restore and is off while it waits, and so is the removed list's Undo,
//! until the engine pushes the list or refuses the restore. A Restore takes back that row's removal
//! made here, so its toast's Undo sends nothing more and Ctrl+Z skips it. Nothing here opens a
//! socket: the library returns what to send.

use fermix_client::chat_wire::protocol::{read_line, ClientEvent, ListOp, ServerEvent};
use fermix_client::checklists::library::Library;
use fermix_client::checklists::pane::{
    drawing, list_offers, Folds, PaneBody, PaneDrawing, PaneList,
};
use fermix_client::checklists::panel::Line;
use fermix_client::checklists::removals::{self, Removals};
use fermix_client::checklists::routing::lists_event;
use serde_json::{json, Value};
use std::collections::BTreeSet;

const GROCERIES: &str = "ck_4q8m2x7v1d";
const PACKING: &str = "ck_2b7n5r8c3j";
const LEMONS: &str = "ci_0r2y8e6u3g";
const MILK: &str = "ci_7t1k4p9w2a";
const BASIL: &str = "ci_3h9d1v8r2k";
const SPICES: &str = "ci_5m2q8w1z7x";
const REMOVED_AT: &str = "2031-06-02T15:00:00.000000Z";
const GONE_ROW: &str = "That row is not in this list any more; pull the list again.";
const TAKEN: &str = "Another list is named \"Groceries\" now, so this one cannot come back under \
                     that name; restore it with a new title, or rename the other list first.";

fn event(line: Value) -> ServerEvent {
    read_line(line.to_string().as_bytes()).expect("a framed line")
}

/// Feeds the daemon's line to the lists, as the chat session does, and returns what they send.
fn receive(lists: &mut Library, line: Value) -> Vec<ClientEvent> {
    lists_event(lists, &event(line)).expect("the lists take it")
}

fn index() -> Value {
    let entries: Vec<Value> = [(GROCERIES, "Groceries"), (PACKING, "Packing")]
        .into_iter()
        .map(|(id, title)| json!({"list_id": id, "title": title, "open": 2, "done": 0, "rev": 1}))
        .collect();
    json!({"type": "checklists", "profile_id": "main", "lists": entries})
}

fn accepted(client_msg_id: &str) -> Value {
    json!({"type": "accepted", "client_msg_id": client_msg_id, "duplicate": false})
}

fn refused(message: &str, client_msg_id: &str) -> Value {
    json!({"type": "error", "reason": "request_failed", "message": message,
        "client_msg_id": client_msg_id})
}

fn text_of(row_id: &str) -> &'static str {
    match row_id {
        LEMONS => "Lemons",
        MILK => "Milk",
        _ => "Basil",
    }
}

/// Groceries at `rev`: the items `rows`, then `removed` in Recently removed, as the engine sends
/// them to a pull that asked for removed rows, and removed itself when `removed_at` says.
fn groceries(rev: u64, rows: &[&str], removed: &[&str], removed_at: Option<&str>) -> Value {
    let item =
        |id: &&str| json!({"row_id": id, "kind": "item", "text": text_of(id), "done": false});
    let gone = |id: &&str| {
        json!({"row_id": id, "kind": "item", "text": text_of(id), "done": false,
            "removed_at": REMOVED_AT})
    };
    json!({"type": "checklist", "profile_id": "main", "list": {
        "list_id": GROCERIES, "title": "Groceries", "rev": rev, "removed_at": removed_at,
        "rows": rows.iter().map(item).collect::<Vec<_>>(),
        "removed": removed.iter().map(gone).collect::<Vec<_>>()}})
}

/// Connected, the index naming both lists, and Groceries pulled without its removed rows and
/// answered at rev 7: Lemons and Milk.
fn holding_groceries() -> Library {
    let mut lists = Library::default();
    lists.negotiated(7);
    receive(&mut lists, index());
    lists.draw(GROCERIES, false);
    receive(&mut lists, groceries(7, &[LEMONS, MILK], &[], None));
    lists
}

/// Groceries held with Recently removed on, pulled with `removed` in it.
fn showing_removed(rows: &[&str], removed: &[&str]) -> (Library, Folds) {
    let mut lists = holding_groceries();
    let mut folds = Folds::default();
    folds.show_removed(GROCERIES);
    lists.draw(GROCERIES, true);
    receive(&mut lists, groceries(7, rows, removed, None));
    (lists, folds)
}

/// The pane on Groceries, drawn by `folds`.
fn pane(lists: &Library, folds: &Folds) -> PaneList {
    match drawing(lists, Some(GROCERIES), folds, &BTreeSet::new()) {
        PaneDrawing::List(list) => *list,
        other => panic!("a list: {other:?}"),
    }
}

/// Recently removed as the pane draws it: under its heading, each removed row with whether its
/// restore waits. None while no heading is drawn.
fn recently_removed(list: &PaneList) -> Option<Vec<(String, bool)>> {
    let PaneBody::Lines(lines) = &list.body else {
        return None;
    };
    let at = lines
        .iter()
        .position(|line| *line == Line::RemovedHeading)?;
    let removed = lines[at + 1..].iter().filter_map(|line| match line {
        Line::Removed { row, restoring } => Some((row.row_id.clone(), *restoring)),
        _ => None,
    });
    Some(removed.collect())
}

fn row(row_id: &str, restoring: bool) -> (String, bool) {
    (row_id.to_owned(), restoring)
}

/// Each pull in `sent`, with whether it asks for the removed rows.
fn pulls(sent: &[ClientEvent]) -> Vec<(String, bool)> {
    sent.iter()
        .filter_map(|event| match event {
            ClientEvent::ChecklistPull {
                list_id, removed, ..
            } => Some((list_id.clone(), *removed == Some(true))),
            _ => None,
        })
        .collect()
}

/// Every restore in `sent`, as (list, row) with `-` for a list's own.
fn restores(sent: &[ClientEvent]) -> Vec<String> {
    sent.iter()
        .filter_map(|event| match event {
            ClientEvent::ChecklistEdit {
                list_id,
                op: ListOp::Restore { row_id, .. },
                ..
            } => {
                let row = row_id.as_deref().unwrap_or("-");
                Some(format!("{}:{row}", list_id.as_deref().unwrap_or("?")))
            }
            _ => None,
        })
        .collect()
}

/// Recently removed is each list's own, off at start (§7). Turned on, the list is pulled with its
/// removed rows, which it draws under the heading and More's menu checks; turned off, it pulls
/// nothing and draws none; with nothing removed it draws nothing and stays checked (§10).
#[test]
fn recently_removed_is_each_lists_own_and_turned_on_pulls_the_removed_rows() {
    let mut lists = holding_groceries();
    let mut folds = Folds::default();
    assert!(!folds.removed_shown(GROCERIES), "off at start");
    assert_eq!(recently_removed(&pane(&lists, &folds)), None);

    folds.show_removed(GROCERIES);
    assert!(!folds.removed_shown(PACKING), "each list its own");
    assert!(!folds.view(PACKING, &BTreeSet::new()).removed_shown);
    let sent = lists.draw(GROCERIES, folds.removed_shown(GROCERIES));
    assert_eq!(pulls(&sent), [row(GROCERIES, true)]);
    receive(&mut lists, groceries(7, &[LEMONS, MILK], &[BASIL], None));
    let shown = pane(&lists, &folds);
    assert_eq!(recently_removed(&shown), Some(vec![row(BASIL, false)]));
    assert!(list_offers(&shown).is_some_and(|offers| offers.removed_shown));
    assert!(lists.draw(GROCERIES, true).is_empty(), "held with them");

    folds.show_removed(GROCERIES);
    let sent = lists.draw(GROCERIES, folds.removed_shown(GROCERIES));
    assert!(sent.is_empty(), "turned off, it pulls nothing");
    let hidden = pane(&lists, &folds);
    assert_eq!(recently_removed(&hidden), None);
    assert!(list_offers(&hidden).is_some_and(|offers| !offers.removed_shown));

    folds.show_removed(GROCERIES);
    receive(&mut lists, groceries(8, &[LEMONS, MILK, BASIL], &[], None));
    let none = pane(&lists, &folds);
    assert_eq!(
        recently_removed(&none),
        None,
        "nothing removed draws nothing"
    );
    let checked = list_offers(&none).is_some_and(|offers| offers.removed_shown);
    assert!(checked, "and stays checked");
}

/// A list whose removed rows hold only a section, which no item was in, draws no Recently
/// removed rather than its heading alone, and More's menu still checks it (§10).
#[test]
fn a_section_removed_alone_draws_no_recently_removed() {
    let mut lists = holding_groceries();
    let mut folds = Folds::default();
    folds.show_removed(GROCERIES);
    lists.draw(GROCERIES, true);
    let mut line = groceries(8, &[LEMONS, MILK], &[], None);
    line["list"]["removed"] = json!([{"row_id": SPICES, "kind": "section", "text": "Spices",
        "removed_at": REMOVED_AT}]);
    receive(&mut lists, line);
    let held = lists.shown(GROCERIES).map(|list| list.removed.len());
    assert_eq!(held, Some(1), "the section is held as removed");
    let shown = pane(&lists, &folds);
    assert_eq!(recently_removed(&shown), None, "no heading alone");
    assert!(list_offers(&shown).is_some_and(|offers| offers.removed_shown));
}

/// Restore sends the row's restore, and is off while it waits, accepted or not, until the engine
/// pushes the list, which draws the row back; refused, it is on again under the engine's sentence
/// (§10).
#[test]
fn restore_is_off_while_its_restore_waits_until_the_push_or_the_refusal() {
    let (mut lists, folds) = showing_removed(&[MILK], &[BASIL, LEMONS]);
    let mut removals = Removals::default();
    let restoring = |lists: &Library| recently_removed(&pane(lists, &folds));

    let (_, sent) = removals::restore(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(BASIL),
        "linux-1".into(),
    );
    assert_eq!(restores(&sent), [format!("{GROCERIES}:{BASIL}")]);
    let waiting = Some(vec![row(BASIL, true), row(LEMONS, false)]);
    assert_eq!(restoring(&lists), waiting);
    assert!(
        !pane(&lists, &folds).restoring,
        "the list's own restore is not waiting"
    );
    receive(&mut lists, accepted("linux-1"));
    assert_eq!(restoring(&lists), waiting, "accepted, until the push");
    receive(&mut lists, groceries(8, &[MILK, BASIL], &[LEMONS], None));
    assert_eq!(restoring(&lists), Some(vec![row(LEMONS, false)]));
    let PaneBody::Lines(lines) = pane(&lists, &folds).body else {
        panic!("Groceries' lines");
    };
    let back = lines
        .iter()
        .any(|line| matches!(line, Line::Item(row) if row.row_id == BASIL));
    assert!(back, "Basil is back among the items");

    removals::restore(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(LEMONS),
        "linux-2".into(),
    );
    assert_eq!(restoring(&lists), Some(vec![row(LEMONS, true)]));
    receive(&mut lists, refused(GONE_ROW, "linux-2"));
    assert_eq!(restoring(&lists), Some(vec![row(LEMONS, false)]));
    assert_eq!(pane(&lists, &folds).refusal.as_deref(), Some(GONE_ROW));
}

/// The removed list's Undo is off while the list's restore waits, sent by Ctrl+Z or by the page's
/// own Undo, until the engine pushes the list or refuses the restore (§10).
#[test]
fn the_removed_lists_undo_is_off_while_its_restore_waits() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let folds = Folds::default();
    let page = |lists: &Library| {
        let list = pane(lists, &folds);
        (list.body == PaneBody::Removed, list.restoring)
    };
    let removing = removals::remove(
        &mut removals,
        &mut lists,
        GROCERIES,
        None,
        "Groceries",
        "linux-1".into(),
    );
    assert!(removing.is_some());
    receive(&mut lists, accepted("linux-1"));
    receive(&mut lists, groceries(8, &[LEMONS], &[], Some(REMOVED_AT)));
    assert_eq!(page(&lists), (true, false), "its page, with Undo on");

    removals::undo(&mut removals, &mut lists, None, "linux-2".into()).expect("Ctrl+Z");
    assert_eq!(page(&lists), (true, true), "Ctrl+Z's restore waits");
    receive(&mut lists, accepted("linux-2"));
    assert_eq!(page(&lists), (true, true), "accepted, until the push");
    receive(&mut lists, groceries(9, &[LEMONS], &[], None));
    assert_eq!(page(&lists), (false, false), "back");

    receive(&mut lists, groceries(10, &[LEMONS], &[], Some(REMOVED_AT)));
    removals::restore(&mut removals, &mut lists, GROCERIES, None, "linux-3".into());
    assert_eq!(page(&lists), (true, true), "the page's own Undo waits");
    receive(&mut lists, refused(TAKEN, "linux-3"));
    assert_eq!(page(&lists), (true, false), "refused, Undo is on again");
}

/// Restore of a row removed here takes back that row's removal, not another's, so its toast's Undo
/// sends nothing and Ctrl+Z takes the removal before it; a row removed elsewhere is restored with
/// nothing taken back (§3.7, §10).
#[test]
fn restore_takes_back_the_rows_removal_made_here() {
    let mut lists = holding_groceries();
    let mut removals = Removals::default();
    let mut made = Vec::new();
    for (row_id, id) in [(MILK, "linux-1"), (LEMONS, "linux-2")] {
        let text = text_of(row_id);
        let removing = removals::remove(
            &mut removals,
            &mut lists,
            GROCERIES,
            Some(row_id),
            text,
            id.into(),
        );
        made.push(removing.expect("removed").0);
        receive(&mut lists, accepted(id));
    }
    receive(&mut lists, groceries(8, &[], &[LEMONS, MILK, BASIL], None));

    let (taken, sent) = removals::restore(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(LEMONS),
        "linux-3".into(),
    );
    assert_eq!(taken.map(|removal| removal.token), Some(made[1]));
    assert_eq!(restores(&sent), [format!("{GROCERIES}:{LEMONS}")]);
    let again = removals::undo(&mut removals, &mut lists, Some(made[1]), "linux-4".into());
    assert_eq!(again, None, "its toast's Undo sends nothing");

    let (none, sent) = removals::restore(
        &mut removals,
        &mut lists,
        GROCERIES,
        Some(BASIL),
        "linux-5".into(),
    );
    assert_eq!(none, None, "Basil was removed elsewhere");
    assert_eq!(restores(&sent), [format!("{GROCERIES}:{BASIL}")]);

    let (milk, sent) =
        removals::undo(&mut removals, &mut lists, None, "linux-6".into()).expect("Ctrl+Z");
    assert_eq!(
        milk.token, made[0],
        "Ctrl+Z takes Milk's removal, the one left"
    );
    assert_eq!(restores(&sent), [format!("{GROCERIES}:{MILK}")]);
}
