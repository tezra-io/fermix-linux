//! The private toolkit runtime, and the environment a child must get.
//!
//! The package carries its own GTK4, libadwaita and GLib in a private prefix at
//! `/usr/lib/fermix-desktop`, built with that path as its `--prefix`. Almost
//! everything that follows from that is already compiled into those libraries:
//! the GIO module directory, the gdk-pixbuf loader cache and the library search
//! path are all resolved without this process saying anything. Two things are
//! not, and this module owns both.
//!
//! The first is `GSETTINGS_SCHEMA_DIR`. GLib reads it from the environment,
//! there is no compiled-in equivalent, and it prepends to the search path. The
//! private directory has to win: GLib resolves a schema id to the first source
//! that has it, and `g_settings_new` on a schema that exists but lacks the key
//! being read aborts the process rather than falling back. A host GTK 4.6
//! installs `org.gtk.gtk4.Settings.FileChooser` without keys GTK 4.16 reads, so
//! a host-first order aborts on the oldest supported target. Every id the
//! private schemas do not define, `org.gnome.desktop.interface` among them,
//! still resolves through an untouched `XDG_DATA_DIRS`.
//!
//! The second is the bundled Adwaita icon theme. It is appended to the default
//! icon theme's search path after the display exists, so a host theme that
//! matches the session still wins and the bundle is only ever a floor.
//!
//! **This process's environment is not a child's environment.** Everything this
//! module sets would otherwise be inherited by `/usr/bin/fermix` and by the
//! browser a sign-in opens, and a Firefox that starts with a private GTK 4.16
//! schema directory prepended is a real failure. [`RuntimeEnv`] records what
//! each variable held before it was touched, including its absence, and every
//! spawn and launch site in this application applies it, so a child gets the
//! environment this process was started with.
//!
//! **Why [`RuntimeEnv::capture`] must be the first statement of `main`.**
//! `std::env::set_var` mutates a process-wide table that `getenv` reads without
//! a lock, so it is sound only while this process has one thread. That is
//! enforced by construction rather than by comment: `capture` takes no argument,
//! so there is nothing to build before calling it, and it is the only function
//! in this crate that writes the environment.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use gtk4 as gtk;
use gtk4::gio;
use gtk4::prelude::*;

/// Where the package installs the private toolkit runtime. The build container
/// carries it at the same absolute path, which is why a build there needs no
/// special case.
pub const PRIVATE_PREFIX: &str = "/usr/lib/fermix-desktop";

/// The one variable this process sets.
pub const SCHEMA_DIR_VARIABLE: &str = "GSETTINGS_SCHEMA_DIR";

/// The private prefix's compiled schemas.
pub fn schema_directory(prefix: &Path) -> PathBuf {
    prefix.join("share/glib-2.0/schemas")
}

/// The private prefix's bundled icon themes.
pub fn icon_directory(prefix: &Path) -> PathBuf {
    prefix.join("share/icons")
}

/// Whether this process is running against the private toolkit runtime.
///
/// The test is that the prefix carries compiled schemas, because that file is
/// what the variable would point at and a directory without it is nothing worth
/// prepending. It answers yes for a packaged install and for a build container,
/// which both carry the prefix at the same absolute path, and no on a developer
/// machine running against a host GTK, which carries no such directory at all.
///
/// The running executable's own location is deliberately not the test. A build
/// inside the container lives in `target/debug` and still links the private
/// toolkit, so a location rule would leave exactly that case reading a host
/// GTK 4.6 schema and aborting on the first file chooser.
pub fn private_runtime_at(prefix: &Path) -> bool {
    schema_directory(prefix).join("gschemas.compiled").is_file()
}

/// The environment this process was started with, for the children it spawns.
///
/// One entry per variable this process set, carrying what that variable held
/// beforehand. `None` is a variable that was not set at all, and restoring it
/// means unsetting it rather than setting it empty: GLib treats an empty
/// `GSETTINGS_SCHEMA_DIR` as a directory named "" and warns about it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuntimeEnv {
    restore: Vec<(String, Option<String>)>,
}

impl RuntimeEnv {
    /// Set this process's environment for the private runtime, and record what
    /// it held before.
    ///
    /// Call this as the first statement of `main`, before any thread exists and
    /// before GTK is initialised. See the module documentation for why that is
    /// a requirement and not a preference.
    pub fn capture() -> Self {
        Self::capture_from(Path::new(PRIVATE_PREFIX))
    }

