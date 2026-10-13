//! The voice socket client against a scripted fake daemon on a real Unix socket.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use fermix_client::realtime::client::{
    connect, CloseReason, ConnectError, Inbox, Incoming, Outbox, HANDSHAKE_TIMEOUT,
};
use fermix_client::realtime::protocol::{
    ClientEvent, DecodeError, Direction, ServerEvent, TurnState, MAX_CHUNK_BYTES,
};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const HELLO: &str = r#"{"type":"server_hello","min_version":1,"max_version":2}"#;

/// The daemon's side of one connection.
struct Peer {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Peer {
    fn read(&mut self) -> Value {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).expect("the client's line");
        assert!(n > 0, "the client closed before sending a line");
        serde_json::from_str(&line).expect("the client writes JSON lines")
    }

    fn send(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).unwrap();
        self.writer.write_all(b"\n").unwrap();
    }

    /// Reads the hello and answers it in the daemon's words.
    fn greet(&mut self) {
        assert_eq!(
            self.read(),
            json!({"type": "client_hello", "protocol_version": 2})
        );
        self.send(HELLO);
    }

    /// Whether the client has closed its end: the next read sees EOF.
    fn sees_eof(&mut self) -> bool {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap() == 0
    }
}

struct FakeDaemon {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    handle: JoinHandle<()>,
}

impl FakeDaemon {
    /// Serves one connection with `script`.
    fn start(script: impl FnOnce(Peer) + Send + 'static) -> FakeDaemon {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("realtime.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let writer = stream.try_clone().unwrap();
            script(Peer {
                reader: BufReader::new(stream),
                writer,
            });
        });
        FakeDaemon {
            _dir: dir,
            socket,
            handle,
        }
    }

    fn connect(&self) -> Result<(Outbox, Inbox), ConnectError> {
        connect(&self.socket, Box::new(|_: &[u8]| {}))
    }

    fn finish(self) {
        self.handle.join().expect("the fake daemon's script passed");
    }
}

#[test]
fn the_handshake_sends_one_hello_and_the_connection_then_carries_control_frames() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        assert_eq!(peer.read(), json!({"type": "call_start"}));
        assert_eq!(peer.read(), json!({"type": "mute", "enabled": true}));
    });
    let (outbox, _inbox) = daemon.connect().expect("the handshake completes");
    outbox.control(ClientEvent::CallStart).unwrap();
    outbox.control(ClientEvent::Mute { enabled: true }).unwrap();
    daemon.finish();
}

#[test]
fn a_daemon_that_finds_the_app_too_old_rejects_it_with_its_direction() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.read();
        peer.send(r#"{"type":"error","reason":"unsupported_protocol_version","direction":"client_too_old","client_version":2,"min_version":3,"max_version":3}"#);
    });
    let Err(ConnectError::Rejected(error)) = daemon.connect() else {
        panic!("the refusal must reach the caller")
    };
    assert_eq!(error.reason, "unsupported_protocol_version");
    assert_eq!(error.direction, Some(Direction::ClientTooOld));
    daemon.finish();
}

#[test]
fn a_daemon_that_finds_the_app_too_new_rejects_it_with_its_direction() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.read();
        peer.send(r#"{"type":"error","reason":"unsupported_protocol_version","direction":"client_too_new","client_version":2,"min_version":1,"max_version":1}"#);
    });
    let Err(ConnectError::Rejected(error)) = daemon.connect() else {
        panic!("the refusal must reach the caller")
    };
    assert_eq!(error.direction, Some(Direction::ClientTooNew));
    daemon.finish();
}

