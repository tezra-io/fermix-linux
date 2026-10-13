//! The voice socket: `realtime.sock`, a blocking std `UnixStream` with one reader thread and
//! one writer thread, so audio never waits on the GTK main loop (spec §1.1, §1.2, §1.8).
//!
//! `connect` completes the handshake on the caller's thread. After it, the `Outbox` queues what
//! the app sends: control frames in order and never dropped, audio in a 20-block ring that drops
//! the oldest block when the socket backs up. The `Inbox` hands out what the daemon sent, and
//! always ends with exactly one `Closed`. The connection ends on EOF, a read or write error, a
//! frame this client refuses, a control frame not written within 5 s, 8 s without write
//! progress, `Outbox::close`, or the last `Outbox` being dropped.

use crate::realtime::playback::rms;
use crate::realtime::protocol::{
    decode_line, ClientEvent, DecodeError, Direction, LineBuffer, ServerError, ServerEvent,
    MAX_CHUNK_BYTES, PROTOCOL_VERSION,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// How long the daemon has to answer `client_hello` (macOS `RealtimeProtocol.swift`).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
/// Outbound audio held while the socket is backed up: about 2 s of 100 ms blocks.
pub const AUDIO_QUEUE_BLOCKS: usize = 20;
/// A control frame not written within this long means the daemon stopped reading.
pub const CONTROL_FLUSH_DEADLINE: Duration = Duration::from_secs(5);
/// A write that makes no progress for this long means the daemon stopped reading. During a
/// call the microphone keeps the audio ring full, so this is the audio stall of spec §1.8.
pub const STALL_DEADLINE: Duration = Duration::from_secs(8);
/// How often a blocked write wakes to check the two deadlines.
const WRITE_TICK: Duration = Duration::from_millis(100);
const READ_CHUNK_BYTES: usize = 16 * 1024;

/// Why `connect` produced no connection.
#[derive(Debug)]
pub enum ConnectError {
    /// No `realtime.sock` (ENOENT): voice is off, or the daemon did not open it.
    NotFound,
    /// The socket exists but nothing accepts on it (ECONNREFUSED).
    Refused,
    /// The daemon said no, or its version window excludes this app's. `direction` says which
    /// side must update, when the refusal was about versions.
    Rejected(Box<ServerError>),
    /// No `server_hello` or `error` within `HANDSHAKE_TIMEOUT`.
    Timeout,
    /// Anything else, including a hang-up (`UnexpectedEof`) or a broken frame (`InvalidData`)
    /// during the handshake.
    Io(io::Error),
}

/// Why a connection ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloseReason {
    /// This app closed it: `Outbox::close`, the last `Outbox` dropped, or the `Inbox` dropped.
    Local,
    /// The daemon closed its end (EOF).
    PeerClosed,
    ReadFailed(String),
    WriteFailed(String),
    /// A control frame waited `CONTROL_FLUSH_DEADLINE` without being written.
    ControlTimeout,
    /// A write made no progress for `STALL_DEADLINE`.
    Stalled,
    /// The daemon sent bytes that are not the wire's.
    Framing(DecodeError),
    /// A control event too long for the daemon's line cap; nothing of it was sent.
    LineTooLong(usize),
}

/// The connection is over; the `Inbox` says why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Event(ServerEvent),
    /// An `audio_delta` of `bytes` bytes went to `on_audio`; `rms` is its level, unsmoothed.
    Audio {
        bytes: usize,
        rms: f32,
    },
    /// Always the last message.
    Closed(CloseReason),
}

/// Runs on the reader thread with each decoded `audio_delta`.
pub type OnAudio = Box<dyn FnMut(&[u8]) + Send>;

/// The sending half. Clones share one connection, which closes when the last one is dropped.
#[derive(Debug, Clone)]
pub struct Outbox {
    owner: Arc<Owner>,
}

/// What the app receives, in the daemon's order.
#[derive(Debug)]
pub struct Inbox {
    events: Receiver<Incoming>,
    ended: RefCell<Option<CloseReason>>,
}

