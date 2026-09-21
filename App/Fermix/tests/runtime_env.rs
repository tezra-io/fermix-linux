//! The environment a child gets.
//!
//! The private toolkit runtime needs one variable set in this process, and no
//! child may inherit it: the packaged `fermix` and the person's browser both
//! start as children of this process, and a GTK 4.6 host application that
//! starts with a GTK 4.16 schema directory prepended aborts on the first
//! setting it reads that the host schemas do not carry.
//!
//! These tests prove the two halves of that separately and then together: the
//! launcher the CLI is spawned through, the launch context a URI is opened
//! through, and a real child whose printed environment is compared with the
//! environment this test process was started with. Nothing here needs a
//! display, a browser or a daemon.

use std::ffi::OsString;

use fermix_desktop::runtime::{RuntimeEnv, SCHEMA_DIR_VARIABLE};
use fermix_desktop::testing::TempDirectory;
use gtk4::gio;
use gtk4::prelude::*;

/// A value that stands for whatever the session had before this process ran.
const PRIOR: &str = "/usr/share/gnome/glib-2.0/schemas";

/// What this process would have set, which is the thing no child may see.
const PRIVATE: &str = "/usr/lib/fermix-desktop/share/glib-2.0/schemas";

/// One variable out of a launch context's environment block.
fn value_in(environment: &[OsString], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    environment
        .iter()
        .filter_map(|entry| entry.to_str())
        .find_map(|entry| entry.strip_prefix(&prefix))
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// The launch context a URI is opened through
// ---------------------------------------------------------------------------

#[test]
fn a_launch_context_carries_the_value_the_session_had() {
    let runtime = RuntimeEnv::restoring(vec![(
        SCHEMA_DIR_VARIABLE.to_string(),
        Some(PRIOR.to_string()),
    )]);

    let environment = runtime.launch_context().environment();

    assert_eq!(
        value_in(&environment, SCHEMA_DIR_VARIABLE).as_deref(),
        Some(PRIOR),
        "the browser must get the schema directory the session had, not this process's"
    );
    assert!(
        !environment
            .iter()
            .any(|entry| entry.to_string_lossy().contains(PRIVATE)),
        "the private prefix reached a launch context"
    );
}

#[test]
fn a_launch_context_unsets_a_variable_the_session_never_had() {
    let runtime = RuntimeEnv::restoring(vec![(SCHEMA_DIR_VARIABLE.to_string(), None)]);

    let environment = runtime.launch_context().environment();

    assert_eq!(
        value_in(&environment, SCHEMA_DIR_VARIABLE),
        None,
        "an absent variable is restored by unsetting it, never by setting it empty"
    );
}

#[test]
fn a_process_that_set_nothing_hands_a_launch_context_its_own_environment() {
    // The development and test configuration. Nothing was set, so there is
    // nothing to put back, and the context is the process's own environment.
    let environment = RuntimeEnv::unchanged().launch_context().environment();

    assert_eq!(
        value_in(&environment, "PATH"),
        std::env::var("PATH").ok(),
        "an unchanged runtime altered a variable it never recorded"
    );
}

// ---------------------------------------------------------------------------
// A real child
// ---------------------------------------------------------------------------

/// A one-line child that prints one variable, or an empty line when it is not
/// set at all. Written before anything spawns, for the reason
/// `tests/service_runner.rs` gives: a fork that inherits another thread's open
/// write handle to the file it is about to execute fails with "Text file busy".
fn environment_printer(directory: &TempDirectory) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let script = directory.join("print-environment");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"${{{SCHEMA_DIR_VARIABLE}-}}\"\n"),
    )
    .expect("the child is written");

    let mut permissions = std::fs::metadata(&script)
        .expect("the child exists")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&script, permissions).expect("the child is executable");

    script
}

