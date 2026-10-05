//! The window run from the `fermix-desktop` package (single_package.md §4,
//! A§4.3). Its GTK is the private one under `/usr/lib/fermix-desktop`, found
//! through the executable's RUNPATH. Run from the package, the window has GLib
//! load the private schemas first, then puts its environment back exactly as it
//! started, so every child inherits the entry environment whoever starts it:
//! a sign-in's browser, a link in a reply, the About dialog's licence. After
//! the display exists it adds the bundled icons under the host's. Run from
//! anywhere else (the Flatpak, a development build) it changes nothing.

use fermix_client::runtime_env::{
    packaged_settings, restore_plan, runs_from_package, Environment, Restore, PRIVATE_ICONS,
    PRIVATE_SCHEMAS,
};
use gtk::{gdk, gio, glib};
use std::path::Path;

/// The first statement of `main`, before GTK and before any thread exists:
/// changing the environment is sound only while the process has one thread.
/// Returns whether the window runs from the package.
pub fn start() -> bool {
    let entry: Environment = std::env::vars_os().collect();
    let exe = std::fs::read_link("/proc/self/exe")
        .expect("the window can read its own executable's path");
    start_from(&entry, &exe)
}

fn start_from(entry: &Environment, exe: &Path) -> bool {
    if !runs_from_package(exe) {
        return false;
    }
    load_schemas(entry, Path::new(PRIVATE_SCHEMAS));
    true
}

/// Has GLib build its default schema source with `schemas` first, then puts
/// back what `entry` had. GLib reads `GSETTINGS_SCHEMA_DIR` once, when it first
/// builds that source, and keeps the source for the life of the process
/// (`initialise_schema_sources` in gio/gsettingsschema.c). Every GSettings
/// lookup after this still finds the private schemas first.
fn load_schemas(entry: &Environment, schemas: &Path) {
    let settings = packaged_settings(entry, schemas);
    for (name, value) in &settings {
        std::env::set_var(name, value);
    }
    if gio::SettingsSchemaSource::default().is_none() {
        glib::g_warning!("fermix", "GLib found no settings schemas at all");
    }
    for change in restore_plan(entry, &settings) {
        match change {
            Restore::Set(name, value) => std::env::set_var(name, value),
            Restore::Unset(name) => std::env::remove_var(name),
        }
    }
}

/// Once the display exists. Appended, so a host icon theme still wins and
/// the bundled one is the floor.
pub fn add_bundled_icons(display: &gdk::Display) {
    gtk::IconTheme::for_display(display).add_search_path(PRIVATE_ICONS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use fermix_client::runtime_env::SCHEMA_DIR;
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::process::Command;

    /// Names the private schema directory in the process this test starts for
    /// itself, and marks that process.
    const ALONE: &str = "FERMIX_TEST_PRIVATE_SCHEMAS";
    const SCHEMA_TEST: &str =
        "runtime_env::tests::the_private_schemas_resolve_after_the_environment_is_put_back";
    const SCHEMA: &str = "io.tezra.Fermix.RuntimeEnvTest";

    /// A directory holding a compiled test schema, removed when dropped.
    struct SchemaDir(PathBuf);

    impl SchemaDir {
        /// `added` puts a key in the schema that only this copy has.
        fn new(name: &str, added: bool) -> SchemaDir {
            let dir = std::env::temp_dir()
                .join(format!("fermix-runtime-env-{}-{name}", std::process::id()));
            std::fs::create_dir_all(&dir).expect("the schema directory is made");
            let extra = if added {
                r#"<key name="added" type="b"><default>true</default></key>"#
            } else {
                ""
            };
            let xml = format!(
                r#"<schemalist><schema id="{SCHEMA}" path="/io/tezra/Fermix/RuntimeEnvTest/">
                <key name="kept" type="b"><default>true</default></key>{extra}</schema></schemalist>"#
            );
            std::fs::write(dir.join(format!("{SCHEMA}.gschema.xml")), xml)
                .expect("the schema is written");
            let compiled = Command::new("glib-compile-schemas")
                .arg(&dir)
                .status()
                .expect("glib-compile-schemas runs");
            assert!(compiled.success(), "glib-compile-schemas failed in {dir:?}");
            SchemaDir(dir)
        }
    }

    impl Drop for SchemaDir {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("the test schema directory {:?} stays: {e}", self.0);
            }
        }
    }

    /// The threads this process has now.
    fn threads() -> usize {
        std::fs::read_dir("/proc/self/task")
            .expect("the process lists its threads")
            .count()
    }

    /// The names whose values differ, never the values: an environment
    /// carries secrets, and test output is kept.
    fn names_that_differ(entry: &Environment, now: &Environment) -> Vec<OsString> {
        let names: BTreeSet<&OsString> = entry.keys().chain(now.keys()).collect();
        names
            .into_iter()
            .filter(|name| entry.get(*name) != now.get(*name))
            .cloned()
            .collect()
    }

    /// Changing the environment while another thread reads it is unsound, and
    /// other tests here start GStreamer, so this one runs again in a process of
    /// its own: once with the schema directory absent at entry, once with it
    /// naming a host copy of the same schema that lacks a key. There it also
    /// checks that GLib building its schema source started no thread.
    #[test]
    fn the_private_schemas_resolve_after_the_environment_is_put_back() {
        if let Some(private) = std::env::var_os(ALONE) {
            return in_a_process_alone(Path::new(&private));
        }
        let private = SchemaDir::new("private", true);
        let host = SchemaDir::new("host", false);
        run_alone(&private, None);
        run_alone(&private, Some(&host));
    }

    fn run_alone(private: &SchemaDir, host: Option<&SchemaDir>) {
        let exe = std::env::current_exe().expect("the test binary knows its own path");
        let mut command = Command::new(exe);
        command
            .args([SCHEMA_TEST, "--exact", "--test-threads=1"])
            .env(ALONE, &private.0);
        match host {
            Some(host) => command.env(SCHEMA_DIR, &host.0),
            None => command.env_remove(SCHEMA_DIR),
        };
        let output = command.output().expect("the test binary runs again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "1 passed" proves the test ran there, not only that nothing failed.
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "alone, with a host schema directory {}:\n{stdout}\n{stderr}",
            host.is_some()
        );
    }

    fn in_a_process_alone(private: &Path) {
        let entry: Environment = std::env::vars_os().collect();
        let before = threads();
        load_schemas(&entry, private);
        assert_eq!(threads(), before, "loading the schemas started a thread");
        let now: Environment = std::env::vars_os().collect();
        let differ = names_that_differ(&entry, &now);
        assert!(differ.is_empty(), "the environment differs in {differ:?}");
        let source = gio::SettingsSchemaSource::default().expect("GLib has schemas");
        let schema = source
            .lookup(SCHEMA, true)
            .expect("the private test schema resolves");
        assert!(
            schema.has_key("added"),
            "a host copy of the schema came before the private one"
        );
    }

    #[test]
    fn run_from_anywhere_else_the_window_changes_nothing() {
        let entry = Environment::from([(SCHEMA_DIR.into(), "/opt/schemas".into())]);
        for exe in [
            "/app/bin/fermix-desktop",
            "/home/someone/src/fermix-linux/desktop/target-sdk/debug/fermix-desktop",
        ] {
            assert!(!start_from(&entry, Path::new(exe)), "{exe}");
        }
    }
}
