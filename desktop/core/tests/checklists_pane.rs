//! The Lists pane as values (M85 lists §3.2, §3.3, §3.4, §3.5, §3.8; §8 "The pane's open
//! state"): whether it is shown, wide and narrow, the owner's last open or close kept in
//! `lists.json`, what it draws from the lists as the library holds them, from the index to a tick
//! under Done and an add in the selected item's section, an item being reworded, each list's own
//! folds, and what a screen reader is told.

use fermix_client::chat_wire::protocol::{read_line, ClientEvent, ListOp, ServerEvent};
use fermix_client::checklists::draft::{self, is_local, Place};
use fermix_client::checklists::library::{Library, MAX_WAITING};
use fermix_client::checklists::pane::{
    self, drawing, lists_path, Folds, NotLoaded, PaneBody, PaneDrawing, PaneList, PaneOpen,
};
use fermix_client::checklists::panel::{self, add_place, splice, Line};
use fermix_client::checklists::routing::lists_event;
use fermix_client::checklists::words;
use serde_json::error::Category;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::ErrorKind;
use std::path::Path;

const GROCERIES: &str = "ck_4q8m2x7v1d";
const PACKING: &str = "ck_2b7n5r8c3j";
const ERRANDS: &str = "ck_7t3w9k1p5z";
const PASSPORT: &str = "ci_1a2b3c4d5e";
const CLOTHES: &str = "ci_6f7g8h9j0k";
const SOCKS: &str = "ci_2m3n4p5q6r";
const SHIRT: &str = "ci_7s8t9v0w1x";
const STAMPS: &str = "ci_8a9b0c1d2e";
const POST: &str = "ci_3f4g5h6j7k";

fn event(line: Value) -> ServerEvent {
    read_line(line.to_string().as_bytes()).expect("a framed line")
}

/// Feeds the daemon's line to the lists, as the chat session does, and returns what they send.
fn receive(lists: &mut Library, line: Value) -> Vec<ClientEvent> {
    lists_event(lists, &event(line)).expect("the lists take it")
}

/// The index: Errands in no folder, then Groceries and Packing in Home.
fn index() -> Value {
    json!({"type": "checklists", "profile_id": "main", "lists": [
        {"list_id": ERRANDS, "title": "Errands", "open": 1, "done": 0, "rev": 1},
        {"list_id": GROCERIES, "title": "Groceries", "folder": "Home", "open": 6, "done": 2,
         "rev": 4},
        {"list_id": PACKING, "title": "Packing", "folder": "Home", "open": 2, "done": 1, "rev": 1}
    ]})
}

/// Packing: Passport in its first group, then Clothes over Socks and a done Shirt.
fn packing(rev: u64, removed_at: Option<&str>) -> Value {
    json!({"type": "checklist", "profile_id": "main", "list": {
        "list_id": PACKING, "title": "Packing", "rev": rev, "removed_at": removed_at, "rows": [
        {"row_id": PASSPORT, "kind": "item", "text": "Passport", "done": false},
        {"row_id": CLOTHES, "kind": "section", "text": "Clothes"},
        {"row_id": SOCKS, "kind": "item", "text": "Socks", "done": false},
        {"row_id": SHIRT, "kind": "item", "text": "Shirt", "done": true}
    ]}})
}

/// Errands: Stamps, and a done Post.
fn errands() -> Value {
    json!({"type": "checklist", "profile_id": "main", "list": {
        "list_id": ERRANDS, "title": "Errands", "rev": 1, "rows": [
        {"row_id": STAMPS, "kind": "item", "text": "Stamps", "done": false},
        {"row_id": POST, "kind": "item", "text": "Post", "done": true}
    ]}})
}

/// Connected to a daemon that serves lists, the index answered.
fn indexed() -> Library {
    let mut lists = Library::default();
    lists.negotiated(7);
    receive(&mut lists, index());
    lists
}

/// What the pane shows asked for `asked`, drawn by `folds`, with nothing settling.
fn shown_list(lists: &Library, asked: Option<&str>, folds: &Folds) -> PaneList {
    shown_settling(lists, asked, folds, &BTreeSet::new())
}

fn shown_settling(
    lists: &Library,
    asked: Option<&str>,
    folds: &Folds,
    settling: &BTreeSet<String>,
) -> PaneList {
    match drawing(lists, asked, folds, settling) {
        PaneDrawing::List(list) => *list,
        other => panic!("a list: {other:?}"),
    }
}

