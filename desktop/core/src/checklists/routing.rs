//! Which of the daemon's events are the lists' (PROTOCOL.md "Lists"): the index, a list, and the
//! answers to the edits sent here, which the library takes; every other event is the timeline's.

use super::library::Library;
use crate::chat_wire::protocol::{ClientEvent, ServerEvent, Unreadable};
use crate::chat_wire::timeline::REFUSED_UNREAD;

/// The one refusal naming an edit that leaves it waiting: the home is moving away
/// (`primary_unreachable`), whose sentence says Fermix keeps the message, and the connection stays
/// open. It is the chat's; the edit stays drawn and is sent again at the next handshake. Every
/// other refusal that names an edit refuses it: one refused for its shape and kept would be
/// refused again at every handshake. The refusals that end a connection name no edit, so they are
/// the chat's, and an edit unanswered when one comes waits for the next handshake, as the chat's
/// outbox does.
const KEEPS_THE_EDIT: [&str; 1] = ["primary_unreachable"];

/// The lists' share of what the daemon sends, taken by the library: the index, a list, and the
/// answers to the edits sent here, read or not. `None` for any other event, the timeline's: an
/// `accepted` or a refusal naming no edit sent here, a refused pull, which names none, and a
/// refusal that keeps its edit (`KEEPS_THE_EDIT`).
pub fn lists_event(lists: &mut Library, event: &ServerEvent) -> Option<Vec<ClientEvent>> {
    match event {
        ServerEvent::Checklists { lists: index } => Some(lists.received_index(index.clone())),
        ServerEvent::Checklist { list, made } => {
            lists.received_list(list.as_ref().clone(), made);
            Some(Vec::new())
        }
        ServerEvent::Accepted {
            client_msg_id,
            duplicate,
            ..
        } => lists.accepted(client_msg_id, *duplicate).then(Vec::new),
        ServerEvent::Error(error) if KEEPS_THE_EDIT.contains(&error.reason.as_str()) => None,
        ServerEvent::Error(error) => lists.refusal(error),
        ServerEvent::Unreadable(unread) => unread_answer(lists, unread),
        _ => None,
    }
}

/// An answer to an edit sent here that this build could not read: a refusal stops drawing the
/// edit, in this app's words; an `accepted` is taken as no duplicate, its list's next push
/// confirming the edit either way.
fn unread_answer(lists: &mut Library, unread: &Unreadable) -> Option<Vec<ClientEvent>> {
    let id = unread.client_msg_id.as_deref()?;
    match unread.kind.as_str() {
        "error" if unread.r#ref.is_none() => lists.refused(id, REFUSED_UNREAD.to_owned()),
        "accepted" => lists.accepted(id, false).then(Vec::new),
        _ => None,
    }
}
