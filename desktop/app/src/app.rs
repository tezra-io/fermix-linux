//! The controller: owns `State`, turns window actions into daemon calls, and
//! redraws the pages. Pages never call the daemon themselves.

use crate::assistant::Assistant;
use crate::audio::Endpoints;
use crate::chat::ChatPage;
use crate::companion::Companion;
use crate::conversation::Conversation;
use crate::daemon::Daemon;
use crate::doctor::DoctorPage;
use crate::home::HomePage;
use crate::integrations::IntegrationsPane;
use crate::logs::LogsPage;
use crate::providers::ProvidersPage;
use crate::settings::{SettingsData, SettingsPage};
use crate::shell::{self, Shell};
use crate::state::{Connection, Snapshot, State, RECENT_FOR};
use crate::voice::VoicePage;
use crate::voice_call::Call;
use adw::prelude::*;
use fermix_client::management::{CallError, PROTOCOL_VERSION};
use fermix_client::model::{Hello, SetupState};
use fermix_client::overview::Overview;
use fermix_client::view::{daemon_problem, hello_problem, Activity, Recent};
use gtk::gio;
use gtk::glib::{
    self,
    variant::{FromVariant, StaticVariantType, ToVariant},
};
use serde_json::Value;
use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How often the window re-reads the daemon while nothing is happening.
const IDLE_REFRESH: Duration = Duration::from_secs(30);

pub struct App {
    pub shell: Shell,
    pub daemon: Daemon,
    pub state: RefCell<State>,
    pub conversation: RefCell<Conversation>,
    pub chat: ChatPage,
    pub settings: SettingsPage,
    pub settings_data: RefCell<SettingsData>,
    /// The setup assistant while it is open.
    pub assistant: RefCell<Option<Rc<Assistant>>>,
    /// Pages that read the daemon themselves while they are in view.
    pub integrations: Rc<IntegrationsPane>,
    doctor: Rc<DoctorPage>,
    logs: Rc<LogsPage>,
    pub voice: VoicePage,
    pub companion: Companion,
    /// The voice connection and its call, if any.
    pub call: RefCell<Call>,
    home: HomePage,
    providers: ProvidersPage,
}

pub fn activate(application: &adw::Application) {
    // Launching again raises the main window, even from behind the companion.
    let main = application
        .windows()
        .into_iter()
        .find(|w| w.is::<adw::ApplicationWindow>());
    if let Some(window) = main {
        window.present();
        return;
    }
    let app = build_app(application);
    start(&app);
}

/// Every page, the window around them, and the controller that owns both.
fn build_app(application: &adw::Application) -> Rc<App> {
    let chat = ChatPage::new();
    let home = HomePage::new();
    let providers = ProvidersPage::new();
    let toast = through_window(application, "win.toast");
    let integrations = IntegrationsPane::new(Daemon::new(), Rc::new(toast));
    let settings = SettingsPage::new(
        providers.root.upcast_ref(),
        &providers.page,
        &integrations.page,
    );
    let doctor = DoctorPage::new(
        Daemon::new(),
        Box::new(through_window(application, "win.page")),
    );
    let logs = LogsPage::new(Daemon::new());
    let voice = VoicePage::new();
    let companion = Companion::new(application);
    let pages = [
        chat.root.upcast_ref::<gtk::Widget>(),
        voice.root.upcast_ref(),
        home.root.upcast_ref(),
        &doctor.root,
        &logs.root,
    ];
    let shell = shell::build(application, &pages, &settings);
    Rc::new(App {
        shell,
        daemon: Daemon::new(),
        state: RefCell::new(State::new()),
        conversation: RefCell::default(),
        chat,
        settings,
        settings_data: RefCell::default(),
        assistant: RefCell::default(),
        integrations,
        doctor,
        logs,
        voice,
        companion,
        call: RefCell::new(Call::new(audio_endpoints())),
        home,
        providers,
    })
}

/// `FERMIX_AUDIO=test` swaps the microphone and speakers for a sine and a
/// fakesink, for the development harness. Nothing else reads it.
fn audio_endpoints() -> Endpoints {
    match std::env::var("FERMIX_AUDIO") {
        Ok(value) if value == "test" => Endpoints::Test,
        _ => Endpoints::Sound,
    }
}

