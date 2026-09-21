//! The Integrations pane.
//!
//! One flat list: what the pane is for, four counted filters with a search
//! beside them, then every integration the daemon published in one row shape,
//! and the sign-in clients at the foot. No collapsible groups, no drawn rules,
//! and no heading of its own: the window's header bar carries the page's title
//! and a second copy of it under the bar is a word the person reads twice.
//!
//! Every word on a row is the daemon's. The one thing this file decides is the
//! order of a chain a person starts with a single gesture: switching on
//! something that is not installed asks for the published consent, installs it,
//! enables it, and then opens the row's own page only when the daemon's next
//! step is a door the person has to walk through. A refused stage stops the
//! chain where it stopped and shows the daemon's sentence.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{PluginAction, PluginOAuthClient, PluginSettingKind, SettingsPane};
use crate::management::vocabulary::plugin_id;
use crate::models::jobs::{phase_word, JobRunner};
use crate::models::plugins::{
    answered_by_dialog, EnableOutcome, FeatureRow, IntegrationFilter, IntegrationRow, PluginsModel,
    FILTERS,
};
use crate::models::providers::ProvidersModel;
use crate::models::{spawn, Change, SettingsModel};
use crate::ui::plain;
use crate::ui::settings::dialogs::consent::ConsentDialog;
use crate::ui::settings::dialogs::oauth_client::OAuthClientDialog;
use crate::ui::settings::dialogs::secret::SecretDialog;
use crate::ui::settings::dialogs::sign_in::SignInDialog;
use crate::ui::settings::dialogs::workspace::WorkspaceDialog;
use crate::ui::widgets::mark::{self, MarkKind};
use crate::ui::CaptionRow;

/// Where a Features row sends someone.
type OpenPane = Box<dyn Fn(SettingsPane)>;

/// The Integrations pane.
pub struct IntegrationsPane {
    view: adw::NavigationView,
    settings: Rc<SettingsModel>,
    model: Rc<PluginsModel>,
    /// A plugin's own sign-in is the same browser hop a provider takes, under
    /// the plugin spelling the contract publishes.
    providers: Rc<ProvidersModel>,
    filters: Vec<(IntegrationFilter, gtk::ToggleButton)>,
    search: gtk::SearchEntry,
    list: gtk::ListBox,
    clients: adw::PreferencesGroup,
    notice: CaptionRow,
    empty: gtk::Label,
    chosen: Cell<IntegrationFilter>,
    /// One page per plugin, built once and pushed again on every visit, and
    /// the detail that redraws each of them from the live row.
    pages: RefCell<BTreeMap<String, adw::NavigationPage>>,
    details: RefCell<BTreeMap<String, Rc<Detail>>>,
    /// Which pane a Features row opens. The feature's own switch lives there.
    open_pane: RefCell<Option<OpenPane>>,
    /// Set while a switch is being written from the daemon's own answer.
    updating: Rc<Cell<bool>>,
    /// The plugin rows on screen, and the names the list was built from, so an
    /// answer that changes nothing does not rebuild what a person is standing
    /// on.
    rows: RefCell<BTreeMap<String, ListRow>>,
    shape: RefCell<Vec<String>>,
}

/// One row of the list, and the part of it that changes.
struct ListRow {
    row: adw::ActionRow,
    switch: gtk::Switch,
}

impl IntegrationsPane {
    /// Build the pane over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let model = PluginsModel::new(Rc::clone(&settings));
        let providers = ProvidersModel::new(Rc::clone(&settings));

        let subtitle = crate::ui::caption(&copy::text(Key::IntegrationsSubtitle));

        // Wide enough to type an integration's name into at the clamp's width,
        // in characters rather than pixels so it grows with the text size.
        let search = gtk::SearchEntry::builder()
            .hexpand(true)
            .width_chars(SEARCH_WIDTH)
            .placeholder_text(copy::text(Key::ActionSearchAccessible))
            .build();
        search.update_property(&[gtk::accessible::Property::Label(&copy::text(
            Key::IntegrationsSearchAccessible,
        ))]);

