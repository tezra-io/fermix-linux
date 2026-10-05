//! The window run from the `fermix-desktop` package (single_package.md §4,
//! A§4.3). Its GTK is the private one under `/usr/lib/fermix-desktop`, found
//! through the executable's RUNPATH. Run from the package, the window points
//! GLib's schemas, GIO's modules, gdk-pixbuf's loaders and GStreamer's plugins
//! and registry at the private runtime, has each library read them, then puts
//! its environment back exactly as it started, so every child inherits the
//! entry environment whoever starts it: a sign-in's browser, a link in a
//! reply, the About dialog's licence. After the display exists it adds the
//! bundled icons under the host's. Run from anywhere else (the Flatpak, a
//! development build) it changes nothing.

use fermix_client::runtime_env::{
    gst_registry, packaged_settings, restore_plan, runs_from_package, Environment, PrivatePaths,
    Restore, PRIVATE_GST_PLUGINS, PRIVATE_ICONS, PRIVATE_SCHEMAS,
};
use gtk::{gdk, gdk_pixbuf, gio, glib};
use std::ffi::{OsStr, OsString};
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
    let cache_home = entry.get(OsStr::new("XDG_CACHE_HOME"));
    let paths = PrivatePaths {
        schemas: PRIVATE_SCHEMAS.into(),
        gst_plugins: PRIVATE_GST_PLUGINS.into(),
        gst_registry: gst_registry(cache_home.map(OsString::as_os_str), &glib::home_dir()),
    };
    load_private_runtime(entry, &paths);
    true
}

/// Sets what `packaged_settings` names, has each library read it, then puts
/// back what `entry` had. Each library reads its configuration once and keeps
/// it for the life of the process, so every later use still finds the
/// private runtime, and every later `gst::init()` returns at once.
fn load_private_runtime(entry: &Environment, paths: &PrivatePaths) {
    let settings = packaged_settings(entry, paths);
    for (name, value) in &settings {
        match value {
            Some(value) => std::env::set_var(name, value),
            None => std::env::remove_var(name),
        }
    }
    read_configuration();
    for change in restore_plan(entry, &settings) {
        match change {
            Restore::Set(name, value) => std::env::set_var(name, value),
            Restore::Unset(name) => std::env::remove_var(name),
        }
    }
}

/// The first use of each library that reads its configuration from the
/// environment:
/// - GLib reads `GSETTINGS_SCHEMA_DIR` when it first builds the default schema
///   source (`initialise_schema_sources` in gio/gsettingsschema.c);
/// - GIO reads `GIO_EXTRA_MODULES` and `GIO_MODULE_DIR` when it first lists its
///   modules (`_g_io_modules_ensure_loaded` in gio/giomodule.c), which a proxy
///   lookup does without starting any of them; no module serves this protocol;
/// - gdk-pixbuf reads `GDK_PIXBUF_MODULE_FILE` when it first lists its loaders
///   (`get_file_formats` in gdk-pixbuf-io.c);
/// - GStreamer reads its registry and plugin paths in its first `gst_init`,
///   which scans the private plugins through the private scanner, a child
///   that inherits the environment as it is now.
fn read_configuration() {
    if gio::SettingsSchemaSource::default().is_none() {
        glib::g_warning!("fermix", "GLib found no settings schemas at all");
    }
    gio::Proxy::default_for_protocol("fermix-none");
    // Every private loader is a module, so no formats means the private
    // loaders.cache was unreadable, and gdk-pixbuf reads its variable again
    // on its next use, by then the entry value.
    if gdk_pixbuf::Pixbuf::formats().is_empty() {
        glib::g_warning!(
            "fermix",
            "gdk-pixbuf could not read the private loader list"
        );
    }
    // Without arguments gst_init_check cannot fail: both of its hooks only
    // return TRUE. Were it to, the next `gst::init()` would start GStreamer
    // with the entry environment, so it must not pass quietly.
    gst::init().expect("GStreamer starts");
}

