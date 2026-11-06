//! One daemon settings section drawn as a preferences group. Each row kind has
//! one control; every write leaves as a window action carrying the section, the
//! row key and the JSON value, so the controller in `settings_flow.rs` does the
//! talking. A section is redrawn only when what it shows changed.
//!
//! A section's advanced rows come last, under one Advanced row that starts closed and stays as
//! the person left it across redraws. A pane can put one line of its own under a row, such as
//! where reach from anywhere stands under its switch, and rows of its own under a row or in its
//! place, such as the Cues filter model, whose two keys are written together.

use crate::fields::{
    field_row, let_go, resume, secret_row as secret_row_into, still_active, typing_in, Live, Slot,
};
use adw::prelude::*;
use fermix_client::settings::{
    awaits_listing, choice_items, list_items, list_prompt, list_with, list_without, number_answer,
    number_view, placeholder, text_answer, text_value, value_width, ChoiceItem, Kind, Row,
    SectionRows, FIELD_CHARS, NOT_SET, UNKNOWN_KIND, WIDEST_VALUE_CHARS,
};
use gtk::glib::{self, variant::ToVariant};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// A number is written once its control has been still this long, not on every click.
const NUMBER_SETTLE: Duration = Duration::from_millis(700);
/// What the button beside a model row whose listing failed does.
const LIST_AGAIN: &str = "List the models again";
/// The row the rows most people never change are folded under.
const ADVANCED: &str = "Advanced";

/// Everything a section's rows are drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// `None` until the section has been read once.
    pub rows: Option<SectionRows>,
    /// Why the section has no rows yet, once a read of it failed.
    pub unread: Option<String>,
    /// The daemon's refusal sentence, by row key.
    pub errors: Vec<(String, String)>,
    /// The provider's models could not be listed: its model row offers to list them again.
    pub listing_failed: bool,
    /// Set while the settings file changed outside Fermix: nothing may be written.
    pub locked: bool,
}

/// Which of a section's rows a view draws; the rest belong to other controls.
pub type Keep = Box<dyn Fn(&str) -> bool>;

/// Takes a row's key and what was set in it.
pub type Hold<T> = Box<dyn Fn(&str, T)>;

/// Where a form's rows send what is set in them: each value as it changes, and each secret as it
/// is typed, which the form holds until it is sent. Nothing leaves as a window action.
pub struct FormSink {
    pub value: Hold<Value>,
    pub secret: Hold<String>,
    /// The title of the row the advanced rows fold under.
    pub folded: &'static str,
}

/// Where a row's writes go: the window's `setting-apply` for a settings section, or a form.
#[derive(Clone)]
enum To {
    Section(String),
    Form(Rc<FormSink>),
}

/// Where a pane's own row goes in a section the descriptor draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Right under the row with that key, open or folded with it.
    Under,
    /// In that row's place: the descriptor draws no control for it, and says nothing under it.
    Instead,
}

/// A row of a pane's own in a section. The same widget is placed again on every redraw, so a
/// change to it never redraws the section; the pane keeps it current.
pub struct Extra {
    pub key: String,
    pub place: Place,
    pub row: gtk::Widget,
}

pub struct SectionView {
    pub id: String,
    pub group: adw::PreferencesGroup,
    keep: Keep,
    /// Each row's widget, by row key (empty for the loading and unread rows).
    widgets: RefCell<Vec<(String, gtk::Widget)>>,
    /// What was added to the group: the widgets above less the advanced ones, which sit in the
    /// Advanced row, plus that row and the note.
    children: RefCell<Vec<gtk::Widget>>,
    /// Whether the Advanced row is open, kept when the section is drawn again.
    advanced_open: Cell<bool>,
    advanced: RefCell<Option<adw::ExpanderRow>>,
    note: Option<Note>,
    extras: Vec<Extra>,
    to: To,
    /// A form's secret fields, kept across redraws so what was typed stays in its field and
    /// nowhere else.
    held: RefCell<Vec<(String, adw::ActionRow)>>,
    live: RefCell<Live>,
    shown: RefCell<Option<Drawn>>,
}

impl SectionView {
    pub fn new(id: &str, title: Option<&str>) -> Rc<SectionView> {
        SectionView::keeping(id, title, Box::new(|_| true))
    }

