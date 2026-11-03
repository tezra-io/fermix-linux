//! The window's side of Cues (M60 cues §4.1, §4.9, §5.4): the gate read on every refresh, the
//! route away from a pane the daemon stopped serving, the count the tray shows and where its line
//! and Show in chat open Chat. The pane itself is `cues.rs`.

use crate::app::App;
use fermix_client::cues::view::{publishes_section, PANE, STATUS_METHOD};
use gtk::glib;
use std::rc::Rc;

impl App {
    /// Show in chat opens the window's Chat at the card. Called once, as the window starts.
    pub fn wire_cues(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.cues.set_reveal(Rc::new(move |proposal_id| {
            if let Some(app) = weak.upgrade() {
                app.chat_reveal(proposal_id);
            }
        }));
    }

    /// The proposals waiting, for the tray: the last `cues.status` count while the daemon runs
    /// and the gate holds, else none.
    pub fn cue_proposals_waiting(&self) -> u64 {
        let state = self.state.borrow();
        let Some(snapshot) = state.snapshot() else {
            return 0;
        };
        let data = self.settings_data.borrow();
        let open = self
            .cues
            .available(&snapshot.capabilities, data.sections.as_deref());
        if !open {
            return 0;
        }
        self.cues.proposals_waiting()
    }

    /// The tray's line: Chat at the oldest proposal waiting, or Chat alone when none is listed
    /// by the time it is asked.
    pub fn show_waiting(self: &Rc<Self>) {
        let app = self.clone();
        glib::spawn_future_local(async move {
            match app.cues.oldest_waiting().await {
                Some(proposal_id) => app.chat_reveal(&proposal_id),
                None => app.show_page("chat"),
            }
        });
    }

    /// Reads `cues.status` while the daemon lists it and publishes the `cues` section: the last
    /// half of the gate, and the counts the page and the tray follow. An older daemon is asked
    /// nothing.
    pub async fn read_cues(&self) {
        let serves = self
            .state
            .borrow()
            .snapshot()
            .is_some_and(|s| s.has_method(STATUS_METHOD));
        if !serves || !self.ensure_sections().await {
            return;
        }
        let published = publishes_section(self.settings_data.borrow().sections.as_deref());
        if !published {
            return;
        }
        self.cues.read_status().await;
        self.render();
    }

    /// The Cues pane on screen goes to Providers once the gate has said no (a downgrade, or a
    /// method the daemon turned out not to have). A pane only being read again stays.
    pub fn leave_shut_cues(&self) {
        if self.shell.visible_pane().as_deref() != Some(PANE) {
            return;
        }
        let shut = {
            let state = self.state.borrow();
            let Some(snapshot) = state.snapshot() else {
                return;
            };
            let data = self.settings_data.borrow();
            self.cues
                .shut(&snapshot.capabilities, data.sections.as_deref())
        };
        if shut {
            self.shell.show_page("providers");
        }
    }
}
