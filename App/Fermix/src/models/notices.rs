//! The one notification this application raises.
//!
//! While Fermix is open, a change in how many things need someone is worth
//! saying once. Nothing is raised for the state the window opened on, because a
//! notification for something that was already there is a notification about
//! nothing; nothing is raised after the application exits, because the window
//! withdraws what it raised on its way out; and nothing is inferred about
//! whether this is a good moment to interrupt. Do Not Disturb is the desktop's
//! to decide and this application never asks.

use std::cell::Cell;

use crate::copy::Key;

/// What one observation asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    /// Raise it: more things need someone than did a moment ago.
    Post,
    /// Take it back: nothing needs anyone now.
    Withdraw,
    /// Say nothing.
    Nothing,
}

/// The count, as this process last saw it.
#[derive(Default)]
pub struct AttentionNotice {
    seen: Cell<Option<usize>>,
}

impl AttentionNotice {
    /// The identifier this notification is raised and withdrawn under. One
    /// identifier, so a second raise replaces the first rather than stacking.
    pub const ID: &'static str = "attention";

    /// The title.
    pub const TITLE: Key = Key::NoticeAttentionTitle;
    /// The body.
    pub const BODY: Key = Key::NoticeAttentionBody;

    /// A notice that has seen nothing yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// One observation of how many things need someone.
    ///
    /// The first is recorded and nothing is said about it: the window has just
    /// opened and whatever it found was already true.
    pub fn observe(&self, count: usize) -> Notice {
        let previous = self.seen.replace(Some(count));

        match previous {
            None => Notice::Nothing,
            Some(previous) if previous == count => Notice::Nothing,
            Some(_) if count == 0 => Notice::Withdraw,
            Some(_) => Notice::Post,
        }
    }

    /// What this process last saw.
    pub fn seen(&self) -> Option<usize> {
        self.seen.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_the_window_opened_on_raises_nothing() {
        let notice = AttentionNotice::new();
        assert_eq!(notice.observe(3), Notice::Nothing);
        assert_eq!(notice.seen(), Some(3));
    }

    #[test]
    fn a_count_that_grew_while_the_window_was_open_is_said_once() {
        let notice = AttentionNotice::new();
        notice.observe(0);

        assert_eq!(notice.observe(1), Notice::Post);
        assert_eq!(notice.observe(1), Notice::Nothing);
        assert_eq!(notice.observe(2), Notice::Post);
    }

    #[test]
    fn a_count_that_fell_to_nothing_takes_the_notification_back() {
        let notice = AttentionNotice::new();
        notice.observe(2);

        assert_eq!(notice.observe(0), Notice::Withdraw);
        assert_eq!(notice.observe(0), Notice::Nothing);
    }
}
