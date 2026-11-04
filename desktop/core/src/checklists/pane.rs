//! The Lists pane (M85 lists §3), as values: whether it is open, wide or narrow, and the owner's
//! last open or close kept in `lists.json` (§3.2, L4); and what it draws now: the list asked for
//! while the index holds it, a new list made here turned to once, its head and lines, each list's
//! own folds and its own Recently removed, an item being reworded, or the state before the index
//! answers or with no list, and the sentence for a new list or a list's Undo refused (§3.3, §3.4,
//! §3.5, §3.8, §10); which Restore, and whether the removed list's Undo, waits for its restore
//! (§10); a list taken back turned to (§3.7); what the list's menu offers for it, and the op Move
//! to folder sends (§3.6); the op a name typed in the head makes (§3.3); and what a screen reader
//! is told as it is drawn again (§6.3).
//!
//! The picker counts each list from the index, as a folded card does. The head under it counts
//! the list as drawn, its edits waiting included, once it is held. While an edit waits for the
//! engine, the picker's count and the head's can differ, until the index after the edit's push
//! catches up.

use super::index::{Folders, Index};
use super::library::Library;
use super::panel::{self, Line, View};
use super::removals::Removal;
use super::state::PendingEdit;
use super::words;
use crate::chat_wire::protocol::ListOp;
use crate::update::store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Where the owner's last open or close is kept, under `$XDG_STATE_HOME`.
const LISTS_FILE: &str = "fermix-desktop/lists.json";

/// Whether the pane is shown. Wide, it is the owner's last open or close, remembered across
/// restarts. Narrow, the pane lies over the chat and shows only when asked since the chat
/// narrowed: narrowing hides it and keeps the choice, and widening shows the choice again.
///
/// The chat learns that it narrowed or widened only as it is laid out, and it is not laid out
/// while another page shows. So an open asked for while the chat is off screen (Ctrl+Shift+L
/// from another page) survives the narrowing the chat's first layout brings: the window narrowed
/// before the ask, not after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneOpen {
    chosen: bool,
    collapsed: bool,
    asked: bool,
    /// The chat has not been laid out since it last came on screen.
    unlaid: bool,
    /// An open was asked for while `unlaid`.
    asked_unlaid: bool,
}

impl Default for PaneOpen {
    fn default() -> PaneOpen {
        PaneOpen::new(false)
    }
}

impl PaneOpen {
    /// Wide, with the owner's last choice: `load`'s answer. The chat is not laid out yet.
    pub fn new(chosen: bool) -> PaneOpen {
        PaneOpen {
            chosen,
            collapsed: false,
            asked: false,
            unlaid: true,
            asked_unlaid: false,
        }
    }

    /// The owner's last open or close, which `save` keeps.
    pub fn chosen(&self) -> bool {
        self.chosen
    }

    pub fn shown(&self) -> bool {
        if self.collapsed {
            self.asked
        } else {
            self.chosen
        }
    }

    /// The owner opened or closed the pane, wherever: the toggle, Ctrl+Shift+L, Hide lists, a
    /// card, and, narrow, Escape or a click on the chat.
    pub fn choose(&mut self, open: bool) {
        self.chosen = open;
        self.asked = open;
        self.asked_unlaid = open && self.unlaid;
    }

    /// The toggle and Ctrl+Shift+L: the pane as it is not shown now.
    pub fn toggle(&mut self) {
        self.choose(!self.shown());
    }

    /// The chat narrowed or widened past the pane's breakpoint. An open asked for before the
    /// chat's first layout since it came on screen stands.
    pub fn collapse(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
        if !self.asked_unlaid {
            self.asked = false;
        }
    }

    /// The chat left the screen: until its first layout once back, whether it is narrow is as
    /// it was when it left.
    pub fn chat_left(&mut self) {
        self.unlaid = true;
        self.asked_unlaid = false;
    }

    /// The chat's first layout since it came on screen is drawn: whether it is narrow is now.
    pub fn chat_laid_out(&mut self) {
        self.unlaid = false;
        self.asked_unlaid = false;
    }

    /// The split view showed or hid the pane: the app's own doing, which matches `shown` and
    /// changes nothing, or the owner's Escape or click on the chat while narrow, which is a
    /// choice. True for a choice, which the caller saves.
    pub fn widget_shown(&mut self, shown: bool) -> bool {
        if shown == self.shown() {
            return false;
        }
        self.choose(shown);
        true
    }
}

/// `$XDG_STATE_HOME/fermix-desktop/lists.json`, or under `~/.local/state`, as the updater's.
pub fn lists_path(state_home: Option<&OsStr>, home: &Path) -> PathBuf {
    store::state_file(state_home, home, LISTS_FILE)
}

#[derive(Serialize, Deserialize)]
struct Remembered {
    open: bool,
}

