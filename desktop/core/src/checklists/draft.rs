//! An edit drawn before the engine confirms it (M85 lists §5.3), as the engine would apply it
//! (PROTOCOL.md "Lists", the table of ops): an add goes to the end of the section it names, right
//! after the row it names, or with neither to the end of the list's first group, the items above
//! its first heading or else its first section. A folder, a restore, a create and a removed list
//! are the engine's to work out, so they change nothing here and are drawn when the engine pushes
//! them. An op the engine would refuse for its shape, such as a check of a heading, changes nothing
//! either; another op the engine refuses is drawn until its refusal puts the rows back.

use super::state::PendingEdit;
use super::{ITEM, SECTION};
use crate::chat_wire::protocol::{List, ListOp, ListRow};

/// The start of the id a row added here is drawn under until a push names it.
pub const LOCAL_PREFIX: &str = "local:";

/// The id a row an edit made here is drawn under: never an engine id (`ci_…`).
pub fn local_row_id(client_msg_id: &str) -> String {
    format!("{LOCAL_PREFIX}{client_msg_id}")
}

/// Whether a row was made here and the engine has not named it yet: it is drawn as sending and
/// takes no edit.
pub fn is_local(row_id: &str) -> bool {
    row_id.starts_with(LOCAL_PREFIX)
}

/// Where an add or a move lands: at the end of a section, or right after a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Section(String),
    After(String),
}

impl Place {
    /// The place an op's two fields name, the section first, as the engine reads them.
    pub fn of(section_id: Option<&str>, after_id: Option<&str>) -> Option<Place> {
        if let Some(id) = section_id {
            return Some(Place::Section(id.to_owned()));
        }
        after_id.map(|id| Place::After(id.to_owned()))
    }

    /// An op's `section_id` and `after_id` for a place.
    pub fn fields(place: Option<Place>) -> (Option<String>, Option<String>) {
        match place {
            Some(Place::Section(id)) => (Some(id), None),
            Some(Place::After(id)) => (None, Some(id)),
            None => (None, None),
        }
    }

    fn row(&self) -> &str {
        match self {
            Place::Section(id) | Place::After(id) => id,
        }
    }
}

/// The list with each edit applied in turn, oldest first.
pub fn applying(edits: &[PendingEdit], list: &List) -> List {
    edits.iter().fold(list.clone(), |list, edit| {
        apply(&edit.op, &edit.client_msg_id, list)
    })
}

/// The list with one op applied, as the engine would apply it. A row it makes is named by
/// `local_row_id(client_msg_id)`.
pub fn apply(op: &ListOp, client_msg_id: &str, mut list: List) -> List {
    let rows = &mut list.rows;
    match op {
        ListOp::Add {
            text,
            note,
            section_id,
            after_id,
        } => {
            let place = Place::of(section_id.as_deref(), after_id.as_deref());
            let at = place_index(place.as_ref(), rows);
            rows.insert(at, local_item(client_msg_id, text, note.as_deref()));
        }
        ListOp::Check { row_id } => mark(rows, row_id, true),
        ListOp::Uncheck { row_id } => mark(rows, row_id, false),
        ListOp::Edit { row_id, text, note } => {
            reword(rows, row_id, text.as_deref(), note.as_deref());
        }
        ListOp::Move {
            row_id,
            section_id,
            after_id,
            to_list_id,
        } => {
            let place = Place::of(section_id.as_deref(), after_id.as_deref());
            moving(rows, row_id, place, to_list_id.is_some());
        }
        ListOp::Section { text, after_id } => {
            heading(rows, client_msg_id, text, after_id.as_deref())
        }
        ListOp::Remove { row_id } => rows.retain(|row| row.row_id != *row_id),
        ListOp::Rename { text } => list.title.clone_from(text),
        ListOp::ClearDone => rows.retain(|row| !row.is_done()),
        ListOp::Create { .. }
        | ListOp::Folder { .. }
        | ListOp::Restore { .. }
        | ListOp::RemoveList => {}
    }
    list
}

/// Where an add, or a move within the list, lands among `rows`. A place naming a row the list
/// does not have lands at the end of the first group.
pub fn place_index(place: Option<&Place>, rows: &[ListRow]) -> usize {
    let named = place.and_then(|place| rows.iter().position(|row| row.row_id == place.row()));
    match (place, named) {
        (Some(Place::Section(_)), Some(heading)) => section_end(rows, heading),
        (Some(Place::After(_)), Some(row)) => row + 1,
        _ => first_group_end(rows),
    }
}

/// The end of the list's first group: the items above its first heading, or, where it opens with
/// a heading, its first section.
pub fn first_group_end(rows: &[ListRow]) -> usize {
    let Some(first) = rows.first() else {
        return 0;
    };
    if first.is_section() {
        return section_end(rows, 0);
    }
    rows.iter()
        .position(ListRow::is_section)
        .unwrap_or(rows.len())
}

/// The index after the last row of the section whose heading is at `heading`: the next
/// heading's, or the list's end.
pub fn section_end(rows: &[ListRow], heading: usize) -> usize {
    rows.iter()
        .skip(heading + 1)
        .position(ListRow::is_section)
        .map_or(rows.len(), |after| heading + 1 + after)
}