/// Wires the controller in, shows the window, and makes the first read.
fn start(app: &Rc<App>) {
    app.state.borrow_mut().background.opens_at_login = crate::portal::opens_at_login();
    install_actions(app);
    follow_visible_page(app);
    keep_alive_with_window(app);
    crate::voice_flow::install_companion(app);
    crate::voice_flow::install_call_actions(app);
    app.shell.show_page("home");
    app.render();
    app.shell.window.present();
    let first = app.clone();
    glib::spawn_future_local(async move {
        first.refresh().await;
        first.land();
        // Until setup is finished the assistant opens with the window (M38 §5.5).
        let setup_needed = first
            .state
            .borrow()
            .snapshot()
            .is_some_and(|s| s.state.readiness.status != "ready");
        if setup_needed {
            first.open_assistant();
        }
        first.reassert_open_at_login().await;
    });
    start_idle_refresh(app);
}

impl App {
    pub fn render(&self) {
        let state = self.state.borrow();
        self.chat
            .render(&self.conversation.borrow().transcript, &state);
        self.home.render(&state);
        self.providers.render(&state, Instant::now());
        self.settings.render(&state, &self.settings_data.borrow());
        // One prominent header action (M38 §5.6): finish setup first, then restart.
        // Behind an outside change the banner comes first: reload, then restart (spec G7).
        let setup_needed = state
            .snapshot()
            .is_some_and(|s| s.state.readiness.status != "ready");
        let restart_pending = state.snapshot().is_some_and(|s| {
            s.state.restart.required && s.state.coexistence.config_state != "external_change"
        });
        self.shell.continue_setup.set_visible(setup_needed);
        self.shell
            .restart
            .set_visible(restart_pending && !setup_needed);
        drop(state);
        self.render_voice();
        self.render_assistant();
    }

    /// Re-reads the daemon and redraws. A failure is a state, never an error dialog.
    pub async fn refresh(&self) {
        let answer = self
            .daemon
            .call(|m| Ok((m.hello()?, m.setup_state()?, m.overview())))
            .await;
        let connection = connection_from(answer);
        let pid = match &connection {
            Connection::Up(snapshot) => Some(snapshot.pid.clone()),
            _ => None,
        };
        {
            let mut state = self.state.borrow_mut();
            if matches!(connection, Connection::Up(_)) {
                state.wake_failed = false;
            }
            state.connection = connection;
        }
        self.read_service().await;
        self.render();
        if let Some(pid) = pid {
            self.follow_daemon_process(pid).await;
        }
    }

    pub fn set_activity(&self, provider: &str, activity: Activity) {
        self.state
            .borrow_mut()
            .activity
            .insert(provider.to_owned(), activity);
        self.render();
    }

    pub fn clear_activity(&self, provider: &str) {
        let mut state = self.state.borrow_mut();
        state.activity.remove(provider);
        state.links.remove(provider);
        state.recent.remove(provider);
    }

    /// Shows how the last action ended (failed, cancelled) on its row for
    /// `RECENT_FOR`, then lets the row relax unless something newer replaced it.
    pub fn settle(self: &Rc<Self>, provider: &str, ended: Activity) {
        assert!(
            !ended.in_flight(),
            "only an ended action settles: {ended:?}"
        );
        self.set_activity(provider, ended.clone());
        let weak = Rc::downgrade(self);
        let provider = provider.to_owned();
        glib::timeout_add_local_once(RECENT_FOR, move || {
            let Some(app) = weak.upgrade() else { return };
            let still_shown = app.state.borrow().activity.get(&provider) == Some(&ended);
            if still_shown {
                app.state.borrow_mut().activity.remove(&provider);
                app.render();
            }
        });
    }

    /// Acknowledges a change on its row for `RECENT_FOR`, then lets the row relax.
    pub fn mark_recent(self: &Rc<Self>, provider: &str, recent: Recent) {
        self.state
            .borrow_mut()
            .recent
            .insert(provider.to_owned(), (recent, Instant::now()));
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(RECENT_FOR + Duration::from_secs(1), move || {
            if let Some(app) = weak.upgrade() {
                app.render();
            }
        });
    }

    /// After the first read: Chat when Fermix is ready, Home when it needs
    /// setting up. Leaves the window alone if the user has already moved.
    fn land(&self) {
        if self.shell.visible_page().as_deref() != Some("home") || !self.is_ready() {
            return;
        }
        self.shell.show_page("chat");
        self.chat.focus_input();
    }

    pub fn is_ready(&self) -> bool {
        self.state
            .borrow()
            .snapshot()
            .is_some_and(|s| s.state.readiness.status == "ready")
    }
}

/// One read of the daemon: who it is, where setup stands, and its own
/// account of itself (which Home can do without).
type Read = (Hello, SetupState, Result<Overview, CallError>);