/// Why the owner's last choice did not load, as a code a log may carry: never the path, which
/// holds the home folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotLoaded {
    /// The file is there and could not be read.
    Read(io::ErrorKind),
    /// It was read, and it is not the pane's state.
    Parse(serde_json::error::Category),
}

/// The owner's last open or close: closed when nothing was saved. A file that cannot be read or
/// is not the pane's state reads as closed too, and the code says why, for the caller to log.
pub fn load(path: &Path) -> Result<bool, NotLoaded> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(NotLoaded::Read(e.kind())),
    };
    serde_json::from_str::<Remembered>(&text)
        .map(|remembered| remembered.open)
        .map_err(|e| NotLoaded::Parse(e.classify()))
}

/// Keeps the owner's choice, replacing the file in one step as the updater's state is.
pub fn save(path: &Path, open: bool) -> io::Result<()> {
    store::save(path, &Remembered { open })
}

/// What the pane draws now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneDrawing {
    /// Before the index answers: the head reads "Lists" and nothing is under it.
    Waiting,
    /// The index names no list: "No lists yet.", under the index's sentence for a new list or a
    /// list's Undo refused, if one was.
    NoLists { refusal: Option<String> },
    /// One list: the one asked for while the index holds it, else the first in picker order.
    List(Box<PaneList>),
}

impl PaneDrawing {
    /// The index's sentence under the head: why the last new list made here, or a list's Undo, did
    /// not stand, until the next index or new list (§3.8, §10).
    pub fn index_refusal(&self) -> Option<&str> {
        match self {
            PaneDrawing::Waiting => None,
            PaneDrawing::NoLists { refusal } => refusal.as_deref(),
            PaneDrawing::List(list) => list.index_refusal.as_deref(),
        }
    }
}

/// The list the pane shows, and the picker it is chosen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneList {
    pub folders: Folders,
    pub list_id: String,
    pub title: String,
    /// The folder the index puts it in, if any.
    pub folder: Option<String>,
    /// The folder the newest Move to folder still waiting takes it to, empty for none: where it
    /// will be once its edits land, while the index still gives `folder`.
    pub moving_to: Option<String>,
    /// Its done items, counted as the head counts them.
    pub done: usize,
    /// Its folder and counts under the picker, on screen and to a screen reader.
    pub head: String,
    pub spoken_head: String,
    pub body: PaneBody,
    /// Recently removed is shown for it (§3.4, §7): More's menu checks it.
    pub removed_shown: bool,
    /// A restore of the list itself waits, until the engine pushes the list or refuses the
    /// restore: the Undo of its removed page is off meanwhile (§10).
    pub restoring: bool,
    /// The engine's sentence for the edit it refused last, over Add an item, until the next edit.
    pub refusal: Option<String>,
    /// The index's sentence for the last new list or list's Undo refused, under the head.
    pub index_refusal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneBody {
    /// Its pull is not answered yet.
    Waiting,
    /// The engine says it was removed: "This list was removed.".
    Removed,
    Lines(Vec<Line>),
}

/// The pane's folds (§3.4, §7): each list's own folded sections, its own Done and its own Recently
/// removed, by list id and heading id, kept while the app runs and off at start. A section, Done
/// or Recently removed opened in one list stays as it was in every other.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folds {
    lists: BTreeMap<String, View>,
}

impl Folds {
    /// How the pane draws `list_id`: its own folds and Done, with the rows in `settling` keeping
    /// their place.
    pub fn view(&self, list_id: &str, settling: &BTreeSet<String>) -> View {
        let mut view = self.lists.get(list_id).cloned().unwrap_or_default();
        view.settling = settling.clone();
        view
    }

    /// Folds the section under `heading_id` in `list_id`, or opens it again.
    pub fn fold(&mut self, list_id: &str, heading_id: &str) {
        let folded = &mut self.lists.entry(list_id.to_owned()).or_default().folded;
        if !folded.remove(heading_id) {
            folded.insert(heading_id.to_owned());
        }
    }

    /// Opens Done in `list_id`, or closes it.
    pub fn open_done(&mut self, list_id: &str) {
        let view = self.lists.entry(list_id.to_owned()).or_default();
        view.done_open = !view.done_open;
    }

    /// Shows Recently removed in `list_id`, or hides it: More's check item, or its heading.
    pub fn show_removed(&mut self, list_id: &str) {
        let view = self.lists.entry(list_id.to_owned()).or_default();
        view.removed_shown = !view.removed_shown;
    }

    /// Whether Recently removed is shown in `list_id`: its pull then asks for the removed rows.
    pub fn removed_shown(&self, list_id: &str) -> bool {
        self.lists
            .get(list_id)
            .is_some_and(|view| view.removed_shown)
    }
}

