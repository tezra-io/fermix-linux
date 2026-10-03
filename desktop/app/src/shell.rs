//! The window: a sidebar of pages over one stack, a toast overlay, and a header
//! with at most two trailing buttons (design_final §1). Settings is pinned at
//! the sidebar's foot; opening it swaps the sidebar for the pane list.
//!
//! In a wide window the pages' sidebar is a rail of icons, as on macOS, each
//! named in its tooltip. Narrow, the sidebar is a page of its own and names
//! its rows under the wordmark. The settings pane list is always named.

use crate::settings::SettingsPage;
use adw::prelude::*;
use fermix_client::settings::pane;
use gtk::gio;
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;

/// (stack name, sidebar label, icon)
/// The Pet page is "companion" in the stack: "voice" is the Settings pane's slug.
pub const PAGES: [(&str, &str, &str); 5] = [
    ("chat", "Chat", "fermix-chat-symbolic"),
    ("companion", "Pet", "audio-input-microphone-symbolic"),
    ("home", "Home", "user-home-symbolic"),
    ("doctor", "Doctor", "fermix-doctor-symbolic"),
    ("logs", "Logs", "text-x-generic-symbolic"),
];

/// The pane Settings opens on when none was open before.
const FIRST_PANE: &str = "providers";
/// The rail's width: one column of the sidebar's own rows, an icon each.
const RAIL_WIDTH: f64 = 56.0;
/// The settings pane list's narrowest and widest: thirteen panes need names.
const PANE_LIST_WIDTH: (f64, f64) = (200.0, 240.0);
/// The wordmark's height at the head of the sidebar, its file's margin included.
/// Its letters stand 22 pixels tall, which is what the two eye-dots need to
/// still read as dots at 1x.
const WORDMARK_HEIGHT: i32 = 26;

pub struct Shell {
    pub window: adw::ApplicationWindow,
    pub toasts: adw::ToastOverlay,
    pub stack: adw::ViewStack,
    pub content: adw::NavigationPage,
    pub split: adw::NavigationSplitView,
    pub sidebar: gtk::ListBox,
    pub restart: gtk::Button,
    /// "Continue setup", shown while Fermix says setup is required.
    pub continue_setup: gtk::Button,
    /// "New conversation", shown only on Chat.
    pub new_chat: gtk::Button,
    sidebar_page: adw::NavigationPage,
    shape: SidebarShape,
    pinned: gtk::ListBox,
    settings_panes: gtk::Stack,
    settings_list: gtk::ListBox,
    /// The page Back to Fermix returns to, and the pane Settings reopens on.
    last_page: RefCell<String>,
    last_pane: RefCell<String>,
}

pub fn build(app: &adw::Application, pages: &[&gtk::Widget; 5], settings: &SettingsPage) -> Shell {
    let stack = adw::ViewStack::new();
    // Each page is an AdwPreferencesPage or AdwStatusPage, which scroll and clamp themselves.
    for ((name, title, _), page) in PAGES.iter().zip(pages) {
        stack.add_titled(*page, Some(name), title);
    }
    stack.add_titled(&settings.root, Some("settings"), "Settings");
    let (sidebar, mut rail) = sidebar_list();
    let (pinned, settings_row) = pinned_list();
    rail.push(settings_row);
    let (restart, continue_setup, new_chat) = header_buttons();
    let content_header = adw::HeaderBar::new();
    content_header.pack_start(&new_chat);
    content_header.pack_end(&primary_menu());
    content_header.pack_end(&restart);
    content_header.pack_end(&continue_setup);
    let content = navigation_page("Home", &content_header, &stack);
    let sidebars = sidebar_stack(&sidebar, &pinned, &settings.sidebar);
    let (sidebar_page, header, wordmark) = sidebar_navigation(&sidebars);
    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content)
        .build();
    let shape = SidebarShape {
        split: split.clone(),
        sidebars,
        header,
        wordmark,
        rail,
    };
    shape.follow_window();
    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&split));
    let window = main_window(app, &toasts, &split);
    Shell {
        window,
        toasts,
        stack,
        content,
        split,
        sidebar,
        restart,
        continue_setup,
        new_chat,
        sidebar_page,
        shape,
        pinned,
        settings_panes: settings.panes.clone(),
        settings_list: settings.list.clone(),
        last_page: RefCell::new("home".into()),
        last_pane: RefCell::new(FIRST_PANE.into()),
    }
}

/// The sidebar's page and its head. The wordmark starts on the left, where the
/// rows start, as a name above a list. The title still names the page: for the
/// back button when narrow, and in words.
fn sidebar_navigation(
    sidebars: &gtk::Stack,
) -> (adw::NavigationPage, adw::HeaderBar, gtk::Picture) {
    let wordmark = crate::wordmark::picture(WORDMARK_HEIGHT);
    wordmark.add_css_class("sidebar-wordmark");
    let header = adw::HeaderBar::builder().show_title(false).build();
    header.pack_start(&wordmark);
    let page = navigation_page("Fermix", &header, sidebars);
    (page, header, wordmark)
}

