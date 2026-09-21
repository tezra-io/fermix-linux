//! Ready.
//!
//! The mascot and the title, the status line, the provider and model in use,
//! and the next steps. Every fact on it is the daemon's own.
//!
//! There is no command-line row: the package puts `fermix` on the path, so
//! there is nothing to copy and nothing to plan (M38 section 5.5). The voice
//! companion row is not here either, because there is no Linux companion to
//! open; its place is taken by the one row that opens Home when advisory
//! failures remain.

use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::SettingsPane;
use crate::models::home;
use crate::models::onboarding::{OnboardingModel, Snapshot};
use crate::models::SettingsModel;

use super::Screen;
use crate::ui::plain;

/// How tall the mascot draws here. The same artwork as Welcome, at the same
/// size: this is the other end of the same journey.
const MASCOT_HEIGHT: i32 = 120;

/// The Ready screen.
pub struct ReadyScreen {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    #[allow(dead_code)]
    model: Rc<OnboardingModel>,
    status: gtk::Label,
    provider: gtk::Label,
    attention: adw::ActionRow,
}

impl ReadyScreen {
    /// Build it over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, model: &Rc<OnboardingModel>) -> Rc<Self> {
        let column = super::screen_column();
        column.set_valign(gtk::Align::Center);

        column.append(&mascot());
        column.append(&title());

        let status = line();
        let provider = line();
        column.append(&status);
        column.append(&provider);

        let (group, channels, attention) = next_steps();
        column.append(&group);

        let screen = Rc::new(Self {
            root: super::screen(&column),
            settings,
            model: Rc::clone(model),
            status,
            provider,
            attention,
        });

        screen.connect(&channels);
        screen
    }

    /// The two next steps go where every other route to a surface goes: the
    /// pane selection is the model's, and Home is the window's own action.
    fn connect(self: &Rc<Self>, channels: &adw::ActionRow) {
        let screen = Rc::clone(self);
        channels.connect_activated(move |row| {
            crate::ui::open_pane(row, &screen.settings, SettingsPane::Channels);
        });

        self.attention.connect_activated(|row| {
            let _ = WidgetExt::activate_action(row, "win.home", None);
        });
    }

    /// The status word, which is the same one Home shows.
    fn status_line(&self) -> String {
        let state = self.settings.state();
        copy::text(home::status_word(&state).key())
    }

    /// The provider and the model in use, as the daemon names them.
    fn provider_line(&self) -> String {
        let state = self.settings.state();
        home::facts(&state, None, self.settings.api().negotiated_version())
            .into_iter()
            .find(|fact| fact.label == Key::HomeRuntimeProvider)
            .map(|fact| fact.value)
            .unwrap_or_default()
    }
}

impl Screen for ReadyScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        self.status.set_label(&self.status_line());
        self.provider.set_label(&self.provider_line());

        // One row, and only while something is still owed. The daemon's own
        // advisory failures are what put it there.
        self.attention.set_visible(snapshot.advisory > 0);
    }
}

fn mascot() -> gtk::Picture {
    let picture = gtk::Picture::for_resource(&format!(
        "{}/brand/fermix-mascot.png",
        crate::app::RESOURCE_PREFIX
    ));
    picture.set_content_fit(gtk::ContentFit::ScaleDown);
    picture.set_height_request(MASCOT_HEIGHT);
    picture.set_can_shrink(true);
    picture.set_halign(gtk::Align::Center);
    picture
}

fn title() -> gtk::Label {
    let label = gtk::Label::builder()
        .label(copy::text(Key::SetupReadyTitle))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    label.add_css_class("title-1");
    label
}

fn line() -> gtk::Label {
    let label = gtk::Label::builder()
        .label("")
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    label.add_css_class("dim-label");
    label
}

/// The next steps: one row that opens the pane where channels are connected,
/// and one that opens Home while anything is still owed.
///
/// There is no voice companion row: the companion exists for macOS today and
/// there is none for Linux, so its place is the one row that says something
/// still needs attention.
fn next_steps() -> (adw::PreferencesGroup, adw::ActionRow, adw::ActionRow) {
    let group = adw::PreferencesGroup::builder()
        .title(copy::text(Key::SetupReadyNext))
        .build();

    let channels = opens(Key::SetupReadyChannels);
    let attention = opens(Key::SetupReadyAttention);
    attention.set_visible(false);

    group.add(&channels);
    group.add(&attention);
    (group, channels, attention)
}

/// A row that opens something says so with the toolkit's own affordance.
fn opens(title: Key) -> adw::ActionRow {
    let row = plain(
        adw::ActionRow::builder()
            .title(copy::text(title))
            .activatable(true)
            .build(),
    );
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row
}
