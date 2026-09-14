//! The command line this binary answers.
//!
//! Almost nothing: the window is the product and the packaged `fermix` command
//! line is where a terminal belongs. Three arguments exist, and each of them is
//! here because something outside this process needs it.
//!
//! * `--version` says what this build is, and is what an install smoke and a bug
//!   report ask. It prints the product version and the build id together,
//!   because the id is the half that decides whether a running window is older
//!   than the application on disk, and a version alone cannot answer that.
//! * Everything else is the toolkit's own and is handed to it unread. The one
//!   that matters is `--gapplication-service`, which the D-Bus activation file
//!   and the activation unit both pass: it is how a launch from a cold session
//!   starts this process through the user manager rather than through the
//!   launcher's own `Exec` line.
//! * `--capture DIR` is this binary's, in a debug build only, and is taken out
//!   before the rest reaches the toolkit. A release build has no such flag, so a
//!   release build passes it on and the toolkit refuses it by name, which is the
//!   loud answer rather than a silent one.

/// This build's product version, which is the crate's, which is the tag's.
/// `scripts/build_packages.sh` holds the three equal.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The one argument this binary reads for itself and answers without a window.
pub const VERSION_FLAG: &str = "--version";

/// What `--version` prints.
pub fn version_line() -> String {
    format!(
        "fermix-desktop {PRODUCT_VERSION} (build {})",
        crate::session::build::COMPILED_BUILD_ID
    )
}

/// Whether this invocation is asking what the build is rather than for a window.
pub fn wants_version(arguments: &[String]) -> bool {
    arguments
        .iter()
        .skip(1)
        .any(|argument| argument == VERSION_FLAG)
}

/// The arguments the toolkit is handed.
///
/// The program name, then everything this binary does not own itself. In a debug
/// build that means the capture flag and its directory are removed; in a release
/// build there is nothing to remove, because the flag does not exist there.
pub fn toolkit_arguments(arguments: &[String]) -> Vec<String> {
    let mut kept = Vec::with_capacity(arguments.len());
    let mut rest = arguments.iter();

    if let Some(program) = rest.next() {
        kept.push(program.clone());
    }

    for argument in rest {
        #[cfg(debug_assertions)]
        if argument == crate::capture::FLAG {
            // The directory belongs to the flag, so it leaves with it.
            break;
        }
        #[cfg(debug_assertions)]
        if argument.starts_with(&format!("{}=", crate::capture::FLAG)) {
            continue;
        }
        kept.push(argument.clone());
    }

    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(pieces: &[&str]) -> Vec<String> {
        pieces.iter().map(|piece| (*piece).to_string()).collect()
    }

    #[test]
    fn the_version_line_carries_the_product_version_and_the_build_id() {
        let line = version_line();

        assert!(line.starts_with("fermix-desktop "));
        assert!(line.contains(PRODUCT_VERSION));
        assert!(line.contains(crate::session::build::COMPILED_BUILD_ID));
    }

    #[test]
    fn only_the_version_flag_asks_what_this_build_is() {
        assert!(wants_version(&arguments(&["fermix-desktop", "--version"])));
        assert!(!wants_version(&arguments(&["fermix-desktop"])));
        assert!(!wants_version(&arguments(&[
            "fermix-desktop",
            "--gapplication-service"
        ])));
        // The program's own path is never read as a flag.
        assert!(!wants_version(&arguments(&["--version"])));
    }

    #[test]
    fn the_toolkits_own_arguments_reach_it() {
        assert_eq!(
            toolkit_arguments(&arguments(&["fermix-desktop", "--gapplication-service"])),
            arguments(&["fermix-desktop", "--gapplication-service"])
        );
        assert_eq!(
            toolkit_arguments(&arguments(&["fermix-desktop", "fermix://settings"])),
            arguments(&["fermix-desktop", "fermix://settings"])
        );
        assert_eq!(toolkit_arguments(&[]), Vec::<String>::new());
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_capture_flag_is_this_binarys_and_does_not_reach_the_toolkit() {
        assert_eq!(
            toolkit_arguments(&arguments(&["fermix-desktop", "--capture", "/tmp/shots"])),
            arguments(&["fermix-desktop"])
        );
        assert_eq!(
            toolkit_arguments(&arguments(&[
                "fermix-desktop",
                "--capture=/tmp/shots",
                "--gapplication-service"
            ])),
            arguments(&["fermix-desktop", "--gapplication-service"])
        );
    }
}
