//! The call's sound (spec_voice §3.3): one GStreamer pipeline per call, built
//! on Begin and taken to Null on End, on error and on quit. Null is what
//! closes the microphone. The capture branch runs the microphone through
//! webrtcdsp's echo canceller at 48 kHz (it takes no rate nearer 24 kHz),
//! then through the core's speech gate (`speech::Uplink`), which takes it down
//! to the wire's 24 kHz in 100 ms blocks and sends every frame no voice is near
//! as silence. The playback branch feeds
//! the reply from the core's `PlaybackQueue` through the echo probe, and
//! silence when there is none, so the canceller always has its reference.
//! Nothing here touches GTK; only the bus watch runs on the main loop.

use fermix_client::realtime::playback::{rms, PlaybackQueue, SAMPLE_RATE};
use fermix_client::realtime::speech::{Uplink, CAPTURE_RATE};
use fermix_client::voice::NO_MICROPHONE;
use gst::prelude::*;
use gtk::glib;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// webrtcdsp takes 8, 16, 32 or 48 kHz only, and the speech gate takes 48.
const DSP_RATE: i32 = CAPTURE_RATE as i32;
/// 20 ms at 24 kHz: what one need-data pulls from the queue.
const BLOCK_SAMPLES: usize = 480;
/// appsrc queues about 40 ms of 24 kHz PCM16 at most (spec §3.3), and reports
/// that as its maximum latency, which leaves room for the speaker's 20 ms
/// processing deadline; left alone, a live appsrc reports 0 and the sink warns.
/// appsrc applies its maximum only with a minimum set, so that is set too.
const APPSRC_MAX_BYTES: u64 = 1_920;
const APPSRC_MAX_LATENCY_NS: i64 = 40_000_000;
const MIC_BUFFER_US: i64 = 40_000;
const SPEAKER_BUFFER_US: i64 = 60_000;
const PERIOD_US: i64 = 10_000;
/// Until the pipeline answers a latency query: appsrc's queue plus the
/// speaker's buffer, spec §3.3's bound on the audio still in flight.
const FALLBACK_LATENCY_MS: u64 = 100;
/// How long closing waits for Null before it logs and carries on.
const CLOSE_WAIT_MS: u64 = 2_000;
const CLIENT_NAME: &str = "Fermix";
const MICROPHONE: &str = "microphone";
const DSP: &str = "dsp";
const UPLINK: &str = "uplink";
const DOWNLINK: &str = "downlink";
const SPEAKER: &str = "speaker";
/// The bus message the playback thread posts when the queue runs dry.
const DRAINED: &str = "fermix-playback-drained";
/// The bus message the capture thread posts with each block it sends, and its one field: the
/// block's linear RMS.
const MIC_LEVEL: &str = "fermix-mic-level";
const RMS: &str = "rms";

/// The two-stage uplink gate (spec §3.3): open only while streaming is armed,
/// on the daemon's `listening`, and the microphone is not muted.
#[derive(Clone, Default)]
pub struct Gate {
    armed: Arc<AtomicBool>,
    muted: Arc<AtomicBool>,
}

impl Gate {
    pub fn arm(&self, on: bool) {
        self.armed.store(on, Ordering::SeqCst);
    }

    pub fn mute(&self, on: bool) {
        self.muted.store(on, Ordering::SeqCst);
    }

