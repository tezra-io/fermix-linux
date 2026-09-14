//! One descriptor row, in one control.
//!
//! Six kinds, one commit rule each, and the rule is the redlines':
//!
//! - a toggle and a closed choice commit on selection;
//! - text, a number and a list item commit on Enter and on deliberate focus
//!   loss, and only when the value actually changed;
//! - Escape puts the daemon's value back, sends nothing, and does not leave
//!   Settings;
//! - a refusal puts the daemon's value back and keeps the daemon's own sentence
//!   under the row;
//! - a refresh landing underneath an edit does not disturb it.
//!
//! The last one is why nothing here rebuilds a control: a row is built once per
//! descriptor shape and updated in place.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{
    SettingValue, SettingsNumberFormat, SettingsOption, SettingsRow, SettingsRowKind,
};
use crate::models::{spawn, SettingsModel};
use crate::ui::CaptionRow;

use super::dialogs::secret::SecretDialog;

/// Above this many suggestions the list is searched rather than scrolled.
const SEARCHABLE_ABOVE: usize = 12;

/// How a pane answers a choice the daemon could not inline.
///
/// The rule keys on the row's own shape and never on its key: a choice that is
/// not read-only and publishes no options is a listing too large for the wire,
/// and the pane that drew the row is what knows which picker answers it. The
/// word on the button is the pane's too, so this component carries no wording
/// for a listing it does not know the subject of.
#[derive(Clone)]
pub struct OpenChoice {
    pub label: Key,
    #[allow(clippy::type_complexity)]
    pub open: Rc<dyn Fn(&str, &str, Option<String>)>,
}

/// Whether one row is a choice the wire does not carry the whole of.
///
/// Two shapes say so and both key on the row rather than on its key: a choice
/// with no options at all, and an open choice, whose published options are
/// hints rather than the whole set. Either way the complete list is somewhere
/// else, and only the pane knows where.
pub fn needs_a_picker(row: &SettingsRow) -> bool {
    row.kind == SettingsRowKind::Choice
        && !row.read_only
        && (row.options.is_empty() || row.suggestions)
}

/// Where one row's widgets go.
pub enum Placement {
    /// Rows to add to the section's own group, in order.
    InGroup(Vec<gtk::Widget>),
    /// A group of its own, for a row that is a list of things.
    OwnGroup(adw::PreferencesGroup),
}

/// The control behind one row.
enum Control {
    Toggle(adw::SwitchRow),
    /// A closed choice, and the values behind the words.
    Choice(adw::ComboRow, Vec<String>),
    /// An open choice or a plain text value: an entry, with the daemon's
    /// suggestions beside it where it published any, and the label it gives
    /// the value in force for the one case where an entry shows nothing.
    Entry(adw::EntryRow, gtk::Label),
    Number(adw::SpinRow),
    Secret(adw::ActionRow, gtk::Button, gtk::Button),
    /// A closed choice the daemon published no options for, which is a listing
    /// too large to inline. The row shows the value in force and the owner of
    /// the pane answers it with a dialog of its own.
    OpenChoice(adw::ActionRow, gtk::Button),
    List(
        adw::PreferencesGroup,
        RefCell<Vec<adw::EntryRow>>,
        adw::ButtonRow,
    ),
    /// A fact the daemon declared read-only. Never a control that cannot work.
    ReadOnly(adw::ActionRow, gtk::Label),
}

/// One row of one section.
pub struct DescriptorRow {
    settings: Rc<SettingsModel>,
    section: String,
    key: String,
    control: Control,
    caption: CaptionRow,
    /// The footer the daemon published, which is what the caption says when
    /// there is no refusal to say instead.
    footer: Option<String>,
    /// The options the daemon published, for a choice read back as a word.
    options: Vec<SettingsOption>,
    /// Whether the secret behind this row is stored, as the daemon last said.
    present: Cell<bool>,
    /// Set while this row is writing a value into its own control, so the
    /// control's handler can tell a person's gesture from a refresh.
    updating: Cell<bool>,
    /// How this pane answers a listing the daemon could not inline.
    picker: Option<OpenChoice>,
}

