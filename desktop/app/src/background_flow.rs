//! Home's Background switches. "Run in the background" registers or
//! unregisters the unit with systemd (spec §6.3); "Open at login" asks the
//! desktop through its portal inside the Flatpak (§6.6), and outside a sandbox
//! keeps the autostart entry itself. Neither ever changes the other.

use crate::app::App;
use crate::dialogs::confirm;
use crate::portal::{remember_login, request_background};
use crate::systemd::{disable_service, enable_service, read_service};
use fermix_client::autostart::{entry_path, read_entry, remove_entry, write_entry};
use fermix_client::service::{
    background_switch, disable_warning, login_answer, LoginAnswer, ServiceRead,
};
use gtk::glib;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

/// Waiting for the unit to stop after a disable: every second, for at most 30 s.
const STOP_INTERVAL: Duration = Duration::from_secs(1);
const STOP_TRIES: u32 = 30;

/// Where the package's binding lives. Readable only with the manifest's
/// `xdg-config/fermix:ro`; without it the file reads as absent.
fn binding_exists() -> bool {
    glib::home_dir()
        .join(".config/fermix/service.json")
        .exists()
}

/// Who keeps the "Open at login" entry. The Background portal serves sandboxed
/// apps; outside a sandbox there is none to ask, so the window writes the
/// entry itself, and the file is the truth.
pub enum LoginEntry {
    Portal,
    File(PathBuf),
}

impl LoginEntry {
    /// Decided once, at startup. `/.flatpak-info` exists only in a Flatpak sandbox.
    pub fn detect() -> LoginEntry {
        if Path::new("/.flatpak-info").exists() {
            return LoginEntry::Portal;
        }
        let config_home = std::env::var_os("XDG_CONFIG_HOME");
        LoginEntry::File(entry_path(config_home.as_deref(), &glib::home_dir()))
    }
}

/// Whether the entry opens Fermix at login. One that cannot be read is logged
/// and drawn off, so the switch offers to write it again.
fn read_login_entry(path: &Path) -> bool {
    read_entry(path).unwrap_or_else(|e| {
        glib::g_warning!(
            "fermix",
            "the autostart entry {} could not be read: {e}",
            path.display()
        );
        false
    })
}

impl App {
    /// Re-reads what systemd and logind say, whether a binding exists, and
    /// outside a sandbox the autostart entry. A daemon that answered once
    /// proves a binding, and disabling keeps it, so that proof outlives the daemon.
    pub async fn read_service(&self) {
        let read = read_service().await;
        let file = binding_exists();
        let mut state = self.state.borrow_mut();
        let answered = state.snapshot().is_some();
        state.background.service = Some(read);
        state.background.binding |= file || answered;
        // Anything may change a file; the portal's answer can only be remembered.
        if let LoginEntry::File(path) = &self.login {
            state.background.opens_at_login = read_login_entry(path);
        }
    }

    /// The switch was moved to `on`. A move that only mirrors what is drawn, or
    /// arrives while a change runs, does nothing.
    pub async fn set_background(self: Rc<Self>, on: bool) {
        let drawn = {
            let state = self.state.borrow();
            let bg = &state.background;
            background_switch(bg.service.as_ref(), bg.binding, bg.service_change.is_some())
        };
        if !drawn.sensitive || drawn.on == on {
            return;
        }
        self.state.borrow_mut().background.service_error = None;
        if on {
            self.enable_background().await;
        } else {
            self.disable_background().await;
        }
        self.state.borrow_mut().background.service_change = None;
        self.read_service().await;
        self.render();
    }

    async fn enable_background(&self) {
        let (linger, up) = {
            let state = self.state.borrow();
            let linger = match &state.background.service {
                Some(ServiceRead::Unit { linger, .. }) => *linger,
                _ => None,
            };
            (linger, state.snapshot().is_some())
        };
        self.state.borrow_mut().background.service_change = Some(true);
        self.render();
        if let Err(sentence) = enable_service(linger).await {
            glib::g_warning!(
                "fermix",
                "the background service was not enabled: {sentence}"
            );
            return self.service_refused(sentence);
        }
        if !up {
            self.wait_for_daemon(None, "Background service enabled")
                .await;
            return;
        }
        self.refresh().await;
        self.shell.toast("Background service enabled");
    }

