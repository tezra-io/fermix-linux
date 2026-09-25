//! The window's handle on the daemon. Management calls are blocking, so each one
//! runs on a GIO worker thread and the UI awaits it; the socket is opened and
//! closed inside the call. Starting a stopped daemon goes through `systemd`.

use fermix_client::management::{CallError, Management};
use gtk::gio;
use gtk::glib;
use std::path::PathBuf;
use std::time::Duration;

/// How long one management exchange may take before the app calls the daemon unresponsive.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Daemon {
    management: Management,
}

impl Daemon {
    pub fn new() -> Self {
        let socket = fermix_home().join("daemon.sock");
        Daemon {
            management: Management::new(socket, CALL_TIMEOUT),
        }
    }

    /// Runs one blocking exchange off the main thread.
    pub async fn call<T, F>(&self, f: F) -> Result<T, CallError>
    where
        T: Send + 'static,
        F: FnOnce(&Management) -> Result<T, CallError> + Send + 'static,
    {
        let management = self.management.clone();
        match gio::spawn_blocking(move || f(&management)).await {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}

/// `$FERMIX_HOME`, else `~/.fermix` — the same default the daemon uses.
pub fn fermix_home() -> PathBuf {
    match std::env::var_os("FERMIX_HOME") {
        Some(home) if !home.is_empty() => PathBuf::from(home),
        _ => glib::home_dir().join(".fermix"),
    }
}
