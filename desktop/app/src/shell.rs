//! The window: a sidebar of pages over one stack, a toast overlay, and a header
//! with at most two trailing buttons (design_final §1). Settings is pinned at
//! the sidebar's foot; opening it swaps the sidebar for the pane list.

use crate::settings::SettingsPage;
use adw::prelude::*;
use fermix_client::settings::pane;
use gtk::gio;
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;

/// (stack name, sidebar label, icon)
/// The Voice page is "companion" in the stack: "voice" is the Settings pane's slug.
pub const PAGES: [(&str, &str, &str); 5] = [
    ("chat", "Chat", "fermix-chat-symbolic"),
    ("companion", "Voice", "audio-input-microphone-symbolic"),
    ("home", "Home", "user-home-symbolic"),
    ("doctor", "Doctor", "fermix-doctor-symbolic"),
    ("logs", "Logs", "text-x-generic-symbolic"),
];

/// The pane Settings opens on when none was open before.
const FIRST_PANE: &str = "providers";
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
    /// "main" or "settings": which list the sidebar shows.
    sidebars: gtk::Stack,
    sidebar_page: adw::NavigationPage,
    /// The sidebar's header, which shows the wordmark over the pages and the
    /// page's own title ("Settings") over the settings panes.
    sidebar_header: adw::HeaderBar,
    wordmark: gtk::Picture,
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
    let sidebar = sidebar_list();
    let pinned = pinned_list();
    let (restart, continue_setup, new_chat) = header_buttons();
    let content_header = adw::HeaderBar::new();
    content_header.pack_start(&new_chat);
    content_header.pack_end(&primary_menu());
    content_header.pack_end(&restart);
    content_header.pack_end(&continue_setup);
    let content = navigation_page("Home", &content_header, &stack);
    let sidebars = sidebar_stack(&sidebar, &pinned, &settings.sidebar);
    let wordmark = crate::wordmark::picture(WORDMARK_HEIGHT);
    let sidebar_header = adw::HeaderBar::builder().title_widget(&wordmark).build();
    // The title still names the page: for the back button when narrow, and in words.
    let sidebar_page = navigation_page("Fermix", &sidebar_header, &sidebars);

    let split = adw::NavigationSplitView::builder()
        .sidebar(&sidebar_page)
        .content(&content)
        .min_sidebar_width(200.0)
        .max_sidebar_width(240.0)
        .build();
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
        sidebars,
        sidebar_page,
        sidebar_header,
        wordmark,
        pinned,
        settings_panes: settings.panes.clone(),
        settings_list: settings.list.clone(),
        last_page: RefCell::new("home".into()),
        last_pane: RefCell::new(FIRST_PANE.into()),
    }
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
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::SlideLeftRight)
        .build();
    stack.add_named(&main, Some("main"));
    stack.add_named(settings, Some("settings"));
    stack
}

fn pinned_list() -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    let line = gtk::Box::builder().spacing(12).build();
    line.append(&gtk::Image::from_icon_name("emblem-system-symbolic"));
    line.append(&gtk::Label::new(Some("Settings")));
    list.append(&gtk::ListBoxRow::builder().child(&line).build());
    list.connect_row_activated(|_, row| {
        if let Err(e) = row.activate_action("win.open-settings", None) {
            glib::g_warning!("fermix", "Settings could not open: {e}");
        }
    });
    list
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

fn sidebar_list() -> gtk::ListBox {
    let list = gtk::ListBox::builder()
        .css_classes(["navigation-sidebar"])
        .build();
    for (name, title, icon) in PAGES {
        let line = gtk::Box::builder().spacing(12).build();
        line.append(&gtk::Image::from_icon_name(icon));
        line.append(&gtk::Label::new(Some(title)));
        let row = gtk::ListBoxRow::builder().child(&line).name(name).build();
        list.append(&row);
    }
    list.connect_row_activated(|_, row| {
        let name = row.widget_name();
        if let Err(e) = row.activate_action("win.page", Some(&name.as_str().to_variant())) {
            glib::g_warning!("fermix", "the sidebar could not open {name}: {e}");
        }
    });
    list
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
        self.sidebars.set_visible_child_name("main");
        self.sidebar_page.set_title("Fermix");
        self.sidebar_header.set_title_widget(Some(&self.wordmark));
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
        self.sidebars.set_visible_child_name("settings");
        self.sidebar_page.set_title("Settings");
        self.sidebar_header.set_title_widget(None::<&gtk::Widget>);
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
