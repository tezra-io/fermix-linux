//! The owner's lists as the chat holds them (M85 lists; PROTOCOL.md "Lists"): one list over the
//! engine's last push with the edits it has not confirmed drawn on top (`state`, `draft`), the
//! connection's share of the lists (`library`, `index`), what a reply's card draws (`cards`), the
//! pane's lines (`panel`), and the words (`words`). Values only: nothing here touches a socket, and
//! the session sends what the library returns.

pub mod cards;
pub mod draft;
pub mod index;
pub mod library;
pub mod panel;
pub mod state;
pub mod words;

use crate::chat_wire::protocol::{List, ListRow};
use serde::Deserialize;

/// The companion version that brings lists (PROTOCOL.md "Version 6").
pub const LISTS_SINCE: u64 = 6;
/// A row's kind, in the daemon's words. A row of any other kind is in no section and never drawn.
pub const ITEM: &str = "item";
pub const SECTION: &str = "section";

/// A list a reply's turn read or changed (PROTOCOL.md "Blocks under a reply"): its id, its title
/// when the turn ended, and the rows the turn added or changed, none for a list it only read.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ListRef {
    pub list_id: String,
    pub title: String,
    #[serde(default)]
    pub rows: Vec<String>,
}

impl ListRow {
    pub fn is_item(&self) -> bool {
        self.kind == ITEM
    }

    pub fn is_section(&self) -> bool {
        self.kind == SECTION
    }

    /// An item marked done. A heading is never done.
    pub fn is_done(&self) -> bool {
        self.is_item() && self.done == Some(true)
    }
}

impl List {
    /// The items, open and done, without the headings.
    pub fn items(&self) -> impl Iterator<Item = &ListRow> {
        self.rows.iter().filter(|row| row.is_item())
    }

    pub fn open_count(&self) -> usize {
        self.items().filter(|row| !row.is_done()).count()
    }

    pub fn done_count(&self) -> usize {
        self.items().filter(|row| row.is_done()).count()
    }

    /// The list as text, as the macOS app's Copy as text writes it: `# Title`, each section under
    /// `## Heading`, each item a `- [ ]` or `- [x]` line with its note's lines under it, indented
    /// two spaces.
    pub fn markdown(&self) -> String {
        let mut lines = vec![format!("# {}", self.title)];
        for section in sections(&self.rows) {
            if let Some(heading) = section.heading {
                lines.push(String::new());
                lines.push(format!("## {}", heading.text));
            }
            lines.extend(section.items.into_iter().flat_map(item_lines));
        }
        lines.join("\n")
    }
}

fn item_lines(item: &ListRow) -> Vec<String> {
    let mark = if item.is_done() { "x" } else { " " };
    let mut lines = vec![format!("- [{mark}] {}", item.text)];
    if let Some(note) = &item.note {
        lines.extend(note.split('\n').map(|line| format!("  {line}")));
    }
    lines
}

/// One part a list is drawn in: its first group (the items above its first heading, with no
/// heading), or a heading with the items under it. Done items stay where the engine keeps them;
/// whoever draws decides where they show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section<'a> {
    pub heading: Option<&'a ListRow>,
    pub items: Vec<&'a ListRow>,
}

/// A list's parts in order. An empty first group is no part; an empty section is one.
pub fn sections(rows: &[ListRow]) -> Vec<Section<'_>> {
    let mut parts = Vec::new();
    let mut current = Section {
        heading: None,
        items: Vec::new(),
    };
    for row in rows {
        if row.is_section() {
            let next = Section {
                heading: Some(row),
                items: Vec::new(),
            };
            keep(&mut parts, std::mem::replace(&mut current, next));
            continue;
        }
        if row.is_item() {
            current.items.push(row);
        }
    }
    keep(&mut parts, current);
    parts
}

fn keep<'a>(parts: &mut Vec<Section<'a>>, part: Section<'a>) {
    if part.heading.is_some() || !part.items.is_empty() {
        parts.push(part);
    }
}
