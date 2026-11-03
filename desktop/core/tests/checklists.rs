//! One list as the chat holds it (M85 lists §5.3, §4.3; PROTOCOL.md "Lists"): an edit drawn at
//! once over the engine's list and placed as the engine would place it, the next pushed rev
//! confirming it, a stale push dropped, a refusal putting the row back with the engine's sentence,
//! Copy as text, and the words. A port of the macOS app's ChecklistStoreTests (state and draft).

use fermix_client::chat_wire::protocol::{read_line, List, ListOp, ListRow, ServerEvent};
use fermix_client::checklists::draft::{self, Place};
use fermix_client::checklists::state::ListState;
use fermix_client::checklists::{sections, words, ITEM};
use std::collections::BTreeMap;

const SERVER: &str = include_str!("../contracts/companion/fixtures/server_events.jsonl");

/// The golden Groceries list (`checklist`, rev 7): Bread in the first group, then Produce with
/// Lemons (noted), Basil done, and Rice.
const BREAD: &str = "ci_8f3k2m9q1x";
const PRODUCE: &str = "ci_5w7p0t4n6b";
const LEMONS: &str = "ci_0r2y8e6u3g";
const BASIL: &str = "ci_3h9d1v8r2k";
const RICE: &str = "ci_6j4c7x0m5s";

fn golden(index: usize) -> (List, BTreeMap<String, String>) {
    let line = SERVER
        .lines()
        .nth(index)
        .expect("the fixture has this line");
    match read_line(line.as_bytes()).expect("a golden line decodes") {
        ServerEvent::Checklist { list, made } => (*list, made),
        other => panic!("line {index} is a list: {other:?}"),
    }
}

fn groceries() -> List {
    golden(53).0
}

fn with(rev: u64, rows: Vec<ListRow>) -> List {
    List {
        rev,
        rows,
        ..groceries()
    }
}

fn texts(list: &List) -> Vec<&str> {
    list.rows.iter().map(|row| row.text.as_str()).collect()
}

fn row<'a>(list: &'a List, id: &str) -> &'a ListRow {
    list.rows
        .iter()
        .find(|row| row.row_id == id)
        .expect("the list has the row")
}

fn item(id: &str, text: &str) -> ListRow {
    ListRow {
        row_id: id.into(),
        kind: ITEM.into(),
        text: text.into(),
        note: None,
        done: Some(false),
        done_at: None,
    }
}

fn applying(op: ListOp) -> List {
    draft::apply(&op, "linux-1", groceries())
}

fn add(text: &str, place: Option<Place>) -> ListOp {
    let (section_id, after_id) = Place::fields(place);
    ListOp::Add {
        text: text.into(),
        note: None,
        section_id,
        after_id,
    }
}

fn moving(row_id: &str, place: Option<Place>, to_list_id: Option<&str>) -> ListOp {
    let (section_id, after_id) = Place::fields(place);
    ListOp::Move {
        row_id: row_id.into(),
        section_id,
        after_id,
        to_list_id: to_list_id.map(str::to_owned),
    }
}

fn check(row_id: &str) -> ListOp {
    ListOp::Check {
        row_id: row_id.into(),
    }
}

fn received(state: &mut ListState, list: List) -> bool {
    state.received(list, &BTreeMap::new())
}

// ---- one list's state ----

#[test]
fn an_edit_is_drawn_at_once_over_the_engines_list_which_stays_as_sent() {
    let mut state = ListState::default();
    assert_eq!(
        state.shown(),
        None,
        "nothing is drawn before a pull answers"
    );
    received(&mut state, groceries());

    state.edit(check(LEMONS), "linux-1".into());

    let shown = state.shown().expect("drawn");
    assert!(row(&shown, LEMONS).is_done());
    assert_eq!(shown.open_count(), 2);
    let engine = state.engine().expect("the engine's list");
    assert!(
        !row(engine, LEMONS).is_done(),
        "the engine's own list is untouched"
    );
}

#[test]
fn an_accepted_edit_is_confirmed_by_the_next_list_and_one_not_yet_accepted_stays_drawn_over_it() {
    let mut state = ListState::default();
    received(&mut state, groceries());
    state.edit(check(LEMONS), "linux-1".into());
    let rename = ListOp::Rename {
        text: "Weekly groceries".into(),
    };
    state.edit(rename, "linux-2".into());
    state.accepted("linux-1");

    let mut rows = groceries().rows;
    rows[2].done = Some(true);
    assert!(received(&mut state, with(8, rows)));

    let pending: Vec<&str> = state
        .pending()
        .iter()
        .map(|edit| edit.client_msg_id.as_str())
        .collect();
    assert_eq!(pending, ["linux-2"]);
    let shown = state.shown().expect("drawn");
    assert_eq!(shown.title, "Weekly groceries");
    assert!(row(&shown, LEMONS).is_done());
}