/// What a child spawned through this runtime's launcher printed.
fn spawn_and_read(runtime: &RuntimeEnv, script: &std::path::Path) -> String {
    let child = runtime
        .launcher(gio::SubprocessFlags::STDOUT_PIPE)
        .spawn(&[script.as_os_str()])
        .expect("the child spawns");

    // Synchronous, because this test is the only thing in the process and a
    // main context would buy nothing: the child prints one line and exits.
    let (stdout, _stderr) = child
        .communicate_utf8(None, gio::Cancellable::NONE)
        .expect("the child answers");

    stdout.map(|line| line.to_string()).unwrap_or_default()
}

#[test]
fn a_child_spawned_through_the_launcher_sees_the_environment_this_process_started_with() {
    let directory = TempDirectory::new("runtime-child");
    let script = environment_printer(&directory);

    let restored = RuntimeEnv::restoring(vec![(
        SCHEMA_DIR_VARIABLE.to_string(),
        Some(PRIOR.to_string()),
    )]);
    assert_eq!(
        spawn_and_read(&restored, &script),
        PRIOR,
        "the packaged command line must not inherit the private toolkit's schema directory"
    );

    let unset = RuntimeEnv::restoring(vec![(SCHEMA_DIR_VARIABLE.to_string(), None)]);
    assert_eq!(
        spawn_and_read(&unset, &script),
        "",
        "a variable the session never set must reach a child unset"
    );
}

#[test]
fn a_child_of_an_unchanged_runtime_inherits_this_process_whole() {
    let directory = TempDirectory::new("runtime-child-plain");
    let script = environment_printer(&directory);

    assert_eq!(
        spawn_and_read(&RuntimeEnv::unchanged(), &script),
        std::env::var(SCHEMA_DIR_VARIABLE).unwrap_or_default(),
        "an unchanged runtime spawns with this process's own environment"
    );
}

// ---------------------------------------------------------------------------
// One variable, and only one
// ---------------------------------------------------------------------------

/// Every `.rs` file in the crate's own source tree.
fn crate_sources() -> Vec<(String, String)> {
    fn walk(directory: &std::path::Path, found: &mut Vec<(String, String)>) {
        let entries = std::fs::read_dir(directory).expect("the source tree reads");
        for entry in entries {
            let path = entry.expect("a source entry").path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|kind| kind == "rs") {
                let body = std::fs::read_to_string(&path).expect("a source file reads");
                found.push((path.display().to_string(), body));
            }
        }
    }

    let mut found = Vec::new();
    walk(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found
}

#[test]
fn the_application_writes_the_environment_in_exactly_one_place() {
    // `std::env::set_var` is sound only while this process has one thread, and
    // the whole arrangement depends on there being one caller, in `main`,
    // before anything else runs. A second call added anywhere else would be a
    // data race that no test would otherwise see, so the count is the gate.
    let calls: Vec<String> = crate_sources()
        .into_iter()
        .filter(|(_, body)| {
            body.lines()
                .any(|line| !line.trim_start().starts_with("//") && line.contains("env::set_var"))
        })
        .map(|(path, _)| path)
        .collect();

    assert_eq!(
        calls.len(),
        1,
        "the environment is written in {} places, and it may be written in one: {calls:?}",
        calls.len()
    );
    assert!(
        calls[0].ends_with("runtime.rs"),
        "the environment is written outside src/runtime.rs: {calls:?}"
    );
}

#[test]
fn the_application_sets_no_variable_the_private_runtime_already_knows() {
    // Each of these is compiled into the library that reads it, because the
    // runtime is configured at its final prefix rather than staged and
    // relocated. Setting one could only ever make it wrong, and would then
    // have to be unset for every child as well.
    //
    // `GSETTINGS_SCHEMA_DIR` is the single exception and is not on this list:
    // GLib has no compiled-in equivalent for it.
    for forbidden in ["GIO_MODULE_DIR", "GDK_PIXBUF_MODULE_FILE", "XDG_DATA_DIRS"] {
        for (path, body) in crate_sources() {
            let written = body.lines().any(|line| {
                let code = line.trim_start();
                !code.starts_with("//") && code.contains(forbidden) && code.contains("set_var")
            });
            assert!(
                !written,
                "{path} sets {forbidden}, and the library already knows it"
            );
        }
    }
}