/// The header's trailing actions and New conversation, all hidden until wanted.
fn header_buttons() -> (gtk::Button, gtk::Button, gtk::Button) {
    let restart = gtk::Button::builder()
        .label("Restart Fermix…")
        .css_classes(["suggested-action"])
        .visible(false)
        .build();
    restart.set_action_name(Some("win.restart"));
    let continue_setup = gtk::Button::builder()
        .label("Continue Setup…")
        .css_classes(["suggested-action"])
        .visible(false)
        .build();
    continue_setup.set_action_name(Some("win.open-assistant"));
    let new_chat = gtk::Button::builder()
        .icon_name("chat-message-new-symbolic")
        .tooltip_text("New conversation")
        .visible(false)
        .build();
    new_chat.set_action_name(Some("win.new-chat"));
    (restart, continue_setup, new_chat)
}

/// The window, which folds the sidebar away when narrow.
fn main_window(
    app: &adw::Application,
    toasts: &adw::ToastOverlay,
    split: &adw::NavigationSplitView,
) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Fermix")
        .default_width(880)
        .default_height(560)
        .width_request(360)
        .height_request(440)
        .content(toasts)
        .build();
    let narrow = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 640sp").expect("a valid breakpoint"),
    );
    narrow.add_setter(split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(narrow);
    window
}

/// The page list over the pinned Settings row, beside the settings pane list.
fn sidebar_stack(pages: &gtk::ListBox, pinned: &gtk::ListBox, settings: &gtk::Box) -> gtk::Stack {
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(pages)
        .build();
    let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
    main.append(&scroller);
    main.append(pinned);
    // Sized by the list on show, so the rail is not as wide as the pane list.
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .hhomogeneous(false)
        .build();
    stack.add_named(&main, Some("main"));
    stack.add_named(settings, Some("settings"));
    stack
}

fn pinned_list() -> (gtk::ListBox, RailRow) {
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let settings = rail_row("Settings", "emblem-system-symbolic");
    // A way into Settings, not a page here: keyboard focus on it never
    // selects it, so Tab through the sidebar never opens Settings.
    settings.row.set_selectable(false);
    list.append(&settings.row);
    list.connect_row_activated(|_, row| {
        if let Err(e) = row.activate_action("win.open-settings", None) {
            glib::g_warning!("fermix", "Settings could not open: {e}");
        }
    });
    (list, settings)
}

/// One row of the pages' sidebar: its icon, and its name beside it, which only
/// a narrow window shows. Screen readers always have the name.
#[derive(Clone)]
struct RailRow {
    row: gtk::ListBoxRow,
    line: gtk::Box,
    name: gtk::Label,
}

fn rail_row(title: &str, icon: &str) -> RailRow {
    let name = gtk::Label::new(Some(title));
    let line = gtk::Box::builder().spacing(12).build();
    line.append(&gtk::Image::from_icon_name(icon));
    line.append(&name);
    let row = gtk::ListBoxRow::builder().child(&line).build();
    row.update_property(&[gtk::accessible::Property::Label(title)]);
    RailRow { row, line, name }
}

/// What the sidebar's look depends on: which list it shows, and whether the
/// window is narrow enough that the sidebar is a page of its own.
#[derive(Clone)]
struct SidebarShape {
    split: adw::NavigationSplitView,
    /// "main" or "settings": which list the sidebar shows.
    sidebars: gtk::Stack,
    /// Shows the page's own title ("Settings") over the settings panes, the
    /// wordmark over the pages when narrow, and nothing over the rail.
    header: adw::HeaderBar,
    wordmark: gtk::Picture,
    rail: Vec<RailRow>,
}

impl SidebarShape {
    /// Fits the sidebar now, and again whenever the window folds or unfolds it.
    fn follow_window(&self) {
        self.fit();
        let refit = self.clone();
        self.split.connect_collapsed_notify(move |_| refit.fit());
    }

    /// Sizes and dresses the sidebar for the list it shows and the window's width.
    fn fit(&self) {
        let on_pages = self.sidebars.visible_child_name().as_deref() == Some("main");
        let narrow = self.split.is_collapsed();
        let (min, max) = if on_pages {
            (RAIL_WIDTH, RAIL_WIDTH)
        } else {
            PANE_LIST_WIDTH
        };
        self.split.set_min_sidebar_width(min);
        self.split.set_max_sidebar_width(max);
        self.header.set_show_title(!on_pages);
        self.wordmark.set_visible(on_pages && narrow);
        let (align, tooltip) = if narrow {
            (gtk::Align::Fill, false)
        } else {
            (gtk::Align::Center, true)
        };
        for rail in &self.rail {
            rail.name.set_visible(narrow);
            rail.line.set_halign(align);
            let name = tooltip.then(|| rail.name.text());
            rail.row.set_tooltip_text(name.as_deref());
        }
    }
}

