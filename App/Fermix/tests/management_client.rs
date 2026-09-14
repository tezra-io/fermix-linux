//! The client, against a fake peer on a real Unix socket.
//!
//! Nothing here touches a real `daemon.sock`. The peer is a thread holding a
//! socket in a temporary directory, scripted to do the five things a daemon can
//! do to a client: answer, answer somebody else, answer too much, answer too
//! little, and never answer at all.

use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::Duration;

use fermix_desktop::management::contract;
use fermix_desktop::management::errors::{ManagementError, TransportError};
use fermix_desktop::management::types::HelloResult;
use fermix_desktop::management::ManagementClient;
use fermix_desktop::testing::TempDirectory;
use gtk4::glib::MainContext;

/// What the peer does with the one connection it accepts.
enum Behaviour {
    /// Answer with these bytes, framed.
    Answer(Vec<u8>),
    /// Accept, hold the connection, and never say anything.
    Silent,
    /// Declare a frame above the contract's ceiling.
    Oversized,
    /// Declare a frame and send less than it declared.
    Short,
}

struct FakePeer {
    path: PathBuf,
    handle: Option<JoinHandle<()>>,
    _directory: TempDirectory,
}

impl FakePeer {
    fn spawn(label: &str, behaviour: Behaviour) -> Self {
        let directory = TempDirectory::new(label);
        let path = directory.join("daemon.sock");
        let listener = UnixListener::bind(&path).expect("the fake peer binds");
        listener.set_nonblocking(true).expect("the fake peer polls");

        let handle = std::thread::spawn(move || {
            let Some(mut connection) = accept_within(&listener, ACCEPT_BUDGET) else {
                // Nobody connected. A refusal the client makes before sending
                // is exactly that, and this peer ends rather than waiting for a
                // connection that is never coming.
                return;
            };

            let mut header = [0u8; 4];
            if connection.read_exact(&mut header).is_err() {
                return;
            }
            let declared = u32::from_be_bytes(header) as usize;
            let mut request = vec![0u8; declared];
            let _ = connection.read_exact(&mut request);

            match behaviour {
                Behaviour::Answer(body) => {
                    let _ = connection.write_all(&(body.len() as u32).to_be_bytes());
                    let _ = connection.write_all(&body);
                    let _ = connection.flush();
                }
                Behaviour::Silent => {
                    // Accepted, and nothing else. The client's deadline is what
                    // ends this exchange.
                    std::thread::sleep(Duration::from_secs(2));
                }
                Behaviour::Oversized => {
                    let ceiling = contract::limits().max_frame_bytes as u32;
                    let _ = connection.write_all(&(ceiling + 1).to_be_bytes());
                    let _ = connection.flush();
                    std::thread::sleep(Duration::from_millis(200));
                }
                Behaviour::Short => {
                    let _ = connection.write_all(&64u32.to_be_bytes());
                    let _ = connection.write_all(b"eight..");
                    let _ = connection.flush();
                }
            }
        });

        Self {
            path,
            handle: Some(handle),
            _directory: directory,
        }
    }

    fn client(&self) -> ManagementClient {
        ManagementClient::new(&self.path)
    }
}

impl Drop for FakePeer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn hello_envelope(request_id: &str) -> Vec<u8> {
    serde_json::json!({
        "request_id": request_id,
        "result": {
            "protocol": {"current_version": 2, "minimum_version": 1, "maximum_version": 2},
            "capabilities": {"methods": ["hello"], "minimum_versions": {"hello": 1}},
            "engine": {
                "engine_id": "fermix",
                "product_version": "0.11.0",
                "build_id": "b7c1f0a4",
                "source_commit": null,
                "distribution_identity": "linux_package",
                "artifact_target": "linux_aarch64",
                "architecture": "aarch64",
                "pid": "47119"
            },
            "setup": {"origin": "http://127.0.0.1:4030", "path": "/setup"}
        }
    })
    .to_string()
    .into_bytes()
}

/// How long the fake peer waits for a connection before giving up. A client
/// that refuses a call before sending it never connects, and a peer that waits
/// forever for that connection hangs the suite rather than failing it.
const ACCEPT_BUDGET: Duration = Duration::from_secs(3);

const DEADLINE: Duration = Duration::from_millis(500);

/// Accept one connection inside a budget. Bounded twice: by the deadline and by
/// the number of polls it takes to reach it.
fn accept_within(
    listener: &UnixListener,
    budget: Duration,
) -> Option<std::os::unix::net::UnixStream> {
    let deadline = std::time::Instant::now() + budget;

    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((connection, _)) => {
                connection
                    .set_nonblocking(false)
                    .expect("the accepted connection blocks");
                return Some(connection);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }

    None
}

#[test]
fn a_valid_exchange_negotiates_and_decodes() {
    let peer = FakePeer::spawn("client-valid", Behaviour::Answer(hello_envelope("fx-1")));
    let client = peer.client();

    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });
    let hello: HelloResult = answer
        .accept(&client)
        .expect("the answer is from the current epoch")
        .expect("the exchange succeeds");

    assert_eq!(hello.engine.product_version, "0.11.0");
    assert_eq!(
        client.negotiated_version(),
        Some(contract::supported_range().max)
    );
}