/// What one read of the daemon says about it. A daemon that answers but cannot
/// serve this app's protocol is down for this app, with the side to update named.
fn connection_from(answer: Result<Read, CallError>) -> Connection {
    let (hello, state, overview) = match answer {
        Ok(read) => read,
        Err(e) => return Connection::Down(daemon_problem(&e)),
    };
    if let Some(problem) = hello_problem(&hello) {
        return Connection::Down(problem);
    }
    Connection::Up(Box::new(Snapshot {
        state,
        version: hello.engine.product_version,
        pid: hello.engine.pid,
        architecture: hello.engine.architecture,
        distribution: hello.engine.distribution_identity,
        protocol: PROTOCOL_VERSION.min(hello.protocol.maximum_version),
        overview: overview
            .inspect_err(|e| glib::g_warning!("fermix", "overview.get failed: {e:?}"))
            .ok(),
    }))
}

/// A callback for a page built before its window: it fires the window action
/// `action` with the text as its target, once there is a window to take it.
fn through_window(application: &adw::Application, action: &'static str) -> impl Fn(&str) {
    let application = application.downgrade();
    move |text: &str| {
        let Some(window) = application.upgrade().and_then(|a| a.active_window()) else {
            glib::g_warning!("fermix", "{action} arrived with no window to take it");
            return;
        };
        if let Err(e) = window.activate_action(action, Some(&text.to_variant())) {
            glib::g_warning!("fermix", "{action} could not be sent: {e}");
        }
    }
}

/// Doctor runs its checks as it comes into view, and Logs reads only while in
/// view. Following the stack catches every way in and out.
fn follow_visible_page(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    app.shell
        .stack
        .connect_visible_child_name_notify(move |stack| {
            let Some(app) = weak.upgrade() else { return };
            let name = stack.visible_child_name();
            match name.as_deref() {
                Some("doctor") => app.doctor.shown(),
                Some("logs") => app.logs.shown(),
                _ => app.logs.hidden(),
            }
        });
}

/// The window owns the controller: actions and timers hold only weak references,
/// so without this the controller would be dropped after the first refresh.
fn keep_alive_with_window(app: &Rc<App>) {
    let owner = app.clone();
    app.shell.window.connect_close_request(move |_| {
        let _ = &owner;
        glib::Propagation::Proceed
    });
}

fn start_idle_refresh(app: &Rc<App>) {
    let weak = Rc::downgrade(app);
    glib::timeout_add_local(IDLE_REFRESH, move || {
        let Some(app) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        let busy = app.state.borrow().in_flight();
        if !busy {
            glib::spawn_future_local(async move { app.refresh().await });
        }
        glib::ControlFlow::Continue
    });
}

/// Registers a window action that takes a string target and runs `handler` on the main loop.
fn on<F, Fut>(app: &Rc<App>, name: &str, handler: F)
where
    F: Fn(Rc<App>, String) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(app);
    action.connect_activate(move |_, target| {
        let (Some(app), Some(target)) = (weak.upgrade(), target.and_then(|t| t.get::<String>()))
        else {
            return;
        };
        glib::spawn_future_local(handler(app, target));
    });
    app.shell.window.add_action(&action);
}

/// Registers a window action whose target is any variant type, such as a tuple.
fn on_value<T, F, Fut>(app: &Rc<App>, name: &str, handler: F)
where
    T: FromVariant + StaticVariantType,
    F: Fn(Rc<App>, T) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let action = gio::SimpleAction::new(name, Some(&T::static_variant_type()));
    let weak = Rc::downgrade(app);
    action.connect_activate(move |_, target| {
        let (Some(app), Some(target)) = (weak.upgrade(), target.and_then(|t| t.get::<T>())) else {
            glib::g_warning!("fermix", "an action arrived without its target");
            return;
        };
        glib::spawn_future_local(handler(app, target));
    });
    app.shell.window.add_action(&action);
}

/// Registers a window action with no target.
fn on_plain<F, Fut>(app: &Rc<App>, name: &str, handler: F)
where
    F: Fn(Rc<App>) -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    let action = gio::SimpleAction::new(name, None);
    let weak = Rc::downgrade(app);
    action.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            glib::spawn_future_local(handler(app));
        }
    });
    app.shell.window.add_action(&action);
}

fn install_actions(app: &Rc<App>) {
    on(app, "page", |app, page| async move { app.show_page(&page) });
    on(
        app,
        "toast",
        |app, text| async move { app.shell.toast(&text) },
    );
    install_settings_actions(app);
    install_setup_actions(app);
    install_service_actions(app);
    install_chat_actions(app);
    install_accels(app);
    on(app, "door", |app, target| async move {
        app.open_door(&target).await
    });
    on(app, "cancel", |app, provider| async move {
        app.cancel(&provider).await
    });
    on(app, "copy-link", |app, provider| async move {
        app.copy_link(&provider)
    });
    on(app, "make-primary", |app, provider| async move {
        app.make_primary(&provider).await
    });
    on(app, "sign-out", |app, provider| async move {
        app.sign_out(&provider).await
    });
    on(app, "remove-key", |app, provider| async move {
        app.remove_key(&provider).await
    });
}

