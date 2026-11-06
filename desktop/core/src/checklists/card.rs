//! One reply's lists card as it stands now (M85 lists §2.2 to §2.5), as a value: from the lists
//! its block names, whether it is live, the lists as the library holds them, and the card's own
//! state (grown past four, and the rows ticked a moment ago that keep their place while they
//! settle); and what a screen reader is told as it is drawn again (§6.3).

use super::cards::{self, OneList};
use super::library::Library;
use super::words;
use super::ListRef;
use crate::chat_wire::protocol::ListRow;
use std::collections::{BTreeMap, BTreeSet};

/// One list as a card names it: its title and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub list_id: String,
    /// On a live card, the engine's title once the list is held, else the block's; on a folded
    /// card, the index's once it names the list, else the block's.
    pub title: String,
    pub stands: Stands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stands {
    /// Not pulled yet, or its pull not answered: no counts and no rows. No spinner: the engine
    /// answers in milliseconds.
    Waiting,
    /// The list carries `removed_at`, or the index does not name it.
    Removed,
    /// Its open and done items.
    Counted { open: usize, done: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drawing {
    /// One list (§2.2): its head; once counted, its rows with "N more" or "Show less"; and the
    /// engine's sentence for the last edit made here that it refused. A removed list is its head
    /// alone (§2.5).
    One {
        head: Head,
        rows: OneList,
        refusal: Option<String>,
    },
    /// Several lists (§2.3): at most three groups, and how many lists the card names past them.
    Several {
        groups: Vec<ListGroup>,
        more_lists: usize,
    },
    /// Folded (§2.4): one line for each list the card drew, counted from the index as it is now.
    /// A folded card never pulls.
    Folded(Vec<Head>),
}

/// One list on a card of several (§2.3, §2.5): its head over the rows its turn touched as they
/// stand now, then the engine's sentence for the last edit made here that it refused. A removed
/// list is its head alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListGroup {
    pub head: Head,
    pub rows: Vec<ListRow>,
    pub refusal: Option<String>,
}

/// What the card of `lists` draws. `None` for a card of no lists, which is no card.
pub fn drawing(
    lists: &[ListRef],
    live: bool,
    library: &Library,
    expanded: bool,
    settling: &BTreeSet<String>,
) -> Option<Drawing> {
    if lists.is_empty() {
        return None;
    }
    if !live {
        let heads = cards::drawn(lists).iter().map(|list| folded(list, library));
        return Some(Drawing::Folded(heads.collect()));
    }
    let [list] = lists else {
        let made = cards::groups(lists, |id| library.shown(id));
        let groups = made
            .groups
            .into_iter()
            .map(|group| list_group(head(&group.list, library), group.rows, library));
        return Some(Drawing::Several {
            groups: groups.collect(),
            more_lists: made.more_lists,
        });
    };
    Some(one(list, library, expanded, settling))
}

fn one(list: &ListRef, library: &Library, expanded: bool, settling: &BTreeSet<String>) -> Drawing {
    let head = head(list, library);
    let shown = library.shown(&list.list_id);
    let rows = match (&head.stands, shown) {
        (Stands::Counted { .. }, Some(shown)) => {
            cards::one_list(&shown, &list.rows, settling, expanded)
        }
        _ => OneList {
            rows: Vec::new(),
            more: 0,
            fewer: false,
        },
    };
    let refusal = match head.stands {
        Stands::Removed => None,
        Stands::Waiting | Stands::Counted { .. } => refusal(&list.list_id, library),
    };
    Drawing::One {
        head,
        rows,
        refusal,
    }
}

/// A group of a card of several: a removed list draws no rows and no sentence.
fn list_group(head: Head, rows: Vec<ListRow>, library: &Library) -> ListGroup {
    if head.stands == Stands::Removed {
        return ListGroup {
            head,
            rows: Vec::new(),
            refusal: None,
        };
    }
    let refusal = refusal(&head.list_id, library);
    ListGroup {
        head,
        rows,
        refusal,
    }
}

/// The engine's sentence for the last edit of `list_id` made here that it refused, until the next.
fn refusal(list_id: &str, library: &Library) -> Option<String> {
    let state = library.state(list_id)?;
    state.refusal().map(str::to_owned)
}

/// A live card's list as the library holds it: removed when the index does not name it or the
/// engine says so, counted once its pull is answered, else waiting.
fn head(list: &ListRef, library: &Library) -> Head {
    let id = &list.list_id;
    let removed =
        library.index().lacks(id) || library.state(id).is_some_and(|state| state.is_removed());
    let shown = library.shown(id);
    let title = shown
        .as_ref()
        .map_or_else(|| list.title.clone(), |shown| shown.title.clone());
    let stands = match &shown {
        _ if removed => Stands::Removed,
        Some(shown) => Stands::Counted {
            open: shown.open_count(),
            done: shown.done_count(),
        },
        None => Stands::Waiting,
    };
    Head {
        list_id: id.clone(),
        title,
        stands,
    }
}

/// A folded card's line for a list, from the index alone: a folded card never pulls.
fn folded(list: &ListRef, library: &Library) -> Head {
    let index = library.index();
    let entry = index.entry(&list.list_id);
    let stands = match entry {
        _ if index.lacks(&list.list_id) => Stands::Removed,
        Some(entry) => Stands::Counted {
            open: usize::try_from(entry.open).unwrap_or(usize::MAX),
            done: usize::try_from(entry.done).unwrap_or(usize::MAX),
        },
        None => Stands::Waiting,
    };
    Head {
        list_id: list.list_id.clone(),
        title: entry.map_or_else(|| list.title.clone(), |entry| entry.title.clone()),
        stands,
    }
}

/// What a screen reader is told as a card is drawn again (§5.3, §6.3), since focus is elsewhere,
/// while the card was on screen at the chat's last look. A refused edit's sentence stays on its
/// list until the list's next edit, and the card tells it only when it first shows it: when it
/// comes under a list the live card already drew without it. A card first drawn, or one turning
/// live, says nothing of a sentence its list already had, so a newer card never tells an older
/// refusal. "This list was removed." is told when a card that drew its one list live turns
/// removed, not by one turning live on a removed list, whose folded part already said so. A card
/// far up the chat says nothing. `was` is what the card drew before, if it drew.
pub fn news<'a>(was: Option<&Drawing>, now: &'a Drawing, on_screen: bool) -> Vec<&'a str> {
    let Some(was) = was.filter(|_| on_screen) else {
        return Vec::new();
    };
    let before = refusals(was);
    let drew = live_lists(was);
    let mut news: Vec<&str> = refusals(now)
        .into_iter()
        .filter(|(list_id, _)| drew.contains(list_id))
        .filter(|refused| !before.contains(refused))
        .map(|(_, sentence)| sentence)
        .collect();
    let drew_live = matches!(was, Drawing::One { .. }) && removed(was).is_none();
    news.extend(removed(now).filter(|_| drew_live));
    news
}