#[test]
fn a_peer_that_accepts_and_never_answers_ends_at_the_deadline() {
    let peer = FakePeer::spawn("client-silent", Behaviour::Silent);
    let client = peer.client();

    let answer =
        MainContext::new().block_on(async { client.hello(Duration::from_millis(150)).await });

    match answer.value {
        Err(ManagementError::Transport(TransportError::Timeout)) => {}
        other => panic!("expected the deadline to fire, got {other:?}"),
    }
}

#[test]
fn the_client_still_works_after_a_deadline_fired() {
    // The socket of a timed-out exchange is closed on that path, so the next
    // exchange is an ordinary one rather than a second failure.
    let silent = FakePeer::spawn("client-recover-silent", Behaviour::Silent);
    let silent_client = silent.client();
    let _ = MainContext::new()
        .block_on(async { silent_client.hello(Duration::from_millis(100)).await });

    let peer = FakePeer::spawn("client-recover", Behaviour::Answer(hello_envelope("fx-1")));
    let client = peer.client();
    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });

    assert!(
        answer.value.is_ok(),
        "the client recovered: {:?}",
        answer.value.err()
    );
}

#[test]
fn a_frame_above_the_ceiling_is_refused_before_it_is_allocated() {
    let peer = FakePeer::spawn("client-oversized", Behaviour::Oversized);
    let client = peer.client();

    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });

    match answer.value {
        Err(ManagementError::Transport(TransportError::Frame(error))) => {
            assert!(
                error.to_string().contains("ceiling"),
                "the refusal names the ceiling: {error}"
            );
        }
        other => panic!("expected the frame to be refused, got {other:?}"),
    }
}

#[test]
fn a_short_frame_is_a_truncation_rather_than_a_short_answer() {
    let peer = FakePeer::spawn("client-short", Behaviour::Short);
    let client = peer.client();

    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });

    match answer.value {
        Err(ManagementError::Transport(TransportError::Frame(error))) => {
            assert!(error.to_string().contains("truncated"), "{error}");
        }
        other => panic!("expected a truncation, got {other:?}"),
    }
}

#[test]
fn a_frame_answering_a_different_request_is_discarded() {
    let peer = FakePeer::spawn(
        "client-mismatch",
        Behaviour::Answer(hello_envelope("fx-999")),
    );
    let client = peer.client();

    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });

    match answer.value {
        Err(ManagementError::NoAnswer { method }) => assert_eq!(method, "hello"),
        other => panic!("expected the frame to be discarded, got {other:?}"),
    }
}

#[test]
fn a_result_from_an_older_epoch_never_reaches_the_model() {
    let peer = FakePeer::spawn("client-epoch", Behaviour::Answer(hello_envelope("fx-1")));
    let client = peer.client();

    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });
    assert!(answer.value.is_ok(), "the exchange itself succeeded");

    // The connection is dropped and taken again while the answer is in hand.
    client.reset();

    assert!(
        answer.accept(&client).is_none(),
        "an answer issued under the previous connection is not acceptable"
    );
}

#[test]
fn a_method_above_the_negotiated_version_is_refused_before_it_is_sent() {
    // The peer answers `hello` with a window that tops out at 1, so every
    // v2 method is refused here rather than sent and refused there.
    let narrow = serde_json::json!({
        "request_id": "fx-1",
        "result": {
            "protocol": {"current_version": 1, "minimum_version": 1, "maximum_version": 1},
            "capabilities": {"methods": ["hello"], "minimum_versions": {"hello": 1}},
            "engine": {
                "engine_id": "fermix", "product_version": "0.10.0", "build_id": null,
                "source_commit": null, "distribution_identity": "linux_package",
                "artifact_target": null, "architecture": "aarch64", "pid": "1"
            },
            "setup": {"origin": "http://127.0.0.1:4030", "path": "/setup"}
        }
    })
    .to_string()
    .into_bytes();

    let peer = FakePeer::spawn("client-narrow", Behaviour::Answer(narrow));
    let client = peer.client();

    let context = MainContext::new();
    let hello = context.block_on(async { client.hello(DEADLINE).await });
    assert!(hello.value.is_ok());
    assert_eq!(client.negotiated_version(), Some(1));

    let refused = context.block_on(async {
        client
            .call::<serde_json::Value>("settings.sections", DEADLINE)
            .await
    });

    match refused.value {
        Err(ManagementError::MethodNeedsNewerDaemon {
            method,
            requires,
            negotiated,
        }) => {
            assert_eq!(method, "settings.sections");
            assert_eq!(requires, 2);
            assert_eq!(negotiated, 1);
        }
        other => panic!("expected a refusal before sending, got {other:?}"),
    }
}

#[test]
fn a_method_the_contract_does_not_publish_is_refused_before_it_is_sent() {
    let peer = FakePeer::spawn("client-unknown", Behaviour::Answer(hello_envelope("fx-1")));
    let client = peer.client();

    let refused = MainContext::new().block_on(async {
        client
            .call::<serde_json::Value>("lifecycle.reboot", DEADLINE)
            .await
    });

    match refused.value {
        Err(ManagementError::MethodNotInContract { method }) => {
            assert_eq!(method, "lifecycle.reboot");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn nothing_is_listening_reads_as_not_running() {
    let directory = TempDirectory::new("client-absent");
    let client = ManagementClient::new(directory.join("daemon.sock"));

    let answer = MainContext::new().block_on(async { client.hello(DEADLINE).await });

    match answer.value {
        Err(ManagementError::Transport(TransportError::NotRunning)) => {}
        other => panic!("expected NotRunning, got {other:?}"),
    }
}
