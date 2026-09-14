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
use gtk4::prelude::*;

fn main() -> gtk4::glib::ExitCode {
    let arguments: Vec<String> = std::env::args().collect();

    if cli::wants_version(&arguments) {
        println!("{}", cli::version_line());
        return gtk4::glib::ExitCode::SUCCESS;
    }

    let application = FermixApplication::new(Paths::resolve());

    #[cfg(debug_assertions)]
    if let Some(directory) = fermix_desktop::capture::directory_from(&arguments) {
        fermix_desktop::capture::arm(&application, directory);
    }

    // Everything this binary does not own itself reaches the toolkit, which is
    // what lets the activation unit start the window with
    // `--gapplication-service` and the launcher hand it a `fermix:` address.
    application.run_with_args(&cli::toolkit_arguments(&arguments))
}
