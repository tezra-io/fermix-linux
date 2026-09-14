//! The Channels pane.
//!
//! One switch row per channel the daemon publishes, and one page behind each of
//! them holding whatever fields that channel has. Nothing about a channel's
//! field set is written down here: the page is the daemon's descriptor for
//! `channels.<name>`, with the credential rows first because they are what a
//! channel is usually waiting for.
//!
//! The switch writes the channel's own enable row, which is an ordinary
//! optimistic write: the position holds until the accepted re-read lands, and a
//! refusal puts it back with the daemon's own sentence under it.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{
    SettingValue, SettingsPane, SettingsRow, SettingsRowKind, SetupChannelRow,
};
use crate::management::vocabulary::ChannelStanding;
use crate::models::{spawn, Change, SettingsModel, State};
use crate::ui::settings::descriptor_form::DescriptorForm;
use crate::ui::widgets::mark::{self, MarkKind};
use crate::ui::CaptionRow;

/// The prefix the daemon publishes a channel's section under.
const PREFIX: &str = "channels.";

/// The Channels pane.
pub struct ChannelsPane {
    view: adw::NavigationView,
    settings: Rc<SettingsModel>,
    group: adw::PreferencesGroup,
    rows: RefCell<BTreeMap<String, Row>>,
    shape: RefCell<Vec<String>>,
    pages: RefCell<BTreeMap<String, adw::NavigationPage>>,
}

/// One channel row and the parts of it that change.
struct Row {
    row: adw::SwitchRow,
    caption: CaptionRow,
    /// Set while this row is writing the daemon's value into its own switch, so
    /// the handler can tell a person's gesture from a refresh.
    updating: Rc<Cell<bool>>,
}

