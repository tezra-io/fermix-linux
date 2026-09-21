//! The five typed CLI operations, against the fake `fermix`.
//!
//! Nothing here runs `/usr/bin/fermix`, reads a real binding or touches a real
//! service. Each test points the runner at a one-line wrapper it writes into its
//! own temporary directory, which names the state the fake command line answers
//! from. The wrapper exists so the state is chosen per invocation rather than by
//! mutating this process's environment, which several tests running at once
//! would race on.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fermix_desktop::runtime::{RuntimeEnv, SCHEMA_DIR_VARIABLE};
use fermix_desktop::service::runner::{ServiceError, OUTPUT_CEILING};
use fermix_desktop::service::types::{ActionKind, ActionScope, Alignment, BindingState, Linger};
use fermix_desktop::service::ServiceRunner;
use fermix_desktop::testing::TempDirectory;
use gtk4::gio;
use gtk4::gio::prelude::CancellableExt;
use gtk4::glib::MainContext;

fn fake_cli() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/fermix")
}

/// One wrapper per state, written once, before any test spawns anything.
///
/// Written once rather than per test, and this is not tidiness: a wrapper
/// written and executed inside one test races every other test in the binary.
/// The tests run on several threads, a spawn forks the whole process, and a
/// fork that inherits another thread's still-open write handle to the file it
/// is about to execute fails with "Text file busy" on Linux. Creating them all
/// before the first spawn removes the race rather than retrying through it.
fn wrappers() -> &'static std::path::Path {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    static ONCE: std::sync::Once = std::sync::Once::new();

    let root = ROOT.get_or_init(|| {
        let directory = Box::leak(Box::new(TempDirectory::new("cli-wrappers")));
        directory.path().to_path_buf()
    });

    ONCE.call_once(|| {
        let states = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli/states");
        for entry in std::fs::read_dir(states).expect("the states are readable") {
            let state = entry.expect("a state directory").path();
            let name = state
                .file_name()
                .expect("a named state")
                .to_string_lossy()
                .into_owned();

            let wrapper = root.join(&name);
            std::fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nFERMIX_FAKE_CLI_STATE='{}' exec '{}' \"$@\"\n",
                    state.display(),
                    fake_cli().display()
                ),
            )
            .expect("the wrapper is written");
            make_executable(&wrapper);
        }

        // One more stand-in, written here for the same reason the others are:
        // it answers with the environment it was given rather than from a
        // state, which is how the environment gate below reads what a child
        // actually saw.
        let probe = root.join(ENVIRONMENT_PROBE);
        std::fs::write(
            &probe,
            format!(
                "#!/bin/sh\nprintf '{{\"schema_version\":1,\"ok\":false,\
                 \"error\":{{\"code\":\"environment\",\"sentence\":\"%s\"}}}}' \
                 \"${{{SCHEMA_DIR_VARIABLE}-{UNSET}}}\"\nexit 1\n"
            ),
        )
        .expect("the probe is written");
        make_executable(&probe);
    });

    root
}

/// The stand-in that answers with the environment it was handed.
const ENVIRONMENT_PROBE: &str = "environment-probe";

/// What the probe prints when the variable is not set at all, which is a
/// different answer from an empty one.
const UNSET: &str = "unset";

/// A command line answering from one state, without this process's environment
/// being touched.
fn runner_for(state_name: &str) -> ServiceRunner {
    let wrapper = wrappers().join(state_name);
    assert!(
        wrapper.exists(),
        "no fake command line for {state_name}: {}",
        wrapper.display()
    );

    ServiceRunner::new(wrapper)
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)
        .expect("the wrapper exists")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("the wrapper is executable");
}

fn uncancelled() -> gio::Cancellable {
    gio::Cancellable::new()
}

// ---------------------------------------------------------------------------
// The five operations
// ---------------------------------------------------------------------------

#[test]
fn service_status_parses_its_envelope() {
    let runner = runner_for("active_aligned");

    let status = MainContext::new()
        .block_on(async { runner.status(&uncancelled()).await })
        .expect("the status parses");

    assert_eq!(status.alignment, Alignment::Aligned);
    assert!(status.enabled);
    assert!(status.active);
    assert_eq!(status.linger, Linger::Enabled);
    assert_eq!(status.binding.state, BindingState::Bound);
    assert_eq!(status.bound_home(), Some("/home/operator/.fermix"));
    assert_eq!(status.installed.build_id.as_deref(), Some("release-9"));
    assert_eq!(status.listener.port, Some(4030));
}

