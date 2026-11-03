//! The Lists pane's lines (M85 lists §3.4, §3.5): what the one list draws, in order; where Alt+Up
//! and Alt+Down put a row, by the engine's ops; which section an added item goes to; and the one
//! splice that turns the drawn lines into the new ones.

use super::draft::Place;
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
    Removed(ListRow),
}

impl Line {
    /// Unique within one list's lines.
    pub fn id(&self) -> String {
        match self {
            Line::Heading { row, .. } => format!("heading:{}", row.row_id),
            Line::Item(row) | Line::Done(row) => row.row_id.clone(),
            Line::DoneHeading { .. } => "done".to_owned(),
            Line::RemovedHeading => "removed".to_owned(),
            Line::Removed(row) => format!("removed:{}", row.row_id),
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
/// moment ago, and whether Recently removed is shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub folded: BTreeSet<String>,
    pub done_open: bool,
    pub settling: BTreeSet<String>,
    pub removed_shown: bool,
}

/// The pane's lines: each section under its heading with its open items, or none while it is
/// folded; then every done item, under a Done fold closed until opened; then, when shown, Recently
/// removed with its items. A row ticked a moment ago keeps its place while it settles.
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

fn removed_lines(list: &List, view: &View) -> Vec<Line> {
    if !view.removed_shown || list.removed.is_empty() {
        return Vec::new();
    }
    let items = list.removed.iter().map(|removed| &removed.row);
    std::iter::once(Line::RemovedHeading)
        .chain(
            items
                .filter(|row| row.is_item())
                .cloned()
                .map(Line::Removed),
        )
        .collect()
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
