//! Providers: one row per provider, every way in on the row itself (design_final §2, §3).
//! Buttons and menu items fire window actions with the provider id as target; the
//! controller in `app.rs` does the work and this page redraws from `State`.

use crate::home::action_button;
use crate::marks::{mark, Kind};
use crate::state::{Connection, Snapshot, State};
use crate::status::{down_view, waiting, DownPage};
use adw::prelude::*;
use fermix_client::model::ProviderRow;
use fermix_client::providers::{connection, Connection as Link, Door};
use fermix_client::view::{row_view, CopyLink, MenuItem, RowView, Suffix};
use gtk::{gio, glib};
use std::cell::RefCell;
use std::time::Instant;

const BROWSER_TOOLTIP: &str = "Opens your browser. Fermix never sees your password.";

struct Row {
    id: String,
    row: adw::ActionRow,
    /// Crossfades between what the row offers, so a sign-in's steps do not jump.
    suffix: gtk::Stack,
    last: RefCell<Option<RowView>>,
}

pub struct ProvidersPage {
    pub root: gtk::Stack,
    /// The list's page; Settings adds the shared routing section beneath it.
    pub page: adw::PreferencesPage,
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<Row>>,
    down: DownPage,
}

impl ProvidersPage {
    pub fn new() -> Self {
        let group = adw::PreferencesGroup::new();
        let page = adw::PreferencesPage::new();
        page.add(&group);
        let down = DownPage::new();
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        root.add_named(&page, Some("list"));
        root.add_named(&down.page, Some("down"));
        ProvidersPage {
            root,
            page,
            group,
            rows: RefCell::default(),
            down,
        }
    }

    pub fn render(&self, state: &State, now: Instant) {
        let down = match &state.connection {
            Connection::Up(snapshot) => return self.render_list(state, snapshot, now),
            Connection::Connecting => waiting("Connecting to Fermix"),
            Connection::Down(_) if state.waking => waiting("Starting Fermix"),
            Connection::Down(problem) => down_view(
                problem,
                state.wake_failed,
                "Providers are read from Fermix. Start it to see them.",
            ),
        };
        self.down.show(down);
        self.root.set_visible_child_name("down");
    }

    fn render_list(&self, state: &State, snapshot: &Snapshot, now: Instant) {
        let providers = &snapshot.state.providers;
        self.ensure_rows(providers);
        for (row, provider) in self.rows.borrow().iter().zip(providers) {
            let view = row_view(
                provider,
                &state.activity(&row.id),
                state.recent(&row.id, now),
            );
            update_row(row, provider, view);
        }
        self.group
            .set_description(Some(list_description(providers)));
        self.down.reset();
        self.root.set_visible_child_name("list");
    }

    /// Rebuilds the list only when the set or order of providers changed.
    fn ensure_rows(&self, providers: &[ProviderRow]) {
        let same = {
            let rows = self.rows.borrow();
            rows.len() == providers.len() && rows.iter().zip(providers).all(|(r, p)| r.id == p.id)
        };
        if same {
            return;
        }
        for old in self.rows.borrow_mut().drain(..) {
            self.group.remove(&old.row);
        }
        let fresh: Vec<Row> = providers.iter().map(new_row).collect();
        for row in &fresh {
            self.group.add(&row.row);
        }
        *self.rows.borrow_mut() = fresh;
    }
}

fn list_description(providers: &[ProviderRow]) -> &'static str {
    if providers.iter().any(|p| connection(p) == Link::Connected) {
        "Fermix answers with your primary provider."
    } else {
        "Fermix answers with your primary provider. Sign in to one to start."
    }
}

