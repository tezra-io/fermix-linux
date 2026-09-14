//! The client against the fixture daemon, over a real socket with real framing.
//!
//! This is the one test that exercises the whole management path end to end:
//! the binary the development loop and the captures use answers the goldens the
//! engine published, and the client decodes them into the structs the surfaces
//! read.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use fermix_desktop::management::contract;
use fermix_desktop::management::errors::{ManagementError, TransportError};
use fermix_desktop::management::types::*;
use fermix_desktop::management::ManagementClient;
use fermix_desktop::testing::TempDirectory;
use gtk4::glib::MainContext;

const DEADLINE: Duration = Duration::from_secs(5);

/// A fixture daemon that is killed and reaped when the test ends, whether it
/// passed or panicked.
struct Fixture {
    child: Option<Child>,
    socket: PathBuf,
    _directory: TempDirectory,
}

impl Fixture {
    fn start(label: &str, scenario: &str) -> Self {
        let directory = TempDirectory::new(label);

        let mut child = Command::new(env!("CARGO_BIN_EXE_fixture-daemon"))
            .arg(directory.path())
            .env("FIXTURE_DAEMON_SCENARIO", scenario)
            .stdout(Stdio::piped())
            .spawn()
            .expect("the fixture daemon starts");

        let stdout = child
            .stdout
            .take()
            .expect("the fixture daemon announces its socket");
        let mut announced = String::new();
        BufReader::new(stdout)
            .read_line(&mut announced)
            .expect("the announcement arrives");

        let socket = PathBuf::from(announced.trim());
        assert_eq!(
            socket,
            directory.join("daemon.sock"),
            "the announced path is the socket"
        );

        Self {
            child: Some(child),
            socket,
            _directory: directory,
        }
    }

