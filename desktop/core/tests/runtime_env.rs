//! The window run from the `fermix-desktop` package: how it knows, the one
//! variable it sets while GLib reads its schemas, and what puts the
//! environment back as the window found it.

use fermix_client::runtime_env::{
    packaged_settings, restore_plan, runs_from_package, schema_dir, Environment, Restore,
    PACKAGED_BIN, PRIVATE_ICONS, PRIVATE_SCHEMAS, SCHEMA_DIR,
};
use std::ffi::{OsStr, OsString};
use std::path::Path;

fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

fn pair(name: &str, value: &str) -> (OsString, OsString) {
    (name.into(), value.into())
}

fn private() -> &'static Path {
    Path::new(PRIVATE_SCHEMAS)
}

/// The environment after the window set `set` over `entry` and then applied
/// the plan, as `main` does.
fn after_restore(entry: &Environment, set: &[(OsString, OsString)]) -> Environment {
    let mut now = entry.clone();
    now.extend(set.iter().cloned());
    for change in restore_plan(entry, set) {
        match change {
            Restore::Set(name, value) => now.insert(name, value),
            Restore::Unset(name) => now.remove(&name),
        };
    }
    now
}

#[test]
fn the_packaged_executable_is_the_one_under_the_private_prefix() {
    assert!(runs_from_package(Path::new(
        "/usr/lib/fermix-desktop/bin/fermix-desktop"
    )));
    // A window still running after an upgrade replaced its file.
    assert!(runs_from_package(Path::new(
        "/usr/lib/fermix-desktop/bin/fermix-desktop (deleted)"
    )));
}

#[test]
fn the_flatpak_and_a_development_build_are_not_the_package() {
    for elsewhere in [
        "/app/bin/fermix-desktop",
        "/home/someone/src/fermix-linux/desktop/target-sdk/debug/fermix-desktop",
        // The link itself never shows: the executable's path is resolved.
        "/usr/bin/fermix-desktop",
        "/usr/lib/fermix-desktop/binaries/fermix-desktop",
        "/usr/lib/fermix-desktop/lib/fermix-desktop",
    ] {
        assert!(!runs_from_package(Path::new(elsewhere)), "{elsewhere}");
    }
}

#[test]
#[should_panic(expected = "absolute")]
fn a_relative_executable_path_is_refused() {
    runs_from_package(Path::new("bin/fermix-desktop"));
}

#[test]
fn the_private_schemas_come_before_the_entry_value() {
    assert_eq!(
        schema_dir(private(), Some(OsStr::new("/opt/schemas:/srv/schemas"))),
        "/usr/lib/fermix-desktop/share/glib-2.0/schemas:/opt/schemas:/srv/schemas"
    );
    assert_eq!(
        schema_dir(Path::new("/tmp/test-schemas"), Some(OsStr::new("/opt"))),
        "/tmp/test-schemas:/opt"
    );
}

#[test]
fn with_no_entry_value_the_private_schemas_stand_alone() {
    assert_eq!(schema_dir(private(), None), PRIVATE_SCHEMAS);
    // GLib skips an empty value, and an empty element would name no directory.
    assert_eq!(schema_dir(private(), Some(OsStr::new(""))), PRIVATE_SCHEMAS);
}

#[test]
#[should_panic(expected = "absolute")]
fn a_relative_schema_directory_is_refused() {
    schema_dir(Path::new("schemas"), None);
}

#[test]
fn the_schema_directory_is_the_only_variable_the_package_sets() {
    let entry = environment(&[
        (SCHEMA_DIR, "/opt/schemas"),
        ("XDG_DATA_DIRS", "/usr/share"),
        ("PATH", "/usr/bin"),
    ]);
    assert_eq!(
        packaged_settings(&entry, private()),
        vec![pair(
            SCHEMA_DIR,
            "/usr/lib/fermix-desktop/share/glib-2.0/schemas:/opt/schemas"
        )]
    );
    let bare = environment(&[("PATH", "/usr/bin")]);
    assert_eq!(
        packaged_settings(&bare, private()),
        vec![pair(SCHEMA_DIR, PRIVATE_SCHEMAS)]
    );
}

#[test]
fn a_window_that_changed_nothing_restores_nothing() {
    let entry = environment(&[(SCHEMA_DIR, "/opt/schemas"), ("PATH", "/usr/bin")]);
    assert_eq!(restore_plan(&entry, &[]), vec![]);
    // Setting a variable to the value it had changes nothing either.
    assert_eq!(restore_plan(&entry, &[pair("PATH", "/usr/bin")]), vec![]);
}

#[test]
fn a_changed_variable_goes_back_to_its_entry_value() {
    let entry = environment(&[(SCHEMA_DIR, "/opt/schemas"), ("PATH", "/usr/bin")]);
    let set = packaged_settings(&entry, private());
    assert_eq!(
        restore_plan(&entry, &set),
        vec![Restore::Set(SCHEMA_DIR.into(), "/opt/schemas".into())]
    );
    assert_eq!(after_restore(&entry, &set), entry);
}

#[test]
fn a_variable_absent_at_entry_is_unset_rather_than_emptied() {
    let entry = environment(&[("PATH", "/usr/bin")]);
    let set = packaged_settings(&entry, private());
    assert_eq!(
        restore_plan(&entry, &set),
        vec![Restore::Unset(SCHEMA_DIR.into())]
    );
    assert_eq!(after_restore(&entry, &set), entry);
}

#[test]
fn an_empty_entry_value_comes_back_empty() {
    let entry = environment(&[(SCHEMA_DIR, ""), ("PATH", "/usr/bin")]);
    let set = packaged_settings(&entry, private());
    assert_eq!(set, vec![pair(SCHEMA_DIR, PRIVATE_SCHEMAS)]);
    assert_eq!(
        restore_plan(&entry, &set),
        vec![Restore::Set(SCHEMA_DIR.into(), "".into())]
    );
    assert_eq!(after_restore(&entry, &set), entry);
}

#[test]
#[should_panic(expected = "not a variable name")]
fn a_name_no_environment_can_hold_is_refused() {
    restore_plan(&Environment::new(), &[pair("A=B", "c")]);
}

#[test]
fn the_private_paths_are_the_ones_the_package_installs() {
    assert_eq!(PACKAGED_BIN, "/usr/lib/fermix-desktop/bin");
    assert_eq!(
        PRIVATE_SCHEMAS,
        "/usr/lib/fermix-desktop/share/glib-2.0/schemas"
    );
    assert_eq!(PRIVATE_ICONS, "/usr/lib/fermix-desktop/share/icons");
    assert_eq!(SCHEMA_DIR, "GSETTINGS_SCHEMA_DIR");
}