#[test]
fn a_server_hello_whose_window_excludes_2_is_refused_by_the_app_itself() {
    for (window, direction) in [
        ((3, 4), Direction::ClientTooOld),
        ((1, 1), Direction::ClientTooNew),
    ] {
        let daemon = FakeDaemon::start(move |mut peer| {
            peer.read();
            peer.send(&format!(
                r#"{{"type":"server_hello","min_version":{},"max_version":{}}}"#,
                window.0, window.1
            ));
        });
        let Err(ConnectError::Rejected(error)) = daemon.connect() else {
            panic!("a window without 2 is no connection")
        };
        assert_eq!(error.reason, "unsupported_protocol_version");
        assert_eq!(error.direction, Some(direction));
        assert_eq!(
            (error.min_version, error.max_version),
            (Some(window.0), Some(window.1))
        );
        daemon.finish();
    }
}

#[test]
fn a_daemon_that_never_says_hello_times_out_after_three_seconds() {
    let (done, finished) = mpsc::channel::<()>();
    let daemon = FakeDaemon::start(move |mut peer| {
        peer.read();
        // Anything but server_hello or error is ignored while the handshake waits.
        peer.send(r#"{"type":"state","state":"idle"}"#);
        let _ = finished.recv_timeout(Duration::from_secs(10));
    });
    let started = Instant::now();
    let result = daemon.connect();
    let waited = started.elapsed();
    assert!(matches!(result, Err(ConnectError::Timeout)), "{result:?}");
    assert!(waited >= HANDSHAKE_TIMEOUT, "{waited:?}");
    assert!(
        waited < HANDSHAKE_TIMEOUT + Duration::from_secs(1),
        "{waited:?}"
    );
    done.send(()).unwrap();
    daemon.finish();
}

#[test]
fn the_handshake_timeout_is_three_seconds() {
    assert_eq!(HANDSHAKE_TIMEOUT, Duration::from_secs(3));
}

#[test]
fn a_missing_socket_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let result = connect(&dir.path().join("realtime.sock"), Box::new(|_: &[u8]| {}));
    assert!(matches!(result, Err(ConnectError::NotFound)), "{result:?}");
}

#[test]
fn a_socket_nobody_listens_on_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("realtime.sock");
    drop(UnixListener::bind(&socket).unwrap());
    let result = connect(&socket, Box::new(|_: &[u8]| {}));
    assert!(matches!(result, Err(ConnectError::Refused)), "{result:?}");
}

#[test]
fn a_daemon_that_hangs_up_during_the_handshake_is_an_io_error() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.read();
    });
    let result = daemon.connect();
    let Err(ConnectError::Io(error)) = result else {
        panic!("a hang-up is not a refusal: {result:?}")
    };
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    daemon.finish();
}

#[test]
fn audio_goes_to_on_audio_and_the_inbox_hears_only_its_size_and_level() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        let audio = STANDARD.encode([0x00, 0x40, 0x00, 0xc0]);
        peer.send(&format!(r#"{{"type":"audio_delta","audio":"{audio}"}}"#));
        peer.send(r#"{"type":"state","state":"speaking"}"#);
        peer.read();
    });
    let (heard, audio) = mpsc::channel();
    let on_audio = Box::new(move |bytes: &[u8]| heard.send(bytes.to_vec()).unwrap());
    let (outbox, inbox) = connect(&daemon.socket, on_audio).unwrap();

    let Incoming::Audio { bytes, rms } = inbox.recv() else {
        panic!("audio first")
    };
    assert_eq!(bytes, 4);
    assert!((rms - 0.5).abs() < 1e-3, "{rms}");
    assert_eq!(audio.try_recv().unwrap(), vec![0x00, 0x40, 0x00, 0xc0]);
    assert_eq!(
        inbox.recv(),
        Incoming::Event(ServerEvent::State {
            state: TurnState::Speaking
        })
    );
    outbox.control(ClientEvent::CallStop).unwrap();
    daemon.finish();
}

#[test]
fn events_that_arrive_with_the_hello_are_not_lost() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.read();
        let burst = format!("{HELLO}\n{{\"type\":\"state\",\"state\":\"idle\"}}\n");
        peer.writer.write_all(burst.as_bytes()).unwrap();
        peer.read();
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    assert_eq!(
        inbox.recv(),
        Incoming::Event(ServerEvent::State {
            state: TurnState::Idle
        })
    );
    outbox.control(ClientEvent::CallStop).unwrap();
    daemon.finish();
}

