//! The chat page's one player of voice notes (macOS `AppPlayer`): it plays a note from the file
//! the daemon grants when play is pressed, and holds that grant until the note stops: at its end,
//! when another note plays, when a call comes up, when the page is left, or when its chip goes.
//! Each note's chip (`voice_chip`) shows what the player stands for it; a note that cannot play
//! makes its chips fall back.

use super::entries::toast;
use super::levels::Turns;
use super::playback::{Ended, Playback};
use super::strip::NOT_READY;
use super::voice_chip::Chip;
use crate::audio::Endpoints;
use crate::media_grants::{Hold, Media};
use fermix_client::chat_wire::grants::Outcome;
use fermix_client::player::{self, Pressed, Shown, TICK_MS};
use gtk::glib;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The note playing: its pipeline, the tick that follows it, and its file's grant, let go after
/// the pipeline is closed (fields drop in order).
struct Playing {
    press: u64,
    playback: Playback,
    tick: Option<glib::SourceId>,
    _hold: Hold,
}

impl Drop for Playing {
    fn drop(&mut self) {
        if let Some(tick) = self.tick.take() {
            tick.remove();
        }
    }
}

/// The chat page's one player of voice notes. Its state is `fermix_client::player`'s; here are
/// the pipeline for the note it holds, the read of a pressed note's file, and the chips to show.
pub struct Player {
    state: RefCell<player::Player>,
    playing: RefCell<Option<Playing>>,
    asking: RefCell<Option<glib::JoinHandle<()>>>,
    pub(super) chips: RefCell<Vec<Weak<Chip>>>,
    media: Media,
    endpoints: Endpoints,
    pub(super) turns: Rc<Turns>,
}

impl Player {
    pub fn new(media: &Media, endpoints: Endpoints) -> Rc<Player> {
        Rc::new(Player {
            state: RefCell::default(),
            playing: RefCell::default(),
            asking: RefCell::default(),
            chips: RefCell::default(),
            media: media.clone(),
            endpoints,
            turns: Rc::default(),
        })
    }

    /// Leaving the page stops whatever plays, and any press still asking.
    pub fn stop(&self) {
        if let Some(asking) = self.asking.take() {
            asking.abort();
        }
        self.state.borrow_mut().stop();
        self.playing.take();
        self.refresh();
    }

    /// A call came up, or ended. While one is up nothing plays and play is not offered.
    pub fn call(&self, up: bool) {
        if self.state.borrow().call_up() == up {
            return;
        }
        if up {
            if let Some(asking) = self.asking.take() {
                asking.abort();
            }
            self.playing.take();
        }
        self.state.borrow_mut().call(up);
        self.refresh();
    }

