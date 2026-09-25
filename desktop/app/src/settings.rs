//! Settings: thirteen panes in four groups (M38 §5.7), a searchable pane list
//! that replaces the sidebar, and the banner for a settings file changed or
//! broken outside Fermix. Most panes are the daemon's sections drawn by
//! `descriptor.rs`; Providers, Channels and Permissions add rows of their own.

use crate::capability_panes::{ComputerPane, Facts, MeetingsPane, Running, MEETINGS_SIGN_IN};
use crate::channels::ChannelsPane;
use crate::descriptor::{Drawn, SectionView};
use crate::state::{Connection, State};
use crate::status::{down_view, waiting, DownPage};
use adw::prelude::*;
use fermix_client::capabilities::{ComputerPermissions, COMPUTER_SIDECAR, MEETBOT};
use fermix_client::ledger::{
    MICROPHONE_STATEMENT, PLATFORM_FACT, RIGHTS, VOICE_COMPANION_STATEMENT,
};
use fermix_client::model::DetectRow;
use fermix_client::settings::{pane, sections_for, Section, SectionRows, GROUPS, PANES};
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// Panes drawn entirely from the daemon's sections, plus any intro above them.
const DESCRIPTOR_PANES: [&str; 7] = [
    "personality",
    "memory",
    "voice",
    "coding",
    "search",
    "images",
    "sandbox",
];

/// What the controller knows about settings, beside `State`.
#[derive(Default)]
pub struct SettingsData {
    pub sections: Option<Vec<Section>>,
    pub rows: HashMap<String, SectionRows>,
    /// The daemon's refusal sentence, by (section, row key).
    pub errors: HashMap<(String, String), String>,
    /// Sections being read now, so a pane opened twice reads them once.
    pub reading: HashSet<String>,
    /// The daemon process the cache was read from; a restart empties it.
    pub pid: Option<String>,
    /// The parser's sentence, known only once a write was refused for it.
    pub unreadable: Option<String>,
    /// Capability jobs being followed, by target (or `meetings_signin`).
    pub jobs: HashMap<String, Running>,
    /// Why a capability's last install did not finish, by target.
    pub job_failures: HashMap<String, String>,
    /// The notetaker's last detection; `None` until read or when the probe failed.
    pub meetbot: Option<DetectRow>,
    /// The computer-use helper's last probe, or why it could not be read.
    pub probe: Option<Result<ComputerPermissions, String>>,
}

impl SettingsData {
    pub fn drawn(&self, section: &str, locked: bool) -> Drawn {
        let mut errors: Vec<(String, String)> = self
            .errors
            .iter()
            .filter(|((s, _), _)| s == section)
            .map(|((_, key), sentence)| (key.clone(), sentence.clone()))
            .collect();
        errors.sort();
        Drawn {
            rows: self.rows.get(section).cloned(),
            errors,
            locked,
        }
    }
}

pub struct SettingsPage {
    pub root: gtk::Stack,
    pub panes: gtk::Stack,
    pub sidebar: gtk::Box,
    pub list: gtk::ListBox,
    pub banner: adw::Banner,
    pub channels: ChannelsPane,
    meetings: MeetingsPane,
    computer: ComputerPane,
    search: gtk::SearchEntry,
    /// Pages whose groups are the daemon's sections, by pane slug.
    pages: HashMap<&'static str, adw::PreferencesPage>,
    /// The Providers pane's own page, which gains the shared routing section.
    providers_page: adw::PreferencesPage,
    views: RefCell<Vec<Rc<SectionView>>>,
    /// Views inside open dialogs; each dialog removes its own when it closes.
    pub dialog_views: Rc<RefCell<Vec<Rc<SectionView>>>>,
    /// Pane slugs the search leaves visible.
    visible: Rc<RefCell<Vec<&'static str>>>,
    down: DownPage,
}

