//! One reply's lists card as it stands now (M85 lists §2.2 to §2.5): a live card waits for its
//! pull, then draws its rows; a removed list says so and is never pulled; a refused edit's
//! sentence shows under the rows; a card of several lists draws a head per group, a removed
//! list's head alone, and a refused edit's sentence under its list's rows; a ticked row keeps its
//! place until its last settle ends; a folded card counts from the index; only a live card
//! pulls when it comes on screen; and what a screen reader is told as a card is drawn again.

use fermix_client::chat::ListsCard;
use fermix_client::chat_wire::protocol::{ClientEvent, List, ListEntry, ListOp, ListRow};
use fermix_client::checklists::card::{drawing, news, Drawing, Head, ListGroup, Settling, Stands};
use fermix_client::checklists::cards::OneList;
use fermix_client::checklists::library::Library;
use fermix_client::checklists::words::LIST_REMOVED;
use fermix_client::checklists::{ListRef, ITEM};
use std::collections::{BTreeMap, BTreeSet};

const A: &str = "ck_aaaaaaaaaa";
const B: &str = "ck_bbbbbbbbbb";
const C: &str = "ck_cccccccccc";
const D: &str = "ck_dddddddddd";

fn list_ref(list_id: &str, rows: &[&str]) -> ListRef {
    ListRef {
        list_id: list_id.into(),
        title: format!("Block {list_id}"),
        rows: rows.iter().map(|row| row.to_string()).collect(),
    }
}

fn item(id: &str, done: bool) -> ListRow {
    ListRow {
        row_id: id.into(),
        kind: ITEM.into(),
        text: id.into(),
        note: None,
        done: Some(done),
        done_at: None,
    }
}

/// `list_id` as the engine sends it, titled "Engine" and its id, with `open` open items and
/// `done` done ones after them.
fn list(list_id: &str, open: usize, done: usize) -> List {
    let open = (0..open).map(|n| item(&format!("ci_open{n:06}"), false));
    let done = (0..done).map(|n| item(&format!("ci_done{n:06}"), true));
    List {
        list_id: list_id.into(),
        title: format!("Engine {list_id}"),
        rev: 1,
        rows: open.chain(done).collect(),
        removed: Vec::new(),
        removed_at: None,
    }
}

fn entry(list_id: &str, open: u64, done: u64) -> ListEntry {
    ListEntry {
        list_id: list_id.into(),
        title: format!("Index {list_id}"),
        folder: None,
        open,
        done,
        rev: 1,
    }
}

/// A library on a connection that serves lists, whose index names `named`.
fn library(named: &[ListEntry]) -> Library {
    let mut library = Library::default();
    library.negotiated(7);
    library.received_index(named.to_vec());
    library
}

fn draw(lists: &[ListRef], live: bool, library: &Library) -> Drawing {
    drawing(lists, live, library, false, &BTreeSet::new()).expect("a card")
}

fn head(list_id: &str, title: &str, stands: Stands) -> Head {
    Head {
        list_id: list_id.into(),
        title: title.into(),
        stands,
    }
}

fn no_rows() -> OneList {
    OneList {
        rows: Vec::new(),
        more: 0,
        fewer: false,
    }
}

fn row_ids(rows: &[ListRow]) -> Vec<&str> {
    rows.iter().map(|row| row.row_id.as_str()).collect()
}

/// A live card of one list shows the block's title and nothing more until the pull is answered,
/// then the engine's title, its counts and its first four open items, the turn's first.
#[test]
fn a_one_list_card_waits_for_its_pull_then_draws_its_rows() {
    let mut library = library(&[entry(A, 6, 2)]);
    let card = [list_ref(A, &["ci_open000005"])];
    assert_eq!(
        draw(&card, true, &library),
        Drawing::One {
            head: head(A, "Block ck_aaaaaaaaaa", Stands::Waiting),
            rows: no_rows(),
            refusal: None,
        }
    );
    library.received_list(list(A, 6, 2), &BTreeMap::new());
    let Drawing::One {
        head: drawn,
        rows,
        refusal,
    } = draw(&card, true, &library)
    else {
        panic!("one list");
    };
    assert_eq!(
        drawn,
        head(
            A,
            "Engine ck_aaaaaaaaaa",
            Stands::Counted { open: 6, done: 2 }
        )
    );
    assert_eq!(
        row_ids(&rows.rows),
        [
            "ci_open000005",
            "ci_open000000",
            "ci_open000001",
            "ci_open000002"
        ]
    );
    assert_eq!(rows.more, 2);
    assert_eq!(refusal, None);
}

