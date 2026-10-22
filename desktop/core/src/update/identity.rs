//! Whether Fermix runs what is installed (M38 §9.2). The package manager
//! replaces files while the engine and the window keep running old code, so
//! a pending restart is read from the disk: the installed engine's
//! `engine.json` against the engine that answers hello, by build id, and the
//! window's own executable, which reads "(deleted)" once replaced. That finds
//! a terminal `apt upgrade` or another account's install as well as the
//! app's own.

use crate::model::EngineIdentity;
use crate::runtime_env::runs_from_package;
use std::path::Path;

/// What the engine package installs about itself.
pub const INSTALLED_ENGINE: &str = "/usr/share/fermix/engine.json";
/// The one distribution this app manages the engine of.
const PACKAGED: &str = "linux_package";
/// How the kernel marks an executable whose file is gone.
const DELETED: &str = " (deleted)";

/// M38 §9.2's table, for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Alignment {
    /// The running engine is the installed build.
    Aligned,
    /// Another build is installed than the one running.
    PendingRestart,
    /// No engine answers.
    NotRunning,
    /// A build id is missing on one side, so nothing can be said.
    Unknown,
    /// The running engine is not this package's kind or machine.
    OwnershipConflict(String),
}

/// The installed engine's identity, from `INSTALLED_ENGINE`'s bytes.
pub fn installed_engine(bytes: &[u8]) -> Result<EngineIdentity, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("{INSTALLED_ENGINE} is not readable: {e}"))
}

/// Build id against build id; the product versions are only for display.
pub fn engine_alignment(
    installed: Option<&EngineIdentity>,
    running: Option<&EngineIdentity>,
) -> Alignment {
    let Some(running) = running else {
        return Alignment::NotRunning;
    };
    let Some(installed) = installed else {
        return Alignment::Unknown;
    };
    if running.distribution_identity.as_deref() != Some(PACKAGED) {
        let kind = running
            .distribution_identity
            .as_deref()
            .unwrap_or("unnamed");
        return Alignment::OwnershipConflict(format!("the running engine is a {kind} build"));
    }
    if let (Some(on_disk), Some(answering)) = (&installed.architecture, &running.architecture) {
        if on_disk != answering {
            return Alignment::OwnershipConflict(format!(
                "the running engine is for {answering}, the installed one for {on_disk}"
            ));
        }
    }
    match (&installed.build_id, &running.build_id) {
        (Some(on_disk), Some(answering)) if on_disk == answering => Alignment::Aligned,
        (Some(_), Some(_)) => Alignment::PendingRestart,
        _ => Alignment::Unknown,
    }
}

/// Whether the packaged window's file was replaced under it. A development
/// build that cargo rebuilt is not an update.
pub fn window_replaced(exe: &Path) -> bool {
    let text = exe.to_string_lossy();
    match text.strip_suffix(DELETED) {
        Some(path) => runs_from_package(Path::new(path)),
        None => false,
    }
}

/// "Restart to finish updating": the engine or the window is behind what is
/// installed. An unknown or conflicting engine never asks for it.
pub fn restart_pending(engine: &Alignment, window_replaced: bool) -> bool {
    window_replaced || *engine == Alignment::PendingRestart
}