#[test]
fn a_list_whose_rev_is_below_the_one_shown_is_dropped_and_the_same_rev_is_taken() {
    let mut state = ListState::default();
    received(&mut state, with(8, groceries().rows));

    assert!(!received(&mut state, with(7, Vec::new())));
    assert_eq!(state.engine().map(|list| list.rev), Some(8));
    assert_eq!(state.shown().map(|list| list.rows.len()), Some(5));
    assert!(
        received(&mut state, with(8, Vec::new())),
        "the same rev again is taken"
    );
}

#[test]
fn a_refused_edit_stops_being_drawn_and_the_row_is_the_engines_again_under_its_sentence() {
    let sentence = "That row is not in this list any more; pull the list again.";
    let mut state = ListState::default();
    received(&mut state, groceries());
    state.edit(check(RICE), "linux-1".into());
    assert!(row(&state.shown().expect("drawn"), RICE).is_done());

    state.refused("linux-1", sentence.into());

    assert!(!row(&state.shown().expect("drawn"), RICE).is_done());
    assert_eq!(state.refusal(), Some(sentence));
    assert!(state.pending().is_empty());
    let uncheck = ListOp::Uncheck {
        row_id: BASIL.into(),
    };
    state.edit(uncheck, "linux-2".into());
    assert_eq!(state.refusal(), None, "the next edit clears the sentence");
}

#[test]
fn an_add_is_drawn_as_sending_until_the_list_that_names_it_in_made_arrives() {
    let mut state = ListState::default();
    received(&mut state, groceries());
    state.edit(
        add("Bread", Some(Place::After(BREAD.into()))),
        "linux-22".into(),
    );

    let shown = state.shown().expect("drawn");
    assert_eq!(shown.rows[1].text, "Bread");
    assert!(draft::is_local(&shown.rows[1].row_id), "drawn as sending");

    let mut rows = groceries().rows;
    rows.insert(1, item("ci_9x9x9x9x9x", "Bread"));
    let made = BTreeMap::from([("linux-22".to_owned(), "ci_9x9x9x9x9x".to_owned())]);
    assert!(state.received(with(8, rows), &made));

    assert!(
        state.pending().is_empty(),
        "made confirms the add before its accepted"
    );
    let shown = state.shown().expect("drawn");
    assert_eq!(
        texts(&shown)
            .iter()
            .filter(|text| **text == "Bread")
            .count(),
        2
    );
    assert!(!shown.rows.iter().any(|row| draft::is_local(&row.row_id)));
}

#[test]
fn the_golden_push_confirms_the_golden_add_by_what_it_made() {
    let (list, made) = golden(53);
    assert_eq!(made.get("mac-21").map(String::as_str), Some(LEMONS));
    let mut state = ListState::default();
    received(&mut state, with(6, Vec::new()));
    let lemons = ListOp::Add {
        text: "Lemons".into(),
        note: Some("The unwaxed ones".into()),
        section_id: Some(PRODUCE.into()),
        after_id: None,
    };
    state.edit(lemons, "mac-21".into());

    assert!(state.received(list, &made));

    assert!(state.pending().is_empty());
    assert_eq!(state.shown(), Some(groceries()));
}

#[test]
fn a_list_the_engine_removed_says_so_until_it_is_restored() {
    let (packing, _) = golden(56);
    let mut state = ListState::default();
    assert!(!state.is_removed());
    received(&mut state, packing);
    assert!(state.is_removed());
}

// ---- drawn as the engine applies it ----

#[test]
fn an_add_goes_to_the_end_of_its_section_right_after_its_row_or_to_the_end_of_the_first_group() {
    let in_produce = applying(add("Milk", Some(Place::Section(PRODUCE.into()))));
    assert_eq!(
        texts(&in_produce),
        ["Bread", "Produce", "Lemons", "Basil", "Rice", "Milk"]
    );
    let after_lemons = applying(add("Milk", Some(Place::After(LEMONS.into()))));
    assert_eq!(
        texts(&after_lemons),
        ["Bread", "Produce", "Lemons", "Milk", "Basil", "Rice"]
    );
    let first_group = applying(add("Milk", None));
    assert_eq!(
        texts(&first_group),
        ["Bread", "Milk", "Produce", "Lemons", "Basil", "Rice"],
        "the items above the first heading"
    );

    let mut rows = groceries().rows[1..].to_vec();
    let dairy = ListRow {
        row_id: "ci_1d2a3i4r5y".into(),
        text: "Dairy".into(),
        ..row(&groceries(), PRODUCE).clone()
    };
    rows.extend([dairy, item("ci_2c3h4e5s6e", "Cheese")]);
    let headed = draft::apply(&add("Milk", None), "linux-1", with(7, rows));
    assert_eq!(
        texts(&headed),
        ["Produce", "Lemons", "Basil", "Rice", "Milk", "Dairy", "Cheese"],
        "or else the end of the first section"
    );
    let empty = draft::apply(&add("Milk", None), "linux-1", with(7, Vec::new()));
    assert_eq!(texts(&empty), ["Milk"]);
    let unknown = applying(add("Milk", Some(Place::After("ci_0000000000".into()))));
    assert_eq!(texts(&unknown), texts(&first_group), "a row the list lacks");
}