    /// A view of only the rows `keep` accepts.
    pub fn keeping(id: &str, title: Option<&str>, keep: Keep) -> Rc<SectionView> {
        SectionView::build(
            id,
            title,
            keep,
            None,
            Vec::new(),
            To::Section(id.to_owned()),
        )
    }

    /// A view of only the rows `keep` accepts, with a line of the pane's own under row `key`,
    /// hidden until `set_note` gives it words.
    pub fn keeping_with_note(id: &str, keep: Keep, key: &str) -> Rc<SectionView> {
        let note = Some(Note::new(key));
        SectionView::build(id, None, keep, note, Vec::new(), To::Section(id.to_owned()))
    }

    /// A view of only the rows `keep` accepts, with the pane's own rows under some and in place
    /// of others. A row drawn instead of the daemon's must still be one `keep` accepts.
    pub fn keeping_with_extras(id: &str, keep: Keep, extras: Vec<Extra>) -> Rc<SectionView> {
        SectionView::build(id, None, keep, None, extras, To::Section(id.to_owned()))
    }

    /// A form drawn from rows the daemon published (M60 cues §7): every change commits as typed
    /// into `sink`, a secret row's text is held there until the form is sent, and a refusal on a
    /// folded row opens its fold. The form's own rows go under and in place of some.
    pub fn form(id: &str, sink: FormSink, extras: Vec<Extra>) -> Rc<SectionView> {
        let to = To::Form(Rc::new(sink));
        SectionView::build(id, None, Box::new(|_| true), None, extras, to)
    }

    /// The next draw opens the fold, as a form whose folded rows already hold something does.
    pub fn open_folded(&self) {
        self.advanced_open.set(true);
    }

    fn build(
        id: &str,
        title: Option<&str>,
        keep: Keep,
        note: Option<Note>,
        extras: Vec<Extra>,
        to: To,
    ) -> Rc<SectionView> {
        let group = adw::PreferencesGroup::new();
        if let Some(title) = title {
            group.set_title(&glib::markup_escape_text(title));
        }
        Rc::new(SectionView {
            id: id.to_owned(),
            group,
            keep,
            widgets: RefCell::default(),
            children: RefCell::default(),
            advanced_open: Cell::new(false),
            advanced: RefCell::default(),
            note,
            extras,
            to,
            held: RefCell::default(),
            live: RefCell::new(Rc::new(Cell::new(true))),
            shown: RefCell::default(),
        })
    }

    /// Draws the section afresh. Someone typing in a row keeps their place: the
    /// rows are rebuilt, and the field they were in gets its text, cursor and
    /// focus back, so a save elsewhere in the form never swallows keystrokes.
    pub fn show(&self, drawn: Drawn) {
        if self.shown.borrow().as_ref() == Some(&drawn) {
            return;
        }
        self.live.borrow().set(false);
        let typing = typing_in(&self.widgets.borrow());
        if typing.is_some() {
            let_go(&self.widgets.borrow());
        }
        if let Some(advanced) = self.advanced.take() {
            self.advanced_open.set(advanced.is_expanded());
            // The pane's own rows outlive the expander they sat in, and go into the next one.
            for extra in self.extras.iter().filter(|e| e.row.is_ancestor(&advanced)) {
                advanced.remove(&extra.row);
            }
            for (_, row) in self
                .held
                .borrow()
                .iter()
                .filter(|(_, r)| r.is_ancestor(&advanced))
            {
                advanced.remove(row);
            }
        }
        if refused_folded(&drawn) {
            self.advanced_open.set(true);
        }
        for old in self.children.borrow_mut().drain(..) {
            self.group.remove(&old);
        }
        self.widgets.borrow_mut().clear();
        let live: Live = Rc::new(Cell::new(true));
        let fresh = match (&drawn.rows, &drawn.unread) {
            (Some(section), _) => {
                self.prune_held(&section.rows);
                self.row_widgets(section, &drawn, &live)
            }
            (None, Some(sentence)) => vec![(String::new(), false, unread_row(&self.id, sentence))],
            (None, None) => vec![(String::new(), false, loading_row())],
        };
        self.place(&fresh);
        let fresh: Vec<(String, gtk::Widget)> = fresh
            .into_iter()
            .map(|(key, _, widget)| (key, widget))
            .collect();
        self.group.set_sensitive(!drawn.locked);
        if let Some(typing) = typing {
            resume(&fresh, &typing);
        }
        *self.widgets.borrow_mut() = fresh;
        *self.live.borrow_mut() = live;
        *self.shown.borrow_mut() = Some(drawn);
    }

