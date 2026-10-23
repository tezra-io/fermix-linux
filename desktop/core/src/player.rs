//! The chat page's one player of voice notes, as state, ported from the macOS app (`AppPlayer`,
//! `ChatSoundState`): one note at a time, so its state is the one truth of what plays. A press on
//! a note asks for its file; the grant plays it in place of whatever played, unless another press
//! or a stop overtook it. A note is let go at its end, when its chip goes, when the page is left,
//! and when a call comes up, while which nothing plays. Letting go of a note lets go of its file
//! (a file is held only while it is read); one that ended still shows what the Mac's does, its
//! start and its whole length, and plays again from a fresh press. No GStreamer: the app keeps a
//! pipeline for exactly the note held here.

use std::collections::HashMap;

/// How far one press of an arrow key moves along a note, in seconds.
pub const STEP: f64 = 5.0;
/// How often the held note's place is read while it plays.
pub const TICK_MS: u64 = 250;

/// The note the player holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub r#ref: String,
    pub playing: bool,
    /// Where it is, in seconds.
    pub position: f64,
}

/// The press whose grant is awaited.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asking {
    press: u64,
    r#ref: String,
}

/// What a press of a note's play button asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressed {
    /// A call is up: nothing plays.
    Nothing,
    Pause,
    Resume,
    /// Ask for the note's file, then call `granted` with this press once it comes.
    Ask(u64),
}

#[derive(Debug, Default)]
pub struct Player {
    held: Option<Held>,
    asking: Option<Asking>,
    presses: u64,
    call_up: bool,
    /// Each note's length as a pipeline read it, kept after the note is let go, until its levels'
    /// decoded one is known. One entry per note played on the page.
    lengths: HashMap<String, f64>,
}

impl Player {
    pub fn held(&self) -> Option<&Held> {
        self.held.as_ref()
    }

