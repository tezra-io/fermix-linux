//! The removals made here, which Undo takes back (M85 lists §3.7, §10): an item's or a whole
//! list's, each under a token its toast's Undo names. Ctrl+Z takes back the newest still kept, and
//! the one before it when pressed again, up to 20 back across lists while the app runs; past 20
//! the oldest goes. Each is taken back once, by Undo or by Ctrl+Z, whichever comes first: the
//! second sends nothing, and so does either after Restore or the removed page's Undo brought the
//! same row or list back. A removal and its restore are edits the library sends as any other.

use super::library::Library;
use crate::chat_wire::protocol::{ClientEvent, ListOp};

/// The most removals kept for Undo.
pub const MAX_KEPT: usize = 20;

/// One removal made here: of the row `row_id` of `list_id`, or of the list itself when none, with
/// the words its toast names it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub token: u64,
    pub list_id: String,
    pub row_id: Option<String>,
    pub text: String,
}

impl Removal {
    /// The op that takes it back: a restore of its row, or of the list under its own title.
    pub fn restore(&self) -> ListOp {
        ListOp::Restore {
            row_id: self.row_id.clone(),
            title: None,
        }
    }
}

/// The removals made here and not taken back, oldest first, at most 20.
#[derive(Debug, Default)]
pub struct Removals {
    /// The last token given.
    made: u64,
    kept: Vec<Removal>,
}

impl Removals {
    /// A removal made here, kept under a new token, which is answered. A removal of the same row,
    /// or of the same list, still kept goes: it was there to remove again, so that removal was
    /// taken back some other way. Past 20 kept, the oldest goes.
    pub fn made(&mut self, list_id: &str, row_id: Option<&str>, text: &str) -> u64 {
        self.kept
            .retain(|removal| !(removal.list_id == list_id && removal.row_id.as_deref() == row_id));
        self.made += 1;
        self.kept.push(Removal {
            token: self.made,
            list_id: list_id.to_owned(),
            row_id: row_id.map(str::to_owned),
            text: text.to_owned(),
        });
        if self.kept.len() > MAX_KEPT {
            self.kept.remove(0);
        }
        self.made
    }

    /// The token of the newest removal still kept: the one Ctrl+Z takes back.
    pub fn newest(&self) -> Option<u64> {
        self.kept.last().map(|removal| removal.token)
    }

    /// The removal `token` names, taken back: answered once, and never again.
    pub fn take_back(&mut self, token: u64) -> Option<Removal> {
        let at = self
            .kept
            .iter()
            .position(|removal| removal.token == token)?;
        Some(self.kept.remove(at))
    }
}

/// Remove, of the row `row_id` of `list_id`, or Remove list with none: sent through `library` as
/// any edit is, and kept for Undo under the token answered with what to send. None when the
/// library makes no edit of it, a row it has not named yet or 64 edits already waiting: then
/// nothing is kept, and no toast offers Undo.
pub fn remove(
    removals: &mut Removals,
    library: &mut Library,
    list_id: &str,
    row_id: Option<&str>,
    text: &str,
    client_msg_id: String,
) -> Option<(u64, Vec<ClientEvent>)> {
    let sent = library.remove(list_id, row_id, client_msg_id)?;
    Some((removals.made(list_id, row_id, text), sent))
}

/// Undo: takes back the removal `token` names, a toast's, or with none the newest still kept,
/// Ctrl+Z's, and sends its restore through `library` as any edit is. None when there is none to
/// take back: nothing is sent.
pub fn undo(
    removals: &mut Removals,
    library: &mut Library,
    token: Option<u64>,
    client_msg_id: String,
) -> Option<(Removal, Vec<ClientEvent>)> {
    let token = token.or_else(|| removals.newest())?;
    let removal = removals.take_back(token)?;
    let sent = library.edit(&removal.list_id, removal.restore(), client_msg_id);
    Some((removal, sent))
}

/// Restore, of the removed row `row_id` of `list_id` from Recently removed, or of the list itself
/// with none, by the Undo of the page a removed list shows (§3.7, §10): sent through `library` as
/// any edit is, whoever removed it. Its removal made here, if one is still kept, is taken back
/// with it and answered, so neither its toast's Undo nor Ctrl+Z sends a second restore.
pub fn restore(
    removals: &mut Removals,
    library: &mut Library,
    list_id: &str,
    row_id: Option<&str>,
    client_msg_id: String,
) -> (Option<Removal>, Vec<ClientEvent>) {
    let own = removals
        .kept
        .iter()
        .find(|removal| removal.list_id == list_id && removal.row_id.as_deref() == row_id)
        .map(|removal| removal.token);
    let taken = own.and_then(|token| removals.take_back(token));
    let restore = ListOp::Restore {
        row_id: row_id.map(str::to_owned),
        title: None,
    };
    (taken, library.edit(list_id, restore, client_msg_id))
}