    /// Adds the rows to the group: the ordinary ones in order with the note and the pane's rows
    /// under theirs, then the advanced ones, with theirs, under one Advanced row, where there are
    /// any.
    fn place(&self, rows: &[(String, bool, gtk::Widget)]) {
        let mut children = Vec::new();
        for (key, _, widget) in rows.iter().filter(|(_, advanced, _)| !advanced) {
            children.push(widget.clone());
            if let Some(note) = self.note.as_ref().filter(|note| note.key == *key) {
                children.push(note.row.clone().upcast());
            }
            children.extend(self.under(key));
        }
        let advanced: Vec<(&String, &gtk::Widget)> = rows
            .iter()
            .filter(|(_, advanced, _)| *advanced)
            .map(|(key, _, widget)| (key, widget))
            .collect();
        if !advanced.is_empty() {
            let expander = adw::ExpanderRow::builder()
                .title(self.folded_title())
                .expanded(self.advanced_open.get())
                .build();
            for (key, widget) in advanced {
                expander.add_row(widget);
                for extra in self.under(key) {
                    expander.add_row(&extra);
                }
            }
            children.push(expander.clone().upcast());
            self.advanced.replace(Some(expander));
        }
        for child in &children {
            self.group.add(child);
        }
        *self.children.borrow_mut() = children;
    }

    /// A form's secret row: the field already holding what was typed, or a new one. Fields of
    /// rows the form no longer has go with their text.
    fn held_row(&self, row: &Row) -> Option<adw::ActionRow> {
        let To::Form(sink) = &self.to else {
            return None;
        };
        if row.kind != Kind::Secret || row.read_only {
            return None;
        }
        let mut held = self.held.borrow_mut();
        if let Some((_, field)) = held.iter().find(|(key, _)| *key == row.key) {
            field.remove_css_class("setting-refused");
            let footer = row.footer.as_deref().unwrap_or("");
            field.set_subtitle(&glib::markup_escape_text(footer));
            return Some(field.clone());
        }
        let field = held_secret_row(sink, row);
        held.push((row.key.clone(), field.clone()));
        Some(field)
    }

    fn folded_title(&self) -> &'static str {
        match &self.to {
            To::Section(_) => ADVANCED,
            To::Form(sink) => sink.folded,
        }
    }

    /// The pane's rows placed under row `key`.
    fn under(&self, key: &str) -> Vec<gtk::Widget> {
        self.extras
            .iter()
            .filter(|e| e.place == Place::Under && e.key == key)
            .map(|e| e.row.clone())
            .collect()
    }

    /// The pane's row drawn in place of row `key`, if it has one.
    fn instead(&self, key: &str) -> Option<gtk::Widget> {
        self.extras
            .iter()
            .find(|e| e.place == Place::Instead && e.key == key)
            .map(|e| e.row.clone())
    }

    /// Shows the pane's line under its row, or hides it.
    pub fn set_note(&self, line: Option<(&str, NoteKind)>) {
        if let Some(note) = &self.note {
            note.show(line);
        }
    }

    fn row_widgets(
        &self,
        section: &SectionRows,
        drawn: &Drawn,
        live: &Live,
    ) -> Vec<(String, bool, gtk::Widget)> {
        section
            .rows
            .iter()
            .filter(|row| (self.keep)(&row.key))
            .map(|row| {
                if let Some(own) = self.instead(&row.key) {
                    return (row.key.clone(), row.advanced, own);
                }
                let error = drawn.errors.iter().find(|(k, _)| *k == row.key);
                let widget = if drawn.listing_failed && awaits_listing(row) {
                    relist_row(&self.id, &self.to, row, live).upcast()
                } else if let Some(held) = self.held_row(row) {
                    held.upcast()
                } else {
                    row_widget(&self.to, row, live)
                };
                if let Some((_, sentence)) = error {
                    show_refusal(&widget, sentence);
                }
                (row.key.clone(), row.advanced, widget)
            })
            .collect()
    }

    /// Lets go of a form's secret fields whose rows are gone.
    fn prune_held(&self, rows: &[Row]) {
        self.held
            .borrow_mut()
            .retain(|(key, _)| rows.iter().any(|r| r.key == *key && r.kind == Kind::Secret));
    }
}

