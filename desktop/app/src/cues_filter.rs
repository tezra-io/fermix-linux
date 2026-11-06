//! The Cues filter model (M60 cues §4.3): the provider and model rows the pane draws in place of
//! the section's own, since the daemon refuses either key alone. A provider picked is held, not
//! written; choosing its model writes both in one `settings.apply`, and the main model clears
//! both. Leaving the pane drops what was held.

use crate::daemon::Daemon;
use crate::descriptor::{info_button, option_factory, value_factory};
use crate::fields::{field_row, still_active};
use crate::settings_flow::NOT_SAVED;
use adw::prelude::*;
use fermix_client::cues::view::{
    main_model, model_shown, model_suggestions, shows_model_row, FilterDraft, FILTER_MODEL_KEY,
    FILTER_PROVIDER_KEY, SECTION,
};
use fermix_client::management::CallError;
use fermix_client::settings::{
    choice_items, placeholder, text_value, ChoiceItem, ChoiceOption, Listing, Row, SectionRows,
    MODELS_UNLISTED, NOT_SET,
};
use gtk::glib;
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SUGGESTIONS: &str = "Suggestions";

/// Where a write's outcome goes: a line in the window, and a fresh read of the section.
pub struct FilterHooks {
    pub toast: Rc<dyn Fn(&str)>,
    pub read_section: Rc<dyn Fn(&str)>,
}

#[derive(Default)]
struct Filter {
    draft: FilterDraft,
    provider_row: Option<Row>,
    model_row: Option<Row>,
    /// A held provider's models, by the provider they were listed for.
    listed: Option<(String, Listing)>,
    refusal: Option<String>,
    writing: bool,
    locked: bool,
}

impl Filter {
    fn saved(&self) -> String {
        self.provider_row
            .as_ref()
            .map(text_value)
            .unwrap_or_default()
    }

    /// The two rows as drawn now; none until the section holds both.
    fn shown(&self) -> Option<Shown> {
        let (provider_row, model_row) = (self.provider_row.clone()?, self.model_row.clone()?);
        let saved = self.saved();
        let listed = self.listed.as_ref().map(|(p, l)| (p.as_str(), l));
        let model_visible = shows_model_row(&self.draft, &saved);
        let (provider_note, model_note) = self.notes(model_visible);
        Some(Shown {
            provider: self.draft.provider(&saved).to_owned(),
            model: model_shown(&model_row, &self.draft, &saved),
            suggestions: model_suggestions(&model_row, &self.draft, &saved, listed),
            model_visible,
            blocked: self.locked || self.writing,
            provider_note,
            model_note,
            provider_row,
            model_row,
        })
    }

    /// The line under each row and whether it is a refusal: the daemon's refusal under the row
    /// it is about, else each row's footer, and a held provider's failed list under the model.
    fn notes(&self, model_visible: bool) -> (Note, Note) {
        let footer = |row: &Option<Row>| row.as_ref().and_then(|r| r.footer.clone());
        let unlisted =
            matches!(&self.listed, Some((_, Err(_)))).then(|| MODELS_UNLISTED.to_owned());
        match (self.refusal.clone(), model_visible) {
            (Some(sentence), true) => ((footer(&self.provider_row), false), (Some(sentence), true)),
            (Some(sentence), false) => ((Some(sentence), true), (None, false)),
            (None, _) => (
                (footer(&self.provider_row), false),
                (unlisted.or(footer(&self.model_row)), false),
            ),
        }
    }
}

/// A row's line, and whether it is a refusal.
type Note = (Option<String>, bool);

/// What one redraw draws.
struct Shown {
    provider_row: Row,
    model_row: Row,
    /// The provider selected: the one held, else the saved one.
    provider: String,
    model: String,
    suggestions: Vec<ChoiceOption>,
    model_visible: bool,
    blocked: bool,
    provider_note: Note,
    model_note: Note,
}

