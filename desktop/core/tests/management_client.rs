//! The management client against a fake daemon on a real Unix socket.

use fermix_client::frame::{read_frame, write_frame};
use fermix_client::management::{CallError, Management, PROTOCOL_VERSION};
use serde_json::{json, Value};
use std::io::ErrorKind;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_millis(300);

/// What the fake daemon does with one connection.
enum Reply {
    /// Answer with this envelope; `$id` in a string is replaced by the request's id.
    Envelope(Value),
    /// Read the request, then hold the connection open without answering.
    Stall(Duration),
    /// Read the request, then close without answering.
    Hangup,
}

struct FakeDaemon {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    handle: JoinHandle<Vec<Value>>,
}

impl FakeDaemon {
    fn start(replies: Vec<Reply>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("daemon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = thread::spawn(move || {
            replies
                .into_iter()
                .map(|reply| serve_one(&listener, reply))
                .collect()
        });
        FakeDaemon {
            _dir: dir,
            socket,
            handle,
        }
    }

    fn client(&self) -> Management {
        Management::new(self.socket.clone(), TIMEOUT)
    }

    fn requests(self) -> Vec<Value> {
        self.handle.join().unwrap()
    }
}

/// Accepts one connection, reads its request, answers it as `reply` says, and
/// hands the request back for the test to inspect.
fn serve_one(listener: &UnixListener, reply: Reply) -> Value {
    let (mut stream, _) = listener.accept().unwrap();
    let request: Value = serde_json::from_slice(&read_frame(&mut stream).unwrap()).unwrap();
    let id = request["request_id"].as_str().unwrap().to_owned();
    match reply {
        Reply::Envelope(v) => {
            let text = serde_json::to_string(&v).unwrap().replace("$id", &id);
            write_frame(&mut stream, text.as_bytes()).unwrap();
        }
        Reply::Stall(d) => thread::sleep(d),
        Reply::Hangup => drop(stream),
    }
    request
}

#[test]
fn a_call_sends_one_versioned_request_and_returns_its_result() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "result": {"ok": true}}),
    )]);
    let result = daemon.client().call("hello", json!({})).unwrap();
    assert_eq!(result, json!({"ok": true}));
    let requests = daemon.requests();
    assert_eq!(requests[0]["method"], "hello");
    assert_eq!(requests[0]["protocol_version"], PROTOCOL_VERSION);
    assert_eq!(requests[0]["params"], json!({}));
}

#[test]
fn each_call_gets_its_own_request_id() {
    let ok = || Reply::Envelope(json!({"request_id": "$id", "result": {}}));
    let daemon = FakeDaemon::start(vec![ok(), ok()]);
    let client = daemon.client();
    client.call("hello", json!({})).unwrap();
    client.call("hello", json!({})).unwrap();
    let requests = daemon.requests();
    assert_ne!(requests[0]["request_id"], requests[1]["request_id"]);
}

#[test]
fn a_refusal_carries_the_daemons_specific_sentence() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "error": {
        "code": "invalid_params", "message": "Request parameters are invalid.",
        "details": {"field": "provider", "sentence": "This provider has no browser sign-in."}}}),
    )]);
    let err = daemon.client().auth_start("anthropic").unwrap_err();
    let CallError::Refused(refusal) = err else {
        panic!("expected a refusal, got {err:?}")
    };
    assert_eq!(refusal.code, "invalid_params");
    assert_eq!(refusal.sentence, "This provider has no browser sign-in.");
    assert_eq!(
        daemon.requests()[0]["params"],
        json!({"provider": "anthropic"})
    );
}

#[test]
fn a_refusal_without_a_specific_sentence_falls_back_to_the_message() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "error": {
        "code": "busy", "message": "Another sign-in is already running.", "details": {}}}),
    )]);
    let CallError::Refused(refusal) = daemon
        .client()
        .call("auth.start", json!({"provider": "xai"}))
        .unwrap_err()
    else {
        panic!("expected a refusal")
    };
    assert_eq!(refusal.sentence, "Another sign-in is already running.");
}