/// How a pane's line is drawn: a plain note, a start that moves on by itself, or a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Plain,
    Progress,
    Failure,
}

/// A pane's one line under a row. The same row is placed again on every redraw, so a change to
/// the line never redraws the section.
struct Note {
    key: String,
    row: adw::ActionRow,
    spinner: adw::Spinner,
}

impl Note {
    fn new(key: &str) -> Note {
        let row = adw::ActionRow::builder()
            .activatable(false)
            .visible(false)
            .build();
        let spinner = adw::Spinner::builder().visible(false).build();
        row.add_suffix(&spinner);
        Note {
            key: key.to_owned(),
            row,
            spinner,
        }
    }

    fn show(&self, line: Option<(&str, NoteKind)>) {
        let Some((text, kind)) = line else {
            self.row.set_visible(false);
            return;
        };
        self.row.set_title(&glib::markup_escape_text(text));
        self.spinner.set_visible(kind == NoteKind::Progress);
        let failure = kind == NoteKind::Failure;
        set_class(&self.row, "error", failure);
        set_class(&self.row, "dimmed", !failure);
        self.row.set_visible(true);
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

fn loading_row() -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title("Reading from Fermix…")
        .build();
    row.add_suffix(&adw::Spinner::new());
    row.upcast()
}

/// Stands in for a section whose read failed, with one way to read it again.
fn unread_row(section: &str, sentence: &str) -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title("These settings did not load")
        .subtitle(glib::markup_escape_text(sentence))
        .build();
    let again = gtk::Button::builder()
        .label("Try again")
        .valign(gtk::Align::Center)
        .action_name("win.section-read")
        .action_target(&section.to_variant())
        .build();
    row.add_suffix(&again);
    row.add_css_class("setting-refused");
    row.upcast()
}

/// The refusal replaces the row's footer until the next read, in the error colour.
fn show_refusal(widget: &gtk::Widget, sentence: &str) {
    let Some(row) = widget.downcast_ref::<adw::PreferencesRow>() else {
        return;
    };
    if let Some(action) = row.downcast_ref::<adw::ActionRow>() {
        action.set_subtitle(&glib::markup_escape_text(sentence));
    } else if let Some(expander) = row.downcast_ref::<adw::ExpanderRow>() {
        expander.set_subtitle(&glib::markup_escape_text(sentence));
    }
    row.add_css_class("setting-refused");
}

fn row_widget(to: &To, row: &Row, live: &Live) -> gtk::Widget {
    if row.read_only {
        return fact_row(row).upcast();
    }
    let widget: gtk::Widget = match row.kind {
        Kind::Toggle => toggle_row(to, row).upcast(),
        Kind::Choice if row.suggestions => suggestion_row(to, row, live).upcast(),
        Kind::Choice => choice_row(to, row).upcast(),
        Kind::Text => text_row(to, row, live).upcast(),
        Kind::Number => number_row(to, row).upcast(),
        Kind::Secret => secret_row(to, row, live),
        Kind::List => list_row(to, row).upcast(),
        Kind::Unknown => unknown_row(row).upcast(),
    };
    if let Some(info) = &row.info {
        add_info(&widget, info);
    }
    widget
}

/// Sends one value for one row: to the window's `setting-apply` action, which does the write, or
/// to the form that holds it.
fn apply(widget: &impl IsA<gtk::Widget>, to: &To, key: &str, value: &Value) {
    let section = match to {
        To::Form(sink) => return (sink.value)(key, value.clone()),
        To::Section(section) => section,
    };
    let target = (section.as_str(), key, value.to_string()).to_variant();
    if let Err(e) = widget.activate_action("win.setting-apply", Some(&target)) {
        glib::g_warning!("fermix", "a setting could not be sent: {e}");
    }
}

