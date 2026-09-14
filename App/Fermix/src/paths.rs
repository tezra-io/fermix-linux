//! Where the engine's command line and its socket are.
//!
//! The packaged path is a compile-time constant and is never resolved on
//! `PATH`: the `fermix` package owns `/usr/bin/fermix` and the desktop package
//! declares an exact-version dependency on it, so resolving on `PATH` would let
//! a shell alias, a Nix profile or a stale standalone install answer for the
//! package this interface is bound to (M38 section 5.6).
//!
//! The two development configurations below are `cfg(debug_assertions)` only.
//! They are declared configurations, not a runtime branch: a release build
//! reads neither variable and cannot be pointed at a fixture.

use std::path::{Path, PathBuf};

/// The packaged command line.
pub const PACKAGED_CLI: &str = "/usr/bin/fermix";

/// Points a debug build at a fixture home holding a `daemon.sock`.
pub const FIXTURE_HOME_VARIABLE: &str = "FERMIX_DESKTOP_FIXTURE_HOME";
/// Points a debug build at a stand-in for the packaged command line.
pub const CLI_VARIABLE: &str = "FERMIX_DESKTOP_CLI";
/// The control socket's name inside a home.
pub const SOCKET_NAME: &str = "daemon.sock";

/// The paths this process runs against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    cli: PathBuf,
    fixture_home: Option<PathBuf>,
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            cli: PathBuf::from(PACKAGED_CLI),
            fixture_home: None,
        }
    }
}

impl Paths {
    /// The paths a release build uses: the packaged command line, and a socket
    /// that comes from the binding `service status` reports.
    pub fn packaged() -> Self {
        Self::default()
    }

    /// The paths this process runs against, resolved from the environment.
    ///
    /// In a release build this is [`Paths::packaged`] and nothing else is read.
    pub fn resolve() -> Self {
        #[cfg(debug_assertions)]
        {
            Self::resolve_from(
                non_empty_env(CLI_VARIABLE).map(PathBuf::from),
                non_empty_env(FIXTURE_HOME_VARIABLE).map(PathBuf::from),
            )
        }
        #[cfg(not(debug_assertions))]
        {
            Self::packaged()
        }
    }

    /// [`Paths::resolve`] with the two development values supplied, which is
    /// what makes the rule testable in both build configurations.
    pub fn resolve_from(cli: Option<PathBuf>, fixture_home: Option<PathBuf>) -> Self {
        Self {
            cli: cli.unwrap_or_else(|| PathBuf::from(PACKAGED_CLI)),
            fixture_home,
        }
    }

    /// The command line to invoke.
    pub fn cli(&self) -> &Path {
        &self.cli
    }

    /// The fixture home, when this process was pointed at one.
    pub fn fixture_home(&self) -> Option<&Path> {
        self.fixture_home.as_deref()
    }

    /// The socket to speak to, when it is known without asking the command
    /// line. A packaged run learns it from the binding `service status`
    /// reports, so this answers nothing there.
    pub fn fixture_socket(&self) -> Option<PathBuf> {
        self.fixture_home
            .as_ref()
            .map(|home| home.join(SOCKET_NAME))
    }
}

/// Gated exactly as its only caller is: a release build reads no environment
/// variable at all, so in that configuration this function does not exist
/// rather than sitting unused.
#[cfg(debug_assertions)]
fn non_empty_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_development_configuration_the_packaged_path_is_used() {
        let paths = Paths::resolve_from(None, None);
        assert_eq!(paths.cli(), Path::new(PACKAGED_CLI));
        assert_eq!(paths.fixture_home(), None);
        assert_eq!(paths.fixture_socket(), None);
    }

    #[test]
    fn a_fixture_home_names_the_socket_inside_it() {
        let paths = Paths::resolve_from(None, Some(PathBuf::from("/tmp/fixture")));
        assert_eq!(
            paths.fixture_socket(),
            Some(PathBuf::from("/tmp/fixture/daemon.sock"))
        );
    }

    #[test]
    fn a_stand_in_command_line_replaces_the_packaged_one() {
        let paths = Paths::resolve_from(Some(PathBuf::from("/tmp/fake/fermix")), None);
        assert_eq!(paths.cli(), Path::new("/tmp/fake/fermix"));
    }

    #[test]
    fn a_release_build_reads_neither_variable() {
        // The resolution a release build performs, exercised in every build so
        // the packaged posture is proven rather than assumed.
        assert_eq!(Paths::packaged(), Paths::resolve_from(None, None));
    }
}