#[test]
fn an_unknown_event_is_delivered_by_name_and_the_connection_stays_up() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        peer.send(r#"{"type":"weather","sky":"clear"}"#);
        peer.send(r#"{"type":"playback_stop"}"#);
        peer.read();
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    assert_eq!(
        inbox.recv(),
        Incoming::Event(ServerEvent::Unknown("weather".into()))
    );
    assert_eq!(inbox.recv(), Incoming::Event(ServerEvent::PlaybackStop));
    outbox.control(ClientEvent::CallStop).unwrap();
    daemon.finish();
}

#[test]
fn eof_from_the_daemon_closes_the_connection_once() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    daemon.finish();
    assert_eq!(inbox.recv(), Incoming::Closed(CloseReason::PeerClosed));
    assert_eq!(
        inbox.recv(),
        Incoming::Closed(CloseReason::PeerClosed),
        "closed stays closed"
    );
    assert!(outbox.control(ClientEvent::CallStop).is_err());
}

#[test]
fn a_line_that_is_not_json_is_a_framing_violation_that_closes() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        peer.send("{not json");
        assert!(peer.sees_eof(), "the client hangs up on a broken peer");
    });
    let (_outbox, inbox) = daemon.connect().unwrap();
    let Incoming::Closed(CloseReason::Framing(DecodeError::NotJson(_))) = inbox.recv() else {
        panic!("a broken line ends the connection")
    };
    daemon.finish();
}

#[test]
fn closing_hangs_up_reports_closed_once_and_refuses_further_frames() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        assert!(peer.sees_eof());
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    let copy = outbox.clone();
    outbox.close();
    daemon.finish();
    assert_eq!(inbox.recv(), Incoming::Closed(CloseReason::Local));
    assert!(copy.control(ClientEvent::CallStop).is_err());
    copy.audio(vec![0; 4]);
}

#[test]
fn dropping_every_outbox_hangs_up() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        assert!(peer.sees_eof(), "no Outbox left, so the socket closes");
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    let copy = outbox.clone();
    drop(outbox);
    drop(copy);
    daemon.finish();
    assert_eq!(inbox.recv(), Incoming::Closed(CloseReason::Local));
}

#[test]
fn audio_blocks_reach_the_daemon_as_base64_chunks() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
        let chunk = peer.read();
        assert_eq!(chunk["type"], "audio_chunk");
        let audio = STANDARD.decode(chunk["audio"].as_str().unwrap()).unwrap();
        assert_eq!(audio, vec![1, 2, 3, 4]);
    });
    let (outbox, _inbox) = daemon.connect().unwrap();
    outbox.audio(vec![1, 2, 3, 4]);
    daemon.finish();
}

/// The daemon stops reading; the app keeps producing audio. What finally arrives is the newest
/// audio in order, with the oldest of the backlog gone.
#[test]
fn a_backed_up_socket_drops_the_oldest_audio_and_keeps_the_newest() {
    let (resume, resumed) = mpsc::channel::<()>();
    let daemon = FakeDaemon::start(move |mut peer| {
        peer.greet();
        resumed.recv().unwrap();
        peer.reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let mut seen = Vec::new();
        let mut line = String::new();
        while peer
            .reader
            .read_line(&mut line)
            .map(|n| n > 0)
            .unwrap_or(false)
        {
            let chunk: Value = serde_json::from_str(&line).unwrap();
            let audio = STANDARD.decode(chunk["audio"].as_str().unwrap()).unwrap();
            seen.push(audio[0]);
            line.clear();
        }
        assert!(seen.len() < 60, "some audio was dropped: {}", seen.len());
        assert_eq!(seen.last(), Some(&59), "the newest block always goes out");
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "in order: {seen:?}");
    });
    let (outbox, _inbox) = daemon.connect().unwrap();
    for n in 0..60u8 {
        outbox.audio(vec![n; MAX_CHUNK_BYTES]);
    }
    thread::sleep(Duration::from_millis(300));
    resume.send(()).unwrap();
    daemon.finish();
}