/// Whether a refusal lands on a row folded under Advanced, which then opens.
fn refused_folded(drawn: &Drawn) -> bool {
    let Some(section) = &drawn.rows else {
        return false;
    };
    section
        .rows
        .iter()
        .any(|row| row.advanced && drawn.errors.iter().any(|(key, _)| *key == row.key))
}

fn titled(row: &Row) -> adw::ActionRow {
    let action = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .build();
    if let Some(footer) = &row.footer {
        action.set_subtitle(&glib::markup_escape_text(footer));
    }
    action
}

fn toggle_row(to: &To, row: &Row) -> adw::SwitchRow {
    let switch = adw::SwitchRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .active(row.value.as_bool().unwrap_or(false))
        .build();
    if let Some(footer) = &row.footer {
        switch.set_subtitle(&glib::markup_escape_text(footer));
    }
    let (to, key) = (to.clone(), row.key.clone());
    switch.connect_active_notify(move |s| apply(s, &to, &key, &Value::Bool(s.is_active())));
    switch
}

fn choice_row(to: &To, row: &Row) -> adw::ComboRow {
    let (items, selected) = choice_items(row);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    let combo = adw::ComboRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .model(&gtk::StringList::new(&labels))
        .selected(u32::try_from(selected).expect("at most 200 options"))
        .build();
    if let Some(footer) = &row.footer {
        combo.set_subtitle(&glib::markup_escape_text(footer));
    }
    let items = Rc::new(items);
    combo.set_factory(Some(&value_factory()));
    combo.set_list_factory(Some(&option_factory(items.clone())));
    let (to, key) = (to.clone(), row.key.clone());
    let last = Cell::new(combo.selected());
    combo.connect_selected_notify(move |combo| {
        let index = combo.selected();
        let item = &items[usize::try_from(index).expect("an index")];
        if item.disabled || item.value.is_none() {
            // Put the selection back: an option the daemon offers but refuses says why.
            let reason = item.hint.as_deref().unwrap_or(NOT_SET);
            combo.set_subtitle(&glib::markup_escape_text(reason));
            combo.set_selected(last.get());
            return;
        }
        if index == last.replace(index) {
            return;
        }
        let value = Value::String(item.value.clone().expect("checked above"));
        apply(combo, &to, &key, &value);
    });
    combo
}

/// Draws the chosen option in the row. The row's own label stops at 20
/// characters; this one shows the whole value when the row has room, and
/// a value too long to be kept whole says the rest in its tooltip.
pub fn value_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
        let label = gtk::Label::builder()
            .xalign(0.0)
            .max_width_chars(WIDEST_VALUE_CHARS)
            .build();
        item.set_child(Some(&label));
    });
    factory.connect_bind(|_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
        let label = item
            .child()
            .and_downcast::<gtk::Label>()
            .expect("set up above");
        let text = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .expect("a choice model holds strings")
            .string();
        // Counted characters only approximate drawn ones, so a value kept
        // whole is never ellipsized: its minimum is its own drawn width.
        let width = value_width(&text, 1);
        let (ellipsize, min_chars, tooltip) = if width.whole() {
            (gtk::pango::EllipsizeMode::None, -1, None)
        } else {
            (
                gtk::pango::EllipsizeMode::End,
                width.min_chars,
                Some(text.as_str()),
            )
        };
        label.set_ellipsize(ellipsize);
        label.set_width_chars(min_chars);
        label.set_tooltip_text(tooltip);
        label.set_text(&text);
    });
    factory
}

/// Draws each option in the open list as its label over its hint.
pub fn option_factory(items: Rc<Vec<ChoiceItem>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
        let label = gtk::Label::builder().xalign(0.0).build();
        let hint = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["dimmed", "caption"])
            .build();
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.append(&label);
        lines.append(&hint);
        item.set_child(Some(&lines));
    });
    factory.connect_bind(move |_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
        let lines = item.child().expect("set up above");
        let label = lines
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("label");
        let hint = label
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .expect("hint");
        let Some(option) = items.get(usize::try_from(item.position()).expect("an index")) else {
            return;
        };
        label.set_text(&option.label);
        hint.set_text(option.hint.as_deref().unwrap_or(""));
        hint.set_visible(option.hint.is_some());
        item.set_selectable(!option.disabled && option.value.is_some());
        lines.set_sensitive(!option.disabled);
    });
    factory
}