    pub fn open(&self) -> bool {
        self.armed.load(Ordering::SeqCst) && !self.muted.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoints {
    /// The default microphone and speakers, through the sound server.
    Sound,
    /// A live sine for the microphone and a clock-synced fakesink for the
    /// speakers. Every other element is the same, so tests run the real chain.
    Test,
}

/// Why the call's sound stopped. GStreamer's own words go to the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioFailure {
    /// The sound server is unreachable, refused the app, or went away.
    NoSoundServer,
    /// The sound server has no source to record from.
    NoMicrophone,
    /// Anything else, in GStreamer's words.
    Other(String),
}

impl AudioFailure {
    /// The Voice page's sentence (spec §4.3). `Other` has no row there; it
    /// borrows §1.4's sentence for a session that died.
    pub fn sentence(&self) -> String {
        match self {
            AudioFailure::NoSoundServer => {
                "Fermix cannot reach the sound server, so it cannot hear or speak. If you \
                 removed its sound permission, voice will not work until you restore it."
            }
            AudioFailure::NoMicrophone => NO_MICROPHONE,
            AudioFailure::Other(_) => "Voice stopped unexpectedly.",
        }
        .to_owned()
    }
}

/// One call's pipeline. Dropping it closes the pipeline too, so the
/// microphone never outlives the call.
pub struct AudioCall {
    pipeline: gst::Pipeline,
    /// `None` until `start` has the pipeline playing, and once released.
    watch: Option<gst::bus::BusWatchGuard>,
}

impl AudioCall {
    /// Builds and plays the pipeline, warm, with the gate closed (it panics on an open one: audio
    /// must not reach the socket before the daemon says `listening`). `on_mic` runs on a GStreamer
    /// streaming thread with each `speech::CHUNK_BYTES` block, only while `gate.open()`; it must
    /// not wait on the main loop, because `stop` joins that thread. `on_failure`, `on_drained`
    /// and `on_level` (each sent block's linear RMS, for the meter) run on the GTK main loop.
    pub fn start(
        endpoints: Endpoints,
        gate: Gate,
        playback: Arc<Mutex<PlaybackQueue>>,
        on_mic: Box<dyn Fn(Vec<u8>) + Send + Sync>,
        on_failure: Box<dyn Fn(AudioFailure)>,
        on_drained: Box<dyn Fn()>,
        on_level: Box<dyn Fn(f32)>,
    ) -> Result<AudioCall, AudioFailure> {
        assert!(!gate.open(), "a call starts with its gate closed");
        let pipeline = gst::Pipeline::new();
        // GStreamer names each probe uniquely in the process, and webrtcdsp
        // finds its probe by name in a process-wide list: two calls never share one.
        let probe = make("webrtcechoprobe")?;
        add_playback(&pipeline, endpoints, &probe, playback)?;
        add_capture(&pipeline, endpoints, probe.name().as_str(), gate, on_mic)?;
        let mut call = AudioCall {
            pipeline,
            watch: None,
        };
        if let Err(e) = call.pipeline.set_state(gst::State::Playing) {
            // Returning drops `call`, which takes the pipeline back to Null.
            let failure = first_error(&call.pipeline);
            return Err(failure.unwrap_or_else(|| AudioFailure::Other(e.to_string())));
        }
        let callbacks = MainLoopCallbacks {
            on_failure,
            on_drained,
            on_level,
        };
        call.watch = Some(watch_bus(&call.pipeline, callbacks)?);
        Ok(call)
    }

    /// How long playback takes from appsrc to the speaker, as the pipeline's
    /// latency query answers it; FALLBACK_LATENCY_MS until it answers live.
    pub fn sink_latency_ms(&self) -> u64 {
        let mut query = gst::query::Latency::new();
        if !self.pipeline.query(&mut query) {
            glib::g_debug!("fermix", "no latency answer yet");
            return FALLBACK_LATENCY_MS;
        }
        match query.result() {
            (true, min, _) => min.mseconds(),
            (false, _, _) => FALLBACK_LATENCY_MS,
        }
    }

    /// Pipeline to Null: this releases the microphone. No callback runs after it returns.
    pub fn stop(mut self) {
        self.release();
    }

