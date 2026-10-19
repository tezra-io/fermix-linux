//! A reply's place cards (`fermix_client::places`), under its words: one stock row per place,
//! which opens the place in this computer's maps app or on the web map, with its website and
//! address in its menu. The words already link every place; the cards only add to them.

use super::entries::{copy_text, toast};
use adw::prelude::*;
use fermix_client::places::{maps_launch, Card};
use gtk::{gio, glib};

/// No wider than a card reads well, and no narrower than a short name would make it, so the
/// cards of one reply after another line up; it still fits the narrowest window.
pub(super) const CARDS_WIDTH: i32 = 480;
pub(super) const CARDS_MIN_WIDTH: i32 = 280;
const NO_MAP: &str = "Your browser did not open the map.";
const NO_WEBSITE: &str = "Your browser did not open the website.";

/// The cards under one reply; hidden until its row carries places.
pub struct Places {
    pub widget: adw::Clamp,
    list: gtk::ListBox,
    shown: Vec<Card>,
}

impl Places {
    pub fn new() -> Places {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .width_request(CARDS_MIN_WIDTH)
            .css_classes(["boxed-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Places")]);
        let widget = adw::Clamp::builder()
            .maximum_size(CARDS_WIDTH)
            .halign(gtk::Align::Start)
            .child(&list)
            .visible(false)
            .build();
        Places {
            widget,
            list,
            shown: Vec::new(),
        }
    }

    /// Shows `cards`, drawing the rows again only when they differ from those shown: at most
    /// once per finished reply, or per read that changed its places.
    pub fn show(&mut self, cards: &[Card]) {
        if self.shown == cards {
            return;
        }
        self.list.remove_all();
        for card in cards {
            self.list.append(&card_row(card));
        }
        self.shown = cards.to_vec();
        self.widget.set_visible(!cards.is_empty());
    }
}

/// One place: its name over what the source says of it. A click or Return opens it in Maps;
/// its menu opens Maps, its website and copies its address, each only where it has one.
fn card_row(card: &Card) -> adw::ActionRow {
    // Markup off before any text: names and hours are the source's own words.
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(&card.name);
    row.set_title_lines(1);
    row.set_subtitle(&card.lines.join("\n"));
    row.set_subtitle_lines(2);
    row.set_tooltip_text(Some(&card.name));
    row.set_activatable(true);
    // The row's own type does not carry `Accessible` in these bindings; its widget does.
    let accessible = row.upcast_ref::<gtk::Widget>();
    accessible.update_property(&[gtk::accessible::Property::Label(&card.spoken)]);
    let pin = gtk::Image::from_icon_name("mark-location-symbolic");
    pin.add_css_class("dimmed");
    pin.set_accessible_role(gtk::AccessibleRole::Presentation);
    row.add_prefix(&pin);
    row.add_suffix(&menu_button(card));
    row.insert_action_group("place", Some(&card_actions(&row, card)));
    let place = card.clone();
    row.connect_activated(move |row| open_in_maps(row, &place));
    row
}

fn menu_button(card: &Card) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(Some("Open in Maps"), Some("place.open-maps"));
    if card.website.is_some() {
        menu.append(Some("Open website"), Some("place.open-website"));
    }
    if card.address.is_some() {
        menu.append(Some("Copy address"), Some("place.copy-address"));
    }
    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .tooltip_text("More")
        .css_classes(["flat", "circular"])
        .build();
    let label = format!("More for {}", card.name);
    button.update_property(&[gtk::accessible::Property::Label(&label)]);
    button
}

/// `place.*` for the row's menu. The actions hold the row weakly: it owns them.
fn card_actions(row: &adw::ActionRow, card: &Card) -> gio::SimpleActionGroup {
    let group = gio::SimpleActionGroup::new();
    let on = |name: &str, act: fn(&adw::ActionRow, &Card)| {
        let action = gio::SimpleAction::new(name, None);
        let (row, card) = (row.downgrade(), card.clone());
        action.connect_activate(move |_, _| {
            if let Some(row) = row.upgrade() {
                act(&row, &card);
            }
        });
        group.add_action(&action);
    };
    on("open-maps", open_in_maps);
    on("open-website", open_website);
    on("copy-address", |row, card| {
        if let Some(address) = &card.address {
            copy_text(row, address);
        }
    });
    group
}

/// The place in this computer's maps app, or on the web map where it has none, or where the app
/// fails to open. A launch the person turned down stays turned down.
fn open_in_maps(widget: &impl IsA<gtk::Widget>, card: &Card) {
    // The deb and rpm run unsandboxed and see the desktop's own handlers. The Flatpak dev build
    // sees only its sandbox's, which has none for `geo:`, so it always opens the web map.
    let handler = gio::AppInfo::default_for_uri_scheme("geo").is_some();
    let launch = maps_launch(card, handler);
    let Some(window) = window_of(widget) else {
        return;
    };
    glib::spawn_future_local(async move {
        let Err(e) = launched(&window, &launch.first).await else {
            return;
        };
        if turned_down(&e) {
            return;
        }
        glib::g_warning!("fermix", "the place did not open: {e}");
        let Some(fallback) = launch.fallback else {
            return toast(&window, NO_MAP);
        };
        if let Err(e) = launched(&window, &fallback).await {
            glib::g_warning!("fermix", "the web map did not open: {e}");
            toast(&window, NO_MAP);
        }
    });
}

fn open_website(row: &adw::ActionRow, card: &Card) {
    let Some(website) = card.website.clone() else {
        return;
    };
    let Some(window) = window_of(row) else {
        return;
    };
    glib::spawn_future_local(async move {
        let Err(e) = launched(&window, &website).await else {
            return;
        };
        if !turned_down(&e) {
            glib::g_warning!("fermix", "the website did not open: {e}");
            toast(&window, NO_WEBSITE);
        }
    });
}

/// The card's window, found before a launch: a reply redrawn while the launch runs takes its
/// row out of the window, and a failure still has to reach the person there.
pub(super) fn window_of(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    let window = widget.root().and_downcast::<gtk::Window>();
    if window.is_none() {
        glib::g_warning!("fermix", "a card outside a window opened nothing");
    }
    window
}

/// Opens `uri` the desktop's way: the OpenURI portal inside a sandbox, GIO outside one.
pub(super) async fn launched(window: &gtk::Window, uri: &str) -> Result<(), glib::Error> {
    gtk::UriLauncher::new(uri).launch_future(Some(window)).await
}

/// The person closed a chooser rather than the launch failing.
pub(super) fn turned_down(e: &glib::Error) -> bool {
    e.matches(gio::IOErrorEnum::Cancelled)
        || e.matches(gtk::DialogError::Dismissed)
        || e.matches(gtk::DialogError::Cancelled)
}