/// Held by every `Outbox` clone: dropping the last one ends the connection.
#[derive(Debug)]
struct Owner {
    shared: Arc<Shared>,
}

/// Held by the `Owner` and both threads.
#[derive(Debug)]
struct Shared {
    queues: Mutex<Queues>,
    /// Wakes the writer: a frame was queued, or the connection ended.
    wake: Condvar,
    /// A handle to the socket, only to shut it down, which unblocks both threads.
    stream: UnixStream,
    events: Sender<Incoming>,
}

#[derive(Debug, Default)]
struct Queues {
    control: VecDeque<(String, Instant)>,
    audio: VecDeque<Vec<u8>>,
    ended: bool,
}

enum Job {
    Control(String, Instant),
    Audio(Vec<u8>),
}

impl Outbox {
    /// Queues a control frame behind earlier ones and ahead of all audio. It is never dropped.
    /// `call_stop` also drops the audio still queued: the daemon answers audio after it with
    /// `not_connected` and hangs up. Close the microphone gate before sending it.
    pub fn control(&self, event: ClientEvent) -> Result<(), Closed> {
        assert!(
            !matches!(event, ClientEvent::AudioChunk { .. }),
            "audio goes through Outbox::audio"
        );
        let shared = &self.owner.shared;
        let line = match event.line() {
            Ok(line) => line,
            Err(too_long) => {
                end(shared, CloseReason::LineTooLong(too_long.0));
                return Err(Closed);
            }
        };
        let mut queues = lock(shared);
        if queues.ended {
            return Err(Closed);
        }
        if event == ClientEvent::CallStop {
            queues.audio.clear();
        }
        queues.control.push_back((line, Instant::now()));
        drop(queues);
        shared.wake.notify_one();
        Ok(())
    }

    /// Queues one block of microphone audio: whole PCM16 samples, at most `MAX_CHUNK_BYTES`.
    /// When `AUDIO_QUEUE_BLOCKS` are already waiting, the oldest is dropped. After the
    /// connection ends this does nothing; the `Inbox` has already said why.
    pub fn audio(&self, pcm16: Vec<u8>) {
        assert!(
            !pcm16.is_empty() && pcm16.len().is_multiple_of(2) && pcm16.len() <= MAX_CHUNK_BYTES,
            "an audio block is whole PCM16 samples within MAX_CHUNK_BYTES, not {} bytes",
            pcm16.len()
        );
        let shared = &self.owner.shared;
        let mut queues = lock(shared);
        if queues.ended {
            return;
        }
        if queues.audio.len() == AUDIO_QUEUE_BLOCKS {
            queues.audio.pop_front();
        }
        queues.audio.push_back(pcm16);
        drop(queues);
        shared.wake.notify_one();
    }

    /// Hangs up now. Queued frames are discarded: the daemon treats EOF as the end of the call.
    pub fn close(&self) {
        end(&self.owner.shared, CloseReason::Local);
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        end(&self.shared, CloseReason::Local);
    }
}

impl Inbox {
    /// Blocks for the next message. After `Closed`, every call returns the same `Closed`.
    pub fn recv(&self) -> Incoming {
        if let Some(reason) = self.ended.borrow().clone() {
            return Incoming::Closed(reason);
        }
        let incoming = self
            .events
            .recv()
            .expect("the voice socket sends Closed before it lets go of the Inbox");
        if let Incoming::Closed(reason) = &incoming {
            *self.ended.borrow_mut() = Some(reason.clone());
        }
        incoming
    }
}