/// The service, Home's switches, and what Home and Voice offer to fix.
fn install_service_actions(app: &Rc<App>) {
    on_plain(app, "restart", |app| async move { app.restart().await });
    on_plain(app, "restart-service", |app| async move {
        app.restart_service().await
    });
    on_plain(app, "start-service", |app| async move {
        app.start_service().await
    });
    on_value(app, "run-in-background", |app, on: bool| async move {
        app.set_background(on).await
    });
    on_value(app, "open-at-login", |app, on: bool| async move {
        app.set_open_at_login(on).await
    });
    on_plain(app, "voice-fix", |app| async move { app.voice_fix().await });
    on_plain(app, "open-setup-page", |app| async move {
        app.open_setup_page().await
    });
}

fn install_chat_actions(app: &Rc<App>) {
    on_plain(app, "send-message", |app| async move {
        app.send_message().await
    });
    on_plain(app, "stop-reply", |app| async move { app.stop_reply() });
    on_plain(
        app,
        "retry-reply",
        |app| async move { app.retry_reply().await },
    );
    on_plain(app, "new-chat", |app| async move {
        app.shell.show_page("chat");
        app.new_conversation()
    });
}

fn install_accels(app: &Rc<App>) {
    let application = app
        .shell
        .window
        .application()
        .expect("the window belongs to the application");
    application.set_accels_for_action("win.new-chat", &["<Ctrl>n"]);
    application.set_accels_for_action("win.open-settings", &["<Ctrl>comma"]);
    application.set_accels_for_action("win.find-setting", &["<Ctrl>f"]);
    for (index, (page, _, _)) in shell::PAGES.iter().enumerate() {
        application.set_accels_for_action(
            &format!("win.page::{page}"),
            &[&format!("<Ctrl>{}", index + 1)],
        );
    }
}

fn install_settings_actions(app: &Rc<App>) {
    on_plain(app, "open-settings", |app| async move {
        app.show_page(&app.shell.last_pane());
    });
    on_plain(app, "find-setting", |app| async move {
        if app.shell.visible_pane().is_none() {
            app.show_page(&app.shell.last_pane());
        }
        app.settings.focus_search();
    });
    on_plain(app, "leave-settings", |app| async move {
        app.shell.leave_settings()
    });
    on(app, "settings-search", |app, query| async move {
        app.search_settings(&query)
    });
    on_plain(app, "settings-reload", |app| async move {
        app.reload_settings().await
    });
    on_value(
        app,
        "setting-apply",
        |app, (section, key, json): (String, String, String)| async move {
            match serde_json::from_str::<Value>(&json) {
                Ok(value) => app.apply_setting(section, key, value).await,
                Err(e) => {
                    glib::g_warning!("fermix", "a control sent a value that is not JSON: {e}")
                }
            }
        },
    );
    on_value(
        app,
        "secret-add",
        |app, (section, key): (String, String)| async move { app.add_secret(section, key) },
    );
    on_value(
        app,
        "secret-remove",
        |app, (section, key): (String, String)| async move { app.remove_secret(section, key).await },
    );
    on(app, "channel-setup", |app, channel| async move {
        app.channel_setup(&channel)
    });
    on(app, "provider-settings", |app, provider| async move {
        app.provider_settings(&provider)
    });
}

/// The setup assistant, and the Meetings and Computer helpers.
fn install_setup_actions(app: &Rc<App>) {
    on_value(
        app,
        "capability-switch",
        |app, (target, on): (String, bool)| async move { app.capability_switch(target, on).await },
    );
    on(app, "cancel-job", |app, job_id| async move {
        app.cancel_job(job_id).await
    });
    on_plain(
        app,
        "open-assistant",
        |app| async move { app.open_assistant() },
    );
    on_plain(app, "assistant-begin", |app| async move {
        app.assistant_advance().await
    });
    on_plain(app, "assistant-next", |app| async move {
        app.assistant_advance().await
    });
    on_plain(app, "assistant-apply", |app| async move {
        app.assistant_apply(true).await
    });
    on(app, "assistant-finish", |app, target| async move {
        app.assistant_finish(&target)
    });
    on_plain(app, "meetings-sign-in", |app| async move {
        app.meetings_sign_in().await
    });
    on_plain(app, "detect-meetbot", |app| async move {
        app.detect_meetbot().await
    });
    on_plain(app, "probe-computer", |app| async move {
        app.probe_computer().await
    });
}
