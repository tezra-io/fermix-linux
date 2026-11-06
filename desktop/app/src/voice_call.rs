//! The voice call's controller (spec_voice §1). The core's `Session` decides;
//! this performs its effects in order and feeds back what happened: the
//! socket's answers, the pipeline's failures and drains, and the call-start
//! timer. The socket and the pipeline live here and nowhere else, and so does
//! reporting what has played of a Grok call's reply (`playback_progress`).

use crate::app::App;
use crate::audio::{AudioCall, AudioFailure, Endpoints, Gate};
use crate::daemon::fermix_home;
use fermix_client::realtime::client::{self, CloseReason, ConnectError, Inbox, Incoming, Outbox};
use fermix_client::realtime::playback::{played_ms, PlaybackQueue};
use fermix_client::realtime::protocol::{ClientEvent, ServerEvent};
use fermix_client::realtime::session::{
    Effect, Input, Session, CALL_START_DEADLINE, PROGRESS_INTERVAL,
};
use fermix_client::voice::Reach;
use gtk::{gio, glib};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Inputs one user action or wire event may lead to, through effects that
/// answer at once. The reducer's longest chain is two; past this it is a loop.
const MAX_STEPS: usize = 8;

/// How long playback must stay without voice before it counts as the end of a
/// reply (macOS `AudioOwner.drainGrace`). A reply arrives faster than it plays,
/// but a network pause can play out its voice between two of its chunks, and
/// that is not its end: the pet would turn from speaking to listening and back.
const DRAIN_GRACE: Duration = Duration::from_millis(350);