/// A field for a text value: commits on Enter or when focus leaves, and only
/// when the text changed; Escape puts the daemon's value back. It is as wide as
/// the value it opens with, within `value_width`'s bounds.
fn text_field(to: &To, row: &Row, live: &Live) -> gtk::Entry {
    let (text, empty) = (text_value(row), placeholder(row));
    let width = value_width(if text.is_empty() { &empty } else { &text }, FIELD_CHARS);
    let entry = gtk::Entry::builder()
        .text(&text)
        .placeholder_text(&empty)
        .valign(gtk::Align::Center)
        .width_chars(width.min_chars)
        .max_width_chars(width.natural_chars)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&row.label)]);
    if let To::Form(_) = to {
        // A form holds what is typed as it is typed; nothing is written until it is sent.
        let (to, key, live) = (to.clone(), row.key.clone(), live.clone());
        entry.connect_changed(move |entry| {
            if live.get() {
                apply(entry, &to, &key, &Value::from(entry.text().trim()));
            }
        });
        return entry;
    }
    let commit = {
        let (to, row, live) = (to.clone(), row.clone(), live.clone());
        move |entry: &gtk::Entry| {
            let Some(value) = text_answer(&row, &entry.text()) else {
                return;
            };
            if live.get() {
                apply(entry, &to, &row.key, &value);
            }
        }
    };
    let on_leave = commit.clone();
    entry.connect_activate(commit);
    let focus = gtk::EventControllerFocus::new();
    let watched = entry.downgrade();
    focus.connect_leave(move |_| {
        if let Some(entry) = watched.upgrade().filter(still_active) {
            on_leave(&entry);
        }
    });
    entry.add_controller(focus);
    restore_on_escape(&entry, text_value(row));
    entry
}

fn restore_on_escape(entry: &gtk::Entry, daemon_value: String) {
    let keys = gtk::EventControllerKey::new();
    let target = entry.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(target) = target.upgrade().filter(|_| key == gtk::gdk::Key::Escape) else {
            return glib::Propagation::Proceed;
        };
        target.set_text(&daemon_value);
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

fn text_row(to: &To, row: &Row, live: &Live) -> adw::ActionRow {
    let action = titled(row);
    let entry = text_field(to, row, live);
    action.add_suffix(&entry);
    field_row(&action, &entry);
    action
}

/// A text value with the daemon's suggestions one click away; any value is allowed.
fn suggestion_row(to: &To, row: &Row, live: &Live) -> adw::ActionRow {
    linked_row(to, row, live, |entry| {
        suggestions_button(to, row, entry).upcast()
    })
}

/// A model row whose provider could not list its models: any name can still
/// be typed, and the button where the suggestions would be lists them again.
fn relist_row(section: &str, to: &To, row: &Row, live: &Live) -> adw::ActionRow {
    let again = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(LIST_AGAIN)
        .action_name("win.section-read")
        .action_target(&section.to_variant())
        .build();
    again.update_property(&[gtk::accessible::Property::Label(LIST_AGAIN)]);
    linked_row(to, row, live, |_| again.upcast())
}

/// A row's text field with one button joined to its end.
fn linked_row(
    to: &To,
    row: &Row,
    live: &Live,
    button: impl FnOnce(&gtk::Entry) -> gtk::Widget,
) -> adw::ActionRow {
    let action = titled(row);
    let entry = text_field(to, row, live);
    let linked = gtk::Box::builder()
        .css_classes(["linked"])
        .valign(gtk::Align::Center)
        .build();
    linked.append(&entry);
    linked.append(&button(&entry));
    action.add_suffix(&linked);
    field_row(&action, &entry);
    action
}

fn suggestions_button(to: &To, row: &Row, entry: &gtk::Entry) -> gtk::MenuButton {
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["navigation-sidebar"])
        .build();
    for option in row.options.iter().filter(|o| !o.disabled) {
        let label = option.hint.as_ref().map_or_else(
            || option.label.clone(),
            |hint| format!("{} · {hint}", option.label),
        );
        let line = gtk::Label::builder().label(label).xalign(0.0).build();
        let item = gtk::ListBoxRow::builder().child(&line).build();
        item.set_widget_name(&option.value);
        list.append(&item);
    }
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(320)
        .child(&list)
        .build();
    let popover = gtk::Popover::builder().child(&scroller).build();
    let (to, key, entry, shut) = (to.clone(), row.key.clone(), entry.clone(), popover.clone());
    list.connect_row_activated(move |_, item| {
        let value = item.widget_name().to_string();
        shut.popdown();
        entry.set_text(&value);
        apply(&entry, &to, &key, &Value::String(value));
    });
    // Nothing to suggest (a model list still on its way, or one that failed):
    // the field still takes any value, and the button opens no empty list.
    gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .popover(&popover)
        .tooltip_text("Suggestions")
        .sensitive(row.options.iter().any(|o| !o.disabled))
        .build()
}

