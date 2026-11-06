//! The Lists pane's lines (M85 lists §3.4, §3.5): what the one list draws, in order, an item
//! being reworded among them; where Move up and Move down, Alt+Up and Alt+Down, put a row, by
//! the engine's ops, and the folded heading a step takes it under (§3.6, §6.1); which section an
//! added item goes to; and the one splice that turns the drawn lines into the new ones.

use super::draft::{is_local, Place};
use super::sections;
use crate::chat_wire::protocol::{List, ListRow};
use std::collections::BTreeSet;

/// One line of the pane's list, in the order drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// A section's heading, and whether the section is folded.
    Heading {
        row: ListRow,
        folded: bool,
    },
    /// An open item, or one ticked a moment ago that keeps its place while it settles.
    Item(ListRow),
    /// The Done fold: how many items are done, and whether it is open.
    DoneHeading {
        count: usize,
        open: bool,
    },
    Done(ListRow),
    /// Recently removed, with each removed item under it.
    RemovedHeading,
    /// A removed item, and whether a restore of it waits: its Restore is off until the engine
    /// pushes the list or refuses the restore.
    Removed {
        row: ListRow,
        restoring: bool,
    },
    /// The entry for the words of the item being reworded, in its place, by its id alone: a push
    /// that changes the item leaves the line, and what is typed in it, as it is.
    Words(String),
    /// The entry for that item's note, under its words.
    Note(String),
}

impl Line {
    /// Unique within one list's lines.
    pub fn id(&self) -> String {
        match self {
            Line::Heading { row, .. } => format!("heading:{}", row.row_id),
            Line::Item(row) | Line::Done(row) => row.row_id.clone(),
            Line::DoneHeading { .. } => "done".to_owned(),
            Line::RemovedHeading => "removed".to_owned(),
            Line::Removed { row, .. } => format!("removed:{}", row.row_id),
            Line::Words(row_id) => format!("words:{row_id}"),
            Line::Note(row_id) => format!("note:{row_id}"),
        }
    }

    /// The item a line draws, where it draws one that can be ticked and reworded.
    pub fn editable(&self) -> Option<&ListRow> {
        match self {
            Line::Item(row) | Line::Done(row) => Some(row),
            _ => None,
        }
    }
}

/// How the pane draws its list now: the sections folded, whether Done is open, the rows ticked a
/// moment ago, whether Recently removed is shown, and the removed rows whose restore waits.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub folded: BTreeSet<String>,
    pub done_open: bool,
    pub settling: BTreeSet<String>,
    pub removed_shown: bool,
    pub restoring: BTreeSet<String>,
}

/// The pane's lines: each section under its heading with its open items, or none while it is
/// folded; then every done item, under a Done fold closed until opened; then, when shown, Recently
/// removed with its items, each restoring while its restore waits. A row ticked a moment ago keeps
/// its place while it settles.
pub fn lines(list: &List, view: &View) -> Vec<Line> {
    let mut lines = Vec::new();
    for section in sections(&list.rows) {
        let folded = section
            .heading
            .is_some_and(|heading| view.folded.contains(&heading.row_id));
        if let Some(heading) = section.heading {
            lines.push(Line::Heading {
                row: heading.clone(),
                folded,
            });
        }
        let open = section.items.into_iter().filter(|row| open_line(row, view));
        lines.extend(open.filter(|_| !folded).cloned().map(Line::Item));
    }
    lines.extend(done_lines(list, view));
    lines.extend(removed_lines(list, view));
    lines
}

fn open_line(row: &ListRow, view: &View) -> bool {
    !row.is_done() || view.settling.contains(&row.row_id)
}

fn done_lines(list: &List, view: &View) -> Vec<Line> {
    let done: Vec<&ListRow> = list.items().filter(|row| !open_line(row, view)).collect();
    if done.is_empty() {
        return Vec::new();
    }
    let heading = Line::DoneHeading {
        count: done.len(),
        open: view.done_open,
    };
    let shown = done.into_iter().filter(|_| view.done_open);
    std::iter::once(heading)
        .chain(shown.cloned().map(Line::Done))
        .collect()
}

/// Recently removed while it is shown: its heading and each removed item, or nothing when no item
/// was removed, as when only a section was (§10).
fn removed_lines(list: &List, view: &View) -> Vec<Line> {
    if !view.removed_shown {
        return Vec::new();
    }
    let items = list.removed.iter().map(|removed| &removed.row);
    let removed: Vec<Line> = items
        .filter(|row| row.is_item())
        .map(|row| Line::Removed {
            row: row.clone(),
            restoring: view.restoring.contains(&row.row_id),
        })
        .collect();
    if removed.is_empty() {
        return Vec::new();
    }
    std::iter::once(Line::RemovedHeading)
        .chain(removed)
        .collect()
}