        let linked = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .build();
        linked.add_css_class("linked");

        let mut filters: Vec<(IntegrationFilter, gtk::ToggleButton)> = Vec::new();
        for filter in FILTERS {
            let button = gtk::ToggleButton::builder()
                .label(copy::text(filter.key()))
                .build();
            if let Some((_, first)) = filters.first() {
                button.set_group(Some(first));
            }
            linked.append(&button);
            filters.push((*filter, button));
        }

        // The filters keep their linked row and the search drops under them
        // when there is not width for both: a row of controls that cannot
        // shrink is a window that cannot reach its own minimum.
        let controls = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(2)
            .min_children_per_line(1)
            .column_spacing(crate::metrics::SPACE_HEADING as u32)
            .row_spacing(crate::metrics::SPACE_HEADING as u32)
            .homogeneous(false)
            .build();
        controls.append(&linked);
        controls.append(&search);

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .show_separators(false)
            .build();

        let empty = crate::ui::caption(&copy::text(Key::IntegrationsEmpty));
        empty.set_visible(false);

        let notice = CaptionRow::new();
        let notice_group = crate::ui::caption_group(&notice);

        let clients = adw::PreferencesGroup::builder()
            .title(copy::text(Key::IntegrationsSignInClients))
            .build();

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&subtitle);
        column.append(&controls);
        column.append(&notice_group);
        column.append(&list);
        column.append(&empty);
        column.append(&clients);

        let view = adw::NavigationView::new();
        view.add(
            &adw::NavigationPage::builder()
                .title(copy::text(Key::PaneIntegrations))
                .child(&crate::ui::scrolled(&crate::ui::clamp(&column)))
                .build(),
        );

        let pane = Rc::new(Self {
            view,
            settings,
            model,
            providers,
            filters,
            search,
            list,
            clients,
            notice,
            empty,
            chosen: Cell::new(IntegrationFilter::Installed),
            pages: RefCell::new(BTreeMap::new()),
            details: RefCell::new(BTreeMap::new()),
            open_pane: RefCell::new(None),
            updating: Rc::new(Cell::new(false)),
            rows: RefCell::new(BTreeMap::new()),
            shape: RefCell::new(Vec::new()),
        });

        pane.connect();
        pane.draw();
        pane
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.view.clone().upcast()
    }

    /// Where a Features row goes.
    pub fn on_open_pane(&self, open: impl Fn(SettingsPane) + 'static) {
        self.open_pane.replace(Some(Box::new(open)));
    }

    /// Put the focus in the search, which is what the window's search action
    /// does on this surface.
    pub fn focus_search(&self) {
        self.search.grab_focus();
    }

    /// Read everything this pane draws.
    pub fn load(self: &Rc<Self>) {
        let model = Rc::clone(&self.model);
        let settings = Rc::clone(&self.settings);
        spawn(async move {
            settings.refresh_setup().await;
            model.refresh().await;
        });
    }

    /// Go back one page, where a plugin's own page is showing.
    pub fn pop(&self) -> bool {
        self.view.pop()
    }

    /// The title of the page showing, where it is not the pane's own.
    pub fn sub_page_title(&self) -> Option<String> {
        let page = self.view.visible_page()?;
        (self.view.navigation_stack().n_items() > 1).then(|| page.title().to_string())
    }

    /// Tell me when the page showing changes.
    pub fn on_page_changed(&self, changed: impl Fn() + 'static) {
        self.view.connect_visible_page_notify(move |_| changed());
    }

    fn connect(self: &Rc<Self>) {
        for (filter, button) in &self.filters {
            let pane = Rc::clone(self);
            let filter = *filter;
            button.connect_toggled(move |button| {
                if !button.is_active() {
                    return;
                }
                pane.chosen.set(filter);
                pane.draw();
            });
        }
        if let Some((_, first)) = self.filters.first() {
            first.set_active(true);
        }

        {
            let pane = Rc::clone(self);
            self.search.connect_search_changed(move |_| pane.draw());
        }
        {
            let pane = Rc::downgrade(self);
            self.model.observe(move || {
                if let Some(pane) = pane.upgrade() {
                    pane.draw();
                }
            });
        }
        {
            let pane = Rc::downgrade(self);
            self.settings.observe(move |change| {
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                if matches!(change, Change::Setup) {
                    pane.draw();
                }
            });
        }
        {
            // The one runner every plugin job runs through: its phase is the
            // line under the filters while something is happening.
            let pane = Rc::downgrade(self);
            let runner = self.model.job();
            let watched = Rc::clone(&runner);
            runner.observe(move || {
                if let Some(pane) = pane.upgrade() {
                    pane.draw_notice(&watched);
                }
            });
        }
    }

    /// The list, the counts and the clients.
    fn draw(self: &Rc<Self>) {
        self.draw_counts();
        self.draw_rows();
        self.draw_clients();
    }

    fn draw_counts(&self) {
        for (filter, button) in &self.filters {
            button.set_label(&copy::fill(
                Key::CountedFilter,
                &[
                    ("{name}", &copy::text(filter.key())),
                    ("{count}", &self.model.count(*filter).to_string()),
                ],
            ));
            crate::ui::shorten(button.upcast_ref::<gtk::Button>());
        }
    }

    /// The list, rebuilt only when the set of rows in it changes.
    ///
    /// A filter or a search changes that set and rebuilds; an answer that
    /// arrives after an action usually does not, and rebuilding then would take
    /// the focus out of the row a person is standing on.
    fn draw_rows(self: &Rc<Self>) {
        let query = self.search.text().to_string();
        let filter = self.chosen.get();

        let features = filter == IntegrationFilter::Features;
        let visible: Vec<String> = if features {
            self.model
                .features()
                .into_iter()
                .filter(|feature| feature.matches(&query))
                .map(|feature| feature.id.to_string())
                .collect()
        } else {
            self.model
                .rows()
                .into_iter()
                .filter(|row| filter.admits(row) && row.matches(&query))
                .map(|row| row.name.clone())
                .collect()
        };

        if *self.shape.borrow() != visible {
            self.rebuild(&visible, features);
            self.shape.replace(visible.clone());
        } else if !features {
            for row in self.model.rows() {
                self.update_row(&row);
            }
        }

        self.empty
            .set_visible(visible.is_empty() && self.model.is_loaded());
    }

    fn rebuild(self: &Rc<Self>, visible: &[String], features: bool) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.rows.borrow_mut().clear();

        if features {
            for feature in self.model.features() {
                if !visible.contains(&feature.id.to_string()) {
                    continue;
                }
                self.list.append(&self.feature_row(&feature));
            }
            return;
        }

        for row in self.model.rows() {
            if !visible.contains(&row.name) {
                continue;
            }
            let built = self.plugin_row(&row);
            self.list.append(&built.row);
            self.rows.borrow_mut().insert(row.name.clone(), built);
        }
    }

    /// One row's changing parts: where it stands, and where its switch is.
    fn update_row(&self, row: &IntegrationRow) {
        let held = self.rows.borrow();
        let Some(built) = held.get(&row.name) else {
            return;
        };

        built.row.set_subtitle(row.subtitle());
        self.updating.set(true);
        built.switch.set_active(row.enabled);
        self.updating.set(false);
    }

    /// One integration row: the mark, the name over where it stands, the switch
    /// and the way in. The switch is its own focus stop and does not activate
    /// the row.
    fn plugin_row(self: &Rc<Self>, row: &IntegrationRow) -> ListRow {
        let widget = plain(
            adw::ActionRow::builder()
                .title(row.title.as_str())
                .subtitle(row.subtitle())
                .activatable(true)
                .build(),
        );

        widget.add_prefix(&mark::from_record(
            MarkKind::Plugin,
            mark::integration(&row.name),
        ));

        let switch = gtk::Switch::builder()
            .valign(gtk::Align::Center)
            .active(row.enabled)
            .build();
        switch.update_property(&[gtk::accessible::Property::Label(&copy::fill(
            Key::IntegrationsSwitchAccessible,
            &[("{name}", &row.title)],
        ))]);
        widget.add_suffix(&switch);
        widget.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        {
            let pane = Rc::clone(self);
            let name = row.name.clone();
            widget.connect_activated(move |_| pane.open(&name));
        }
        {
            let pane = Rc::clone(self);
            let name = row.name.clone();
            let updating = Rc::clone(&self.updating);
            switch.connect_state_set(move |switch, wanted| {
                if !updating.get() {
                    pane.toggle(&name, wanted, switch);
                }
                glib::Propagation::Proceed
            });
        }

        ListRow {
            row: widget,
            switch,
        }
    }

    /// One native driver row: where it stands, and the pane that owns it.
    fn feature_row(self: &Rc<Self>, feature: &FeatureRow) -> adw::ActionRow {
        let widget = plain(
            adw::ActionRow::builder()
                .title(copy::text(feature.title))
                .subtitle(copy::text(feature.summary))
                .activatable(true)
                .build(),
        );

        widget.add_prefix(&mark::from_record(
            MarkKind::Feature,
            mark::integration(feature.id),
        ));
        widget.add_suffix(&crate::ui::value_label(&copy::text(feature.standing())));

        let open = gtk::Button::builder()
            .label(copy::text(Key::IntegrationsOpen))
            .valign(gtk::Align::Center)
            .build();
        widget.add_suffix(&open);

        {
            let pane = Rc::clone(self);
            let target = feature.pane;
            widget.connect_activated(move |_| pane.route(target));
        }
        {
            let pane = Rc::clone(self);
            let target = feature.pane;
            open.connect_clicked(move |_| pane.route(target));
        }

        widget
    }

    fn route(&self, pane: SettingsPane) {
        if let Some(open) = self.open_pane.borrow().as_ref() {
            open(pane);
        }
    }

    fn draw_clients(self: &Rc<Self>) {
        clear(&self.clients);

        let clients = self.model.clients();
        self.clients.set_visible(!clients.is_empty());
        for client in clients {
            self.clients.add(&self.client_row(&client));
        }
    }

    /// One sign-in client: the vendor's own name from the mark record, because
    /// the wire publishes the family's id and nothing else.
    fn client_row(self: &Rc<Self>, client: &PluginOAuthClient) -> adw::ActionRow {
        let record = mark::oauth_client(&client.provider);
        let title = record
            .map(|record| record.display_name.clone())
            .unwrap_or_else(|| client.provider.clone());

        let widget = plain(
            adw::ActionRow::builder()
                .title(title)
                .subtitle(if client.configured {
                    copy::text(Key::SecretStored)
                } else {
                    String::new()
                })
                .activatable(true)
                .build(),
        );

        widget.add_prefix(&mark::from_record(MarkKind::OauthClient, record));
        widget.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        let pane = Rc::clone(self);
        let client = client.clone();
        widget.connect_activated(move |row| {
            OAuthClientDialog::present(
                Rc::clone(&pane.model),
                Rc::clone(&pane.settings),
                client.clone(),
                row,
            );
        });

        widget
    }

    /// The line under the filters: the last refusal, or the step a job is on.
    fn draw_notice(&self, runner: &Rc<JobRunner>) {
        if let Some(sentence) = self.model.refusal() {
            self.notice.set(Some(&sentence.text));
            return;
        }

        let word = runner
            .job()
            .and_then(|job| job.phase)
            .as_deref()
            .and_then(phase_word);

        match word {
            Some(word) => self.notice.set(Some(&copy::text(word))),
            None => self.notice.set(None),
        }
    }

    /// The row's switch, both ways.
    ///
    /// Switching on something that is not installed is one gesture and three
    /// stages: the published consent, the install, then the enable. A refused
    /// stage stops the chain there and the switch goes back.
    fn toggle(self: &Rc<Self>, name: &str, wanted: bool, switch: &gtk::Switch) {
        self.model.clear_refusal();

        let Some(row) = self.model.row(name) else {
            return;
        };

        if wanted && !row.installed {
            let pane = Rc::clone(self);
            let name = name.to_string();
            let restore = switch.clone();
            let updating = Rc::clone(&self.updating);
            ConsentDialog::present(
                &row,
                switch,
                move || pane.install_then_enable(&name),
                move || {
                    // Nothing was installed, so the switch goes back to where
                    // the daemon has it.
                    updating.set(true);
                    restore.set_active(false);
                    updating.set(false);
                },
            );
            return;
        }

        let pane = Rc::clone(self);
        let name = name.to_string();
        let switch = switch.clone();
        spawn(async move {
            match pane.model.set_enabled(wanted, &name).await {
                EnableOutcome::Refused(_) => {
                    pane.updating.set(true);
                    switch.set_active(!wanted);
                    pane.updating.set(false);
                }
                EnableOutcome::Configure(row) => pane.open(&row.name),
                EnableOutcome::Done => {}
            }
        });
    }

    /// The install a consent answered, and the enable it was asked for.
    fn install_then_enable(self: &Rc<Self>, name: &str) {
        let pane = Rc::clone(self);
        let name = name.to_string();

        spawn(async move {
            // A refused start is already on the model and under the filters.
            if pane
                .model
                .perform(PluginAction::Install, &name)
                .await
                .is_some()
            {
                return;
            }
            pane.watch_install(name);
        });
    }

    /// Follow one install to its end, then finish the chain.
    ///
    /// The runner is the model's, so the chain survives this pane being left:
    /// the enable and the re-read happen wherever the person has gone.
    fn watch_install(self: &Rc<Self>, name: String) {
        let runner = self.model.job();
        let pane = Rc::clone(self);
        let watched = Rc::clone(&runner);
        let finished = Rc::new(Cell::new(false));

        runner.observe(move || {
            if !watched.is_terminal() || finished.replace(true) {
                return;
            }
            let pane = Rc::clone(&pane);
            let name = name.clone();
            spawn(async move {
                match pane.model.install_finished(&name).await {
                    EnableOutcome::Configure(row) => pane.open(&row.name),
                    EnableOutcome::Refused(_) | EnableOutcome::Done => {}
                }
            });
        });
    }

    /// Open one plugin's own page, building it the first time.
    ///
    /// The page addresses its plugin by name and reads the live row every time
    /// it draws, so a workspace discovered while it was open is on it.
    fn open(self: &Rc<Self>, name: &str) {
        let page = self.pages.borrow().get(name).cloned();
        let page = match page {
            Some(page) => page,
            None => {
                let Some(row) = self.model.row(name) else {
                    return;
                };
                let page = self.build_page(&row);
                self.view.add(&page);
                self.pages
                    .borrow_mut()
                    .insert(name.to_string(), page.clone());
                page
            }
        };

        self.view.push(&page);

        let model = Rc::clone(&self.model);
        spawn(async move {
            model.refresh().await;
        });
    }

    /// One plugin's page: where it stands, the next step in the daemon's own
    /// words, its verbs, its settings and its doors.
    fn build_page(self: &Rc<Self>, row: &IntegrationRow) -> adw::NavigationPage {
        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");

        let detail = Rc::new(Detail {
            pane: Rc::downgrade(self),
            name: row.name.clone(),
            status: adw::PreferencesGroup::new(),
            verbs: adw::PreferencesGroup::new(),
            settings: adw::PreferencesGroup::new(),
            doors: adw::PreferencesGroup::new(),
            notice: CaptionRow::new(),
            shape: RefCell::new(String::new()),
        });

        let notice_group = crate::ui::caption_group(&detail.notice);

        column.append(&detail.status);
        column.append(&detail.verbs);
        column.append(&detail.settings);
        column.append(&detail.doors);
        column.append(&notice_group);

        detail.draw();
        {
            let detail = Rc::downgrade(&detail);
            self.model.observe(move || {
                if let Some(detail) = detail.upgrade() {
                    detail.draw();
                }
            });
        }

        // The pane owns the detail for as long as it owns the page, and the
        // detail holds the pane weakly: the two would otherwise keep each
        // other alive after the window has gone.
        self.details
            .borrow_mut()
            .insert(row.name.clone(), Rc::clone(&detail));

        adw::NavigationPage::builder()
            .title(row.title.as_str())
            .child(&crate::ui::scrolled(&crate::ui::clamp(&column)))
            .build()
    }
}

