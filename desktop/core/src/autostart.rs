//! "Open at login" when the window runs outside a sandbox (single_package.md
//! §4). Inside the Flatpak the Background portal writes and deletes the XDG
//! autostart entry; outside one there is no portal to ask, so the window keeps
//! the same file itself, and the file is the truth for the switch.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

/// The entry's file name: the app id, as the portal names it.
pub const ENTRY_NAME: &str = "io.tezra.Fermix.desktop";

/// Where the entry lives: `$XDG_CONFIG_HOME/autostart`, or `~/.config/autostart`
/// when the variable is unset, empty or relative, which the XDG base directory
/// specification says to ignore.
pub fn entry_path(config_home: Option<&OsStr>, home: &Path) -> PathBuf {
    assert!(
        home.is_absolute(),
        "the home directory is absolute: {home:?}"
    );
    let config = match config_home.map(Path::new) {
        Some(dir) if dir.is_absolute() => dir.to_path_buf(),
        _ => home.join(".config"),
    };
    config.join("autostart").join(ENTRY_NAME)
}

/// The entry. At login Fermix starts in the tray, with the command line the
/// Background portal is given.
pub fn entry_text() -> &'static str {
    "[Desktop Entry]\n\
     Type=Application\n\
     Name=Fermix\n\
     Exec=fermix-desktop --background\n\
     Icon=io.tezra.Fermix\n\
     Terminal=false\n"
}

/// Whether the session opens Fermix for an entry with this text. `Hidden=true`,
/// and GNOME's `X-GNOME-Autostart-enabled=false`, turn an entry off in place.
pub fn entry_opens(text: &str) -> bool {
    !text.lines().any(|line| {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        matches!(
            (key.trim(), value.trim()),
            ("Hidden", "true") | ("X-GNOME-Autostart-enabled", "false")
        )
    })
}

/// Whether the entry at `path` opens Fermix at login. No entry is off.
pub fn read_entry(path: &Path) -> io::Result<bool> {
    guard(path);
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(entry_opens(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Writes the entry, and the autostart directory when there is none.
pub fn write_entry(path: &Path) -> io::Result<()> {
    guard(path);
    let dir = path.parent().expect("the entry sits in a directory");
    std::fs::create_dir_all(dir)?;
    std::fs::write(path, entry_text())
}

/// Removes the entry. One already gone is off already.
pub fn remove_entry(path: &Path) -> io::Result<()> {
    guard(path);
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// These functions touch the entry and no other file.
fn guard(path: &Path) {
    assert!(
        path.is_absolute() && path.file_name() == Some(OsStr::new(ENTRY_NAME)),
        "not an autostart entry for Fermix: {path:?}"
    );
}
