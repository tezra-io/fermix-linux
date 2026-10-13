//! The Voice page's controller. Before a call: which one thing stands in the
//! way (spec_voice §4.3), and the existing flows that fix it: start Fermix, turn
//! voice on, store the key, restart. Voice settings are boot-bound, so turning
//! voice on never restarts Fermix by itself (spec R9).

use crate::app::App;
use crate::microphones::{MicrophoneWatch, SOUND_SERVER};
use crate::state::Snapshot;
use crate::voice::VoiceView;
use adw::prelude::*;
use fermix_client::realtime::session::{Input, Mode};
use fermix_client::voice::{
    call_status, expression, lost_microphone_sentence, main_window_close, voice_gate, GateAction,
    MainWindowClose, Microphone, Reach, Source, VoiceFacts,
};
use gtk::{gio, glib};
use serde_json::Value;
use std::rc::Rc;
use std::time::Duration;

const SECTION: &str = "realtime";
const ENABLED: &str = "realtime_enabled";
const KEY: &str = "openai_api_key";
/// How long a call's microphone may be gone from the list before the call ends:
/// a headset changing profile drops its microphone for a moment and brings it back.
const MICROPHONE_GRACE: Duration = Duration::from_secs(2);
/// How often the Voice page reads the list again while in view, so the row
/// follows a new default input (see `MicrophoneWatch::refresh`).
const MICROPHONE_POLL: Duration = Duration::from_secs(2);

impl App {
    /// The Voice page and the companion, from the daemon's facts and the call.
    pub fn voice_view(&self) -> VoiceView {
        let state = self.state.borrow();
        let snapshot = state.snapshot();
        let call = self.call.borrow();
        let session = &call.session;
        let microphone = self.microphone.borrow();
        let gate = voice_gate(&facts(snapshot, call.reach, &microphone));
        // What the last attempt found never stops another try.
        let can_begin = voice_gate(&facts(snapshot, Reach::Untried, &microphone)).ready;
        let in_call = session.in_call();
        let status = call_status(session, &gate, &microphone);
        VoiceView {
            gate,
            can_begin,
            status,
            expression: expression(session.visual_mode()),
            level: session.level(),
            mic_level: session.mic_level(),
            in_call,
            muted: session.muted(),
            can_stop: session.can_stop(),
            can_cancel_task: session.can_cancel_task(),
            busy: !in_call && session.mode() == Mode::Connecting,
            transcript: session.transcript().to_vec(),
            task: session.task_line(),
            usage: session.usage_line(),
            microphone: microphone.label().to_owned(),
        }
    }

    /// Redraws only what the call changes, as often as the call changes.
    pub fn render_voice(&self) {
        let view = self.voice_view();
        let mute = self
            .application
            .lookup_action("voice-mute")
            .and_downcast::<gio::SimpleAction>()
            .expect("the call's actions are installed before anything draws");
        // The button and the menu item follow this state, which follows the call.
        mute.set_state(&view.muted.to_variant());
        self.voice.render(&view);
        self.companion.render(&view);
    }

    /// Starts following the sound server's inputs, the first time a voice surface
    /// shows. A watch that cannot start leaves the microphone unknown, and the
    /// next showing tries again; the call's own attempt still says what it finds.
    pub fn watch_microphones(self: &Rc<Self>) {
        if self.microphone_watch.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let on_change = move |sources: Vec<Source>| {
            if let Some(app) = weak.upgrade() {
                app.microphone_changed(&sources);
            }
        };
        match MicrophoneWatch::start(SOUND_SERVER, on_change) {
            Ok(watch) => {
                let sources = watch.sources();
                self.microphone_watch.replace(Some(watch));
                self.microphone_changed(&sources);
            }
            Err(e) => glib::g_warning!("fermix", "the microphone list cannot be read: {e}"),
        }
    }