/// One plugin's page, redrawn from the live row.
struct Detail {
    pane: std::rc::Weak<IntegrationsPane>,
    name: String,
    status: adw::PreferencesGroup,
    verbs: adw::PreferencesGroup,
    settings: adw::PreferencesGroup,
    doors: adw::PreferencesGroup,
    notice: CaptionRow,
    /// What the page was last built from, so a redraw that changes nothing does
    /// not take the focus out of a field.
    shape: RefCell<String>,
}

impl Detail {
    /// The pane this page belongs to, while it is still there.
    fn pane(&self) -> Option<Rc<IntegrationsPane>> {
        self.pane.upgrade()
    }

    fn draw(self: &Rc<Self>) {
        let Some(pane) = self.pane() else {
            return;
        };
        let Some(row) = pane.model.row(&self.name) else {
            return;
        };

        let shape = format!(
            "{}|{}|{:?}|{}|{}|{}",
            row.status,
            row.verbs.join(","),
            row.primary_action,
            row.settings.len(),
            row.credential_present,
            row.workspace_label.clone().unwrap_or_default()
        );
        if *self.shape.borrow() == shape {
            return;
        }
        self.shape.replace(shape);

        self.status.set_title(&row.title);
        self.status.set_description(Some(&row.status));

        self.draw_verbs(&row);
        self.draw_settings(&row);
        self.draw_doors(&row);

        // A group with nothing in it draws its own hairline, so a plugin with
        // no settings and no doors shows neither.
        self.settings.set_visible(!row.settings.is_empty());
        self.doors
            .set_visible(self.doors.first_child().is_some() && has_doors(&row, &pane));
        self.notice.set(
            pane.model
                .refusal()
                .map(|sentence| sentence.text)
                .as_deref(),
        );
    }