#[test]
fn a_missing_socket_means_the_daemon_is_down() {
    let dir = tempfile::tempdir().unwrap();
    let client = Management::new(dir.path().join("daemon.sock"), TIMEOUT);
    assert!(matches!(
        client.call("hello", json!({})),
        Err(CallError::DaemonDown(ErrorKind::NotFound))
    ));
}

#[test]
fn a_stale_socket_file_means_the_daemon_is_down() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("daemon.sock");
    drop(UnixListener::bind(&socket).unwrap());
    let client = Management::new(socket, TIMEOUT);
    assert!(matches!(
        client.call("hello", json!({})),
        Err(CallError::DaemonDown(ErrorKind::ConnectionRefused))
    ));
}

#[test]
fn a_daemon_that_never_answers_times_out_within_the_deadline() {
    let daemon = FakeDaemon::start(vec![Reply::Stall(Duration::from_secs(2))]);
    let started = Instant::now();
    let err = daemon.client().call("hello", json!({})).unwrap_err();
    assert!(matches!(err, CallError::Timeout), "got {err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the deadline was not honoured"
    );
}

#[test]
fn a_daemon_that_hangs_up_is_an_io_error() {
    let daemon = FakeDaemon::start(vec![Reply::Hangup]);
    assert!(matches!(
        daemon.client().call("hello", json!({})),
        Err(CallError::Io(_))
    ));
}

#[test]
fn an_answer_for_another_request_is_discarded() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "someone-else", "result": {}}),
    )]);
    assert!(matches!(
        daemon.client().call("hello", json!({})),
        Err(CallError::Protocol(_))
    ));
}

#[test]
fn an_answer_with_both_result_and_error_is_a_protocol_error() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "result": {},
        "error": {"code": "x", "message": "y", "details": {}}}),
    )]);
    assert!(matches!(
        daemon.client().call("hello", json!({})),
        Err(CallError::Protocol(_))
    ));
}

#[test]
fn a_refused_secret_never_appears_in_the_error() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "error": {
        "code": "secret_store_failed", "message": "The key could not be stored.", "details": {}}}),
    )]);
    let err = daemon
        .client()
        .secret_set("openai_api_key", "sk-very-secret-value")
        .unwrap_err();
    assert!(!format!("{err:?}").contains("sk-very-secret-value"));
    assert_eq!(
        daemon.requests()[0]["params"]["value"],
        "sk-very-secret-value"
    );
}

#[test]
fn a_result_that_does_not_fit_the_model_is_a_protocol_error() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "result": {"providers": "nope"}}),
    )]);
    assert!(matches!(
        daemon.client().setup_state(),
        Err(CallError::Protocol(_))
    ));
}

#[test]
fn a_failed_job_without_its_reason_is_a_protocol_error() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "result": {
        "job_id": "job:1", "kind": "auth", "status": "failed", "phase": "verifying", "progress": null,
        "budget_ms": 300000, "started_at": "2026-09-24T00:00:00Z", "finished_at": null,
        "result": null, "failure": null}}),
    )]);
    assert!(matches!(
        daemon.client().job_get("job:1"),
        Err(CallError::Protocol(_))
    ));
}

#[test]
fn hello_reports_the_engine_version() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "result": {
        "protocol": {"current_version": 2, "minimum_version": 1, "maximum_version": 2},
        "capabilities": {"methods": ["hello"], "minimum_versions": {"hello": 1}},
        "engine": {"product_version": "0.11.0", "pid": "2312517", "engine_id": "fermix-core"},
        "setup": {}}}),
    )]);
    let hello = daemon.client().hello().unwrap();
    assert_eq!(hello.engine.product_version, "0.11.0");
    assert_eq!(hello.engine.pid.as_deref(), Some("2312517"));
}

