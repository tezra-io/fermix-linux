//! The lists' words (M85 lists §4.3): the macOS app's `lists.*` strings as written, and the two
//! this app adds. The counts and names are composed here, so a card, the pane and a screen reader
//! read alike: on screen the parts of a line stand apart by " · ", and to a screen reader by ", ".

pub const LISTS: &str = "Lists";
pub const ADD_ITEM: &str = "Add an item";
pub const SHOW_LESS: &str = "Show less";
pub const DONE: &str = "Done";
pub const NEW_LIST: &str = "New list";
pub const RENAME_LIST: &str = "Rename list";
pub const MOVE_TO_FOLDER: &str = "Move to folder";
pub const MOVE_TO_LIST: &str = "Move to list";
pub const NEW_FOLDER: &str = "New folder";
pub const NO_FOLDER: &str = "No folder";
pub const COPY_AS_TEXT: &str = "Copy as text";
pub const CLEAR_DONE: &str = "Clear done";
pub const RECENTLY_REMOVED: &str = "Recently removed";
pub const REMOVE_LIST: &str = "Remove list";
pub const UNDO: &str = "Undo";
pub const LIST_REMOVED: &str = "This list was removed.";
pub const HIDE_LISTS: &str = "Hide lists";
pub const OPEN_LIST: &str = "Open list";
pub const NO_LISTS: &str = "No lists yet.";
pub const MOVE_UP: &str = "Move up";
pub const MOVE_DOWN: &str = "Move down";
pub const REMOVE: &str = "Remove";
pub const RESTORE: &str = "Restore";
pub const LIST_NAME: &str = "List name";
pub const FOLDER_NAME: &str = "Folder name";
pub const ADD_NOTE: &str = "Add a note";
/// No macOS string: the Keyboard Shortcuts dialog's line for Ctrl+Shift+L.
pub const SHOW_OR_HIDE: &str = "Show or hide lists";
/// No macOS string: its edits waiting to send have no cap.
pub const TOO_MANY: &str = "Too many changes are waiting to send";

/// Every fixed word the lists show, so a test can hold them to the copy rules.
pub const STRINGS: [&str; 28] = [
    LISTS,
    ADD_ITEM,
    SHOW_LESS,
    DONE,
    NEW_LIST,
    RENAME_LIST,
    MOVE_TO_FOLDER,
    MOVE_TO_LIST,
    NEW_FOLDER,
    NO_FOLDER,
    COPY_AS_TEXT,
    CLEAR_DONE,
    RECENTLY_REMOVED,
    REMOVE_LIST,
    UNDO,
    LIST_REMOVED,
    HIDE_LISTS,
    OPEN_LIST,
    NO_LISTS,
    MOVE_UP,
    MOVE_DOWN,
    REMOVE,
    RESTORE,
    LIST_NAME,
    FOLDER_NAME,
    ADD_NOTE,
    SHOW_OR_HIDE,
    TOO_MANY,
];

/// "6 open".
pub fn open(count: usize) -> String {
    format!("{count} open")
}

/// "2 done".
pub fn done(count: usize) -> String {
    format!("{count} done")
}

/// "6 open · 2 done", a card's header and the pane's head.
pub fn counts(open_count: usize, done_count: usize) -> String {
    format!("{} · {}", open(open_count), done(done_count))
}

/// "6 open, 2 done", the counts to a screen reader, which would read the dot out.
pub fn spoken_counts(open_count: usize, done_count: usize) -> String {
    format!("{}, {}", open(open_count), done(done_count))
}

/// "Projects · 6 open · 2 done" under the pane's picker, or the counts alone for a list in no
/// folder.
pub fn head_line(folder: Option<&str>, open_count: usize, done_count: usize) -> String {
    let counts = counts(open_count, done_count);
    match folder {
        Some(folder) => format!("{folder} · {counts}"),
        None => counts,
    }
}

/// "2 more", under a one-list card's first four items.
pub fn more(count: usize) -> String {
    format!("{count} more")
}

/// "and 2 more lists", after a card's three groups.
pub fn more_lists(count: usize) -> String {
    format!("and {count} more lists")
}

/// "Done · 2", the pane's fold over its done items.
pub fn done_heading(count: usize) -> String {
    format!("{DONE} · {count}")
}

/// "Done, 2", the fold to a screen reader.
pub fn spoken_done_heading(count: usize) -> String {
    format!("{DONE}, {count}")
}

/// "Removed Lemons.", the toast after a removal.
pub fn removed(text: &str) -> String {
    format!("Removed {text}.")
}

/// "Groceries · 3 open", a folded card's button for one list.
pub fn folded(title: &str, open_count: usize) -> String {
    format!("{title} · {}", open(open_count))
}

/// "Fermix, list, 6 open": a one-list card, and a folded card's button, to a screen reader.
pub fn list_label(title: &str, open_count: usize) -> String {
    format!("{title}, list, {}", open(open_count))
}

/// "Fermix, Portfolio site and Groceries, lists": a card of several lists, to a screen reader.
pub fn lists_label(titles: &[&str]) -> String {
    let Some((last, rest)) = titles.split_last() else {
        return "lists".to_owned();
    };
    if rest.is_empty() {
        return format!("{last}, lists");
    }
    format!("{} and {last}, lists", rest.join(", "))
}

/// A list's or a folder's name as a menu item's label, which reads `_` as a mnemonic: each `_`
/// doubled, so it shows as written. A section heading does not read it, so a name there stays as
/// it is.
pub fn menu_name(name: &str) -> String {
    name.replace('_', "__")
}
