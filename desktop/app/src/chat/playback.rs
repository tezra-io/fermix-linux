//! A voice note playing: `filesrc ! oggdemux ! opusdec ! audioconvert ! audioresample !` the
//! speakers (pulsesink, or a clock-synced fakesink under `FERMIX_AUDIO=test`). Dropping it stops
//! the sound, the watch first, so `ended` never runs after. An end of stream counts only if it
//! carries the last seek's number: one posted before a seek restarted the note is stale.

use super::levels::{first_error, ogg_opus, why, words};
use crate::audio::{self, AudioFailure, Endpoints};
use gst::prelude::*;
use gtk::glib;
use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

const CLIENT_NAME: &str = "Fermix";

/// How a note stopped by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Ended {
    /// It played to its end.
    End,
    /// It could not play on, in GStreamer's words: the note falls back to its file tile.
    Failed(String),
}

pub(crate) struct Playback {
    pipeline: gst::Pipeline,
    /// The number of the last seek the pipeline took; its end of stream carries it.
    seek: Rc<Cell<Option<gst::Seqnum>>>,
    watch: Option<gst::bus::BusWatchGuard>,
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.watch.take();
        audio::close(&self.pipeline);
    }
}

impl Playback {
    /// Plays `path` from its start. Either it fails here, or `ended` runs once, on this thread's
    /// main loop, when the note stops by itself, unless it is dropped first.
    pub(crate) fn start(
        endpoints: Endpoints,
        path: &Path,
        ended: impl FnOnce(Ended) + 'static,
    ) -> Result<Playback, String> {
        gst::init().map_err(|e| format!("GStreamer did not start: {e}"))?;
        let pipeline = gst::Pipeline::new();
        let tail = [
            audio::make("audioconvert").map_err(why)?,
            audio::make("audioresample").map_err(why)?,
            speaker(endpoints).map_err(why)?,
        ];
        ogg_opus(&pipeline, path, &tail).map_err(why)?;
        let mut playback = Playback {
            pipeline,
            seek: Rc::default(),
            watch: None,
        };
        if let Err(e) = playback.pipeline.set_state(gst::State::Playing) {
            return Err(first_error(&playback.pipeline).unwrap_or_else(|| e.to_string()));
        }
        playback.watch = Some(watch(&playback.pipeline, playback.seek.clone(), ended)?);
        Ok(playback)
    }

    pub(crate) fn pause(&self, paused: bool) -> Result<(), String> {
        let state = if paused {
            gst::State::Paused
        } else {
            gst::State::Playing
        };
        self.pipeline
            .set_state(state)
            .map(|_| ())
            .map_err(|e| first_error(&self.pipeline).unwrap_or_else(|| e.to_string()))
    }

    /// Where it is, in seconds, once the pipeline knows.
    pub(crate) fn position(&self) -> Option<f64> {
        let position = self.pipeline.query_position::<gst::ClockTime>()?;
        Some(position.nseconds() as f64 / 1e9)
    }

    /// Its length as the container says, until the levels' decoded one is known.
    pub(crate) fn length(&self) -> Option<f64> {
        let length = self.pipeline.query_duration::<gst::ClockTime>()?;
        Some(length.nseconds() as f64 / 1e9)
    }

    /// Moves the note to `seconds`. Its number is kept only once the pipeline took the seek: a
    /// refused one (during the first preroll) leaves the end of stream to come the note's own.
    pub(crate) fn seek(&self, seconds: f64) -> Result<(), String> {
        let target = gst::ClockTime::from_nseconds((seconds.max(0.0) * 1e9) as u64);
        let event = gst::event::Seek::new(
            1.0,
            gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
            gst::SeekType::Set,
            target,
            gst::SeekType::None,
            gst::ClockTime::NONE,
        );
        let number = event.seqnum();
        if !self.pipeline.send_event(event) {
            return Err(format!("the pipeline did not seek to {seconds} s"));
        }
        self.seek.set(Some(number));
        Ok(())
    }
}

fn speaker(endpoints: Endpoints) -> Result<gst::Element, AudioFailure> {
    let speaker = match endpoints {
        Endpoints::Sound => audio::make("pulsesink")?,
        Endpoints::Test => audio::make("fakesink")?,
    };
    match endpoints {
        Endpoints::Sound => speaker.set_property("client-name", CLIENT_NAME),
        Endpoints::Test => speaker.set_property("sync", true),
    }
    Ok(speaker)
}