/// Everything one voice connection owns.
pub struct Call {
    pub session: Session,
    /// What the last connection attempt found; a new attempt forgets it.
    pub reach: Reach,
    outbox: Option<Outbox>,
    audio: Option<AudioCall>,
    gate: Gate,
    playback: Arc<Mutex<PlaybackQueue>>,
    /// Reports what has played every `PROGRESS_INTERVAL` while the call's sound runs.
    progress_tick: Option<glib::SourceId>,
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
            progress_tick: None,
            endpoints,
        }
    }

    /// How much of the current reply has reached the speaker.
    pub fn played_ms(&self) -> u64 {
        let played = lock(&self.playback).played_since_anchor();
        let latency = self.audio.as_ref().map_or(0, AudioCall::sink_latency_ms);
        played_ms(played, latency)
    }

    /// Sends what changed of each playback id since the last report. Only a
    /// call takes them: after `call_stop` the daemon answers one by closing the
    /// socket. An OpenAI call's audio carries no ids, so it sends nothing.
    fn report_playback(&self) {
        if !self.session.in_call() {
            return;
        }
        for progress in lock(&self.playback).take_progress() {
            send(
                self.outbox.as_ref(),
                ClientEvent::PlaybackProgress(progress),
            );
        }
    }

    fn stop_progress_tick(&mut self) {
        if let Some(tick) = self.progress_tick.take() {
            tick.remove();
        }
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
                call.stop_progress_tick();
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
            Effect::NewPlayback => lock(&call.playback).start_call(),
            Effect::ReportPlayback => call.report_playback(),
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
        let on_audio: client::OnAudio = Box::new(move |pcm16, playback_id| {
            lock(&playback).push_pcm16(pcm16, playback_id);
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
        let on_drained = drain_reporter(self, Arc::clone(&playback));
        let weak = Rc::downgrade(self);
        let on_level = Box::new(move |rms: f32| {
            if let Some(app) = weak.upgrade() {
                app.voice_input(Input::MicLevel(rms));
            }
        });
        let started = AudioCall::start(
            endpoints, gate, playback, on_mic, on_failure, on_drained, on_level,
        );
        match started {
            Ok(audio) => {
                let tick = self.progress_tick();
                let mut call = self.call.borrow_mut();
                call.audio = Some(audio);
                call.stop_progress_tick();
                call.progress_tick = Some(tick);
                Ok(())
            }
            Err(failure) => {
                glib::g_warning!("fermix", "the call's sound did not start: {failure:?}");
                Err(failure)
            }
        }
    }

    /// Reports playback every `PROGRESS_INTERVAL` until `stop_progress_tick`.
    /// A report that says nothing changed sends nothing.
    fn progress_tick(self: &Rc<Self>) -> glib::SourceId {
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(PROGRESS_INTERVAL, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.call.borrow().report_playback();
            glib::ControlFlow::Continue
        })
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
            call.stop_progress_tick();
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

/// Reports the end of a reply once playback has stayed without voice for
/// `DRAIN_GRACE`, with or without Live's padding still queued. A later drain of
/// voice supersedes a pending one, and voice queued at the deadline means the
/// drain was a gap, not the end. What has played of each playback id goes out
/// at once on either notice, so a reply's last id is reported drained when the
/// queue runs dry; one that hands over to the next id inside the queue goes out
/// on the next tick.
fn drain_reporter(app: &Rc<App>, queue: Arc<Mutex<PlaybackQueue>>) -> Box<dyn Fn(bool)> {
    let weak = Rc::downgrade(app);
    let drains = Rc::new(Cell::new(0_u64));
    Box::new(move |voice| {
        if let Some(app) = weak.upgrade() {
            app.call.borrow().report_playback();
        }
        let Some(drain) = number_drain(&drains, voice) else {
            return;
        };
        let (weak, queue, drains) = (weak.clone(), Arc::clone(&queue), Rc::clone(&drains));
        glib::timeout_add_local_once(DRAIN_GRACE, move || {
            if !reply_ended(&drains, drain, &queue) {
                return;
            }
            if let Some(app) = weak.upgrade() {
                app.voice_input(Input::Drained);
            }
        });
    })
}

/// Numbers a drain of voice, the newest of `drains`. The queue running dry with
/// no voice ending is no drain of a reply, and supersedes none.
fn number_drain(drains: &Cell<u64>, voice: bool) -> Option<u64> {
    if !voice {
        return None;
    }
    drains.set(drains.get() + 1);
    Some(drains.get())
}

/// At `drain`'s deadline: the reply has ended if no later drain of voice came
/// and nothing up to the end of the last voiced chunk is still queued. Padding
/// after that chunk does not hold the reply open.
fn reply_ended(drains: &Cell<u64>, drain: u64, queue: &Mutex<PlaybackQueue>) -> bool {
    drains.get() == drain && !lock(queue).voice_queued()
}

/// The playback queue's lock. A panic while holding it already ended the call's
/// sound; the queue itself is still whole, so carry on with it.
fn lock(queue: &Mutex<PlaybackQueue>) -> std::sync::MutexGuard<'_, PlaybackQueue> {
    queue.lock().unwrap_or_else(|poisoned| {
        glib::g_warning!("fermix", "the playback queue was poisoned");
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 100 ms of Live's padding, digital silence.
    const PADDING: [u8; 4_800] = [0; 4_800];

    /// 100 ms of voice, at int16 RMS 2,000.
    fn voice() -> Vec<u8> {
        2_000_i16.to_le_bytes().repeat(2_400)
    }

    fn pulled(queue: &Mutex<PlaybackQueue>, samples: usize) {
        lock(queue).pull(&mut vec![0; samples]);
    }

    #[test]
    fn only_a_drain_of_voice_is_numbered() {
        let drains = Cell::new(0);
        assert_eq!(number_drain(&drains, true), Some(1));
        assert_eq!(number_drain(&drains, false), None, "the queue ran dry");
        assert_eq!(number_drain(&drains, true), Some(2));
    }

    /// Live's padding keeps the queue full for the whole call, so the deadline asks whether
    /// voice is queued, not whether anything is.
    #[test]
    fn a_reply_has_ended_at_its_deadline_with_padding_still_queued() {
        let queue = Mutex::new(PlaybackQueue::new());
        let drains = Cell::new(0);
        lock(&queue).push_pcm16(&voice(), None);
        lock(&queue).push_pcm16(&PADDING, None);
        let drain = number_drain(&drains, true).expect("a drain of voice");
        assert!(
            !reply_ended(&drains, drain, &queue),
            "the voice is still queued"
        );
        pulled(&queue, 2_400);
        assert!(reply_ended(&drains, drain, &queue));
        assert!(!lock(&queue).is_empty(), "the padding is still queued");
    }

    #[test]
    fn a_later_drain_or_more_voice_holds_the_reply_open() {
        let queue = Mutex::new(PlaybackQueue::new());
        let drains = Cell::new(0);
        let first = number_drain(&drains, true).expect("a drain of voice");
        number_drain(&drains, false);
        assert!(
            reply_ended(&drains, first, &queue),
            "the queue running dry supersedes nothing"
        );
        let second = number_drain(&drains, true).expect("a drain of voice");
        assert!(!reply_ended(&drains, first, &queue), "superseded");
        lock(&queue).push_pcm16(&PADDING, None);
        lock(&queue).push_pcm16(&voice(), None);
        assert!(
            !reply_ended(&drains, second, &queue),
            "voice came in the gap"
        );
    }
}
