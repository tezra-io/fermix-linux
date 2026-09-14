//! The application's own presentation state, under `$XDG_STATE_HOME`.
//!
//! Window geometry, the sidebar preference and the last Settings pane. Nothing
//! about the engine is cached here: configuration belongs to the daemon and the
//! home binding belongs to the command line.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The directory name under `$XDG_STATE_HOME`.
pub const STATE_DIRECTORY: &str = "fermix-desktop";
/// The file inside it.
pub const STATE_FILE: &str = "state.json";
/// Points the state file somewhere else, which is how tests keep off the host.
pub const STATE_DIRECTORY_OVERRIDE: &str = "FERMIX_DESKTOP_STATE_DIR";

/// What survives a launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    pub sidebar_visible: bool,
    /// Only the selected pane persists across launches, never the scroll
    /// position, which belongs to the process.
    pub last_pane: Option<String>,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            width: crate::metrics::WINDOW_DEFAULT_WIDTH,
            height: crate::metrics::WINDOW_DEFAULT_HEIGHT,
            maximized: false,
            sidebar_visible: true,
            last_pane: None,
        }
    }
}

impl WindowState {
    /// A geometry inside the window's declared minimum, whatever was recorded.
    /// A state file from a different build, or a hand edit, cannot produce a
    /// window too small to use.
    pub fn clamped(self) -> Self {
        Self {
            width: self.width.max(crate::metrics::WINDOW_MINIMUM_WIDTH),
            height: self.height.max(crate::metrics::WINDOW_MINIMUM_HEIGHT),
            ..self
        }
    }
}

/// Where the state file lives for this process.
pub fn state_directory() -> PathBuf {
    if let Some(override_path) = non_empty_env(STATE_DIRECTORY_OVERRIDE) {
        return PathBuf::from(override_path);
    }
    match non_empty_env("XDG_STATE_HOME") {
        Some(base) => PathBuf::from(base).join(STATE_DIRECTORY),
        None => home_directory().join(".local/state").join(STATE_DIRECTORY),
    }
}

/// Read the recorded state for this process.
pub fn load() -> WindowState {
    load_from(&state_directory())
}

/// Write the state for this process.
pub fn save(state: &WindowState) -> std::io::Result<()> {
    save_to(&state_directory(), state)
}

/// Read the recorded state out of one directory.
///
/// An absent, unreadable or unparseable file is the default state, reported
/// once to the journal rather than swallowed: this is presentation, and
/// refusing to open a window because a remembered width did not parse would be
/// the wrong trade.
pub fn load_from(directory: &Path) -> WindowState {
    let path = directory.join(STATE_FILE);
    if !path.exists() {
        return WindowState::default();
    }

    match read_from(&path) {
        Ok(state) => state.clamped(),
        Err(reason) => {
            gtk4::glib::g_warning!(
                "fermix-desktop",
                "ignoring unreadable window state at {}: {reason}",
                path.display()
            );
            WindowState::default()
        }
    }
}

/// Write the state into one directory, creating it if it is not there yet.
pub fn save_to(directory: &Path, state: &WindowState) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;

    let encoded = serde_json::to_vec_pretty(state)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;

    // Written beside the target and renamed over it, so a crash between the two
    // leaves the previous state rather than half of this one.
    let target = directory.join(STATE_FILE);
    let temporary = directory.join(format!("{STATE_FILE}.new"));
    std::fs::write(&temporary, encoded)?;
    std::fs::rename(&temporary, &target)
}

fn read_from(path: &Path) -> Result<WindowState, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

fn non_empty_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

fn home_directory() -> PathBuf {
    match non_empty_env("HOME") {
        Some(home) => PathBuf::from(home),
        None => PathBuf::from("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_geometry_below_the_minimum_is_brought_back_up_to_it() {
        let state = WindowState {
            width: 10,
            height: 10,
            ..WindowState::default()
        }
        .clamped();
        assert_eq!(state.width, crate::metrics::WINDOW_MINIMUM_WIDTH);
        assert_eq!(state.height, crate::metrics::WINDOW_MINIMUM_HEIGHT);
    }

    #[test]
    fn the_default_is_the_declared_window_size() {
        let state = WindowState::default();
        assert_eq!(state.width, crate::metrics::WINDOW_DEFAULT_WIDTH);
        assert_eq!(state.height, crate::metrics::WINDOW_DEFAULT_HEIGHT);
        assert!(state.sidebar_visible);
        assert_eq!(state.last_pane, None);
    }

    #[test]
    fn state_round_trips_through_json() {
        let state = WindowState {
            width: 900,
            height: 700,
            maximized: true,
            sidebar_visible: false,
            last_pane: Some("providers".to_string()),
        };

        let encoded = serde_json::to_vec(&state).expect("encodes");
        let decoded: WindowState = serde_json::from_slice(&encoded).expect("decodes");

        assert_eq!(decoded, state);
    }

    #[test]
    fn state_round_trips_through_a_directory() {
        let directory = crate::testing::TempDirectory::new("state-round-trip");
        let state = WindowState {
            width: 901,
            height: 701,
            maximized: true,
            sidebar_visible: false,
            last_pane: Some("memory".to_string()),
        };

        save_to(directory.path(), &state).expect("saves");

        assert_eq!(load_from(directory.path()), state);
    }

    #[test]
    fn an_empty_directory_reads_as_the_default_state() {
        let directory = crate::testing::TempDirectory::new("state-absent");
        assert_eq!(load_from(directory.path()), WindowState::default());
    }

    #[test]
    fn an_unparseable_state_file_reads_as_the_default_state() {
        let directory = crate::testing::TempDirectory::new("state-corrupt");
        std::fs::write(directory.path().join(STATE_FILE), b"{ not json").expect("writes");

        assert_eq!(load_from(directory.path()), WindowState::default());
    }
}