fn number_row(to: &To, row: &Row) -> adw::SpinRow {
    let view = Rc::new(number_view(row));
    let adjustment = gtk::Adjustment::new(
        view.value,
        view.min,
        view.max,
        view.step,
        view.step * 10.0,
        0.0,
    );
    let spin = adw::SpinRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .adjustment(&adjustment)
        .digits(view.digits)
        .build();
    if let Some(footer) = &row.footer {
        spin.set_subtitle(&glib::markup_escape_text(footer));
    }
    // The figure reads with its unit ("85%", "24 hours"), and types back with or without it.
    let shows = view.clone();
    spin.connect_output(move |spin| {
        spin.set_text(&shows.text(spin.value()));
        true
    });
    let reads = view.clone();
    spin.connect_input(move |spin| Some(reads.parse(&spin.text()).ok_or(())));
    write_when_still(&spin, to, row);
    spin
}

/// Writes the number once the control has been still for `NUMBER_SETTLE`,
/// and only when it differs from the daemon's value.
fn write_when_still(spin: &adw::SpinRow, to: &To, row: &Row) {
    let (to, row, pending) = (to.clone(), row.clone(), Rc::new(Cell::new(0u64)));
    spin.connect_value_notify(move |spin| {
        let ticket = pending.get() + 1;
        pending.set(ticket);
        let (spin, to, row, pending) = (spin.clone(), to.clone(), row.clone(), pending.clone());
        glib::timeout_add_local_once(NUMBER_SETTLE, move || {
            if pending.get() != ticket {
                return;
            }
            let value = number_answer(&row, spin.value());
            if value.as_f64() != row.value.as_f64() {
                apply(&spin, &to, &row.key, &value);
            }
        });
    });
}

/// A secret row whose store and remove go to the window's `secret-set` and
/// `secret-remove` actions, carrying the section and the row key; in a form, a field whose
/// text the form holds until it is sent.
fn secret_row(to: &To, row: &Row, live: &Live) -> gtk::Widget {
    let section = match to {
        To::Form(sink) => return held_secret_row(sink, row).upcast(),
        To::Section(section) => section,
    };
    let action = titled(row);
    let target = Rc::new((section.clone(), row.key.clone()));
    let on_remove = target.clone();
    let slot = Slot {
        label: row.label.clone(),
        present: row.present.unwrap_or(false),
        store: Rc::new(move |entry, value| {
            let (section, key) = &*target;
            let sent = (section.as_str(), key.as_str(), value.as_str()).to_variant();
            if let Err(e) = entry.activate_action("win.secret-set", Some(&sent)) {
                glib::g_warning!("fermix", "a secret could not be sent: {e}");
            }
        }),
        remove: Rc::new(move |button| {
            let (section, key) = &*on_remove;
            let sent = (section.as_str(), key.as_str()).to_variant();
            if let Err(e) = button.activate_action("win.secret-remove", Some(&sent)) {
                glib::g_warning!("fermix", "a secret could not be removed: {e}");
            }
        }),
    };
    secret_row_into(&action, slot, live);
    action.upcast()
}

