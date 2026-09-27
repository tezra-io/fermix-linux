//! The turn's work as the page shows it: one live line while Fermix works (the
//! orb, what it is doing, for how long), and each group of tools, which is that
//! live line while the turn is on it and then folds into one line that opens
//! to a row per tool. The words come from the core's `activity` module.

use adw::prelude::*;
use fermix_client::activity::{live_meta, live_phrase, summary, tool_words};
use fermix_client::chat::{RunState, ToolRun};

/// The running turn, as its live line needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Live {
    pub stopping: bool,
    /// Seconds since the question was asked.
    pub elapsed: i64,
}

/// The orb, with a turning ring while a tool runs; what Fermix is doing; and
/// the small text after it. A screen reader hears the phrase alone.
#[derive(Clone)]
pub struct LiveLine {
    pub row: gtk::Box,
    ring: gtk::Box,
    phrase: gtk::Label,
    meta: gtk::Label,
}

impl LiveLine {
    pub fn new() -> LiveLine {
        let ring = gtk::Box::builder().css_classes(["chat-ring"]).build();
        let slot = gtk::Overlay::builder()
            .child(&ring)
            .valign(gtk::Align::Center)
            .build();
        let dot = orb();
        dot.set_halign(gtk::Align::Center);
        slot.add_overlay(&dot);
        let phrase = gtk::Label::builder()
            .css_classes(["chat-live-phrase"])
            .build();
        let meta = gtk::Label::builder()
            .css_classes(["caption", "numeric", "dim-label"])
            .visible(false)
            .build();
        let row = gtk::Box::builder()
            .spacing(10)
            .css_classes(["chat-live"])
            .accessible_role(gtk::AccessibleRole::Status)
            .build();
        row.append(&slot);
        row.append(&phrase);
        row.append(&meta);
        LiveLine {
            row,
            ring,
            phrase,
            meta,
        }
    }

    /// `turning` puts the ring round the orb, while a tool runs.
    pub fn show(&self, phrase: &str, meta: Option<&str>, turning: bool) {
        assert!(!phrase.is_empty(), "the live line always says something");
        self.phrase.set_text(phrase);
        self.meta.set_visible(meta.is_some());
        self.meta
            .set_text(&meta.map(|m| format!("· {m}")).unwrap_or_default());
        if turning {
            self.ring.add_css_class("turning");
        } else {
            self.ring.remove_css_class("turning");
        }
        self.row
            .update_property(&[gtk::accessible::Property::Label(phrase)]);
    }

    /// The line for a turn with no tools to show yet.
    pub fn thinking(&self, live: &Live) {
        let phrase = live_phrase(&[], live.stopping);
        self.show(&phrase, live_meta(0, live.elapsed).as_deref(), false);
    }
}

/// A header that is the live line while the turn is on it, and afterwards a
/// folded line with a chevron; either way it opens and closes the body under it.
struct Disclosure {
    column: gtk::Box,
    header: gtk::Button,
    face: gtk::Stack,
    live: LiveLine,
    used: gtk::Label,
    failed: gtk::Label,
    ended: gtk::Label,
}

impl Disclosure {
    fn new(body: &impl IsA<gtk::Widget>) -> Disclosure {
        let live = LiveLine::new();
        let (folded, used, failed, ended) = folded_line();
        let face = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(200)
            .hhomogeneous(false)
            .interpolate_size(true)
            .build();
        face.add_named(&live.row, Some("live"));
        face.add_named(&folded, Some("folded"));
        let header = gtk::Button::builder()
            .child(&face)
            .halign(gtk::Align::Start)
            .css_classes(["flat", "chat-disclosure"])
            .build();
        body.add_css_class("chat-disclosure-body");
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .transition_duration(200)
            .child(body)
            .build();
        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .build();
        column.append(&header);
        column.append(&revealer);
        header.update_state(&[gtk::accessible::State::Expanded(Some(false))]);
        header.connect_clicked(move |header| toggle(header, &revealer));
        Disclosure {
            column,
            header,
            face,
            live,
            used,
            failed,
            ended,
        }
    }

