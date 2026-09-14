//! The surfaces.
//!
//! Every file under here draws with libadwaita widgets and nothing else: the
//! application paints no ground, draws no box and sizes no type. What this
//! module holds is the handful of arrangements every surface shares, so the
//! clamp, the caption and the visibility rule are written once.

pub mod doctor;
pub mod home;
pub mod logs;
pub mod onboarding;
pub mod recovery;
pub mod settings;
pub mod widgets;

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::metrics;

/// What one page contributes to the window's header bar.
///
/// The window owns the bar, the back control, the one prominent action and the
/// primary menu; a page hands it the controls that belong to the page. The
/// redlines cap the trailing children at three, and the window counts.
#[derive(Default, Clone)]
pub struct PageToolbar {
    pub start: Vec<gtk::Widget>,
    pub end: Vec<gtk::Widget>,
}

/// The content column every surface but Logs sits in.
pub fn clamp(child: &impl IsA<gtk::Widget>) -> adw::Clamp {
    adw::Clamp::builder()
        .maximum_size(metrics::CLAMP_MAXIMUM)
        .tightening_threshold(metrics::CLAMP_TIGHTENING)
        .child(child)
        .build()
}

/// A vertical scroller that never scrolls sideways.
pub fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(child)
        .build()
}

/// Let one button's label shorten rather than hold the window open.
///
/// Ellipsizing is what makes the label's minimum small, which is what lets the
/// window reach the minimum size the redlines fix for it. Its natural size is
/// left alone on purpose: a cap on that is a label that is cut short at every
/// width, including the default one, where there was room for the whole word.
/// The accessible name and the tooltip are the caller's, and are always the
/// whole word.
pub fn shorten(button: &gtk::Button) {
    let Some(label) = button.child().and_downcast::<gtk::Label>() else {
        return;
    };
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
}

/// One statement the platform owes a person, as a row of body copy that wraps.
///
/// These are the sentences the product exists to be honest with: what Linux
/// never asks, what a machine cannot be made to do, why a helper cannot be
/// installed here. They are the only thing on the pane there is to read, so
/// they are read at body size in a box of the toolkit's, rather than in the
/// smallest and dimmest type the deck has.
pub fn statement_row(text: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(text)
        .title_lines(0)
        .activatable(false)
        .build()
}

/// Supporting copy, at body size in the toolkit's caption style.
pub fn caption(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .build();
    label.add_css_class("caption");
    label.add_css_class("dim-label");
    label
}

/// Two facts on one line.
///
/// The separator is punctuation rather than a word, which is why it is not a
/// catalogue row: there is nothing here for a translator to move, and the two
/// halves either side of it are the daemon's own.
pub fn beside(left: &str, right: &str) -> String {
    format!("{left} \u{b7} {right}")
}

/// One instant in this computer's own clock, with the day it happened on.
///
/// The daemon writes an offset timestamp and a person reads their own clock.
/// A stamp this build cannot parse is shown exactly as the daemon wrote it
/// rather than replaced with a guess. Logs has its own, to the millisecond and
/// without a date, because a log line is read against the lines around it.
pub fn local_moment(stamp: &str) -> String {
    let Ok(parsed) = gtk::glib::DateTime::from_iso8601(stamp, None) else {
        return stamp.to_string();
    };
    let Ok(local) = parsed.to_local() else {
        return stamp.to_string();
    };

    local
        .format("%x %H:%M")
        .map(|formatted| formatted.to_string())
        .unwrap_or_else(|_| stamp.to_string())
}

/// A value beside its label, dimmed the way the toolkit dims a suffix.
///
/// It never wraps: a fact's value belongs on the line its label is on, and a
/// value too long for the width available is shortened at its end rather than
/// folded under the label it belongs to. It holds on to a short value whole:
/// an ellipsizing label's minimum is the ellipsis, so a row whose supporting
/// text wraps takes the width first and cuts a word that would have fitted.
pub fn value_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .wrap(false)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .xalign(1.0)
        .build();
    label.add_css_class("dim-label");
    set_value(&label, text);
    label
}

/// Write a value into a label [`value_label`] built.
///
/// Through here rather than through `set_label`, so the floor is true of the
/// words the label carries now: every one of these is built empty and filled
/// when the daemon answers.
pub fn set_value(label: &gtk::Label, text: &str) {
    label.set_label(text);
    label.set_width_chars(VALUE_FLOOR.min(text.chars().count() as i32));
}

/// The most characters a value keeps hold of before it starts shortening.
///
/// Long enough for every word the product's own vocabularies carry, short
/// enough that a sentence the daemon puts in a value slot still shortens rather
/// than holding the row open.
const VALUE_FLOOR: i32 = 16;

/// An identifier, in the one style reserved for identifiers.
///
/// It wraps, and it wraps between characters as well as between words: an
/// identifier is a path, a unit name or a line out of a journal, and one long
/// enough to hold the window open is the thing the evidence expanders are full
/// of. Wrapping rather than shortening, because evidence a person cannot read
/// the end of is evidence they have to go and find somewhere else; the natural
/// width is still the whole line, so nothing wraps until it has to.
pub fn identifier_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build();
    label.add_css_class("monospace");
    label
}