pub struct FilterRows {
    pub provider: adw::ComboRow,
    pub model: adw::ActionRow,
    entry: gtk::Entry,
    suggest: gtk::ListBox,
    suggest_button: gtk::MenuButton,
    daemon: Daemon,
    hooks: FilterHooks,
    state: RefCell<Filter>,
    /// The combo's items as last drawn.
    items: RefCell<Vec<ChoiceItem>>,
    /// Set while the combo is refilled, so its own notifications write nothing.
    filling: Cell<bool>,
    /// The info button, added once the row's info is known.
    info: Cell<bool>,
}

impl FilterRows {
    pub fn new(daemon: Daemon, hooks: FilterHooks) -> Rc<FilterRows> {
        let provider = adw::ComboRow::builder().visible(false).build();
        provider.set_factory(Some(&value_factory()));
        let entry = gtk::Entry::builder().valign(gtk::Align::Center).build();
        let suggest = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["navigation-sidebar"])
            .build();
        let suggest_button = suggestions_button(&suggest);
        let model = adw::ActionRow::builder().visible(false).build();
        let linked = gtk::Box::builder()
            .css_classes(["linked"])
            .valign(gtk::Align::Center)
            .build();
        linked.append(&entry);
        linked.append(&suggest_button);
        model.add_suffix(&linked);
        field_row(&model, &entry);
        let rows = Rc::new(FilterRows {
            provider,
            model,
            entry,
            suggest,
            suggest_button,
            daemon,
            hooks,
            state: RefCell::default(),
            items: RefCell::default(),
            filling: Cell::new(false),
            info: Cell::new(false),
        });
        rows.wire();
        rows
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        // The choice is taken once the combo's own notification is over: taking it may redraw
        // the combo, and a combo changed from inside its own signal can crash.
        self.provider.connect_selected_notify(move |combo| {
            let Some(rows) = weak.upgrade().filter(|r| !r.filling.get()) else {
                return;
            };
            let index = combo.selected();
            glib::idle_add_local_once(move || rows.chose_provider(index));
        });
        let weak = Rc::downgrade(self);
        self.entry.connect_activate(move |entry| {
            if let Some(rows) = weak.upgrade() {
                rows.chose_model(&entry.text());
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        focus.connect_leave(move |_| {
            let Some(rows) = weak.upgrade().filter(|r| still_active(&r.entry)) else {
                return;
            };
            rows.chose_model(&rows.entry.text());
        });
        self.entry.add_controller(focus);
        let weak = Rc::downgrade(self);
        self.suggest.connect_row_activated(move |_, item| {
            let Some(rows) = weak.upgrade() else { return };
            rows.suggest_button.popdown();
            let value = item.widget_name().to_string();
            rows.entry.set_text(&value);
            rows.chose_model(&value);
        });
    }

    /// The section as the daemon last published it, and whether writes are blocked.
    pub fn show(&self, section: Option<&SectionRows>, locked: bool) {
        let find = |key: &str| section.and_then(|s| s.rows.iter().find(|r| r.key == key).cloned());
        {
            let mut state = self.state.borrow_mut();
            state.provider_row = find(FILTER_PROVIDER_KEY);
            state.model_row = find(FILTER_MODEL_KEY);
            state.locked = locked;
        }
        self.draw();
    }

    /// Leaving the pane drops a provider held without its model.
    pub fn drop_held(&self) {
        let mut state = self.state.borrow_mut();
        state.draft.drop_held();
        state.listed = None;
        state.refusal = None;
        drop(state);
        self.draw();
    }

    /// Reads what to draw first and lets go of the state: a widget change can move the focus,
    /// and leaving the model field may write.
    fn draw(&self) {
        let shown = self.state.borrow().shown();
        let Some(shown) = shown else {
            self.provider.set_visible(false);
            self.model.set_visible(false);
            return;
        };
        self.draw_provider(&shown.provider_row, &shown.provider);
        self.draw_model(&shown.model_row, &shown.model);
        self.fill_suggestions(&shown.suggestions);
        self.model.set_visible(shown.model_visible);
        self.provider.set_sensitive(!shown.blocked);
        self.model.set_sensitive(!shown.blocked);
        let (on_provider, on_model) = (&shown.provider_note, &shown.model_note);
        note(
            self.provider.upcast_ref(),
            on_provider.0.as_deref(),
            on_provider.1,
        );
        note(self.model.upcast_ref(), on_model.0.as_deref(), on_model.1);
    }

    /// Refills the combo only when its options changed, and moves its selection only when the
    /// provider shown is not the one selected.
    fn draw_provider(&self, row: &Row, shown: &str) {
        let mut chosen = row.clone();
        chosen.value = json!(shown);
        let (items, selected) = choice_items(&chosen);
        let selected = u32::try_from(selected).expect("at most 200 options");
        self.provider
            .set_title(&glib::markup_escape_text(&row.label));
        self.add_info(row);
        self.filling.set(true);
        if *self.items.borrow() != items {
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            self.provider
                .set_model(Some(&gtk::StringList::new(&labels)));
            self.provider
                .set_list_factory(Some(&option_factory(Rc::new(items.clone()))));
            *self.items.borrow_mut() = items;
        }
        if self.provider.selected() != selected {
            self.provider.set_selected(selected);
        }
        self.filling.set(false);
        self.provider.set_visible(true);
    }

    fn add_info(&self, row: &Row) {
        let Some(info) = row.info.as_deref().filter(|_| !self.info.get()) else {
            return;
        };
        self.provider
            .add_suffix(&info_button(info, "More about this setting"));
        self.info.set(true);
    }

    /// The model field: what it holds, unless someone is typing in it.
    fn draw_model(&self, row: &Row, value: &str) {
        self.model.set_title(&glib::markup_escape_text(&row.label));
        self.entry.set_placeholder_text(Some(&placeholder(row)));
        self.entry
            .update_property(&[gtk::accessible::Property::Label(&row.label)]);
        if !self.entry.has_focus() && self.entry.text() != value {
            self.entry.set_text(value);
        }
    }

    fn fill_suggestions(&self, options: &[ChoiceOption]) {
        while let Some(child) = self.suggest.first_child() {
            self.suggest.remove(&child);
        }
        for option in options.iter().filter(|o| !o.disabled) {
            let label = option.hint.as_ref().map_or_else(
                || option.label.clone(),
                |hint| format!("{} · {hint}", option.label),
            );
            let line = gtk::Label::builder().label(label).xalign(0.0).build();
            let item = gtk::ListBoxRow::builder().child(&line).build();
            item.set_widget_name(&option.value);
            self.suggest.append(&item);
        }
        self.suggest_button
            .set_sensitive(options.iter().any(|o| !o.disabled));
    }

    // ---- choosing ----

    fn chose_provider(self: &Rc<Self>, index: u32) {
        let items = self.items.borrow().clone();
        let Some(item) = items.get(usize::try_from(index).expect("an index")) else {
            return;
        };
        let saved = self.state.borrow().saved();
        if item.disabled || item.value.is_none() {
            // An option the daemon offers but refuses says why, and the selection goes back.
            let reason = item.hint.as_deref().unwrap_or(NOT_SET);
            self.draw();
            self.provider
                .set_subtitle(&glib::markup_escape_text(reason));
            return;
        }
        let value = item.value.clone().expect("checked above");
        if value.is_empty() {
            self.chose_main_model(&saved);
            return;
        }
        if value == saved {
            self.drop_held();
            return;
        }
        self.hold(value);
    }

    fn chose_main_model(self: &Rc<Self>, saved: &str) {
        if saved.is_empty() {
            self.drop_held();
            return;
        }
        self.write(main_model());
    }

    /// Holds a provider, puts the focus in its model field and lists its models.
    fn hold(self: &Rc<Self>, provider: String) {
        {
            let mut state = self.state.borrow_mut();
            state.draft.hold(&provider);
            state.refusal = None;
            state.listed = None;
        }
        self.draw();
        self.entry.grab_focus();
        let rows = self.clone();
        glib::spawn_future_local(async move { rows.list_models(provider).await });
    }

    async fn list_models(&self, provider: String) {
        let target = provider.clone();
        let answer = self.daemon.call(move |m| m.models_list(&target)).await;
        let listing = answer.map(|page| page.models).map_err(|e| {
            glib::g_warning!(
                "fermix",
                "the Cues filter could not list {provider}'s models: {e:?}"
            );
            MODELS_UNLISTED.to_owned()
        });
        let mut state = self.state.borrow_mut();
        // A list that arrives after another provider was picked is not this one's.
        if state.draft.provider(&state.saved()) != provider {
            return;
        }
        state.listed = Some((provider, listing));
        drop(state);
        self.draw();
    }

    /// A model typed or picked. Nothing is written while a write is out (the field loses its
    /// focus as the main model hides it), for the model already saved, or for a blank one.
    fn chose_model(self: &Rc<Self>, typed: &str) {
        let state = self.state.borrow();
        let saved = state.saved();
        let unchanged = state
            .model_row
            .as_ref()
            .is_some_and(|row| text_value(row).trim() == typed.trim());
        let write = state.draft.choose_model(&saved, typed);
        let wanted = !state.writing && (state.draft.holds(&saved) || !unchanged);
        drop(state);
        if let Some(values) = write.filter(|_| wanted) {
            self.write(values);
        }
    }

    // ---- writing ----

    fn write(self: &Rc<Self>, values: Map<String, Value>) {
        self.state.borrow_mut().writing = true;
        self.draw();
        let rows = self.clone();
        glib::spawn_future_local(async move {
            let answer = rows
                .daemon
                .call(move |m| m.settings_apply(SECTION, values))
                .await;
            rows.written(answer.map(|applied| applied.side_effects));
        });
    }

    fn written(&self, answer: Result<Vec<String>, CallError>) {
        let refusal = match answer {
            Ok(side_effects) => {
                side_effects.iter().for_each(|s| (self.hooks.toast)(s));
                None
            }
            Err(e) => Some(refusal_sentence(e)),
        };
        let mut state = self.state.borrow_mut();
        state.writing = false;
        if refusal.is_none() {
            state.draft.drop_held();
            state.listed = None;
        }
        state.refusal = refusal;
        drop(state);
        self.draw();
        (self.hooks.read_section)(SECTION);
    }
}

/// The daemon's own sentence for a refused write; anything else is logged and said plainly.
fn refusal_sentence(e: CallError) -> String {
    match e {
        CallError::Refused(r) => r.sentence,
        other => {
            glib::g_warning!("fermix", "the Cues filter model was not written: {other:?}");
            NOT_SAVED.to_owned()
        }
    }
}

/// A row's line under its title: a refusal in the refused style, else a footer, else none.
fn note(row: &adw::PreferencesRow, text: Option<&str>, refused: bool) {
    let subtitle = glib::markup_escape_text(text.unwrap_or(""));
    if let Some(combo) = row.downcast_ref::<adw::ComboRow>() {
        combo.set_subtitle(&subtitle);
    } else if let Some(action) = row.downcast_ref::<adw::ActionRow>() {
        action.set_subtitle(&subtitle);
    }
    if refused {
        row.add_css_class("setting-refused");
    } else {
        row.remove_css_class("setting-refused");
    }
}

fn suggestions_button(list: &gtk::ListBox) -> gtk::MenuButton {
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(320)
        .child(list)
        .build();
    let popover = gtk::Popover::builder().child(&scroller).build();
    gtk::MenuButton::builder()
        .icon_name("pan-down-symbolic")
        .popover(&popover)
        .tooltip_text(SUGGESTIONS)
        .build()
}
