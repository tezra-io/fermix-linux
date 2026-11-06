//! What a reply's lists card draws and what the Lists pane draws (M85 lists §2, §3.3 to §3.5): only
//! the newest card of a list is live, a one-list card's four open rows with the turn's first, a
//! card of several lists' bounded groups, the pane's lines and folds, where Alt+Up and Alt+Down
//! put a row, where an added item goes, the picker's order, the list the pane shows, and the one
//! splice a change makes. A port of the macOS app's ChecklistCardsTests and ChecklistPanelTests.

use fermix_client::chat_wire::protocol::{List, ListEntry, ListRow, RemovedRow};
use fermix_client::checklists::cards::{self, GROUP_LISTS, ONE_LIST_ROWS};
use fermix_client::checklists::draft::Place;
use fermix_client::checklists::index::Folders;
use fermix_client::checklists::library::Library;
use fermix_client::checklists::panel::{self, Line, Steps, View};
use fermix_client::checklists::{ListRef, ITEM, SECTION};
use std::collections::{BTreeMap, BTreeSet};

fn list_ref(list_id: &str, rows: &[&str]) -> ListRef {
    ListRef {
        list_id: list_id.into(),
        title: list_id.into(),
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

fn heading(id: &str) -> ListRow {
    ListRow {
        row_id: id.into(),
        kind: SECTION.into(),
        text: id.into(),
        note: None,
        done: None,
        done_at: None,
    }
}

fn list(list_id: &str, rows: Vec<ListRow>) -> List {
    List {
        list_id: list_id.into(),
        title: list_id.into(),
        rev: 1,
        rows,
        removed: Vec::new(),
        removed_at: None,
    }
}

fn row_ids(rows: &[ListRow]) -> Vec<&str> {
    rows.iter().map(|row| row.row_id.as_str()).collect()
}

fn line_ids(lines: &[Line]) -> Vec<String> {
    lines.iter().map(Line::id).collect()
}

fn live(cards: &[(u64, Vec<ListRef>)]) -> Vec<u64> {
    let refs = cards.iter().map(|(seq, refs)| (*seq, refs.as_slice()));
    cards::live(refs).into_iter().collect()
}

fn entry(list_id: &str, title: &str, folder: Option<&str>) -> ListEntry {
    ListEntry {
        list_id: list_id.into(),
        title: title.into(),
        folder: folder.map(str::to_owned),
        open: 1,
        done: 0,
        rev: 1,
    }
}

/// Fermix: one loose item, Active with a and done b, Later with c.
fn fermix() -> List {
    list(
        "ck_a",
        vec![
            item("loose", false),
            heading("active"),
            item("a", false),
            item("b", true),
            heading("later"),
            item("c", false),
        ],
    )
}

// ---- cards (M85 lists §2) ----

#[test]
fn only_the_newest_card_that_draws_a_list_is_live() {
    let cards = vec![
        (1, vec![list_ref("fermix", &[])]),
        (2, vec![]),
        (3, vec![list_ref("fermix", &[]), list_ref("groceries", &[])]),
        (4, vec![list_ref("packing", &[])]),
        (
            5,
            vec![list_ref("groceries", &[]), list_ref("portfolio", &[])],
        ),
        (6, vec![list_ref("packing", &[])]),
    ];

    assert_eq!(live(&cards), [5, 6]);
    assert_eq!(live(&cards[..4]), [3, 4]);
    assert_eq!(live(&cards[..2]), [1], "a reply with no lists has no card");
    assert!(live(&[]).is_empty());
}

#[test]
fn only_the_lists_a_card_draws_count_so_a_fourth_it_only_names_keeps_its_older_card_live() {
    let several = ["fermix", "portfolio", "groceries", "garden"].map(|id| list_ref(id, &[]));
    let mut cards = vec![(1, vec![list_ref("garden", &[])]), (2, several.to_vec())];

    assert_eq!(GROUP_LISTS, 3);
    assert_eq!(live(&cards), [1, 2]);
    cards.push((3, vec![list_ref("groceries", &[])]));
    assert_eq!(live(&cards), [1, 3]);
}

#[test]
fn a_one_list_card_holds_at_most_four_open_items_the_turns_first_and_counts_the_rest() {
    let packing = list(
        "packing",
        vec![
            item("passport", true),
            heading("trip"),
            item("charger", false),
            item("boots", false),
            item("sunscreen", false),
            item("jacket", false),
            item("book", false),
            item("adapter", false),
        ],
    );
    let touched: Vec<String> = ["jacket", "sunscreen", "passport"]
        .map(String::from)
        .to_vec();
    let none = BTreeSet::new();

    let card = cards::one_list(&packing, &touched, &none, false);
    assert_eq!(ONE_LIST_ROWS, 4);
    assert_eq!(
        row_ids(&card.rows),
        ["sunscreen", "jacket", "charger", "boots"],
        "the turn's open items first, each part in the list's order"
    );
    assert_eq!((card.more, card.fewer), (2, false));

    let settling = BTreeSet::from(["passport".to_owned()]);
    let settled = cards::one_list(&packing, &[], &settling, false);
    assert_eq!(
        settled.rows[0].row_id, "passport",
        "a row ticked here keeps its place"
    );
    assert_eq!(settled.more, 3);

    let grown = cards::one_list(&packing, &touched, &none, true);
    assert_eq!(
        row_ids(&grown.rows),
        ["sunscreen", "jacket", "charger", "boots", "book", "adapter"],
        "grown, every open item, the first four where they were"
    );
    assert_eq!((grown.more, grown.fewer), (0, true));

    let short = list("l", vec![item("a", false)]);
    let short_grown = cards::one_list(&short, &[], &none, true);
    assert_eq!(row_ids(&short_grown.rows), ["a"]);
    assert_eq!(
        (short_grown.more, short_grown.fewer),
        (0, false),
        "nothing to fold"
    );
}

#[test]
fn a_card_of_several_lists_draws_three_groups_of_at_most_three_touched_rows_and_names_the_rest() {
    let refs = vec![
        list_ref("fermix", &["a", "b", "c", "d", "gone"]),
        list_ref("portfolio", &["p"]),
        list_ref("groceries", &[]),
        list_ref("packing", &["x"]),
        list_ref("garden", &["y"]),
    ];
    let rows = ["d", "c", "b", "a"].map(|id| item(id, id == "b")).to_vec();
    let lists = BTreeMap::from([
        ("fermix", list("fermix", rows)),
        ("portfolio", list("portfolio", vec![item("p", false)])),
    ]);

    let card = cards::groups(&refs, |id| lists.get(id).cloned());

    let drawn: Vec<&str> = card
        .groups
        .iter()
        .map(|g| g.list.list_id.as_str())
        .collect();
    assert_eq!(drawn, ["fermix", "portfolio", "groceries"]);
    assert_eq!(
        row_ids(&card.groups[0].rows),
        ["a", "b", "c"],
        "the turn's order, at most three, a removed row skipped"
    );
    assert!(
        card.groups[0].rows[1].is_done(),
        "a row the turn ticked shows done"
    );
    assert_eq!(row_ids(&card.groups[1].rows), ["p"]);
    assert!(card.groups[2].rows.is_empty(), "a list the turn only read");
    assert_eq!(card.more_lists, 2);
    assert_eq!(
        cards::groups(&refs[..2], |id| lists.get(id).cloned()).more_lists,
        0
    );
}

// ---- the pane's lines (M85 lists §3.4) ----

#[test]
fn the_lines_are_each_section_under_its_heading_then_done_closed_until_opened_then_recently_removed(
) {
    let closed = panel::lines(&fermix(), &View::default());
    assert_eq!(
        line_ids(&closed),
        ["loose", "heading:active", "a", "heading:later", "c", "done"]
    );

    let view = View {
        folded: BTreeSet::from(["later".to_owned()]),
        done_open: true,
        ..View::default()
    };
    let folded = panel::lines(&fermix(), &view);
    assert_eq!(
        line_ids(&folded),
        ["loose", "heading:active", "a", "heading:later", "done", "b"]
    );
    assert!(matches!(folded[3], Line::Heading { folded: true, .. }));
    assert_eq!(
        folded[4],
        Line::DoneHeading {
            count: 1,
            open: true
        }
    );

    let view = View {
        settling: BTreeSet::from(["b".to_owned()]),
        ..View::default()
    };
    let settling = panel::lines(&fermix(), &view);
    assert_eq!(
        line_ids(&settling),
        ["loose", "heading:active", "a", "b", "heading:later", "c"],
        "a row ticked a moment ago keeps its place, and Done has nothing yet"
    );

    let mut with_removed = fermix();
    let removed_row = |row: ListRow| RemovedRow {
        row,
        removed_at: "2031-06-02T14:05:00.000000Z".into(),
    };
    with_removed.removed = vec![removed_row(item("old", true)), removed_row(heading("gone"))];
    let view = View {
        removed_shown: true,
        ..View::default()
    };
    let removed = panel::lines(&with_removed, &view);
    assert_eq!(
        line_ids(&removed)[5..],
        ["done", "removed", "removed:old"],
        "a removed heading is not drawn"
    );
    let hidden = panel::lines(&with_removed, &View::default());
    assert_eq!(line_ids(&hidden).last().map(String::as_str), Some("done"));
}

#[test]
fn a_step_lands_past_the_line_above_or_below_a_heading_included_and_nowhere_an_op_cannot_say() {
    let mut rows = fermix().rows;
    rows.insert(3, item("b2", false));
    let view = View {
        done_open: true,
        ..View::default()
    };
    let lines = panel::lines(&list("ck_a", rows), &view);
    let after = |id: &str| Some(Place::After(id.into()));

    assert_eq!(
        panel::step("b2", true, &lines),
        after("active"),
        "first in its section"
    );
    assert_eq!(
        panel::step("c", true, &lines),
        after("b2"),
        "up past a heading"
    );
    assert_eq!(panel::step("a", true, &lines), after("loose"));
    assert_eq!(
        panel::step("loose", true, &lines),
        None,
        "the top of a headless group"
    );
    assert_eq!(panel::step("a", false, &lines), after("b2"));
    assert_eq!(
        panel::step("b2", false, &lines),
        after("later"),
        "down past a heading"
    );
    assert_eq!(
        panel::step("c", false, &lines),
        None,
        "below the open lines"
    );
    assert_eq!(
        panel::step("b", true, &lines),
        None,
        "a done item does not step"
    );
    assert_eq!(panel::step("heading:active", false, &lines), None);
    assert_eq!(panel::step("nowhere", false, &lines), None);
}

/// The steps are taken over the list's open lines, as if no section were folded, nothing were
/// settling and Done were closed, whatever the pane draws (M85 lists §3.6, §6.1).
#[test]
fn move_up_and_move_down_step_over_the_open_lines_with_nothing_folded_or_settling() {
    let list = fermix();
    let after = |id: &str| Some(Place::After(id.into()));
    let drawn = View {
        done_open: true,
        settling: BTreeSet::from(["b".to_owned()]),
        ..View::default()
    };

    assert_eq!(
        panel::steps(&list, "a"),
        Steps {
            up: after("loose"),
            down: after("later"),
        }
    );
    let lines = panel::lines(&list, &drawn);
    assert_eq!(
        panel::step("b", true, &lines),
        after("active"),
        "drawn, b keeps its place while it settles, and would step"
    );
    assert_eq!(
        panel::steps(&list, "b"),
        Steps::default(),
        "a done item does not step, settling or not"
    );
    assert_eq!(
        panel::steps(&list, "loose"),
        Steps {
            up: None,
            down: after("active"),
        },
        "the top of a headless group"
    );
    assert_eq!(
        panel::steps(&list, "c"),
        Steps {
            up: after("a"),
            down: None,
        },
        "nothing below the last open line"
    );
    assert_eq!(panel::steps(&list, "active"), Steps::default());
}

/// A step into a folded section lands where it would with the section open, and the item goes
/// out of sight under that section's heading, which takes the focus; a step into a section drawn
/// open, or into the first group, hides nothing.
#[test]
fn a_step_into_a_folded_section_lands_as_if_it_were_open_and_goes_under_its_heading() {
    let list = fermix();
    let folded = |id: &str| View {
        folded: BTreeSet::from([id.to_owned()]),
        ..View::default()
    };
    let up = panel::steps(&list, "c").up.expect("c steps up");
    assert_eq!(up, Place::After("a".into()), "the end of active");
    let active = folded("active");
    assert!(!line_ids(&panel::lines(&list, &active)).contains(&"a".to_owned()));
    assert_eq!(
        panel::hidden_under(&list, &active, &up).as_deref(),
        Some("heading:active")
    );
    let down = panel::steps(&list, "a").down.expect("a steps down");
    assert_eq!(down, Place::After("later".into()), "first in later");
    assert_eq!(
        panel::hidden_under(&list, &folded("later"), &down).as_deref(),
        Some("heading:later")
    );
    assert_eq!(panel::hidden_under(&list, &folded("later"), &up), None);
    assert_eq!(panel::hidden_under(&list, &View::default(), &up), None);
    let loose = Place::After("loose".into());
    assert_eq!(panel::hidden_under(&list, &active, &loose), None);
}

/// A row the engine has not named yet takes no step, and no step lands after one, up or down; a
/// step past one to a named row stays on (§5.3).
#[test]
fn a_row_the_engine_has_not_named_neither_steps_nor_takes_a_step_that_lands_after_it() {
    let sending = "local:linux-0000000000000001";
    let between = list(
        "ck_a",
        vec![item("x", false), item(sending, false), item("y", false)],
    );
    let list = list(
        "ck_a",
        vec![item(sending, false), item("y", false), item("w", false)],
    );
    let after = |id: &str| Some(Place::After(id.into()));

    assert_eq!(panel::steps(&list, sending), Steps::default());
    assert_eq!(
        panel::steps(&list, "w"),
        Steps {
            up: None,
            down: None,
        },
        "up would land after the sending row"
    );
    assert_eq!(panel::steps(&list, "y").down, after("w"));
    assert_eq!(
        panel::steps(&between, "x").down,
        None,
        "down would land after the sending row"
    );
    assert_eq!(
        panel::steps(&between, "y").up,
        after("x"),
        "past the sending row to a named one"
    );
}

#[test]
fn an_added_item_goes_to_the_selected_items_section_else_the_first_group() {
    let list = fermix();
    let section = |id: &str| Some(Place::Section(id.into()));

    assert_eq!(panel::add_place(&list, Some("a")), section("active"));
    assert_eq!(
        panel::add_place(&list, Some("b")),
        section("active"),
        "a done item too"
    );
    assert_eq!(panel::add_place(&list, Some("c")), section("later"));
    assert_eq!(panel::add_place(&list, Some("loose")), None);
    assert_eq!(panel::add_place(&list, None), None);
    assert_eq!(
        panel::add_place(&list, Some("active")),
        None,
        "a heading is no item"
    );
}

#[test]
fn a_tick_splices_one_line_and_a_push_that_changed_nothing_splices_none() {
    let before = panel::lines(&fermix(), &View::default());
    assert_eq!(panel::splice(&before, &before), None);

    let mut ticked = fermix();
    ticked.rows[2].done = Some(true);
    let view = View {
        settling: BTreeSet::from(["a".to_owned()]),
        ..View::default()
    };
    let after = panel::lines(&ticked, &view);
    let splice = panel::splice(&before, &after).expect("a change");
    assert_eq!((splice.at, splice.removed), (2, 1));
    assert_eq!(splice.added, [Line::Item(ticked.rows[2].clone())]);

    let settled = panel::lines(&ticked, &View::default());
    let splice = panel::splice(&after, &settled).expect("the row folds away");
    let mut rebuilt = after.clone();
    let range = splice.at..splice.at + splice.removed;
    let _gone: Vec<Line> = rebuilt.splice(range, splice.added).collect();
    assert_eq!(rebuilt, settled);
}

// ---- the picker and the list the pane shows (M85 lists §3.3) ----

#[test]
fn the_picker_names_each_folder_over_its_lists_in_index_order_then_the_lists_in_no_folder() {
    let folders = Folders::of(&[
        entry("ck_a", "Fermix", Some("Projects")),
        entry("ck_b", "Groceries", None),
        entry("ck_c", "Packing", Some("Trips")),
        entry("ck_d", "Portfolio site", Some("Projects")),
    ]);

    assert_eq!(folders.names(), ["Projects", "Trips"]);
    let titles: Vec<Vec<&str>> = folders
        .folders
        .iter()
        .map(|folder| folder.lists.iter().map(|l| l.title.as_str()).collect())
        .collect();
    assert_eq!(titles, [vec!["Fermix", "Portfolio site"], vec!["Packing"]]);
    let loose: Vec<&str> = folders.loose.iter().map(|l| l.title.as_str()).collect();
    assert_eq!(loose, ["Groceries"]);
}

#[test]
fn an_item_moves_to_the_other_lists_by_folder_in_picker_order() {
    let folders = Folders::of(&[
        entry("ck_a", "Fermix", Some("Projects")),
        entry("ck_b", "Groceries", None),
        entry("ck_c", "Packing", Some("Trips")),
        entry("ck_d", "Portfolio site", Some("Projects")),
    ]);
    let titles = |folders: &Folders| -> (Vec<(String, Vec<String>)>, Vec<String>) {
        let named = folders
            .folders
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.lists.iter().map(|l| l.title.clone()).collect(),
                )
            })
            .collect();
        let loose = folders.loose.iter().map(|l| l.title.clone()).collect();
        (named, loose)
    };

    let others = folders.without("ck_a");
    assert_eq!(
        titles(&others),
        (
            vec![
                ("Projects".to_owned(), vec!["Portfolio site".to_owned()]),
                ("Trips".to_owned(), vec!["Packing".to_owned()]),
            ],
            vec!["Groceries".to_owned()]
        )
    );
    assert!(!others.is_empty());
    let others = folders.without("ck_c");
    assert_eq!(
        others.names(),
        ["Projects"],
        "a folder with no other list is left out"
    );
    assert_eq!(titles(&folders.without("ck_b")).1, Vec::<String>::new());
    assert!(
        !folders.without("ck_b").is_empty(),
        "lists in folders, none loose"
    );

    let alone = Folders::of(&[entry("ck_a", "Fermix", Some("Projects"))]);
    assert!(alone.without("ck_a").is_empty(), "no other list");
    assert_eq!(alone.without("ck_a"), Folders::default());
}

