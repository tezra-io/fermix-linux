//! About you.
//!
//! Four rows, one write. Zero typing is a valid answer: every field is
//! prefilled from what the daemon already holds, and from this account where it
//! holds nothing. Nothing is written here — Applying is the screen that writes,
//! and it writes all four keys at once so the daemon takes them whole or
//! refuses them whole.
//!
//! A refused write comes back to this screen with the daemon's own sentence
//! under the rows, and the answers a person typed are still in them.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{SettingsOption, SettingsRow};
use crate::metrics;
use crate::models::onboarding::{Answers, Block, OnboardingModel, Snapshot, PERSONALIZATION};
use crate::models::{spawn, SettingsModel};
use crate::ui::CaptionRow;

use super::Screen;
use crate::ui::plain;

/// Above this many zones the dialog is searched rather than read. The count is
/// the daemon's, so nothing here keys on which row this is.
const SEARCHABLE_ABOVE: usize = 12;

/// The About you screen.
pub struct AboutYouScreen {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    model: Rc<OnboardingModel>,
    name: adw::EntryRow,
    zone: adw::ActionRow,
    zone_value: gtk::Label,
    style: adw::ComboRow,
    assistant: adw::EntryRow,
    refusal: CaptionRow,
    blocked: CaptionRow,
    /// The style options as the daemon published them, in list order.
    styles: RefCell<Vec<SettingsOption>>,
    /// The zones as the daemon published them.
    zones: RefCell<Vec<SettingsOption>>,
    /// Set while the screen is writing the model's own values into its rows, so
    /// a row's handler can tell a person's typing from a redraw.
    updating: std::cell::Cell<bool>,
}

impl AboutYouScreen {
    /// Build it over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, model: &Rc<OnboardingModel>) -> Rc<Self> {
        let column = super::screen_column();
        column.append(&crate::ui::caption(&copy::text(Key::SetupAboutYouBody)));

        let name = entry(Key::SetupFieldYourName);
        let assistant = entry(Key::SetupFieldAssistantName);
        let (zone, zone_value) = zone_row();
        let style = plain(
            adw::ComboRow::builder()
                .title(copy::text(Key::SetupFieldStyle))
                .build(),
        );

        let group = adw::PreferencesGroup::new();
        group.add(&name);
        group.add(&zone);
        group.add(&style);
        group.add(&assistant);
        column.append(&group);

        let refusal = CaptionRow::new();
        let blocked = CaptionRow::new();
        column.append(&crate::ui::caption_group(&refusal));
        column.append(&crate::ui::caption_group(&blocked));

        let screen = Rc::new(Self {
            root: super::screen(&column),
            settings,
            model: Rc::clone(model),
            name,
            zone,
            zone_value,
            style,
            assistant,
            refusal,
            blocked,
            styles: RefCell::new(Vec::new()),
            zones: RefCell::new(Vec::new()),
            updating: std::cell::Cell::new(false),
        });

        screen.connect();
        screen
    }

    fn connect(self: &Rc<Self>) {
        for entry in [&self.name, &self.assistant] {
            let screen = Rc::clone(self);
            entry.connect_changed(move |_| screen.collect());
        }

        let screen = Rc::clone(self);
        self.style
            .connect_selected_notify(move |_| screen.collect());

        let screen = Rc::clone(self);
        self.zone
            .connect_activated(move |row| screen.choose_zone(row));
    }

    /// Take what the rows say into the model. Nothing is written to the daemon.
    fn collect(&self) {
        if self.updating.get() {
            return;
        }

        let styles = self.styles.borrow();
        let style = styles
            .get(self.style.selected() as usize)
            .map(|option| option.value.clone())
            .unwrap_or_default();

        self.model.set_answers(Answers {
            name: self.name.text().to_string(),
            timezone: self.zone_value.label().to_string(),
            style,
            assistant: self.assistant.text().to_string(),
        });
    }

    /// Read the section this screen writes, then fill the rows from it.
    fn load(self: &Rc<Self>) {
        let screen = Rc::clone(self);
        let settings = Rc::clone(&self.settings);

        spawn(async move {
            settings.refresh_section(PERSONALIZATION).await;
            let rows = settings.state().rows(PERSONALIZATION);
            screen.model.prefill(&rows);
            screen.fill(&rows);
        });
    }

    /// Put the daemon's own options and the model's answers into the rows.
    fn fill(&self, rows: &[SettingsRow]) {
        self.styles.replace(options(rows, "communication_style"));
        self.zones.replace(options(rows, "timezone"));

        let styles = self.styles.borrow().clone();
        let labels: Vec<&str> = styles.iter().map(|option| option.label.as_str()).collect();
        let list = gtk::StringList::new(&labels);

        let answers = self.model.answers();
        self.updating.set(true);
        self.style.set_model(Some(&list));
        self.style.set_selected(index_of(&styles, &answers.style));
        self.name.set_text(&answers.name);
        self.assistant.set_text(&answers.assistant);
        self.write_zone(&answers.timezone);
        self.updating.set(false);
    }

    /// The zone row shows the daemon's own label for a zone it published, and
    /// the identifier itself for one it did not.
    fn write_zone(&self, zone: &str) {
        let label = self
            .zones
            .borrow()
            .iter()
            .find(|option| option.value == zone)
            .map(|option| option.label.clone())
            .unwrap_or_else(|| zone.to_string());

        self.zone_value.set_label(zone);
        self.zone.set_subtitle(&label);
    }

    /// The searchable zone dialog: the daemon's own list, and nothing read off
    /// this host.
    fn choose_zone(self: &Rc<Self>, anchor: &adw::ActionRow) {
        let zones = self.zones.borrow().clone();
        if zones.is_empty() {
            return;
        }

        let screen = Rc::clone(self);
        zone_dialog(&zones, anchor, move |chosen| {
            screen.write_zone(&chosen);
            screen.collect();
        });
    }
}