/// Every row an op names, as the row it acts on or as its place.
pub fn named_rows(op: &ListOp) -> Vec<&str> {
    match op {
        ListOp::Check { row_id }
        | ListOp::Uncheck { row_id }
        | ListOp::Remove { row_id }
        | ListOp::Edit { row_id, .. } => vec![row_id],
        ListOp::Move {
            row_id,
            section_id,
            after_id,
            ..
        } => [Some(row_id), section_id.as_ref(), after_id.as_ref()]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect(),
        ListOp::Add {
            section_id,
            after_id,
            ..
        } => [section_id, after_id]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect(),
        ListOp::Section { after_id, .. } => after_id.iter().map(String::as_str).collect(),
        ListOp::Restore { row_id, .. } => row_id.iter().map(String::as_str).collect(),
        ListOp::Create { .. }
        | ListOp::Folder { .. }
        | ListOp::Rename { .. }
        | ListOp::RemoveList
        | ListOp::ClearDone => Vec::new(),
    }
}

/// Whether an op is worth sending: words the engine trims to nothing are refused there, so they
/// are no edit here. An edit with no words changes only its note.
pub fn sendable(op: &ListOp) -> bool {
    match op {
        ListOp::Create { title: words, .. }
        | ListOp::Add { text: words, .. }
        | ListOp::Section { text: words, .. }
        | ListOp::Rename { text: words } => !words.trim().is_empty(),
        ListOp::Edit { text, note, .. } => match text {
            Some(words) => !words.trim().is_empty(),
            None => note.is_some(),
        },
        _ => true,
    }
}

/// The edit a rewording of `was` saves (M85 lists §3.5): only what changed from the item as the
/// rewording began, each part trimmed at its ends, as the engine trims what it stores. Words
/// trimmed to nothing keep the old words, and a note emptied is cleared (`note: ""`). `note` is
/// `None` where the words alone are reworded, as on a card, and then no note is sent. `None` when
/// nothing changed.
pub fn reworded(was: &ListRow, words: &str, note: Option<&str>) -> Option<ListOp> {
    let words = words.trim();
    let text = (!words.is_empty() && words != was.text).then(|| words.to_owned());
    let note = note
        .map(str::trim)
        .filter(|note| *note != was.note.as_deref().unwrap_or_default())
        .map(str::to_owned);
    if text.is_none() && note.is_none() {
        return None;
    }
    Some(ListOp::Edit {
        row_id: was.row_id.clone(),
        text,
        note,
    })
}

fn local_item(client_msg_id: &str, text: &str, note: Option<&str>) -> ListRow {
    ListRow {
        row_id: local_row_id(client_msg_id),
        kind: ITEM.to_owned(),
        text: text.to_owned(),
        note: note.filter(|note| !note.is_empty()).map(str::to_owned),
        done: Some(false),
        done_at: None,
    }
}

fn heading(rows: &mut Vec<ListRow>, client_msg_id: &str, text: &str, after_id: Option<&str>) {
    let after = after_id.and_then(|id| rows.iter().position(|row| row.row_id == id));
    let at = after.map_or(rows.len(), |row| row + 1);
    let row = ListRow {
        row_id: local_row_id(client_msg_id),
        kind: SECTION.to_owned(),
        text: text.to_owned(),
        note: None,
        done: None,
        done_at: None,
    };
    rows.insert(at, row);
}

/// Marks an item done or not; unticking forgets when it was done.
fn mark(rows: &mut [ListRow], row_id: &str, done: bool) {
    let Some(row) = item_mut(rows, row_id) else {
        return;
    };
    row.done = Some(done);
    if !done {
        row.done_at = None;
    }
}

/// An item's new words, its note, or both. An empty note clears it.
fn reword(rows: &mut [ListRow], row_id: &str, text: Option<&str>, note: Option<&str>) {
    let Some(row) = item_mut(rows, row_id) else {
        return;
    };
    if let Some(text) = text {
        text.clone_into(&mut row.text);
    }
    if let Some(note) = note {
        row.note = Some(note.to_owned()).filter(|note| !note.is_empty());
    }
}

/// A move within the list takes the item out and puts it where an add would land in what is left;
/// a move to another list takes it out. A heading is not moved, and a move within the list that
/// names no place changes nothing.
fn moving(rows: &mut Vec<ListRow>, row_id: &str, place: Option<Place>, leaving: bool) {
    let Some(at) = rows
        .iter()
        .position(|row| row.row_id == row_id && row.is_item())
    else {
        return;
    };
    if !leaving && place.is_none() {
        return;
    }
    let row = rows.remove(at);
    if leaving {
        return;
    }
    let at = place_index(place.as_ref(), rows);
    rows.insert(at, row);
}

fn item_mut<'a>(rows: &'a mut [ListRow], row_id: &str) -> Option<&'a mut ListRow> {
    rows.iter_mut()
        .find(|row| row.row_id == row_id && row.is_item())
}
