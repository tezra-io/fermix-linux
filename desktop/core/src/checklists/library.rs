//! Every list the chat draws, and the connection's share of them (M85 lists §5.2 to §5.4;
//! PROTOCOL.md "Lists").
//!
//! It mirrors what the engine holds for this connection: each list pulled, in the order pulled, at
//! most 16, a pull past that letting the oldest go. A list is pulled when a card or the pane first
//! draws it and the index of this connection names it; one the index does not name was removed and
//! is never pulled, until an index names it. Every list held is pulled again at each handshake, in
//! its order, once the index answers. An edit is drawn at once and sent; one the engine has not
//! answered when the connection drops is sent again at the next handshake, in the order made, and
//! the engine's `client_msg_id` dedup makes it land once. At most 64 edits wait.
//!
//! Each call returns what to send, and nothing is returned before a handshake whose version brings
//! lists: what is drawn meanwhile is pulled, and what is edited sent, at the next one. The session
//! feeds it every list event and the answers to its edits, and nothing else writes it.

use super::draft;
use super::index::{Created, Index};
use super::state::ListState;
use super::words::TOO_MANY;
use super::LISTS_SINCE;
use crate::chat_wire::protocol::{ClientEvent, List, ListEntry, ListOp, ServerError, PROFILE};
use crate::chat_wire::timeline::MAX_PENDING;
use std::collections::{BTreeMap, BTreeSet};

/// The most lists a connection holds (PROTOCOL.md "Lists", Holding).
pub const MAX_HELD: usize = 16;
/// The most edits that wait for the engine's answer, and the most lists that wait for an index to
/// name them, the ones held when a connection dropped among them: the chat outbox's own cap.
pub const MAX_WAITING: usize = MAX_PENDING;
const _: () = assert!(MAX_HELD <= MAX_WAITING, "the lists held wait whole");

/// What the engine holds for this connection: a list, or the list a create made before a push or
/// an index names it, by that create's `client_msg_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    List(String),
    Made(String),
}

/// An edit sent and not yet answered, as it goes on the wire every time it is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Outgoing {
    client_msg_id: String,
    /// `None` for a create.
    list_id: Option<String>,
    op: ListOp,
}

