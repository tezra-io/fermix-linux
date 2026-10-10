//! The Phone row's two reads and the Phone dialog's pairing window (M60 §4). Every step the
//! dialog shows is the core reducer's answer to what the daemon said; this owns only when each
//! call is made. It reads the window the dialog shows once a second, the contract's cadence, and
//! cancels it when the dialog closes in Scan or Compare, so no window is left waiting for a scan.

use crate::app::App;
use crate::phone::{PhoneView, Screen};
use adw::prelude::*;
use fermix_client::management::CallError;
use fermix_client::mobile::PairingStart;
use fermix_client::pairing::{reduce, Answer, EndAction, Progress, Step, TurnOn};
use fermix_client::phone::{
    Forgetting, Intent, CHANNEL, ENDED_UNREADABLE, POLL_MS, SECTION, SWITCH_KEY,
};
use fermix_client::settings::read_failure;
use fermix_client::view::channel_title;
use gtk::glib;
use serde_json::{Map, Value};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The most reads one window takes: its two minutes at one a second, and a margin.
const MAX_POLLS: u32 = 150;

/// The Phone dialog's state beside `State`.
pub struct Phone {
    pub step: Step,
    /// A decision on its way to the daemon.
    pub deciding: bool,
    pub forgetting: Forgetting,
    /// Counts the dialog's pieces of work. Starting one, or closing the dialog, moves it on, and
    /// work begun under an earlier count stops at its next answer.
    work: u64,
    /// The dialog while it is up.
    pub view: Option<Rc<PhoneView>>,
}

impl Default for Phone {
    fn default() -> Phone {
        Phone {
            step: Step::Waiting { session: None },
            deciding: false,
            forgetting: Forgetting::default(),
            work: 0,
            view: None,
        }
    }
}

/// The row button's action target.
pub fn intent(target: &str) -> Option<Intent> {
    match target {
        "pair" => Some(Intent::Pair),
        "phones" => Some(Intent::Phones),
        _ => None,
    }
}

pub fn target(intent: Intent) -> &'static str {
    match intent {
        Intent::Pair => "pair",
        Intent::Phones => "phones",
    }
}

fn turning(throws_switch: bool, progress: Progress, refusal: Option<String>) -> Step {
    Step::TurnOn(TurnOn {
        throws_switch,
        progress,
        refusal,
    })
}

fn unix_now() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
}

