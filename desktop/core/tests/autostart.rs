//! "Open at login" outside a sandbox: where the window keeps its XDG autostart
//! entry, what the entry says, and how the file reads back. Files are written
//! only under a temporary `XDG_CONFIG_HOME`.

use fermix_client::autostart::{
    entry_opens, entry_path, entry_text, read_entry, remove_entry, write_entry, ENTRY_NAME,
};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const HOME: &str = "/home/someone";

/// What the Background portal wrote for the Flatpak, on the owner's machine.
const FLATPAK_ENTRY: &str = "[Desktop Entry]\nType=Application\nName=io.tezra.Fermix\n\
                             Exec=flatpak run --command=fermix-desktop io.tezra.Fermix\n\
                             X-Flatpak=io.tezra.Fermix\n";

fn under(config: &tempfile::TempDir) -> PathBuf {
    entry_path(Some(config.path().as_os_str()), Path::new(HOME))
}

#[test]
fn the_entry_sits_in_the_autostart_directory_of_xdg_config_home() {
    assert_eq!(
        entry_path(Some(OsStr::new("/srv/config")), Path::new(HOME)),
        Path::new("/srv/config/autostart/io.tezra.Fermix.desktop")
    );
    assert_eq!(ENTRY_NAME, "io.tezra.Fermix.desktop");
}

#[test]
fn without_xdg_config_home_the_entry_sits_under_dot_config() {
    let expected = Path::new("/home/someone/.config/autostart/io.tezra.Fermix.desktop");
    assert_eq!(entry_path(None, Path::new(HOME)), expected);
    // The base directory specification treats an empty or relative value as unset.
    assert_eq!(entry_path(Some(OsStr::new("")), Path::new(HOME)), expected);
    assert_eq!(
        entry_path(Some(OsStr::new("config")), Path::new(HOME)),
        expected
    );
}

#[test]
#[should_panic(expected = "absolute")]
fn a_relative_home_is_refused() {
    entry_path(None, Path::new("someone"));
}

#[test]
#[should_panic(expected = "not an autostart entry")]
fn no_other_file_is_ever_removed() {
    let _ = remove_entry(Path::new("/home/someone/.config/autostart/other.desktop"));
}

#[test]
fn the_entry_starts_fermix_in_the_tray_as_the_portal_was_asked_to() {
    assert_eq!(
        entry_text(),
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Fermix\n\
         Exec=fermix-desktop --background\n\
         Icon=io.tezra.Fermix\n\
         Terminal=false\n"
    );
    assert!(entry_opens(entry_text()));
}

#[test]
fn an_entry_turned_off_in_place_does_not_open_fermix() {
    let hidden = format!("{}Hidden=true\n", entry_text());
    assert!(!entry_opens(&hidden));
    let gnome_off = format!("{}X-GNOME-Autostart-enabled = false\n", entry_text());
    assert!(!entry_opens(&gnome_off));
    let shown = format!(
        "{}Hidden=false\nX-GNOME-Autostart-enabled=true\n",
        entry_text()
    );
    assert!(entry_opens(&shown));
    assert!(entry_opens(FLATPAK_ENTRY));
}

#[test]
fn turning_on_writes_the_entry_and_turning_off_removes_it() {
    let config = tempfile::tempdir().unwrap();
    let path = under(&config);
    // Not even the autostart directory exists yet.
    assert!(!read_entry(&path).unwrap());
    write_entry(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), entry_text());
    assert!(read_entry(&path).unwrap());
    remove_entry(&path).unwrap();
    assert!(!path.exists());
    assert!(!read_entry(&path).unwrap());
    // An entry already gone is already off.
    remove_entry(&path).unwrap();
}

#[test]
fn turning_on_replaces_the_entry_the_flatpak_left() {
    let config = tempfile::tempdir().unwrap();
    let path = under(&config);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, FLATPAK_ENTRY).unwrap();
    assert!(read_entry(&path).unwrap());
    write_entry(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), entry_text());
}

#[test]
fn an_entry_that_cannot_be_read_is_an_error_rather_than_off() {
    let config = tempfile::tempdir().unwrap();
    let path = under(&config);
    fs::create_dir_all(&path).unwrap();
    assert!(read_entry(&path).is_err());
}
