//! The setup assistant's controller: which stage to show, the one About you
//! write, the restart when one is needed, and the finish gate (M38 §5.5).

use crate::app::App;
use crate::assistant::{Assistant, Mark};
use adw::prelude::*;
use fermix_client::management::CallError;
use fermix_client::onboarding::{finish_gate, landing, personalization_answer, Stage};
use fermix_client::view::answers_with;
use gtk::glib;
use std::rc::Rc;
use std::time::Instant;

fn tag(stage: Stage) -> &'static str {
    match stage {
        Stage::Welcome | Stage::Starting => "welcome",
        Stage::Connect => "connect",
        Stage::AboutYou => "about",
        Stage::Applying => "applying",
        Stage::Ready => "ready",
    }
}

impl App {
    /// Opens the assistant, or brings it forward if it is open already.
    pub fn open_assistant(self: &Rc<Self>) {
        if let Some(open) = self.assistant.borrow().as_ref() {
            open.dialog.present(Some(&self.shell.window));
            return;
        }
        let assistant = Rc::new(Assistant::new());
        let weak = Rc::downgrade(self);
        assistant.dialog.connect_closed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.assistant.borrow_mut().take();
            }
        });
        assistant.dialog.present(Some(&self.shell.window));
        *self.assistant.borrow_mut() = Some(assistant);
        self.render();
    }

    /// Draws the assistant's live parts from `State`: the provider rows and
    /// whether Connect your AI may be left.
    pub fn render_assistant(&self) {
        let assistant = self.assistant.borrow();
        let Some(assistant) = assistant.as_ref() else {
            return;
        };
        let state = self.state.borrow();
        assistant.providers.render(&state, Instant::now());
        let connected = state
            .snapshot()
            .is_some_and(|s| fermix_client::onboarding::connect_done(&s.state));
        assistant.connect_next.set_sensitive(connected);
    }

    /// Moves to the first stage with work left, from what Fermix reports now.
    pub async fn assistant_advance(self: Rc<Self>) {
        self.refresh().await;
        let stage = match self.state.borrow().snapshot() {
            Some(snapshot) => landing(&snapshot.state),
            None => {
                return self
                    .shell
                    .toast("Fermix is not answering. Start it from Home.")
            }
        };
        if stage == Stage::Applying {
            return self.assistant_apply(false).await;
        }
        if stage == Stage::Ready {
            return self.assistant_finish_gate().await;
        }
        self.with_assistant(|a| a.show(tag(stage)));
    }

    /// Applying: the About you write when `write`, then a restart if Fermix
    /// asks for one, then the finish gate. A refused save goes back to About you.
    pub async fn assistant_apply(self: Rc<Self>, write: bool) {
        self.with_assistant(|a| {
            a.show("applying");
            a.applying.error.set_visible(false);
            a.applying.retry.set_visible(false);
            a.applying.save.row.set_visible(write);
            a.applying.save.mark(Mark::Working);
            a.applying.restart.row.set_visible(false);
        });
        if write && !self.save_about_you().await {
            return;
        }
        self.with_assistant(|a| a.applying.save.mark(Mark::Done));
        let restart = self
            .state
            .borrow()
            .snapshot()
            .is_some_and(|s| s.state.restart.required);
        if restart {
            self.with_assistant(|a| {
                a.applying.restart.row.set_visible(true);
                a.applying.restart.mark(Mark::Working);
            });
            if let Err(sentence) = self.restart_now().await {
                self.with_assistant(|a| a.applying.restart.mark(Mark::Failed));
                return self.applying_failed(&sentence);
            }
            self.with_assistant(|a| a.applying.restart.mark(Mark::Done));
        }
        self.assistant_finish_gate().await;
    }

    /// The one write About you makes: all four keys, then a fresh read.
    async fn save_about_you(&self) -> bool {
        let Some(values) = self.about_you_answer() else {
            return false;
        };
        let answer = self
            .daemon
            .call(move |m| m.settings_apply("personalization", values))
            .await;
        if let Err(e) = answer {
            glib::g_warning!("fermix", "About you was not saved: {e:?}");
            let sentence = match e {
                CallError::Refused(r) => r.sentence,
                _ => "Fermix did not answer, so nothing was saved.".into(),
            };
            self.with_assistant(|a| {
                a.show("about");
                a.about.error.set_text(&sentence);
                a.about.error.set_visible(true);
            });
            return false;
        }
        self.with_assistant(|a| a.about.error.set_visible(false));
        self.settings_data
            .borrow_mut()
            .rows
            .remove("personalization");
        self.refresh().await;
        true
    }

    fn about_you_answer(&self) -> Option<serde_json::Map<String, serde_json::Value>> {
        let assistant = self.assistant.borrow();
        let about = &assistant.as_ref()?.about;
        let style = usize::try_from(about.style.selected()).ok()?;
        let account = glib::user_name().to_string_lossy().into_owned();
        Some(personalization_answer(
            &about.name.text(),
            &about.zone.text(),
            style,
            &about.assistant.text(),
            &account,
        ))
    }

    async fn assistant_finish_gate(&self) {
        self.refresh().await;
        let verdict = {
            let state = self.state.borrow();
            let Some(snapshot) = state.snapshot() else {
                drop(state);
                return self.applying_failed("Fermix stopped answering.");
            };
            finish_gate(snapshot.distribution.as_deref(), &snapshot.state)
                .map(|()| answers_with(&snapshot.state))
        };
        match verdict {
            Ok(line) => self.with_assistant(|a| {
                a.ready
                    .set_description(Some(&format!("Answers with {line}.")));
                a.show("ready");
            }),
            Err(sentence) => self.applying_failed(sentence),
        }
    }

    fn applying_failed(&self, sentence: &str) {
        self.with_assistant(|a| {
            a.show("applying");
            a.applying.error.set_text(sentence);
            a.applying.error.set_visible(true);
            a.applying.retry.set_visible(true);
        });
    }

    /// Closes the assistant and opens where the person chose to go next.
    pub fn assistant_finish(self: &Rc<Self>, target: &str) {
        if let Some(assistant) = self.assistant.borrow_mut().take() {
            assistant.dialog.close();
        }
        self.show_page(target);
        if target == "chat" {
            self.chat.focus_input();
        }
    }

    fn with_assistant(&self, f: impl FnOnce(&Assistant)) {
        match self.assistant.borrow().as_ref() {
            Some(assistant) => f(assistant),
            None => glib::g_debug!("fermix", "the assistant closed before its step finished"),
        }
    }
}