impl ChannelsPane {
    /// Build the pane over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let group = adw::PreferencesGroup::builder()
            .title(copy::text(Key::ChannelsGroup))
            .build();

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&group);

        let view = adw::NavigationView::new();
        view.add(
            &adw::NavigationPage::builder()
                .title(copy::text(Key::PaneChannels))
                .child(&crate::ui::scrolled(&crate::ui::clamp(&column)))
                .build(),
        );

        let pane = Rc::new(Self {
            view,
            settings,
            group,
            rows: RefCell::new(BTreeMap::new()),
            shape: RefCell::new(Vec::new()),
            pages: RefCell::new(BTreeMap::new()),
        });

        pane.connect();
        pane.draw();
        pane
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.view.clone().upcast()
    }

    /// Read everything this pane draws.
    pub fn load(self: &Rc<Self>) {
        let settings = Rc::clone(&self.settings);
        spawn(async move {
            settings.refresh_setup().await;
            settings.refresh_pane(SettingsPane::Channels).await;
        });
    }

    /// Go back one page, where a channel's own page is showing.
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
        let pane = Rc::downgrade(self);
        self.settings.observe(move |change| {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            match change {
                Change::Setup | Change::Sections => pane.draw(),
                Change::Section(section) if section.starts_with(PREFIX) => pane.draw(),
                Change::Row((section, _)) if section.starts_with(PREFIX) => pane.draw(),
                _ => {}
            }
        });
    }

    /// The channels the daemon published, with the section each renders from.
    fn channels(&self) -> Vec<(SetupChannelRow, String)> {
        let state = self.settings.state();
        let Some(setup) = state.setup.as_ref() else {
            return Vec::new();
        };

        setup
            .channels
            .iter()
            .map(|channel| {
                let section = format!("{PREFIX}{}", channel.name);
                (channel.clone(), section)
            })
            .collect()
    }

    fn draw(self: &Rc<Self>) {
        let channels = self.channels();
        let shape: Vec<String> = channels
            .iter()
            .map(|(channel, _)| channel.name.clone())
            .collect();

        if *self.shape.borrow() != shape {
            self.rebuild(&channels);
            self.shape.replace(shape);
        }

        for (channel, section) in &channels {
            self.update(channel, section);
        }
    }

    fn rebuild(self: &Rc<Self>, channels: &[(SetupChannelRow, String)]) {
        for held in self.rows.borrow_mut().values() {
            self.group.remove(&held.row);
            self.group.remove(held.caption.row());
        }
        self.rows.borrow_mut().clear();

        for (channel, section) in channels {
            let built = self.build_row(channel, section);
            self.group.add(&built.row);
            self.group.add(built.caption.row());
            self.rows.borrow_mut().insert(channel.name.clone(), built);
        }
    }

    fn build_row(self: &Rc<Self>, channel: &SetupChannelRow, section: &str) -> Row {
        let updating = Rc::new(Cell::new(false));
        let row = adw::SwitchRow::builder()
            .title(self.title_of(section, &channel.name))
            .activatable(true)
            .build();

        row.add_prefix(&mark::slot(MarkKind::Channel, &channel.name));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        {
            let pane = Rc::clone(self);
            let name = channel.name.clone();
            row.connect_activated(move |_| pane.open(&name));
        }
        {
            let pane = Rc::clone(self);
            let section = section.to_string();
            let updating = Rc::clone(&updating);
            row.connect_active_notify(move |row| {
                if updating.get() {
                    return;
                }
                pane.write(&section, row.is_active());
            });
        }

        Row {
            row,
            caption: CaptionRow::new(),
            updating,
        }
    }

    /// The daemon's own title for one channel's section, which is what a person
    /// calls the channel. A channel the setup state names before the inventory
    /// has been read keeps the name the daemon wrote.
    fn title_of(&self, section: &str, fallback: &str) -> String {
        self.settings
            .state()
            .sections
            .iter()
            .find(|published| published.id == section)
            .map(|published| published.title.clone())
            .unwrap_or_else(|| fallback.to_string())
    }

    fn update(&self, channel: &SetupChannelRow, section: &str) {
        let held = self.rows.borrow();
        let Some(built) = held.get(&channel.name) else {
            return;
        };

        built.row.set_title(&self.title_of(section, &channel.name));
        built.row.set_subtitle(&standing_of(channel));

        let state = self.settings.state();
        let key = enabled_key(&state, section);
        let id = (section.to_string(), key.clone());
        let value = state
            .value_of(&id)
            .or_else(|| state.daemon_value(section, &key));

        // The daemon's own answer, laid over by whatever is not confirmed yet.
        let on = match value {
            Some(SettingValue::Toggle(on)) => on,
            _ => channel.enabled,
        };

        built.updating.set(true);
        built.row.set_active(on);
        built.updating.set(false);

        built
            .caption
            .set(state.refusal(&id).map(|sentence| sentence.text.as_str()));
    }

    /// Write one channel's enable answer, explicitly.
    fn write(self: &Rc<Self>, section: &str, enabled: bool) {
        let settings = Rc::clone(&self.settings);
        let section = section.to_string();
        spawn(async move {
            let key = enabled_key(&settings.state(), &section);
            if key.is_empty() {
                return;
            }
            settings
                .apply(&section, &key, SettingValue::Toggle(enabled))
                .await;
        });
    }

    /// Open one channel's own page, building it the first time.
    fn open(self: &Rc<Self>, name: &str) {
        let section = format!("{PREFIX}{name}");
        let page = self.pages.borrow().get(name).cloned();
        let page = match page {
            Some(page) => page,
            None => {
                let page = self.build_page(&section, name);
                self.view.add(&page);
                self.pages
                    .borrow_mut()
                    .insert(name.to_string(), page.clone());
                page
            }
        };

        self.view.push(&page);

        let settings = Rc::clone(&self.settings);
        spawn(async move {
            settings.refresh_section(&section).await;
        });
    }

    /// One channel's page: its own rows, credentials first.
    fn build_page(self: &Rc<Self>, section: &str, name: &str) -> adw::NavigationPage {
        let form = DescriptorForm::restricted(
            Rc::clone(&self.settings),
            SettingsPane::Channels,
            vec![section.to_string()],
            None,
        );
        form.set_order(secrets_first);

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&form.widget());

        adw::NavigationPage::builder()
            .title(self.title_of(section, name))
            .child(&crate::ui::scrolled(&crate::ui::clamp(&column)))
            .build()
    }
}

/// Credential rows first, then the rest, each half in the daemon's own order.
fn secrets_first(rows: &mut [SettingsRow]) {
    rows.sort_by_key(|row| u8::from(row.kind != SettingsRowKind::Secret));
}

/// The key that turns one channel on.
///
/// Read off the section the daemon published rather than composed from the
/// channel's name: the toggle is whichever row that section carries, and it is
/// the last one, after the credentials it gates.
fn enabled_key(state: &State, section: &str) -> String {
    state
        .rows(section)
        .iter()
        .rev()
        .find(|row| row.kind == SettingsRowKind::Toggle)
        .map(|row| row.key.clone())
        .unwrap_or_default()
}

/// Where one channel stands, in the word the daemon's own atom renders as.
fn standing_of(channel: &SetupChannelRow) -> String {
    match ChannelStanding::of(channel.status.as_deref()) {
        ChannelStanding::Working => copy::text(Key::ChannelStatusWorking),
        ChannelStanding::NotFinished => copy::text(Key::ChannelStatusNotFinished),
        // The daemon said nothing about this channel, so neither does the row.
        ChannelStanding::Unreported => String::new(),
    }
}