#[test]
fn a_fresh_host_reports_no_binding_and_nothing_running() {
    let runner = runner_for("fresh");

    let status = MainContext::new()
        .block_on(async { runner.status(&uncancelled()).await })
        .expect("the status parses");

    assert_eq!(status.alignment, Alignment::NotRunning);
    assert_eq!(status.binding.state, BindingState::Unbound);
    assert_eq!(status.bound_home(), None);
    assert!(status.running.is_none());
}

#[test]
fn a_newer_engine_installed_reports_pending_restart() {
    let runner = runner_for("pending_restart");

    let status = MainContext::new()
        .block_on(async { runner.status(&uncancelled()).await })
        .expect("the status parses");

    assert_eq!(status.alignment, Alignment::PendingRestart);
    let installed = status.installed.build_id.clone();
    let running = status
        .running
        .as_ref()
        .and_then(|engine| engine.build_id.clone());
    assert_ne!(installed, running, "a build id is compared with a build id");
}

#[test]
fn a_packaged_install_answers_with_the_service_state_it_left_behind() {
    let runner = runner_for("install_ok");

    let installed = MainContext::new()
        .block_on(async { runner.install(None, None, &uncancelled()).await })
        .expect("the install parses");

    // The command line runs the whole transaction and answers once. There are
    // no phases to read: the ladder is driven by that one answer.
    let status = installed
        .status()
        .expect("a packaged engine answers with the service result");
    assert_eq!(status.alignment, Alignment::Aligned);
    assert!(status.enabled);
    assert!(status.active);
    assert_eq!(status.linger, Linger::Enabled);
}

#[test]
fn an_engine_that_writes_its_own_unit_answers_with_what_it_did() {
    let runner = runner_for("install_unit");

    let installed = MainContext::new()
        .block_on(async { runner.install(None, None, &uncancelled()).await })
        .expect("the install parses");

    assert!(
        installed.status().is_none(),
        "an action result is not a service state"
    );
}

#[test]
fn service_install_takes_a_home_and_a_port_as_arguments() {
    let runner = runner_for("install_ok");

    let installed = MainContext::new()
        .block_on(async {
            runner
                .install(
                    Some(Path::new("/home/operator/.fermix")),
                    Some(4031),
                    &uncancelled(),
                )
                .await
        })
        .expect("the install parses");

    assert_eq!(
        installed.status().map(|status| status.bound_home()),
        Some(Some("/home/operator/.fermix"))
    );
}

#[test]
fn service_uninstall_answers_with_what_it_did() {
    let runner = runner_for("active_aligned");

    let uninstalled = MainContext::new()
        .block_on(async { runner.uninstall(&uncancelled()).await })
        .expect("the uninstall parses");

    assert_eq!(uninstalled.action, ActionKind::Uninstalled);
    assert_eq!(uninstalled.scope, ActionScope::User);
}

#[test]
fn restart_parses_its_envelope() {
    let runner = runner_for("restart_ok");

    let restarted = MainContext::new()
        .block_on(async { runner.restart(false, &uncancelled()).await })
        .expect("the restart parses");

    assert_eq!(restarted.previous_pid.as_deref(), Some("4711"));
    assert_ne!(
        restarted.pid, "4711",
        "an answer alone is not a new generation"
    );
    assert_eq!(restarted.alignment, Alignment::Aligned);
}

#[test]
fn a_restart_that_found_nothing_running_is_a_recovery_rather_than_a_refusal() {
    let runner = runner_for("restart_recovered");

    let restarted = MainContext::new()
        .block_on(async { runner.restart(false, &uncancelled()).await })
        .expect("the restart parses");

    assert!(restarted.previous_pid.is_none());
    assert_eq!(restarted.alignment, Alignment::Aligned);
}

#[test]
fn diagnostics_export_parses_its_envelope() {
    let runner = runner_for("export_ok");

    let export = MainContext::new()
        .block_on(async { runner.export_diagnostics(&uncancelled()).await })
        .expect("the export parses");

    assert_eq!(export.schema_version, 1);
    assert_eq!(export.mode, "offline");
    assert!(
        export.sources.contains_key("logs"),
        "the object the user writes to a file"
    );
}

#[test]
fn a_degraded_export_says_why_each_source_is_missing_rather_than_omitting_it() {
    let runner = runner_for("export_degraded");

    let export = MainContext::new()
        .block_on(async { runner.export_diagnostics(&uncancelled()).await })
        .expect("the export parses");

    let service = export.sources.get("service").expect("a service source");
    assert!(
        service.reason.is_some(),
        "nothing omitted is read as healthy"
    );
    assert!(service.data.is_none());
}