fn new_row(provider: &ProviderRow) -> Row {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&provider.label))
        .build();
    row.add_prefix(&mark(Kind::Provider, &provider.id));
    let suffix = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .hhomogeneous(false)
        .interpolate_size(true)
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&suffix);
    // The row itself opens the provider's own settings: model, effort, sign-in route.
    row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    row.set_activatable(true);
    let id = provider.id.clone();
    row.connect_activated(move |row| {
        let target = id.to_variant();
        if let Err(e) = row.activate_action("win.provider-settings", Some(&target)) {
            glib::g_warning!("fermix", "the provider's settings could not open: {e}");
        }
    });
    Row {
        id: provider.id.clone(),
        row,
        suffix,
        last: RefCell::default(),
    }
}

/// Redraws a row only when what it shows changed, so an open menu survives a refresh.
fn update_row(row: &Row, provider: &ProviderRow, view: RowView) {
    if row.last.borrow().as_ref() == Some(&view) {
        return;
    }
    // An account label or a daemon sentence may hold markup characters.
    row.row
        .set_subtitle(&glib::markup_escape_text(&view.subtitle));
    let fresh = gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    if view.is_error {
        let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
        warning.add_css_class("error");
        fresh.append(&warning);
    }
    for widget in suffix_widgets(provider, &view.suffix) {
        fresh.append(&widget);
    }
    swap_suffix(&row.suffix, &fresh);
    *row.last.borrow_mut() = Some(view);
}

/// Crossfades to `fresh`. The child still fading out from the last swap is
/// dropped now; the one on screen stays until it has faded.
fn swap_suffix(stack: &gtk::Stack, fresh: &gtk::Box) {
    let on_screen = stack.visible_child();
    let mut child = stack.first_child();
    while let Some(old) = child {
        child = old.next_sibling();
        if Some(&old) != on_screen.as_ref() {
            stack.remove(&old);
        }
    }
    stack.add_child(fresh);
    stack.set_visible_child(fresh);
}

fn suffix_widgets(provider: &ProviderRow, suffix: &Suffix) -> Vec<gtk::Widget> {
    let id = provider.id.as_str();
    let (lead, more) = match suffix {
        Suffix::Busy { copy_link, cancel } => return busy_widgets(id, *copy_link, *cancel),
        Suffix::Resting { lead, more } => (lead, more),
    };
    let mut widgets = Vec::new();
    if let Some(lead) = lead {
        let button = action_button(&lead.verb, "win.door", Some(&lead.door.target(id)));
        if lead.door == Door::BrowserSignIn {
            button.set_tooltip_text(Some(BROWSER_TOOLTIP));
        }
        widgets.push(button.upcast());
    }
    if !more.is_empty() {
        widgets.push(menu_button(id, more).upcast());
    }
    widgets
}

fn busy_widgets(id: &str, copy_link: CopyLink, cancel: bool) -> Vec<gtk::Widget> {
    let mut widgets: Vec<gtk::Widget> = vec![adw::Spinner::new().upcast()];
    if copy_link != CopyLink::None {
        let copy = action_button("Copy link", "win.copy-link", Some(id));
        copy.set_tooltip_text(Some("Copy the sign-in link to open it in any browser"));
        if copy_link == CopyLink::Rescue {
            copy.add_css_class("suggested-action");
        }
        widgets.push(copy.upcast());
    }
    if cancel {
        widgets.push(action_button("Cancel", "win.cancel", Some(id)).upcast());
    }
    widgets
}

fn menu_button(id: &str, items: &[MenuItem]) -> gtk::MenuButton {
    let menu = gio::Menu::new();
    for item in items {
        let (action, target) = match item {
            MenuItem::MakePrimary => ("win.make-primary", id.to_owned()),
            MenuItem::Door(door) => ("win.door", door.target(id)),
            MenuItem::ReplaceKey => ("win.door", Door::ApiKey.target(id)),
            MenuItem::RemoveKey => ("win.remove-key", id.to_owned()),
            MenuItem::SignOut => ("win.sign-out", id.to_owned()),
        };
        menu.append_item(&menu_item(&item.label(), action, &target));
    }
    gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .tooltip_text("More")
        .build()
}

fn menu_item(label: &str, action: &str, target: &str) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
    item
}
