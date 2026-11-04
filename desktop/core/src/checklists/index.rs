//! The index of the owner's lists as the engine last sent it (PROTOCOL.md "Lists", `checklists`),
//! whether the daemon serves lists at all, and the picker's order (M85 lists §3.3).

use crate::chat_wire::protocol::ListEntry;

/// A create sent here and the list it made: the pane turns to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub client_msg_id: String,
    pub list_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    /// The negotiated version brings lists; the toggle and every list event wait for it.
    served: bool,
    /// `None` until the index is answered.
    entries: Option<Vec<ListEntry>>,
    /// Why the last new list did not stand, until the next one.
    refusal: Option<String>,
    /// The list the newest create sent here made.
    created: Option<Created>,
}

impl Index {
    pub fn served(&self) -> bool {
        self.served
    }

    pub fn entries(&self) -> Option<&[ListEntry]> {
        self.entries.as_deref()
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    pub fn created(&self) -> Option<&Created> {
        self.created.as_ref()
    }

    pub fn entry(&self, list_id: &str) -> Option<&ListEntry> {
        self.entries
            .as_ref()?
            .iter()
            .find(|entry| entry.list_id == list_id)
    }

    /// Whether the index, once it has answered, does not name the list. The index holds every
    /// live list, so such a list was removed: it is drawn so and never pulled.
    pub fn lacks(&self, list_id: &str) -> bool {
        self.entries
            .as_ref()
            .is_some_and(|entries| !entries.iter().any(|entry| entry.list_id == list_id))
    }

    /// The live list holding this title, compared case-blind as the engine keeps titles apart.
    pub fn titled(&self, title: &str) -> Option<&ListEntry> {
        let wanted = title.to_lowercase();
        self.entries
            .as_ref()?
            .iter()
            .find(|entry| entry.title.to_lowercase() == wanted)
    }

    /// The picker's folders and loose lists.
    pub fn folders(&self) -> Folders {
        Folders::of(self.entries.as_deref().unwrap_or_default())
    }

    /// The list the pane shows: the one asked for while the index holds it, else the first in
    /// picker order. `None` before the index answers, and when there are no lists.
    pub fn shown(&self, asked: Option<&str>) -> Option<&ListEntry> {
        let entries = self.entries.as_ref()?;
        let asked = asked.and_then(|id| entries.iter().find(|entry| entry.list_id == id));
        // The picker opens with the first folder's first list, which is the first list in any
        // folder, since folders go in the order the index first names them.
        let first = entries.iter().find(|entry| entry.folder.is_some());
        asked.or(first).or_else(|| entries.first())
    }

    pub(super) fn serve(&mut self, served: bool) {
        self.served = served;
    }

    /// A new index: it also ends the last new list's refusal.
    pub(super) fn show(&mut self, entries: Vec<ListEntry>) {
        self.entries = Some(entries);
        self.refusal = None;
    }

    pub(super) fn refuse(&mut self, sentence: Option<String>) {
        self.refusal = sentence;
    }

    pub(super) fn made(&mut self, created: Created) {
        self.created = Some(created);
    }
}

/// One folder of the picker, with its lists in the index's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    pub name: String,
    pub lists: Vec<ListEntry>,
}

/// The picker's order (M85 lists §3.3): each folder as a heading over its lists, in the order the
/// index first names it, then the lists in no folder.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folders {
    pub folders: Vec<Folder>,
    pub loose: Vec<ListEntry>,
}

impl Folders {
    pub fn of(entries: &[ListEntry]) -> Folders {
        let mut names: Vec<&str> = Vec::new();
        for folder in entries.iter().filter_map(|entry| entry.folder.as_deref()) {
            if !names.contains(&folder) {
                names.push(folder);
            }
        }
        let folders = names
            .into_iter()
            .map(|name| Folder {
                name: name.to_owned(),
                lists: in_folder(entries, Some(name)),
            })
            .collect();
        Folders {
            folders,
            loose: in_folder(entries, None),
        }
    }

    /// The folder names, in picker order.
    pub fn names(&self) -> Vec<&str> {
        self.folders
            .iter()
            .map(|folder| folder.name.as_str())
            .collect()
    }

    /// The lists an item of `list_id` can move to (M85 lists §3.6): these, in the same order,
    /// without `list_id`, a folder left with no list dropped.
    pub fn without(&self, list_id: &str) -> Folders {
        let others = |lists: &[ListEntry]| -> Vec<ListEntry> {
            lists
                .iter()
                .filter(|entry| entry.list_id != list_id)
                .cloned()
                .collect()
        };
        let folders = self
            .folders
            .iter()
            .map(|folder| Folder {
                name: folder.name.clone(),
                lists: others(&folder.lists),
            })
            .filter(|folder| !folder.lists.is_empty())
            .collect();
        Folders {
            folders,
            loose: others(&self.loose),
        }
    }

    /// Whether no list is named.
    pub fn is_empty(&self) -> bool {
        self.loose.is_empty() && self.folders.iter().all(|folder| folder.lists.is_empty())
    }
}

fn in_folder(entries: &[ListEntry], folder: Option<&str>) -> Vec<ListEntry> {
    entries
        .iter()
        .filter(|entry| entry.folder.as_deref() == folder)
        .cloned()
        .collect()
}