/// Audio after `call_stop` is `not_connected` to the daemon, which then hangs up. Control frames
/// go ahead of queued audio, so `call_stop` must take the call's queued audio with it.
#[test]
fn call_stop_discards_the_audio_still_queued_for_the_call() {
    let (resume, resumed) = mpsc::channel::<()>();
    let daemon = FakeDaemon::start(move |mut peer| {
        peer.greet();
        resumed.recv().unwrap();
        let mut after_stop = Vec::new();
        let mut stopped = false;
        let mut line = String::new();
        peer.reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        while peer
            .reader
            .read_line(&mut line)
            .map(|n| n > 0)
            .unwrap_or(false)
        {
            let frame: Value = serde_json::from_str(&line).unwrap();
            if stopped {
                after_stop.push(frame["type"].as_str().unwrap().to_owned());
            }
            stopped |= frame["type"] == "call_stop";
            line.clear();
        }
        assert!(stopped, "call_stop arrived");
        assert!(
            after_stop.is_empty(),
            "nothing after call_stop: {after_stop:?}"
        );
    });
    let (outbox, _inbox) = daemon.connect().unwrap();
    for n in 0..40u8 {
        outbox.audio(vec![n; MAX_CHUNK_BYTES]);
    }
    thread::sleep(Duration::from_millis(200));
    outbox.control(ClientEvent::CallStop).unwrap();
    resume.send(()).unwrap();
    daemon.finish();
}

/// Control frames are never dropped, but one that cannot be written within 5 s means the
/// daemon has stopped reading, and the connection is dead.
#[test]
fn a_control_frame_stuck_for_five_seconds_ends_the_connection() {
    let (done, finished) = mpsc::channel::<()>();
    let daemon = FakeDaemon::start(move |mut peer| {
        peer.greet();
        let _ = finished.recv_timeout(Duration::from_secs(15));
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    for n in 0..40u8 {
        outbox.audio(vec![n; MAX_CHUNK_BYTES]);
    }
    thread::sleep(Duration::from_millis(200));
    let asked = Instant::now();
    outbox.control(ClientEvent::CallStop).unwrap();
    assert_eq!(inbox.recv(), Incoming::Closed(CloseReason::ControlTimeout));
    let waited = asked.elapsed();
    assert!(waited >= Duration::from_secs(5), "{waited:?}");
    assert!(waited < Duration::from_secs(7), "{waited:?}");
    done.send(()).unwrap();
    daemon.finish();
}

/// A steady call sends no control frames, so a daemon that stops reading mid-call is caught by
/// the audio backing up with no write progress for 8 s.
#[test]
fn audio_stalled_for_eight_seconds_ends_the_connection() {
    let (done, finished) = mpsc::channel::<()>();
    let daemon = FakeDaemon::start(move |mut peer| {
        peer.greet();
        let _ = finished.recv_timeout(Duration::from_secs(20));
    });
    let (outbox, inbox) = daemon.connect().unwrap();
    let started = Instant::now();
    for n in 0..40u8 {
        outbox.audio(vec![n; MAX_CHUNK_BYTES]);
    }
    assert_eq!(inbox.recv(), Incoming::Closed(CloseReason::Stalled));
    let waited = started.elapsed();
    assert!(waited >= Duration::from_secs(8), "{waited:?}");
    assert!(waited < Duration::from_secs(10), "{waited:?}");
    done.send(()).unwrap();
    daemon.finish();
}

#[test]
#[should_panic(expected = "PCM16")]
fn an_odd_audio_block_is_a_caller_bug() {
    let daemon = FakeDaemon::start(|mut peer| {
        peer.greet();
    });
    let (outbox, _inbox) = daemon.connect().unwrap();
    outbox.audio(vec![1, 2, 3]);
}