/// The lines of `list_id` as the pane draws it.
fn lines_of(
    lists: &Library,
    list_id: &str,
    folds: &Folds,
    settling: &BTreeSet<String>,
) -> Vec<Line> {
    match shown_settling(lists, Some(list_id), folds, settling).body {
        PaneBody::Lines(lines) => lines,
        other => panic!("its lines: {other:?}"),
    }
}

/// Packing's lines, drawn by `folds`, the rows in `settling` keeping their place.
fn lines(lists: &Library, folds: &Folds, settling: &BTreeSet<String>) -> Vec<Line> {
    lines_of(lists, PACKING, folds, settling)
}

fn ids(lines: &[Line]) -> Vec<String> {
    lines.iter().map(Line::id).collect()
}

fn pulls(sent: &[ClientEvent]) -> Vec<&str> {
    let pulled = sent.iter().filter_map(|event| match event {
        ClientEvent::ChecklistPull { list_id, .. } => Some(list_id.as_str()),
        _ => None,
    });
    pulled.collect()
}

#[test]
fn wide_the_pane_is_shown_as_the_owner_last_chose() {
    assert!(
        !PaneOpen::new(false).shown(),
        "closed when nothing was kept"
    );
    assert!(PaneOpen::new(true).shown(), "a kept open shows at start");
    let mut open = PaneOpen::new(false);
    open.toggle();
    assert!(open.shown() && open.chosen());
    open.toggle();
    assert!(!open.shown() && !open.chosen());
    open.choose(true);
    assert!(open.shown() && open.chosen(), "a card opens it");
}

#[test]
fn narrowing_hides_the_pane_and_keeps_the_choice_and_widening_shows_it_again() {
    let mut open = PaneOpen::new(true);
    open.collapse(true);
    assert!(!open.shown(), "narrow, it shows only when asked");
    assert!(open.chosen(), "the choice is kept");
    open.collapse(false);
    assert!(open.shown(), "widening shows the kept open");
    let mut closed = PaneOpen::new(false);
    closed.collapse(true);
    closed.collapse(false);
    assert!(!closed.shown(), "and keeps a close closed");
}

#[test]
fn an_open_or_close_while_narrow_is_the_owners_choice_like_any_other() {
    let mut open = PaneOpen::new(false);
    open.chat_laid_out();
    open.collapse(true);
    open.toggle();
    assert!(open.shown() && open.chosen(), "opened over the narrow chat");
    open.collapse(false);
    assert!(open.shown(), "and kept open when widened");
    open.collapse(true);
    assert!(!open.shown(), "narrowing hides it again");
    open.choose(true);
    assert!(
        open.widget_shown(false),
        "Escape or a click on the chat closes it: the owner's choice"
    );
    assert!(!open.shown() && !open.chosen());
    open.collapse(false);
    assert!(!open.shown(), "a close made narrow holds wide");
}

/// The chat learns it narrowed only as it is laid out, and it is not while another page shows.
/// An open asked for then, as Ctrl+Shift+L from another page asks, survives the narrowing the
/// chat's first layout brings; once that layout is drawn, a narrowing hides the pane as before.
/// With no ask, coming back to a chat that narrowed while away shows the pane narrow only if
/// asked, as any narrowing does.
#[test]
fn an_open_asked_while_the_chat_is_off_screen_survives_its_first_layout_narrow() {
    let mut open = PaneOpen::new(false);
    open.chat_laid_out();
    open.chat_left();
    open.choose(true);
    open.collapse(true);
    assert!(
        open.shown(),
        "Ctrl+Shift+L from another page, the window narrowed meanwhile"
    );
    open.chat_laid_out();
    open.collapse(false);
    open.collapse(true);
    assert!(!open.shown(), "a narrowing after the first layout hides it");
    assert!(open.chosen());
    let mut away = PaneOpen::new(true);
    away.chat_laid_out();
    away.chat_left();
    away.collapse(true);
    assert!(
        !away.shown(),
        "back on a chat that narrowed while away, with no ask"
    );
    let mut start = PaneOpen::new(false);
    start.choose(true);
    start.collapse(true);
    assert!(start.shown(), "an ask before the chat's very first layout");
    let mut left = PaneOpen::new(false);
    left.chat_laid_out();
    left.choose(true);
    left.chat_left();
    left.collapse(true);
    assert!(
        !left.shown(),
        "an ask made on Chat before it left is no ask from away"
    );
}