    /// [`RuntimeEnv::capture`] against a named prefix.
    ///
    /// Separate so the rule can be driven from a test directory, and private so
    /// nothing outside this module can point a running application at another
    /// prefix.
    fn capture_from(prefix: &Path) -> Self {
        if !private_runtime_at(prefix) {
            return Self::unchanged();
        }

        let prior = non_empty(SCHEMA_DIR_VARIABLE);
        std::env::set_var(SCHEMA_DIR_VARIABLE, schema_directory(prefix));
        Self::restoring(vec![(SCHEMA_DIR_VARIABLE.to_string(), prior)])
    }

    /// The value for a process that set nothing, which is every development run
    /// and every test.
    pub fn unchanged() -> Self {
        Self {
            restore: Vec::new(),
        }
    }

    /// The value that restores exactly these variables.
    pub fn restoring(restore: Vec<(String, Option<String>)>) -> Self {
        Self { restore }
    }

    /// What a child's environment is put back to, one entry per variable.
    pub fn restores(&self) -> &[(String, Option<String>)] {
        &self.restore
    }

    /// A subprocess launcher whose child gets the entry environment.
    ///
    /// The launcher inherits this process's environment and then has each
    /// recorded variable put back, which is the same answer as building the
    /// whole environment from scratch and is one that cannot forget a variable
    /// the session set.
    pub fn launcher(&self, flags: gio::SubprocessFlags) -> gio::SubprocessLauncher {
        let launcher = gio::SubprocessLauncher::new(flags);
        for (name, value) in &self.restore {
            match value {
                Some(value) => launcher.setenv(OsStr::new(name), OsStr::new(value), true),
                None => launcher.unsetenv(OsStr::new(name)),
            }
        }
        launcher
    }

    /// A launch context whose child gets the entry environment.
    ///
    /// The display's own context is used when there is one, so a launch still
    /// carries startup notification; without a display the plain context is the
    /// same object with nothing attached.
    pub fn launch_context(&self) -> gio::AppLaunchContext {
        let context = match gtk::gdk::Display::default() {
            Some(display) => display.app_launch_context().upcast(),
            None => gio::AppLaunchContext::new(),
        };

        for (name, value) in &self.restore {
            match value {
                Some(value) => context.setenv(OsStr::new(name), OsStr::new(value)),
                None => context.unsetenv(OsStr::new(name)),
            }
        }
        context
    }
}

/// Put the bundled Adwaita icon theme behind whatever the host has.
///
/// Appended rather than prepended, so a host theme that matches the session is
/// still preferred and the bundle is the floor. A session with no Adwaita at
/// all, which is what a Hyprland install usually is, draws its icons from here.
/// Called after the display exists, because the icon theme is per display.
/// **This call is required, not a precaution.** GTK builds its icon search path
/// from `XDG_DATA_DIRS` and the user's own data directory. It does not derive it
/// from the prefix it was compiled with, so the bundled theme is invisible until
/// something names it, and this is the one place where "the runtime's paths are
/// compiled in, set nothing" does not hold. The GIO module directory, the
/// gdk-pixbuf loader cache and the schema directory genuinely are compiled in;
/// the icon search path is not. Slice 1's runtime smoke asserted the opposite
/// and failed, which is how this is known rather than assumed.
///
/// Setting `XDG_DATA_DIRS` instead would be the wrong repair: it would move
/// every other data lookup along with the icons.
pub fn add_bundled_icons(display: &gtk::gdk::Display) {
    add_bundled_icons_from(display, Path::new(PRIVATE_PREFIX));
}

/// [`add_bundled_icons`] against a named prefix.
///
/// Public so a display-bearing test can drive it at a directory of its own
/// making, rather than depending on whether the machine running the suite
/// happens to carry the real prefix.
pub fn add_bundled_icons_from(display: &gtk::gdk::Display, prefix: &Path) {
    let Some(directory) = bundled_icon_search_path(prefix) else {
        return;
    };
    gtk::IconTheme::for_display(display).add_search_path(directory);
}

