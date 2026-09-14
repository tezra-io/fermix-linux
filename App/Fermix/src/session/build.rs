//! This process's own build, against the one the package installed.
//!
//! The package manager replaces files on disk and never touches a running
//! process, so a window that has been open across an upgrade is older than the
//! application that is installed (M38 section 9.2). The comparison is a build
//! id against a build id: a product version is a display and package-resolution
//! value and never stands in for one, and an id this process cannot establish
//! stays unknown rather than being read as aligned.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The build id compiled into this binary. `build.rs` takes it from the
/// environment at build time and writes `dev` when nothing set one, which is
/// every developer build.
pub const COMPILED_BUILD_ID: &str = env!("FERMIX_DESKTOP_BUILD_ID");

/// The id a build that was never stamped carries. It is compared with nothing:
/// a development build has no installed manifest to disagree with.
pub const DEVELOPMENT_BUILD_ID: &str = "dev";

/// Where the package records what it installed.
pub const MANIFEST_PATH: &str = "/usr/share/fermix-desktop/build.json";

/// Points the manifest somewhere else, which is how tests and a development run
/// keep off the host. A release build reads the packaged path and nothing else.
pub const MANIFEST_OVERRIDE: &str = "FERMIX_DESKTOP_BUILD_MANIFEST";

/// How this process compares with the application that is installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuiAlignment {
    /// Both ids are known and they are the same.
    Aligned,
    /// Both ids are known and they differ: this window is older than the
    /// application on disk.
    Stale,
    /// One of the two ids could not be established. Nothing is claimed.
    Unknown,
}

/// The manifest the package writes beside the binary.
#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    #[serde(default)]
    build_id: Option<String>,
}

/// Where this process reads the installed manifest from.
pub fn manifest_path() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(path) = std::env::var(MANIFEST_OVERRIDE) {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }

    PathBuf::from(MANIFEST_PATH)
}

/// The installed build id, where the manifest names one.
pub fn installed_build_id() -> Option<String> {
    read_build_id(&manifest_path())
}

/// How this process compares with what is installed.
pub fn alignment() -> GuiAlignment {
    compare(COMPILED_BUILD_ID, installed_build_id().as_deref())
}

/// [`alignment`] with both ids supplied, which is what makes the rule testable
/// without a packaged install.
pub fn compare(compiled: &str, installed: Option<&str>) -> GuiAlignment {
    if compiled.is_empty() || compiled == DEVELOPMENT_BUILD_ID {
        return GuiAlignment::Unknown;
    }

    match installed {
        None => GuiAlignment::Unknown,
        Some("") => GuiAlignment::Unknown,
        Some(installed) if installed == compiled => GuiAlignment::Aligned,
        Some(_) => GuiAlignment::Stale,
    }
}

/// The build id inside one manifest file.
///
/// An absent, unreadable or unparseable manifest is an unknown id rather than a
/// failure: this is a comparison, and refusing to draw a window because a file
/// the package owns could not be read would make a packaging defect look like a
/// broken application.
fn read_build_id(path: &Path) -> Option<String> {
    let body = std::fs::read_to_string(path).ok()?;
    let manifest: Manifest = serde_json::from_str(&body).ok()?;

    manifest.build_id.filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TempDirectory;

    #[test]
    fn two_known_ids_that_agree_are_aligned_and_two_that_differ_are_stale() {
        assert_eq!(compare("b7c1f0a4", Some("b7c1f0a4")), GuiAlignment::Aligned);
        assert_eq!(compare("b7c1f0a4", Some("e29a5d10")), GuiAlignment::Stale);
    }

    #[test]
    fn an_id_that_cannot_be_established_stays_unknown() {
        assert_eq!(compare("b7c1f0a4", None), GuiAlignment::Unknown);
        assert_eq!(compare("b7c1f0a4", Some("")), GuiAlignment::Unknown);
        assert_eq!(
            compare(DEVELOPMENT_BUILD_ID, Some("e29a5d10")),
            GuiAlignment::Unknown
        );
        assert_eq!(compare("", Some("e29a5d10")), GuiAlignment::Unknown);
    }

    #[test]
    fn a_manifest_names_the_installed_build_and_a_broken_one_names_nothing() {
        let directory = TempDirectory::new("build-manifest");

        let good = directory.path().join("good.json");
        std::fs::write(&good, br#"{"build_id":"e29a5d10","version":"0.1.0"}"#)
            .expect("the manifest is written");
        assert_eq!(read_build_id(&good).as_deref(), Some("e29a5d10"));

        let empty = directory.path().join("empty.json");
        std::fs::write(&empty, b"{}").expect("the manifest is written");
        assert_eq!(read_build_id(&empty), None);

        let broken = directory.path().join("broken.json");
        std::fs::write(&broken, b"not json").expect("the manifest is written");
        assert_eq!(read_build_id(&broken), None);

        assert_eq!(read_build_id(&directory.path().join("absent.json")), None);
    }
}