impl SettingsPage {
    /// `providers` is the Providers pane built by `providers.rs`, and `providers_page` its list;
    /// `integrations` is the pane `integrations.rs` builds and feeds itself.
    pub fn new(
        providers: &gtk::Widget,
        providers_page: &adw::PreferencesPage,
        integrations: &adw::PreferencesPage,
    ) -> SettingsPage {
        let panes = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        panes.add_named(providers, Some("providers"));
        let channels = ChannelsPane::new();
        panes.add_named(&channels.page, Some("channels"));
        panes.add_named(&permissions_page(), Some("permissions"));
        panes.add_named(integrations, Some("integrations"));
        let meetings = MeetingsPane::new();
        panes.add_named(&meetings.page, Some("meetings"));
        let computer = ComputerPane::new();
        panes.add_named(&computer.page, Some("computer"));
        let mut pages = HashMap::new();
        for slug in DESCRIPTOR_PANES {
            let page = adw::PreferencesPage::new();
            if let Some(intro) = intro(slug) {
                page.add(&intro);
            }
            panes.add_named(&page, Some(slug));
            pages.insert(slug, page);
        }
        let banner = adw::Banner::new("");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&banner);
        content.append(&panes);
        panes.set_vexpand(true);
        let down = DownPage::new();
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        root.add_named(&content, Some("panes"));
        root.add_named(&down.page, Some("down"));
        let visible = Rc::new(RefCell::new(PANES.iter().map(|p| p.slug).collect()));
        let (sidebar, list, search) = sidebar(&visible);
        SettingsPage {
            root,
            panes,
            sidebar,
            list,
            banner,
            channels,
            meetings,
            computer,
            search,
            pages,
            providers_page: providers_page.clone(),
            views: RefCell::default(),
            dialog_views: Rc::default(),
            visible,
            down,
        }
    }

    pub fn render(&self, state: &State, data: &SettingsData) {
        let Connection::Up(snapshot) = &state.connection else {
            return self.render_down(state);
        };
        self.down.reset();
        self.root.set_visible_child_name("panes");
        if let Some(sections) = &data.sections {
            self.ensure_views(sections);
        }
        let config = snapshot.state.coexistence.config_state.as_str();
        let locked = config != "clear";
        let views = self.views.borrow();
        let dialog_views = self.dialog_views.borrow();
        for view in views.iter().chain(dialog_views.iter()) {
            view.show(data.drawn(&view.id, locked));
        }
        self.channels.render(&snapshot.state.channels, data, locked);
        self.render_capabilities(data, locked, snapshot.architecture.as_deref());
        self.show_banner(config, data.unreadable.as_deref());
    }

    fn render_capabilities(&self, data: &SettingsData, locked: bool, architecture: Option<&str>) {
        let facts = |section: &str, target: &str| Facts {
            rows: data.rows.get(section),
            running: data.jobs.get(target),
            failure: data.job_failures.get(target),
            locked,
        };
        self.meetings.render(
            &facts("meetings", MEETBOT),
            &data.drawn("meetings", locked),
            data.meetbot.as_ref(),
            data.jobs.get(MEETINGS_SIGN_IN),
        );
        self.computer.render(
            &facts("computer_use", COMPUTER_SIDECAR),
            &data.drawn("computer_use", locked),
            data.probe.as_ref(),
            architecture,
        );
    }

    fn render_down(&self, state: &State) {
        let view = match &state.connection {
            Connection::Down(_) if state.waking => waiting("Starting Fermix"),
            Connection::Down(problem) => down_view(
                problem,
                state.wake_failed,
                "Settings are read from Fermix. Start it to change them.",
            ),
            _ => waiting("Connecting to Fermix"),
        };
        self.down.show(view);
        self.root.set_visible_child_name("down");
    }

    /// Adds one group per section to each descriptor pane, once the sections are known.
    fn ensure_views(&self, sections: &[Section]) {
        if !self.views.borrow().is_empty() {
            return;
        }
        let mut views = Vec::new();
        for (slug, page) in &self.pages {
            let title = pane(slug).map_or("", |p| p.title);
            for section in sections_for(slug, sections) {
                let heading = (section.title != title).then_some(section.title.as_str());
                let view = SectionView::new(&section.id, heading);
                page.add(&view.group);
                views.push(view);
            }
        }
        views.push(self.channels.editors_view(sections));
        views.extend(self.providers_views(sections));
        *self.views.borrow_mut() = views;
    }

    /// Providers draws its own list; below it sits the one shared section, "Model behavior".
    fn providers_views(&self, sections: &[Section]) -> Vec<Rc<SectionView>> {
        sections_for("providers", sections)
            .into_iter()
            .filter(|s| !s.id.starts_with("providers."))
            .map(|s| {
                let view = SectionView::new(&s.id, Some(&s.title));
                self.providers_page.add(&view.group);
                view
            })
            .collect()
    }

    fn show_banner(&self, config: &str, parser_sentence: Option<&str>) {
        match config {
            "external_change" => {
                self.banner.set_title(
                    "Settings changed outside Fermix. Reload before changing anything, so \
                     nothing you did elsewhere is overwritten.",
                );
                self.banner.set_button_label(Some("Reload"));
                self.banner.set_action_name(Some("win.settings-reload"));
                self.banner.set_revealed(true);
            }
            "config_unreadable" => {
                let detail = parser_sentence.unwrap_or("Nothing has been changed.");
                self.banner.set_title(&glib::markup_escape_text(&format!(
                    "The settings file cannot be read. {detail}"
                )));
                self.banner.set_button_label(None);
                self.banner.set_action_name(None);
                self.banner.set_revealed(true);
            }
            _ => self.banner.set_revealed(false),
        }
    }

    /// Shows only the panes in `slugs`, and each group heading that still has one.
    pub fn filter(&self, slugs: Vec<&'static str>) {
        *self.visible.borrow_mut() = slugs;
        self.list.invalidate_filter();
    }

    pub fn focus_search(&self) {
        self.search.grab_focus();
    }
}