/// The other lists keep the index's order, folders and lists alike, never the alphabet's.
#[test]
fn the_other_lists_keep_the_index_order_not_the_alphabets() {
    let folders = Folders::of(&[
        entry("ck_z", "Zebra", Some("Trips")),
        entry("ck_y", "Yarn", None),
        entry("ck_m", "Mail", Some("Home")),
        entry("ck_a", "Apples", Some("Trips")),
        entry("ck_b", "Bills", None),
    ]);
    let others = folders.without("ck_m");
    assert_eq!(others.names(), ["Trips"]);
    let titles = |lists: &[ListEntry]| -> Vec<String> {
        lists.iter().map(|list| list.title.clone()).collect()
    };
    assert_eq!(titles(&others.folders[0].lists), ["Zebra", "Apples"]);
    assert_eq!(titles(&others.loose), ["Yarn", "Bills"]);
    let others = folders.without("ck_y");
    assert_eq!(others.names(), ["Trips", "Home"]);
}

#[test]
fn the_pane_shows_the_list_asked_for_while_the_index_holds_it_else_the_first_in_picker_order() {
    let mut lists = Library::default();
    assert_eq!(lists.index().shown(None), None, "nothing before the index");
    lists.negotiated(7);
    lists.received_index(vec![
        entry("ck_b", "Groceries", None),
        entry("ck_a", "Fermix", Some("Projects")),
    ]);
    let shown = |lists: &Library, asked| lists.index().shown(asked).map(|e| e.list_id.clone());

    assert_eq!(
        shown(&lists, None).as_deref(),
        Some("ck_a"),
        "the first folder's list"
    );
    assert_eq!(shown(&lists, Some("ck_b")).as_deref(), Some("ck_b"));
    assert_eq!(shown(&lists, Some("ck_gone")).as_deref(), Some("ck_a"));

    lists.received_index(vec![entry("ck_b", "Groceries", None)]);
    assert_eq!(shown(&lists, Some("ck_a")).as_deref(), Some("ck_b"));
    lists.received_index(Vec::new());
    assert_eq!(shown(&lists, None), None, "no lists yet");
}
