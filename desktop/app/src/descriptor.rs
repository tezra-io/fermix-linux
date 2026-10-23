//! One daemon settings section drawn as a preferences group. Each row kind has
//! one control; every write leaves as a window action carrying the section, the
//! row key and the JSON value, so the controller in `settings_flow.rs` does the
//! talking. A section is redrawn only when what it shows changed.
//!
//! A section's advanced rows come last, under one Advanced row that starts closed and stays as
//! the person left it across redraws. A pane can put one line of its own under a row, such as
//! where reach from anywhere stands under its switch.

use crate::fields::{
    field_row, let_go, resume, secret_row as secret_row_into, still_active, typing_in, Live, Slot,
};
use adw::prelude::*;
use fermix_client::settings::{
    awaits_listing, choice_items, list_items, list_with, list_without, number_answer, number_view,
    placeholder, text_answer, text_value, value_width, ChoiceItem, Kind, Row, SectionRows,
    FIELD_CHARS, NOT_SET, UNKNOWN_KIND, WIDEST_VALUE_CHARS,
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
    live: RefCell<Live>,
    shown: RefCell<Option<Drawn>>,
}

impl SectionView {
    pub fn new(id: &str, title: Option<&str>) -> Rc<SectionView> {
        SectionView::keeping(id, title, Box::new(|_| true))
    }

    /// A view of only the rows `keep` accepts.
    pub fn keeping(id: &str, title: Option<&str>, keep: Keep) -> Rc<SectionView> {
        SectionView::build(id, title, keep, None)
    }

    /// A view of only the rows `keep` accepts, with a line of the pane's own under row `key`,
    /// hidden until `set_note` gives it words.
    pub fn keeping_with_note(id: &str, keep: Keep, key: &str) -> Rc<SectionView> {
        SectionView::build(id, None, keep, Some(Note::new(key)))
    }

    fn build(id: &str, title: Option<&str>, keep: Keep, note: Option<Note>) -> Rc<SectionView> {
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
        }
        for old in self.children.borrow_mut().drain(..) {
            self.group.remove(&old);
        }
        self.widgets.borrow_mut().clear();
        let live: Live = Rc::new(Cell::new(true));
        let fresh = match (&drawn.rows, &drawn.unread) {
            (Some(section), _) => self.row_widgets(section, &drawn, &live),
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

    /// Adds the rows to the group: the ordinary ones in order with the note under its row, then
    /// the advanced ones under one Advanced row, where there are any.
    fn place(&self, rows: &[(String, bool, gtk::Widget)]) {
        let mut children = Vec::new();
        for (key, _, widget) in rows.iter().filter(|(_, advanced, _)| !advanced) {
            children.push(widget.clone());
            if let Some(note) = self.note.as_ref().filter(|note| note.key == *key) {
                children.push(note.row.clone().upcast());
            }
        }
        let advanced: Vec<&gtk::Widget> = rows
            .iter()
            .filter(|(_, advanced, _)| *advanced)
            .map(|(_, _, widget)| widget)
            .collect();
        if !advanced.is_empty() {
            let expander = adw::ExpanderRow::builder()
                .title(ADVANCED)
                .expanded(self.advanced_open.get())
                .build();
            for widget in advanced {
                expander.add_row(widget);
            }
            children.push(expander.clone().upcast());
            self.advanced.replace(Some(expander));
        }
        for child in &children {
            self.group.add(child);
        }
        *self.children.borrow_mut() = children;
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
                let error = drawn.errors.iter().find(|(k, _)| *k == row.key);
                let widget = if drawn.listing_failed && awaits_listing(row) {
                    relist_row(&self.id, row, live).upcast()
                } else {
                    row_widget(&self.id, row, live)
                };
                if let Some((_, sentence)) = error {
                    show_refusal(&widget, sentence);
                }
                (row.key.clone(), row.advanced, widget)
            })
            .collect()
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

fn row_widget(section: &str, row: &Row, live: &Live) -> gtk::Widget {
    if row.read_only {
        return fact_row(row).upcast();
    }
    let widget: gtk::Widget = match row.kind {
        Kind::Toggle => toggle_row(section, row).upcast(),
        Kind::Choice if row.suggestions => suggestion_row(section, row, live).upcast(),
        Kind::Choice => choice_row(section, row).upcast(),
        Kind::Text => text_row(section, row, live).upcast(),
        Kind::Number => number_row(section, row).upcast(),
        Kind::Secret => secret_row(section, row, live).upcast(),
        Kind::List => list_row(section, row).upcast(),
        Kind::Unknown => unknown_row(row).upcast(),
    };
    if let Some(info) = &row.info {
        add_info(&widget, info);
    }
    widget
}

/// Sends one value for one row. The window's `setting-apply` action does the write.
fn apply(widget: &impl IsA<gtk::Widget>, section: &str, key: &str, value: &Value) {
    let target = (section, key, value.to_string()).to_variant();
    if let Err(e) = widget.activate_action("win.setting-apply", Some(&target)) {
        glib::g_warning!("fermix", "a setting could not be sent: {e}");
    }
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

fn toggle_row(section: &str, row: &Row) -> adw::SwitchRow {
    let switch = adw::SwitchRow::builder()
        .title(glib::markup_escape_text(&row.label))
        .active(row.value.as_bool().unwrap_or(false))
        .build();
    if let Some(footer) = &row.footer {
        switch.set_subtitle(&glib::markup_escape_text(footer));
    }
    let (section, key) = (section.to_owned(), row.key.clone());
    switch.connect_active_notify(move |s| apply(s, &section, &key, &Value::Bool(s.is_active())));
    switch
}

fn choice_row(section: &str, row: &Row) -> adw::ComboRow {
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
    let (section, key) = (section.to_owned(), row.key.clone());
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
        apply(combo, &section, &key, &value);
    });
    combo
}