    /// Removes the bus watch first, so no main-loop callback runs after this,
    /// then closes the pipeline, which joins the streaming threads, so no
    /// `on_mic` either.
    fn release(&mut self) {
        self.watch.take();
        close(&self.pipeline);
    }
}

impl Drop for AudioCall {
    fn drop(&mut self) {
        self.release();
    }
}

/// Takes the pipeline to Null and waits for it, CLOSE_WAIT_MS at most; past
/// that it logs and carries on.
fn close(pipeline: &gst::Pipeline) {
    if let Err(e) = pipeline.set_state(gst::State::Null) {
        glib::g_warning!("fermix", "the voice pipeline refused Null: {e}");
    }
    let (reached, state, _) = pipeline.state(gst::ClockTime::from_mseconds(CLOSE_WAIT_MS));
    if reached.is_err() || state != gst::State::Null {
        glib::g_warning!(
            "fermix",
            "the voice pipeline is {state:?}, not Null, after {CLOSE_WAIT_MS} ms"
        );
    }
}

fn pcm_caps(rate: i32) -> gst::Caps {
    gst_audio::AudioCapsBuilder::new_interleaved()
        .format(gst_audio::AudioFormat::S16le)
        .rate(rate)
        .channels(1)
        .build()
}

/// A plugin the runtime ships is missing, or elements would not link.
fn broken(what: &str, e: glib::BoolError) -> AudioFailure {
    glib::g_warning!("fermix", "voice audio could not {what}: {e}");
    AudioFailure::Other(format!("could not {what}: {e}"))
}

fn make(factory: &str) -> Result<gst::Element, AudioFailure> {
    gst::ElementFactory::make(factory)
        .build()
        .map_err(|e| broken(&format!("make {factory}"), e))
}

fn capsfilter(rate: i32) -> Result<gst::Element, AudioFailure> {
    gst::ElementFactory::make("capsfilter")
        .property("caps", pcm_caps(rate))
        .build()
        .map_err(|e| broken("make a capsfilter", e))
}

fn link(pipeline: &gst::Pipeline, chain: &[gst::Element]) -> Result<(), AudioFailure> {
    pipeline
        .add_many(chain)
        .map_err(|e| broken("add a branch", e))?;
    gst::Element::link_many(chain).map_err(|e| broken("link a branch", e))
}

fn microphone(endpoints: Endpoints) -> Result<gst::Element, AudioFailure> {
    let builder = match endpoints {
        Endpoints::Sound => gst::ElementFactory::make("pulsesrc")
            .property("client-name", CLIENT_NAME)
            .property("buffer-time", MIC_BUFFER_US)
            .property("latency-time", PERIOD_US),
        Endpoints::Test => gst::ElementFactory::make("audiotestsrc")
            .property("is-live", true)
            .property_from_str("wave", "sine"),
    };
    builder
        .name(MICROPHONE)
        .build()
        .map_err(|e| broken("make the microphone", e))
}

fn speaker(endpoints: Endpoints) -> Result<gst::Element, AudioFailure> {
    let builder = match endpoints {
        Endpoints::Sound => gst::ElementFactory::make("pulsesink")
            .property("client-name", CLIENT_NAME)
            .property("buffer-time", SPEAKER_BUFFER_US)
            .property("latency-time", PERIOD_US),
        Endpoints::Test => gst::ElementFactory::make("fakesink").property("sync", true),
    };
    builder
        .name(SPEAKER)
        .build()
        .map_err(|e| broken("make the speaker", e))
}

/// microphone → 48 kHz mono → webrtcdsp → appsink, whose callback runs the
/// speech gate and hands its whole 24 kHz blocks to `on_mic` while the gate is open.
fn add_capture(
    pipeline: &gst::Pipeline,
    endpoints: Endpoints,
    probe: &str,
    gate: Gate,
    on_mic: Box<dyn Fn(Vec<u8>) + Send + Sync>,
) -> Result<(), AudioFailure> {
    let dsp = gst::ElementFactory::make("webrtcdsp")
        .name(DSP)
        .property("probe", probe)
        .property("echo-cancel", true)
        .property("high-pass-filter", true)
        .property("noise-suppression", true)
        .property_from_str("noise-suppression-level", "low")
        .property("gain-control", true)
        .build()
        .map_err(|e| broken("make webrtcdsp", e))?;
    let uplink = gst_app::AppSink::builder()
        .name(UPLINK)
        .caps(&pcm_caps(DSP_RATE))
        .sync(false)
        .max_buffers(8)
        .drop(true)
        .build();
    let mut speech = Uplink::new();
    uplink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| take_mic(appsink, &mut speech, &gate, &on_mic))
            .build(),
    );
    let chain = [
        microphone(endpoints)?,
        make("audioconvert")?,
        make("audioresample")?,
        capsfilter(DSP_RATE)?,
        dsp,
        uplink.upcast(),
    ];
    link(pipeline, &chain)
}

/// Takes one appsink buffer of 48 kHz PCM16 through the speech gate and, while
/// the gate is open, hands on each whole block it lets out and posts its level.
/// While the gate is closed the detector still hears the room, so it is warm
/// when the gate opens, but the wire gets silence and nothing is handed on:
/// nothing captured while it was closed, not even the pre-roll, is ever sent.
/// An unreadable buffer is an error, which ends the call.
fn take_mic(
    appsink: &gst_app::AppSink,
    speech: &mut Uplink,
    gate: &Gate,
    on_mic: &(dyn Fn(Vec<u8>) + Send + Sync),
) -> Result<gst::FlowSuccess, gst::FlowError> {
    // It fails only at EOS or while flushing, with nothing left to hand on.
    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
    let map = sample
        .buffer()
        .and_then(|buffer| buffer.map_readable().ok());
    let Some(map) = map else {
        glib::g_warning!("fermix", "a microphone sample had no readable buffer");
        return Err(gst::FlowError::Error);
    };
    let (pairs, odd) = map.as_slice().as_chunks::<2>();
    if !odd.is_empty() {
        glib::g_warning!("fermix", "a microphone sample split a PCM16 sample");
        return Err(gst::FlowError::Error);
    }
    let capture: Vec<i16> = pairs.iter().map(|pair| i16::from_le_bytes(*pair)).collect();
    let open = gate.open();
    let blocks = speech.push(&capture, open);
    if !open {
        return Ok(gst::FlowSuccess::Ok);
    }
    for block in blocks {
        post_level(appsink, rms(&block));
        on_mic(block);
    }
    Ok(gst::FlowSuccess::Ok)
}

