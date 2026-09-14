//! Home.
//!
//! Status first, then the two background controls, then what needs someone,
//! then the runtime facts behind a collapsed expander. Every word on it is the
//! daemon's, the command line's or the catalogue's, and the only thing this
//! file decides is where each one goes.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::home::{AttentionAction, AttentionRow, HomeModel, HomeSnapshot, RowTitle};
use crate::models::{spawn, Change, SettingsModel};

use super::{caption, fact_row, identifier_label, open_pane, value_label, CaptionRow};

/// The Home surface.
pub struct HomePage {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    home: Rc<HomeModel>,
    status: gtk::Label,
    background: adw::SwitchRow,
    background_caption: CaptionRow,
    login: adw::SwitchRow,
    login_caption: CaptionRow,
    attention: adw::PreferencesGroup,
    attention_rows: RefCell<Vec<adw::ActionRow>>,
    attention_ids: RefCell<Vec<String>>,
    attention_empty: gtk::Label,
    facts: Vec<(Key, gtk::Label)>,
    /// Set while the page is writing a control's own value into it, so the
    /// control's handler can tell a person's gesture from a refresh.
    updating: Cell<bool>,
}

impl HomePage {
    /// Build Home over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, home: Rc<HomeModel>) -> Rc<Self> {
        let column = super::column();
        column.add_css_class("fermix-gutter");

        let status = value_label("");
        let background = adw::SwitchRow::builder()
            .title(copy::text(Key::HomeSwitchRunInBackground))
            .build();
        let background_caption = CaptionRow::new();
        let login = adw::SwitchRow::builder()
            .title(copy::text(Key::HomeSwitchOpenAtLogin))
            .build();
        let login_caption = CaptionRow::new();

        let background_group = background_group(
            &status,
            &background,
            &background_caption,
            &login,
            &login_caption,
        );

        let (attention, attention_empty) = attention_group();
        let (runtime, facts) = runtime_group();

        column.append(&background_group);
        column.append(&attention);
        column.append(&runtime);

        let root: gtk::Widget = super::scrolled(&super::clamp(&column)).upcast();

        let page = Rc::new(Self {
            root,
            settings,
            home,
            status,
            background,
            background_caption,
            login,
            login_caption,
            attention,
            attention_rows: RefCell::new(Vec::new()),
            attention_ids: RefCell::new(Vec::new()),
            attention_empty,
            facts,
            updating: Cell::new(false),
        });