/// A list the index does not name, or one the engine says was removed, is drawn removed, its head
/// alone: no rows, whatever this app held of it, and no refused edit's sentence (§2.5).
#[test]
fn a_list_the_index_does_not_name_or_the_engine_removed_is_drawn_removed() {
    let mut library = library(&[entry(A, 1, 0)]);
    let gone = [list_ref(B, &[])];
    assert_eq!(
        draw(&gone, true, &library),
        Drawing::One {
            head: head(B, "Block ck_bbbbbbbbbb", Stands::Removed),
            rows: no_rows(),
            refusal: None,
        }
    );
    library.draw(A, false);
    library.received_list(list(A, 1, 0), &BTreeMap::new());
    let check = ListOp::Check {
        row_id: "ci_open000000".into(),
    };
    library.edit(A, check, "linux-1".into());
    library.refused(
        "linux-1",
        "This list was removed earlier in the call.".into(),
    );
    let mut removed = list(A, 1, 0);
    removed.removed_at = Some("2026-11-03T08:00:00.000000Z".into());
    library.received_list(removed, &BTreeMap::new());
    let Drawing::One {
        head: drawn,
        rows,
        refusal,
    } = draw(&[list_ref(A, &[])], true, &library)
    else {
        panic!("one list");
    };
    assert_eq!(drawn.stands, Stands::Removed);
    assert_eq!((rows, refusal), (no_rows(), None));
}

/// An edit the engine refused leaves its sentence under the rows, as the engine wrote it.
#[test]
fn a_refused_edit_leaves_the_engines_sentence_under_the_rows() {
    let mut library = library(&[entry(A, 1, 0)]);
    library.draw(A, false);
    library.received_list(list(A, 1, 0), &BTreeMap::new());
    let check = ListOp::Check {
        row_id: "ci_open000000".into(),
    };
    library.edit(A, check, "linux-1".into());
    let sentence = "That row is not in this list any more; pull the list again.";
    library.refused("linux-1", sentence.into());
    let Drawing::One { refusal, .. } = draw(&[list_ref(A, &[])], true, &library) else {
        panic!("one list");
    };
    assert_eq!(refusal.as_deref(), Some(sentence));
}

/// A card of several lists draws a head per group, at most three, each standing as its list does,
/// and counts the lists past them.
#[test]
fn a_card_of_several_lists_draws_a_head_per_group_and_counts_the_rest() {
    let mut library = library(&[entry(A, 2, 0), entry(B, 1, 0), entry(D, 1, 0)]);
    library.received_list(list(A, 2, 0), &BTreeMap::new());
    let card = [
        list_ref(A, &["ci_open000001"]),
        list_ref(B, &[]),
        list_ref(C, &[]),
        list_ref(D, &[]),
    ];
    let Drawing::Several { groups, more_lists } = draw(&card, true, &library) else {
        panic!("several lists");
    };
    let heads: Vec<&Head> = groups.iter().map(|group| &group.head).collect();
    assert_eq!(
        heads,
        [
            &head(
                A,
                "Engine ck_aaaaaaaaaa",
                Stands::Counted { open: 2, done: 0 }
            ),
            &head(B, "Block ck_bbbbbbbbbb", Stands::Waiting),
            &head(C, "Block ck_cccccccccc", Stands::Removed),
        ]
    );
    assert_eq!(row_ids(&groups[0].rows), ["ci_open000001"]);
    assert_eq!(more_lists, 1);
}

/// On a card of several lists, a list the engine removed, or one the index no longer names, is
/// its head alone, "This list was removed.": the rows its turn touched are not drawn, so none can
/// be ticked, and no refused edit's sentence shows (§2.5).
#[test]
fn a_removed_list_on_a_card_of_several_draws_its_head_alone() {
    let mut library = library(&[entry(A, 2, 0), entry(B, 2, 0), entry(C, 2, 0)]);
    for id in [A, B, C] {
        library.draw(id, false);
        library.received_list(list(id, 2, 0), &BTreeMap::new());
    }
    let check = ListOp::Check {
        row_id: "ci_open000001".into(),
    };
    library.edit(B, check, "linux-1".into());
    library.refused("linux-1", "Not now.".into());
    let mut removed = list(B, 2, 0);
    removed.removed_at = Some("2026-11-03T08:00:00.000000Z".into());
    library.received_list(removed, &BTreeMap::new());
    library.received_index(vec![entry(A, 2, 0), entry(B, 2, 0)]);
    let touched = ["ci_open000000", "ci_open000001"];
    let card = [
        list_ref(A, &touched),
        list_ref(B, &touched),
        list_ref(C, &touched),
    ];
    let Drawing::Several { groups, .. } = draw(&card, true, &library) else {
        panic!("several lists");
    };
    assert_eq!(row_ids(&groups[0].rows), touched, "a list that stands");
    for group in &groups[1..] {
        assert_eq!(group.head.stands, Stands::Removed, "{}", group.head.list_id);
        assert_eq!(
            (group.rows.as_slice(), &group.refusal),
            (&[][..], &None),
            "{}",
            group.head.list_id
        );
    }
}