impl DescriptorRow {
    /// Build the control for one row.
    pub fn build(
        settings: Rc<SettingsModel>,
        section: &str,
        row: &SettingsRow,
        picker: Option<OpenChoice>,
    ) -> Rc<Self> {
        let this = Rc::new(Self {
            settings,
            section: section.to_string(),
            key: row.key.clone(),
            control: build_control(row, picker.as_ref()),
            caption: CaptionRow::new(),
            footer: row.footer.clone(),
            options: row.options.clone(),
            present: Cell::new(row.present.unwrap_or(false)),
            updating: Cell::new(false),
            picker,
        });

        this.connect(row);
        this.update(row);
        this
    }

    /// The key this row writes.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Where this row's widgets go.
    pub fn placement(&self) -> Placement {
        match &self.control {
            Control::Toggle(row) => self.in_group(row.clone().upcast()),
            Control::Choice(row, _) => self.in_group(row.clone().upcast()),
            Control::Entry(row, _) => self.in_group(row.clone().upcast()),
            Control::Number(row) => self.in_group(row.clone().upcast()),
            Control::Secret(row, _, _) => self.in_group(row.clone().upcast()),
            Control::OpenChoice(row, _) => self.in_group(row.clone().upcast()),
            Control::ReadOnly(row, _) => self.in_group(row.clone().upcast()),
            Control::List(group, _, _) => Placement::OwnGroup(group.clone()),
        }
    }

    fn in_group(&self, control: gtk::Widget) -> Placement {
        Placement::InGroup(vec![control, self.caption.row().clone().upcast()])
    }

    /// Write the daemon's row into the control, without the control mistaking
    /// it for a gesture.
    ///
    /// A control a person is typing in is left exactly as it is: that is the
    /// "a refresh cannot overwrite an active edit" rule, and it is enforced
    /// here rather than by asking the model not to refresh.
    pub fn update(self: &Rc<Self>, row: &SettingsRow) {
        self.updating.set(true);
        self.write(row);
        self.updating.set(false);
        self.draw_caption();
    }

    fn write(self: &Rc<Self>, row: &SettingsRow) {
        let usable = !row.read_only && !self.is_busy();

        match &self.control {
            Control::Toggle(switch) => {
                switch.set_active(as_bool(&row.value));
                switch.set_sensitive(usable);
            }
            Control::Choice(combo, values) => {
                if let Some(index) = values
                    .iter()
                    .position(|value| *value == as_text(&row.value))
                {
                    combo.set_selected(index as u32);
                }
                combo.set_sensitive(usable);
            }
            Control::Entry(entry, standing) => {
                if !entry.has_focus() {
                    entry.set_text(&as_text(&row.value));
                }
                entry.set_sensitive(usable);
                // An entry with nothing in it reads as a value nobody set. The
                // daemon says what an empty value means wherever it publishes
                // an option for it, so that word stands where the value would,
                // in the daemon's own words, and leaves the moment there is a
                // value to read.
                let word = self.word_for(&row.value);
                let empty = entry.text().is_empty();
                crate::ui::set_value(standing, if empty { word.as_str() } else { "" });
                standing.set_visible(empty && !word.is_empty());
            }
            Control::Number(spin) => {
                if !spin.has_focus() {
                    spin.set_value(as_number(&row.value));
                }
                spin.set_sensitive(usable);
            }
            Control::Secret(widget, add, remove) => {
                let present = row.present.unwrap_or(false);
                self.present.set(present);
                widget.set_subtitle(&if present {
                    copy::text(Key::SecretStored)
                } else {
                    String::new()
                });
                add.set_label(&copy::text(if present {
                    Key::SecretReplace
                } else {
                    Key::SecretAdd
                }));
                add.set_sensitive(!self.is_busy());
                remove.set_visible(present);
                remove.set_sensitive(!self.is_busy());
            }
            Control::OpenChoice(widget, button) => {
                widget.set_subtitle(&self.word_for(&row.value));
                button.set_sensitive(usable && !self.is_busy());
            }
            Control::List(group, entries, add) => self.write_list(group, entries, add, row),
            Control::ReadOnly(_, value) => crate::ui::set_value(value, &self.word_for(&row.value)),
        }
    }