    /// The next step in the daemon's own word, and one button per verb it
    /// published, routed on the id beside it.
    fn draw_verbs(self: &Rc<Self>, row: &IntegrationRow) {
        clear(&self.verbs);

        if let Some(verb) = row.primary_verb.as_deref() {
            self.verbs.add(&plain(
                adw::ActionRow::builder()
                    .title(copy::text(Key::IntegrationsNextStep))
                    .subtitle(verb)
                    .activatable(false)
                    .build(),
            ));
        }

        let buttons = row.buttons();
        if buttons.is_empty() {
            return;
        }

        // One row of buttons, wrapping rather than running off a narrow
        // window. The words are the daemon's and so is the id each one runs.
        let wrap = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(buttons.len() as u32)
            .column_spacing(crate::metrics::SPACE_TIGHT as u32)
            .row_spacing(crate::metrics::SPACE_TIGHT as u32)
            .margin_top(crate::metrics::SPACE_HEADING)
            .margin_bottom(crate::metrics::SPACE_HEADING)
            .margin_start(crate::metrics::SPACE_HEADING)
            .margin_end(crate::metrics::SPACE_HEADING)
            .build();

        for (word, action) in buttons {
            let button = gtk::Button::builder()
                .label(word.as_str())
                .valign(gtk::Align::Center)
                .build();
            wrap.append(&button);

            let detail = Rc::clone(self);
            button.connect_clicked(move |button| detail.perform(action, button));
        }

        self.verbs.add(&plain(
            adw::PreferencesRow::builder()
                .activatable(false)
                .selectable(false)
                .focusable(false)
                .child(&wrap)
                .build(),
        ));
    }