impl Screen for AboutYouScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        self.refusal.set(
            snapshot
                .refusal
                .as_ref()
                .map(|sentence| sentence.text.as_str()),
        );

        self.blocked.set(
            match snapshot.block {
                Some(Block::Personalization) => Some(copy::text(Block::Personalization.key())),
                _ => None,
            }
            .as_deref(),
        );
    }

    fn shown(self: Rc<Self>) {
        self.load();
    }
}

fn entry(label: Key) -> adw::EntryRow {
    plain(
        adw::EntryRow::builder()
            .title(copy::text(label))
            // Enter in a field commits it and reaches the bar's one suggested
            // action, which is the same key the rest of the product commits
            // with.
            .activates_default(true)
            .build(),
    )
}

/// The zone row: a row that opens something, which says so.
fn zone_row() -> (adw::ActionRow, gtk::Label) {
    let row = plain(
        adw::ActionRow::builder()
            .title(copy::text(Key::SetupFieldTimeZone))
            .activatable(true)
            .build(),
    );
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

    // The identifier the daemon takes, kept beside the label a person reads.
    let value = gtk::Label::new(Some(""));
    value.set_visible(false);
    row.add_prefix(&value);

    (row, value)
}

/// One option list off a published row.
fn options(rows: &[SettingsRow], key: &str) -> Vec<SettingsOption> {
    rows.iter()
        .find(|row| row.key == key)
        .map(|row| row.options.clone())
        .unwrap_or_default()
}

fn index_of(options: &[SettingsOption], value: &str) -> u32 {
    options
        .iter()
        .position(|option| option.value == value)
        .unwrap_or(0) as u32
}

/// The zone picker: the daemon's own zones, searched when there are enough of
/// them to be worth searching.
fn zone_dialog(
    zones: &[SettingsOption],
    anchor: &impl IsA<gtk::Widget>,
    chosen: impl Fn(String) + 'static,
) {
    let list = gtk::ListBox::new();
    list.add_css_class("navigation-sidebar");
    for zone in zones {
        list.append(&plain(
            adw::ActionRow::builder()
                .title(zone.label.as_str())
                .subtitle(zone.hint.clone().unwrap_or_default())
                .activatable(true)
                .build(),
        ));
    }

    let search = gtk::SearchEntry::builder()
        .visible(zones.len() > SEARCHABLE_ABOVE)
        .placeholder_text(copy::text(Key::ActionSearchAccessible))
        .build();
    connect_search(&search, &list);

    let column = crate::ui::column();
    column.append(&search);
    column.append(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(metrics::CLAMP_TIGHTENING)
            .propagate_natural_height(true)
            .child(&list)
            .build(),
    );

    let header = adw::HeaderBar::new();
    let toolbar = adw::ToolbarView::builder().content(&column).build();
    toolbar.add_top_bar(&header);

    let dialog = adw::Dialog::builder()
        .title(copy::text(Key::SetupFieldTimeZone))
        .child(&toolbar)
        .build();

    let values: Vec<String> = zones.iter().map(|zone| zone.value.clone()).collect();
    let picked = dialog.clone();
    list.connect_row_activated(move |_, row| {
        // A row keeps its index while it is filtered, so the index is what says
        // which zone was chosen.
        if let Some(value) = values.get(row.index().max(0) as usize) {
            chosen(value.clone());
        }
        picked.close();
    });

    dialog.present(Some(anchor.as_ref()));
    search.grab_focus();
}

/// Filtering hides rows rather than rebuilding them, so a zone keeps the index
/// its value is found by.
fn connect_search(search: &gtk::SearchEntry, list: &gtk::ListBox) {
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