/// The words above a pane's controls, where the platform owes the reader something first.
fn intro(slug: &str) -> Option<adw::PreferencesGroup> {
    let text = match slug {
        "voice" => format!("{VOICE_COMPANION_STATEMENT}\n\n{MICROPHONE_STATEMENT}"),
        _ => return None,
    };
    let group = adw::PreferencesGroup::new();
    group.set_description(Some(&glib::markup_escape_text(&text)));
    Some(group)
}

/// M38 §7.4, rendered: who holds each right, how to take it back, where it is kept.
fn permissions_page() -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    let fact = adw::PreferencesGroup::new();
    fact.set_description(Some(PLATFORM_FACT));
    page.add(&fact);
    let group = adw::PreferencesGroup::new();
    for right in &RIGHTS {
        let row = adw::ExpanderRow::builder()
            .title(right.title)
            .subtitle(right.principal)
            .build();
        for (title, text) in [
            ("How you take it back", right.revoke),
            ("Where it is kept", right.artifact),
        ] {
            let line = adw::ActionRow::builder()
                .title(title)
                .subtitle(text)
                .css_classes(["property"])
                .build();
            row.add_row(&line);
        }
        group.add(&row);
    }
    page.add(&group);
    page
}

/// The pane list: search, the way back, then each group's panes under its heading.
fn sidebar(visible: &Rc<RefCell<Vec<&'static str>>>) -> (gtk::Box, gtk::ListBox, gtk::SearchEntry) {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search settings")
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .margin_bottom(6)
        .build();
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    list.append(&nav_row("back", "Back to Fermix", "go-previous-symbolic"));
    for (group, slugs) in GROUPS {
        list.append(&heading_row(group));
        for slug in slugs {
            let pane = pane(slug).expect("every grouped slug is a pane");
            list.append(&nav_row(pane.slug, pane.title, pane.icon));
        }
    }
    let shown = visible.clone();
    list.set_filter_func(move |row| row_visible(row, &shown.borrow()));
    wire_sidebar(&list, &search);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();
    let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sidebar.append(&search);
    sidebar.append(&scroller);
    (sidebar, list, search)
}

fn row_visible(row: &gtk::ListBoxRow, visible: &[&'static str]) -> bool {
    let name = row.widget_name();
    if name == "back" {
        return true;
    }
    match name.strip_prefix("group:") {
        Some(group) => GROUPS
            .iter()
            .find(|(g, _)| *g == group)
            .is_some_and(|(_, slugs)| slugs.iter().any(|s| visible.contains(s))),
        None => visible.contains(&name.as_str()),
    }
}

fn wire_sidebar(list: &gtk::ListBox, search: &gtk::SearchEntry) {
    list.connect_row_activated(|_, row| {
        let name = row.widget_name();
        let result = if name == "back" {
            row.activate_action("win.leave-settings", None)
        } else {
            row.activate_action("win.page", Some(&name.as_str().to_variant()))
        };
        if let Err(e) = result {
            glib::g_warning!("fermix", "the settings list could not open {name}: {e}");
        }
    });
    search.connect_search_changed(|entry| {
        let query = entry.text().to_string();
        if let Err(e) = entry.activate_action("win.settings-search", Some(&query.to_variant())) {
            glib::g_warning!("fermix", "settings search failed: {e}");
        }
    });
    let first = list.clone();
    search.connect_activate(move |_| {
        let mut child = first.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            let Ok(row) = widget.downcast::<gtk::ListBoxRow>() else {
                continue;
            };
            let name = row.widget_name();
            if row.is_child_visible() && name != "back" && !name.starts_with("group:") {
                row.activate();
                return;
            }
        }
    });
}

fn nav_row(name: &str, title: &str, icon: &str) -> gtk::ListBoxRow {
    let line = gtk::Box::builder().spacing(12).build();
    line.append(&gtk::Image::from_icon_name(icon));
    line.append(&gtk::Label::new(Some(title)));
    gtk::ListBoxRow::builder().child(&line).name(name).build()
}

fn heading_row(group: &str) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(group)
        .xalign(0.0)
        .margin_top(12)
        .css_classes(["heading", "dim-label"])
        .build();
    gtk::ListBoxRow::builder()
        .child(&label)
        .name(format!("group:{group}"))
        .activatable(false)
        .selectable(false)
        .build()
}