    /// The plugin's own settings, in the two kinds the manifest declares.
    fn draw_settings(self: &Rc<Self>, row: &IntegrationRow) {
        clear(&self.settings);

        for setting in &row.settings {
            match setting.kind {
                Some(PluginSettingKind::Boolean) => {
                    let widget = plain(
                        adw::SwitchRow::builder()
                            .title(setting.label.as_str())
                            .active(setting.value.as_deref() == Some(TRUE))
                            .build(),
                    );
                    let detail = Rc::clone(self);
                    let key = setting.key.clone();
                    widget.connect_active_notify(move |widget| {
                        detail.write_setting(&key, if widget.is_active() { TRUE } else { FALSE });
                    });
                    self.settings.add(&widget);
                }
                // A kind this build has never seen reads as text, which is what
                // the wire carries for every setting anyway.
                _ => {
                    let widget = plain(
                        adw::EntryRow::builder()
                            .title(setting.label.as_str())
                            .text(setting.value.clone().unwrap_or_default())
                            .build(),
                    );
                    let detail = Rc::clone(self);
                    let key = setting.key.clone();
                    widget.connect_entry_activated(move |widget| {
                        detail.write_setting(&key, &widget.text());
                    });
                    self.settings.add(&widget);
                }
            }
        }
    }