impl App {
    /// Puts the dialog up for what the row's button, or the last setup screen, asked for.
    pub fn open_phone(self: &Rc<Self>, intent: Intent) {
        let open = self.phone.borrow().view.clone();
        if let Some(view) = open {
            view.dialog.present(Some(&self.shell.window));
            return;
        }
        let dialog_views = self.settings.dialog_views.clone();
        let view = Rc::new(PhoneView::new(&self.phone_title(), dialog_views));
        let weak = Rc::downgrade(self);
        view.dialog.connect_closed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.phone_closed();
            }
        });
        self.phone.borrow_mut().view = Some(view.clone());
        self.render();
        view.dialog.present(Some(&self.shell.window));
        let app = self.clone();
        glib::spawn_future_local(async move {
            let token = app.phone_begin();
            match intent {
                Intent::Pair => app.phone_pair(token).await,
                Intent::Phones => app.phone_phones(token).await,
            }
        });
    }

    /// The daemon's own title for the channel's section.
    fn phone_title(&self) -> String {
        let data = self.settings_data.borrow();
        let section = data.sections.iter().flatten().find(|s| s.id == SECTION);
        section.map_or_else(|| channel_title(CHANNEL).to_owned(), |s| s.title.clone())
    }

    /// Draws the dialog from what is known now; called by every render.
    pub fn render_phone(&self) {
        let phone = self.phone.borrow();
        let Some(view) = phone.view.as_ref() else {
            return;
        };
        let data = self.settings_data.borrow();
        let switch = data
            .rows
            .get(SECTION)
            .and_then(|section| section.rows.iter().find(|r| r.key == SWITCH_KEY));
        let screen = Screen {
            step: &phone.step,
            deciding: phone.deciding,
            forgetting: &phone.forgetting,
            devices: data.phone_devices.as_ref(),
            footer: switch.and_then(|row| row.footer.as_deref()),
            now: unix_now(),
        };
        view.show(&screen);
    }

    /// Cancel and Done. What closing cancels is `phone_closed`'s, since the dialog also goes by
    /// Escape and by its close button.
    pub fn phone_dismiss(&self) {
        let view = self.phone.borrow().view.clone();
        if let Some(view) = view {
            view.dialog.close();
        }
    }

    /// The dialog left the screen, however it left: the window it showed is cancelled, nothing it
    /// followed is followed any more, and its state is put down.
    fn phone_closed(self: &Rc<Self>) {
        let (view, open) = {
            let mut phone = self.phone.borrow_mut();
            phone.work += 1;
            phone.deciding = false;
            phone.forgetting = Forgetting::default();
            let open = phone.step.open_session().map(str::to_owned);
            phone.step = Step::Waiting { session: None };
            (phone.view.take(), open)
        };
        if let Some(view) = view {
            view.close_down();
        }
        let Some(open) = open else {
            return;
        };
        let app = self.clone();
        glib::spawn_future_local(async move { app.phone_cancel(open).await });
    }

    fn phone_begin(&self) -> u64 {
        let mut phone = self.phone.borrow_mut();
        phone.work += 1;
        phone.work
    }

    fn phone_current(&self, token: u64) -> bool {
        let phone = self.phone.borrow();
        phone.view.is_some() && phone.work == token
    }

    fn phone_show(&self, token: u64, step: Step) {
        if !self.phone_current(token) {
            return;
        }
        self.phone.borrow_mut().step = step;
        self.render();
    }

    /// The window the dialog shows, while `token`'s work is the dialog's.
    fn phone_open_session(&self, token: u64) -> Option<String> {
        if !self.phone_current(token) {
            return None;
        }
        self.phone.borrow().step.open_session().map(str::to_owned)
    }

    /// One answer, through the reducer. An answer that reaches a dialog that has moved on is
    /// dropped, and a window the answer leaves open that nothing will show is cancelled.
    async fn phone_take(&self, token: u64, answer: Answer) {
        if !self.phone_current(token) {
            return;
        }
        let transition = reduce(&self.phone.borrow().step, answer);
        self.phone.borrow_mut().step = transition.step;
        self.render();
        if let Some(abandoned) = transition.abandons {
            self.phone_cancel(abandoned).await;
        }
    }

    /// The daemon's sentence for a failed call, logged with what failed. Nothing a call carried
    /// is logged, so neither is the pairing link.
    fn phone_refusal(&self, e: &CallError, method: &str) -> String {
        glib::g_warning!("fermix", "{method} failed: {e:?}");
        read_failure(e)
    }

    /// Pairing begins on the channel as it stands: Turn on while it is not running, and the
    /// window once it is.
    async fn phone_pair(&self, token: u64) {
        self.phone_show(token, Step::Waiting { session: None });
        let status = self.daemon.call(|m| m.mobile_status()).await;
        let status = status.map_err(|e| self.phone_refusal(&e, "mobile.status"));
        self.settings_data.borrow_mut().phone_status = Some(status.clone());
        let answer = match status {
            Ok(status) => Answer::Status(status),
            Err(sentence) => return self.phone_take(token, Answer::Refused(sentence)).await,
        };
        self.phone_take(token, answer).await;
        if self.phone.borrow().step != (Step::Waiting { session: None }) {
            return;
        }
        self.phone_open_window(token).await;
    }

    /// Opens a window and follows it. A window open somewhere else is read from
    /// `mobile.status`, which names it.
    async fn phone_open_window(&self, token: u64) {
        if !self.phone_current(token) {
            return;
        }
        self.phone_show(token, Step::Waiting { session: None });
        let answer = match self.daemon.call(|m| m.mobile_pair_start()).await {
            Ok(started) if !self.phone_current(token) => return self.phone_abandon(&started).await,
            Ok(started) => Answer::Started(started),
            Err(CallError::Refused(r)) if r.code == "busy" => self.phone_elsewhere().await,
            Err(e) => Answer::Refused(self.phone_refusal(&e, "mobile.pair.start")),
        };
        self.phone_take(token, answer).await;
        self.phone_follow(token).await;
    }

    /// A start that answered after the dialog closed opened a window nobody will see, so it is
    /// cancelled at once.
    async fn phone_abandon(&self, started: &PairingStart) {
        let session = &started.session;
        if session.state.is_terminal() {
            return;
        }
        if let Some(id) = session.session_id.clone() {
            self.phone_cancel(id).await;
        }
    }

    async fn phone_elsewhere(&self) -> Answer {
        match self.daemon.call(|m| m.mobile_status()).await {
            Ok(status) => Answer::Busy(status.pairing),
            Err(e) => Answer::Refused(self.phone_refusal(&e, "mobile.status")),
        }
    }

    /// Reads the window the dialog shows once a second until it ends. A resumed window is read
    /// at once, since nothing of it is on screen yet. Past `MAX_POLLS` a window still open is
    /// one this app cannot follow: it ends with the app's sentence and is cancelled.
    async fn phone_follow(&self, token: u64) {
        let resumed = match &self.phone.borrow().step {
            Step::Waiting { session } => session.clone(),
            _ => None,
        };
        if let Some(session) = resumed {
            self.phone_read(token, session).await;
        }
        for _ in 0..MAX_POLLS {
            let Some(session) = self.phone_open_session(token) else {
                return;
            };
            glib::timeout_future(Duration::from_millis(POLL_MS)).await;
            if self.phone_open_session(token).as_ref() != Some(&session) {
                return;
            }
            self.phone_read(token, session).await;
        }
        let Some(session) = self.phone_open_session(token) else {
            return;
        };
        glib::g_warning!(
            "fermix",
            "a pairing window was still open after {MAX_POLLS} reads"
        );
        self.phone_take(token, Answer::Refused(ENDED_UNREADABLE.into()))
            .await;
        self.phone_cancel(session).await;
    }

    async fn phone_read(&self, token: u64, session: String) {
        let answer = match self.daemon.call(move |m| m.mobile_pair_get(&session)).await {
            Ok(view) => Answer::Session(view),
            Err(e) => Answer::Refused(self.phone_refusal(&e, "mobile.pair.get")),
        };
        self.phone_take(token, answer).await;
    }

    /// Cancels a window. A refusal is logged and nothing more: the window closes on the daemon's
    /// own clock within two minutes regardless.
    async fn phone_cancel(&self, session: String) {
        let answer = self
            .daemon
            .call(move |m| m.mobile_pair_cancel(&session))
            .await;
        if let Err(e) = answer {
            glib::g_warning!("fermix", "mobile.pair.cancel failed: {e:?}");
        }
    }

    /// Turn on's one button: the switch where it is off, then the app's restart, each shown as
    /// it runs. A refusal stays on the step in its own words. After the restart the window is
    /// asked for whatever the channel did, so a channel that still could not start says why in
    /// the daemon's sentence.
    pub async fn phone_turn_on(self: Rc<Self>) {
        let turn_on = match &self.phone.borrow().step {
            Step::TurnOn(turn_on) if turn_on.progress == Progress::Idle => turn_on.clone(),
            _ => return,
        };
        let token = self.phone_begin();
        if turn_on.throws_switch {
            self.phone_show(token, turning(true, Progress::Applying, None));
            if let Err(sentence) = self.phone_switch_on().await {
                return self.phone_show(token, turning(true, Progress::Idle, Some(sentence)));
            }
        }
        self.phone_show(token, turning(false, Progress::Restarting, None));
        if let Err(sentence) = self.restart_now().await {
            return self.phone_show(token, turning(false, Progress::Idle, Some(sentence)));
        }
        if !self.phone_current(token) {
            return;
        }
        self.read_phone_row().await;
        self.phone_open_window(token).await;
    }

    async fn phone_switch_on(&self) -> Result<(), String> {
        let mut values = Map::new();
        values.insert(SWITCH_KEY.to_owned(), Value::Bool(true));
        let answer = self
            .daemon
            .call(move |m| m.settings_apply(SECTION, values))
            .await;
        if let Err(e) = answer {
            return Err(self.phone_refusal(&e, "settings.apply"));
        }
        self.read_section(SECTION).await;
        Ok(())
    }

    pub async fn phone_decide(self: Rc<Self>, approved: bool) {
        let session = match &self.phone.borrow().step {
            Step::Compare(compare) => compare.session.clone(),
            _ => return,
        };
        if self.phone.borrow().deciding {
            return;
        }
        let token = self.phone_begin();
        self.phone.borrow_mut().deciding = true;
        self.render();
        let answer = self
            .daemon
            .call(move |m| m.mobile_pair_decide(&session, approved))
            .await;
        let answer = match answer {
            Ok(view) => Answer::Session(view),
            Err(e) => Answer::Refused(self.phone_refusal(&e, "mobile.pair.decide")),
        };
        if self.phone_current(token) {
            self.phone.borrow_mut().deciding = false;
        }
        self.phone_take(token, answer).await;
        self.phone_follow(token).await;
    }

    /// Ended's one way on: Pair again, or Start over, which cancels the window open somewhere
    /// else before opening a new one.
    pub async fn phone_ending(self: Rc<Self>) {
        let action = match &self.phone.borrow().step {
            Step::Ended(ending) => ending.action.clone(),
            _ => return,
        };
        let token = self.phone_begin();
        match action {
            EndAction::PairAgain => self.phone_pair(token).await,
            EndAction::StartOver { session } => {
                if let Some(session) = session {
                    self.phone_cancel(session).await;
                }
                self.phone_open_window(token).await;
            }
        }
    }

    /// The phones, with the phone just paired among them: Done on Paired, and what the row's
    /// Change… opens.
    pub async fn phone_phones(&self, token: u64) {
        self.phone_show(token, Step::Phones);
        self.read_section(SECTION).await;
        self.read_phone_row().await;
    }

    pub async fn phone_pair_another(self: Rc<Self>) {
        let token = self.phone_begin();
        self.phone_pair(token).await;
    }

    pub async fn phone_show_phones(self: Rc<Self>) {
        let token = self.phone_begin();
        self.phone_phones(token).await;
    }

    pub fn phone_forget_ask(&self, device: &str) {
        self.phone.borrow_mut().forgetting.ask(device);
        self.render();
    }

    pub fn phone_forget_withdraw(&self) {
        self.phone.borrow_mut().forgetting.withdraw();
        self.render();
    }

    /// The second press forgets the phone the row asked about. The list is read again before
    /// the row stops saying so, so a forgotten phone leaves the list rather than offering Forget
    /// once more; a refusal stays under its row in the daemon's words.
    pub async fn phone_forget(self: Rc<Self>) {
        let confirmed = self.phone.borrow_mut().forgetting.confirm();
        let Some(device) = confirmed else {
            return;
        };
        self.render();
        let id = device.clone();
        let answer = self
            .daemon
            .call(move |m| m.mobile_devices_revoke(&id))
            .await;
        let refusal = match answer {
            Ok(_) => None,
            Err(e) => Some(self.phone_refusal(&e, "mobile.devices.revoke")),
        };
        if refusal.is_none() {
            self.read_phone_row().await;
        }
        self.phone
            .borrow_mut()
            .forgetting
            .finished(&device, refusal);
        self.render();
    }

    /// Reads the channel and its phones, which is everything the Phone row states.
    pub async fn read_phone_row(&self) {
        let status = self.daemon.call(|m| m.mobile_status()).await;
        let status = status.map_err(|e| self.phone_refusal(&e, "mobile.status"));
        let devices = self.daemon.call(|m| m.mobile_devices_list()).await;
        let devices = devices.map_err(|e| self.phone_refusal(&e, "mobile.devices.list"));
        {
            let mut data = self.settings_data.borrow_mut();
            data.phone_status = Some(status);
            data.phone_devices = Some(devices);
        }
        self.render();
    }
}