/// The directory to append to the icon search path, or nothing when this is not
/// the packaged runtime.
///
/// Split out from the call that performs it so the decision can be proven
/// without a display: a failure here is a window that opens with no icons at
/// all, which reads as a theming problem rather than a packaging one and would
/// otherwise be caught by nobody until somebody looked at a screenshot.
pub fn bundled_icon_search_path(prefix: &Path) -> Option<PathBuf> {
    private_runtime_at(prefix).then(|| icon_directory(prefix))
}

/// An environment value, with an empty string treated as absent.
fn non_empty(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDirectory;

    /// A prefix that looks like the installed one, as far as the rule reads it.
    fn with_compiled_schemas(directory: &TempDirectory) -> PathBuf {
        let prefix = directory.join("usr/lib/fermix-desktop");
        let schemas = schema_directory(&prefix);
        std::fs::create_dir_all(&schemas).expect("the schema directory is created");
        std::fs::write(schemas.join("gschemas.compiled"), b"").expect("the schemas are written");
        prefix
    }

    #[test]
    fn a_prefix_with_compiled_schemas_is_the_private_runtime() {
        let directory = TempDirectory::new("runtime-present");
        assert!(private_runtime_at(&with_compiled_schemas(&directory)));
    }

    #[test]
    fn a_host_without_the_prefix_is_not_the_private_runtime() {
        let directory = TempDirectory::new("runtime-absent");
        assert!(!private_runtime_at(
            &directory.join("usr/lib/fermix-desktop")
        ));
    }

    #[test]
    fn a_prefix_whose_schemas_were_never_compiled_is_not_the_private_runtime() {
        // The directory alone is not the file the variable would point at, and
        // prepending it would shadow the host schemas with nothing.
        let directory = TempDirectory::new("runtime-uncompiled");
        let prefix = directory.join("usr/lib/fermix-desktop");
        std::fs::create_dir_all(schema_directory(&prefix)).expect("the directory is created");
        assert!(!private_runtime_at(&prefix));
    }

    #[test]
    fn a_development_run_sets_nothing_and_restores_nothing() {
        let directory = TempDirectory::new("runtime-development");
        let captured = RuntimeEnv::capture_from(&directory.join("usr/lib/fermix-desktop"));

        // Nothing was set, so there is nothing for a child to have put back.
        // The environment itself is deliberately not asserted here: these tests
        // run several to a process and one that read a process-wide table would
        // be answering for the whole binary rather than for this call.
        assert_eq!(captured, RuntimeEnv::unchanged());
        assert!(captured.restores().is_empty());
    }

    #[test]
    fn the_packaged_runtime_contributes_its_bundled_icon_theme() {
        // GTK does not find this directory on its own: it builds the icon
        // search path from XDG_DATA_DIRS, never from its compiled-in prefix.
        // Naming it is mandatory, and a window whose icons are all missing is
        // what its absence looks like.
        let directory = TempDirectory::new("icons-present");
        let prefix = with_compiled_schemas(&directory);

        assert_eq!(
            bundled_icon_search_path(&prefix),
            Some(prefix.join("share/icons"))
        );
    }

    #[test]
    fn a_host_toolkit_contributes_no_bundled_icon_theme() {
        // A development run against a host GTK must be left exactly as it was,
        // and there is no directory to name in any case.
        let directory = TempDirectory::new("icons-absent");

        assert_eq!(
            bundled_icon_search_path(&directory.join("usr/lib/fermix-desktop")),
            None
        );
    }

    #[test]
    fn the_schema_directory_sits_under_the_prefix_the_package_installs() {
        assert_eq!(
            schema_directory(Path::new(PRIVATE_PREFIX)),
            PathBuf::from("/usr/lib/fermix-desktop/share/glib-2.0/schemas")
        );
        assert_eq!(
            icon_directory(Path::new(PRIVATE_PREFIX)),
            PathBuf::from("/usr/lib/fermix-desktop/share/icons")
        );
    }

    #[test]
    fn an_absent_variable_is_restored_by_unsetting_it_rather_than_emptying_it() {
        let captured = RuntimeEnv::restoring(vec![(SCHEMA_DIR_VARIABLE.to_string(), None)]);
        assert_eq!(
            captured.restores(),
            &[(SCHEMA_DIR_VARIABLE.to_string(), None)]
        );
    }
}