/// Connects to `path` and completes the handshake within `HANDSHAKE_TIMEOUT`. Blocking: call it
/// off the GTK main thread. `on_audio` gets each decoded `audio_delta` on the reader thread; the
/// `Inbox` gets `Audio { .. }` without the payload, so the main loop never carries audio.
pub fn connect(path: &Path, on_audio: OnAudio) -> Result<(Outbox, Inbox), ConnectError> {
    let mut stream = UnixStream::connect(path).map_err(connect_error)?;
    let mut lines = LineBuffer::new();
    let early = handshake(&mut stream, &mut lines)?;
    stream.set_read_timeout(None).map_err(ConnectError::Io)?;
    stream
        .set_write_timeout(Some(WRITE_TICK))
        .map_err(ConnectError::Io)?;
    let (sender, events) = mpsc::channel();
    let shared = Arc::new(Shared {
        queues: Mutex::default(),
        wake: Condvar::new(),
        stream: stream.try_clone().map_err(ConnectError::Io)?,
        events: sender,
    });
    let owner = Owner {
        shared: Arc::clone(&shared),
    };
    let reader = stream.try_clone().map_err(ConnectError::Io)?;
    let read_shared = Arc::clone(&shared);
    thread::Builder::new()
        .name("fermix-voice-read".into())
        .spawn(move || read_loop(reader, read_shared, lines, early, on_audio))
        .map_err(ConnectError::Io)?;
    thread::Builder::new()
        .name("fermix-voice-write".into())
        .spawn(move || write_loop(stream, shared))
        .map_err(ConnectError::Io)?;
    let inbox = Inbox {
        events,
        ended: RefCell::new(None),
    };
    Ok((
        Outbox {
            owner: Arc::new(owner),
        },
        inbox,
    ))
}

fn connect_error(error: io::Error) -> ConnectError {
    match error.kind() {
        io::ErrorKind::NotFound => ConnectError::NotFound,
        io::ErrorKind::ConnectionRefused => ConnectError::Refused,
        _ => ConnectError::Io(error),
    }
}

/// Sends `client_hello` and waits for the answer, ignoring every other frame (spec §1.2).
/// Returns the lines that arrived after `server_hello`: they belong to the session.
fn handshake(
    stream: &mut UnixStream,
    lines: &mut LineBuffer,
) -> Result<Vec<Vec<u8>>, ConnectError> {
    let hello = ClientEvent::ClientHello {
        protocol_version: PROTOCOL_VERSION,
    };
    let hello = hello.line().expect("the hello fits in a line");
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(ConnectError::Io)?;
    stream
        .write_all(hello.as_bytes())
        .map_err(ConnectError::Io)?;
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut buffer = [0u8; READ_CHUNK_BYTES];
    // Bounded by the deadline: every pass reads bytes or runs its clock down.
    loop {
        let bytes = read_before(stream, deadline, &mut buffer)?;
        let mut ready = lines.push(bytes).map_err(invalid_data)?;
        if let Some(index) = answer(&ready)? {
            return Ok(ready.split_off(index + 1));
        }
    }
}

fn read_before<'b>(
    stream: &mut UnixStream,
    deadline: Instant,
    buffer: &'b mut [u8],
) -> Result<&'b [u8], ConnectError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(ConnectError::Timeout);
    }
    stream
        .set_read_timeout(Some(left))
        .map_err(ConnectError::Io)?;
    match stream.read(buffer) {
        Ok(0) => Err(ConnectError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Fermix closed the voice connection during the handshake",
        ))),
        Ok(n) => Ok(&buffer[..n]),
        Err(e) if timed_out(&e) => Err(ConnectError::Timeout),
        Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(&buffer[..0]),
        Err(e) => Err(ConnectError::Io(e)),
    }
}

/// Where the daemon's answer is among these lines, if it came: a `server_hello` inside the
/// window, or a refusal.
fn answer(lines: &[Vec<u8>]) -> Result<Option<usize>, ConnectError> {
    for (index, line) in lines.iter().enumerate() {
        match decode_line(line).map_err(invalid_data)? {
            ServerEvent::ServerHello {
                min_version,
                max_version,
            } => return check_window(min_version, max_version).map(|()| Some(index)),
            ServerEvent::Error(error) => return Err(ConnectError::Rejected(Box::new(error))),
            _ => continue,
        }
    }
    Ok(None)
}