/// The lines with the item `row_id` drawn as it is reworded (§3.5): its words' entry, then its
/// note's, in its place, an open item or a done one. A heading, a row the engine has not named
/// yet, and an item not drawn are never reworded, and the lines stay as they were.
pub fn with_rewording(lines: Vec<Line>, row_id: &str) -> Vec<Line> {
    if is_local(row_id) {
        return lines;
    }
    let mut drawn = Vec::with_capacity(lines.len() + 1);
    for line in lines {
        if line.editable().is_some_and(|row| row.row_id == row_id) {
            drawn.push(Line::Words(row_id.to_owned()));
            drawn.push(Line::Note(row_id.to_owned()));
        } else {
            drawn.push(line);
        }
    }
    drawn
}

/// Whether the lines draw the item `row_id` as it is reworded: once they do not, its rewording
/// has ended.
pub fn rewords(lines: &[Line], row_id: &str) -> bool {
    lines
        .iter()
        .any(|line| matches!(line, Line::Words(id) if id == row_id))
}

/// One step up or down for an open item: past the line above or below, a heading included, so
/// the item crosses into the next section. Up, it lands after the line two above it, a heading
/// putting it first in its section; down, after the line below it. `None` where no op can say it:
/// above the top of a list whose first group has no heading, below the last open line, and for a
/// line that is not an open item.
pub fn step(row_id: &str, up: bool, lines: &[Line]) -> Option<Place> {
    let at = lines.iter().position(|line| line.id() == row_id)?;
    if !matches!(lines[at], Line::Item(_)) {
        return None;
    }
    let next = if up {
        lines.get(at.checked_sub(2)?)?
    } else {
        lines.get(at + 1)?
    };
    match next {
        Line::Heading { row, .. } | Line::Item(row) => Some(Place::After(row.row_id.clone())),
        _ => None,
    }
}

/// Where Move up and Move down put an item, as Alt+Up and Alt+Down do (M85 lists §3.6, §6.1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Steps {
    pub up: Option<Place>,
    pub down: Option<Place>,
}

/// Move up and Move down for the item `row_id`: a `step` each way over the list's open lines, as
/// if no section were folded, nothing were settling and Done were closed, as the macOS app steps.
/// So neither a done item nor one ticked a moment ago steps, and a step into a folded section
/// lands where it would with the section open. Nor does a row the engine has not named yet step,
/// nor does a step land after one: no op names such a row (§5.3). Passing over one to a named row
/// stays on.
pub fn steps(list: &List, row_id: &str) -> Steps {
    if is_local(row_id) {
        return Steps::default();
    }
    let lines = lines(list, &View::default());
    let named = |place: &Place| match place {
        Place::Section(id) | Place::After(id) => !is_local(id),
    };
    Steps {
        up: step(row_id, true, &lines).filter(named),
        down: step(row_id, false, &lines).filter(named),
    }
}

/// The id of the heading's line an item moved to `place` goes under, where `view` folds that
/// section: the item is then not drawn, and the pane gives the heading the focus, so a held
/// Alt+Up stops there. `None` in the first group or a section drawn open.
pub fn hidden_under(list: &List, view: &View, place: &Place) -> Option<String> {
    let (Place::Section(id) | Place::After(id)) = place;
    let at = list.rows.iter().position(|row| row.row_id == *id)?;
    let heading = list.rows[..=at].iter().rev().find(|row| row.is_section())?;
    if !view.folded.contains(&heading.row_id) {
        return None;
    }
    let line = Line::Heading {
        row: heading.clone(),
        folded: true,
    };
    Some(line.id())
}

/// Where an item added from the pane goes (M85 lists §3.5): the selected item's section, or with
/// nothing selected, or an item of the first group selected, the first group (`None`).
pub fn add_place(list: &List, selected: Option<&str>) -> Option<Place> {
    let selected = selected?;
    let section = sections(&list.rows)
        .into_iter()
        .find(|section| section.items.iter().any(|row| row.row_id == selected))?;
    section
        .heading
        .map(|heading| Place::Section(heading.row_id.clone()))
}

/// One change to the drawn lines: from `at`, `removed` lines go and `added` take their place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub at: usize,
    pub removed: usize,
    pub added: Vec<Line>,
}

/// The one splice that turns `old` into `new`, between their common first and last lines; `None`
/// when nothing changed, so a push that changed nothing redraws nothing.
pub fn splice(old: &[Line], new: &[Line]) -> Option<Splice> {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    if head == old.len() && head == new.len() {
        return None;
    }
    let tail = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    Some(Splice {
        at: head,
        removed: old.len() - head - tail,
        added: new[head..new.len() - tail].to_vec(),
    })
}