/// The lists a live card draws, each with its rows and sentence: a folded card draws neither.
fn live_lists(drawing: &Drawing) -> Vec<&str> {
    match drawing {
        Drawing::One { head, .. } => vec![head.list_id.as_str()],
        Drawing::Several { groups, .. } => groups
            .iter()
            .map(|group| group.head.list_id.as_str())
            .collect(),
        Drawing::Folded(_) => Vec::new(),
    }
}

/// Each refused edit's sentence the card shows, with the list it is under: its one list's, or
/// each group's.
fn refusals(drawing: &Drawing) -> Vec<(&str, &str)> {
    match drawing {
        Drawing::One { head, refusal, .. } => refusal
            .as_deref()
            .map(|sentence| (head.list_id.as_str(), sentence))
            .into_iter()
            .collect(),
        Drawing::Several { groups, .. } => groups
            .iter()
            .filter_map(|g| Some((g.head.list_id.as_str(), g.refusal.as_deref()?)))
            .collect(),
        Drawing::Folded(_) => Vec::new(),
    }
}

/// "This list was removed." on a card of one list whose list was removed.
fn removed(drawing: &Drawing) -> Option<&'static str> {
    match drawing {
        Drawing::One { head, .. } if head.stands == Stands::Removed => Some(words::LIST_REMOVED),
        Drawing::One { .. } | Drawing::Several { .. } | Drawing::Folded(_) => None,
    }
}

/// The rows ticked a moment ago that keep their place while they settle (§2.2), each until the
/// last settle begun for it ends: a row ticked, unticked and ticked again within a settle keeps
/// its place for the whole of the second.
#[derive(Debug, Default)]
pub struct Settling {
    /// Each row settling, with how many of its settles have not ended.
    rows: BTreeMap<String, usize>,
}

impl Settling {
    /// `row_id` begins to settle.
    pub fn begin(&mut self, row_id: &str) {
        *self.rows.entry(row_id.to_owned()).or_default() += 1;
    }

    /// One settle of `row_id` ended: true when it was its last, so the row stops settling.
    pub fn end(&mut self, row_id: &str) -> bool {
        let Some(left) = self.rows.get_mut(row_id) else {
            return false;
        };
        *left -= 1;
        if *left > 0 {
            return false;
        }
        self.rows.remove(row_id);
        true
    }

    /// The rows settling now.
    pub fn rows(&self) -> BTreeSet<String> {
        self.rows.keys().cloned().collect()
    }
}