fn watch(
    pipeline: &gst::Pipeline,
    seek: Rc<Cell<Option<gst::Seqnum>>>,
    ended: impl FnOnce(Ended) + 'static,
) -> Result<gst::bus::BusWatchGuard, String> {
    let mut ended = Some(ended);
    let bus = pipeline.bus().expect("a pipeline has a bus");
    bus.add_watch_local(move |_, message| {
        let how = match message.view() {
            gst::MessageView::Eos(_) if stale(message, seek.get()) => {
                glib::g_debug!(
                    "fermix",
                    "a voice note's end from before its seek is passed over"
                );
                return glib::ControlFlow::Continue;
            }
            gst::MessageView::Eos(_) => Ended::End,
            gst::MessageView::Error(error) => {
                let why = words(error);
                glib::g_warning!("fermix", "a voice note stopped playing: {why}");
                Ended::Failed(why)
            }
            _ => return glib::ControlFlow::Continue,
        };
        if let Some(ended) = ended.take() {
            ended(how);
        }
        glib::ControlFlow::Break
    })
    .map_err(|e| format!("could not watch the bus: {e}"))
}

/// Whether `message` was posted before the last seek, by what played before it. Before any seek
/// every end counts.
fn stale(message: &gst::Message, seek: Option<gst::Seqnum>) -> bool {
    seek.is_some_and(|seek| message.seqnum() != seek)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::levels::tests::{fixture, on_own_loop, run_until};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    type Seen = Rc<RefCell<Option<Ended>>>;

    fn play(name: &str) -> (Playback, Seen) {
        let seen: Seen = Rc::default();
        let into = seen.clone();
        let playback = Playback::start(Endpoints::Test, &fixture(name), move |how| {
            *into.borrow_mut() = Some(how)
        })
        .expect("starts");
        (playback, seen)
    }

    #[test]
    fn an_ogg_opus_note_plays_in_real_time_to_its_end() {
        on_own_loop(|context| {
            let (playback, seen) = play("speech-opus.ogg");
            assert!(run_until(context, Duration::from_secs(2), || playback
                .position()
                .is_some_and(|at| at > 0.5)));
            assert!(seen.borrow().is_none(), "half a second in, it still plays");
            let length = playback.length().expect("a length");
            assert!((length - 1.504).abs() < 0.01, "{length}");
            assert!(run_until(context, Duration::from_secs(3), || seen
                .borrow()
                .is_some()));
            assert_eq!(seen.take(), Some(Ended::End));
        });
    }

    #[test]
    fn a_paused_note_stays_where_it_is_and_a_seek_moves_it() {
        on_own_loop(|context| {
            let (playback, seen) = play("speech-opus.ogg");
            assert!(run_until(context, Duration::from_secs(2), || playback
                .position()
                .is_some_and(|at| at > 0.2)));
            playback.pause(true).expect("pauses");
            let paused_at = playback.position().expect("a place");
            run_until(context, Duration::from_millis(400), || false);
            let still = playback.position().expect("a place");
            assert!((still - paused_at).abs() < 0.05, "{paused_at} then {still}");

            playback.seek(1.0).expect("seeks");
            run_until(context, Duration::from_millis(200), || false);
            let moved = playback.position().expect("a place");
            assert!((moved - 1.0).abs() < 0.1, "{moved}");
            playback.pause(false).expect("resumes");
            assert!(run_until(context, Duration::from_secs(2), || seen
                .borrow()
                .is_some()));
            assert_eq!(seen.take(), Some(Ended::End));
        });
    }

    /// An end posted before a seek restarted the note belongs to what played before it: the note
    /// plays on from where it was moved to, and only its own end counts.
    #[test]
    fn a_seek_after_the_end_ignores_the_end_already_posted() {
        on_own_loop(|context| {
            let (playback, seen) = play("speech-opus.ogg");
            // The loop does not turn, so the note's end waits on the bus.
            std::thread::sleep(Duration::from_millis(1900));
            playback.seek(0.0).expect("seeks");
            run_until(context, Duration::from_millis(700), || {
                seen.borrow().is_some()
            });
            assert_eq!(
                *seen.borrow(),
                None,
                "the end before the seek is not this note's"
            );
            assert!(run_until(context, Duration::from_millis(2500), || seen
                .borrow()
                .is_some()));
            assert_eq!(seen.take(), Some(Ended::End));
        });
    }

    #[test]
    fn an_ogg_vorbis_note_fails_on_the_bus() {
        on_own_loop(|context| {
            let (_playback, seen) = play("speech-vorbis.ogg");
            assert!(run_until(context, Duration::from_secs(2), || seen
                .borrow()
                .is_some()));
            let Some(Ended::Failed(why)) = seen.take() else {
                panic!("a Vorbis note cannot play");
            };
            assert!(why.starts_with("oggdemux"), "{why}");
        });
    }

    #[test]
    fn a_dropped_note_never_reports() {
        on_own_loop(|context| {
            let (playback, seen) = play("speech-opus.ogg");
            assert!(run_until(context, Duration::from_secs(2), || playback
                .position()
                .is_some_and(|at| at > 0.1)));
            drop(playback);
            assert!(!run_until(context, Duration::from_secs(2), || seen
                .borrow()
                .is_some()));
        });
    }
}