fn navigation_page(
    title: &str,
    header: &adw::HeaderBar,
    child: &impl IsA<gtk::Widget>,
) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(header);
    toolbar.set_content(Some(child));
    adw::NavigationPage::builder()
        .title(title)
        .child(&toolbar)
        .build()
}

fn sidebar_list() -> (gtk::ListBox, Vec<RailRow>) {
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let mut rail = Vec::with_capacity(PAGES.len());
    for (name, title, icon) in PAGES {
        let page = rail_row(title, icon);
        page.row.set_widget_name(name);
        list.append(&page.row);
        rail.push(page);
    }
    list.connect_row_selected(|list, row| {
        if let Some(row) = row.filter(|_| follows_selection(list)) {
            open_page(row);
        }
    });
    // A click on the row already selected: narrow, it shows the page again.
    list.connect_row_activated(|_, row| open_page(row));
    (list, rail)
}

fn open_page(row: &gtk::ListBoxRow) {
    let name = row.widget_name();
    if let Err(e) = row.activate_action("win.page", Some(&name.as_str().to_variant())) {
        glib::g_warning!("fermix", "the sidebar could not open {name}: {e}");
    }
}

/// Keyboard focus moves a sidebar's selection (GTK does), so in a wide window
/// the page follows the selection and the highlighted row is always the page
/// on show. Narrow, the sidebar is a page of its own: only activation leaves it.
pub fn follows_selection(list: &gtk::ListBox) -> bool {
    list.ancestor(adw::NavigationSplitView::static_type())
        .and_downcast::<adw::NavigationSplitView>()
        .is_some_and(|split| !split.is_collapsed())
}

fn primary_menu() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(Some("New Conversation"), Some("win.new-chat"));
    menu.append(Some("Settings"), Some("win.open-settings"));
    menu.append(Some("Keyboard Shortcuts"), Some("app.shortcuts"));
    menu.append(Some("About Fermix"), Some("app.about"));
    menu.append(Some("Quit"), Some("app.quit"));
    gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .tooltip_text("Main menu")
        .primary(true)
        .build()
}

impl Shell {
    /// Shows one page or settings pane, and keeps the sidebar and title in step.
    pub fn show_page(&self, name: &str) {
        if pane(name).is_some() {
            return self.show_pane(name);
        }
        let Some((_, title, _)) = PAGES.iter().find(|(n, _, _)| *n == name) else {
            glib::g_warning!("fermix", "no page named {name}");
            return;
        };
        self.stack.set_visible_child_name(name);
        self.shape.sidebars.set_visible_child_name("main");
        self.sidebar_page.set_title("Fermix");
        self.shape.fit();
        self.pinned.unselect_all();
        *self.last_page.borrow_mut() = name.to_owned();
        self.content.set_title(title);
        self.new_chat.set_visible(name == "chat");
        let index = PAGES
            .iter()
            .position(|(n, _, _)| *n == name)
            .expect("found above");
        let row = self
            .sidebar
            .row_at_index(i32::try_from(index).expect("few pages"));
        self.sidebar.select_row(row.as_ref());
        self.split.set_show_content(true);
    }

    fn show_pane(&self, slug: &str) {
        let title = pane(slug).expect("checked by the caller").title;
        self.stack.set_visible_child_name("settings");
        self.settings_panes.set_visible_child_name(slug);
        self.shape.sidebars.set_visible_child_name("settings");
        self.sidebar_page.set_title("Settings");
        self.shape.fit();
        self.content.set_title(title);
        self.new_chat.set_visible(false);
        *self.last_pane.borrow_mut() = slug.to_owned();
        select_named(&self.settings_list, slug);
        self.split.set_show_content(true);
    }

    /// The settings pane last shown, which Settings reopens on.
    pub fn last_pane(&self) -> String {
        self.last_pane.borrow().clone()
    }

    pub fn leave_settings(&self) {
        let page = self.last_page.borrow().clone();
        self.show_page(&page);
    }

    pub fn visible_page(&self) -> Option<String> {
        self.stack.visible_child_name().map(|name| name.to_string())
    }

    /// The settings pane on screen, if Settings is.
    pub fn visible_pane(&self) -> Option<String> {
        if self.visible_page().as_deref() != Some("settings") {
            return None;
        }
        self.settings_panes
            .visible_child_name()
            .map(|name| name.to_string())
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }
}

fn select_named(list: &gtk::ListBox, name: &str) {
    let mut child = list.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        let Ok(row) = widget.downcast::<gtk::ListBoxRow>() else {
            continue;
        };
        if row.widget_name() == name {
            list.select_row(Some(&row));
            return;
        }
    }
}
