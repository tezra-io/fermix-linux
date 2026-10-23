//! A voice note's levels and its length (`waveform`), read by decoding it once:
//! `filesrc ! oggdemux ! opusdec ! audioconvert ! audioresample ! appsink`, the appsink taking
//! mono 32-bit float at `waveform::RATE`. Its callback adds each buffer to the note's `Power` on
//! the streaming thread, and ends the pass past `MAX_FRAMES`; the bus watch hands what the pass
//! came to back on the main loop. An Ogg holding no Opus stops at oggdemux, its stream linked to
//! nothing ("not-linked"), and that failure is what turns the note into its file tile.

use crate::audio::{self, AudioFailure};
use fermix_client::media::{Admission, MAX_WAITING};
use fermix_client::waveform::{self, Levels, Power, Unheard};
use futures_channel::oneshot;
use gst::prelude::*;
use gtk::glib;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// The page's level passes take turns, two at once and 64 waiting (`media::Admission`, the
/// pictures' budget), the oldest waiting giving way: a chat full of long notes never decodes them
/// all together.
#[derive(Default)]
pub(crate) struct Turns {
    admission: RefCell<Admission<oneshot::Sender<()>>>,
}

/// A pass's turn, given on when it is dropped, however the pass ends.
pub(crate) struct Turn(Rc<Turns>);

impl Turns {
    /// Waits for a turn; `None` when this pass gave way to newer ones.
    pub(crate) async fn wait(self: &Rc<Self>) -> Option<Turn> {
        let (go, given) = oneshot::channel();
        let (run, gave_way) = self.admission.borrow_mut().ask(go);
        drop(gave_way);
        if let Some(go) = run {
            go.send(()).expect("the receiver is held here");
        }
        given.await.ok()?;
        Some(Turn(self.clone()))
    }

    /// The next waiting pass runs; one dropped meanwhile, its chip gone, hands its turn on.
    fn next(&self) {
        for _ in 0..=MAX_WAITING {
            let Some(go) = self.admission.borrow_mut().done() else {
                return;
            };
            if go.send(()).is_ok() {
                return;
            }
        }
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        self.0.next();
    }
}

/// One pass. Dropping it ends it, the watch first, so `done` never runs after.
pub(crate) struct Pass {
    pipeline: gst::Pipeline,
    watch: Option<gst::bus::BusWatchGuard>,
}

impl Drop for Pass {
    fn drop(&mut self) {
        self.watch.take();
        audio::close(&self.pipeline);
    }
}

/// Starts reading `path`'s levels. Either it fails here, or `done` runs once, on this thread's
/// main loop, with what the pass came to, unless the pass is dropped first.
pub(crate) fn start(
    path: &Path,
    done: impl FnOnce(Result<Levels, Unheard>) + 'static,
) -> Result<Pass, Unheard> {
    // Once per process; later calls return at once.
    gst::init().map_err(|e| Unheard::Undecodable(format!("GStreamer did not start: {e}")))?;
    let pipeline = gst::Pipeline::new();
    let power = Arc::new(Mutex::new(Power::new(waveform::WINDOW_FRAMES)));
    let tail = [
        audio::make("audioconvert").map_err(undecodable)?,
        audio::make("audioresample").map_err(undecodable)?,
        sink(power.clone()).upcast(),
    ];
    ogg_opus(&pipeline, path, &tail).map_err(undecodable)?;
    let mut pass = Pass {
        pipeline,
        watch: None,
    };
    if let Err(e) = pass.pipeline.set_state(gst::State::Playing) {
        // Returning drops `pass`, which takes the pipeline back to Null.
        return Err(Unheard::Undecodable(
            first_error(&pass.pipeline).unwrap_or_else(|| e.to_string()),
        ));
    }
    pass.watch = Some(watch(&pass.pipeline, power, done)?);
    Ok(pass)
}

fn undecodable(failure: AudioFailure) -> Unheard {
    Unheard::Undecodable(why(failure))
}

/// An element that would not make, add or link, in GStreamer's words.
pub(crate) fn why(failure: AudioFailure) -> String {
    match failure {
        AudioFailure::Other(why) => why,
        other => format!("{other:?}"),
    }
}

/// `filesrc ! oggdemux`, the note's stream linked to `opusdec ! tail` once oggdemux finds it.
pub(crate) fn ogg_opus(
    pipeline: &gst::Pipeline,
    path: &Path,
    tail: &[gst::Element],
) -> Result<(), AudioFailure> {
    let source = audio::make("filesrc")?;
    source.set_property("location", path);
    let demux = audio::make("oggdemux")?;
    let decoder = audio::make("opusdec")?;
    let broken = |what: &str, e: glib::BoolError| AudioFailure::Other(format!("{what}: {e}"));
    pipeline
        .add_many([&source, &demux, &decoder])
        .and_then(|()| pipeline.add_many(tail))
        .map_err(|e| broken("could not add the note's elements", e))?;
    let mut chain = vec![decoder.clone()];
    chain.extend_from_slice(tail);
    source
        .link(&demux)
        .and_then(|()| gst::Element::link_many(&chain))
        .map_err(|e| broken("could not link the note's elements", e))?;
    let input = decoder.static_pad("sink").expect("opusdec has a sink pad");
    let input = input.downgrade();
    demux.connect_pad_added(move |_, stream| {
        if let Some(input) = input.upgrade() {
            link_stream(stream, &input);
        }
    });
    Ok(())
}