/// The app checks the window itself: a daemon's hello is not a promise that it speaks 2.
fn check_window(min_version: u32, max_version: u32) -> Result<(), ConnectError> {
    if (min_version..=max_version).contains(&PROTOCOL_VERSION) {
        return Ok(());
    }
    let direction = if PROTOCOL_VERSION < min_version {
        Direction::ClientTooOld
    } else {
        Direction::ClientTooNew
    };
    Err(ConnectError::Rejected(Box::new(ServerError {
        reason: "unsupported_protocol_version".into(),
        kind: None,
        detail: None,
        direction: Some(direction),
        client_version: Some(PROTOCOL_VERSION),
        min_version: Some(min_version),
        max_version: Some(max_version),
        required_for: None,
    })))
}

fn invalid_data(error: DecodeError) -> ConnectError {
    ConnectError::Io(io::Error::new(
        io::ErrorKind::InvalidData,
        format!("the daemon's handshake frame is not the wire's: {error:?}"),
    ))
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

fn lock(shared: &Shared) -> MutexGuard<'_, Queues> {
    shared
        .queues
        .lock()
        .expect("nothing panics while holding the voice queues")
}

/// Ends the connection once: marks it ended, drops what is queued, shuts the socket down so
/// both threads stop, and tells the `Inbox` why. Later calls do nothing.
fn end(shared: &Shared, reason: CloseReason) {
    let first = {
        let mut queues = lock(shared);
        let first = !queues.ended;
        queues.ended = true;
        queues.control.clear();
        queues.audio.clear();
        first
    };
    shared.wake.notify_all();
    match shared.stream.shutdown(Shutdown::Both) {
        Ok(()) => {}
        // Already down, which is what this asks for.
        Err(e) if e.kind() == io::ErrorKind::NotConnected => {}
        Err(e) => panic!("the voice socket could not be shut down: {e}"),
    }
    if first {
        // An Err means the Inbox is gone, and with it everyone who could be told.
        shared.events.send(Incoming::Closed(reason)).ok();
    }
}

/// Ends the connection if the reader thread unwinds, so a panic in `on_audio` still closes
/// the socket and still ends the `Inbox` with `Closed`.
struct EndOnPanic(Arc<Shared>);

impl Drop for EndOnPanic {
    fn drop(&mut self) {
        if thread::panicking() {
            end(
                &self.0,
                CloseReason::ReadFailed("the voice reader panicked".into()),
            );
        }
    }
}

fn read_loop(
    stream: UnixStream,
    shared: Arc<Shared>,
    lines: LineBuffer,
    early: Vec<Vec<u8>>,
    mut on_audio: OnAudio,
) {
    let guard = EndOnPanic(shared);
    let reason = read_until_closed(stream, &guard.0.events, lines, early, &mut on_audio);
    end(&guard.0, reason);
}

/// Reads and delivers until the connection ends, and says why it ended. Runs as long as the
/// socket is open; `end` shuts it down, which makes the read return.
fn read_until_closed(
    mut stream: UnixStream,
    events: &Sender<Incoming>,
    mut lines: LineBuffer,
    early: Vec<Vec<u8>>,
    on_audio: &mut OnAudio,
) -> CloseReason {
    for line in early {
        if let Err(reason) = deliver(&line, events, on_audio) {
            return reason;
        }
    }
    let mut buffer = [0u8; READ_CHUNK_BYTES];
    loop {
        let n = match stream.read(&mut buffer) {
            Ok(0) => return CloseReason::PeerClosed,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return CloseReason::ReadFailed(e.to_string()),
        };
        let ready = match lines.push(&buffer[..n]) {
            Ok(ready) => ready,
            Err(e) => return CloseReason::Framing(e),
        };
        if let Err(reason) = deliver_all(ready, events, on_audio) {
            return reason;
        }
    }
}