/// On a card of several lists, an edit the engine refused leaves its sentence under the rows of
/// the list it edited, and under no other (§2.5, §5.3).
#[test]
fn a_refused_tick_on_a_card_of_several_shows_the_sentence_under_its_list() {
    let mut library = library(&[entry(A, 2, 0), entry(B, 2, 0)]);
    for id in [A, B] {
        library.draw(id, false);
        library.received_list(list(id, 2, 0), &BTreeMap::new());
    }
    let check = ListOp::Check {
        row_id: "ci_open000000".into(),
    };
    library.edit(B, check, "linux-1".into());
    let sentence = "That row is not in this list any more; pull the list again.";
    library.refused("linux-1", sentence.into());
    let touched = ["ci_open000000"];
    let card = [list_ref(A, &touched), list_ref(B, &touched)];
    let Drawing::Several { groups, .. } = draw(&card, true, &library) else {
        panic!("several lists");
    };
    let sentences: Vec<Option<&str>> = groups.iter().map(|g| g.refusal.as_deref()).collect();
    assert_eq!(sentences, [None, Some(sentence)]);
    let ListGroup { rows, .. } = &groups[1];
    assert_eq!(row_ids(rows), touched, "the row back as the engine has it");
}

/// A ticked row keeps its place until the last settle begun for it ends: ticked, unticked and
/// ticked again within one settle, it keeps its place for the whole of the second.
#[test]
fn a_row_ticked_again_while_it_settles_keeps_its_place_for_the_whole_second_settle() {
    let mut settling = Settling::default();
    settling.begin("ci_open000000");
    settling.begin("ci_open000001");
    settling.begin("ci_open000000");
    assert!(!settling.end("ci_open000000"), "the first settle's end");
    assert!(settling.rows().contains("ci_open000000"), "still settling");
    assert!(settling.end("ci_open000001"));
    assert!(settling.end("ci_open000000"), "the second settle's end");
    assert!(settling.rows().is_empty());
    assert!(!settling.end("ci_open000000"), "no settle left to end");
}

/// A folded card draws a line for each list it drew, counted from the index as it is now, never
/// from a pull: a list the index no longer names is removed, and before the index answers each
/// line is its block's title alone.
#[test]
fn a_folded_card_counts_from_the_index_and_names_a_removed_list() {
    let card = [
        list_ref(A, &[]),
        list_ref(B, &[]),
        list_ref(C, &[]),
        list_ref(D, &[]),
    ];
    let unanswered = Library::default();
    let Drawing::Folded(waiting) = draw(&card, false, &unanswered) else {
        panic!("folded");
    };
    assert!(waiting.iter().all(|head| head.stands == Stands::Waiting));
    let mut library = library(&[entry(A, 3, 1), entry(C, 0, 4)]);
    library.received_list(list(A, 9, 9), &BTreeMap::new());
    assert_eq!(
        draw(&card, false, &library),
        Drawing::Folded(vec![
            head(
                A,
                "Index ck_aaaaaaaaaa",
                Stands::Counted { open: 3, done: 1 }
            ),
            head(B, "Block ck_bbbbbbbbbb", Stands::Removed),
            head(
                C,
                "Index ck_cccccccccc",
                Stands::Counted { open: 0, done: 4 }
            ),
        ]),
        "the first three lists the card drew, by the index, not by the list held"
    );
}

/// A card of no lists is no card.
#[test]
fn a_card_of_no_lists_draws_nothing() {
    let library = library(&[]);
    assert_eq!(drawing(&[], true, &library, false, &BTreeSet::new()), None);
}

/// A live card pulls the lists it draws when it comes on screen, its first three; a folded card
/// never pulls.
#[test]
fn only_a_live_card_pulls_and_only_the_lists_it_draws() {
    let lists = vec![
        list_ref(A, &[]),
        list_ref(B, &[]),
        list_ref(C, &[]),
        list_ref(D, &[]),
    ];
    let live = ListsCard {
        lists: lists.clone(),
        live: true,
    };
    let pulled: Vec<&str> = live.pulls().iter().map(|l| l.list_id.as_str()).collect();
    assert_eq!(pulled, [A, B, C]);
    let folded = ListsCard { lists, live: false };
    assert!(folded.pulls().is_empty());
    let mut library = library(&[entry(A, 1, 0), entry(B, 1, 0), entry(C, 1, 0)]);
    let sent: Vec<ClientEvent> = live
        .pulls()
        .iter()
        .flat_map(|list| library.draw(&list.list_id, false))
        .collect();
    assert_eq!(
        sent.len(),
        3,
        "each list the live card draws is pulled once"
    );
}