#[test]
fn the_split_showing_or_hiding_as_the_app_asked_is_no_choice() {
    let mut open = PaneOpen::new(true);
    assert!(!open.widget_shown(true));
    open.collapse(true);
    assert!(!open.widget_shown(false), "the narrowing hid it");
    assert!(open.chosen());
}

#[test]
fn the_choice_is_kept_under_the_state_home_beside_the_updaters() {
    let home = Path::new("/home/ada");
    assert_eq!(
        lists_path(Some(OsStr::new("/var/state")), home),
        Path::new("/var/state/fermix-desktop/lists.json")
    );
    for ignored in [None, Some(OsStr::new("")), Some(OsStr::new("relative"))] {
        assert_eq!(
            lists_path(ignored, home),
            Path::new("/home/ada/.local/state/fermix-desktop/lists.json"),
            "{ignored:?}"
        );
    }
}

#[test]
fn the_kept_choice_round_trips_whole_and_a_missing_file_reads_as_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fermix-desktop/lists.json");
    assert_eq!(pane::load(&path), Ok(false), "nothing kept yet");
    pane::save(&path, true).unwrap();
    assert_eq!(pane::load(&path), Ok(true));
    pane::save(&path, false).unwrap();
    assert_eq!(pane::load(&path), Ok(false));
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text, "{\n  \"open\": false\n}");
    let folder = dir.path().join("fermix-desktop");
    let names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["lists.json"], "no temporary file is left");
}

#[test]
fn a_file_that_does_not_load_is_a_code_never_the_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lists.json");
    std::fs::write(&path, "{\"open\": ").unwrap();
    assert_eq!(pane::load(&path), Err(NotLoaded::Parse(Category::Eof)));
    std::fs::write(&path, "{\"open\": \"yes\"}").unwrap();
    assert_eq!(pane::load(&path), Err(NotLoaded::Parse(Category::Data)));
    let folder = dir.path().join("folder.json");
    std::fs::create_dir(&folder).unwrap();
    let unread = pane::load(&folder);
    assert_eq!(unread, Err(NotLoaded::Read(ErrorKind::IsADirectory)));
    let logged = format!("{unread:?}");
    assert!(!logged.contains("folder"), "{logged}");
}

#[test]
fn before_the_index_answers_the_pane_draws_its_head_alone_and_with_none_it_has_no_lists() {
    let mut lists = Library::default();
    assert_eq!(
        drawing(&lists, None, &Folds::default(), &BTreeSet::new()),
        PaneDrawing::Waiting
    );
    lists.negotiated(7);
    assert_eq!(
        drawing(&lists, Some(PACKING), &Folds::default(), &BTreeSet::new()),
        PaneDrawing::Waiting
    );
    receive(
        &mut lists,
        json!({"type": "checklists", "profile_id": "main", "lists": []}),
    );
    assert_eq!(
        drawing(&lists, Some(PACKING), &Folds::default(), &BTreeSet::new()),
        PaneDrawing::NoLists
    );
}

#[test]
fn the_pane_shows_the_list_asked_for_while_the_index_names_it_else_the_first_in_picker_order() {
    let lists = indexed();
    let folds = Folds::default();
    let first = shown_list(&lists, None, &folds);
    assert_eq!(
        first.list_id, GROCERIES,
        "the first folder's first list leads the picker"
    );
    assert_eq!(shown_list(&lists, Some(ERRANDS), &folds).list_id, ERRANDS);
    let gone = shown_list(&lists, Some("ck_0000000000"), &folds);
    assert_eq!(
        gone.list_id, GROCERIES,
        "one the index lacks turns to the first"
    );
    let names = first.folders.names();
    assert_eq!(names, ["Home"]);
    let loose: Vec<&str> = first
        .folders
        .loose
        .iter()
        .map(|e| e.title.as_str())
        .collect();
    assert_eq!(loose, ["Errands"]);
}