/// The meter's bridge to the main loop, as `post_drained` is playback's.
fn post_level(appsink: &gst_app::AppSink, level: f32) {
    let structure = gst::Structure::builder(MIC_LEVEL).field(RMS, level).build();
    let notice = gst::message::Application::builder(structure)
        .src(appsink)
        .build();
    if let Err(e) = appsink.post_message(notice) {
        glib::g_warning!("fermix", "the microphone level was not posted: {e}");
    }
}

/// appsrc → 48 kHz mono → echo probe → speaker. Each need-data pulls the next
/// 20 ms from the queue.
fn add_playback(
    pipeline: &gst::Pipeline,
    endpoints: Endpoints,
    probe: &gst::Element,
    playback: Arc<Mutex<PlaybackQueue>>,
) -> Result<(), AudioFailure> {
    let downlink = gst_app::AppSrc::builder()
        .name(DOWNLINK)
        .caps(&pcm_caps(SAMPLE_RATE as i32))
        .is_live(true)
        .format(gst::Format::Time)
        .max_bytes(APPSRC_MAX_BYTES)
        .min_latency(0)
        .max_latency(APPSRC_MAX_LATENCY_NS)
        .block(false)
        .build();
    let mut fed = Fed::default();
    downlink.set_callbacks(
        gst_app::AppSrcCallbacks::builder()
            .need_data(move |appsrc, _| feed(appsrc, &playback, &mut fed))
            .build(),
    );
    let chain = [
        downlink.upcast(),
        make("audioconvert")?,
        make("audioresample")?,
        capsfilter(DSP_RATE)?,
        probe.clone(),
        speaker(endpoints)?,
    ];
    link(pipeline, &chain)
}

/// The playback clock. Blocks are stamped from the running time of the first
/// request plus the samples sent since, so the clock-synced speaker paces the
/// requests in real time and the echo probe sees the far end on the
/// pipeline's clock.
#[derive(Default)]
struct Fed {
    start: Option<gst::ClockTime>,
    samples: u64,
}

fn samples_time(samples: u64) -> gst::ClockTime {
    gst::ClockTime::from_nseconds(samples * 1_000_000_000 / u64::from(SAMPLE_RATE))
}

/// Answers one need-data with the next 20 ms of reply, silence past its end.
fn feed(appsrc: &gst_app::AppSrc, playback: &Mutex<PlaybackQueue>, fed: &mut Fed) {
    let mut block = [0i16; BLOCK_SAMPLES];
    let pull = playback
        .lock()
        .expect("no thread panics while holding the playback queue")
        .pull(&mut block);
    let start = *fed.start.get_or_insert_with(|| {
        appsrc
            .current_running_time()
            .expect("a live source is asked for data only while playing")
    });
    let pts = start + samples_time(fed.samples);
    fed.samples += BLOCK_SAMPLES as u64;
    let bytes: Vec<u8> = block
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let mut buffer = gst::Buffer::from_mut_slice(bytes);
    let stamped = buffer.get_mut().expect("a new buffer has one owner");
    stamped.set_pts(pts);
    stamped.set_duration(samples_time(BLOCK_SAMPLES as u64));
    match appsrc.push_buffer(buffer) {
        Ok(_) => {}
        Err(gst::FlowError::Flushing) => glib::g_debug!("fermix", "playback stopping"),
        Err(e) => glib::g_warning!("fermix", "appsrc refused a playback block: {e:?}"),
    }
    if pull.drained {
        post_drained(appsrc);
    }
}

/// The bridge from the playback thread to the main loop. The thread only posts
/// a message on the pipeline's bus, which is thread-safe; the bus watch, on
/// the main loop already, calls `on_drained`. So the non-Send callback never
/// leaves the main thread, arrives in order with errors, and stops with the
/// watch.
fn post_drained(appsrc: &gst_app::AppSrc) {
    let notice = gst::message::Application::builder(gst::Structure::new_empty(DRAINED))
        .src(appsrc)
        .build();
    if let Err(e) = appsrc.post_message(notice) {
        glib::g_warning!("fermix", "the drained notice was not posted: {e}");
    }
}

