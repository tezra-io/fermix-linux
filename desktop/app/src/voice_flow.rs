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
    call_status, expression, voice_gate, GateAction, Microphone, Reach, Source, VoiceFacts,
};
use gtk::{gio, glib};
use serde_json::Value;
use std::rc::Rc;

const SECTION: &str = "realtime";
const ENABLED: &str = "realtime_enabled";
const KEY: &str = "openai_api_key";

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
        let status = call_status(session, gate.ready || in_call);
        VoiceView {
            gate,
            can_begin,
            word: status.label,
            icon: status.icon,
            palette: status.palette,
            expression: expression(session.visual_mode()),
            level: session.level(),
            in_call,
            muted: session.muted(),
            can_stop: session.can_stop(),
            can_cancel_task: session.can_cancel_task(),
            busy: !in_call && session.mode() == Mode::Connecting,
            caption: session.caption_line(),
            task: session.task_line(),
            usage: session.usage_line(),
            microphone: microphone.label().to_owned(),
        }
    }

    /// Redraws only what the call changes, as often as the call changes.
    pub fn render_voice(&self) {
        let view = self.voice_view();
        let mute = application(self)
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

    fn microphone_changed(&self, sources: &[Source]) {
        let now = Microphone::from_sources(sources);
        if *self.microphone.borrow() == now {
            return;
        }
        glib::g_info!("fermix", "voice records from {now:?}");
        self.microphone.replace(now);
        self.render_voice();
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
    application(app).add_action(&mute);
    let weak = Rc::downgrade(app);
    application(app).connect_shutdown(move |_| {
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
/// the main window back.
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
        let companion_up = weak
            .upgrade()
            .is_some_and(|app| app.companion.window.is_visible());
        if !companion_up {
            return glib::Propagation::Proceed;
        }
        window.set_visible(false);
        glib::Propagation::Stop
    });
}

impl App {
    /// Closing the companion never quits Fermix by surprise: with the main
    /// window hidden behind it, the main window comes back instead.
    pub fn show_companion(self: &Rc<Self>, on: bool) {
        if on {
            self.watch_microphones();
        }
        self.companion.window.set_visible(on);
        // The switch's own notify calls back in here; only a change sets it.
        if self.voice.companion.is_active() != on {
            self.voice.companion.set_active(on);
        }
        if !on && !self.shell.window.is_visible() {
            self.shell.window.present();
        }
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
    application(app).add_action(&action);
}

fn application(app: &App) -> gtk::Application {
    app.shell
        .window
        .application()
        .expect("the window belongs to the application")
}