#[test]
fn a_restart_takes_the_drain_lease_and_hands_it_back_when_systemd_refuses() {
    let daemon = FakeDaemon::start(vec![
        Reply::Envelope(
            json!({"request_id": "$id", "result": {"lease_id": "lease:7", "ttl_ms": 30000}}),
        ),
        Reply::Envelope(
            json!({"request_id": "$id", "result": {"lease_id": "lease:7", "status": "cancelled"}}),
        ),
    ]);
    let client = daemon.client();
    let lease = client.prepare_restart().unwrap();
    client.cancel_restart(&lease.lease_id).unwrap();
    let requests = daemon.requests();
    assert_eq!(requests[0]["method"], "lifecycle.prepare");
    assert_eq!(requests[1]["method"], "lifecycle.cancel");
    assert_eq!(requests[1]["params"], json!({"lease_id": "lease:7"}));
}

#[test]
fn a_busy_restart_is_refused_before_anything_commits() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "error": {
        "code": "busy", "message": "Another restart is already being prepared.", "details": {}}}),
    )]);
    assert!(matches!(
        daemon.client().prepare_restart(),
        Err(CallError::Refused(_))
    ));
    assert_eq!(daemon.requests().len(), 1);
}

#[test]
fn secret_clear_and_setup_session_send_their_params() {
    let daemon = FakeDaemon::start(vec![
        Reply::Envelope(
            json!({"request_id": "$id", "result": {"id": "openai_api_key", "present": false,
            "restart": {"required": false, "reasons": []}}}),
        ),
        Reply::Envelope(
            json!({"request_id": "$id", "result": {"url": "http://127.0.0.1:4000/setup?t=x", "expires_at_ms": 1}}),
        ),
    ]);
    let client = daemon.client();
    assert!(!client.secret_clear("openai_api_key").unwrap().present);
    assert!(client
        .setup_session()
        .unwrap()
        .url
        .starts_with("http://127.0.0.1"));
    let requests = daemon.requests();
    assert_eq!(requests[0]["params"], json!({"id": "openai_api_key"}));
    assert_eq!(requests[1]["method"], "setup.session.create");
}

#[test]
fn settings_calls_send_their_section_and_only_the_changed_values() {
    let answer = |result: Value| Reply::Envelope(json!({"request_id": "$id", "result": result}));
    let restart = json!({"required": false, "reasons": []});
    let daemon = FakeDaemon::start(vec![
        answer(json!({"sections": []})),
        answer(json!({"id": "memory", "title": "Memory", "rows": []})),
        answer(
            json!({"applied": ["review_interval_hours"], "restart": restart,
            "readiness": {"status": "ready", "failure_count": 0}, "side_effects": []}),
        ),
        answer(
            json!({"reloaded": true, "restart": restart, "config_state": "clear",
            "readiness": {"status": "ready", "failure_count": 0}}),
        ),
    ]);
    let client = daemon.client();
    client.settings_sections().unwrap();
    client.settings_get("memory").unwrap();
    let mut values = serde_json::Map::new();
    values.insert("review_interval_hours".into(), json!(12));
    client.settings_apply("memory", values).unwrap();
    client.settings_reload().unwrap();
    let requests = daemon.requests();
    assert_eq!(
        requests[0]["params"],
        json!({}),
        "input-free methods refuse any key"
    );
    assert_eq!(requests[1]["params"], json!({"section": "memory"}));
    assert_eq!(
        requests[2]["params"],
        json!({"section": "memory", "values": {"review_interval_hours": 12}})
    );
    assert_eq!(requests[3]["method"], "settings.reload");
}

#[test]
fn a_refusal_names_the_row_it_is_about() {
    let daemon = FakeDaemon::start(vec![Reply::Envelope(
        json!({"request_id": "$id", "error": {
        "code": "invalid_params", "message": "Request parameters are invalid.",
        "details": {"field": "review_interval_hours", "sentence": "This setting takes a number."}}}),
    )]);
    let mut values = serde_json::Map::new();
    values.insert("review_interval_hours".into(), json!("x"));
    let CallError::Refused(refusal) = daemon
        .client()
        .settings_apply("memory", values)
        .unwrap_err()
    else {
        panic!("expected a refusal")
    };
    assert_eq!(refusal.field.as_deref(), Some("review_interval_hours"));
    assert_eq!(refusal.sentence, "This setting takes a number.");
}