    /// A value as the daemon words it: a choice's own label where it published
    /// one, and the value itself otherwise.
    fn word_for(&self, value: &SettingValue) -> String {
        let text = as_text(value);
        self.options
            .iter()
            .find(|option| option.value == text)
            .map(|option| option.label.clone())
            .unwrap_or(text)
    }

    /// A list row is as many entries as the value has. An entry a person is
    /// typing in is never taken away from them.
    fn write_list(
        self: &Rc<Self>,
        group: &adw::PreferencesGroup,
        entries: &RefCell<Vec<adw::EntryRow>>,
        add: &adw::ButtonRow,
        row: &SettingsRow,
    ) {
        if entries.borrow().iter().any(|entry| entry.has_focus()) {
            return;
        }

        let items = as_list(&row.value);
        {
            let mut held = entries.borrow_mut();
            for entry in held.drain(..) {
                group.remove(&entry);
            }
        }

        // The row that adds another item stays at the foot of the list, so it
        // is taken out and put back around the items rather than drifting into
        // the middle of them.
        group.remove(add);
        for item in &items {
            self.append_item(group, entries, item, false);
        }
        group.add(add);
    }

    fn is_busy(&self) -> bool {
        self.settings
            .state()
            .is_busy(&(self.section.clone(), self.key.clone()))
    }

    /// The caption under the row: the daemon's refusal where there is one, and
    /// the daemon's footer otherwise.
    fn draw_caption(&self) {
        let id = (self.section.clone(), self.key.clone());
        let refusal = self
            .settings
            .state()
            .refusal(&id)
            .map(|sentence| sentence.text.clone());

        match refusal {
            Some(text) => self.caption.set(Some(&text)),
            // A footer that already rides on the control as a subtitle is not
            // repeated underneath it.
            None if self.carries_subtitle() => self.caption.set(None),
            None => self.caption.set(self.footer.as_deref()),
        }
    }

    fn carries_subtitle(&self) -> bool {
        matches!(
            self.control,
            Control::Toggle(_)
                | Control::Choice(_, _)
                | Control::Number(_)
                | Control::List(_, _, _)
                | Control::OpenChoice(_, _)
        )
    }

    // ---- Commit ---------------------------------------------------------

    fn connect(self: &Rc<Self>, row: &SettingsRow) {
        match &self.control {
            Control::Toggle(switch) => {
                let owner = Rc::clone(self);
                switch.connect_active_notify(move |switch| {
                    if owner.updating.get() {
                        return;
                    }
                    owner.commit(SettingValue::Toggle(switch.is_active()));
                });
            }
            Control::Choice(combo, values) => {
                let owner = Rc::clone(self);
                let values = values.clone();
                combo.connect_selected_notify(move |combo| {
                    if owner.updating.get() {
                        return;
                    }
                    let Some(value) = values.get(combo.selected() as usize) else {
                        return;
                    };
                    owner.commit(SettingValue::Text(value.clone()));
                });
            }
            Control::Entry(entry, _) => {
                self.connect_entry(entry);
                // The pane's picker replaces the hints menu rather than sitting
                // beside it: both answer the same question, and the picker
                // answers it with the whole list rather than with the hints.
                match self.picker.clone() {
                    Some(picker) if needs_a_picker(row) => {
                        entry.add_suffix(&self.picker_button(picker.label))
                    }
                    _ if !row.options.is_empty() => entry.add_suffix(&self.suggestions_button(row)),
                    _ => {}
                }
            }
            Control::Number(spin) => self.connect_number(spin),
            Control::Secret(_, add, remove) => self.connect_secret(add, remove),
            Control::OpenChoice(_, button) => self.connect_picker(button),
            Control::List(group, _, add) => self.connect_list(group, add),
            Control::ReadOnly(_, _) => {}
        }
    }