    /// Follows the microphones and reads them again every 2 s until the page is
    /// hidden. A second call while shown changes nothing.
    pub fn voice_page_shown(self: &Rc<Self>) {
        if self.microphone_poll.borrow().is_some() {
            return;
        }
        self.watch_microphones();
        let weak = Rc::downgrade(self);
        let poll = glib::timeout_add_local(MICROPHONE_POLL, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.refresh_microphones();
            glib::ControlFlow::Continue
        });
        self.microphone_poll.replace(Some(poll));
    }

    /// Stops the poll; the watch goes on. Calling it while hidden changes nothing.
    pub fn voice_page_hidden(&self) {
        if let Some(poll) = self.microphone_poll.take() {
            poll.remove();
        }
    }

    /// Asks the sound server again. Without a watch there is nothing to ask; the
    /// next showing tries to start one.
    fn refresh_microphones(self: &Rc<Self>) {
        let answer = self
            .microphone_watch
            .borrow()
            .as_ref()
            .map(MicrophoneWatch::refresh);
        let Some(answer) = answer else {
            return;
        };
        match answer {
            Ok(sources) => self.microphone_changed(&sources),
            Err(e) => {
                // Once per spell, not on every poll.
                if *self.microphone.borrow() != Microphone::Unknown {
                    glib::g_warning!("fermix", "the microphone list cannot be read: {e}");
                }
                self.set_microphone(Microphone::Unknown);
            }
        }
    }

    fn microphone_changed(self: &Rc<Self>, sources: &[Source]) {
        let recorded = self.microphone.borrow().clone();
        self.set_microphone(Microphone::from_sources(sources));
        if recorded.lost_in(sources) && self.call.borrow().session.in_call() {
            self.end_call_unless_microphone_returns(recorded);
        }
    }

    fn set_microphone(self: &Rc<Self>, now: Microphone) {
        if *self.microphone.borrow() == now {
            return;
        }
        glib::g_info!("fermix", "voice records from {now:?}");
        self.microphone.replace(now);
        self.render_voice();
    }

    /// The sound server moves a call's recording to whatever is left: another microphone
    /// nobody chose, or a copy of the speakers, so the call would go on hearing the room or
    /// itself. Unless the call's own microphone is back after the grace period, the call
    /// ends and says why. A later call is not this one's to end.
    fn end_call_unless_microphone_returns(self: &Rc<Self>, recorded: Microphone) {
        let call = self.call.borrow().session.call_number();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(MICROPHONE_GRACE, move || {
            let Some(app) = weak.upgrade() else { return };
            if app.call.borrow().session.call_number() != call {
                return;
            }
            // The list, not the row: the provider's default flag lags the server's, so the
            // row can still name a stand-in after the microphone is back.
            let sources = app
                .microphone_watch
                .borrow()
                .as_ref()
                .map(MicrophoneWatch::sources);
            let Some(sources) = sources else { return };
            if let Some(sentence) = lost_microphone_sentence(&recorded, &sources) {
                app.voice_input(Input::AudioFailed(sentence.to_owned()));
            }
        });
    }

    /// The button beside what stands in the way.
    pub async fn voice_fix(self: Rc<Self>) {
        let Some(action) = self.voice_view().gate.action else {
            return;
        };
        // After a fix, the last attempt's answer no longer says anything.
        self.call.borrow_mut().reach = Reach::Untried;
        match action {
            GateAction::StartFermix => self.start_service().await,
            GateAction::TurnOn => {
                let (section, key) = (SECTION.to_owned(), ENABLED.to_owned());
                self.apply_setting(section, key, Value::Bool(true)).await
            }
            GateAction::AddKey => self.add_secret(SECTION.into(), KEY.into()),
            GateAction::Restart => self.restart().await,
        }
    }
}

fn facts<'a>(
    snapshot: Option<&'a Snapshot>,
    reach: Reach,
    microphone: &'a Microphone,
) -> VoiceFacts<'a> {
    VoiceFacts {
        state: snapshot.map(|s| &s.state),
        realtime: snapshot
            .and_then(|s| s.overview.as_ref())
            .and_then(|o| o.realtime.as_ref()),
        reach,
        microphone,
    }
}