    pub fn holds(&self, r#ref: &str) -> bool {
        self.held.as_ref().is_some_and(|held| held.r#ref == r#ref)
    }

    pub fn call_up(&self) -> bool {
        self.call_up
    }

    /// A press of `r#ref`'s play button: the held note pauses or resumes, any other is asked for.
    pub fn press(&mut self, r#ref: &str) -> Pressed {
        if self.call_up {
            return Pressed::Nothing;
        }
        if let Some(held) = self.held.as_mut().filter(|held| held.r#ref == r#ref) {
            held.playing = !held.playing;
            return if held.playing {
                Pressed::Resume
            } else {
                Pressed::Pause
            };
        }
        self.presses += 1;
        self.asking = Some(Asking {
            press: self.presses,
            r#ref: r#ref.to_owned(),
        });
        Pressed::Ask(self.presses)
    }

    /// `press`'s file came. True when it plays, from its start, in place of whatever played; false
    /// when another press, a stop or a call overtook it, and its file is let go.
    pub fn granted(&mut self, press: u64) -> bool {
        let Some(asking) = self.asking.take_if(|asking| asking.press == press) else {
            return false;
        };
        self.held = Some(Held {
            r#ref: asking.r#ref,
            playing: true,
            position: 0.0,
        });
        true
    }

    /// Lets go of `r#ref`, held or asked for: it played to its end, could not play on, or its
    /// chip went. Nothing else is touched.
    pub fn let_go(&mut self, r#ref: &str) {
        if self.holds(r#ref) {
            self.held = None;
        }
        if self
            .asking
            .as_ref()
            .is_some_and(|asking| asking.r#ref == r#ref)
        {
            self.asking = None;
        }
    }

    /// Lets go of everything, held and asked for: the page was left.
    pub fn stop(&mut self) {
        self.held = None;
        self.asking = None;
    }

    /// A call came up, or ended. Coming up, it stops the player.
    pub fn call(&mut self, up: bool) {
        self.call_up = up;
        if up {
            self.stop();
        }
    }

    /// The held note's place, as the pipeline read it.
    pub fn moved(&mut self, seconds: f64) {
        if let Some(held) = self.held.as_mut() {
            held.position = seconds.max(0.0);
        }
    }

    /// The held note's length, as the pipeline read it. No length is ignored.
    pub fn length_read(&mut self, seconds: f64) {
        if !(seconds.is_finite() && seconds > 0.0) {
            return;
        }
        if let Some(held) = self.held.as_ref() {
            self.lengths.insert(held.r#ref.clone(), seconds);
        }
    }

    /// Moves the held `r#ref` to `seconds`, no further than its end where its length is known
    /// (`decoded` as for `shown`), returning where it lands. A note not held has no file to move
    /// along until it is played.
    pub fn seek(&mut self, r#ref: &str, seconds: f64, decoded: Option<f64>) -> Option<f64> {
        if self.call_up || !self.holds(r#ref) {
            return None;
        }
        let end = self.shown(r#ref, decoded).length;
        self.moved(seconds.min(end.unwrap_or(f64::INFINITY)));
        self.held.as_ref().map(|held| held.position)
    }

    /// What `r#ref`'s chip shows, given the length its levels decoded once they are read: where
    /// it is while held, and its start otherwise, a note that ended included.
    pub fn shown(&self, r#ref: &str, decoded: Option<f64>) -> Shown {
        let length = decoded.or_else(|| self.lengths.get(r#ref).copied());
        match self.held.as_ref().filter(|held| held.r#ref == r#ref) {
            Some(held) => Shown {
                held: true,
                playing: held.playing,
                position: held.position,
                length,
            },
            None => Shown {
                held: false,
                playing: false,
                position: 0.0,
                length,
            },
        }
    }
}

/// A key the waveform takes as a slider does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Home,
    End,
}

/// What a note's chip shows of the player.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    pub held: bool,
    pub playing: bool,
    pub position: f64,
    /// The decoded length once the levels are read, the pipeline's until then; none before either.
    pub length: Option<f64>,
}

impl Shown {
    /// Playing, or paused past its start.
    pub fn under_way(&self) -> bool {
        self.held && (self.playing || self.position > 0.0)
    }

    /// The time shown: where it is while under way, its length while still.
    pub fn shown_time(&self) -> Option<f64> {
        if self.under_way() {
            return Some(self.position);
        }
        self.length
    }

    /// How much of the waveform is filled, 0 to 1.
    pub fn filled(&self) -> f64 {
        match self.length {
            Some(length) if self.held && length > 0.0 => (self.position / length).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// Where `key` moves the note: Left and Right a step, held to the note; Home to its start;
    /// End to its end, nowhere until its length is known.
    pub fn keyed(&self, key: Key) -> Option<f64> {
        match key {
            Key::Left => Some(self.stepped(false)),
            Key::Right => Some(self.stepped(true)),
            Key::Home => Some(0.0),
            Key::End => self.at(1.0),
        }
    }

    /// Where an arrow key's step lands, held to the note.
    pub fn stepped(&self, forward: bool) -> f64 {
        let step = if forward { STEP } else { -STEP };
        let end = self.length.filter(|length| *length >= 0.0);
        (self.position + step).clamp(0.0, end.unwrap_or(f64::INFINITY))
    }

    /// Where a click at `fraction` of the waveform lands; nowhere until the length is known.
    pub fn at(&self, fraction: f64) -> Option<f64> {
        self.length.map(|length| length * fraction.clamp(0.0, 1.0))
    }
}

/// A time as the chip writes it, "m:ss", with the hours only where it has any. Rounded to the
/// second, half to even, as the macOS formatter.
pub fn written(seconds: f64) -> String {
    let whole = if seconds.is_finite() {
        seconds.max(0.0).round_ties_even() as u64
    } else {
        0
    };
    let (hours, minutes, seconds) = (whole / 3600, whole / 60 % 60, whole % 60);
    if hours > 0 {
        return format!("{hours}:{minutes:02}:{seconds:02}");
    }
    format!("{minutes}:{seconds:02}")
}