    /// Enter commits, focus loss commits a changed value, Escape restores.
    fn connect_entry(self: &Rc<Self>, entry: &adw::EntryRow) {
        {
            let owner = Rc::clone(self);
            entry.connect_entry_activated(move |entry| owner.commit_text(entry.text().as_str()));
        }

        {
            let owner = Rc::clone(self);
            entry.connect_changed(move |entry| {
                if owner.updating.get() {
                    return;
                }
                owner.settings.set_draft(
                    &owner.section,
                    &owner.key,
                    SettingValue::Text(entry.text().to_string()),
                );
            });
        }

        {
            let owner = Rc::clone(self);
            entry.connect_has_focus_notify(move |entry| {
                if entry.has_focus() || owner.updating.get() {
                    return;
                }
                owner.commit_text(entry.text().as_str());
            });
        }

        self.install_escape(entry);
    }

    fn connect_number(self: &Rc<Self>, spin: &adw::SpinRow) {
        {
            let owner = Rc::clone(self);
            spin.connect_has_focus_notify(move |spin| {
                if spin.has_focus() || owner.updating.get() {
                    return;
                }
                owner.commit_number(spin.value());
            });
        }

        {
            let owner = Rc::clone(self);
            spin.connect_changed(move |spin| {
                if owner.updating.get() {
                    return;
                }
                owner.settings.set_draft(
                    &owner.section,
                    &owner.key,
                    SettingValue::Number(spin.value()),
                );
            });
        }

        self.install_escape(spin);
    }

    /// The button that opens the pane's own picker beside an open choice.
    fn picker_button(self: &Rc<Self>, label: Key) -> gtk::Button {
        let button = gtk::Button::builder()
            .label(copy::text(label))
            .valign(gtk::Align::Center)
            .build();
        button.add_css_class("flat");
        self.connect_picker(&button);
        button
    }

    /// The picker is the pane's, and it is handed the value in force so the
    /// dialog opens on what is actually selected.
    fn connect_picker(self: &Rc<Self>, button: &gtk::Button) {
        let owner = Rc::clone(self);
        button.connect_clicked(move |_| {
            let Some(picker) = owner.picker.as_ref() else {
                return;
            };
            let current = owner
                .settings
                .state()
                .daemon_value(&owner.section, &owner.key)
                .map(|value| as_text(&value))
                .filter(|value| !value.is_empty());

            (picker.open)(&owner.section, &owner.key, current);
        });
    }

    fn connect_secret(self: &Rc<Self>, add: &gtk::Button, remove: &gtk::Button) {
        {
            let owner = Rc::clone(self);
            add.connect_clicked(move |button| {
                SecretDialog::present(
                    Rc::clone(&owner.settings),
                    &owner.section,
                    &owner.key,
                    owner.present.get(),
                    button,
                );
            });
        }

        let owner = Rc::clone(self);
        remove.connect_clicked(move |_| {
            let settings = Rc::clone(&owner.settings);
            let (section, key) = (owner.section.clone(), owner.key.clone());
            spawn(async move {
                // The refusal is recorded under the row by the model, which is
                // where a person is looking when they pressed this.
                let _ = settings.clear_secret(&section, &key).await;
            });
        });
    }

    /// A list commits the whole value: the wire takes a list, not an item.
    fn connect_list(self: &Rc<Self>, group: &adw::PreferencesGroup, add: &adw::ButtonRow) {
        let owner = Rc::clone(self);
        add.connect_activated(move |_| owner.add_item());
        group.add(add);
    }

    fn add_item(self: &Rc<Self>) {
        let Control::List(group, entries, add) = &self.control else {
            return;
        };

        // An unsubmitted addition is a draft: it survives leaving the pane and
        // is never sent until it says something.
        let mut items = self.current_list();
        items.push(String::new());
        self.settings
            .set_draft(&self.section, &self.key, SettingValue::List(items));

        group.remove(add);
        self.append_item(group, entries, "", true);
        group.add(add);
    }