impl Outgoing {
    fn event(&self) -> ClientEvent {
        ClientEvent::ChecklistEdit {
            client_msg_id: self.client_msg_id.clone(),
            profile_id: PROFILE.to_owned(),
            list_id: self.list_id.clone(),
            op: self.op.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct Library {
    index: Index,
    states: BTreeMap<String, ListState>,
    /// Oldest pull first.
    held: Vec<Held>,
    /// The held lists pulled with their removed rows.
    with_removed: BTreeSet<String>,
    /// Lists drawn and not pulled, each with whether its removed rows were asked for: before the
    /// index of this connection answers, or while it does not name them. Pulled, in this order,
    /// once an index names them.
    waiting: Vec<(String, bool)>,
    /// The index has answered on this connection.
    index_current: bool,
    /// Every edit sent and not yet answered, in the order made.
    unanswered: Vec<Outgoing>,
    /// The creates the engine took, by `client_msg_id`, with the title each asked for. Each waits
    /// for the push that names what it made, or for an index holding its title. None outlives
    /// the first index of the next connection.
    creating: BTreeMap<String, String>,
    connected: bool,
}

impl Library {
    pub fn index(&self) -> &Index {
        &self.index
    }

    pub fn state(&self, list_id: &str) -> Option<&ListState> {
        self.states.get(list_id)
    }

    /// The list as drawn: the engine's, with the edits made here applied.
    pub fn shown(&self, list_id: &str) -> Option<List> {
        self.states.get(list_id)?.shown()
    }

    /// How many creates wait for what they made to be named.
    pub fn creates_waiting(&self) -> usize {
        self.creating.len()
    }

    /// A card or the pane draws this list: it is pulled unless the engine holds it already, with
    /// its removed rows where Recently removed is shown. It waits for an index of this connection
    /// that names it.
    pub fn draw(&mut self, list_id: &str, removed: bool) -> Vec<ClientEvent> {
        let holds = self.holds(list_id) && (!removed || self.with_removed.contains(list_id));
        if holds {
            return Vec::new();
        }
        let ready = self.connected && self.index.served() && self.index_current;
        if !ready || self.index.lacks(list_id) {
            self.wait(list_id, removed);
            return Vec::new();
        }
        self.pull(list_id, removed)
    }

    /// One edit of a list: drawn at once and sent. Words the engine would trim to nothing are no
    /// edit, and neither is an op naming a row added here that the engine has not named yet: it
    /// has no id the engine knows. Past 64 edits waiting, the edit is not drawn, and the list says
    /// why. Every edit passes here.
    pub fn edit(&mut self, list_id: &str, op: ListOp, client_msg_id: String) -> Vec<ClientEvent> {
        let names_local = draft::named_rows(&op).into_iter().any(draft::is_local);
        if !draft::sendable(&op) || names_local {
            return Vec::new();
        }
        if self.unanswered.len() >= MAX_WAITING {
            self.state_mut(list_id).refuse(TOO_MANY.to_owned());
            return Vec::new();
        }
        self.state_mut(list_id)
            .edit(op.clone(), client_msg_id.clone());
        let mut out = self.draw(list_id, false);
        let sent = Outgoing {
            client_msg_id,
            list_id: Some(list_id.to_owned()),
            op,
        };
        out.extend(self.deliver(sent.event()));
        self.unanswered.push(sent);
        out
    }

    /// A new list, in a folder when one is named. It is drawn once the engine has made it: the
    /// index then says what this create made, and the pane turns to it. `None` when it is no
    /// edit: a blank title, or 64 edits already waiting, which the index's refusal says.
    pub fn create(
        &mut self,
        title: &str,
        folder: Option<&str>,
        client_msg_id: String,
    ) -> Option<Vec<ClientEvent>> {
        let op = ListOp::Create {
            title: title.to_owned(),
            folder: folder.map(str::to_owned),
        };
        if !draft::sendable(&op) {
            return None;
        }
        if self.unanswered.len() >= MAX_WAITING {
            self.index.refuse(Some(TOO_MANY.to_owned()));
            return None;
        }
        self.index.refuse(None);
        let sent = Outgoing {
            client_msg_id,
            list_id: None,
            op,
        };
        let out = self.deliver(sent.event());
        self.unanswered.push(sent);
        Some(out)
    }

    /// A handshake completed at `version`. A new connection holds nothing, so what the last one
    /// held waits to be pulled again, in the order it was pulled, ahead of anything drawn since;
    /// past `MAX_WAITING` in all, the oldest drawn since drop. A daemon whose version brings lists
    /// is first sent every edit it has not answered, in the order made, then asked for the index,
    /// which pulls them: an edit it had applied is answered duplicate before the list carrying it
    /// arrives, so that list confirms it and it is never drawn twice.
    pub fn negotiated(&mut self, version: u64) -> Vec<ClientEvent> {
        self.connected = true;
        self.index_current = false;
        self.index.serve(version >= LISTS_SINCE);
        let again: Vec<(String, bool)> = self
            .held
            .iter()
            .filter_map(|held| match held {
                Held::List(id) => Some((id.clone(), self.with_removed.contains(id))),
                Held::Made(_) => None,
            })
            .collect();
        let since = std::mem::take(&mut self.waiting);
        let since: Vec<(String, bool)> = since
            .into_iter()
            .filter(|(id, _)| !self.holds(id))
            .collect();
        let room = MAX_WAITING.saturating_sub(again.len());
        let dropped = since.len().saturating_sub(room);
        self.waiting = again
            .into_iter()
            .chain(since.into_iter().skip(dropped))
            .collect();
        self.held.clear();
        self.with_removed.clear();
        let mut out: Vec<ClientEvent> = self.unanswered.iter().map(Outgoing::event).collect();
        out.push(ClientEvent::ChecklistsPull {
            profile_id: PROFILE.to_owned(),
        });
        out.into_iter()
            .flat_map(|event| self.deliver(event))
            .collect()
    }

    /// The connection ended: nothing asked on it will be answered. What is held is kept, to be
    /// pulled again once the next handshake's index answers.
    pub fn disconnected(&mut self) {
        self.connected = false;
        self.index_current = false;
    }

    /// The index (`checklists`). It names what the waiting creates made, by their titles, and
    /// pulls every waiting list it names. The first of a connection lets go of the creates it
    /// cannot name.
    pub fn received_index(&mut self, lists: Vec<ListEntry>) -> Vec<ClientEvent> {
        self.index.show(lists);
        let first = !self.index_current;
        self.index_current = true;
        let mut out = self.find_creates(first);
        out.extend(self.pull_waiting());
        out
    }

    /// One list whole (`checklist`), as a pull's answer or a push. `made` names what this
    /// connection's creates and adds made since the last push.
    pub fn received_list(&mut self, list: List, made: &BTreeMap<String, String>) {
        let found: Vec<(String, String)> = made
            .iter()
            .filter(|(client_msg_id, _)| self.creating.contains_key(*client_msg_id))
            .map(|(client_msg_id, list_id)| (client_msg_id.clone(), list_id.clone()))
            .collect();
        for (client_msg_id, list_id) in found {
            self.found(&client_msg_id, &list_id);
        }
        let list_id = list.list_id.clone();
        self.state_mut(&list_id).received(list, made);
    }

    /// An `accepted` for an edit sent here: true, or false for any other request's. The engine
    /// holds the list a create made from the moment it applies it, as the newest, so the mirror
    /// holds a place for it until a push or an index names it; not for a duplicate, which it
    /// applied on an earlier connection and which holds nothing of this one.
    pub fn accepted(&mut self, client_msg_id: &str, duplicate: bool) -> bool {
        let Some(at) = self.position(client_msg_id) else {
            return false;
        };
        let sent = self.unanswered.remove(at);
        let Some(list_id) = sent.list_id else {
            self.create_accepted(sent.client_msg_id, &sent.op, duplicate);
            return true;
        };
        self.state_mut(&list_id).accepted(client_msg_id);
        true
    }

    /// A refusal the daemon sent. The lists' only when it names an edit sent here; a refusal that
    /// names no request, or another request, is the chat's, and so is a refused pull, which names
    /// none. `None` when it is not the lists', else what to send.
    pub fn refusal(&mut self, error: &ServerError) -> Option<Vec<ClientEvent>> {
        let client_msg_id = error.client_msg_id.as_deref()?;
        let reported = || format!("The daemon reported: {}", error.reason);
        let sentence = error.message.clone().unwrap_or_else(reported);
        self.refused(client_msg_id, sentence)
    }

    /// The edit `client_msg_id` was refused, in `sentence`: it stops being drawn, the list shows
    /// the sentence, and a held list is pulled again (PROTOCOL.md "Lists", Refusals). A refused
    /// create's sentence is the index's. `None` when the id names no edit sent here.
    pub fn refused(&mut self, client_msg_id: &str, sentence: String) -> Option<Vec<ClientEvent>> {
        let at = self.position(client_msg_id)?;
        let sent = self.unanswered.remove(at);
        let Some(list_id) = sent.list_id else {
            self.index.refuse(Some(sentence));
            return Some(Vec::new());
        };
        self.state_mut(&list_id).refused(client_msg_id, sentence);
        if !self.holds(&list_id) {
            return Some(Vec::new());
        }
        let removed = self.with_removed.contains(&list_id);
        Some(self.pull(&list_id, removed))
    }

    fn position(&self, client_msg_id: &str) -> Option<usize> {
        self.unanswered
            .iter()
            .position(|sent| sent.client_msg_id == client_msg_id)
    }

    fn create_accepted(&mut self, client_msg_id: String, op: &ListOp, duplicate: bool) {
        if let ListOp::Create { title, .. } = op {
            self.creating
                .insert(client_msg_id.clone(), title.trim().to_owned());
        }
        if !duplicate {
            self.hold(Held::Made(client_msg_id), false);
        }
    }

    /// The creates an index names by their titles: each is the list it made, pulled unless held,
    /// and shown. On the first index of a connection a create it does not name is let go.
    fn find_creates(&mut self, letting_go: bool) -> Vec<ClientEvent> {
        let creates: Vec<(String, String)> = self
            .creating
            .iter()
            .map(|(client_msg_id, title)| (client_msg_id.clone(), title.clone()))
            .collect();
        let mut out = Vec::new();
        for (client_msg_id, title) in creates {
            let named = self.index.titled(&title).map(|entry| entry.list_id.clone());
            match named {
                Some(list_id) => {
                    self.found(&client_msg_id, &list_id);
                    out.extend(self.draw(&list_id, false));
                }
                None if letting_go => self.let_go(&client_msg_id),
                None => {}
            }
        }
        out
    }

    /// A create named what it made: the place the mirror held for it is that list's, and the
    /// pane turns to it.
    fn found(&mut self, client_msg_id: &str, list_id: &str) {
        self.creating.remove(client_msg_id);
        let made = Held::Made(client_msg_id.to_owned());
        let at = self.held.iter().position(|held| *held == made);
        let already = self.holds(list_id);
        if let Some(at) = at.filter(|_| already) {
            self.held.remove(at);
        }
        if let Some(at) = at.filter(|_| !already) {
            self.held[at] = Held::List(list_id.to_owned());
        }
        self.index.made(Created {
            client_msg_id: client_msg_id.to_owned(),
            list_id: list_id.to_owned(),
        });
    }

    fn let_go(&mut self, client_msg_id: &str) {
        self.creating.remove(client_msg_id);
        let made = Held::Made(client_msg_id.to_owned());
        self.held.retain(|held| *held != made);
    }

    fn pull_waiting(&mut self) -> Vec<ClientEvent> {
        let waiting = std::mem::take(&mut self.waiting);
        let (ready, still): (Vec<_>, Vec<_>) = waiting
            .into_iter()
            .partition(|(id, _)| !self.index.lacks(id));
        self.waiting = still;
        ready
            .into_iter()
            .flat_map(|(id, removed)| self.pull(&id, removed))
            .collect()
    }

    /// A list drawn before it can be pulled. Past 64 waiting the oldest is dropped; a card or the
    /// pane that still draws it asks again.
    fn wait(&mut self, list_id: &str, removed: bool) {
        if let Some(drawn) = self.waiting.iter_mut().find(|(id, _)| id == list_id) {
            drawn.1 |= removed;
            return;
        }
        self.waiting.push((list_id.to_owned(), removed));
        if self.waiting.len() > MAX_WAITING {
            self.waiting.remove(0);
        }
    }

    /// The one way a list is pulled. The engine makes every list it is asked for the newest it
    /// holds, whether or not it held it already, so the mirror does the same with every pull, the
    /// one a refused edit sends included: past 16 the two then let go of the same list.
    fn pull(&mut self, list_id: &str, removed: bool) -> Vec<ClientEvent> {
        self.hold(Held::List(list_id.to_owned()), removed);
        self.deliver(ClientEvent::ChecklistPull {
            profile_id: PROFILE.to_owned(),
            list_id: list_id.to_owned(),
            removed: removed.then_some(true),
        })
    }

    /// Mirrors the engine's holding: the list becomes the newest held, and past 16 the oldest is
    /// let go, so nothing more of it arrives until it is drawn again.
    fn hold(&mut self, held: Held, removed: bool) {
        self.held.retain(|old| *old != held);
        if let Held::List(id) = &held {
            self.with_removed.remove(id);
        }
        if let (Held::List(id), true) = (&held, removed) {
            self.with_removed.insert(id.clone());
        }
        self.held.push(held);
        if self.held.len() <= MAX_HELD {
            return;
        }
        if let Held::List(gone) = self.held.remove(0) {
            self.with_removed.remove(&gone);
        }
    }

    fn holds(&self, list_id: &str) -> bool {
        self.held
            .iter()
            .any(|held| matches!(held, Held::List(id) if id == list_id))
    }

    fn state_mut(&mut self, list_id: &str) -> &mut ListState {
        self.states.entry(list_id.to_owned()).or_default()
    }

    /// The event, on a connection to a daemon whose version brings lists; else nothing.
    fn deliver(&self, event: ClientEvent) -> Vec<ClientEvent> {
        if !self.connected || !self.index.served() {
            return Vec::new();
        }
        vec![event]
    }
}
