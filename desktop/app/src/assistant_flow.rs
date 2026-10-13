//! The setup assistant's controller: which stage to show, Starting when Fermix
//! is not answering, the one About you write, the restart when one is needed,
//! and the finish gate (M38 §5.5).

use crate::app::App;
use crate::assistant::{Assistant, Mark};
use crate::dialogs::form_dialog;
use crate::service::{WAKE_INTERVAL, WAKE_TRIES};
use crate::state::Connection;
use adw::prelude::*;
use fermix_client::management::CallError;
use fermix_client::onboarding::{
    already_answered, finish_gate, landing, personalization_answer, starting_silent, Stage,
    STOPPED_ANSWERING,
};
use fermix_client::phone::{offers_phone, Intent};
use fermix_client::view::answers_with;
use gtk::glib;
use std::rc::Rc;
use std::time::Instant;

fn tag(stage: Stage) -> &'static str {
    match stage {
        Stage::Welcome => "welcome",
        Stage::Starting => "starting",
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

    /// Opens the assistant stopped on Starting with `sentence`, why the binding
    /// cannot be used. Nothing is started: Try again reads the binding again.
    pub fn open_recovery(self: &Rc<Self>, sentence: &str) {
        self.open_assistant();
        self.with_assistant(|a| a.starting.service.mark(Mark::Failed));
        self.starting_failed(sentence);
    }

    /// Moves to the first stage with work left, from what Fermix reports now.
    /// With nothing answering, Starting brings Fermix up first.
    pub async fn assistant_advance(self: Rc<Self>) {
        self.refresh().await;
        let stage = self
            .state
            .borrow()
            .snapshot()
            .map(|snapshot| landing(&snapshot.state));
        match stage {
            Some(stage) => self.assistant_land(stage).await,
            None => self.assistant_start().await,
        }
    }

    async fn assistant_land(self: Rc<Self>, stage: Stage) {
        match stage {
            Stage::Applying => self.assistant_apply(false).await,
            Stage::Ready => self.assistant_finish_gate().await,
            _ => self.with_assistant(|a| a.show(tag(stage))),
        }
    }

    /// Starting: the engine's own activation, then the wait for it to answer
    /// this window, each on its row, then on to where setup stands. A failure
    /// stays on the page with its sentence and Try again. A start already
    /// under way, such as a second press, is left to finish, and a daemon that
    /// answered but cannot serve this window stops it before anything runs, or
    /// names why the wait failed.
    async fn assistant_start(self: Rc<Self>) {
        if self.state.borrow().waking {
            return glib::g_debug!("fermix", "Fermix is already starting");
        }
        self.with_assistant(|a| {
            a.show("starting");
            a.starting.error.set_visible(false);
            a.starting.retry.set_visible(false);
            a.starting.service.mark(Mark::Working);
            a.starting.answer.row.set_visible(false);
        });
        if let Some(sentence) = self.answered_unusable() {
            self.with_assistant(|a| {
                a.starting.service.mark(Mark::Done);
                a.starting.answer.row.set_visible(true);
                a.starting.answer.mark(Mark::Failed);
            });
            return self.starting_failed(&sentence);
        }
        if let Err(sentence) = self.start_engine().await {
            self.with_assistant(|a| a.starting.service.mark(Mark::Failed));
            return self.starting_failed(&sentence);
        }
        self.with_assistant(|a| {
            a.starting.service.mark(Mark::Done);
            a.starting.answer.row.set_visible(true);
            a.starting.answer.mark(Mark::Working);
        });
        if !self.wait_for_daemon(None, "Fermix started").await {
            self.with_assistant(|a| a.starting.answer.mark(Mark::Failed));
            let sentence = self
                .answered_unusable()
                .unwrap_or_else(|| starting_silent(WAKE_INTERVAL * WAKE_TRIES));
            return self.starting_failed(&sentence);
        }
        self.with_assistant(|a| a.starting.answer.mark(Mark::Done));
        let stage = self.state.borrow().snapshot().map(|s| landing(&s.state));
        match stage {
            Some(stage) => self.assistant_land(stage).await,
            None => self.starting_failed(STOPPED_ANSWERING),
        }
    }

    /// What the daemon said when it answered but cannot serve this window.
    fn answered_unusable(&self) -> Option<String> {
        match &self.state.borrow().connection {
            Connection::Down(problem) => already_answered(problem),
            _ => None,
        }
    }

    fn starting_failed(&self, sentence: &str) {
        self.with_assistant(|a| {
            a.show("starting");
            a.starting.error.set_text(sentence);
            a.starting.error.set_visible(true);
            a.starting.retry.set_visible(true);
        });
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
        // The phone row is offered only where the daemon publishes the phone channel.
        let offers = self.ensure_sections().await
            && self
                .settings_data
                .borrow()
                .sections
                .as_deref()
                .is_some_and(offers_phone);
        let verdict = {
            let state = self.state.borrow();
            let Some(snapshot) = state.snapshot() else {
                drop(state);
                return self.applying_failed(STOPPED_ANSWERING);
            };
            finish_gate(snapshot.distribution.as_deref(), &snapshot.state)
                .map(|()| answers_with(&snapshot.state))
        };
        match verdict {
            Ok(line) => self.with_assistant(|a| {
                a.ready
                    .set_description(Some(&format!("Answers with {line}.")));
                a.phone.set_visible(offers);
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
        // Out of the cell first: closing emits `closed` at once, and its
        // handler borrows the cell again.
        let assistant = self.assistant.borrow_mut().take();
        if let Some(assistant) = assistant {
            assistant.dialog.close();
        }
        if target == "phone" {
            self.show_page("channels");
            return self.open_phone(Intent::Pair);
        }
        self.show_page(target);
        if target == "chat" {
            self.chat.focus_input();
        }
    }

    /// Shows a form: as the setup assistant's next page while it is open, so a
    /// popup never raises a second one, and otherwise in a dialog of its own.
    /// A second press while a form slides in finds one up already (the
    /// assistant's own stages are tagged, forms are not) and opens nothing.
    pub fn present_form(&self, page: &adw::NavigationPage, width: i32, height: Option<i32>) {
        let nav = self.assistant.borrow().as_ref().map(|a| a.nav.clone());
        let Some(nav) = nav else {
            return form_dialog(page, width, height).present(Some(&self.shell.window));
        };
        if nav.visible_page().is_some_and(|p| p.tag().is_none()) {
            glib::g_debug!(
                "fermix",
                "a form is up in the assistant; {} waits",
                page.title()
            );
            return;
        }
        nav.push(page);
    }

    /// Whether the setup assistant is on screen.
    pub fn assisting(&self) -> bool {
        self.assistant.borrow().is_some()
    }

    fn with_assistant(&self, f: impl FnOnce(&Assistant)) {
        match self.assistant.borrow().as_ref() {
            Some(assistant) => f(assistant),
            None => glib::g_debug!("fermix", "the assistant closed before its step finished"),
        }
    }
}