    /// One entry of a list, wired to commit the whole list.
    fn append_item(
        self: &Rc<Self>,
        group: &adw::PreferencesGroup,
        entries: &RefCell<Vec<adw::EntryRow>>,
        item: &str,
        focus: bool,
    ) {
        let entry = adw::EntryRow::builder().title("").text(item).build();

        let remove = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .valign(gtk::Align::Center)
            .build();
        remove.add_css_class("flat");
        remove.update_property(&[gtk::accessible::Property::Label(&copy::text(
            Key::ListRemoveAccessible,
        ))]);
        entry.add_suffix(&remove);

        {
            let owner = Rc::clone(self);
            entry.connect_entry_activated(move |_| owner.commit_list());
        }
        {
            let owner = Rc::clone(self);
            entry.connect_has_focus_notify(move |entry| {
                if entry.has_focus() || owner.updating.get() {
                    return;
                }
                owner.commit_list();
            });
        }
        {
            let owner = Rc::clone(self);
            let entry = entry.clone();
            remove.connect_clicked(move |_| owner.remove_item(&entry));
        }

        group.add(&entry);
        entries.borrow_mut().push(entry.clone());

        if focus {
            entry.grab_focus();
        }
    }

    fn remove_item(self: &Rc<Self>, entry: &adw::EntryRow) {
        let Control::List(group, entries, _) = &self.control else {
            return;
        };

        entries.borrow_mut().retain(|held| held != entry);
        group.remove(entry);
        self.commit_list();
    }

    /// The list as the entries now stand.
    fn current_list(&self) -> Vec<String> {
        let Control::List(_, entries, _) = &self.control else {
            return Vec::new();
        };

        entries
            .borrow()
            .iter()
            .map(|entry| entry.text().to_string())
            .collect()
    }

    /// The daemon's suggestions, beside an entry that also takes anything else.
    fn suggestions_button(self: &Rc<Self>, row: &SettingsRow) -> gtk::MenuButton {
        let button = gtk::MenuButton::builder()
            .icon_name("pan-down-symbolic")
            .valign(gtk::Align::Center)
            .build();
        button.add_css_class("flat");
        button.update_property(&[gtk::accessible::Property::Label(&row.label)]);

        let list = suggestion_list(&row.options);
        let search = gtk::SearchEntry::builder()
            // A short list is read; a long one is searched. The count is the
            // daemon's, so nothing here keys on which row this is.
            .visible(row.options.len() > SEARCHABLE_ABOVE)
            .placeholder_text(copy::text(Key::ActionSearchAccessible))
            .build();
        connect_suggestion_search(&search, &list);

        {
            // A row keeps its index while it is filtered, so the index is what
            // says which value was chosen.
            let values: Vec<String> = row
                .options
                .iter()
                .map(|option| option.value.clone())
                .collect();
            let owner = Rc::clone(self);
            let button = button.clone();
            list.connect_row_activated(move |_, chosen| {
                let Some(value) = values.get(chosen.index().max(0) as usize) else {
                    return;
                };

                owner.write_text(value);
                owner.commit_text(value);
                button.popdown();
            });
        }

        button.set_popover(Some(&suggestion_popover(&search, &list)));
        button
    }

    fn write_text(&self, value: &str) {
        let Control::Entry(entry, _) = &self.control else {
            return;
        };
        self.updating.set(true);
        entry.set_text(value);
        self.updating.set(false);
    }

