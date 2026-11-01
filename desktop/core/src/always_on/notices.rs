//! The desktop notifications of Always On (Always On setup §3.1): the engine names each
//! occurrence it raises (`summary.notice`: the server ready, the move done, the server out of
//! reach, a server to set up again, the newest backup a week old) by an id that stays while it
//! stands. The app raises a notice whose id it has not raised yet, and keeps the last sixteen ids
//! it raised: an id can come back after an outage clears, and the set stays bounded.

use crate::home::Notice;
use std::collections::VecDeque;

/// How many raised ids are kept.
pub const POSTED: usize = 16;

/// The ids of the notices raised, oldest first.
#[derive(Debug, Default)]
pub struct Posted {
    ids: VecDeque<String>,
}

impl Posted {
    /// Whether `notice` is one to raise now; it is kept as raised.
    pub fn fresh(&mut self, notice: &Notice) -> bool {
        assert!(!notice.id.is_empty(), "a notice is named by its id");
        if self.ids.contains(&notice.id) {
            return false;
        }
        if self.ids.len() == POSTED {
            self.ids.pop_front();
        }
        self.ids.push_back(notice.id.clone());
        true
    }
}
