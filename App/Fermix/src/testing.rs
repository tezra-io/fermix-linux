//! Test support that both the unit tests inside `src/` and the integration
//! tests under `tests/` use.
//!
//! It lives in the library because a unit test cannot reach `tests/common`, and
//! it is small enough that carrying it is cheaper than two copies that drift.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Where test directories live.
///
/// Deliberately short and deliberately not the system temporary directory: a
/// Unix socket address is capped at around a hundred bytes, and on macOS the
/// per-user temporary directory alone spends half of that. A test that binds a
/// socket under it fails with an address-too-long refusal that looks like a
/// defect in the client.
pub const TEST_ROOT: &str = "/tmp/fermix-desktop-tests";

/// Points test directories somewhere else, for a host whose `/tmp` is not
/// writable.
pub const TEST_ROOT_OVERRIDE: &str = "FERMIX_DESKTOP_TEST_ROOT";

/// The root every test directory sits under.
pub fn test_root() -> PathBuf {
    match std::env::var(TEST_ROOT_OVERRIDE) {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(TEST_ROOT),
    }
}

/// A directory under the system temporary directory that removes itself.
///
/// The removal asserts the path it is about to delete before deleting it: it
/// must sit under the temporary directory, carry at least four components and
/// contain no parent traversal. A recursive delete that can be handed a
/// collapsed path is how a test suite wipes a person's files.
pub struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    /// A fresh directory, named for the test that asked for it.
    pub fn new(label: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = test_root().join(format!("{}-{}-{}", label, std::process::id(), unique));
        std::fs::create_dir_all(&path).expect("a temporary directory can be created");
        Self { path }
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// One path inside it.
    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        if safe_to_remove(&self.path) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Whether a recursive delete of this path is allowed.
///
/// It must sit at least two levels *inside* the temporary directory, never be
/// the temporary directory itself, carry at least four components and contain
/// no parent traversal. An empty interpolation that collapses a path therefore
/// fails every one of those rather than deleting a root.
pub fn safe_to_remove(path: &Path) -> bool {
    let root = test_root();
    let inside = path.starts_with(&root) && path != root;
    let deep_enough = path.components().count() >= 4;
    let no_traversal = !path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir));

    inside && deep_enough && no_traversal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_temporary_directory_exists_and_then_does_not() {
        let path;
        {
            let directory = TempDirectory::new("self-test");
            path = directory.path().to_path_buf();
            assert!(path.is_dir());
        }
        assert!(!path.exists());
    }

    #[test]
    fn a_path_outside_the_test_root_is_never_removed() {
        assert!(!safe_to_remove(Path::new("/")));
        assert!(!safe_to_remove(Path::new("/Users/someone")));
        assert!(!safe_to_remove(Path::new("/tmp")));
        assert!(!safe_to_remove(&test_root()));
        assert!(!safe_to_remove(&std::env::temp_dir()));
    }

    #[test]
    fn a_path_with_a_parent_traversal_is_never_removed() {
        assert!(!safe_to_remove(&test_root().join("a/../../..")));
    }

    #[test]
    fn a_socket_under_a_test_directory_fits_a_unix_socket_address() {
        // A hundred and four bytes is the smallest limit of the platforms this
        // suite runs on, and the client's own tests bind inside these paths.
        let directory = TempDirectory::new("a-label-as-long-as-any-test-uses");
        let socket = directory.join("daemon.sock");
        assert!(
            socket.to_string_lossy().len() < 100,
            "{} is too long for a socket address",
            socket.display()
        );
    }
}
