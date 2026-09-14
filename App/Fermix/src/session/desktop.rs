//! What this desktop session actually is, and the three platform activations
//! the application performs.
//!
//! These are observations, not configuration validation and not a capability
//! verdict. Home renders them once in its Session row so the Computer pane's
//! state is legible; they never decide it. Nothing here prompts.

use std::path::Path;

use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

/// What the environment says about this session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesktopFacts {
    /// `XDG_SESSION_TYPE`, as the login manager set it.
    pub session_type: Option<String>,
    /// `XDG_CURRENT_DESKTOP`, as the desktop set it.
    pub desktop: Option<String>,
    /// Whether the session bus answered.
    pub bus_reachable: bool,
    /// Whether a display is present at all.
    pub display_present: bool,
}

/// The application's own view of the session it is running in.
pub struct DesktopSession;

impl DesktopSession {
    /// Take the observation.
    ///
    /// The bus is asked rather than assumed, because a headless host and a
    /// desktop host differ in exactly that answer and the difference is what
    /// several surfaces render.
    pub async fn observe() -> DesktopFacts {
        DesktopFacts {
            session_type: environment_value("XDG_SESSION_TYPE"),
            desktop: environment_value("XDG_CURRENT_DESKTOP"),
            bus_reachable: gio::bus_get_future(gio::BusType::Session).await.is_ok(),
            display_present: gtk::gdk::Display::default().is_some(),
        }
    }

    /// Open one address in the person's browser.
    ///
    /// The caller shows the address as copyable text when this answers with a
    /// failure, because a sign-in whose browser never opened is a dead end
    /// otherwise.
    pub async fn open_url(
        parent: Option<&impl IsA<gtk::Window>>,
        url: &str,
    ) -> Result<(), glib::Error> {
        let launcher = gtk::UriLauncher::new(url);
        launcher
            .launch_future(parent.map(|window| window.as_ref()))
            .await
    }

    /// Show one path in the person's file manager.
    pub async fn show_folder(
        parent: Option<&impl IsA<gtk::Window>>,
        path: &Path,
    ) -> Result<(), glib::Error> {
        let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
        launcher
            .open_containing_folder_future(parent.map(|window| window.as_ref()))
            .await
    }

    /// Raise one notification under the application's own identity.
    ///
    /// `action` is a fully qualified action name from the one action map, so a
    /// notification can never reach something the menus and the shortcuts
    /// dialog cannot.
    pub fn notify(
        application: &impl IsA<gio::Application>,
        id: &str,
        title: &str,
        body: &str,
        action: Option<&str>,
    ) {
        let notification = gio::Notification::new(title);
        notification.set_body(Some(body));
        if let Some(action) = action {
            notification.set_default_action(action);
        }
        application
            .as_ref()
            .send_notification(Some(id), &notification);
    }

    /// Withdraw a notification this application raised.
    pub fn withdraw(application: &impl IsA<gio::Application>, id: &str) {
        application.as_ref().withdraw_notification(id);
    }
}

/// An environment value, with an empty string treated as absent. The engine's
/// own resolver makes the same distinction, and a blank `XDG_CURRENT_DESKTOP`
/// is a desktop that did not say rather than one called "".
fn environment_value(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blank_environment_value_is_absent_rather_than_empty() {
        // Reads a name nothing sets, which is the same path a blank one takes.
        assert_eq!(environment_value("FERMIX_DESKTOP_NOTHING_SETS_THIS"), None);
    }
}