/// One labelled fact: a row with a title and a value, and nothing to press.
///
/// The value's label comes back with the row, because the value is the one
/// thing about a fact that changes and hunting for it in the tree afterwards is
/// how a refresh quietly stops updating one row.
pub fn fact_row(label: Key, value: &str) -> (adw::ActionRow, gtk::Label) {
    let row = adw::ActionRow::builder()
        .title(copy::text(label))
        .activatable(false)
        .build();
    let value = value_label(value);
    row.add_suffix(&value);
    (row, value)
}

/// A line of supporting copy that lives under the row it belongs to.
///
/// A refusal belongs beneath the control it refused, and libadwaita has no slot
/// for one on every row kind. This is the toolkit's own row holding a caption,
/// added straight after its control, and hidden until there is something to
/// say.
#[derive(Clone)]
pub struct CaptionRow {
    row: adw::PreferencesRow,
    label: gtk::Label,
}

impl CaptionRow {
    /// A hidden caption row.
    pub fn new() -> Self {
        let label = caption("");
        label.set_margin_start(metrics::SPACE_HEADING);
        label.set_margin_end(metrics::SPACE_HEADING);
        label.set_margin_top(metrics::SPACE_TIGHT);
        label.set_margin_bottom(metrics::SPACE_TIGHT);

        let row = adw::PreferencesRow::builder()
            .activatable(false)
            .selectable(false)
            .focusable(false)
            .child(&label)
            .visible(false)
            .build();

        Self { row, label }
    }

    /// The row, to add to a group.
    pub fn row(&self) -> &adw::PreferencesRow {
        &self.row
    }

    /// Say something, or say nothing and disappear.
    pub fn set(&self, text: Option<&str>) {
        match text {
            Some(text) if !text.is_empty() => {
                self.label.set_label(text);
                self.row.set_visible(true);
            }
            _ => {
                self.label.set_label("");
                self.row.set_visible(false);
            }
        }
    }

    /// Whether it is saying anything.
    pub fn is_shown(&self) -> bool {
        self.row.is_visible()
    }
}

impl Default for CaptionRow {
    fn default() -> Self {
        Self::new()
    }
}

/// A group holding one caption, which is there only while the caption is.
///
/// A preferences group draws its own box, so an empty one is a hairline around
/// nothing. The group follows the row rather than each caller remembering to
/// hide it.
pub fn caption_group(caption: &CaptionRow) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.add(caption.row());
    group.set_visible(caption.is_shown());

    caption
        .row()
        .bind_property("visible", &group, "visible")
        .sync_create()
        .build();

    group
}

/// A vertical box with the gap between groups the metrics module fixes.
pub fn column() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(metrics::SPACE_GUTTER)
        .build()
}

/// Tell one page every time it is shown.
///
/// This is what a surface reads its data on: a page that has just been put in
/// front of someone asks the daemon once. Polling is a separate question, and
/// `watch_visibility` answers that one.
pub fn on_shown(widget: &impl IsA<gtk::Widget>, shown: impl Fn() + 'static) {
    widget.as_ref().connect_map(move |_| shown());
}

/// Tell one page when it is in front of a person, and when it is not.
///
/// Visible means mapped *and* in the active window: a page behind another
/// window is not being read, and the polls that follow this rule are the reason
/// an idle Fermix costs nothing. The next time the window comes forward the
/// page hears about it and starts again.
pub fn watch_visibility(widget: &impl IsA<gtk::Widget>, on_change: impl Fn(bool) + 'static) {
    let widget = widget.as_ref().clone();
    let on_change: Rc<dyn Fn(bool)> = Rc::new(on_change);
    let connected = Rc::new(Cell::new(false));

    let evaluate: Rc<dyn Fn()> = {
        let widget = widget.clone();
        let on_change = Rc::clone(&on_change);
        Rc::new(move || on_change(is_in_front(&widget)))
    };

    {
        let evaluate = Rc::clone(&evaluate);
        let connected = Rc::clone(&connected);
        widget.connect_map(move |widget| {
            if !connected.replace(true) {
                if let Some(window) = widget.root().and_downcast::<gtk::Window>() {
                    let evaluate = Rc::clone(&evaluate);
                    window.connect_is_active_notify(move |_| evaluate());
                }
            }
            evaluate();
        });
    }

    widget.connect_unmap(move |_| on_change(false));
}

/// Whether one widget is mapped and in the window someone is using.
pub fn is_in_front(widget: &gtk::Widget) -> bool {
    let active = widget
        .root()
        .and_downcast::<gtk::Window>()
        .map(|window| window.is_active())
        .unwrap_or(false);

    widget.is_mapped() && active
}

/// Open Settings at the pane that answers something.
///
/// The selection is the model's, so the presentation opens where it is told and
/// stays there. Every surface that sends a person to a pane goes through here:
/// two ways to open one pane would be two ways to get it wrong.
pub fn open_pane(
    widget: &impl IsA<gtk::Widget>,
    settings: &crate::models::SettingsModel,
    pane: crate::management::types::SettingsPane,
) {
    settings.select_pane(pane);
    let _ = WidgetExt::activate_action(widget.as_ref(), "win.settings", None);
}

/// The window one widget is in, for a dialog that needs a parent.
pub fn window_of(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.as_ref().root().and_downcast::<gtk::Window>()
}
