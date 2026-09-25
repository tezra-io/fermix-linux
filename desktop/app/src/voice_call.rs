//! The voice call's controller (spec_voice §1). The core's `Session` decides;
//! this performs its effects in order and feeds back what happened: the
//! socket's answers, the pipeline's failures and drains, and the call-start
//! timer. The socket and the pipeline live here and nowhere else.

use crate::app::App;
use crate::audio::{AudioCall, AudioFailure, Endpoints, Gate};
use crate::daemon::fermix_home;
use fermix_client::realtime::client::{self, CloseReason, ConnectError, Inbox, Incoming, Outbox};
use fermix_client::realtime::playback::{played_ms, PlaybackQueue};
use fermix_client::realtime::protocol::{ClientEvent, ServerEvent};
use fermix_client::realtime::session::{Effect, Input, Session, CALL_START_DEADLINE};
use fermix_client::voice::Reach;
use gtk::{gio, glib};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Inputs one user action or wire event may lead to, through effects that
/// answer at once. The reducer's longest chain is two; past this it is a loop.
const MAX_STEPS: usize = 8;

/// Everything one voice connection owns.
pub struct Call {
    pub session: Session,
    /// What the last connection attempt found; a new attempt forgets it.
    pub reach: Reach,
    outbox: Option<Outbox>,
    audio: Option<AudioCall>,
    gate: Gate,
    playback: Arc<Mutex<PlaybackQueue>>,
    endpoints: Endpoints,
}

impl Call {
    pub fn new(endpoints: Endpoints) -> Call {
        Call {
            session: Session::new(),
            reach: Reach::Untried,
            outbox: None,
            audio: None,
            gate: Gate::default(),
            playback: Arc::new(Mutex::new(PlaybackQueue::new())),
            endpoints,
        }
    }

    /// How much of the current reply has reached the speaker.
    pub fn played_ms(&self) -> u64 {
        let played = lock(&self.playback).played_since_anchor();
        let latency = self.audio.as_ref().map_or(0, AudioCall::sink_latency_ms);
        played_ms(played, latency)
    }
}

impl App {
    /// Applies `input` and performs what follows, then redraws the voice views.
    /// An effect that answers at once ends its batch: what follows it belongs
    /// to a call that did not start (no `call_start` for a dead microphone).
    pub fn voice_input(self: &Rc<Self>, input: Input) {
        let mut pending = VecDeque::from([input]);
        for _ in 0..MAX_STEPS {
            let Some(input) = pending.pop_front() else {
                break;
            };
            let effects = self.call.borrow_mut().session.apply(input);
            if let Some(answer) = effects.into_iter().find_map(|e| self.perform(e)) {
                pending.push_back(answer);
            }
        }
        assert!(
            pending.is_empty(),
            "the voice session kept answering itself past {MAX_STEPS} steps: {pending:?}"
        );
        self.render_voice();
    }

    /// One effect. An effect that fails at once answers with its input.
    fn perform(self: &Rc<Self>, effect: Effect) -> Option<Input> {
        let mut call = self.call.borrow_mut();
        match effect {
            Effect::Connect => {
                drop(call);
                self.connect();
            }
            Effect::Send(event) => send(call.outbox.as_ref(), event),
            Effect::StartAudio => {
                drop(call);
                let failure = self.start_audio().err()?;
                return Some(Input::AudioFailed(failure.sentence()));
            }
            Effect::StopAudio => {
                // Reaching Null can take a moment; the call is not borrowed meanwhile.
                let audio = call.audio.take();
                drop(call);
                if let Some(audio) = audio {
                    audio.stop();
                }
            }
            Effect::Arm(on) => call.gate.arm(on),
            Effect::MuteMic(on) => call.gate.mute(on),
            Effect::FlushPlayback => lock(&call.playback).clear(),
            Effect::ResetAnchor => lock(&call.playback).reset_anchor(),
            Effect::WatchCallStart(number) => {
                let weak = Rc::downgrade(self);
                glib::timeout_add_local_once(CALL_START_DEADLINE, move || {
                    if let Some(app) = weak.upgrade() {
                        app.voice_input(Input::CallDeadline(number));
                    }
                });
            }
            Effect::Disconnect => call.outbox = None,
        }
        None
    }