/// What the bus watch calls, on the main loop.
struct MainLoopCallbacks {
    on_failure: Box<dyn Fn(AudioFailure)>,
    on_drained: Box<dyn Fn()>,
    on_level: Box<dyn Fn(f32)>,
}

fn watch_bus(
    pipeline: &gst::Pipeline,
    callbacks: MainLoopCallbacks,
) -> Result<gst::bus::BusWatchGuard, AudioFailure> {
    let mut watcher = Watcher {
        pipeline: pipeline.downgrade(),
        callbacks,
        failed: false,
    };
    let bus = pipeline.bus().expect("a pipeline has a bus");
    bus.add_watch_local(move |_, message| {
        watcher.handle(message);
        glib::ControlFlow::Continue
    })
    .map_err(|e| broken("watch the pipeline's bus", e))
}

/// The bus watch's state. It runs on the main loop only.
struct Watcher {
    pipeline: glib::WeakRef<gst::Pipeline>,
    callbacks: MainLoopCallbacks,
    failed: bool,
}

impl Watcher {
    fn handle(&mut self, message: &gst::Message) {
        match message.view() {
            gst::MessageView::Error(error) => self.fail(error),
            gst::MessageView::Warning(warning) => glib::g_warning!(
                "fermix",
                "voice audio: {} ({:?})",
                warning.error(),
                warning.debug()
            ),
            gst::MessageView::Latency(_) => self.recalculate_latency(),
            gst::MessageView::Application(notice) => self.notice(notice),
            _ => {}
        }
    }

    /// One of the streaming threads' own notices.
    fn notice(&self, notice: &gst::message::Application) {
        let Some(structure) = notice.structure() else {
            return;
        };
        if structure.has_name(DRAINED) {
            (self.callbacks.on_drained)();
        } else if structure.has_name(MIC_LEVEL) {
            let level = structure
                .get::<f32>(RMS)
                .expect("the capture thread posts its level as an f32");
            (self.callbacks.on_level)(level);
        }
    }

    /// Any error ends the call. The first closes the pipeline, which releases
    /// the microphone, and is reported; later ones are only logged.
    fn fail(&mut self, error: &gst::message::Error) {
        let failure = failure_from(error);
        if self.failed {
            return;
        }
        self.failed = true;
        if let Some(pipeline) = self.pipeline.upgrade() {
            close(&pipeline);
        }
        (self.callbacks.on_failure)(failure);
    }

    fn recalculate_latency(&self) {
        let Some(pipeline) = self.pipeline.upgrade() else {
            return;
        };
        if let Err(e) = pipeline.recalculate_latency() {
            glib::g_warning!("fermix", "the voice pipeline's latency did not settle: {e}");
        }
    }
}

/// The error a failed state change left on the bus, if any.
fn first_error(pipeline: &gst::Pipeline) -> Option<AudioFailure> {
    let bus = pipeline.bus().expect("a pipeline has a bus");
    let message = bus.pop_filtered(&[gst::MessageType::Error])?;
    match message.view() {
        gst::MessageView::Error(error) => Some(failure_from(error)),
        other => unreachable!("the error filter let through {other:?}"),
    }
}

/// Logs GStreamer's words and names the failure.
fn failure_from(error: &gst::message::Error) -> AudioFailure {
    let source = error
        .src()
        .map(|s| s.name().to_string())
        .unwrap_or_default();
    let cause = error.error();
    glib::g_warning!(
        "fermix",
        "voice audio: {source}: {cause} ({:?})",
        error.debug()
    );
    failure_of(&source, &cause)
}