/// Links the note's first stream to the decoder. A stream that is no Opus does not link, and
/// oggdemux then stops the pipeline with an error, which is the fallback's signal.
fn link_stream(stream: &gst::Pad, input: &gst::Pad) {
    if input.is_linked() {
        glib::g_debug!("fermix", "a voice note's second stream is not played");
        return;
    }
    if let Err(e) = stream.link(input) {
        let kind = stream
            .current_caps()
            .and_then(|caps| caps.structure(0).map(|first| first.name().to_string()));
        glib::g_debug!("fermix", "a voice note's stream {kind:?} is no Opus: {e:?}");
    }
}

/// The appsink: mono float at the levels' rate, its samples added as they come.
fn sink(power: Arc<Mutex<Power>>) -> gst_app::AppSink {
    let caps = gst_audio::AudioCapsBuilder::new_interleaved()
        .format(gst_audio::AudioFormat::F32le)
        .rate(waveform::RATE as i32)
        .channels(1)
        .build();
    let sink = gst_app::AppSink::builder().caps(&caps).sync(false).build();
    let callbacks = gst_app::AppSinkCallbacks::builder()
        .new_sample(move |sink| add_sample(sink, &power))
        .build();
    sink.set_callbacks(callbacks);
    sink
}

/// On the streaming thread: one buffer's samples into the power, and the end of the pass past
/// `MAX_FRAMES`.
fn add_sample(
    sink: &gst_app::AppSink,
    power: &Mutex<Power>,
) -> Result<gst::FlowSuccess, gst::FlowError> {
    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
    let (chunks, rest) = map.as_slice().as_chunks::<4>();
    if !rest.is_empty() {
        glib::g_warning!("fermix", "a voice note's buffer is not whole floats");
        return Err(gst::FlowError::Error);
    }
    let samples: Vec<f32> = chunks
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    let mut power = power.lock().expect("no callback panics holding the power");
    power.add(&samples);
    if power.frames() > waveform::MAX_FRAMES {
        return Err(gst::FlowError::Eos);
    }
    Ok(gst::FlowSuccess::Ok)
}

/// The bus watch: the end of the stream, or the first error, settles the pass.
fn watch(
    pipeline: &gst::Pipeline,
    power: Arc<Mutex<Power>>,
    done: impl FnOnce(Result<Levels, Unheard>) + 'static,
) -> Result<gst::bus::BusWatchGuard, Unheard> {
    let mut done = Some(done);
    let bus = pipeline.bus().expect("a pipeline has a bus");
    bus.add_watch_local(move |_, message| {
        let failure = match message.view() {
            gst::MessageView::Eos(_) => None,
            gst::MessageView::Error(error) => Some(words(error)),
            _ => return glib::ControlFlow::Continue,
        };
        let heard = waveform::heard(&power.lock().expect("unpoisoned"), failure);
        if let Some(done) = done.take() {
            done(heard);
        }
        glib::ControlFlow::Break
    })
    .map_err(|e| Unheard::Undecodable(format!("could not watch the bus: {e}")))
}

/// GStreamer's words for an error, with the element that raised it; its details go to the debug
/// log. A note this app cannot decode ends its pass this way by design, so the caller decides
/// whether it is a fault.
pub(crate) fn words(error: &gst::message::Error) -> String {
    let source = error
        .src()
        .map(|s| s.name().to_string())
        .unwrap_or_default();
    glib::g_debug!(
        "fermix",
        "a voice note: {source}: {} ({:?})",
        error.error(),
        error.debug()
    );
    format!("{source}: {}", error.error())
}

