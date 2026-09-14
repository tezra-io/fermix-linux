//! The XDG autostart entry.
//!
//! "Open at login" is independent of the background service, as it is on macOS:
//! the service keeps Fermix answering whether or not this window ever opens.
//! The durable artifact is one file in the person's own home, which is what the
//! Permissions ledger says about it.

use std::path::{Path, PathBuf};

/// The entry's basename, which is the application identity.
pub const ENTRY_NAME: &str = "io.tezra.Fermix.desktop";
/// Points the autostart directory somewhere else, which is how tests keep off
/// the host.
pub const DIRECTORY_OVERRIDE: &str = "FERMIX_DESKTOP_AUTOSTART_DIR";

/// Where the entry lives for this process.
pub fn autostart_directory() -> PathBuf {
    if let Some(directory) = non_empty_env(DIRECTORY_OVERRIDE) {
        return PathBuf::from(directory);
    }
    match non_empty_env("XDG_CONFIG_HOME") {
        Some(base) => PathBuf::from(base).join("autostart"),
        None => home_directory().join(".config/autostart"),
    }
}

/// Whether Fermix opens at login, read from the entry itself.
///
/// An entry carrying `Hidden=true` is off. That spelling exists so a desktop's
/// own autostart settings and this switch agree about a file the person may
/// have turned off there.
pub fn is_enabled_in(directory: &Path) -> bool {
    let path = directory.join(ENTRY_NAME);
    match std::fs::read_to_string(&path) {
        Ok(contents) => !contents.lines().any(is_hidden_line),
        Err(_) => false,
    }
}

/// Turn the entry on or off in one directory.
///
/// Turning it off rewrites the entry with `Hidden=true` rather than deleting
/// it, because a desktop that lists autostart entries should keep showing this
/// one as something the person can turn back on.
pub fn set_enabled_in(directory: &Path, enabled: bool) -> std::io::Result<()> {
    let path = directory.join(ENTRY_NAME);

    if !enabled && !path.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(directory)?;
    std::fs::write(&path, entry_contents(enabled))
}

/// Whether Fermix opens at login.
pub fn is_enabled() -> bool {
    is_enabled_in(&autostart_directory())
}

/// Turn the entry on or off.
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    set_enabled_in(&autostart_directory(), enabled)
}

/// The entry's bytes.
///
/// `Name` is the product name and is not a copy key: a desktop entry's `Name`
/// is an identity string the launcher indexes, not a sentence the interface
/// renders, and it is the same string in all six places the identity appears.
fn entry_contents(enabled: bool) -> String {
    let hidden = if enabled { "" } else { "Hidden=true\n" };
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Fermix\n\
         Exec=fermix-desktop --gapplication-service\n\
         Icon=io.tezra.Fermix\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         {hidden}"
    )
}

fn is_hidden_line(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case("hidden=true")
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
    use crate::testing::TempDirectory;

    #[test]
    fn no_entry_means_fermix_does_not_open_at_login() {
        let directory = TempDirectory::new("autostart-absent");
        assert!(!is_enabled_in(directory.path()));
    }

    #[test]
    fn turning_it_on_writes_the_entry() {
        let directory = TempDirectory::new("autostart-on");
        set_enabled_in(directory.path(), true).expect("writes");

        assert!(is_enabled_in(directory.path()));
        let contents = std::fs::read_to_string(directory.join(ENTRY_NAME)).expect("reads");
        assert!(contents.contains("Exec=fermix-desktop --gapplication-service"));
        assert!(!contents.contains("Hidden=true"));
    }

    #[test]
    fn turning_it_off_hides_the_entry_rather_than_deleting_it() {
        let directory = TempDirectory::new("autostart-off");
        set_enabled_in(directory.path(), true).expect("writes");
        set_enabled_in(directory.path(), false).expect("rewrites");

        assert!(directory.join(ENTRY_NAME).exists());
        assert!(!is_enabled_in(directory.path()));
    }

    #[test]
    fn turning_it_off_when_there_was_never_an_entry_writes_nothing() {
        let directory = TempDirectory::new("autostart-never");
        set_enabled_in(directory.path(), false).expect("does nothing");

        assert!(!directory.join(ENTRY_NAME).exists());
    }

    #[test]
    fn a_hidden_entry_turns_back_on_in_place() {
        let directory = TempDirectory::new("autostart-again");
        set_enabled_in(directory.path(), true).expect("writes");
        set_enabled_in(directory.path(), false).expect("hides");
        set_enabled_in(directory.path(), true).expect("shows");

        assert!(is_enabled_in(directory.path()));
    }
}
