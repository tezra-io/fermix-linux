//! One list as this app holds it (M85 lists §5.3), as a value: the engine's last push, the edits
//! sent and not yet confirmed, and the engine's sentence for the last one it refused. The library
//! is its one writer.

use super::draft;
use crate::chat_wire::protocol::{List, ListOp};
use std::collections::BTreeMap;

/// One edit sent here and not yet confirmed by a push: drawn over the engine's list until then.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEdit {
    pub client_msg_id: String,
    pub op: ListOp,
    /// The engine answered `accepted`: it holds the change, and the next list it sends carries it.
    pub accepted: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListState {
    /// The list as the engine last sent it, `None` until a pull is answered.
    engine: Option<List>,
    /// Oldest first, each drawn over `engine` until a push confirms it.
    pending: Vec<PendingEdit>,
    /// Why the last edit made here did not stand, until the next edit.
    refusal: Option<String>,
}

impl ListState {
    pub fn engine(&self) -> Option<&List> {
        self.engine.as_ref()
    }

    pub fn pending(&self) -> &[PendingEdit] {
        &self.pending
    }

    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// What every view draws: the engine's list with every edit not yet confirmed applied over it,
    /// in the order they were made.
    pub fn shown(&self) -> Option<List> {
        let engine = self.engine.as_ref()?;
        Some(draft::applying(&self.pending, engine))
    }

    /// The engine says the list was removed and can still be restored.
    pub fn is_removed(&self) -> bool {
        self.engine
            .as_ref()
            .is_some_and(|list| list.removed_at.is_some())
    }

    /// An edit made here: drawn at once, and sent by the library.
    pub fn edit(&mut self, op: ListOp, client_msg_id: String) {
        self.pending.push(PendingEdit {
            client_msg_id,
            op,
            accepted: false,
        });
        self.refusal = None;
    }

    /// An edit this app would not send, with the sentence that says why, until the next edit.
    pub fn refuse(&mut self, sentence: String) {
        self.refusal = Some(sentence);
    }

    /// The engine took the edit; its change is in the next list it sends.
    pub fn accepted(&mut self, client_msg_id: &str) {
        let edit = self
            .pending
            .iter_mut()
            .find(|edit| edit.client_msg_id == client_msg_id);
        if let Some(edit) = edit {
            edit.accepted = true;
        }
    }

    /// A list the engine sent: the answer to a pull, or a push after a change. One whose rev is
    /// below the one shown is dropped, so a late answer never takes back a change already drawn.
    /// Every edit the engine had accepted is in it, and so is every add `made` names; the rest
    /// stay drawn over it. Answers whether the list was taken.
    pub fn received(&mut self, list: List, made: &BTreeMap<String, String>) -> bool {
        if self
            .engine
            .as_ref()
            .is_some_and(|shown| list.rev < shown.rev)
        {
            return false;
        }
        self.engine = Some(list);
        self.pending
            .retain(|edit| !edit.accepted && !made.contains_key(&edit.client_msg_id));
        true
    }

    /// The engine refused the edit: it changed nothing, so it stops being drawn and the row is
    /// the engine's again, under the engine's sentence.
    pub fn refused(&mut self, client_msg_id: &str, sentence: String) {
        self.dropped(client_msg_id);
        self.refusal = Some(sentence);
    }

    /// The engine refused the edit, and its sentence is not the list's to show: it stops being
    /// drawn, and the list keeps the sentence it had.
    pub fn dropped(&mut self, client_msg_id: &str) {
        self.pending
            .retain(|edit| edit.client_msg_id != client_msg_id);
    }
}