/// Once the display exists. Appended, so a host icon theme still wins and
/// the bundled one is the floor.
pub fn add_bundled_icons(display: &gdk::Display) {
    gtk::IconTheme::for_display(display).add_search_path(PRIVATE_ICONS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Command;

    /// Names the fixture directory in the process this test starts for
    /// itself, and marks that process.
    const ALONE: &str = "FERMIX_TEST_PRIVATE_RUNTIME";
    const RUNTIME_TEST: &str =
        "runtime_env::tests::the_private_runtime_is_in_force_after_the_environment_is_put_back";
    const SCHEMA: &str = "io.tezra.Fermix.RuntimeEnvTest";
    /// An image format only the host's loader list names.
    const HOST_FORMAT: &str = "fermixhost";
    /// A GIO module only the host's module directory holds. It is not a
    /// library, so GIO prints its name if it ever tries to load it.
    const HOST_MODULE: &str = "libfermixhost.so";
    /// Every variable the window sets, and the unversioned names GStreamer
    /// falls back to: absent unless a run names them.
    const VARIABLES: [&str; 12] = [
        "GSETTINGS_SCHEMA_DIR",
        "GIO_EXTRA_MODULES",
        "GIO_MODULE_DIR",
        "GDK_PIXBUF_MODULE_FILE",
        "GST_REGISTRY",
        "GST_REGISTRY_1_0",
        "GST_PLUGIN_PATH",
        "GST_PLUGIN_PATH_1_0",
        "GST_PLUGIN_SYSTEM_PATH",
        "GST_PLUGIN_SYSTEM_PATH_1_0",
        "GST_PLUGIN_SCANNER",
        "GST_PLUGIN_SCANNER_1_0",
    ];
    /// What a host run names, each pointing into the fixture.
    const HOST_VALUES: [(&str, &str); 7] = [
        ("GSETTINGS_SCHEMA_DIR", "host-schemas"),
        ("GIO_EXTRA_MODULES", "host-gio"),
        ("GDK_PIXBUF_MODULE_FILE", "host-loaders.cache"),
        ("GST_REGISTRY", "host-registry.bin"),
        ("GST_PLUGIN_PATH", "host-plugins"),
        ("GST_PLUGIN_SYSTEM_PATH", "host-plugins"),
        ("GST_PLUGIN_SCANNER", "host-scanner"),
    ];

    /// Private and host copies of what each library could read, under one
    /// directory removed when dropped. The plugins are links to the SDK's.
    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Fixture {
            let root =
                std::env::temp_dir().join(format!("fermix-runtime-env-{}", std::process::id()));
            std::fs::create_dir_all(&root).expect("the fixture directory is made");
            let fixture = Fixture(root);
            fixture.schemas("private-schemas", true);
            fixture.schemas("host-schemas", false);
            let installed = installed_plugins();
            fixture.plugin(&installed, "private-plugins", "libgstcoreelements.so");
            fixture.plugin(&installed, "host-plugins", "libgstaudiotestsrc.so");
            // The user's own plugin directory, under XDG_DATA_HOME.
            fixture.plugin(&installed, "data/gstreamer-1.0/plugins", "libgstlevel.so");
            fixture.write(&format!("host-gio/{HOST_MODULE}"), "not a library");
            fixture.write("host-loaders.cache", &host_loaders(&fixture.0));
            // A scanner that leaves a mark wherever GStreamer starts it.
            fixture.write("host-scanner", "#!/bin/sh\n: > \"$0.ran\"\n");
            let executable = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(fixture.0.join("host-scanner"), executable)
                .expect("the host scanner is made executable");
            fixture
        }

        fn write(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            let dir = path.parent().expect("a fixture file has a directory");
            std::fs::create_dir_all(dir).expect("the fixture file's directory is made");
            std::fs::write(&path, text).expect("the fixture file is written");
        }

        fn plugin(&self, installed: &Path, dir: &str, file: &str) {
            let dir = self.0.join(dir);
            std::fs::create_dir_all(&dir).expect("the plugin directory is made");
            std::os::unix::fs::symlink(installed.join(file), dir.join(file))
                .expect("the plugin is linked");
        }

        /// `added` puts a key in the schema that only this copy has.
        fn schemas(&self, name: &str, added: bool) {
            let extra = if added {
                r#"<key name="added" type="b"><default>true</default></key>"#
            } else {
                ""
            };
            let xml = format!(
                r#"<schemalist><schema id="{SCHEMA}" path="/io/tezra/Fermix/RuntimeEnvTest/">
                <key name="kept" type="b"><default>true</default></key>{extra}</schema></schemalist>"#
            );
            self.write(&format!("{name}/{SCHEMA}.gschema.xml"), &xml);
            let compiled = Command::new("glib-compile-schemas")
                .arg(self.0.join(name))
                .status()
                .expect("glib-compile-schemas runs");
            assert!(compiled.success(), "glib-compile-schemas failed for {name}");
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Err(e) = std::fs::remove_dir_all(&self.0) {
                eprintln!("the test fixture {:?} stays: {e}", self.0);
            }
        }
    }

    /// The directory the build's own GStreamer installed its plugins in.
    fn installed_plugins() -> PathBuf {
        let output = Command::new("pkg-config")
            .args(["--variable=pluginsdir", "gstreamer-1.0"])
            .output()
            .expect("pkg-config runs");
        assert!(output.status.success(), "pkg-config knows no gstreamer-1.0");
        let dir = String::from_utf8(output.stdout).expect("the plugin directory is UTF-8");
        PathBuf::from(dir.trim())
    }

    /// A loader list naming one format, whose loader is never opened.
    fn host_loaders(root: &Path) -> String {
        let module = root.join("libpixbufloader-fermixhost.so");
        format!(
            "{module:?}\n\"{HOST_FORMAT}\" 0 \"gdk-pixbuf\" \"Fermix host\" \"LGPL\"\n\
             \"image/x-{HOST_FORMAT}\" \"\"\n\"fxh\" \"\"\n\"FXH\" \"\" 100\n\n"
        )
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
    /// its own: once with none of the variables at entry, once with each
    /// naming a host copy. A host copy is a schema that lacks a key, a GIO
    /// module, an image loader, a registry, a plugin directory and a scanner.
    #[test]
    fn the_private_runtime_is_in_force_after_the_environment_is_put_back() {
        if let Some(root) = std::env::var_os(ALONE) {
            return in_a_process_alone(Path::new(&root));
        }
        let fixture = Fixture::new();
        run_alone(&fixture, false);
        run_alone(&fixture, true);
    }

    fn run_alone(fixture: &Fixture, host: bool) {
        let exe = std::env::current_exe().expect("the test binary knows its own path");
        let cache = if host { "cache-host" } else { "cache-bare" };
        let mut command = Command::new(exe);
        command
            .args([RUNTIME_TEST, "--exact", "--test-threads=1"])
            .env(ALONE, &fixture.0)
            .env("XDG_CACHE_HOME", fixture.0.join(cache))
            .env("XDG_DATA_HOME", fixture.0.join("data"));
        for name in VARIABLES {
            command.env_remove(name);
        }
        for (name, file) in HOST_VALUES.iter().filter(|_| host) {
            command.env(name, fixture.0.join(file));
        }
        let output = command.output().expect("the test binary runs again");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // "1 passed" proves the test ran there, not only that nothing failed.
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "alone, with host values {host}:\n{stdout}\n{stderr}"
        );
        assert!(
            !stderr.contains(HOST_MODULE),
            "GIO tried the host's module directory:\n{stderr}"
        );
    }

    fn in_a_process_alone(root: &Path) {
        let entry: Environment = std::env::vars_os().collect();
        let cache_home = entry
            .get(OsStr::new("XDG_CACHE_HOME"))
            .expect("the test names a cache directory");
        let paths = PrivatePaths {
            schemas: root.join("private-schemas"),
            gst_plugins: root.join("private-plugins"),
            gst_registry: gst_registry(Some(cache_home), &glib::home_dir()),
        };
        let before = threads();
        load_private_runtime(&entry, &paths);
        assert_eq!(
            threads(),
            before,
            "reading the configuration started a thread"
        );
        let differ = names_that_differ(&entry, &std::env::vars_os().collect());
        assert!(differ.is_empty(), "the environment differs in {differ:?}");
        assert_private_schema();
        assert_private_plugins(root, Path::new(cache_home), &paths.gst_registry);
        let formats = gdk_pixbuf::Pixbuf::formats();
        assert!(
            !formats
                .iter()
                .any(|f| f.name().as_deref() == Some(HOST_FORMAT)),
            "gdk-pixbuf read the host's loader list"
        );
        // Were GIO to read its module list now, the host's directory would be
        // in it, and the parent finds the failed load in this process's output.
        gio::Proxy::default_for_protocol("fermix-none");
    }

    fn assert_private_schema() {
        let source = gio::SettingsSchemaSource::default().expect("GLib has schemas");
        let schema = source
            .lookup(SCHEMA, true)
            .expect("the private test schema resolves");
        assert!(
            schema.has_key("added"),
            "a host copy of the schema came before the private one"
        );
    }

    fn assert_private_plugins(root: &Path, cache_home: &Path, registry: &Path) {
        gst::init().expect("GStreamer starts");
        assert!(
            gst::ElementFactory::find("fakesink").is_some(),
            "the private plugin is missing"
        );
        for host in ["audiotestsrc", "level"] {
            let found = gst::ElementFactory::find(host).is_some();
            assert!(!found, "{host} came from a host plugin directory");
        }
        assert!(registry.exists(), "GStreamer wrote no private registry");
        let shared = cache_home.join("gstreamer-1.0");
        assert!(!shared.exists(), "GStreamer wrote the shared registry");
        let host = root.join("host-registry.bin");
        assert!(!host.exists(), "GStreamer wrote the host's registry");
        let ran = root.join("host-scanner.ran");
        assert!(!ran.exists(), "GStreamer started the host's plugin scanner");
    }

    #[test]
    fn run_from_anywhere_else_the_window_changes_nothing() {
        let entry = Environment::from([("GSETTINGS_SCHEMA_DIR".into(), "/opt/schemas".into())]);
        for exe in [
            "/app/bin/fermix-desktop",
            "/home/someone/src/fermix-linux/desktop/target-sdk/debug/fermix-desktop",
        ] {
            assert!(!start_from(&entry, Path::new(exe)), "{exe}");
        }
    }
}