    fn show_live(&self, phrase: &str, meta: Option<&str>, turning: bool) {
        self.live.show(phrase, meta, turning);
        self.face.set_visible_child_name("live");
        self.label_header(phrase);
    }

    fn show_folded(&self, text: &str, failed: Option<&str>, ended: Option<&str>, label: &str) {
        self.used.set_text(text);
        show_part(&self.failed, failed);
        show_part(&self.ended, ended);
        self.face.set_visible_child_name("folded");
        self.label_header(label);
    }

    /// Whether it is open is the header's expanded state, set on each toggle.
    fn label_header(&self, what: &str) {
        self.header
            .update_property(&[gtk::accessible::Property::Label(what)]);
    }
}

/// A group of tools, opening to a row per tool.
pub struct Group {
    disclosure: Disclosure,
    list: gtk::Box,
    rows: Vec<RunRow>,
}

/// One tool's row, whose marker and word follow its state in place.
struct RunRow {
    id: String,
    marker: gtk::Box,
    word: gtk::Label,
}

impl Group {
    pub fn new(runs: &[ToolRun]) -> Group {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let mut group = Group {
            disclosure: Disclosure::new(&list),
            list,
            rows: Vec::new(),
        };
        group.grow(runs);
        group
    }

    pub fn widget(&self) -> gtk::Widget {
        self.disclosure.column.clone().upcast()
    }

    /// Adds the runs that are new and follows the state of each; false when
    /// `runs` is not this group's runs with more after them.
    pub fn grow(&mut self, runs: &[ToolRun]) -> bool {
        let same = self
            .rows
            .iter()
            .zip(runs)
            .all(|(row, run)| row.id == run.id);
        if !same || runs.len() < self.rows.len() {
            return false;
        }
        for (row, run) in self.rows.iter().zip(runs) {
            set_state(&row.marker, &row.word, run.state);
        }
        for run in &runs[self.rows.len()..] {
            let (widget, row) = run_row(run);
            self.list.append(&widget);
            self.rows.push(row);
        }
        true
    }

    /// Live while the turn is on this group; otherwise the folded line.
    pub fn set_live(&self, runs: &[ToolRun], live: Option<&Live>) {
        let Some(live) = live else {
            let folded = summary(runs);
            return self.disclosure.show_folded(
                &folded.text,
                folded.failed.as_deref(),
                folded.ended.as_deref(),
                &folded.label,
            );
        };
        let finished = runs.iter().filter(|r| r.state != RunState::Running).count();
        let phrase = live_phrase(runs, live.stopping);
        let meta = live_meta(finished, live.elapsed);
        self.disclosure
            .show_live(&phrase, meta.as_deref(), finished < runs.len());
    }
}

/// The assistant's reasoning, when an agent sends it: live while it comes in,
/// then folded to "Thought", opening to the text, small and dim.
pub struct Thought {
    disclosure: Disclosure,
    text: gtk::Label,
}

impl Thought {
    pub fn new(text: &str) -> Thought {
        let label = gtk::Label::builder()
            .label(text)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .selectable(true)
            .css_classes(["dim-label", "chat-thought-text"])
            .build();
        Thought {
            disclosure: Disclosure::new(&label),
            text: label,
        }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.disclosure.column.clone().upcast()
    }

    pub fn grow(&self, text: &str) {
        self.text.set_text(text);
    }

    pub fn set_live(&self, live: Option<&Live>) {
        match live {
            Some(live) => {
                let meta = live_meta(0, live.elapsed);
                let phrase = live_phrase(&[], live.stopping);
                self.disclosure.show_live(&phrase, meta.as_deref(), false)
            }
            None => self
                .disclosure
                .show_folded("Thought", None, None, "Thought"),
        }
    }
}

