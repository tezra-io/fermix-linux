//! A phone's voice note in its row (macOS `ChatVoiceNote`): a chip with play and pause, the note's
//! waveform, which fills as it plays and which a click or a slider's keys move along, and the
//! time, its length while still and where it is while under way. The waveform's levels are read
//! once the chip is on screen (`levels`), from a file the daemon grants for that read alone; the
//! page's one `Player` (`voice_note`) plays the note. A note this app cannot decode, an Ogg with
//! no Opus in it, loses its chip, and its row shows it as a file tile in its lane (the row's
//! `when_unplayable`); one whose bytes are gone says Not available.

use super::levels::{self, Turns};
use super::strip::{Readers, NOT_AVAILABLE};
use super::voice_note::Player;
use crate::media_grants::Media;
use fermix_client::chat_wire::grants::Outcome;
use fermix_client::chat_wire::protocol::MediaRef;
use fermix_client::player::{written, Key, Shown};
use fermix_client::waveform::{self, Levels, Unheard};
use futures_channel::oneshot;
use gtk::prelude::*;
use gtk::{cairo, gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A note's levels are asked for at most this many times, once each time it comes on screen.
const TRIES: u8 = 2;
/// From one bar's start to the next.
const PITCH: f64 = waveform::BAR_WIDTH + waveform::BAR_GAP;
/// The bars not yet played, against the ink of those played.
const UNPLAYED_ALPHA: f64 = 0.35;
/// The ring round the waveform while keyboard focus is on it: libadwaita's focus outline, 2 px
/// at half the ink, drawn here since no stock style draws one round a drawing area.
const FOCUS_WIDTH: f64 = 2.0;
const FOCUS_ALPHA: f64 = 0.5;
const FOCUS_RADIUS: f64 = 6.0;
/// The room kept round the bars for the ring and a gap inside it, as libadwaita sets its outline
/// off what it rings.
const FOCUS_INSET: i32 = 4;
/// The bars' own room.
const BARS_HEIGHT: i32 = 28;
const BARS_WIDTH: i32 = 180;

/// What reading a note's levels came to.
enum Read {
    Heard(Result<Levels, Unheard>),
    /// The daemon has no bytes for it.
    Gone,
    /// The read did not happen: no grant, or no turn. It may be tried again.
    Failed(String),
}

/// One voice note's chip: the player line, or Not available.
pub struct Chip {
    note: MediaRef,
    readers: Readers,
    pub widget: gtk::Stack,
    button: gtk::Button,
    waveform: gtk::DrawingArea,
    time: gtk::Label,
    levels: RefCell<Option<Levels>>,
    filled: Cell<f64>,
    tries: Cell<u8>,
    reading: Cell<bool>,
    settled: Cell<bool>,
    /// False once the note could not play; its row then shows it as a file tile.
    plays: Cell<bool>,
    /// What the row does then, given by the row once it is drawn.
    unplayable: RefCell<Option<Box<dyn FnOnce()>>>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
}

impl Chip {
    pub fn new(note: &MediaRef, readers: &Readers) -> Rc<Chip> {
        let (line, button, waveform, time) = player_line();
        let widget = gtk::Stack::builder()
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        widget.add_named(&line, Some("player"));
        widget.add_named(&not_available(), Some("gone"));
        widget.update_property(&[gtk::accessible::Property::Label("Voice note")]);
        let chip = Rc::new(Chip {
            note: note.clone(),
            readers: readers.clone(),
            widget,
            button,
            waveform,
            time,
            levels: RefCell::default(),
            filled: Cell::new(0.0),
            tries: Cell::new(0),
            reading: Cell::new(false),
            settled: Cell::new(false),
            plays: Cell::new(true),
            unplayable: RefCell::default(),
            task: RefCell::default(),
        });
        chip.connect();
        readers.player.chips.borrow_mut().push(Rc::downgrade(&chip));
        chip.show(&readers.player);
        chip
    }

    fn connect(self: &Rc<Self>) {
        let chip = Rc::downgrade(self);
        self.button.connect_clicked(move |_| {
            if let Some(chip) = chip.upgrade() {
                chip.readers.player.press(&chip);
            }
        });
        let chip = Rc::downgrade(self);
        self.waveform.set_draw_func(move |_, cr, width, height| {
            if let Some(chip) = chip.upgrade() {
                chip.draw(cr, width, height);
            }
        });
        // Focus coming or going, or turning visible with the keyboard, redraws the ring.
        self.waveform
            .connect_state_flags_changed(|waveform, _| waveform.queue_draw());
        let click = gtk::GestureClick::new();
        let chip = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _, x, _| {
            let Some(chip) = chip.upgrade() else {
                return;
            };
            let (inset, width) = (f64::from(FOCUS_INSET), f64::from(chip.waveform.width()));
            let bars = width - 2.0 * inset;
            if bars > 0.0 {
                chip.seek_to((x - inset) / bars);
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        self.waveform.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let chip = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| match chip.upgrade() {
            Some(chip) => chip.key(key),
            None => glib::Propagation::Proceed,
        });
        self.waveform.add_controller(keys);
    }

    pub fn r#ref(&self) -> &str {
        &self.note.r#ref
    }

    /// Whether the note can still play, as far as this app has found.
    pub fn plays(&self) -> bool {
        self.plays.get()
    }

    /// `hook` runs once, when the note turns out not to play.
    pub fn when_unplayable(&self, hook: impl FnOnce() + 'static) {
        self.unplayable.replace(Some(Box::new(hook)));
    }

    /// The chip came on screen: its levels are read, until they are.
    pub fn on_screen(self: &Rc<Self>) {
        if self.settled.get() || self.reading.get() || self.tries.get() >= TRIES {
            return;
        }
        self.tries.set(self.tries.get() + 1);
        self.reading.set(true);
        let chip = Rc::downgrade(self);
        let note = self.note.clone();
        let (media, turns) = (
            self.readers.media.clone(),
            self.readers.player.turns.clone(),
        );
        let task = glib::spawn_future_local(async move {
            let read = read_levels(&note, &media, &turns).await;
            if let Some(chip) = chip.upgrade() {
                chip.heard(read);
            }
        });
        if let Some(earlier) = self.task.replace(Some(task)) {
            earlier.abort();
        }
    }

    fn heard(&self, read: Read) {
        self.reading.set(false);
        match read {
            Read::Heard(Ok(levels)) => {
                *self.levels.borrow_mut() = Some(levels);
                self.settled.set(true);
                self.show(&self.readers.player);
            }
            Read::Heard(Err(unheard)) if unheard.falls_back() => {
                self.settled.set(true);
                self.fall_back(&unheard.to_string());
            }
            Read::Heard(Err(unheard)) => {
                glib::g_debug!("fermix", "voice note {} rests: {unheard}", self.note.r#ref);
                self.settled.set(true);
            }
            Read::Gone => {
                self.settled.set(true);
                self.gone();
            }
            Read::Failed(why) => {
                glib::g_debug!("fermix", "no levels for {} yet: {why}", self.note.r#ref);
            }
        }
    }

    /// The player line as the one player stands for this note.
    pub(super) fn show(&self, player: &Player) {
        let shown = player.shown(&self.note.r#ref, self.decoded());
        let (icon, words) = if shown.playing {
            ("media-playback-pause-symbolic", "Pause")
        } else {
            ("media-playback-start-symbolic", "Play")
        };
        self.button.set_icon_name(icon);
        self.button.set_tooltip_text(Some(words));
        self.button
            .update_property(&[gtk::accessible::Property::Label(words)]);
        self.button.set_sensitive(!player.call_up());
        let time = shown.shown_time().map(written).unwrap_or_default();
        self.time.set_label(&time);
        self.waveform.update_property(&[
            gtk::accessible::Property::ValueMax(shown.length.unwrap_or(0.0)),
            gtk::accessible::Property::ValueNow(shown.position),
            gtk::accessible::Property::ValueText(&time),
        ]);
        self.filled.set(shown.filled());
        self.waveform.queue_draw();
    }

    /// Thin rounded bars as tall as the note is loud, one every pitch from the left on whole
    /// pixels, centred on the middle line: those played in the label's ink, the rest faint.
    /// Before the levels are read the same bars rest low and even, so nothing moves when they come.
    /// The bars in the label's ink, the played ones whole, and the focus ring while keyboard
    /// focus is on the waveform (a mouse click shows none).
    fn draw(&self, cr: &cairo::Context, width: i32, height: i32) {
        let (width, height) = (f64::from(width), f64::from(height));
        let ink = self.waveform.color();
        let inset = f64::from(FOCUS_INSET);
        cr.translate(inset, inset);
        let mut drawn = self.draw_bars(cr, &ink, width - 2.0 * inset, height - 2.0 * inset);
        cr.translate(-inset, -inset);
        if drawn.is_ok() && self.waveform.has_visible_focus() {
            drawn = focus_ring(cr, &ink, width, height);
        }
        if let Err(e) = drawn {
            glib::g_warning!("fermix", "a voice note's waveform did not draw: {e}");
        }
    }

    fn draw_bars(
        &self,
        cr: &cairo::Context,
        ink: &gdk::RGBA,
        width: f64,
        height: f64,
    ) -> Result<(), cairo::Error> {
        let count = waveform::bars_fitting(width);
        let levels = waveform::bar_levels(self.levels.borrow().as_ref(), count);
        let played = waveform::played(self.filled.get(), count);
        for (index, level) in levels.iter().enumerate() {
            let alpha = if index < played { 1.0 } else { UNPLAYED_ALPHA };
            set_ink(cr, ink, alpha);
            let bar = waveform::bar_height(*level, height);
            rounded_bar(cr, index as f64 * PITCH, (height - bar) / 2.0, bar);
            cr.fill()?;
        }
        Ok(())
    }

    fn seek_to(&self, fraction: f64) {
        let shown = self.current();
        if let Some(at) = shown.at(fraction) {
            self.readers
                .player
                .seek(&self.note.r#ref, at, self.decoded());
        }
    }

    /// A slider's keys: Left, Right, Home and End move the note where `Shown::keyed` says, and
    /// Space plays and pauses.
    fn key(self: &Rc<Self>, key: gdk::Key) -> glib::Propagation {
        let key = match key {
            gdk::Key::Left => Key::Left,
            gdk::Key::Right => Key::Right,
            gdk::Key::Home => Key::Home,
            gdk::Key::End => Key::End,
            gdk::Key::space => {
                self.readers.player.press(self);
                return glib::Propagation::Stop;
            }
            _ => return glib::Propagation::Proceed,
        };
        if let Some(target) = self.current().keyed(key) {
            self.readers
                .player
                .seek(&self.note.r#ref, target, self.decoded());
        }
        glib::Propagation::Stop
    }

    /// The note's length as its levels decoded it, once they are read.
    fn decoded(&self) -> Option<f64> {
        self.levels.borrow().as_ref().map(Levels::seconds)
    }

    fn current(&self) -> Shown {
        self.readers.player.shown(&self.note.r#ref, self.decoded())
    }

    /// The note cannot play: the chip goes, and its row shows the note as a file tile from now,
    /// what the computer's own app can open.
    pub(super) fn fall_back(&self, why: &str) {
        if !self.plays.replace(false) {
            return;
        }
        glib::g_debug!(
            "fermix",
            "voice note {} is a file tile: {why}",
            self.note.r#ref
        );
        self.widget.set_visible(false);
        let hook = self.unplayable.take();
        match hook {
            Some(hook) => hook(),
            None => glib::g_critical!("fermix", "voice note {} has no row", self.note.r#ref),
        }
    }

    /// The daemon has no bytes for the note: Not available, where the player was.
    pub(super) fn gone(&self) {
        self.widget.set_visible_child_name("gone");
    }
}

impl Drop for Chip {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.readers.player.chip_gone(&self.note.r#ref);
    }
}

/// Reads a note's levels: its turn first, then its file for the pass alone. The pass, the
/// grant and the turn are let go in that order when the future ends or is dropped.
async fn read_levels(note: &MediaRef, media: &Media, turns: &Rc<Turns>) -> Read {
    if let Err(unheard) = waveform::admit(note.size_bytes) {
        return Read::Heard(Err(unheard));
    }
    let Some(_turn) = turns.wait().await else {
        return Read::Failed("it gave way to newer notes".to_owned());
    };
    let (_hold, outcome) = media.read(&note.r#ref).await;
    let path = match outcome {
        Outcome::Granted(path) => path,
        Outcome::Missing => return Read::Gone,
        Outcome::Failed(why) => return Read::Failed(why),
    };
    let (done, heard) = oneshot::channel();
    let pass = levels::start(&path, move |result| {
        if done.send(result).is_err() {
            glib::g_debug!("fermix", "a levels pass ended after its chip went");
        }
    });
    let _pass = match pass {
        Ok(pass) => pass,
        Err(unheard) => return Read::Heard(Err(unheard)),
    };
    match heard.await {
        Ok(result) => Read::Heard(result),
        Err(oneshot::Canceled) => Read::Failed("the pass ended without a word".to_owned()),
    }
}

/// Play, the waveform and the time, on one line.
fn player_line() -> (gtk::Box, gtk::Button, gtk::DrawingArea, gtk::Label) {
    let button = gtk::Button::builder()
        .icon_name("media-playback-start-symbolic")
        .valign(gtk::Align::Center)
        .css_classes(["circular", "flat"])
        .build();
    let waveform = gtk::DrawingArea::builder()
        .content_height(BARS_HEIGHT + 2 * FOCUS_INSET)
        .width_request(BARS_WIDTH + 2 * FOCUS_INSET)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .focusable(true)
        .accessible_role(gtk::AccessibleRole::Slider)
        .build();
    waveform.update_property(&[
        gtk::accessible::Property::Label("Where the voice note is"),
        gtk::accessible::Property::Orientation(gtk::Orientation::Horizontal),
        gtk::accessible::Property::ValueMin(0.0),
    ]);
    let time = gtk::Label::builder()
        .width_chars(5)
        .xalign(1.0)
        .css_classes(["numeric", "caption"])
        .build();
    let line = gtk::Box::builder().spacing(6).build();
    line.append(&button);
    line.append(&waveform);
    line.append(&time);
    (line, button, waveform, time)
}

/// Where the player would be, for a note the daemon no longer has.
fn not_available() -> gtk::Box {
    let icon = gtk::Image::from_icon_name("audio-x-generic-symbolic");
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    let words = gtk::Label::new(Some(NOT_AVAILABLE));
    let line = gtk::Box::builder()
        .spacing(6)
        .css_classes(["dimmed"])
        .build();
    line.append(&icon);
    line.append(&words);
    line
}

/// A bar `height` tall at `x`, `y`, with round ends.
fn set_ink(cr: &cairo::Context, ink: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        f64::from(ink.red()),
        f64::from(ink.green()),
        f64::from(ink.blue()),
        f64::from(ink.alpha()) * alpha,
    );
}

/// A rounded outline just inside the waveform's edge.
fn focus_ring(
    cr: &cairo::Context,
    ink: &gdk::RGBA,
    width: f64,
    height: f64,
) -> Result<(), cairo::Error> {
    use std::f64::consts::{FRAC_PI_2, PI};
    let inset = FOCUS_WIDTH / 2.0;
    let (left, top, right, bottom) = (inset, inset, width - inset, height - inset);
    let radius = FOCUS_RADIUS.min((bottom - top) / 2.0);
    cr.new_sub_path();
    cr.arc(right - radius, top + radius, radius, -FRAC_PI_2, 0.0);
    cr.arc(right - radius, bottom - radius, radius, 0.0, FRAC_PI_2);
    cr.arc(left + radius, bottom - radius, radius, FRAC_PI_2, PI);
    cr.arc(left + radius, top + radius, radius, PI, PI + FRAC_PI_2);
    cr.close_path();
    cr.set_line_width(FOCUS_WIDTH);
    set_ink(cr, ink, FOCUS_ALPHA);
    cr.stroke()
}

fn rounded_bar(cr: &cairo::Context, x: f64, y: f64, height: f64) {
    let radius = waveform::BAR_WIDTH / 2.0;
    cr.new_sub_path();
    cr.arc(x + radius, y + radius, radius, std::f64::consts::PI, 0.0);
    cr.arc(
        x + radius,
        y + height - radius,
        radius,
        0.0,
        std::f64::consts::PI,
    );
    cr.close_path();
}