fn deliver_all(
    ready: Vec<Vec<u8>>,
    events: &Sender<Incoming>,
    on_audio: &mut OnAudio,
) -> Result<(), CloseReason> {
    ready
        .iter()
        .try_for_each(|line| deliver(line, events, on_audio))
}

/// Decodes one line and hands it on. Audio goes to `on_audio` first, then its size and level
/// to the `Inbox`. A dropped `Inbox` means nobody is listening, so the connection ends.
fn deliver(
    line: &[u8],
    events: &Sender<Incoming>,
    on_audio: &mut OnAudio,
) -> Result<(), CloseReason> {
    let incoming = match decode_line(line).map_err(CloseReason::Framing)? {
        ServerEvent::AudioDelta { audio } => {
            on_audio(&audio);
            Incoming::Audio {
                bytes: audio.len(),
                rms: rms(&audio),
            }
        }
        event => Incoming::Event(event),
    };
    events.send(incoming).map_err(|_| CloseReason::Local)
}

/// Writes queued frames until the connection ends.
fn write_loop(mut stream: UnixStream, shared: Arc<Shared>) {
    while let Some(job) = next_job(&shared) {
        if let Err(reason) = write_job(&mut stream, &shared, job) {
            end(&shared, reason);
            return;
        }
    }
}

/// The next frame to write, control first; `None` once the connection has ended.
fn next_job(shared: &Shared) -> Option<Job> {
    let mut queues = lock(shared);
    // Each pass either returns or waits for `wake`, which every enqueue and `end` notify.
    loop {
        if queues.ended {
            return None;
        }
        if let Some((line, since)) = queues.control.pop_front() {
            return Some(Job::Control(line, since));
        }
        if let Some(pcm16) = queues.audio.pop_front() {
            return Some(Job::Audio(pcm16));
        }
        queues = shared
            .wake
            .wait(queues)
            .expect("nothing panics while holding the voice queues");
    }
}

fn write_job(stream: &mut UnixStream, shared: &Shared, job: Job) -> Result<(), CloseReason> {
    match job {
        Job::Control(line, since) => write_watched(stream, shared, line.as_bytes(), Some(since)),
        Job::Audio(pcm16) => {
            let chunk = ClientEvent::audio_chunk(&pcm16);
            let line = chunk
                .line()
                .expect("a chunk within MAX_CHUNK_BYTES fits a line");
            write_watched(stream, shared, line.as_bytes(), None)
        }
    }
}

/// Writes one whole line, checking the deadlines each time the socket stays full for a tick.
/// Bounded: every pass writes bytes or runs a deadline's clock down.
fn write_watched(
    stream: &mut UnixStream,
    shared: &Shared,
    bytes: &[u8],
    control_since: Option<Instant>,
) -> Result<(), CloseReason> {
    let mut written = 0;
    let mut progress = Instant::now();
    while written < bytes.len() {
        match stream.write(&bytes[written..]) {
            Ok(0) => return Err(CloseReason::WriteFailed("the socket took no bytes".into())),
            Ok(n) => {
                written += n;
                progress = Instant::now();
            }
            Err(e) if timed_out(&e) => check_deadlines(shared, control_since, progress)?,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(CloseReason::WriteFailed(e.to_string())),
        }
    }
    Ok(())
}

/// A stuck write is dead after `STALL_DEADLINE`; a control frame, the one being written or the
/// oldest waiting, after `CONTROL_FLUSH_DEADLINE`.
fn check_deadlines(
    shared: &Shared,
    control_since: Option<Instant>,
    progress: Instant,
) -> Result<(), CloseReason> {
    if progress.elapsed() >= STALL_DEADLINE {
        return Err(CloseReason::Stalled);
    }
    let oldest = control_since.or_else(|| lock(shared).control.front().map(|(_, since)| *since));
    match oldest {
        Some(since) if since.elapsed() >= CONTROL_FLUSH_DEADLINE => {
            Err(CloseReason::ControlTimeout)
        }
        _ => Ok(()),
    }
}