    /// Escape restores the daemon's value and sends nothing.
    ///
    /// It stops there only when there was something to restore: with no edit in
    /// hand, Escape is the window's, and leaves Settings.
    fn install_escape(self: &Rc<Self>, widget: &impl IsA<gtk::Widget>) {
        let controller = gtk::EventControllerKey::new();
        let owner = Rc::clone(self);

        controller.connect_key_pressed(move |_, key, _, _| {
            if key != gtk::gdk::Key::Escape {
                return glib::Propagation::Proceed;
            }
            if owner.revert() {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });

        widget.as_ref().add_controller(controller);
    }

    /// Put the daemon's value back in the control. Answers whether there was
    /// anything to put back.
    pub fn revert(&self) -> bool {
        let Some(value) = self.settings.state().daemon_value(&self.section, &self.key) else {
            return false;
        };

        let had_draft = self.settings.draft(&self.section, &self.key).is_some();
        self.settings.discard_draft(&self.section, &self.key);

        self.updating.set(true);
        let differs = match &self.control {
            Control::Entry(entry, _) => {
                let restored = as_text(&value);
                let differs = entry.text() != restored;
                entry.set_text(&restored);
                differs
            }
            Control::Number(spin) => {
                let restored = as_number(&value);
                let differs = (spin.value() - restored).abs() > f64::EPSILON;
                spin.set_value(restored);
                differs
            }
            _ => false,
        };
        self.updating.set(false);

        had_draft || differs
    }

    fn commit_text(&self, text: &str) {
        if self.unchanged(&SettingValue::Text(text.to_string())) {
            self.settings.discard_draft(&self.section, &self.key);
            return;
        }
        self.commit(SettingValue::Text(text.to_string()));
    }

    fn commit_number(&self, value: f64) {
        if self.unchanged(&SettingValue::Number(value)) {
            self.settings.discard_draft(&self.section, &self.key);
            return;
        }
        self.commit(SettingValue::Number(value));
    }

    /// Whether a value is what the daemon already has, in which case there is
    /// nothing to send.
    fn unchanged(&self, value: &SettingValue) -> bool {
        let Some(current) = self.settings.state().daemon_value(&self.section, &self.key) else {
            return false;
        };

        match (&current, value) {
            (SettingValue::Number(held), SettingValue::Number(typed)) => {
                (held - typed).abs() < f64::EPSILON
            }
            _ => as_text(&current) == as_text(value),
        }
    }

    fn commit(&self, value: SettingValue) {
        let settings = Rc::clone(&self.settings);
        let section = self.section.clone();
        let key = self.key.clone();

        spawn(async move {
            settings.apply(&section, &key, value).await;
        });
    }

    /// Commit the whole list, which is what a list entry's Enter does.
    pub fn commit_list(&self) {
        let items: Vec<String> = self
            .current_list()
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();

        if self.unchanged(&SettingValue::List(items.clone())) {
            self.settings.discard_draft(&self.section, &self.key);
            return;
        }

        self.commit(SettingValue::List(items));
    }
}

/// The daemon's options, as a list somebody can pick from.
fn suggestion_list(options: &[SettingsOption]) -> gtk::ListBox {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");

    for option in options {
        list.append(
            &adw::ActionRow::builder()
                .title(option.label.as_str())
                .subtitle(option.hint.clone().unwrap_or_default())
                .activatable(true)
                .build(),
        );
    }

    list
}

/// Filtering the suggestions hides rows rather than rebuilding them, so an
/// option keeps the index its value is found by.
fn connect_suggestion_search(search: &gtk::SearchEntry, list: &gtk::ListBox) {
    let list = list.clone();
    search.connect_search_changed(move |entry| {
        let needle = entry.text().to_lowercase();
        let mut child = list.first_child();

        while let Some(widget) = child {
            if let Some(row) = widget.downcast_ref::<adw::ActionRow>() {
                let title = row.title().to_lowercase();
                let subtitle = row.subtitle().unwrap_or_default().to_lowercase();
                row.set_visible(
                    needle.is_empty() || title.contains(&needle) || subtitle.contains(&needle),
                );
            }
            child = widget.next_sibling();
        }
    });
}

fn suggestion_popover(search: &gtk::SearchEntry, list: &gtk::ListBox) -> gtk::Popover {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(crate::metrics::SPACE_TIGHT)
        .build();

    column.append(search);
    column.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(crate::metrics::CLAMP_TIGHTENING)
            .propagate_natural_height(true)
            .child(list)
            .build(),
    );

    gtk::Popover::builder().child(&column).build()
}