    /// The doors a person walks through: the credential slot, the sign-in
    /// client and the workspace, each where the daemon published one.
    fn draw_doors(self: &Rc<Self>, row: &IntegrationRow) {
        clear(&self.doors);

        let takes_token = row
            .actions
            .iter()
            .any(|action| matches!(action, PluginAction::AddToken | PluginAction::ReplaceToken));
        if takes_token {
            self.doors.add(&self.token_row(row));
        }

        let Some(pane) = self.pane() else {
            return;
        };

        if let Some(client) = pane.model.client_for(row) {
            let widget = plain(
                adw::ActionRow::builder()
                    .title(copy::text(Key::IntegrationsClientRow))
                    .subtitle(if client.configured {
                        copy::text(Key::SecretStored)
                    } else {
                        String::new()
                    })
                    .activatable(true)
                    .build(),
            );
            widget.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            self.doors.add(&widget);

            let owner = Rc::clone(&pane);
            widget.connect_activated(move |row| {
                OAuthClientDialog::present(
                    Rc::clone(&owner.model),
                    Rc::clone(&owner.settings),
                    client.clone(),
                    row,
                );
            });
        }

        if row.binds_workspace() {
            let widget = plain(
                adw::ActionRow::builder()
                    .title(copy::text(Key::IntegrationsWorkspaceRow))
                    .subtitle(row.workspace_label.clone().unwrap_or_default())
                    .activatable(true)
                    .build(),
            );
            widget.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            self.doors.add(&widget);

            let owner = Rc::clone(&pane);
            let name = row.name.clone();
            widget.connect_activated(move |row| {
                WorkspaceDialog::present(Rc::clone(&owner.model), &name, row);
            });
        }
    }

