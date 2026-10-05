//! The window run from the `fermix-desktop` package, against the private GTK
//! runtime under `/usr/lib/fermix-desktop` (single_package.md §4, A§4.3). The
//! libraries find their own modules and loaders through the prefix they were
//! built for. The schemas are the exception: GLib finds them through
//! `GSETTINGS_SCHEMA_DIR`, which it reads once, when it first builds its
//! default schema source. So the window sets that one variable, has GLib build
//! the source, and puts its environment back as it found it, which every child
//! then inherits. Only the decisions live here; the app changes the environment.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;

/// A process environment, by variable name.
pub type Environment = BTreeMap<OsString, OsString>;

/// Where the package installs the window. `/usr/bin/fermix-desktop` is a
/// symbolic link to the executable in it.
pub const PACKAGED_BIN: &str = "/usr/lib/fermix-desktop/bin";
/// The private runtime's compiled schemas.
pub const PRIVATE_SCHEMAS: &str = "/usr/lib/fermix-desktop/share/glib-2.0/schemas";
/// The private runtime's icon themes, a floor under the host's.
pub const PRIVATE_ICONS: &str = "/usr/lib/fermix-desktop/share/icons";
/// The one variable the window sets, and only when it runs from the package.
pub const SCHEMA_DIR: &str = "GSETTINGS_SCHEMA_DIR";

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

/// What the packaged window sets while GLib reads `schemas`, with each value:
/// the schema directory alone.
pub fn packaged_settings(entry: &Environment, schemas: &Path) -> Vec<(OsString, OsString)> {
    let before = entry.get(OsStr::new(SCHEMA_DIR)).map(OsString::as_os_str);
    vec![(SCHEMA_DIR.into(), schema_dir(schemas, before))]
}

/// What puts the environment back to `entry` after the window set `set`.
/// Nothing else has run by then, so only what the window set needs undoing.
pub fn restore_plan(entry: &Environment, set: &[(OsString, OsString)]) -> Vec<Restore> {
    set.iter()
        .filter_map(|(name, value)| {
            assert!(
                !name.is_empty() && !name.to_string_lossy().contains('='),
                "not a variable name: {name:?}"
            );
            match entry.get(name) {
                Some(before) if before == value => None,
                Some(before) => Some(Restore::Set(name.clone(), before.clone())),
                None => Some(Restore::Unset(name.clone())),
            }
        })
        .collect()
}
