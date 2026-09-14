//! Compile the GResource bundle, and stamp this build's own identity.
//!
//! One bundle, built at compile time, so the application performs no runtime
//! file lookup for its icons, its brand artwork or its stylesheet.

/// What a build nobody stamped carries. The application compares a build id
/// with a build id and treats this one as no id at all, so a development window
/// never claims to be older or newer than an installed application.
const DEVELOPMENT_BUILD_ID: &str = "dev";

fn main() {
    glib_build_tools::compile_resources(
        &["resources"],
        "resources/fermix.gresource.xml",
        "fermix.gresource",
    );

    stamp_build_id();

    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=contracts/management/protocol.schema.json");
}

/// The build id this binary compares with `/usr/share/fermix-desktop/build.json`
/// (M38 section 9.2).
///
/// The release rail sets `FERMIX_DESKTOP_BUILD_ID` and writes the same value
/// into the manifest the package installs, so the two agree by construction
/// rather than by anybody remembering to edit both.
fn stamp_build_id() {
    let stamped = std::env::var("FERMIX_DESKTOP_BUILD_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| DEVELOPMENT_BUILD_ID.to_string());

    println!("cargo:rustc-env=FERMIX_DESKTOP_BUILD_ID={stamped}");
    println!("cargo:rerun-if-env-changed=FERMIX_DESKTOP_BUILD_ID");
}