/// The call's actions, on the application so the companion reaches them too.
pub fn install_call_actions(app: &Rc<App>) {
    app_action(app, "voice-call", toggle_call);
    app_action(app, "voice-stop", |app| {
        let played_ms = app.call.borrow().played_ms();
        app.voice_input(Input::Stop { played_ms });
    });
    app_action(app, "voice-cancel-task", |app| {
        app.voice_input(Input::CancelTask)
    });
    // Stateful: the mute button and the companion's menu item show the state,
    // and a press asks for the other one. The state is the call's own; only
    // render_voice sets it.
    let mute = gio::SimpleAction::new_stateful("voice-mute", None, &false.to_variant());
    let weak = Rc::downgrade(app);
    mute.connect_change_state(move |_, wanted| {
        let Some(app) = weak.upgrade() else { return };
        let wanted = wanted.and_then(glib::Variant::get::<bool>);
        let wanted = wanted.expect("voice-mute holds a boolean");
        app.voice_input(Input::Mute(wanted));
    });
    app.application.add_action(&mute);
    let weak = Rc::downgrade(app);
    app.application.connect_shutdown(move |_| {
        if let Some(app) = weak.upgrade() {
            app.hang_up();
        }
    });
}

/// Begins or ends the call. With something in the way, a click on the
/// companion brings up the Voice page, which says what.
fn toggle_call(app: &Rc<App>) {
    if app.call.borrow().session.in_call() {
        app.voice_input(Input::End);
        return;
    }
    if !app.voice_view().can_begin {
        app.shell.window.present();
        app.show_page("companion");
        return;
    }
    app.voice_input(Input::Begin);
}

/// The companion window's wiring: the Voice page's switch, its menu's way back
/// to Fermix, and closing. Closing the main window while the companion is up
/// only hides it, so the call carries on; closing the companion then brings
/// the main window back. Without the companion, closing the main window quits.
pub fn install_companion(app: &Rc<App>) {
    app_action(app, "show-voice", |app| {
        app.shell.window.present();
        app.show_page("companion");
    });
    app_action(app, "companion-close", |app| app.show_companion(false));
    let weak = Rc::downgrade(app);
    app.voice.companion.connect_active_notify(move |row| {
        if let Some(app) = weak.upgrade() {
            app.show_companion(row.is_active());
        }
    });
    let weak = Rc::downgrade(app);
    app.companion.window.connect_close_request(move |_| {
        if let Some(app) = weak.upgrade() {
            app.show_companion(false);
        }
        glib::Propagation::Stop
    });
    let weak = Rc::downgrade(app);
    app.shell.window.connect_close_request(move |window| {
        let Some(app) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        match main_window_close(app.companion.window.is_visible(), app.in_tray()) {
            MainWindowClose::Hide => window.set_visible(false),
            // Nothing on screen would show a call, so none is left running.
            MainWindowClose::ToTray => {
                if app.call.borrow().session.in_call() {
                    app.hang_up();
                }
                window.set_visible(false);
            }
            // Closing would leave Fermix running unseen behind the hidden companion window.
            // Shutdown hangs up any call while the window is still whole.
            MainWindowClose::Quit => app.application.quit(),
        }
        glib::Propagation::Stop
    });
}

impl App {
    /// Closing the companion never quits Fermix by surprise: with the main
    /// window hidden behind it, the main window comes back instead, unless the
    /// tray icon keeps Fermix within reach.
    pub fn show_companion(self: &Rc<Self>, on: bool) {
        if on {
            self.watch_microphones();
        }
        self.companion.window.set_visible(on);
        // The switch's own notify calls back in here; only a change sets it.
        if self.voice.companion.is_active() != on {
            self.voice.companion.set_active(on);
        }
        // With the main window closed too, the tray icon may be all that is left: a call
        // then ends, since nothing on screen would show it. Without one the window returns.
        let unseen = !on && !self.shell.window.is_visible();
        if unseen && !self.in_tray() {
            self.shell.window.present();
        } else if unseen && self.call.borrow().session.in_call() {
            self.hang_up();
        }
        self.show_tray();
    }
}

/// An action on the application, so both voice windows reach it.
fn app_action(app: &Rc<App>, name: &str, handler: impl Fn(&Rc<App>) + 'static) {
    let action = gio::SimpleAction::new(name, None);
    let weak = Rc::downgrade(app);
    action.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            handler(&app);
        }
    });
    app.application.add_action(&action);
}
