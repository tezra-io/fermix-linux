//! The window run from the `fermix-desktop` package, against the private GTK
//! runtime under `/usr/lib/fermix-desktop` (single_package.md §4, A§4.3). The
//! libraries find their own modules and loaders through the prefix they were
//! built for, but each also reads the environment once, the first time it
//! needs its configuration, and a host value there would point it at the
//! host's schemas, modules, loaders or plugins. So the window sets those
//! variables, has each library read them, and puts its environment back as it
//! found it, which every child then inherits. Only the decisions live here;
//! the app changes the environment.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A process environment, by variable name.
pub type Environment = BTreeMap<OsString, OsString>;

/// A variable as the window holds it while the libraries read it: a value,
/// or `None` for absent.
pub type Setting = (OsString, Option<OsString>);

/// Where the package installs the window. `/usr/bin/fermix-desktop` is a
/// symbolic link to the executable in it.
pub const PACKAGED_BIN: &str = "/usr/lib/fermix-desktop/bin";
/// The private runtime's compiled schemas.
pub const PRIVATE_SCHEMAS: &str = "/usr/lib/fermix-desktop/share/glib-2.0/schemas";
/// The private runtime's icon themes, a floor under the host's.
pub const PRIVATE_ICONS: &str = "/usr/lib/fermix-desktop/share/icons";
/// The private runtime's GStreamer plugins.
pub const PRIVATE_GST_PLUGINS: &str = "/usr/lib/fermix-desktop/lib/gstreamer-1.0";
/// Where GLib finds schemas, ahead of `XDG_DATA_DIRS`.
pub const SCHEMA_DIR: &str = "GSETTINGS_SCHEMA_DIR";

/// What the private libraries are pointed at while they read their configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivatePaths {
    pub schemas: PathBuf,
    pub gst_plugins: PathBuf,
    pub gst_registry: PathBuf,
}

/// One change that puts the environment back as the window found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restore {
    /// The window changed this variable: it gets its entry value back.
    Set(OsString, OsString),
    /// The window set a variable the entry environment lacked: it goes.
    Unset(OsString),
}

/// Whether the window's own executable, with links resolved, is the packaged
/// one. The Flatpak's and a development build's are elsewhere. A window whose
/// file an upgrade replaced reads as `fermix-desktop (deleted)`, still inside.
pub fn runs_from_package(exe: &Path) -> bool {
    assert!(
        exe.is_absolute(),
        "the executable's path is absolute: {exe:?}"
    );
    exe.starts_with(PACKAGED_BIN)
}

/// `GSETTINGS_SCHEMA_DIR` while GLib reads the schemas: `schemas` first, then
/// whatever the entry environment named. GLib takes a schema from the first
/// directory that has it, and a host GTK's schema can lack a key the private
/// GTK reads, which aborts the process; so the private copy must win. Schemas
/// it does not carry still resolve through `XDG_DATA_DIRS`.
pub fn schema_dir(schemas: &Path, entry: Option<&OsStr>) -> OsString {
    assert!(
        schemas.is_absolute(),
        "the schema directory is absolute: {schemas:?}"
    );
    let mut value = OsString::from(schemas);
    if let Some(entry) = entry.filter(|v| !v.is_empty()) {
        value.push(":");
        value.push(entry);
    }
    value
}

/// The private GStreamer's registry: its own file, so the host's GStreamer and
/// the private one never rewrite each other's. Under `$XDG_CACHE_HOME`, or
/// `~/.cache` when that is unset, empty or relative, which the XDG base
/// directory specification says to ignore. The file name carries the machine
/// architecture, as GStreamer's own does, for a home shared between machines.
pub fn gst_registry(cache_home: Option<&OsStr>, home: &Path) -> PathBuf {
    assert!(
        home.is_absolute(),
        "the home directory is absolute: {home:?}"
    );
    let cache = match cache_home.map(Path::new) {
        Some(dir) if dir.is_absolute() => dir.to_path_buf(),
        _ => home.join(".cache"),
    };
    let file = format!("registry.{}.bin", std::env::consts::ARCH);
    cache.join("fermix-desktop/gstreamer-1.0").join(file)
}

/// What the packaged window sets while the libraries read their configuration.
/// GIO loads only its own module directory. gdk-pixbuf reads its own loader
/// list. GStreamer keeps its own registry, scans only the private plugins (the
/// system path also leaves out `~/.local/share/gstreamer-1.0/plugins`), and
/// starts its own scanner, which an empty value selects. Each `_1_0` name
/// hides the unversioned one.
pub fn packaged_settings(entry: &Environment, paths: &PrivatePaths) -> Vec<Setting> {
    let schemas = entry.get(OsStr::new(SCHEMA_DIR)).map(OsString::as_os_str);
    let set = |name: &str, value: &OsStr| (name.into(), Some(value.to_owned()));
    let unset = |name: &str| (name.into(), None);
    vec![
        set(SCHEMA_DIR, &schema_dir(&paths.schemas, schemas)),
        set("GIO_EXTRA_MODULES", OsStr::new("")),
        unset("GIO_MODULE_DIR"),
        unset("GDK_PIXBUF_MODULE_FILE"),
        set("GST_REGISTRY_1_0", paths.gst_registry.as_os_str()),
        set("GST_PLUGIN_SYSTEM_PATH_1_0", paths.gst_plugins.as_os_str()),
        set("GST_PLUGIN_PATH_1_0", OsStr::new("")),
        set("GST_PLUGIN_SCANNER_1_0", OsStr::new("")),
    ]
}

/// What puts the environment back to `entry` after the window applied
/// `settings`. Nothing else has run by then, so only those need undoing.
pub fn restore_plan(entry: &Environment, settings: &[Setting]) -> Vec<Restore> {
    settings
        .iter()
        .filter_map(|(name, during)| {
            assert!(
                !name.is_empty() && !name.to_string_lossy().contains('='),
                "not a variable name: {name:?}"
            );
            match entry.get(name) {
                before if before == during.as_ref() => None,
                Some(before) => Some(Restore::Set(name.clone(), before.clone())),
                None => Some(Restore::Unset(name.clone())),
            }
        })
        .collect()
}