/// Build the control one row kind asks for.
fn build_control(row: &SettingsRow, picker: Option<&OpenChoice>) -> Control {
    if row.read_only {
        return read_only(row);
    }

    // A closed choice with nothing to choose from is a control that cannot
    // work: the pane's own picker answers it, and where the pane offers none
    // the row states the value in force. An open choice keeps its entry either
    // way, because a person may type a value that is on no list.
    if needs_a_picker(row) && !row.suggestions {
        return match picker {
            Some(picker) => open_choice(row, picker.label),
            None => read_only(row),
        };
    }

    match row.kind {
        SettingsRowKind::Toggle => Control::Toggle(
            adw::SwitchRow::builder()
                .title(row.label.as_str())
                .subtitle(row.footer.clone().unwrap_or_default())
                .build(),
        ),
        // An open choice takes an off-list value, so it is an entry with the
        // daemon's suggestions beside it rather than a list that refuses.
        SettingsRowKind::Choice if row.suggestions => entry_control(row),
        SettingsRowKind::Choice => {
            let words: Vec<&str> = row
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect();
            let values: Vec<String> = row
                .options
                .iter()
                .map(|option| option.value.clone())
                .collect();

            let combo = adw::ComboRow::builder()
                .title(row.label.as_str())
                .subtitle(row.footer.clone().unwrap_or_default())
                .model(&gtk::StringList::new(&words))
                .build();
            Control::Choice(combo, values)
        }
        SettingsRowKind::Text => entry_control(row),
        SettingsRowKind::Number => Control::Number(number_row(row)),
        SettingsRowKind::Secret => secret_row(row),
        SettingsRowKind::List => list_group(row),
        // A kind this build has never seen renders as the fact it is. A control
        // guessed at from an unknown kind would write the wrong shape.
        SettingsRowKind::Unrecognized => read_only(row),
    }
}

/// A listing too large to inline: the value in force, and the pane's own way in.
fn open_choice(row: &SettingsRow, label: Key) -> Control {
    let widget = adw::ActionRow::builder()
        .title(row.label.as_str())
        .activatable(false)
        .build();

    let button = gtk::Button::builder()
        .label(copy::text(label))
        .valign(gtk::Align::Center)
        .build();
    widget.add_suffix(&button);

    Control::OpenChoice(widget, button)
}

/// A secret is a row that says whether one is stored, and two verbs. The value
/// itself never appears here, and never leaves the one dialog that takes it.
fn secret_row(row: &SettingsRow) -> Control {
    let widget = adw::ActionRow::builder()
        .title(row.label.as_str())
        .activatable(false)
        .build();

    let remove = gtk::Button::builder()
        .label(copy::text(Key::SecretRemove))
        .valign(gtk::Align::Center)
        .build();
    let add = gtk::Button::builder()
        .label(copy::text(Key::SecretAdd))
        .valign(gtk::Align::Center)
        .build();

    widget.add_suffix(&remove);
    widget.add_suffix(&add);
    Control::Secret(widget, add, remove)
}

/// A list is a group of its own: one entry per item, and the row that adds
/// another at its foot.
fn list_group(row: &SettingsRow) -> Control {
    let group = adw::PreferencesGroup::builder()
        .title(row.label.as_str())
        .description(row.footer.clone().unwrap_or_default())
        .build();
    let add = adw::ButtonRow::builder()
        .title(copy::text(Key::ListAdd))
        .start_icon_name("list-add-symbolic")
        .build();

    Control::List(group, RefCell::new(Vec::new()), add)
}

/// A fact, with the value beside it and nothing to press.
fn read_only(row: &SettingsRow) -> Control {
    let widget = adw::ActionRow::builder()
        .title(row.label.as_str())
        .subtitle(row.footer.clone().unwrap_or_default())
        .activatable(false)
        .build();
    let value = crate::ui::value_label("");
    widget.add_suffix(&value);
    Control::ReadOnly(widget, value)
}

fn entry_control(row: &SettingsRow) -> Control {
    let entry = adw::EntryRow::builder().title(row.label.as_str()).build();

    // Where a value would sit, for the one case where the entry is empty and
    // the daemon has published a word for what empty means. Built with the row
    // rather than hunted for in the tree afterwards.
    let standing = crate::ui::value_label("");
    standing.set_visible(false);
    entry.add_suffix(&standing);

    Control::Entry(entry, standing)
}