    /// The credential slot, which is the one door to a plugin's own token.
    fn token_row(self: &Rc<Self>, row: &IntegrationRow) -> adw::ActionRow {
        let widget = plain(
            adw::ActionRow::builder()
                .title(copy::text(Key::SecretFieldLabel))
                .subtitle(if row.credential_present {
                    copy::text(Key::SecretStored)
                } else {
                    String::new()
                })
                .activatable(false)
                .build(),
        );

        let button = gtk::Button::builder()
            .label(copy::text(if row.credential_present {
                Key::SecretReplace
            } else {
                Key::SecretAdd
            }))
            .valign(gtk::Align::Center)
            .build();
        widget.add_suffix(&button);

        let Some(pane) = self.pane() else {
            return widget;
        };
        let settings = Rc::clone(&pane.settings);
        let id = plugin_id(&row.name);
        let present = row.credential_present;
        button.connect_clicked(move |button| {
            SecretDialog::present(Rc::clone(&settings), PLUGIN_SECTION, &id, present, button);
        });

        widget
    }

    /// One published action, run by its id.
    fn perform(self: &Rc<Self>, action: PluginAction, anchor: &gtk::Button) {
        if answered_by_dialog(action) {
            self.open_door(action, anchor);
            return;
        }

        let Some(pane) = self.pane() else {
            return;
        };

        if action == PluginAction::SignIn {
            let providers = Rc::clone(&pane.providers);
            let anchor = anchor.clone();
            let id = plugin_id(&self.name);
            spawn(async move {
                if let Ok(started) = providers.start_sign_in(&id).await {
                    SignInDialog::present(providers, started, &anchor);
                }
            });
            return;
        }

        let detail = Rc::clone(self);
        spawn(async move {
            let refusal = pane.model.perform(action, &detail.name).await;
            detail
                .notice
                .set(refusal.map(|sentence| sentence.text).as_deref());
        });
    }