    pub(super) fn press(self: &Rc<Self>, chip: &Rc<Chip>) {
        let pressed = self.state.borrow_mut().press(chip.r#ref());
        match pressed {
            Pressed::Nothing => {}
            Pressed::Pause => self.pause(true),
            Pressed::Resume => self.pause(false),
            Pressed::Ask(press) => self.ask(press, chip),
        }
        self.refresh();
    }

    fn pause(&self, paused: bool) {
        let playing = self.playing.borrow();
        let Some(playing) = playing.as_ref() else {
            return;
        };
        if let Err(why) = playing.playback.pause(paused) {
            glib::g_warning!("fermix", "a voice note did not pause or resume: {why}");
        }
    }

    /// Asks for the pressed note's file; the press that overtakes it, or a stop, drops the read.
    fn ask(self: &Rc<Self>, press: u64, chip: &Rc<Chip>) {
        let (player, chip) = (Rc::downgrade(self), Rc::downgrade(chip));
        let media = self.media.clone();
        let task = glib::spawn_future_local(async move {
            let Some(r#ref) = chip.upgrade().map(|chip| chip.r#ref().to_owned()) else {
                return;
            };
            let (hold, outcome) = media.read(&r#ref).await;
            if let (Some(player), Some(chip)) = (player.upgrade(), chip.upgrade()) {
                player.answered(press, &chip, hold, outcome);
            }
        });
        if let Some(earlier) = self.asking.replace(Some(task)) {
            earlier.abort();
        }
    }

    fn answered(self: &Rc<Self>, press: u64, chip: &Rc<Chip>, hold: Hold, outcome: Outcome) {
        let r#ref = chip.r#ref();
        let path = match outcome {
            Outcome::Granted(path) => path,
            Outcome::Missing => {
                self.state.borrow_mut().let_go(r#ref);
                chip.gone();
                return self.refresh();
            }
            Outcome::Failed(why) => {
                glib::g_warning!("fermix", "voice note {ref} did not play: {why}");
                self.state.borrow_mut().let_go(r#ref);
                toast(&chip.widget, NOT_READY);
                return self.refresh();
            }
        };
        if self.state.borrow_mut().granted(press) {
            self.play(press, chip, path, hold);
        }
        self.refresh();
    }

    /// Plays the granted file in place of whatever played, whose file is let go.
    fn play(self: &Rc<Self>, press: u64, chip: &Rc<Chip>, path: PathBuf, hold: Hold) {
        self.playing.take();
        match Playback::start(self.endpoints, &path, self.on_end(press)) {
            Ok(playback) => {
                *self.playing.borrow_mut() = Some(Playing {
                    press,
                    playback,
                    tick: Some(self.tick()),
                    _hold: hold,
                });
            }
            Err(why) => {
                glib::g_warning!("fermix", "voice note {} did not start: {why}", chip.r#ref());
                self.state.borrow_mut().let_go(chip.r#ref());
                chip.fall_back(&why);
            }
        }
    }

    /// What the pipeline's bus says when the note stops by itself, handled out of the watch.
    fn on_end(self: &Rc<Self>, press: u64) -> impl FnOnce(Ended) + 'static {
        let player = Rc::downgrade(self);
        move |how| {
            glib::idle_add_local_once(move || {
                if let Some(player) = player.upgrade() {
                    player.ended(press, how);
                }
            });
        }
    }

    /// The note played to its end, back at its start and let go with its file, or could not play
    /// on and is its file tile from now.
    fn ended(&self, press: u64, how: Ended) {
        let playing = self
            .playing
            .borrow_mut()
            .take_if(|playing| playing.press == press);
        let Some(playing) = playing else {
            return;
        };
        drop(playing);
        let r#ref = self.state.borrow().held().map(|held| held.r#ref.clone());
        let Some(r#ref) = r#ref else {
            return;
        };
        self.state.borrow_mut().let_go(&r#ref);
        if let Ended::Failed(why) = &how {
            for chip in self.live_chips() {
                if chip.r#ref() == r#ref {
                    chip.fall_back(why);
                }
            }
        }
        self.refresh();
    }

    fn tick(self: &Rc<Self>) -> glib::SourceId {
        let player = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(TICK_MS), move || {
            let Some(player) = player.upgrade() else {
                return glib::ControlFlow::Break;
            };
            player.moved();
            glib::ControlFlow::Continue
        })
    }

    /// Where the note is, and its length once the pipeline knows it.
    fn moved(&self) {
        let (position, length) = match self.playing.borrow().as_ref() {
            Some(playing) => (playing.playback.position(), playing.playback.length()),
            None => return,
        };
        let mut state = self.state.borrow_mut();
        if let Some(position) = position {
            state.moved(position);
        }
        if let Some(length) = length {
            state.length_read(length);
        }
        drop(state);
        self.refresh();
    }

    pub(super) fn seek(&self, r#ref: &str, seconds: f64, decoded: Option<f64>) {
        let Some(at) = self.state.borrow_mut().seek(r#ref, seconds, decoded) else {
            return;
        };
        if let Some(playing) = self.playing.borrow().as_ref() {
            if let Err(why) = playing.playback.seek(at) {
                glib::g_warning!("fermix", "a voice note did not move: {why}");
            }
        }
        self.refresh();
    }

    pub(super) fn shown(&self, r#ref: &str, decoded: Option<f64>) -> Shown {
        self.state.borrow().shown(r#ref, decoded)
    }

    pub(super) fn call_up(&self) -> bool {
        self.state.borrow().call_up()
    }

    fn live_chips(&self) -> Vec<Rc<Chip>> {
        let mut chips = self.chips.borrow_mut();
        chips.retain(|chip| chip.strong_count() > 0);
        chips.iter().filter_map(Weak::upgrade).collect()
    }

    fn refresh(&self) {
        for chip in self.live_chips() {
            chip.show(self);
        }
    }

    /// A chip went: its note stops, unless another chip on the page shows it.
    pub(super) fn chip_gone(&self, r#ref: &str) {
        if self.live_chips().iter().any(|chip| chip.r#ref() == r#ref) {
            return;
        }
        let held = self.state.borrow().holds(r#ref);
        self.state.borrow_mut().let_go(r#ref);
        if held {
            self.playing.take();
        }
    }
}