// ---------------------------------------------------------------------------
// Refusals
// ---------------------------------------------------------------------------

#[test]
fn a_non_zero_exit_with_an_error_envelope_carries_the_command_lines_sentence() {
    let runner = runner_for("linger_denied");

    let refused =
        MainContext::new().block_on(async { runner.install(None, None, &uncancelled()).await });

    match refused {
        Err(ServiceError::Refused { code, sentence }) => {
            assert_eq!(code, "linger_denied");
            assert!(
                sentence.contains("after you log out"),
                "the sentence a retry needs survives: {sentence}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn a_foreign_unit_is_published_by_the_read_and_refuses_the_change() {
    // The two halves are one fact seen from two verbs: the status names the
    // unit it found, and the verb that would have rewritten it refuses.
    let status = MainContext::new()
        .block_on(async { runner_for("foreign_unit").status(&uncancelled()).await })
        .expect("the status parses");
    assert!(status.unit.foreign);
    assert!(!status.unit.vendor);

    let refused = MainContext::new().block_on(async {
        runner_for("foreign_unit_refused")
            .install(None, None, &uncancelled())
            .await
    });

    match refused {
        Err(ServiceError::Refused { code, sentence }) => {
            assert_eq!(code, "foreign_unit");
            assert!(
                sentence.contains("Remove or rename it"),
                "the one thing a person can do: {sentence}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn an_idle_restart_the_engine_cannot_serve_refuses_with_its_reason() {
    let runner = runner_for("restart_idle_unavailable");

    let refused = MainContext::new().block_on(async { runner.restart(true, &uncancelled()).await });

    match refused {
        Err(ServiceError::Refused { code, sentence }) => {
            assert_eq!(code, "idle_restart_unavailable");
            assert!(
                sentence.contains("interrupts any work in progress"),
                "the dialog shows this beside the disabled action: {sentence}"
            );
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn every_published_refusal_reaches_a_caller_with_its_code_and_its_sentence() {
    // One state per code the command line publishes, each answering from the
    // engine's own golden. A code that stopped being decodable here is a
    // refusal a surface would render as "something went wrong".
    let cases: &[(&str, &str, Verb)] = &[
        ("app_managed", "app_managed", Verb::Status),
        (
            "user_manager_unreachable",
            "user_manager_unreachable",
            Verb::Status,
        ),
        ("foreign_distribution", "foreign_distribution", Verb::Status),
        ("linger_denied", "linger_denied", Verb::Install),
        ("loginctl_absent", "loginctl_absent", Verb::Install),
        ("no_identity", "no_identity", Verb::Install),
        ("invalid_home", "invalid_home", Verb::Install),
        ("home_change_refused", "home_change_refused", Verb::Install),
        ("foreign_unit_refused", "foreign_unit", Verb::Install),
        (
            "install_activation_timeout",
            "activation_timeout",
            Verb::Install,
        ),
        ("health_unavailable", "health_unavailable", Verb::Install),
        ("invalid_port", "invalid_port", Verb::Install),
        ("config_write_failed", "config_write_failed", Verb::Install),
        (
            "binding_write_failed",
            "binding_write_failed",
            Verb::Install,
        ),
        ("systemctl_failed", "systemctl_failed", Verb::Install),
        ("service_unbound", "service_unbound", Verb::Restart),
        (
            "restart_idle_unavailable",
            "idle_restart_unavailable",
            Verb::Restart,
        ),
        ("restart_refused", "lifecycle_refused", Verb::Restart),
        (
            "diagnostics_unavailable",
            "diagnostics_unavailable",
            Verb::Export,
        ),
    ];

    for (state, expected, verb) in cases {
        let runner = runner_for(state);
        let refused = MainContext::new().block_on(async {
            match verb {
                Verb::Status => runner.status(&uncancelled()).await.map(|_| ()),
                Verb::Install => runner.install(None, None, &uncancelled()).await.map(|_| ()),
                Verb::Restart => runner.restart(false, &uncancelled()).await.map(|_| ()),
                Verb::Export => runner.export_diagnostics(&uncancelled()).await.map(|_| ()),
            }
        });

        match refused {
            Err(ServiceError::Refused { code, sentence }) => {
                assert_eq!(&code, expected, "the {state} state");
                assert!(!sentence.is_empty(), "{state} refused without a sentence");
            }
            other => panic!("expected {expected} from {state}, got {other:?}"),
        }
    }
}

/// The four typed operations a refusal can arrive from.
enum Verb {
    Status,
    Install,
    Restart,
    Export,
}

#[test]
fn output_beyond_the_ceiling_is_refused() {
    let runner = runner_for("oversize");

    let refused = MainContext::new().block_on(async { runner.status(&uncancelled()).await });

    match refused {
        Err(ServiceError::Output(reason)) => {
            assert!(reason.contains(&OUTPUT_CEILING.to_string()), "{reason}");
        }
        other => panic!("expected the output to be refused, got {other:?}"),
    }
}

#[test]
fn a_command_line_that_cannot_run_is_a_launch_failure() {
    let directory = TempDirectory::new("cli-missing");
    let runner = ServiceRunner::new(directory.join("not-installed"));

    let refused = MainContext::new().block_on(async { runner.status(&uncancelled()).await });

    match refused {
        Err(ServiceError::Launch(_)) => {}
        other => panic!("expected a launch failure, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The environment the engine is spawned with
// ---------------------------------------------------------------------------

/// The sentence the probe answered with, which is the value of the one variable
/// the private toolkit runtime sets.
fn variable_seen_by_the_child(runtime: RuntimeEnv) -> String {
    let runner = ServiceRunner::new(wrappers().join(ENVIRONMENT_PROBE)).with_runtime_env(runtime);

    let refused = MainContext::new().block_on(async { runner.status(&uncancelled()).await });

    match refused {
        Err(ServiceError::Refused { code, sentence }) => {
            assert_eq!(code, "environment", "the probe answered something else");
            sentence
        }
        other => panic!("the probe did not answer: {other:?}"),
    }
}

#[test]
fn the_engine_is_spawned_with_the_environment_this_process_started_with() {
    // The failure this gate exists for: the window sets a schema directory for
    // its own private GTK, the engine inherits it, and every GSettings read the
    // engine makes resolves against a toolkit it was not built against. The
    // child gets the value the session had, or none where the session had none.
    let session_value = "/usr/share/gnome/glib-2.0/schemas";

    assert_eq!(
        variable_seen_by_the_child(RuntimeEnv::restoring(vec![(
            SCHEMA_DIR_VARIABLE.to_string(),
            Some(session_value.to_string()),
        )])),
        session_value
    );

    assert_eq!(
        variable_seen_by_the_child(RuntimeEnv::restoring(vec![(
            SCHEMA_DIR_VARIABLE.to_string(),
            None,
        )])),
        UNSET,
        "a variable the session never set must reach the engine unset"
    );
}

// ---------------------------------------------------------------------------
// Deadlines and cancellation
// ---------------------------------------------------------------------------

#[test]
fn a_child_that_never_answers_ends_at_its_deadline() {
    let runner =
        runner_for("hang").with_deadlines(Duration::from_millis(300), Duration::from_millis(300));

    let started = Instant::now();
    let refused = MainContext::new().block_on(async { runner.status(&uncancelled()).await });

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the deadline ended it, not the child"
    );
    match refused {
        Err(ServiceError::Timeout) => {}
        other => panic!("expected the deadline to fire, got {other:?}"),
    }
}

#[test]
fn cancelling_ends_a_child_that_never_answers() {
    let runner =
        runner_for("hang").with_deadlines(Duration::from_secs(30), Duration::from_secs(30));

    let cancellable = gio::Cancellable::new();
    let started = Instant::now();

    let refused = MainContext::new().block_on(async {
        let (result, ()) = futures_util::future::join(runner.status(&cancellable), async {
            gtk4::glib::timeout_future(Duration::from_millis(150)).await;
            cancellable.cancel();
        })
        .await;
        result
    });

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the cancellation ended it, well inside the deadline"
    );
    assert!(refused.is_err(), "a cancelled operation has no result");
}

#[test]
fn an_already_cancelled_operation_never_spawns() {
    let runner = runner_for("hang");

    let cancellable = gio::Cancellable::new();
    cancellable.cancel();

    let started = Instant::now();
    let refused = MainContext::new().block_on(async { runner.status(&cancellable).await });

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(refused.is_err());
}

#[test]
fn the_published_deadlines_are_the_ones_a_plain_runner_uses() {
    use fermix_desktop::service::runner::{MUTATION_DEADLINE, READ_DEADLINE};

    let runner = ServiceRunner::new("/usr/bin/fermix");
    assert_eq!(runner.read_deadline(), READ_DEADLINE);
    assert_eq!(runner.mutation_deadline(), MUTATION_DEADLINE);
    assert_eq!(READ_DEADLINE, Duration::from_secs(30));
    assert_eq!(MUTATION_DEADLINE, Duration::from_secs(120));
}