    /// Opens `realtime.sock` off the main thread. Reply audio goes straight
    /// from the socket's reader to the playback queue; the rest comes back
    /// through the `Inbox`.
    fn connect(self: &Rc<Self>) {
        let path = fermix_home().join("realtime.sock");
        let playback = Arc::clone(&self.call.borrow().playback);
        let on_audio: client::OnAudio = Box::new(move |pcm16| {
            lock(&playback).push_pcm16(pcm16);
        });
        self.call.borrow_mut().reach = Reach::Untried;
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let answer = gio::spawn_blocking(move || client::connect(&path, on_audio)).await;
            let Some(app) = weak.upgrade() else { return };
            let answer = answer.unwrap_or_else(|_| {
                Err(ConnectError::Io(std::io::Error::other(
                    "the voice connection thread panicked",
                )))
            });
            match answer {
                Ok((outbox, inbox)) => {
                    app.call.borrow_mut().outbox = Some(outbox);
                    app.pump(inbox);
                    app.voice_input(Input::Connected);
                }
                Err(error) => {
                    glib::g_info!("fermix", "voice did not connect: {error:?}");
                    app.call.borrow_mut().reach = Reach::from_connect(&error);
                    app.voice_input(Input::ConnectFailed(error));
                }
            }
        });
    }

    /// Hands every message from the daemon to the session, in order, until
    /// the connection's one `Closed`, which the `Inbox` always ends with.
    fn pump(self: &Rc<Self>, inbox: Inbox) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut inbox = inbox;
            loop {
                let next = gio::spawn_blocking(move || {
                    let incoming = inbox.recv();
                    (inbox, incoming)
                })
                .await;
                let Ok((back, incoming)) = next else {
                    // The session must still hear that the connection is over.
                    glib::g_critical!("fermix", "the voice reader thread panicked");
                    let reason = CloseReason::ReadFailed("the voice reader panicked".into());
                    deliver(&weak, Incoming::Closed(reason));
                    return;
                };
                inbox = back;
                if !deliver(&weak, incoming) {
                    return;
                }
            }
        });
    }

    /// Builds the call's pipeline with the gate shut. The microphone's blocks
    /// go to the socket from GStreamer's own thread, only while the gate is open.
    fn start_audio(self: &Rc<Self>) -> Result<(), AudioFailure> {
        // Once per process; later calls return at once. Chat never needs it.
        gst::init().map_err(|e| AudioFailure::Other(format!("GStreamer did not start: {e}")))?;
        let (endpoints, gate, playback, outbox) = {
            let call = self.call.borrow();
            let Some(outbox) = call.outbox.clone() else {
                return Err(AudioFailure::Other("no voice connection".into()));
            };
            (
                call.endpoints,
                call.gate.clone(),
                Arc::clone(&call.playback),
                outbox,
            )
        };
        let on_mic = Box::new(move |block: Vec<u8>| outbox.audio(block));
        let weak = Rc::downgrade(self);
        let on_failure = Box::new(move |failure: AudioFailure| {
            glib::g_warning!("fermix", "the call's sound failed: {failure:?}");
            if let Some(app) = weak.upgrade() {
                app.voice_input(Input::AudioFailed(failure.sentence()));
            }
        });
        let weak = Rc::downgrade(self);
        let queue = Arc::clone(&playback);
        let on_drained = Box::new(move || {
            // The next reply may already be queued: the drain is then stale.
            if !lock(&queue).is_empty() {
                return;
            }
            if let Some(app) = weak.upgrade() {
                app.voice_input(Input::Drained);
            }
        });
        match AudioCall::start(endpoints, gate, playback, on_mic, on_failure, on_drained) {
            Ok(audio) => {
                self.call.borrow_mut().audio = Some(audio);
                Ok(())
            }
            Err(failure) => {
                glib::g_warning!("fermix", "the call's sound did not start: {failure:?}");
                Err(failure)
            }
        }
    }

    /// Ends a call that is up, then lets go of the connection, and starts the
    /// session over, since it no longer has one. The daemon takes the closed
    /// socket as the end of the call too.
    pub fn hang_up(self: &Rc<Self>) {
        if self.call.borrow().session.in_call() {
            self.voice_input(Input::End);
        }
        let (audio, outbox) = {
            let mut call = self.call.borrow_mut();
            call.session = Session::new();
            (call.audio.take(), call.outbox.take())
        };
        if let Some(audio) = audio {
            audio.stop();
        }
        if let Some(outbox) = outbox {
            outbox.close();
        }
    }
}

/// One message from the daemon to the session. False once the connection is
/// over (or the app is gone), which ends the pump.
fn deliver(app: &std::rc::Weak<App>, incoming: Incoming) -> bool {
    // The core has no logger; a word this app does not know yet goes here.
    if let Incoming::Event(ServerEvent::Unknown(kind)) = &incoming {
        glib::g_debug!("fermix", "the voice socket sent an unknown event: {kind}");
    }
    let closed = matches!(incoming, Incoming::Closed(_));
    let Some(app) = app.upgrade() else {
        return false;
    };
    app.voice_input(Input::Wire(incoming));
    !closed
}

/// A control frame. A closed connection needs no report here: the `Inbox`
/// already carries why, and the session hears it next.
fn send(outbox: Option<&Outbox>, event: ClientEvent) {
    let Some(outbox) = outbox else {
        glib::g_warning!("fermix", "no voice connection for {event:?}");
        return;
    };
    if outbox.control(event).is_err() {
        glib::g_debug!(
            "fermix",
            "the voice connection closed before a control frame"
        );
    }
}

/// The playback queue's lock. A panic while holding it already ended the call's
/// sound; the queue itself is still whole, so carry on with it.
fn lock(queue: &Mutex<PlaybackQueue>) -> std::sync::MutexGuard<'_, PlaybackQueue> {
    queue.lock().unwrap_or_else(|poisoned| {
        glib::g_warning!("fermix", "the playback queue was poisoned");
        poisoned.into_inner()
    })
}