/// A form's secret: typed into a password field and handed to the form as typed, never shown
/// anywhere else and never sent before the form is. The row outlives redraws, so its text is
/// never set from outside its field.
fn held_secret_row(sink: &Rc<FormSink>, row: &Row) -> adw::ActionRow {
    let action = titled(row);
    let entry = gtk::PasswordEntry::builder()
        .show_peek_icon(true)
        .valign(gtk::Align::Center)
        .width_chars(FIELD_CHARS)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&row.label)]);
    action.add_suffix(&entry);
    field_row(&action, &entry);
    if let Some(info) = &row.info {
        action.add_suffix(&info_button(info, "More about this setting"));
    }
    let (sink, key) = (sink.clone(), row.key.clone());
    entry.connect_changed(move |entry| (sink.secret)(&key, entry.text().to_string()));
    action
}

/// A list is written whole: each change sends the full replacement.
fn list_row(to: &To, row: &Row) -> adw::ExpanderRow {
    let items = list_items(row);
    let expander = adw::ExpanderRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .expanded(true)
        .build();
    if let Some(footer) = &row.footer {
        expander.set_subtitle(&glib::markup_escape_text(footer));
    }
    for item in &items {
        let line = adw::ActionRow::builder()
            .title(glib::markup_escape_text(item))
            .build();
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text(format!("Remove {item}"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let (to, key, next) = (to.clone(), row.key.clone(), list_without(&items, item));
        remove.connect_clicked(move |b| apply(b, &to, &key, &Value::from(next.clone())));
        line.add_suffix(&remove);
        expander.add_row(&line);
    }
    expander.add_row(&list_adder(to, row, items));
    expander
}

/// The field a new entry is typed in, its title the row's example where it has one.
fn list_adder(to: &To, row: &Row, items: Vec<String>) -> adw::EntryRow {
    let adder = adw::EntryRow::builder()
        .title(glib::markup_escape_text(&list_prompt(row)))
        .show_apply_button(true)
        .build();
    adder
        .upcast_ref::<gtk::Widget>()
        .update_property(&[gtk::accessible::Property::Label(&row.label)]);
    let (to, key) = (to.clone(), row.key.clone());
    adder.connect_apply(move |adder| {
        if let Some(next) = list_with(&items, &adder.text()) {
            apply(adder, &to, &key, &Value::from(next));
        }
    });
    adder
}

/// A value shown here and changed elsewhere.
fn fact_row(row: &Row) -> adw::ActionRow {
    let action = titled(row);
    let value = match (&row.value, row.kind) {
        (Value::String(v), Kind::Choice) => row
            .options
            .iter()
            .find(|o| o.value == *v)
            .map_or_else(|| v.clone(), |o| o.label.clone()),
        (Value::String(v), _) if !v.is_empty() => v.clone(),
        (Value::Bool(on), _) => if *on { "On" } else { "Off" }.to_owned(),
        (Value::Number(n), _) => n.to_string(),
        (Value::Array(items), _) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        _ => NOT_SET.to_owned(),
    };
    let label = gtk::Label::builder()
        .label(value)
        .selectable(true)
        .css_classes(["dimmed"])
        .build();
    action.add_suffix(&label);
    action
}

fn unknown_row(row: &Row) -> adw::ActionRow {
    adw::ActionRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .subtitle(UNKNOWN_KIND)
        .build()
}

/// The longer explanation, kept one click away behind an (i).
fn add_info(widget: &gtk::Widget, info: &str) {
    let more = info_button(info, "More about this setting");
    if let Some(row) = widget.downcast_ref::<adw::ActionRow>() {
        row.add_suffix(&more);
    } else if let Some(row) = widget.downcast_ref::<adw::ComboRow>() {
        row.add_suffix(&more);
    } else if let Some(row) = widget.downcast_ref::<adw::EntryRow>() {
        row.add_suffix(&more);
    }
}

/// An (i) that opens `info` in a popover; `tooltip` says what it explains.
pub fn info_button(info: &str, tooltip: &str) -> gtk::MenuButton {
    let text = gtk::Label::builder()
        .label(info)
        .wrap(true)
        .max_width_chars(48)
        .xalign(0.0)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    let popover = gtk::Popover::builder().child(&text).build();
    gtk::MenuButton::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .popover(&popover)
        .build()
}