    async fn disable_background(&self) {
        let active = self
            .state
            .borrow()
            .snapshot()
            .and_then(|s| s.overview.as_ref())
            .map(|o| o.agents.main.active_conversations);
        let body = disable_warning(active);
        let heading = "Turn off the background service?";
        if !confirm(&self.shell.window, heading, &body, "Turn Off", true).await {
            // The switch snaps back to what systemd says.
            return self.render();
        }
        self.state.borrow_mut().background.service_change = Some(false);
        self.render();
        if let Err(sentence) = disable_service().await {
            glib::g_warning!(
                "fermix",
                "the background service was not disabled: {sentence}"
            );
            return self.service_refused(sentence);
        }
        self.wait_for_stop().await;
        self.refresh().await;
        self.shell.toast("Background service disabled");
    }

    /// Keeps a refusal under the switch, where its command can be copied, until
    /// the next change.
    fn service_refused(&self, sentence: String) {
        self.state.borrow_mut().background.service_error = Some(sentence);
    }

    /// Re-reads the unit until systemd says it has stopped, or the tries run out.
    async fn wait_for_stop(&self) {
        for _ in 0..STOP_TRIES {
            self.read_service().await;
            let stopped = matches!(
                &self.state.borrow().background.service,
                Some(ServiceRead::Unit { unit, .. })
                    if unit.active_state == "inactive" || unit.active_state == "failed"
            );
            if stopped {
                return;
            }
            glib::timeout_future(STOP_INTERVAL).await;
        }
        glib::g_warning!(
            "fermix",
            "fermix.service had not stopped after {STOP_TRIES} s"
        );
    }

    /// The "Open at login" switch was moved to `on`.
    pub async fn set_open_at_login(self: Rc<Self>, on: bool) {
        {
            let state = self.state.borrow();
            let bg = &state.background;
            if bg.login_change.is_some() || bg.opens_at_login == on {
                return;
            }
        }
        self.state.borrow_mut().background.login_change = Some(on);
        self.render();
        match &self.login {
            LoginEntry::Portal => self.ask_desktop(on).await,
            LoginEntry::File(path) => self.keep_entry(path, on),
        }
        self.state.borrow_mut().background.login_change = None;
        self.render();
    }

    /// Whether Fermix opens at login, when the window starts: the portal's
    /// remembered answer, or the entry itself.
    pub fn read_open_at_login(&self) -> bool {
        match &self.login {
            LoginEntry::Portal => crate::portal::opens_at_login(),
            LoginEntry::File(path) => read_login_entry(path),
        }
    }

    /// At startup: the portal has no getter, so a remembered yes is asked again,
    /// and an entry the window keeps is written again. Either repairs an entry
    /// an update left stale, such as the Flatpak's after a move to the package.
    pub async fn reassert_open_at_login(&self) {
        if !self.state.borrow().background.opens_at_login {
            return;
        }
        match &self.login {
            LoginEntry::Portal => self.ask_desktop(true).await,
            LoginEntry::File(path) => self.keep_entry(path, true),
        }
        self.render();
    }

    /// Outside a sandbox: writes the entry to open at login, or removes it, then
    /// reads back what the file says now.
    fn keep_entry(&self, path: &Path, on: bool) {
        let changed = if on {
            write_entry(path)
        } else {
            remove_entry(path)
        };
        if let Err(e) = changed {
            glib::g_warning!(
                "fermix",
                "the autostart entry {} was not changed: {e}",
                path.display()
            );
            self.shell.toast(if on {
                "Fermix could not turn on Open at login."
            } else {
                "Fermix could not turn off Open at login."
            });
        }
        self.state.borrow_mut().background.opens_at_login = read_login_entry(path);
    }

    async fn ask_desktop(&self, on: bool) {
        let answer = match request_background(on).await {
            Ok((code, autostart)) => login_answer(code, autostart, on),
            Err(e) => {
                glib::g_warning!("fermix", "the Background portal was not asked: {e}");
                LoginAnswer::Refused("Your desktop could not change this, so nothing changed.")
            }
        };
        let now = match answer {
            LoginAnswer::Set(now) => now,
            LoginAnswer::Cancelled => return,
            LoginAnswer::Refused(sentence) if on => {
                self.shell.toast(sentence);
                false
            }
            LoginAnswer::Refused(sentence) => return self.shell.toast(sentence),
        };
        self.state.borrow_mut().background.opens_at_login = now;
        if let Err(e) = remember_login(now) {
            glib::g_warning!("fermix", "the Open at login answer was not kept: {e}");
            self.shell.toast(
                "Fermix could not remember this answer, so the switch may be wrong next time.",
            );
        }
    }
}