#[test]
fn a_move_within_the_list_lands_as_an_add_would_and_a_move_to_another_list_leaves_it() {
    let bread_down = applying(moving(BREAD, Some(Place::Section(PRODUCE.into())), None));
    assert_eq!(
        texts(&bread_down),
        ["Produce", "Lemons", "Basil", "Rice", "Bread"]
    );
    let rice_up = applying(moving(RICE, Some(Place::After(PRODUCE.into())), None));
    assert_eq!(
        texts(&rice_up),
        ["Bread", "Produce", "Rice", "Lemons", "Basil"]
    );
    let away = applying(moving(RICE, None, Some("ck_9t3k6w0p5h")));
    assert_eq!(texts(&away), ["Bread", "Produce", "Lemons", "Basil"]);
    let heading = applying(moving(PRODUCE, None, Some("ck_9t3k6w0p5h")));
    assert_eq!(heading, groceries(), "a heading is not moved");
    let nowhere = applying(moving(RICE, None, None));
    assert_eq!(nowhere, groceries(), "a move that names no place");
}

#[test]
fn check_uncheck_and_edit_draw_at_once_on_an_item_and_never_on_a_heading() {
    assert_eq!(
        applying(check(PRODUCE)),
        groceries(),
        "a heading is never done"
    );
    let unticked = applying(ListOp::Uncheck {
        row_id: BASIL.into(),
    });
    assert_eq!(unticked.done_count(), 0);
    assert_eq!(
        row(&unticked, BASIL).done_at,
        None,
        "unticking forgets when"
    );

    let reworded = applying(ListOp::Edit {
        row_id: LEMONS.into(),
        text: Some("Limes".into()),
        note: Some(String::new()),
    });
    assert_eq!(row(&reworded, LEMONS).text, "Limes");
    assert_eq!(row(&reworded, LEMONS).note, None, "an empty note clears it");
    let noted = applying(ListOp::Edit {
        row_id: LEMONS.into(),
        text: None,
        note: Some("Two".into()),
    });
    assert_eq!(
        (
            row(&noted, LEMONS).text.as_str(),
            row(&noted, LEMONS).note.as_deref()
        ),
        ("Lemons", Some("Two"))
    );
}

#[test]
fn remove_rename_clear_done_and_a_section_draw_at_once_and_the_engines_own_ops_change_nothing() {
    let removed = applying(ListOp::Remove {
        row_id: BREAD.into(),
    });
    assert_eq!(texts(&removed), ["Produce", "Lemons", "Basil", "Rice"]);
    let renamed = applying(ListOp::Rename {
        text: "Weekly".into(),
    });
    assert_eq!(renamed.title, "Weekly");
    assert_eq!(
        texts(&applying(ListOp::ClearDone)),
        ["Bread", "Produce", "Lemons", "Rice"]
    );
    let section = applying(ListOp::Section {
        text: "Dairy".into(),
        after_id: None,
    });
    assert_eq!(section.rows.last().map(|row| row.is_section()), Some(true));

    let engines = [
        ListOp::Folder {
            folder: "Home".into(),
        },
        ListOp::RemoveList,
        ListOp::Restore {
            row_id: Some(BASIL.into()),
            title: None,
        },
        ListOp::Create {
            title: "Packing".into(),
            folder: None,
        },
    ];
    for op in engines {
        assert_eq!(
            applying(op.clone()),
            groceries(),
            "{op:?} is drawn when pushed"
        );
    }
}

#[test]
fn an_op_names_every_row_it_acts_on_or_is_placed_by() {
    let local = draft::local_row_id("linux-1");
    let l = local.as_str();
    let cases = [
        (check(l), vec![l]),
        (
            moving(RICE, Some(Place::After(l.into())), None),
            vec![RICE, l],
        ),
        (add("Eggs", Some(Place::Section(l.into()))), vec![l]),
        (
            ListOp::Section {
                text: "Dairy".into(),
                after_id: Some(l.into()),
            },
            vec![l],
        ),
        (ListOp::ClearDone, vec![]),
    ];
    for (op, named) in cases {
        assert_eq!(draft::named_rows(&op), named, "{op:?}");
    }
    assert!(draft::is_local(&local));
    assert!(!draft::is_local(BREAD));
}

