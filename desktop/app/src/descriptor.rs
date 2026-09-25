//! One daemon settings section drawn as a preferences group. Each row kind has
//! one control; every write leaves as a window action carrying the section, the
//! row key and the JSON value, so the controller in `settings_flow.rs` does the
//! talking. A section is redrawn only when what it shows changed.

use adw::prelude::*;
use fermix_client::settings::{
    choice_items, list_items, list_with, list_without, number_answer, number_view, placeholder,
    text_answer, text_value, ChoiceItem, Kind, Row, SectionRows, NOT_SET, UNKNOWN_KIND,
};
use gtk::glib::{self, variant::ToVariant};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// A number is written once its control has been still this long, not on every click.
const NUMBER_SETTLE: Duration = Duration::from_millis(700);

/// Everything a section's rows are drawn from.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    /// `None` until the section has been read once.
    pub rows: Option<SectionRows>,
    /// The daemon's refusal sentence, by row key.
    pub errors: Vec<(String, String)>,
    /// Set while the settings file changed outside Fermix: nothing may be written.
    pub locked: bool,
}

/// Which of a section's rows a view draws; the rest belong to other controls.
pub type Keep = Box<dyn Fn(&str) -> bool>;

pub struct SectionView {
    pub id: String,
    pub group: adw::PreferencesGroup,
    keep: Keep,
    widgets: RefCell<Vec<gtk::Widget>>,
    shown: RefCell<Option<Drawn>>,
}

impl SectionView {
    pub fn new(id: &str, title: Option<&str>) -> Rc<SectionView> {
        SectionView::keeping(id, title, Box::new(|_| true))
    }

    /// A view of only the rows `keep` accepts.
    pub fn keeping(id: &str, title: Option<&str>, keep: Keep) -> Rc<SectionView> {
        let group = adw::PreferencesGroup::new();
        if let Some(title) = title {
            group.set_title(&glib::markup_escape_text(title));
        }
        Rc::new(SectionView {
            id: id.to_owned(),
            group,
            keep,
            widgets: RefCell::default(),
            shown: RefCell::default(),
        })
    }

    pub fn show(&self, drawn: Drawn) {
        if self.shown.borrow().as_ref() == Some(&drawn) {
            return;
        }
        for old in self.widgets.borrow_mut().drain(..) {
            self.group.remove(&old);
        }
        let fresh = match &drawn.rows {
            None => vec![loading_row()],
            Some(section) => self.row_widgets(section, &drawn.errors),
        };
        for widget in &fresh {
            self.group.add(widget);
        }
        self.group.set_sensitive(!drawn.locked);
        *self.widgets.borrow_mut() = fresh;
        *self.shown.borrow_mut() = Some(drawn);
    }

    fn row_widgets(&self, section: &SectionRows, errors: &[(String, String)]) -> Vec<gtk::Widget> {
        section
            .rows
            .iter()
            .filter(|row| (self.keep)(&row.key))
            .map(|row| {
                let error = errors.iter().find(|(k, _)| *k == row.key).map(|(_, s)| s);
                let widget = row_widget(&self.id, row);
                if let Some(sentence) = error {
                    show_refusal(&widget, sentence);
                }
                widget
            })
            .collect()
    }
}

fn loading_row() -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .title("Reading from Fermix…")
        .build();
    row.add_suffix(&adw::Spinner::new());
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

fn row_widget(section: &str, row: &Row) -> gtk::Widget {
    if row.read_only {
        return fact_row(row).upcast();
    }
    let widget: gtk::Widget = match row.kind {
        Kind::Toggle => toggle_row(section, row).upcast(),
        Kind::Choice if row.suggestions => suggestion_row(section, row).upcast(),
        Kind::Choice => choice_row(section, row).upcast(),
        Kind::Text => text_row(section, row).upcast(),
        Kind::Number => number_row(section, row).upcast(),
        Kind::Secret => secret_row(section, row).upcast(),
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

/// Draws each option in the open list as its label over its hint.
fn option_factory(items: Rc<Vec<ChoiceItem>>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, object| {
        let item = object.downcast_ref::<gtk::ListItem>().expect("a list item");
        let label = gtk::Label::builder().xalign(0.0).build();
        let hint = gtk::Label::builder()
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
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
/// when the text changed; Escape puts the daemon's value back.
fn text_field(section: &str, row: &Row) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .text(text_value(row))
        .placeholder_text(placeholder(row))
        .valign(gtk::Align::Center)
        .width_chars(18)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(&row.label)]);
    let commit = {
        let (section, row) = (section.to_owned(), row.clone());
        move |entry: &gtk::Entry| {
            if let Some(value) = text_answer(&row, &entry.text()) {
                apply(entry, &section, &row.key, &value);
            }
        }
    };
    let on_leave = commit.clone();
    entry.connect_activate(commit);
    let focus = gtk::EventControllerFocus::new();
    let watched = entry.clone();
    focus.connect_leave(move |_| on_leave(&watched));
    entry.add_controller(focus);
    restore_on_escape(&entry, text_value(row));
    entry
}

fn restore_on_escape(entry: &gtk::Entry, daemon_value: String) {
    let keys = gtk::EventControllerKey::new();
    let target = entry.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key != gtk::gdk::Key::Escape {
            return glib::Propagation::Proceed;
        }
        target.set_text(&daemon_value);
        glib::Propagation::Stop
    });
    entry.add_controller(keys);
}

fn text_row(section: &str, row: &Row) -> adw::ActionRow {
    let action = titled(row);
    action.add_suffix(&text_field(section, row));
    action
}

/// A text value with the daemon's suggestions one click away; any value is allowed.
fn suggestion_row(section: &str, row: &Row) -> adw::ActionRow {
    let action = titled(row);
    let entry = text_field(section, row);
    let linked = gtk::Box::builder()
        .css_classes(["linked"])
        .valign(gtk::Align::Center)
        .build();
    linked.append(&entry);
    linked.append(&suggestions_button(section, row, &entry));
    action.add_suffix(&linked);
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
    gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .popover(&popover)
        .tooltip_text("Suggestions")
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

/// A secret never shows its value: only whether one is stored, with ways to add,
/// replace or remove it. The dialog that takes the value is the controller's.
fn secret_row(section: &str, row: &Row) -> adw::ActionRow {
    let action = titled(row);
    let present = row.present.unwrap_or(false);
    let state = if present { "Stored" } else { "Not stored" };
    let target = (section, row.key.as_str()).to_variant();
    let status = gtk::Label::builder()
        .label(state)
        .css_classes(["dim-label"])
        .build();
    action.add_suffix(&status);
    let verb = if present { "Replace…" } else { "Add…" };
    let add = button(verb, "win.secret-add", &target);
    add.update_property(&[gtk::accessible::Property::Label(&format!(
        "{verb} {}",
        row.label
    ))]);
    action.add_suffix(&add);
    if present {
        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text(format!("Remove {}", row.label))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        remove.set_action_name(Some("win.secret-remove"));
        remove.set_action_target_value(Some(&target));
        action.add_suffix(&remove);
    }
    action
}

fn button(label: &str, action: &str, target: &glib::Variant) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build();
    button.set_action_name(Some(action));
    button.set_action_target_value(Some(target));
    button
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
        .css_classes(["dim-label"])
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
        .icon_name("help-about-symbolic")
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