/// The pulse elements' own words (gst-plugins-good 1.26 `pulsesrc.c` and
/// `pulsesink.c`, untranslated): "Failed to connect:" or "Failed to create
/// context" when the sound server is unreachable or refuses the app,
/// "Disconnected:" when it goes away mid-call, and "Failed to connect stream:"
/// when it has no device to open. "Disconnected: Entity killed" is the server
/// still there, ending the stream: for the microphone, its device went away
/// with no other to move to.
fn failure_of(source: &str, error: &glib::Error) -> AudioFailure {
    let text = error.message();
    let no_device = text.starts_with("Failed to connect stream:");
    let killed = text == "Disconnected: Entity killed";
    if source == MICROPHONE && (no_device || killed) {
        return AudioFailure::NoMicrophone;
    }
    if no_device {
        return AudioFailure::Other(text.to_owned());
    }
    let server = [
        "Failed to connect:",
        "Failed to create context",
        "Disconnected:",
    ];
    if server.iter().any(|words| text.starts_with(words)) {
        return AudioFailure::NoSoundServer;
    }
    AudioFailure::Other(text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fermix_client::realtime::speech::CHUNK_BYTES;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    const PATIENCE: Duration = Duration::from_secs(5);

    /// What the four callbacks saw.
    #[derive(Default)]
    struct Seen {
        blocks: Arc<Mutex<Vec<Vec<u8>>>>,
        failures: Rc<RefCell<Vec<AudioFailure>>>,
        drained: Rc<Cell<u32>>,
        levels: Rc<RefCell<Vec<f32>>>,
    }

    impl Seen {
        fn sent(&self) -> Vec<Vec<u8>> {
            self.blocks
                .lock()
                .expect("no test panics holding it")
                .clone()
        }

        fn blocks(&self) -> Vec<usize> {
            self.sent().iter().map(Vec::len).collect()
        }
    }

    fn queue_with(samples: usize) -> Arc<Mutex<PlaybackQueue>> {
        let mut queue = PlaybackQueue::new();
        queue.push_pcm16(&vec![0x20; samples * 2]);
        Arc::new(Mutex::new(queue))
    }

    fn start_test(gate: &Gate, playback: &Arc<Mutex<PlaybackQueue>>) -> (AudioCall, Seen) {
        let seen = Seen::default();
        let blocks = seen.blocks.clone();
        let failures = seen.failures.clone();
        let drained = seen.drained.clone();
        let levels = seen.levels.clone();
        let call = AudioCall::start(
            Endpoints::Test,
            gate.clone(),
            playback.clone(),
            Box::new(move |block| blocks.lock().expect("unpoisoned").push(block)),
            Box::new(move |failure| failures.borrow_mut().push(failure)),
            Box::new(move || drained.set(drained.get() + 1)),
            Box::new(move |level| levels.borrow_mut().push(level)),
        )
        .expect("the test pipeline starts");
        (call, seen)
    }

    /// Runs `test` with its own main context as this thread's default, so the
    /// bus watches of tests on parallel threads never share a loop.
    fn on_own_loop(test: impl FnOnce(&glib::MainContext)) {
        gst::init().expect("GStreamer initialises");
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| test(&context))
            .expect("a new context is free to acquire");
    }

    /// Iterates the loop until `done` holds, for `limit` at most.
    fn run_until(context: &glib::MainContext, limit: Duration, done: impl Fn() -> bool) -> bool {
        let deadline = Instant::now() + limit;
        while !done() && Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    fn run_for(context: &glib::MainContext, span: Duration) {
        run_until(context, span, || false);
    }

    fn assert_playing(call: &AudioCall) {
        let (reached, state, _) = call.pipeline.state(gst::ClockTime::from_seconds(5));
        assert!(reached.is_ok(), "the pipeline did not settle: {reached:?}");
        assert_eq!(state, gst::State::Playing);
    }

    fn rate(element: &gst::Element, pad: &str) -> Option<i32> {
        let caps = element.static_pad(pad)?.current_caps()?;
        caps.structure(0)?.get::<i32>("rate").ok()
    }

    #[test]
    fn the_test_pipeline_plays_through_the_dsp_at_48_khz() {
        on_own_loop(|context| {
            let (call, seen) = start_test(&Gate::default(), &queue_with(0));
            assert_playing(&call);
            let dsp = call.pipeline.by_name(DSP).expect("the dsp");
            let probe_name = dsp.property::<String>("probe");
            let probe = call
                .pipeline
                .by_name(&probe_name)
                .expect("the dsp's own probe");
            let uplink = call.pipeline.by_name(UPLINK).expect("the uplink");
            let negotiated = || {
                rate(&dsp, "src").is_some()
                    && rate(&probe, "src").is_some()
                    && rate(&uplink, "sink").is_some()
            };
            assert!(
                run_until(context, PATIENCE, negotiated),
                "caps never settled"
            );
            assert_eq!(rate(&dsp, "sink"), Some(48_000));
            assert_eq!(rate(&dsp, "src"), Some(48_000));
            assert_eq!(rate(&probe, "sink"), Some(48_000));
            // The speech gate takes 48 kHz and decimates to the wire's 24 kHz itself.
            assert_eq!(rate(&uplink, "sink"), Some(48_000));
            assert!(seen.failures.borrow().is_empty());
            call.stop();
        });
    }

    #[test]
    fn a_closed_or_muted_gate_sends_nothing() {
        on_own_loop(|context| {
            let gate = Gate::default();
            let (call, seen) = start_test(&gate, &queue_with(0));
            assert_playing(&call);
            run_for(context, Duration::from_millis(600));
            assert!(seen.blocks().is_empty(), "blocks while not armed");
            gate.mute(true);
            gate.arm(true);
            run_for(context, Duration::from_millis(600));
            assert!(seen.blocks().is_empty(), "blocks while muted");
            // The control: the same capture sends as soon as the gate opens.
            gate.mute(false);
            assert!(run_until(context, PATIENCE, || !seen.blocks().is_empty()));
            call.stop();
        });
    }

    #[test]
    fn an_open_gate_sends_whole_100_ms_blocks_until_muted() {
        on_own_loop(|context| {
            let gate = Gate::default();
            let (call, seen) = start_test(&gate, &queue_with(0));
            assert_playing(&call);
            gate.arm(true);
            assert!(run_until(context, PATIENCE, || seen.blocks().len() >= 3));
            let blocks = seen.blocks();
            assert!(blocks.iter().all(|&len| len == CHUNK_BYTES), "{blocks:?}");
            gate.mute(true);
            // A block already past the gate may still land.
            run_for(context, Duration::from_millis(150));
            let at_mute = seen.blocks().len();
            run_for(context, Duration::from_millis(600));
            assert_eq!(seen.blocks().len(), at_mute, "blocks while muted");
            call.stop();
        });
    }

    /// The meter hears of every block sent, on the main loop, in order, as its RMS. (The test
    /// microphone's loud steady sine passes the speech gate after webrtcdsp; what the gate
    /// lets through is the core's to test, with real speech and noise.)
    #[test]
    fn each_block_sent_reports_its_level_on_the_main_loop() {
        on_own_loop(|context| {
            let gate = Gate::default();
            let (call, seen) = start_test(&gate, &queue_with(0));
            assert_playing(&call);
            gate.arm(true);
            assert!(run_until(context, PATIENCE, || seen.levels.borrow().len() >= 3));
            gate.mute(true);
            run_for(context, Duration::from_millis(300));
            let expected: Vec<f32> = seen.sent().iter().map(|block| rms(block)).collect();
            assert_eq!(*seen.levels.borrow(), expected);
            call.stop();
        });
    }

    #[test]
    fn a_queued_reply_plays_in_real_time_and_drains_once() {
        on_own_loop(|context| {
            let playback = queue_with(12_000); // 500 ms
            let (call, seen) = start_test(&Gate::default(), &playback);
            assert_playing(&call);
            let began = Instant::now();
            assert!(run_until(context, PATIENCE, || seen.drained.get() > 0));
            let took = began.elapsed();
            assert!(
                took >= Duration::from_millis(400),
                "played too fast: {took:?}"
            );
            {
                let queue = playback.lock().expect("unpoisoned");
                assert!(queue.is_empty());
                assert_eq!(queue.played_since_anchor(), 12_000);
            }
            run_for(context, Duration::from_millis(400));
            assert_eq!(seen.drained.get(), 1, "silence drained again");
            playback
                .lock()
                .expect("unpoisoned")
                .push_pcm16(&[0x20; 960]);
            assert!(run_until(context, PATIENCE, || seen.drained.get() == 2));
            assert!(seen.failures.borrow().is_empty());
            call.stop();
        });
    }

    #[test]
    fn stop_reaches_null_and_nothing_fires_after() {
        on_own_loop(|context| {
            let gate = Gate::default();
            let playback = queue_with(0);
            let (call, seen) = start_test(&gate, &playback);
            assert_playing(&call);
            gate.arm(true);
            assert!(run_until(context, PATIENCE, || !seen.blocks().is_empty()));
            let pipeline = call.pipeline.clone();
            call.stop();
            assert_eq!(pipeline.current_state(), gst::State::Null);
            let blocks = seen.blocks().len();
            // Null flushes the bus; open it again, so only a removed watch
            // keeps these two messages from reaching the callbacks.
            let bus = pipeline.bus().expect("a pipeline has a bus");
            bus.set_flushing(false);
            let mic = pipeline.by_name(MICROPHONE).expect("the microphone");
            gst::element_error!(mic, gst::ResourceError::Failed, ("Failed to connect: x"));
            let drained = gst::message::Application::new(gst::Structure::new_empty(DRAINED));
            bus.post(drained).expect("the bus takes it");
            playback
                .lock()
                .expect("unpoisoned")
                .push_pcm16(&[0x20; 960]);
            run_for(context, Duration::from_millis(500));
            assert_eq!(seen.blocks().len(), blocks, "on_mic after stop");
            assert!(seen.failures.borrow().is_empty(), "on_failure after stop");
            assert_eq!(seen.drained.get(), 0, "on_drained after stop");
            assert!(
                !playback.lock().expect("unpoisoned").is_empty(),
                "pulled after stop"
            );
        });
    }

    #[test]
    #[should_panic(expected = "a call starts with its gate closed")]
    fn a_call_never_starts_with_its_gate_open() {
        on_own_loop(|_| {
            let gate = Gate::default();
            gate.arm(true);
            start_test(&gate, &queue_with(0));
        });
    }

    #[test]
    fn dropping_a_call_also_closes_the_pipeline() {
        on_own_loop(|_| {
            let (call, _seen) = start_test(&Gate::default(), &queue_with(0));
            assert_playing(&call);
            let pipeline = call.pipeline.clone();
            drop(call);
            assert_eq!(pipeline.current_state(), gst::State::Null);
        });
    }

    #[test]
    fn a_pipeline_error_is_reported_once_and_closes_the_pipeline() {
        on_own_loop(|context| {
            let (call, seen) = start_test(&Gate::default(), &queue_with(0));
            assert_playing(&call);
            let mic = call.pipeline.by_name(MICROPHONE).expect("the microphone");
            gst::element_error!(
                mic,
                gst::ResourceError::Failed,
                ("Failed to connect: Connection refused")
            );
            gst::element_error!(
                mic,
                gst::ResourceError::Failed,
                ("Failed to connect stream: x")
            );
            assert!(run_until(context, PATIENCE, || !seen
                .failures
                .borrow()
                .is_empty()));
            run_for(context, Duration::from_millis(200));
            assert_eq!(*seen.failures.borrow(), vec![AudioFailure::NoSoundServer]);
            assert_eq!(call.pipeline.current_state(), gst::State::Null);
            call.stop();
        });
    }

    #[test]
    fn sink_latency_comes_from_the_playing_pipeline() {
        on_own_loop(|_| {
            let (call, _seen) = start_test(&Gate::default(), &queue_with(0));
            assert_playing(&call);
            // fakesink adds no latency, so only a real answer is this small.
            assert!(call.sink_latency_ms() < FALLBACK_LATENCY_MS);
            call.stop();
        });
    }

    #[test]
    fn pulse_errors_map_to_the_failures_the_voice_page_names() {
        let error = |text: &str| glib::Error::new(gst::ResourceError::Failed, text);
        let cases = [
            (
                MICROPHONE,
                "Failed to connect: Connection refused",
                AudioFailure::NoSoundServer,
            ),
            (
                SPEAKER,
                "Failed to connect: Access denied",
                AudioFailure::NoSoundServer,
            ),
            (
                SPEAKER,
                "Disconnected: Connection terminated",
                AudioFailure::NoSoundServer,
            ),
            (
                MICROPHONE,
                "Failed to create context",
                AudioFailure::NoSoundServer,
            ),
            // The server kills a recording stream whose device went away with nowhere to move it.
            (
                MICROPHONE,
                "Disconnected: Entity killed",
                AudioFailure::NoMicrophone,
            ),
            (
                SPEAKER,
                "Disconnected: Entity killed",
                AudioFailure::NoSoundServer,
            ),
            (
                MICROPHONE,
                "Failed to connect stream: No such entity",
                AudioFailure::NoMicrophone,
            ),
            (
                SPEAKER,
                "Failed to connect stream: No such entity",
                AudioFailure::Other("Failed to connect stream: No such entity".into()),
            ),
            (
                DSP,
                "Internal data stream error.",
                AudioFailure::Other("Internal data stream error.".into()),
            ),
        ];
        for (source, text, expected) in cases {
            assert_eq!(
                failure_of(source, &error(text)),
                expected,
                "{source}: {text}"
            );
        }
    }

    #[test]
    fn each_failure_has_its_voice_page_sentence() {
        assert_eq!(
            AudioFailure::NoSoundServer.sentence(),
            "Fermix cannot reach the sound server, so it cannot hear or speak. If you removed \
             its sound permission, voice will not work until you restore it."
        );
        assert_eq!(AudioFailure::NoMicrophone.sentence(), NO_MICROPHONE);
        assert_eq!(
            AudioFailure::Other("Internal data stream error.".into()).sentence(),
            "Voice stopped unexpectedly."
        );
    }
}
