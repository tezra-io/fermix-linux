//! What the updater remembers between runs, in `$XDG_STATE_HOME/fermix-desktop`
//! (the app keeps no other state of its own): the switch, the newest feed
//! accepted, the last check and the skipped version, written atomically. And
//! the downloads folder, private to this account and emptied at each launch.

use fermix_client::update::store::{
    clear_downloads, download_dir, load, prepare_downloads, save, state_path, Checked, LastCheck,
    Remembered,
};
use fermix_client::update::version::Version;
use std::ffi::OsStr;
use std::fs::Permissions;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;

fn mode(path: &Path) -> u32 {
    std::fs::symlink_metadata(path)
        .unwrap()
        .permissions()
        .mode()
        & 0o7777
}

#[test]
fn state_lives_under_xdg_state_home_or_local_state() {
    let home = Path::new("/home/ada");
    assert_eq!(
        state_path(Some(OsStr::new("/var/state")), home),
        Path::new("/var/state/fermix-desktop/updates.json")
    );
    for ignored in ["", "relative/state"] {
        assert_eq!(
            state_path(Some(OsStr::new(ignored)), home),
            Path::new("/home/ada/.local/state/fermix-desktop/updates.json"),
            "{ignored:?} is ignored, as the XDG specification says"
        );
    }
    assert_eq!(
        state_path(None, home),
        Path::new("/home/ada/.local/state/fermix-desktop/updates.json")
    );
}

#[test]
fn downloads_live_under_the_cache() {
    let home = Path::new("/home/ada");
    assert_eq!(
        download_dir(None, home),
        Path::new("/home/ada/.cache/fermix-desktop/updates")
    );
    assert_eq!(
        download_dir(Some(OsStr::new("/fast/cache")), home),
        Path::new("/fast/cache/fermix-desktop/updates")
    );
    assert_eq!(
        download_dir(Some(OsStr::new("cache")), home),
        Path::new("/home/ada/.cache/fermix-desktop/updates")
    );
}

#[test]
fn nothing_remembered_yet_checks_automatically() {
    let dir = tempfile::tempdir().unwrap();
    let remembered = load(&dir.path().join("absent/updates.json")).unwrap();
    assert_eq!(remembered, Remembered::default());
    assert!(remembered.automatic, "on by default");
    assert_eq!(remembered.accepted_generated_at, None);
    assert_eq!(remembered.last_check, None);
    assert_eq!(remembered.skipped, None);
}

#[test]
fn what_is_saved_is_what_is_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fermix-desktop/updates.json");
    let remembered = Remembered {
        automatic: false,
        accepted_generated_at: Some(1_792_638_000),
        last_check: Some(LastCheck {
            at: 1_792_640_000,
            result: Checked::Available {
                version: Version::parse("0.4.0").unwrap(),
                critical: true,
            },
        }),
        skipped: Some(Version::parse("0.3.9").unwrap()),
    };
    save(&path, &remembered).unwrap();
    assert_eq!(load(&path).unwrap(), remembered);
    for result in [
        Checked::UpToDate,
        Checked::Development,
        Checked::Stranded {
            latest: Version::parse("1.0.0").unwrap(),
        },
        Checked::Failed {
            sentence: "fermix.ai did not answer.".into(),
        },
    ] {
        let again = Remembered {
            last_check: Some(LastCheck { at: 5, result }),
            ..Remembered::default()
        };
        save(&path, &again).unwrap();
        assert_eq!(load(&path).unwrap(), again);
    }
}

#[test]
fn a_save_replaces_the_file_in_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("updates.json");
    std::fs::write(&path, "{\"automatic\": true}").unwrap();
    let off = Remembered {
        automatic: false,
        ..Remembered::default()
    };
    save(&path, &off).unwrap();
    assert_eq!(load(&path).unwrap(), off);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["updates.json"], "no temporary file is left");
}

#[test]
fn a_damaged_file_is_an_error_to_log_not_a_reset() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("updates.json");
    std::fs::write(&path, "{\"automatic\": ").unwrap();
    assert!(load(&path).is_err());
    std::fs::write(&path, "{\"skipped\": \"0.4\"}").unwrap();
    assert!(load(&path).is_err(), "a version that is not X.Y.Z");
}

#[test]
fn fields_a_later_build_adds_are_read_past() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("updates.json");
    std::fs::write(&path, "{\"automatic\": false, \"channel\": \"beta\"}").unwrap();
    assert!(!load(&path).unwrap().automatic);
}

#[test]
fn launch_empties_the_downloads_folder() {
    let dir = tempfile::tempdir().unwrap();
    let updates = dir.path().join("updates");
    assert_eq!(clear_downloads(&updates).unwrap(), 0, "no folder yet");
    prepare_downloads(&updates).unwrap();
    std::fs::write(updates.join("fermix-desktop_0.4.0_amd64.deb"), "x").unwrap();
    std::fs::write(updates.join("fermix-desktop_0.4.0_amd64.deb.partial"), "x").unwrap();
    assert_eq!(clear_downloads(&updates).unwrap(), 2);
    assert_eq!(std::fs::read_dir(&updates).unwrap().count(), 0);
}

#[test]
fn the_downloads_folder_is_made_private_to_this_account() {
    let dir = tempfile::tempdir().unwrap();
    let updates = dir.path().join("cache/fermix-desktop/updates");
    prepare_downloads(&updates).unwrap();
    assert_eq!(mode(&updates), 0o700);
    prepare_downloads(&updates).unwrap();
    assert_eq!(
        mode(&updates),
        0o700,
        "one already there and private is used"
    );
}

#[test]
fn a_downloads_folder_open_to_others_a_link_or_a_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let open = dir.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, Permissions::from_mode(0o755)).unwrap();
    let refused = prepare_downloads(&open).unwrap_err().to_string();
    assert!(refused.contains("755"), "{refused}");
    assert_eq!(
        mode(&open),
        0o755,
        "a folder it did not make is left as it is"
    );
    let private = dir.path().join("private");
    std::fs::create_dir(&private).unwrap();
    std::fs::set_permissions(&private, Permissions::from_mode(0o700)).unwrap();
    let link = dir.path().join("link");
    symlink(&private, &link).unwrap();
    let refused = prepare_downloads(&link).unwrap_err().to_string();
    assert!(refused.contains("link"), "{refused}");
    let file = dir.path().join("file");
    std::fs::write(&file, "x").unwrap();
    assert!(prepare_downloads(&file).is_err());
}

#[test]
fn launch_never_empties_a_folder_through_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("Documents");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::set_permissions(&elsewhere, Permissions::from_mode(0o700)).unwrap();
    std::fs::write(elsewhere.join("letter.odt"), "the person's own").unwrap();
    let updates = dir.path().join("updates");
    symlink(&elsewhere, &updates).unwrap();
    assert!(clear_downloads(&updates).is_err());
    assert_eq!(
        std::fs::read(elsewhere.join("letter.odt")).unwrap(),
        b"the person's own"
    );
}
