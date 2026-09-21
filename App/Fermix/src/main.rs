//! The application binary.
//!
//! It answers the one question this process answers without a window, resolves
//! the paths it runs against, and hands everything else to the toolkit. In a
//! release build the paths are the packaged command line and nothing else; the
//! two development configurations, and the capture mode below, are read only
//! under `cfg(debug_assertions)` and are declared configurations rather than a
//! runtime branch.

use fermix_desktop::app::FermixApplication;
use fermix_desktop::cli;
use fermix_desktop::paths::Paths;
use fermix_desktop::runtime::RuntimeEnv;
use gtk4::prelude::*;

fn main() -> gtk4::glib::ExitCode {
    // First, before anything else in this process. It writes the environment
    // this build's private GTK needs, which is sound only while this process
    // has one thread, and it records what each variable held so that every
    // child this application spawns gets the environment it was started with.
    // `src/runtime.rs` says why both halves are requirements.
    let runtime = RuntimeEnv::capture();

    let arguments: Vec<String> = std::env::args().collect();

    if cli::wants_version(&arguments) {
        println!("{}", cli::version_line());
        return gtk4::glib::ExitCode::SUCCESS;
    }

    let application = FermixApplication::new(Paths::resolve(), runtime);

    #[cfg(debug_assertions)]
    if let Some(directory) = fermix_desktop::capture::directory_from(&arguments) {
        fermix_desktop::capture::arm(&application, directory);
    }

    // Everything this binary does not own itself reaches the toolkit, which is
    // what lets the activation unit start the window with
    // `--gapplication-service` and the launcher hand it a `fermix:` address.
    application.run_with_args(&cli::toolkit_arguments(&arguments))
}