/// The pane as `library` holds the lists now, showing `asked` while the index names it, drawn by
/// its own `folds`, the rows in `settling` keeping their place. The head counts the list as drawn,
/// its edits waiting included, once it is held, and by the index before.
pub fn drawing(
    library: &Library,
    asked: Option<&str>,
    folds: &Folds,
    settling: &BTreeSet<String>,
) -> PaneDrawing {
    let index = library.index();
    if index.entries().is_none() {
        return PaneDrawing::Waiting;
    }
    let index_refusal = index.refusal().map(str::to_owned);
    let Some(entry) = index.shown(asked) else {
        return PaneDrawing::NoLists {
            refusal: index_refusal,
        };
    };
    let id = &entry.list_id;
    let held = library.shown(id);
    let removed = library.state(id).is_some_and(|state| state.is_removed());
    let (open, done) = match &held {
        Some(list) => (list.open_count(), list.done_count()),
        None => (count(entry.open), count(entry.done)),
    };
    let folder = entry.folder.as_deref();
    let pending = library.state(id).map_or(&[][..], |state| state.pending());
    let mut view = folds.view(id, settling);
    let (rows_restoring, restoring) = restores(pending);
    view.restoring = rows_restoring;
    let body = match &held {
        _ if removed => PaneBody::Removed,
        Some(list) => PaneBody::Lines(panel::lines(list, &view)),
        None => PaneBody::Waiting,
    };
    PaneDrawing::List(Box::new(PaneList {
        folders: index.folders(),
        list_id: id.clone(),
        title: held.map_or_else(|| entry.title.clone(), |list| list.title),
        folder: folder.map(str::to_owned),
        moving_to: moving_to(pending),
        done,
        head: words::head_line(folder, open, done),
        spoken_head: words::spoken_head_line(folder, open, done),
        body,
        removed_shown: view.removed_shown,
        restoring,
        refusal: library
            .state(id)
            .and_then(|state| state.refusal())
            .map(str::to_owned),
        index_refusal,
    }))
}

/// The list the pane was last asked for (§3.3, §3.7): by a card, the picker, a new list made here,
/// or Undo of a list.
/// The pane turns to a new list once, when the index first holds the list its create made, as its
/// push or the index names it; until then it stays on the list it showed, and a list picked after
/// the turn stays shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asked {
    list_id: Option<String>,
    /// The create whose list the pane turned to last, by its `client_msg_id`.
    turned: Option<String>,
}

impl Asked {
    /// A card or the picker asks for `list_id`.
    pub fn ask(&mut self, list_id: &str) {
        self.list_id = Some(list_id.to_owned());
    }

    /// Undo took `removal` back (§10): a list taken back is asked for, and the pane shows it once
    /// the index names it again. An item taken back leaves the pane on the list it shows.
    pub fn taken_back(&mut self, removal: &Removal) {
        if removal.row_id.is_none() {
            self.ask(&removal.list_id);
        }
    }

    /// The list asked for now: the one the newest create made here, when `index` names what it
    /// made, holds that list, and the pane has not turned to it yet; else the last asked for.
    pub fn now(&mut self, index: &Index) -> Option<&str> {
        let created = index
            .created()
            .filter(|made| self.turned.as_deref() != Some(made.client_msg_id.as_str()))
            .filter(|made| index.entry(&made.list_id).is_some());
        if let Some(made) = created {
            self.turned = Some(made.client_msg_id.clone());
            self.list_id = Some(made.list_id.clone());
        }
        self.list_id.as_deref()
    }
}

/// What the restores in `pending` bring back: the removed rows, and whether the list itself
/// (§10).
fn restores(pending: &[PendingEdit]) -> (BTreeSet<String>, bool) {
    let restored: Vec<Option<&str>> = pending
        .iter()
        .filter_map(|edit| match &edit.op {
            ListOp::Restore { row_id, .. } => Some(row_id.as_deref()),
            _ => None,
        })
        .collect();
    let rows = restored
        .iter()
        .flatten()
        .map(|id| (*id).to_owned())
        .collect();
    (rows, restored.contains(&None))
}

/// The folder the newest folder edit in `pending` moves the list to, empty for none.
fn moving_to(pending: &[PendingEdit]) -> Option<String> {
    pending.iter().rev().find_map(|edit| match &edit.op {
        ListOp::Folder { folder } => Some(folder.clone()),
        _ => None,
    })
}

/// What the list's menu, behind More, offers for the list drawn (M85 lists §3.6, §10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListOffers {
    /// Every folder's name, in picker order, for Move to folder.
    pub folders: Vec<String>,
    /// The folder the index puts the list in, empty for none: the one Move to folder checks.
    pub folder: String,
    /// The folder the newest Move to folder still waiting takes the list to, empty for none.
    pub moving_to: Option<String>,
    /// Copy as text, once the list's pull is answered.
    pub copy: bool,
    /// Clear done, once the list's pull is answered and while an item drawn is done, a tick
    /// waiting or settling included.
    pub clear_done: bool,
    /// Recently removed is shown, which checks its item; with nothing removed it draws nothing and
    /// stays checked (§10).
    pub removed_shown: bool,
}

