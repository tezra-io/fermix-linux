//! The window run from the `fermix-desktop` package: how it knows, what it
//! sets while the private libraries read their configuration, and what puts
//! the environment back as the window found it.

use fermix_client::runtime_env::{
    gst_registry, packaged_settings, restore_plan, runs_from_package, schema_dir, Environment,
    PrivatePaths, Restore, Setting, PACKAGED_BIN, PRIVATE_GST_PLUGINS, PRIVATE_ICONS,
    PRIVATE_SCHEMAS, SCHEMA_DIR,
};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

const HOME: &str = "/home/someone";

fn environment(pairs: &[(&str, &str)]) -> Environment {
    pairs
        .iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
}

fn set(name: &str, value: &str) -> Setting {
    (name.into(), Some(value.into()))
}

fn unset(name: &str) -> Setting {
    (name.into(), None)
}

fn private() -> &'static Path {
    Path::new(PRIVATE_SCHEMAS)
}

fn paths() -> PrivatePaths {
    PrivatePaths {
        schemas: PRIVATE_SCHEMAS.into(),
        gst_plugins: PRIVATE_GST_PLUGINS.into(),
        gst_registry: "/home/someone/.cache/fermix-desktop/gstreamer-1.0/registry.bin".into(),
    }
}

/// The environment after the window applied `settings` over `entry` and then
/// the plan, as `main` does.
fn after_restore(entry: &Environment, settings: &[Setting]) -> Environment {
    let mut now = entry.clone();
    for (name, value) in settings {
        match value {
            Some(value) => now.insert(name.clone(), value.clone()),
            None => now.remove(name),
        };
    }
    for change in restore_plan(entry, settings) {
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
fn the_gstreamer_registry_is_the_windows_own_under_xdg_cache_home() {
    let file = format!("registry.{}.bin", std::env::consts::ARCH);
    assert_eq!(
        gst_registry(Some(OsStr::new("/srv/cache")), Path::new(HOME)),
        PathBuf::from("/srv/cache/fermix-desktop/gstreamer-1.0").join(&file)
    );
    let fallback = PathBuf::from("/home/someone/.cache/fermix-desktop/gstreamer-1.0").join(&file);
    assert_eq!(gst_registry(None, Path::new(HOME)), fallback);
    // The base directory specification treats an empty or relative value as unset.
    assert_eq!(
        gst_registry(Some(OsStr::new("")), Path::new(HOME)),
        fallback
    );
    assert_eq!(
        gst_registry(Some(OsStr::new("cache")), Path::new(HOME)),
        fallback
    );
}

#[test]
#[should_panic(expected = "absolute")]
fn a_relative_home_for_the_registry_is_refused() {
    gst_registry(None, Path::new("someone"));
}

#[test]
fn the_package_points_each_library_at_the_private_runtime_alone() {
    let entry = environment(&[(SCHEMA_DIR, "/opt/schemas"), ("PATH", "/usr/bin")]);
    assert_eq!(
        packaged_settings(&entry, &paths()),
        vec![
            set(
                SCHEMA_DIR,
                "/usr/lib/fermix-desktop/share/glib-2.0/schemas:/opt/schemas"
            ),
            set("GIO_EXTRA_MODULES", ""),
            unset("GIO_MODULE_DIR"),
            unset("GDK_PIXBUF_MODULE_FILE"),
            set(
                "GST_REGISTRY_1_0",
                "/home/someone/.cache/fermix-desktop/gstreamer-1.0/registry.bin"
            ),
            set(
                "GST_PLUGIN_SYSTEM_PATH_1_0",
                "/usr/lib/fermix-desktop/lib/gstreamer-1.0"
            ),
            set("GST_PLUGIN_PATH_1_0", ""),
            set("GST_PLUGIN_SCANNER_1_0", ""),
        ]
    );
}

#[test]
fn a_window_that_changed_nothing_restores_nothing() {
    let entry = environment(&[(SCHEMA_DIR, "/opt/schemas"), ("PATH", "/usr/bin")]);
    assert_eq!(restore_plan(&entry, &[]), vec![]);
    // A value the variable already had, or an absence it already had, changes nothing.
    assert_eq!(restore_plan(&entry, &[set("PATH", "/usr/bin")]), vec![]);
    assert_eq!(restore_plan(&entry, &[unset("GIO_MODULE_DIR")]), vec![]);
}

#[test]
fn a_changed_variable_goes_back_to_its_entry_value() {
    let entry = environment(&[(SCHEMA_DIR, "/opt/schemas"), ("PATH", "/usr/bin")]);
    let settings = [set(SCHEMA_DIR, "/private:/opt/schemas")];
    assert_eq!(
        restore_plan(&entry, &settings),
        vec![Restore::Set(SCHEMA_DIR.into(), "/opt/schemas".into())]
    );
    assert_eq!(after_restore(&entry, &settings), entry);
}

#[test]
fn a_variable_absent_at_entry_is_unset_rather_than_emptied() {
    let entry = environment(&[("PATH", "/usr/bin")]);
    let settings = [set("GIO_EXTRA_MODULES", "")];
    assert_eq!(
        restore_plan(&entry, &settings),
        vec![Restore::Unset("GIO_EXTRA_MODULES".into())]
    );
    assert_eq!(after_restore(&entry, &settings), entry);
}

#[test]
fn a_variable_unset_for_the_start_comes_back_with_its_entry_value() {
    let entry = environment(&[("GIO_MODULE_DIR", "/snap/gio/modules")]);
    let settings = [unset("GIO_MODULE_DIR")];
    assert_eq!(
        restore_plan(&entry, &settings),
        vec![Restore::Set(
            "GIO_MODULE_DIR".into(),
            "/snap/gio/modules".into()
        )]
    );
    assert_eq!(after_restore(&entry, &settings), entry);
}

#[test]
fn an_empty_entry_value_comes_back_empty() {
    let entry = environment(&[(SCHEMA_DIR, ""), ("PATH", "/usr/bin")]);
    let settings = packaged_settings(&entry, &paths());
    assert_eq!(settings[0], set(SCHEMA_DIR, PRIVATE_SCHEMAS));
    assert!(restore_plan(&entry, &settings).contains(&Restore::Set(SCHEMA_DIR.into(), "".into())));
    assert_eq!(after_restore(&entry, &settings), entry);
}

#[test]
fn whatever_the_host_set_comes_back_after_the_start() {
    let host = environment(&[
        (SCHEMA_DIR, "/snap/schemas"),
        ("GIO_EXTRA_MODULES", "/snap/gio"),
        ("GIO_MODULE_DIR", "/snap/gio/modules"),
        ("GDK_PIXBUF_MODULE_FILE", "/snap/loaders.cache"),
        ("GST_REGISTRY_1_0", "/tmp/registry.bin"),
        ("GST_PLUGIN_SYSTEM_PATH_1_0", "/usr/lib/gstreamer-1.0"),
        ("GST_PLUGIN_PATH_1_0", "/home/someone/plugins"),
        ("GST_PLUGIN_SCANNER_1_0", "/usr/libexec/gst-plugin-scanner"),
        ("GST_PLUGIN_PATH", "/opt/plugins"),
    ]);
    let bare = environment(&[("PATH", "/usr/bin")]);
    for entry in [host, bare] {
        let settings = packaged_settings(&entry, &paths());
        assert_eq!(after_restore(&entry, &settings), entry);
    }
}

#[test]
#[should_panic(expected = "not a variable name")]
fn a_name_no_environment_can_hold_is_refused() {
    restore_plan(&Environment::new(), &[set("A=B", "c")]);
}

#[test]
fn the_private_paths_are_the_ones_the_package_installs() {
    assert_eq!(PACKAGED_BIN, "/usr/lib/fermix-desktop/bin");
    assert_eq!(
        PRIVATE_SCHEMAS,
        "/usr/lib/fermix-desktop/share/glib-2.0/schemas"
    );
    assert_eq!(PRIVATE_ICONS, "/usr/lib/fermix-desktop/share/icons");
    assert_eq!(
        PRIVATE_GST_PLUGINS,
        "/usr/lib/fermix-desktop/lib/gstreamer-1.0"
    );
    assert_eq!(SCHEMA_DIR, "GSETTINGS_SCHEMA_DIR");
}