/// The error a failed state change left on the bus, if any.
pub(crate) fn first_error(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus().expect("a pipeline has a bus");
    let message = bus.pop_filtered(&[gst::MessageType::Error])?;
    match message.view() {
        gst::MessageView::Error(error) => Some(words(error)),
        other => unreachable!("the error filter let through {other:?}"),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const PATIENCE: Duration = Duration::from_secs(5);

    pub(crate) fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/voice")
            .join(name)
    }

    /// Runs `test` with its own main context as this thread's default.
    pub(crate) fn on_own_loop(test: impl FnOnce(&glib::MainContext)) {
        gst::init().expect("GStreamer initialises");
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| test(&context))
            .expect("a new context is free to acquire");
    }

    pub(crate) fn run_until(
        context: &glib::MainContext,
        limit: Duration,
        done: impl Fn() -> bool,
    ) -> bool {
        let deadline = Instant::now() + limit;
        while !done() && Instant::now() < deadline {
            context.iteration(false);
            std::thread::sleep(Duration::from_millis(5));
        }
        done()
    }

    type Heard = Rc<RefCell<Option<Result<Levels, Unheard>>>>;

    fn read(context: &glib::MainContext, path: &Path) -> (Result<Pass, Unheard>, Heard) {
        let seen: Heard = Rc::default();
        let into = seen.clone();
        let pass = start(path, move |heard| *into.borrow_mut() = Some(heard));
        if pass.is_ok() {
            assert!(run_until(context, PATIENCE, || seen.borrow().is_some()));
        }
        (pass, seen)
    }

    #[test]
    fn an_ogg_opus_note_reads_its_levels_and_its_decoded_length() {
        on_own_loop(|context| {
            let (pass, seen) = read(context, &fixture("speech-opus.ogg"));
            assert!(pass.is_ok());
            let levels = seen.take().expect("done ran").expect("levels");
            assert!(
                (levels.seconds() - 1.504).abs() < 0.01,
                "{}",
                levels.seconds()
            );
            let values = levels.values();
            assert_eq!(values.len(), waveform::BUCKETS);
            assert_eq!(values.iter().copied().fold(0.0, f32::max), 1.0);
            let room = values[..8].iter().copied().fold(0.0, f32::max);
            assert!(
                room < 0.5,
                "the room tone before the words reads low: {room}"
            );
        });
    }

    #[test]
    fn an_ogg_vorbis_note_is_undecodable_and_falls_back() {
        on_own_loop(|context| {
            let (pass, seen) = read(context, &fixture("speech-vorbis.ogg"));
            assert!(
                pass.is_ok(),
                "the failure comes on the bus, not at the start"
            );
            let unheard = seen.take().expect("done ran").expect_err("no levels");
            assert!(unheard.falls_back(), "{unheard:?}");
        });
    }

    #[test]
    fn a_file_that_is_no_ogg_is_undecodable() {
        on_own_loop(|context| {
            let (pass, seen) = read(context, &fixture("SOURCE.md"));
            assert!(pass.is_ok());
            let unheard = seen.take().expect("done ran").expect_err("no levels");
            assert!(unheard.falls_back(), "{unheard:?}");
        });
    }

    #[test]
    fn a_file_that_is_gone_fails_at_the_start() {
        on_own_loop(|context| {
            let (pass, seen) = read(context, &fixture("gone.ogg"));
            let Err(unheard) = pass else {
                panic!("a missing file cannot start");
            };
            assert!(unheard.falls_back(), "{unheard:?}");
            assert!(seen.borrow().is_none());
        });
    }

    #[test]
    fn a_pass_dropped_before_its_end_never_reports() {
        on_own_loop(|context| {
            let seen: Heard = Rc::default();
            let into = seen.clone();
            let pass = start(&fixture("speech-opus.ogg"), move |heard| {
                *into.borrow_mut() = Some(heard)
            })
            .expect("starts");
            drop(pass);
            assert!(!run_until(context, Duration::from_millis(300), || seen
                .borrow()
                .is_some()));
        });
    }

    type Seen = Rc<RefCell<Vec<(&'static str, Option<Turn>)>>>;

    /// A pass that waits for its turn and keeps it until the test drops it.
    fn wait(turns: &Rc<Turns>, name: &'static str, seen: &Seen) -> glib::JoinHandle<()> {
        let (turns, seen) = (turns.clone(), seen.clone());
        glib::spawn_future_local(async move {
            let turn = turns.wait().await;
            seen.borrow_mut().push((name, turn));
        })
    }

    fn names(seen: &Seen) -> Vec<&'static str> {
        seen.borrow().iter().map(|(name, _)| *name).collect()
    }

    #[test]
    fn two_passes_run_at_once_and_the_next_waits_for_a_turn() {
        on_own_loop(|context| {
            let turns = Rc::new(Turns::default());
            let seen: Seen = Rc::default();
            let _tasks: Vec<_> = ["first", "second", "third"]
                .into_iter()
                .map(|name| wait(&turns, name, &seen))
                .collect();
            run_until(context, Duration::from_millis(200), || false);
            assert_eq!(names(&seen), ["first", "second"]);
            seen.borrow_mut().remove(0);
            assert!(run_until(context, PATIENCE, || seen.borrow().len() == 2));
            assert_eq!(names(&seen), ["second", "third"]);
            assert!(seen.borrow().iter().all(|(_, turn)| turn.is_some()));
        });
    }

    /// A pass whose chip went while it waited gives its turn on: the one after it runs.
    #[test]
    fn a_pass_that_left_while_waiting_gives_its_turn_to_the_next() {
        on_own_loop(|context| {
            let turns = Rc::new(Turns::default());
            let seen: Seen = Rc::default();
            let _running = [wait(&turns, "first", &seen), wait(&turns, "second", &seen)];
            let gone = wait(&turns, "gone", &seen);
            let _last = wait(&turns, "last", &seen);
            run_until(context, Duration::from_millis(200), || false);
            // As a chip's task goes: aborted, and its future dropped with the handle.
            gone.abort();
            drop(gone);
            seen.borrow_mut().remove(0);
            assert!(run_until(context, PATIENCE, || seen.borrow().len() == 2));
            assert_eq!(names(&seen), ["second", "last"]);
        });
    }
}