const COUNTED: Stands = Stands::Counted { open: 2, done: 0 };

/// A card of one list, `A`, standing as `stands`, with `refusal` under its rows.
fn one(stands: Stands, refusal: Option<&str>) -> Drawing {
    Drawing::One {
        head: head(A, "Groceries", stands),
        rows: no_rows(),
        refusal: refusal.map(str::to_owned),
    }
}

/// A card of several lists, each with its refused edit's sentence, if any.
fn several(lists: &[(&str, Option<&str>)]) -> Drawing {
    let groups = lists.iter().map(|(list_id, refusal)| ListGroup {
        head: head(list_id, list_id, COUNTED),
        rows: Vec::new(),
        refusal: refusal.map(str::to_owned),
    });
    Drawing::Several {
        groups: groups.collect(),
        more_lists: 0,
    }
}

/// A refused edit's sentence is told as it comes under a list the card drew: a second list
/// refused in the words a first one shows is told too, and a card drawn again with the sentences
/// it had tells nothing. A card off screen tells nothing (§6.3).
#[test]
fn a_refusal_is_told_as_it_comes_under_a_list_even_in_words_another_list_shows() {
    let busy = "This list is being changed on another device; try again in a moment.";
    let gone = "That row is not in this list any more; pull the list again.";
    let refused = one(COUNTED, Some(gone));
    assert_eq!(news(Some(&one(COUNTED, None)), &refused, true), [gone]);
    assert!(
        news(Some(&one(COUNTED, None)), &refused, false).is_empty(),
        "off screen"
    );
    assert!(
        news(Some(&refused), &refused, true).is_empty(),
        "under it already"
    );
    let first = several(&[(A, Some(busy)), (B, None)]);
    let both = several(&[(A, Some(busy)), (B, Some(busy))]);
    assert_eq!(news(Some(&first), &both, true), [busy], "B's, in A's words");
    assert!(news(Some(&both), &both, true).is_empty());
    let other = several(&[(A, Some(gone)), (B, Some(busy))]);
    assert_eq!(news(Some(&both), &other, true), [gone], "new words under A");
}

/// A refused edit's sentence stays on its list until the list's next edit, on whichever card
/// draws the list live, and a card tells it only where it first shows it: a newer card drawn with
/// the list, or a card turning live, says nothing of the older refusal (§5.3).
#[test]
fn a_card_first_showing_a_list_says_nothing_of_its_older_refusal() {
    let gone = "That row is not in this list any more; pull the list again.";
    let mut library = library(&[entry(A, 2, 0)]);
    library.draw(A, false);
    library.received_list(list(A, 2, 0), &BTreeMap::new());
    let card = [list_ref(A, &[])];
    let older = draw(&card, true, &library);
    let check = ListOp::Check {
        row_id: "ci_open000000".into(),
    };
    library.edit(A, check, "linux-1".into());
    library.refused("linux-1", gone.into());
    let refused = draw(&card, true, &library);
    assert_eq!(
        news(Some(&older), &refused, true),
        [gone],
        "the card it was made on"
    );
    let Drawing::One { refusal, .. } = &refused else {
        panic!("one list");
    };
    assert_eq!(
        refusal.as_deref(),
        Some(gone),
        "the list's, on any card drawing it"
    );
    assert!(news(None, &refused, true).is_empty(), "a newer card");
    let folded = draw(&card, false, &library);
    assert!(
        news(Some(&folded), &refused, true).is_empty(),
        "a card turning live"
    );
}

/// "This list was removed." is told once, when a card of one list that was on screen at the
/// chat's last look turns removed. A card off screen, first drawn, or turning live on a removed
/// list, whose folded part already said so, says nothing (§6.3).
#[test]
fn this_list_was_removed_is_told_once_by_a_card_of_one_list_on_screen() {
    let removed = one(Stands::Removed, None);
    for before in [one(COUNTED, None), one(Stands::Waiting, None)] {
        assert_eq!(news(Some(&before), &removed, true), [LIST_REMOVED]);
        assert!(
            news(Some(&before), &removed, false).is_empty(),
            "off screen"
        );
    }
    assert!(
        news(Some(&removed), &removed, true).is_empty(),
        "told already"
    );
    assert!(news(None, &removed, true).is_empty(), "first drawn");
    let folded = Drawing::Folded(vec![head(A, "Groceries", Stands::Removed)]);
    assert!(
        news(Some(&folded), &removed, true).is_empty(),
        "turned live"
    );
}
