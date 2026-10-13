//! The controller's side of the tray icon: the view it shows, read from the
//! same state Home draws, and what its rows do. A row that opens a page or may
//! ask a question brings the window up first. Fermix is never left running
//! with nothing on screen: without a tray host the window comes back.

use crate::app::App;
use crate::state::Connection;
use crate::tray::{Tray, TrayEvent};
use adw::prelude::*;
use fermix_client::service::{background_switch, BackgroundSwitch};
use fermix_client::tray::{tray_view, Command, Daemon, TrayFacts, TrayView};
use gtk::glib;
use std::rc::Rc;
use std::time::Duration;

/// How long a start in the background waits for a tray host before it shows
/// the window instead.
const HOST_GRACE: Duration = Duration::from_secs(3);

impl App {
    /// Puts the icon up. `hidden`: the window stays closed if a tray host
    /// takes the icon within `HOST_GRACE`, and opens otherwise.
    pub fn start_tray(self: &Rc<Self>, hidden: bool) {
        let weak = Rc::downgrade(self);
        let view = self.tray_view();
        glib::spawn_future_local(async move {
            let events = weak.clone();
            let tray = Tray::connect(view, move |event| {
                if let Some(app) = events.upgrade() {
                    app.tray_event(event);
                }
            })
            .await;
            let Some(app) = weak.upgrade() else { return };
            match tray {
                Ok(tray) => *app.tray.borrow_mut() = Some(tray),
                Err(e) => glib::g_warning!("fermix", "the tray icon is not available: {e}"),
            }
        });
        if hidden {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(HOST_GRACE, move || {
                if let Some(app) = weak.upgrade() {
                    app.surface_if_unseen();
                }
            });
        }
    }

    /// Whether a tray host shows the icon, so Fermix can be reached with no window.
    pub fn in_tray(&self) -> bool {
        self.tray.borrow().as_ref().is_some_and(Tray::hosted)
    }

    pub fn show_tray(&self) {
        if let Some(tray) = self.tray.borrow().as_ref() {
            tray.show(self.tray_view());
        }
    }

    fn tray_view(&self) -> TrayView {
        let background = self.background();
        let state = self.state.borrow();
        let daemon = match &state.connection {
            Connection::Connecting => Daemon::Reading,
            Connection::Down(_) if state.waking => Daemon::Waking,
            Connection::Down(problem) => Daemon::Down(problem),
            Connection::Up(snapshot) => Daemon::Up {
                state: &snapshot.state,
                uptime_ms: snapshot.overview.as_ref().and_then(|o| o.daemon.uptime_ms),
            },
        };
        tray_view(&TrayFacts {
            daemon,
            pet_shown: self.companion.window.is_visible(),
            background: &background,
        })
    }

    fn tray_event(self: &Rc<Self>, event: TrayEvent) {
        match event {
            TrayEvent::Activate => self.shell.window.present(),
            TrayEvent::Command(command) => self.tray_command(command),
            // The status line is read as the menu opens, as on macOS, so it is
            // never an hour-old "Running".
            TrayEvent::Opening => {
                let app = self.clone();
                glib::spawn_future_local(async move { app.refresh().await });
            }
            TrayEvent::Hosted(true) => {}
            TrayEvent::Hosted(false) => self.surface_if_unseen(),
        }
    }

    fn tray_command(self: &Rc<Self>, command: Command) {
        if command.needs_window() {
            self.shell.window.present();
        }
        match command {
            Command::OpenFermix => {}
            Command::Settings => self.window_action("open-settings", None),
            Command::Doctor => self.show_page("doctor"),
            Command::Restart => self.window_action("restart", None),
            Command::RestartService => self.window_action("restart-service", None),
            Command::StartService => self.window_action("start-service", None),
            Command::TogglePet => self.show_companion(!self.companion.window.is_visible()),
            Command::ToggleBackground => {
                let on = self.background().on;
                self.window_action("run-in-background", Some(&(!on).to_variant()));
            }
            Command::Quit => self.application.quit(),
        }
    }

    /// Home's "Run in the background" switch, as Home draws it.
    fn background(&self) -> BackgroundSwitch {
        let state = self.state.borrow();
        let bg = &state.background;
        background_switch(bg.service.as_ref(), bg.binding, bg.service_change.is_some())
    }

    fn window_action(&self, name: &str, target: Option<&glib::Variant>) {
        let window = self.shell.window.upcast_ref::<gtk::Widget>();
        if let Err(e) = WidgetExt::activate_action(window, &format!("win.{name}"), target) {
            glib::g_warning!("fermix", "the tray could not run {name}: {e}");
        }
    }

    /// With no tray host and nothing on screen, Fermix would run unseen: the
    /// window comes back.
    fn surface_if_unseen(&self) {
        let unseen = !self.shell.window.is_visible() && !self.companion.window.is_visible();
        if unseen && !self.in_tray() {
            self.shell.window.present();
        }
    }
}