        page.connect();
        page.draw();
        page
    }

    /// The widget to put in the window.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// What this page draws, as data. The capture mode and the widget tests
    /// read it rather than the labels.
    pub fn snapshot(&self) -> HomeSnapshot {
        self.home.snapshot()
    }

    fn connect(self: &Rc<Self>) {
        self.connect_model();
        self.connect_switches();

        {
            // Shown is when Home reads; in front of someone is when it keeps
            // reading.
            let page = Rc::clone(self);
            super::on_shown(&self.root, move || {
                let home = Rc::clone(&page.home);
                let settings = Rc::clone(&page.settings);
                spawn(async move {
                    home.observe_desktop().await;
                    settings.refresh_overview().await;
                    settings.refresh_service().await;
                });
            });
        }

        // Home is the only surface that reads the overview, and it keeps
        // reading it only while it is in front of someone.
        let page = Rc::clone(self);
        super::watch_visibility(&self.root, move |visible| {
            if visible {
                page.home.start_polling();
            } else {
                page.home.stop_polling();
            }
        });
    }

    fn connect_model(self: &Rc<Self>) {
        {
            let page = Rc::downgrade(self);
            self.settings.observe(move |change| {
                let Some(page) = page.upgrade() else {
                    return;
                };
                if matches!(
                    change,
                    Change::Daemon | Change::Setup | Change::Service | Change::Sections
                ) {
                    page.draw();
                }
            });
        }
    }

    fn connect_switches(self: &Rc<Self>) {
        {
            let page = Rc::clone(self);
            self.background.connect_active_notify(move |switch| {
                if page.updating.get() {
                    return;
                }
                page.write_background(switch.is_active());
            });
        }

        let page = Rc::clone(self);
        self.login.connect_active_notify(move |switch| {
            if page.updating.get() {
                return;
            }
            page.write_login(switch.is_active());
        });
    }

    /// The switch keeps the position it was moved to until the command line
    /// answers; a refusal puts it back and says why underneath it.
    fn write_background(self: &Rc<Self>, enabled: bool) {
        self.background.set_sensitive(false);
        self.background_caption.set(None);

        let page = Rc::clone(self);
        spawn(async move {
            let outcome = page.settings.set_background_service(enabled).await;
            page.background.set_sensitive(true);

            if let Err(refusal) = outcome {
                page.background_caption.set(Some(&refusal.text));
            }
            page.draw();
        });
    }

    fn write_login(self: &Rc<Self>, enabled: bool) {
        self.login_caption.set(None);

        if let Err(reason) = self.settings.set_open_at_login(enabled) {
            self.login_caption.set(Some(&reason));
        }
        self.draw();
    }

    /// Draw the current state. Called on every change the model publishes, and
    /// it never rebuilds a control a person could be using.
    pub fn draw(&self) {
        let snapshot = self.home.snapshot();

        crate::ui::set_value(&self.status, &copy::text(snapshot.status.key()));

        self.updating.set(true);
        self.background.set_active(snapshot.background_enabled);
        self.login.set_active(snapshot.open_at_login);
        self.updating.set(false);

        for (label, value) in self.facts.iter() {
            if let Some(fact) = snapshot.facts.iter().find(|fact| fact.label == *label) {
                crate::ui::set_value(value, &fact.value);
            }
        }

        self.draw_attention(&snapshot.attention);
    }

    /// Rebuild the attention rows, but only when the rows themselves changed.
    ///
    /// A poll landing every five seconds must not take the focus out of a
    /// button somebody is about to press.
    fn draw_attention(&self, rows: &[AttentionRow]) {
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let details: Vec<Option<String>> = rows.iter().map(|row| row.detail.clone()).collect();

        self.attention_empty.set_visible(rows.is_empty());

        if *self.attention_ids.borrow() == ids {
            // The same gaps, possibly with newer supporting text.
            for (row, detail) in self.attention_rows.borrow().iter().zip(details) {
                row.set_subtitle(detail.as_deref().unwrap_or_default());
            }
            return;
        }

        for row in self.attention_rows.borrow_mut().drain(..) {
            self.attention.remove(&row);
        }

        let mut drawn = Vec::with_capacity(rows.len());
        for row in rows {
            let widget = self.attention_row(row);
            self.attention.add(&widget);
            drawn.push(widget);
        }

        self.attention_rows.replace(drawn);
        self.attention_ids.replace(ids);
    }

    fn attention_row(&self, row: &AttentionRow) -> adw::ActionRow {
        let widget = adw::ActionRow::builder().activatable(false).build();

        match &row.title {
            RowTitle::Words(key) => widget.set_title(&copy::text(*key)),
            RowTitle::Identifier(id) => {
                widget.set_title("");
                widget.add_prefix(&identifier_label(id));
            }
        }
        if let Some(detail) = row.detail.as_deref() {
            widget.set_subtitle(detail);
        }

        if let Some(action) = row.action.clone() {
            widget.add_suffix(&self.attention_button(action));
        }

        widget
    }

    fn attention_button(&self, action: AttentionAction) -> gtk::Button {
        let button = gtk::Button::builder()
            .label(copy::text(action.key()))
            .valign(gtk::Align::Center)
            .build();

        let settings = Rc::clone(&self.settings);
        button.connect_clicked(move |button| match &action {
            AttentionAction::OpenPane(pane, _) => open_pane(button, &settings, *pane),
            // Two rows, two sentences, one confirmation: the design gives the
            // update its own words and the same dialog takes it.
            AttentionAction::Restart | AttentionAction::FinishUpdating => {
                let _ = WidgetExt::activate_action(button, "win.restart", None);
            }
            AttentionAction::Quit => {
                let _ = WidgetExt::activate_action(button, "app.quit", None);
            }
            AttentionAction::OpenDoctor => {
                let _ = WidgetExt::activate_action(button, "win.doctor", None);
            }
            AttentionAction::Reload => {
                let settings = Rc::clone(&settings);
                spawn(async move {
                    settings.reload().await;
                });
            }
        });

        button
    }
}

/// One row per gap, and one centred sentence when there are none.
fn attention_group() -> (adw::PreferencesGroup, gtk::Label) {
    let group = adw::PreferencesGroup::builder()
        .title(copy::text(Key::HomeGroupAttention))
        .build();

    let empty = caption(&copy::text(Key::HomeAttentionEmpty));
    empty.set_xalign(0.5);
    empty.set_halign(gtk::Align::Center);
    group.add(&empty);

    (group, empty)
}

/// Status first, then the two controls, each with the caption row that carries
/// the command line's refusal when it refuses.
fn background_group(
    status: &gtk::Label,
    background: &adw::SwitchRow,
    background_caption: &CaptionRow,
    login: &adw::SwitchRow,
    login_caption: &CaptionRow,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(copy::text(Key::HomeGroupBackground))
        .build();

    let status_row = adw::ActionRow::builder()
        .title(copy::text(Key::HomeRowStatus))
        .activatable(false)
        .build();
    status_row.add_suffix(status);

    group.add(&status_row);
    group.add(background);
    group.add(background_caption.row());
    group.add(login);
    group.add(login_caption.row());
    group
}

/// The nine labelled facts, collapsed by default, in the order M38 fixes.
fn runtime_group() -> (adw::PreferencesGroup, Vec<(Key, gtk::Label)>) {
    // The expander is the heading: a group title above a single expander of the
    // same name says it twice.
    let group = adw::PreferencesGroup::new();
    let expander = adw::ExpanderRow::builder()
        .title(copy::text(Key::HomeGroupRuntimeDetails))
        .expanded(false)
        .build();

    let labels = [
        Key::HomeRuntimeEngine,
        Key::HomeRuntimeManagementProtocol,
        Key::HomeRuntimeUptime,
        Key::HomeRuntimeProvider,
        Key::HomeRuntimeChannels,
        Key::HomeRuntimeSkills,
        Key::HomeRuntimeTools,
        Key::HomeRuntimeService,
        Key::HomeRuntimeSession,
    ];

    let mut facts = Vec::with_capacity(labels.len());
    for label in labels {
        let (row, value) = fact_row(label, "");
        expander.add_row(&row);
        facts.push((label, value));
    }

    group.add(&expander);
    (group, facts)
}