    /// The doors a verb opens rather than performs.
    fn open_door(self: &Rc<Self>, action: PluginAction, anchor: &gtk::Button) {
        let Some(pane) = self.pane() else {
            return;
        };
        let Some(row) = pane.model.row(&self.name) else {
            return;
        };

        match action {
            PluginAction::AddToken | PluginAction::ReplaceToken => SecretDialog::present(
                Rc::clone(&pane.settings),
                PLUGIN_SECTION,
                &plugin_id(&self.name),
                row.credential_present,
                anchor,
            ),
            PluginAction::SetUpClient => {
                let Some(client) = pane.model.client_for(&row) else {
                    return;
                };
                OAuthClientDialog::present(
                    Rc::clone(&pane.model),
                    Rc::clone(&pane.settings),
                    client,
                    anchor,
                );
            }
            PluginAction::ChooseWorkspace => {
                // The workspaces are discovered first where the daemon has none
                // to offer yet, and the dialog opens on what it published.
                let owner = Rc::clone(&pane);
                let name = self.name.clone();
                let anchor = anchor.clone();
                let empty = row.workspaces.is_empty();
                spawn(async move {
                    if empty {
                        owner.model.discover_workspaces(&name).await;
                    }
                    WorkspaceDialog::present(Rc::clone(&owner.model), &name, &anchor);
                });
            }
            _ => {}
        }
    }

    fn write_setting(self: &Rc<Self>, key: &str, value: &str) {
        let Some(pane) = self.pane() else {
            return;
        };
        let detail = Rc::clone(self);
        let key = key.to_string();
        let value = value.to_string();
        spawn(async move {
            let refusal = pane.model.set_setting(&detail.name, &key, &value).await;
            detail
                .notice
                .set(refusal.map(|sentence| sentence.text).as_deref());
        });
    }
}

/// Whether one row has any door at all: a credential slot, a sign-in client or
/// a workspace.
fn has_doors(row: &IntegrationRow, pane: &IntegrationsPane) -> bool {
    row.actions
        .iter()
        .any(|action| matches!(action, PluginAction::AddToken | PluginAction::ReplaceToken))
        || pane.model.client_for(row).is_some()
        || row.binds_workspace()
}

/// Empty one group of the rows it holds.
fn clear(group: &adw::PreferencesGroup) {
    let mut rows: Vec<adw::PreferencesRow> = Vec::new();
    collect_rows(group.upcast_ref::<gtk::Widget>(), &mut rows);
    for row in rows {
        group.remove(&row);
    }
}

fn collect_rows(widget: &gtk::Widget, into: &mut Vec<adw::PreferencesRow>) {
    let mut child = widget.first_child();
    while let Some(candidate) = child {
        match candidate.downcast_ref::<adw::PreferencesRow>() {
            Some(row) => into.push(row.clone()),
            None => collect_rows(&candidate, into),
        }
        child = candidate.next_sibling();
    }
}

/// A plugin credential belongs to no settings section: the id the contract
/// publishes is the whole address.
const PLUGIN_SECTION: &str = "";

/// How wide the search is, in characters.
const SEARCH_WIDTH: i32 = 10;

/// The two words a boolean setting takes on the wire.
const TRUE: &str = "true";
const FALSE: &str = "false";
