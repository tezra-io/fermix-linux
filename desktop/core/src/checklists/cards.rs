//! What a reply's lists card draws (M85 lists §2), as values: which card in the conversation is
//! live, a one-list card's rows, and a card of several lists' groups.

use super::ListRef;
use crate::chat_wire::protocol::{List, ListRow};
use std::collections::BTreeSet;

/// A one-list card's open items, at most, until "N more" grows it.
pub const ONE_LIST_ROWS: usize = 4;
/// A card of several lists: rows a group, and groups a card, at most.
pub const GROUP_ROWS: usize = 3;
pub const GROUP_LISTS: usize = 3;

/// The cards that are live, by the seq of the reply under which each stands. Only the newest card
/// that draws a given list is live, so a card is live when no newer card draws any of the lists it
/// draws; every other card folds to one line. A list that a card of several only names past its
/// three groups is not drawn there, so it does not count. `cards` runs oldest first; a reply with
/// no lists has no card.
pub fn live<'a, I>(cards: I) -> BTreeSet<u64>
where
    I: IntoIterator<Item = (u64, &'a [ListRef])>,
    I::IntoIter: DoubleEndedIterator,
{
    let mut drawn_newer: BTreeSet<&str> = BTreeSet::new();
    let mut live = BTreeSet::new();
    for (seq, refs) in cards.into_iter().rev().filter(|(_, refs)| !refs.is_empty()) {
        let ids: Vec<&str> = drawn(refs).iter().map(|r| r.list_id.as_str()).collect();
        if ids.iter().all(|id| !drawn_newer.contains(id)) {
            live.insert(seq);
        }
        drawn_newer.extend(ids);
    }
    live
}

/// The lists a card draws: a one-list card its one, a card of several its first three.
pub fn drawn(refs: &[ListRef]) -> &[ListRef] {
    &refs[..refs.len().min(GROUP_LISTS)]
}

/// A one-list card's rows, and what "N more" and "Show less" say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OneList {
    pub rows: Vec<ListRow>,
    /// The open items not drawn, for "N more".
    pub more: usize,
    /// The card is grown past four, so "Show less" has something to fold.
    pub fewer: bool,
}

/// A one-list card's rows: at most four open items, the ones the turn touched first, each part
/// in the list's own order. Grown, it holds every open item in the same order, so the first four
/// stay where they were. A row ticked here a moment ago (`settling`) keeps its place while it
/// settles. Done items are counted in the header and never drawn.
pub fn one_list(
    list: &List,
    touched: &[String],
    settling: &BTreeSet<String>,
    expanded: bool,
) -> OneList {
    let open: Vec<&ListRow> = list
        .items()
        .filter(|row| !row.is_done() || settling.contains(&row.row_id))
        .collect();
    let (first, rest): (Vec<&ListRow>, Vec<&ListRow>) = open
        .iter()
        .copied()
        .partition(|row| touched.contains(&row.row_id));
    let limit = if expanded { open.len() } else { ONE_LIST_ROWS };
    let rows: Vec<ListRow> = first.into_iter().chain(rest).take(limit).cloned().collect();
    OneList {
        more: open.len() - rows.len(),
        fewer: expanded && open.len() > ONE_LIST_ROWS,
        rows,
    }
}

/// One group of a card of several lists: the list's name over the rows its turn touched, as they
/// stand now, at most three. A row the turn ticked shows done, the card being a receipt of what
/// happened; a row since removed is not drawn; a list the turn only read is its name row alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub list: ListRef,
    pub rows: Vec<ListRow>,
}

/// A card of several lists: at most three groups, and how many lists it names past them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Groups {
    pub groups: Vec<Group>,
    pub more_lists: usize,
}

/// The groups a card of several lists draws, each from the list as `lists` has it now.
pub fn groups(refs: &[ListRef], lists: impl Fn(&str) -> Option<List>) -> Groups {
    let groups = drawn(refs)
        .iter()
        .map(|list| Group {
            list: list.clone(),
            rows: touched_rows(list, lists(&list.list_id)),
        })
        .collect();
    Groups {
        groups,
        more_lists: refs.len().saturating_sub(GROUP_LISTS),
    }
}

fn touched_rows(list: &ListRef, now: Option<List>) -> Vec<ListRow> {
    let rows = now.map(|now| now.rows).unwrap_or_default();
    list.rows
        .iter()
        .filter_map(|id| rows.iter().find(|row| row.row_id == *id && row.is_item()))
        .take(GROUP_ROWS)
        .cloned()
        .collect()
}