#[test]
fn a_list_not_pulled_yet_draws_its_head_from_the_index_and_no_lines() {
    let lists = indexed();
    let shown = shown_list(&lists, None, &Folds::default());
    assert_eq!(shown.title, "Groceries");
    assert_eq!(shown.head, "Home · 6 open · 2 done");
    assert_eq!(shown.spoken_head, "Home, 6 open, 2 done");
    assert_eq!(shown.body, PaneBody::Waiting);
    assert_eq!(shown.refusal, None);
    let loose = shown_list(&lists, Some(ERRANDS), &Folds::default());
    assert_eq!(
        loose.head, "1 open · 0 done",
        "the counts alone in no folder"
    );
    assert_eq!(loose.spoken_head, "1 open, 0 done");
}

#[test]
fn the_head_reads_the_folder_and_counts_to_the_eye_and_to_a_screen_reader() {
    assert_eq!(
        words::head_line(Some("Projects"), 6, 2),
        "Projects · 6 open · 2 done"
    );
    assert_eq!(
        words::spoken_head_line(Some("Projects"), 6, 2),
        "Projects, 6 open, 2 done"
    );
    assert_eq!(words::spoken_head_line(None, 6, 2), "6 open, 2 done");
}

#[test]
fn a_held_list_draws_its_lines_and_its_head_counts_the_edits_waiting() {
    let mut lists = indexed();
    assert_eq!(pulls(&lists.draw(PACKING, false)), [PACKING]);
    receive(&mut lists, packing(1, None));
    let folds = Folds::default();
    let held = lines(&lists, &folds, &BTreeSet::new());
    let heading = format!("heading:{CLOTHES}");
    assert_eq!(ids(&held), [PASSPORT, &heading, SOCKS, "done"]);
    assert_eq!(
        held[3],
        Line::DoneHeading {
            count: 1,
            open: false
        }
    );
    let check = ListOp::Check {
        row_id: SOCKS.into(),
    };
    lists.edit(PACKING, check, "linux-t1".into());
    let shown = shown_list(&lists, Some(PACKING), &folds);
    assert_eq!(shown.head, "Home · 1 open · 2 done", "drawn at once");
}

#[test]
fn a_removed_list_says_so_and_a_refused_edit_shows_the_engines_sentence() {
    let mut lists = indexed();
    lists.draw(PACKING, false);
    receive(&mut lists, packing(1, None));
    let check = ListOp::Check {
        row_id: SOCKS.into(),
    };
    lists.edit(PACKING, check, "linux-t1".into());
    let sentence = "That row is not in this list any more; pull the list again.";
    let refused = json!({"type": "error", "reason": "request_failed", "message": sentence,
        "client_msg_id": "linux-t1"});
    assert_eq!(pulls(&receive(&mut lists, refused)), [PACKING]);
    let shown = shown_list(&lists, Some(PACKING), &Folds::default());
    assert_eq!(shown.refusal.as_deref(), Some(sentence));
    assert_eq!(shown.head, "Home · 2 open · 1 done", "the row is back");
    receive(&mut lists, packing(2, Some("2026-11-03T09:00:00Z")));
    let removed = shown_list(&lists, Some(PACKING), &Folds::default());
    assert_eq!(removed.body, PaneBody::Removed);
}

/// The pane opened at a list by a card: it pulls it, draws its lines, a tick keeps its place
/// while it settles and then goes under Done in one splice, and an add goes to the selected
/// item's section, drawn at once as sending.
#[test]
fn the_pane_pulls_what_it_shows_settles_a_tick_under_done_and_adds_to_the_selected_section() {
    let mut lists = indexed();
    let mut open = PaneOpen::new(false);
    open.choose(true);
    assert!(open.shown(), "a card's open shows the pane");
    let asked = Some(PACKING);
    assert_eq!(
        shown_list(&lists, asked, &Folds::default()).body,
        PaneBody::Waiting
    );
    assert_eq!(pulls(&lists.draw(PACKING, false)), [PACKING]);
    assert!(lists.draw(PACKING, false).is_empty(), "pulled once");
    receive(&mut lists, packing(1, None));
    let folds = Folds::default();
    let mut settling = BTreeSet::new();
    let before = lines(&lists, &folds, &settling);
    let check = ListOp::Check {
        row_id: SOCKS.into(),
    };
    settling.insert(SOCKS.to_owned());
    lists.edit(PACKING, check, "linux-t1".into());
    let kept = lines(&lists, &folds, &settling);
    assert_eq!(ids(&kept), ids(&before), "it keeps its place");
    settling.remove(SOCKS);
    let settled = lines(&lists, &folds, &settling);
    let heading = format!("heading:{CLOTHES}");
    assert_eq!(ids(&settled), [PASSPORT, &heading, "done"]);
    let one = splice(&kept, &settled).expect("a change");
    assert_eq!((one.at, one.removed), (2, 2), "Socks and the old Done go");
    assert_eq!(
        one.added,
        [Line::DoneHeading {
            count: 2,
            open: false
        }]
    );
    let list = lists.shown(PACKING).expect("held");
    let place = add_place(&list, Some(PASSPORT));
    assert_eq!(
        place, None,
        "the first group's item adds to the first group"
    );
    let place = add_place(&list, Some(SOCKS));
    assert_eq!(place, Some(Place::Section(CLOTHES.into())));
    let (section_id, after_id) = Place::fields(place);
    let add = ListOp::Add {
        text: "Hat".into(),
        note: None,
        section_id,
        after_id,
    };
    lists.edit(PACKING, add, "linux-a1".into());
    let added = lines(&lists, &folds, &settling);
    let [_, _, hat, _] = added.as_slice() else {
        panic!("Hat in Clothes: {added:?}");
    };
    let hat = hat.editable().expect("an item");
    assert!(
        is_local(&hat.row_id) && hat.text == "Hat",
        "drawn as sending"
    );
}