/// The list's menu for `list`, or none while the list is removed: More is hidden then.
pub fn list_offers(list: &PaneList) -> Option<ListOffers> {
    let held = match list.body {
        PaneBody::Removed => return None,
        PaneBody::Waiting => false,
        PaneBody::Lines(_) => true,
    };
    let folders = list
        .folders
        .names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    Some(ListOffers {
        folders,
        folder: list.folder.clone().unwrap_or_default(),
        moving_to: list.moving_to.clone(),
        copy: held,
        clear_done: held && list.done > 0,
        removed_shown: list.removed_shown,
    })
}

/// The op Move to folder sends when the owner picks `picked`, empty for no folder, as No folder's
/// own pick is: none for the folder the list will be in once its edits land, which is the one
/// checked unless a move to another still waits (§7). It is drawn when the index comes back.
pub fn folder_op(offers: &ListOffers, picked: &str) -> Option<ListOp> {
    let landing = offers.moving_to.as_deref().unwrap_or(&offers.folder);
    (picked != landing).then(|| ListOp::Folder {
        folder: picked.to_owned(),
    })
}

/// What the name typed in the head's place of the picker names (§3.3): a new list, the list's
/// new title, or a new folder for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Naming {
    /// New list, from the picker.
    NewList,
    /// Rename list, of the list titled `title` as the naming began.
    Rename { title: String },
    /// New folder, from Move to folder: what the list's menu offered as the naming began.
    NewFolder(ListOffers),
}

/// The op a name typed for `naming` makes once Return keeps it (§3.3, §10), trimmed at its ends
/// as the engine trims what it stores. A blank name is no edit, and neither is a rename to the
/// title the list had as the naming began; a change of case alone is a rename. A new list is made
/// in no folder. A new folder is a folder op as Move to folder makes it, so none for the folder
/// the list will be in once its edits land.
pub fn named(naming: &Naming, typed: &str) -> Option<ListOp> {
    let name = typed.trim();
    if name.is_empty() {
        return None;
    }
    match naming {
        Naming::NewList => Some(ListOp::Create {
            title: name.to_owned(),
            folder: None,
        }),
        Naming::Rename { title } => (name != title).then(|| ListOp::Rename {
            text: name.to_owned(),
        }),
        Naming::NewFolder(offers) => folder_op(offers, name),
    }
}

/// The drawing with the item `row_id` drawn as it is reworded, its words' entry then its note's in
/// its place (§3.5), while its lines draw it as an item the engine has named.
pub fn rewording(drawing: PaneDrawing, row_id: Option<&str>) -> PaneDrawing {
    match (drawing, row_id) {
        (PaneDrawing::List(mut list), Some(row_id)) => {
            if let PaneBody::Lines(lines) = &mut list.body {
                *lines = panel::with_rewording(std::mem::take(lines), row_id);
            }
            PaneDrawing::List(list)
        }
        (drawing, _) => drawing,
    }
}

/// What a screen reader is told as the pane is drawn again (§3.8, §5.3, §6.3), since focus is
/// elsewhere, while the pane is on screen. The index's sentence for a new list or a list's Undo
/// refused is told when it comes under the head, with lists or with none. A refused edit's
/// sentence stays on its list until the list's next edit, and the pane tells it only when it first
/// shows it: when it comes over Add an item of the list the pane already showed. Turned to a list
/// with a sentence on it, the pane says nothing of that older refusal. "This list was removed." is
/// told when the list it shows turns removed, or it turns to a removed list.
pub fn news<'a>(was: Option<&PaneDrawing>, now: &'a PaneDrawing, on_screen: bool) -> Vec<&'a str> {
    if !on_screen {
        return Vec::new();
    }
    let was_made = was.map(PaneDrawing::index_refusal);
    let not_made = now
        .index_refusal()
        .filter(|sentence| was_made.is_some_and(|was| was != Some(*sentence)));
    let PaneDrawing::List(list) = now else {
        return not_made.into_iter().collect();
    };
    let same = match was {
        Some(PaneDrawing::List(old)) if old.list_id == list.list_id => Some(old),
        _ => None,
    };
    let refused = same.and_then(|old| {
        list.refusal
            .as_deref()
            .filter(|_| old.refusal != list.refusal && matches!(list.body, PaneBody::Lines(_)))
    });
    let was_removed = same.is_some_and(|old| old.body == PaneBody::Removed);
    let removed = (list.body == PaneBody::Removed && !was_removed).then_some(words::LIST_REMOVED);
    not_made.into_iter().chain(refused).chain(removed).collect()
}

fn count(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}