/// The chevron and the folded line's parts: what was used, what failed (in
/// the warning colour), and how it was cut short (dim).
fn folded_line() -> (gtk::Box, gtk::Label, gtk::Label, gtk::Label) {
    let chevron = gtk::Image::builder()
        .icon_name("pan-end-symbolic")
        .css_classes(["chat-chevron", "dim-label"])
        .build();
    let used = gtk::Label::builder()
        .css_classes(["chat-live-phrase", "still"])
        .build();
    let failed = gtk::Label::builder()
        .css_classes(["warning"])
        .visible(false)
        .build();
    let ended = gtk::Label::builder()
        .css_classes(["dim-label"])
        .visible(false)
        .build();
    let line = gtk::Box::builder().spacing(6).build();
    line.append(&chevron);
    line.append(&used);
    line.append(&failed);
    line.append(&ended);
    (line, used, failed, ended)
}

fn show_part(label: &gtk::Label, text: Option<&str>) {
    label.set_visible(text.is_some());
    label.set_text(&text.map(|t| format!("· {t}")).unwrap_or_default());
}

fn toggle(header: &gtk::Button, revealer: &gtk::Revealer) {
    let open = !revealer.reveals_child();
    revealer.set_reveal_child(open);
    header.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
    if open {
        header.add_css_class("open");
    } else {
        header.remove_css_class("open");
    }
}

/// The kind's icon, the tool's name and what it did, then how it went.
fn run_row(run: &ToolRun) -> (gtk::Box, RunRow) {
    let words = tool_words(&run.name, run.kind);
    let row = gtk::Box::builder()
        .spacing(8)
        .css_classes(["chat-tool-row"])
        .build();
    let icon = gtk::Image::builder()
        .icon_name(words.icon)
        .css_classes(["dim-label"])
        .build();
    row.append(&icon);
    let name = gtk::Label::builder()
        .label(&words.name)
        .css_classes(["chat-tool-name"])
        .build();
    row.append(&name);
    if let Some(detail) = &words.detail {
        let detail = gtk::Label::builder()
            .label(detail)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["dim-label"])
            .build();
        row.append(&detail);
    }
    let marker = gtk::Box::builder().valign(gtk::Align::Center).build();
    row.append(&marker);
    let word = gtk::Label::builder().css_classes(["caption"]).build();
    row.append(&word);
    set_state(&marker, &word, run.state);
    let row_parts = RunRow {
        id: run.id.clone(),
        marker,
        word,
    };
    (row, row_parts)
}

/// A spinner while it runs, a check when done; a failure, a stop and a reply
/// that ended around it also say so in a word.
fn set_state(slot: &gtk::Box, word: &gtk::Label, state: RunState) {
    // The slot holds the one marker drawn last.
    if let Some(old) = slot.first_child() {
        slot.remove(&old);
    }
    let (icon, text, class) = match state {
        RunState::Running => (None, "", "dim-label"),
        RunState::Done => (Some("object-select-symbolic"), "", "dim-label"),
        RunState::Failed => (Some("dialog-warning-symbolic"), "Failed", "warning"),
        RunState::Stopped => (Some("media-playback-stop-symbolic"), "Stopped", "dim-label"),
        RunState::Unfinished => (None, "Didn't finish", "dim-label"),
    };
    let marker: Option<gtk::Widget> = match (state, icon) {
        (RunState::Running, _) => Some(adw::Spinner::new().upcast()),
        (_, Some(icon)) => Some(gtk::Image::from_icon_name(icon).upcast()),
        (_, None) => None,
    };
    if let Some(marker) = marker {
        marker.add_css_class(class);
        slot.append(&marker);
    }
    word.set_text(text);
    word.set_visible(!text.is_empty());
    word.set_css_classes(&["caption", class]);
}

/// The breathing accent orb. The motion is CSS, which GTK holds still when
/// animations are off or less motion is asked for.
fn orb() -> gtk::Box {
    gtk::Box::builder()
        .css_classes(["chat-orb"])
        .valign(gtk::Align::Center)
        .build()
}