/// Each list keeps its own folds while the app runs (§3.4): a section folded and Done opened in
/// Packing are so again when the pane comes back to it, and Errands, shown between, has its Done
/// closed until it is opened there.
#[test]
fn each_list_keeps_its_own_folded_sections_and_done() {
    let mut lists = indexed();
    lists.draw(PACKING, false);
    receive(&mut lists, packing(1, None));
    lists.draw(ERRANDS, false);
    receive(&mut lists, errands());
    let none = BTreeSet::new();
    let mut folds = Folds::default();
    folds.fold(PACKING, CLOTHES);
    folds.open_done(PACKING);
    let heading = format!("heading:{CLOTHES}");
    let packing = lines(&lists, &folds, &none);
    assert_eq!(ids(&packing), [PASSPORT, &heading, "done", SHIRT]);
    let errands = lines_of(&lists, ERRANDS, &folds, &none);
    assert_eq!(ids(&errands), [STAMPS, "done"], "Errands' Done closed");
    folds.open_done(ERRANDS);
    let errands = lines_of(&lists, ERRANDS, &folds, &none);
    assert_eq!(ids(&errands), [STAMPS, "done", POST]);
    folds.fold(PACKING, CLOTHES);
    let packing = lines(&lists, &folds, &none);
    assert_eq!(
        ids(&packing),
        [PASSPORT, &heading, SOCKS, "done", SHIRT],
        "Done still open"
    );
}

/// Packing's lines with `row_id` being reworded.
fn rewording(lists: &Library, folds: &Folds, row_id: &str) -> Vec<Line> {
    let drawn = pane::rewording(
        drawing(lists, Some(PACKING), folds, &BTreeSet::new()),
        Some(row_id),
    );
    let PaneDrawing::List(list) = drawn else {
        panic!("a list: {drawn:?}");
    };
    match list.body {
        PaneBody::Lines(lines) => lines,
        other => panic!("its lines: {other:?}"),
    }
}

/// An item being reworded is drawn as two entries in its place, its words then its note (§3.5),
/// an open item and a done one alike. A heading, a row the engine has not named yet, and an item
/// not drawn (in a folded section) are never reworded; the entries are no item to tick or to add
/// after.
#[test]
fn an_item_being_reworded_is_drawn_as_its_words_then_its_note_in_its_place() {
    let mut lists = indexed();
    lists.draw(PACKING, false);
    receive(&mut lists, packing(1, None));
    let mut folds = Folds::default();
    let heading = format!("heading:{CLOTHES}");
    let (words, note) = (format!("words:{SOCKS}"), format!("note:{SOCKS}"));
    let socks = rewording(&lists, &folds, SOCKS);
    assert_eq!(ids(&socks), [PASSPORT, &heading, &words, &note, "done"]);
    assert_eq!(
        socks[2..4],
        [Line::Words(SOCKS.into()), Line::Note(SOCKS.into())]
    );
    assert!(socks[2].editable().is_none() && socks[3].editable().is_none());
    assert!(panel::rewords(&socks, SOCKS) && !panel::rewords(&socks, PASSPORT));
    folds.open_done(PACKING);
    let shirt = rewording(&lists, &folds, SHIRT);
    let (words, note) = (format!("words:{SHIRT}"), format!("note:{SHIRT}"));
    assert_eq!(
        ids(&shirt),
        [PASSPORT, &heading, SOCKS, "done", &words, &note]
    );
    let as_drawn = lines(&lists, &folds, &BTreeSet::new());
    assert_eq!(rewording(&lists, &folds, CLOTHES), as_drawn, "a heading");
    let add = ListOp::Add {
        text: "Hat".into(),
        note: None,
        section_id: None,
        after_id: None,
    };
    lists.edit(PACKING, add, "linux-a1".into());
    let sending = draft::local_row_id("linux-a1");
    let drawn = lines(&lists, &folds, &BTreeSet::new());
    assert_eq!(rewording(&lists, &folds, &sending), drawn, "sending");
    folds.fold(PACKING, CLOTHES);
    let folded = rewording(&lists, &folds, SOCKS);
    assert!(!panel::rewords(&folded, SOCKS), "not drawn, not reworded");
}