/// Draws the chosen option in the row. The row's own label stops at 20
/// characters; this one shows the whole value when the row has room, and
/// a value too long to be kept whole says the rest in its tooltip.
fn value_factory() -> gtk::SignalListItemFactory {
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
fn option_factory(items: Rc<Vec<ChoiceItem>>) -> gtk::SignalListItemFactory {
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
fn text_field(section: &str, row: &Row, live: &Live) -> gtk::Entry {
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
    let commit = {
        let (section, row, live) = (section.to_owned(), row.clone(), live.clone());
        move |entry: &gtk::Entry| {
            let Some(value) = text_answer(&row, &entry.text()) else {
                return;
            };
            if live.get() {
                apply(entry, &section, &row.key, &value);
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

fn text_row(section: &str, row: &Row, live: &Live) -> adw::ActionRow {
    let action = titled(row);
    let entry = text_field(section, row, live);
    action.add_suffix(&entry);
    field_row(&action, &entry);
    action
}

/// A text value with the daemon's suggestions one click away; any value is allowed.
fn suggestion_row(section: &str, row: &Row, live: &Live) -> adw::ActionRow {
    linked_row(section, row, live, |entry| {
        suggestions_button(section, row, entry).upcast()
    })
}

/// A model row whose provider could not list its models: any name can still
/// be typed, and the button where the suggestions would be lists them again.
fn relist_row(section: &str, row: &Row, live: &Live) -> adw::ActionRow {
    let again = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text(LIST_AGAIN)
        .action_name("win.section-read")
        .action_target(&section.to_variant())
        .build();
    again.update_property(&[gtk::accessible::Property::Label(LIST_AGAIN)]);
    linked_row(section, row, live, |_| again.upcast())
}

/// A row's text field with one button joined to its end.
fn linked_row(
    section: &str,
    row: &Row,
    live: &Live,
    button: impl FnOnce(&gtk::Entry) -> gtk::Widget,
) -> adw::ActionRow {
    let action = titled(row);
    let entry = text_field(section, row, live);
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

fn suggestions_button(section: &str, row: &Row, entry: &gtk::Entry) -> gtk::MenuButton {
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
    let (section, key, entry, shut) = (
        section.to_owned(),
        row.key.clone(),
        entry.clone(),
        popover.clone(),
    );
    list.connect_row_activated(move |_, item| {
        let value = item.widget_name().to_string();
        shut.popdown();
        entry.set_text(&value);
        apply(&entry, &section, &key, &Value::String(value));
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

fn number_row(section: &str, row: &Row) -> adw::SpinRow {
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
    write_when_still(&spin, section, row);
    spin
}

/// Writes the number once the control has been still for `NUMBER_SETTLE`,
/// and only when it differs from the daemon's value.
fn write_when_still(spin: &adw::SpinRow, section: &str, row: &Row) {
    let (section, row, pending) = (section.to_owned(), row.clone(), Rc::new(Cell::new(0u64)));
    spin.connect_value_notify(move |spin| {
        let ticket = pending.get() + 1;
        pending.set(ticket);
        let (spin, section, row, pending) =
            (spin.clone(), section.clone(), row.clone(), pending.clone());
        glib::timeout_add_local_once(NUMBER_SETTLE, move || {
            if pending.get() != ticket {
                return;
            }
            let value = number_answer(&row, spin.value());
            if value.as_f64() != row.value.as_f64() {
                apply(&spin, &section, &row.key, &value);
            }
        });
    });
}

/// A secret row whose store and remove go to the window's `secret-set` and
/// `secret-remove` actions, carrying the section and the row key.
fn secret_row(section: &str, row: &Row, live: &Live) -> adw::ActionRow {
    let action = titled(row);
    let target = Rc::new((section.to_owned(), row.key.clone()));
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
    action
}

/// A list is written whole: each change sends the full replacement.
fn list_row(section: &str, row: &Row) -> adw::ExpanderRow {
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
        let (section, key, next) = (
            section.to_owned(),
            row.key.clone(),
            list_without(&items, item),
        );
        remove.connect_clicked(move |b| apply(b, &section, &key, &Value::from(next.clone())));
        line.add_suffix(&remove);
        expander.add_row(&line);
    }
    expander.add_row(&list_adder(section, &row.key, items));
    expander
}

fn list_adder(section: &str, key: &str, items: Vec<String>) -> adw::EntryRow {
    let adder = adw::EntryRow::builder()
        .title("Add")
        .show_apply_button(true)
        .build();
    let (section, key) = (section.to_owned(), key.to_owned());
    adder.connect_apply(move |adder| {
        if let Some(next) = list_with(&items, &adder.text()) {
            apply(adder, &section, &key, &Value::from(next));
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
    let more = gtk::MenuButton::builder()
        .icon_name("dialog-information-symbolic")
        .tooltip_text("More about this setting")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .popover(&popover)
        .build();
    if let Some(row) = widget.downcast_ref::<adw::ActionRow>() {
        row.add_suffix(&more);
    } else if let Some(row) = widget.downcast_ref::<adw::ComboRow>() {
        row.add_suffix(&more);
    }
}