/// The bounds, the step and the digits are the daemon's.
fn number_row(row: &SettingsRow) -> adw::SpinRow {
    let step = row.step.unwrap_or(1.0).abs().max(f64::MIN_POSITIVE);
    let adjustment = gtk::Adjustment::new(
        as_number(&row.value),
        row.min.unwrap_or(0.0),
        row.max.unwrap_or(f64::from(i32::MAX)),
        step,
        step,
        0.0,
    );

    let spin = adw::SpinRow::builder()
        .title(row.label.as_str())
        .subtitle(row.footer.clone().unwrap_or_default())
        .adjustment(&adjustment)
        .digits(digits(step))
        .build();

    if row.format == Some(SettingsNumberFormat::Percent) {
        show_as_percent(&spin);
    }
    if let Some(unit) = row.unit.as_ref() {
        spin.add_suffix(&crate::ui::value_label(unit));
    }

    spin
}

/// How many decimals a step needs to be typed exactly.
fn digits(step: f64) -> u32 {
    let mut digits = 0;
    let mut scaled = step;
    while scaled.fract().abs() > 1e-9 && digits < 6 {
        scaled *= 10.0;
        digits += 1;
    }
    digits
}

/// A fraction the daemon stores as a fraction, read as a percentage.
///
/// The value on the wire never changes: this is the display and the typing,
/// which is why both halves are here rather than one.
fn show_as_percent(spin: &adw::SpinRow) {
    let Some(button) = find_spin_button(spin.clone().upcast()) else {
        return;
    };

    button.connect_output(|button| {
        let shown = (button.value() * 100.0).round();
        button.set_text(&format!("{shown}%"));
        glib::Propagation::Stop
    });

    button.connect_input(|button| {
        let typed: String = button
            .text()
            .chars()
            .filter(|character| character.is_ascii_digit() || *character == '.')
            .collect();

        match typed.parse::<f64>() {
            Ok(value) => Some(Ok(value / 100.0)),
            Err(_) => Some(Err(())),
        }
    });
}

fn find_spin_button(widget: gtk::Widget) -> Option<gtk::SpinButton> {
    if let Ok(button) = widget.clone().downcast::<gtk::SpinButton>() {
        return Some(button);
    }

    let mut child = widget.first_child();
    while let Some(candidate) = child {
        if let Some(found) = find_spin_button(candidate.clone()) {
            return Some(found);
        }
        child = candidate.next_sibling();
    }
    None
}

/// One value, as the control that shows it needs it.
pub fn as_text(value: &SettingValue) -> String {
    match value {
        SettingValue::Text(text) => text.clone(),
        SettingValue::Number(number) => number.to_string(),
        SettingValue::Toggle(on) => on.to_string(),
        SettingValue::List(items) => items.join(", "),
        SettingValue::Absent => String::new(),
    }
}

/// One value, as a switch needs it.
pub fn as_bool(value: &SettingValue) -> bool {
    matches!(value, SettingValue::Toggle(true))
}

/// One value, as a spin needs it.
pub fn as_number(value: &SettingValue) -> f64 {
    match value {
        SettingValue::Number(number) => *number,
        SettingValue::Text(text) => text.parse().unwrap_or_default(),
        _ => 0.0,
    }
}

/// One value, as a list editor needs it.
pub fn as_list(value: &SettingValue) -> Vec<String> {
    match value {
        SettingValue::List(items) => items.clone(),
        SettingValue::Text(text) if !text.is_empty() => vec![text.clone()],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_step_says_how_many_digits_it_needs() {
        assert_eq!(digits(1.0), 0);
        assert_eq!(digits(0.1), 1);
        assert_eq!(digits(0.01), 2);
    }

    #[test]
    fn a_value_reads_as_whatever_its_control_needs() {
        assert_eq!(as_text(&SettingValue::Text("x".into())), "x");
        assert_eq!(as_text(&SettingValue::Absent), "");
        assert!(as_bool(&SettingValue::Toggle(true)));
        assert!(!as_bool(&SettingValue::Absent));
        assert_eq!(as_number(&SettingValue::Number(7.5)), 7.5);
        assert_eq!(
            as_list(&SettingValue::List(vec!["a".into(), "b".into()])),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(as_list(&SettingValue::Absent).is_empty());
    }
}