/// A push while an item is reworded leaves its entries where they are when it changes the item
/// itself or the lines around them, so what is typed stays; a push that takes the item away
/// ends the rewording (§3.5, §3.4).
#[test]
fn a_push_mid_rewording_keeps_its_entries_and_one_without_the_item_ends_it() {
    let mut lists = indexed();
    lists.draw(PACKING, false);
    receive(&mut lists, packing(1, None));
    let folds = Folds::default();
    let before = rewording(&lists, &folds, SOCKS);
    let reworded_elsewhere = json!({"type": "checklist", "profile_id": "main", "list": {
        "list_id": PACKING, "title": "Packing", "rev": 2, "rows": [
        {"row_id": PASSPORT, "kind": "item", "text": "Passport", "note": "Renewed", "done": false},
        {"row_id": CLOTHES, "kind": "section", "text": "Clothes"},
        {"row_id": SOCKS, "kind": "item", "text": "Wool socks", "note": "Two", "done": false},
        {"row_id": SHIRT, "kind": "item", "text": "Shirt", "done": true}
    ]}});
    receive(&mut lists, reworded_elsewhere);
    let after = rewording(&lists, &folds, SOCKS);
    let change = splice(&before, &after).expect("Passport changed");
    assert_eq!(
        (change.at, change.removed),
        (0, 1),
        "Passport alone is redrawn"
    );
    let taken = json!({"type": "checklist", "profile_id": "main", "list": {
        "list_id": PACKING, "title": "Packing", "rev": 3, "rows": [
        {"row_id": PASSPORT, "kind": "item", "text": "Passport", "done": false},
        {"row_id": CLOTHES, "kind": "section", "text": "Clothes"},
        {"row_id": SHIRT, "kind": "item", "text": "Shirt", "done": true}
    ]}});
    receive(&mut lists, taken);
    let gone = rewording(&lists, &folds, SOCKS);
    assert!(!panel::rewords(&gone, SOCKS), "the rewording ends");
    let heading = format!("heading:{CLOTHES}");
    assert_eq!(ids(&gone), [PASSPORT, &heading, "done"]);
}

/// A refused edit's sentence stays on its list until the list's next edit, and the pane tells it
/// only where it first shows it: as it comes over Add an item of the list the pane showed, on
/// screen. Turned to a list with an older sentence on it, first drawn, or off screen, the pane
/// says nothing of it (§5.3, §6.3).
#[test]
fn the_pane_tells_a_refusal_only_as_it_comes_to_the_list_it_shows() {
    let mut lists = indexed();
    for (id, list) in [(PACKING, packing(1, None)), (ERRANDS, errands())] {
        lists.draw(id, false);
        receive(&mut lists, list);
    }
    let folds = Folds::default();
    let none = BTreeSet::new();
    let packing_before = drawing(&lists, Some(PACKING), &folds, &none);
    let errands_before = drawing(&lists, Some(ERRANDS), &folds, &none);
    let check = ListOp::Check {
        row_id: SOCKS.into(),
    };
    lists.edit(PACKING, check, "linux-t1".into());
    let sentence = "That row is not in this list any more; pull the list again.";
    let refused = json!({"type": "error", "reason": "request_failed", "message": sentence,
        "client_msg_id": "linux-t1"});
    receive(&mut lists, refused);
    let refused = drawing(&lists, Some(PACKING), &folds, &none);
    assert_eq!(
        pane::news(Some(&packing_before), &refused, true),
        [sentence]
    );
    for (was, why) in [
        (Some(&refused), "told already"),
        (Some(&errands_before), "turned to Packing"),
        (None, "first drawn"),
    ] {
        assert!(pane::news(was, &refused, true).is_empty(), "{why}");
    }
    assert!(
        pane::news(Some(&packing_before), &refused, false).is_empty(),
        "off screen"
    );
    let (removed, removed_refused) = refused_once_removed(sentence);
    assert!(
        pane::news(Some(&removed), &removed_refused, true).is_empty(),
        "removed, with no Add an item to show it over"
    );
}

