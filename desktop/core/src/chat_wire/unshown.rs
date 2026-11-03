//! The cards the daemon sent that this build could not read, and what the chat says in their
//! place. A card that asks the owner and cannot be drawn still waits for an answer, so one
//! placeholder stands for all of them: in the conversation for a proposal, among the approval
//! cards for an approval. Each is kept by the id it carries, where that reads, so its
//! resolution takes it away; every one goes at the next handshake, after which the daemon sends
//! again each card still waiting.

use std::collections::BTreeSet;

/// The placeholder's heading in the conversation.
pub const TITLE: &str = "Something needs your answer";
/// A proposal can be answered from a paired phone or the owner's other chats too (PROTOCOL.md
/// "Proposals").
pub const PROPOSAL: &str = "Fermix sent a request this app cannot show. Answer it from a paired \
    phone or another chat you have with Fermix.";
/// An approval's token is answered only on the connection whose turn asked for it (PROTOCOL.md
/// "Approvals"), so no other chat can answer it.
pub const APPROVAL: &str = "Fermix asked for your approval, but this app cannot show the \
    request. No other chat can answer it, so it expires unanswered.";
/// Every sentence here, for the strings test.
pub const STRINGS: [&str; 3] = [TITLE, PROPOSAL, APPROVAL];

/// The most unreadable cards of one kind kept by their ids. Past it a card of that kind is kept
/// without one, which no resolution takes, so its placeholder stands until the next handshake.
pub const MAX_KEPT: usize = 64;

/// What an unreadable card asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Asks {
    Proposal,
    Approval,
}

/// One unreadable card, by its id where that reads.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Unshown {
    pub asks: Asks,
    pub id: Option<String>,
}

/// Keeps an unreadable card, once per id, up to `MAX_KEPT` of its kind; past that, once without
/// an id. At most `MAX_KEPT + 1` of a kind are kept.
pub fn keep(kept: &mut BTreeSet<Unshown>, asks: Asks, id: Option<String>) {
    let of_kind = kept.iter().filter(|card| card.asks == asks).count();
    let id = id.filter(|_| of_kind < MAX_KEPT);
    kept.insert(Unshown { asks, id });
}

/// The daemon resolved the card `id`: its placeholder goes. One kept without an id stays.
pub fn resolved(kept: &mut BTreeSet<Unshown>, asks: Asks, id: &str) {
    kept.retain(|card| card.asks != asks || card.id.as_deref() != Some(id));
}

/// Whether an unreadable card of this kind still waits, so its placeholder shows.
pub fn waiting(kept: &BTreeSet<Unshown>, asks: Asks) -> bool {
    kept.iter().any(|card| card.asks == asks)
}