#[test]
fn words_trimmed_to_nothing_are_no_edit_and_a_note_alone_is_one() {
    let blank = ["", "  ", "\n\t"];
    for words in blank {
        assert!(!draft::sendable(&add(words, None)), "{words:?}");
        let rename = ListOp::Rename { text: words.into() };
        assert!(!draft::sendable(&rename));
    }
    let note_only = ListOp::Edit {
        row_id: LEMONS.into(),
        text: None,
        note: Some(String::new()),
    };
    assert!(draft::sendable(&note_only), "clearing a note is an edit");
    let nothing = ListOp::Edit {
        row_id: LEMONS.into(),
        text: None,
        note: None,
    };
    assert!(!draft::sendable(&nothing));
    assert!(draft::sendable(&check(LEMONS)));
}

// ---- Copy as text ----

#[test]
fn a_list_reads_as_the_macos_apps_copy_as_text_with_a_two_line_note() {
    assert_eq!(
        groceries().markdown(),
        "# Groceries\n- [ ] Bread\n\n## Produce\n- [ ] Lemons\n  The unwaxed ones\n- [x] Basil\n- [ ] Rice"
    );
    let noted = applying(ListOp::Edit {
        row_id: LEMONS.into(),
        text: None,
        note: Some("Unwaxed\nFour".into()),
    });
    assert!(noted
        .markdown()
        .contains("- [ ] Lemons\n  Unwaxed\n  Four\n- [x] Basil"));
}

#[test]
fn a_lists_sections_are_its_first_group_and_each_heading_with_its_items() {
    let list = groceries();
    let parts = sections(&list.rows);
    let headings: Vec<Option<&str>> = parts
        .iter()
        .map(|part| part.heading.map(|row| row.text.as_str()))
        .collect();
    assert_eq!(headings, [None, Some("Produce")]);
    let items: Vec<Vec<&str>> = parts
        .iter()
        .map(|part| part.items.iter().map(|row| row.text.as_str()).collect())
        .collect();
    assert_eq!(items, [vec!["Bread"], vec!["Lemons", "Basil", "Rice"]]);

    let mut odd = groceries().rows;
    odd[0].kind = "checkbox".into();
    let odd = sections(&odd);
    assert_eq!(
        odd.len(),
        1,
        "a row of a kind this build does not know is in none"
    );
}

// ---- words ----

#[test]
fn the_lists_words_are_sentence_case_free_of_mac_and_of_wire_tokens() {
    for text in words::STRINGS {
        let first = text.chars().next().expect("not empty");
        assert!(first.is_uppercase(), "{text} starts a sentence");
        assert!(!text.contains("Mac"), "{text} names the Mac");
        assert!(!text.contains('_'), "{text} carries a wire token");
        assert!(!text.contains("  "), "{text} has a double space");
    }
}

#[test]
fn the_counts_and_names_read_as_the_copy_deck_says_to_the_eye_and_to_a_screen_reader() {
    assert_eq!(words::counts(6, 2), "6 open · 2 done");
    assert_eq!(words::spoken_counts(6, 2), "6 open, 2 done");
    assert_eq!(
        words::head_line(Some("Projects"), 6, 2),
        "Projects · 6 open · 2 done"
    );
    assert_eq!(words::head_line(None, 6, 2), "6 open · 2 done");
    assert_eq!(words::more(2), "2 more");
    assert_eq!(words::more_lists(2), "and 2 more lists");
    assert_eq!(words::done_heading(2), "Done · 2");
    assert_eq!(words::spoken_done_heading(2), "Done, 2");
    assert_eq!(words::removed("Lemons"), "Removed Lemons.");
    assert_eq!(words::folded("Groceries", 3), "Groceries · 3 open");
    assert_eq!(words::list_label("Fermix", 6), "Fermix, list, 6 open");
    assert_eq!(
        words::lists_label(&["Fermix", "Portfolio site", "Groceries"]),
        "Fermix, Portfolio site and Groceries, lists"
    );
    assert_eq!(
        words::lists_label(&["Fermix", "Groceries"]),
        "Fermix and Groceries, lists"
    );
}

#[test]
fn a_names_underscore_is_doubled_in_a_menu_item_and_kept_as_written_elsewhere() {
    assert_eq!(words::menu_name("to_do_list"), "to__do__list");
    assert_eq!(words::menu_name("Groceries"), "Groceries");
}