/// Packing drawn removed while a tick of Socks waits, then drawn again with the tick refused in
/// `sentence`.
fn refused_once_removed(sentence: &str) -> (PaneDrawing, PaneDrawing) {
    let mut lists = indexed();
    lists.draw(PACKING, false);
    receive(&mut lists, packing(1, None));
    let check = ListOp::Check {
        row_id: SOCKS.into(),
    };
    lists.edit(PACKING, check, "linux-t3".into());
    receive(&mut lists, packing(2, Some("2026-11-03T09:00:00Z")));
    let (folds, none) = (Folds::default(), BTreeSet::new());
    let removed = drawing(&lists, Some(PACKING), &folds, &none);
    let refused = json!({"type": "error", "reason": "request_failed", "message": sentence,
        "client_msg_id": "linux-t3"});
    receive(&mut lists, refused);
    let refused = drawing(&lists, Some(PACKING), &folds, &none);
    let PaneDrawing::List(list) = &refused else {
        panic!("Packing is drawn");
    };
    assert_eq!(list.body, PaneBody::Removed);
    assert_eq!(list.refusal.as_deref(), Some(sentence));
    (removed, refused)
}

/// A sentence on the list the pane shows, replaced by another with no draw between, is told: an
/// engine's refusal, then an edit refused for the 64 edits already waiting (§3.8, §6.3).
#[test]
fn a_sentence_the_pane_shows_replaced_by_another_is_told() {
    let mut lists = indexed();
    for (id, list) in [(PACKING, packing(1, None)), (ERRANDS, errands())] {
        lists.draw(id, false);
        receive(&mut lists, list);
    }
    let (folds, none) = (Folds::default(), BTreeSet::new());
    let check = ListOp::Check {
        row_id: SOCKS.into(),
    };
    lists.edit(PACKING, check.clone(), "linux-t1".into());
    let sentence = "That row is not in this list any more; pull the list again.";
    let refused = json!({"type": "error", "reason": "request_failed", "message": sentence,
        "client_msg_id": "linux-t1"});
    receive(&mut lists, refused);
    let refused = drawing(&lists, Some(PACKING), &folds, &none);
    for n in 0..MAX_WAITING {
        let note = ListOp::Edit {
            row_id: STAMPS.into(),
            text: None,
            note: Some(format!("note {n}")),
        };
        lists.edit(ERRANDS, note, format!("linux-e{n}"));
    }
    lists.edit(PACKING, check, "linux-t2".into());
    let too_many = drawing(&lists, Some(PACKING), &folds, &none);
    assert_eq!(
        pane::news(Some(&refused), &too_many, true),
        [words::TOO_MANY]
    );
}

/// "This list was removed." is told once, when the list the pane shows turns removed or the pane
/// turns to a removed list, on screen (§3.8, §6.3).
#[test]
fn the_pane_tells_a_removed_list_once_as_it_shows_it() {
    let mut lists = indexed();
    lists.draw(PACKING, false);
    receive(&mut lists, packing(1, None));
    let folds = Folds::default();
    let none = BTreeSet::new();
    let drawn = drawing(&lists, Some(PACKING), &folds, &none);
    let errands = drawing(&lists, Some(ERRANDS), &folds, &none);
    receive(&mut lists, packing(2, Some("2026-11-03T09:00:00Z")));
    let removed = drawing(&lists, Some(PACKING), &folds, &none);
    for was in [&drawn, &errands] {
        assert_eq!(pane::news(Some(was), &removed, true), [words::LIST_REMOVED]);
    }
    assert!(
        pane::news(Some(&removed), &removed, true).is_empty(),
        "told already"
    );
    assert!(
        pane::news(Some(&drawn), &removed, false).is_empty(),
        "off screen"
    );
}