    fn client(&self) -> ManagementClient {
        ManagementClient::new(&self.socket)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    MainContext::new().block_on(future)
}

#[test]
fn hello_negotiates_the_highest_version_both_halves_speak() {
    let fixture = Fixture::start("fixture-hello", "default");
    let client = fixture.client();

    let hello: HelloResult = block_on(client.hello(DEADLINE))
        .value
        .expect("hello succeeds");

    assert_eq!(
        client.negotiated_version(),
        Some(contract::supported_range().max)
    );
    assert_eq!(
        hello.protocol.maximum_version,
        contract::supported_range().max
    );
    assert!(!hello.engine.product_version.is_empty());
    assert!(hello
        .capabilities
        .methods
        .contains(&"overview.get".to_string()));
}

#[test]
fn the_surfaces_read_their_own_methods() {
    let fixture = Fixture::start("fixture-reads", "default");
    let client = fixture.client();

    let overview: OverviewResult = block_on(client.call("overview.get", DEADLINE))
        .value
        .expect("overview.get succeeds");
    assert!(overview.capabilities.total > 0);

    let state: SetupStateResult = block_on(client.call("setup.state.get", DEADLINE))
        .value
        .expect("setup.state.get succeeds");
    assert!(!state.providers.is_empty());

    let sections: SettingsSectionsResult = block_on(client.call("settings.sections", DEADLINE))
        .value
        .expect("settings.sections succeeds");
    assert!(!sections.sections.is_empty());

    let plugins: PluginsListResult = block_on(client.call("plugins.list", DEADLINE))
        .value
        .expect("plugins.list succeeds");
    assert!(!plugins.plugins.is_empty());

    let logs: LogsQueryResult = block_on(client.request(
        "logs.query",
        &LogsQueryParams {
            limit: Some(200),
            level: None,
            subsystem: None,
            search: None,
            direction: None,
            cursor: None,
        },
        DEADLINE,
    ))
    .value
    .expect("logs.query succeeds");
    assert_eq!(logs.entries.len() as u32, logs.count);
}

#[test]
fn one_section_is_served_per_call() {
    let fixture = Fixture::start("fixture-section", "default");
    let client = fixture.client();

    for section in ["memory", "personalization", "sandbox", "realtime"] {
        let answer: SettingsGetResult = block_on(client.request(
            "settings.get",
            &SettingsGetParams {
                section: section.to_string(),
            },
            DEADLINE,
        ))
        .value
        .unwrap_or_else(|error| panic!("settings.get {section} failed: {error}"));

        assert_eq!(answer.id, section);
        assert!(!answer.rows.is_empty(), "{section} publishes no rows");
    }
}

#[test]
fn a_request_this_daemon_has_no_golden_for_is_refused_with_a_published_envelope() {
    let fixture = Fixture::start("fixture-unknown", "default");
    let client = fixture.client();

    // Every published method has a golden, so the absent case is a section
    // this build's goldens do not carry. The fixture daemon composes nothing:
    // the refusal is the envelope the engine published for that code.
    let refused = block_on(client.request::<_, serde_json::Value>(
        "settings.get",
        &SettingsGetParams {
            section: "a_section_from_the_future".to_string(),
        },
        DEADLINE,
    ));

    match refused.value {
        Err(ManagementError::Wire(error)) => {
            assert_eq!(error.code, "method_not_found");
            assert_eq!(error.message, published_message("method_not_found"));
        }
        other => panic!("expected the published refusal, got {other:?}"),
    }
}

/// The fixed sentence the engine publishes for one error code.
fn published_message(code: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("contracts/management/fixtures/errors.jsonl");

    std::fs::read_to_string(path)
        .expect("the error goldens read")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("a golden parses"))
        .find(|record| record["code"] == code)
        .and_then(|record| {
            record["response"]["error"]["message"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no golden for {code}"))
}

#[test]
fn the_setup_required_scenario_reports_a_gating_failure() {
    let fixture = Fixture::start("fixture-setup", "setup_required");
    let client = fixture.client();

    let state: SetupStateResult = block_on(client.call("setup.state.get", DEADLINE))
        .value
        .expect("setup.state.get succeeds");

    let gating: Vec<&ReadinessFailure> = state
        .readiness
        .failures
        .iter()
        .filter(|failure| failure.gating)
        .collect();

    assert_eq!(gating.len(), 1);
    assert_eq!(gating[0].detail_key, "personalization");
    assert_eq!(gating[0].pane, SettingsPane::Personality);
    assert!(!state.restart.required);
}

#[test]
fn the_restart_pending_scenario_carries_the_daemons_own_reasons() {
    let fixture = Fixture::start("fixture-restart", "restart_pending");
    let client = fixture.client();

    let state: SetupStateResult = block_on(client.call("setup.state.get", DEADLINE))
        .value
        .expect("setup.state.get succeeds");

    assert!(state.restart.required);
    assert_eq!(state.restart.reasons.len(), 2);
    assert!(state
        .restart
        .reasons
        .iter()
        .all(|reason| !reason.sentence.is_empty()));

    let overview: OverviewResult = block_on(client.call("overview.get", DEADLINE))
        .value
        .expect("overview.get succeeds");
    assert!(overview.health.restart_required);
}

#[test]
fn the_external_change_scenario_refuses_every_write_until_the_reload() {
    let fixture = Fixture::start("fixture-external", "external_change");
    let client = fixture.client();

    let state: SetupStateResult = block_on(client.call("setup.state.get", DEADLINE))
        .value
        .expect("setup.state.get succeeds");
    assert_eq!(state.coexistence.config_state, ConfigState::ExternalChange);

    let refused = block_on(client.request::<_, serde_json::Value>(
        "settings.apply",
        &SettingsApplyParams {
            section: "memory".to_string(),
            values: Default::default(),
        },
        DEADLINE,
    ));

    match refused.value {
        Err(ManagementError::Wire(error)) => {
            assert_eq!(error.code, "external_change");
            assert_eq!(error.detail("section"), Some("memory"));
        }
        other => panic!("expected the write to be refused, got {other:?}"),
    }
}

#[test]
fn the_unreadable_scenario_carries_the_parsers_own_sentence_and_no_reload() {
    let fixture = Fixture::start("fixture-unreadable", "unreadable");
    let client = fixture.client();

    let state: SetupStateResult = block_on(client.call("setup.state.get", DEADLINE))
        .value
        .expect("setup.state.get succeeds");
    assert_eq!(
        state.coexistence.config_state,
        ConfigState::ConfigUnreadable
    );

    let refused = block_on(client.call::<serde_json::Value>("settings.reload", DEADLINE));

    match refused.value {
        Err(ManagementError::Wire(error)) => {
            assert_eq!(error.code, "config_unreadable");
            assert!(
                error.sentence.is_some(),
                "the parser's own message is what a person is shown"
            );
        }
        other => panic!("expected the reload to be refused, got {other:?}"),
    }
}

#[test]
fn the_not_running_scenario_announces_a_socket_nothing_answers_on() {
    let fixture = Fixture::start("fixture-absent", "not_running");
    let client = fixture.client();

    let answer = block_on(client.hello(DEADLINE));

    match answer.value {
        Err(ManagementError::Transport(TransportError::NotRunning)) => {}
        other => panic!("expected NotRunning, got {other:?}"),
    }
}

#[test]
fn the_web_door_is_on_a_port_nothing_else_on_this_machine_holds() {
    // The vendored golden names 4030, which is the port a developer's own
    // Fermix daemon holds and the port every other fixture daemon would want.
    // This peer opens one the operating system chose and says so in `hello`, so
    // two of these at once both work and neither ever asks a real daemon
    // whether it is alive.
    let fixture = Fixture::start("fixture-health", "default");
    let client = fixture.client();

    let hello: HelloResult = block_on(client.hello(DEADLINE))
        .value
        .expect("hello succeeds");

    let published = published_origin();
    assert_ne!(
        hello.setup.origin, published,
        "the served origin is this process's door rather than the golden's"
    );

    let authority = hello
        .setup
        .origin
        .strip_prefix("http://")
        .expect("a loopback origin");
    assert!(authority.starts_with("127.0.0.1:"), "{authority}");

    let mut door = std::net::TcpStream::connect(authority).expect("the door accepts");
    std::io::Write::write_all(&mut door, b"GET /health/live HTTP/1.0\r\n\r\n")
        .expect("the request is written");

    let mut answer = String::new();
    std::io::Read::read_to_string(&mut door, &mut answer).expect("the door answers");
    assert!(answer.starts_with("HTTP/1.0 200 "), "{answer}");
}

#[test]
fn a_scenario_that_declares_no_web_door_opens_none() {
    // Port zero is how a scenario says there is nothing to ask, which is what
    // holds the ladder's "checking that it answers" row open.
    let fixture = Fixture::start("fixture-no-door", "onboarding_starting");
    let client = fixture.client();

    let hello: HelloResult = block_on(client.hello(DEADLINE))
        .value
        .expect("hello succeeds");

    assert!(
        hello.setup.origin.ends_with(":0"),
        "the declared absence is served unchanged: {}",
        hello.setup.origin
    );
}

/// The origin the vendored `hello` golden publishes.
fn published_origin() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("contracts/management/fixtures/success.jsonl");

    std::fs::read_to_string(path)
        .expect("the success goldens read")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("a golden parses"))
        .find(|record| record["method"] == "hello")
        .and_then(|record| {
            record["response"]["result"]["setup"]["origin"]
                .as_str()
                .map(str::to_string)
        })
        .expect("the hello golden names an origin")
}
