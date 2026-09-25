//! The setup assistant (M38 §5.5): Welcome, Connect your AI, About you,
//! Applying and Ready, as pages in one dialog over the window. It opens while
//! Fermix says setup is required; closing it marks nothing done, and Home keeps
//! offering to continue. Its rules live in `fermix_client::onboarding`.

use crate::providers::ProvidersPage;
use adw::prelude::*;
use fermix_client::onboarding::{DEFAULT_STYLE, STYLES};
use gtk::glib;

pub struct AboutFields {
    pub name: adw::EntryRow,
    pub zone: adw::EntryRow,
    pub style: adw::ComboRow,
    pub assistant: adw::EntryRow,
    /// The daemon's reason when the save was refused.
    pub error: gtk::Label,
}

pub struct ApplyingRows {
    pub save: Step,
    pub restart: Step,
    pub error: gtk::Label,
    pub retry: gtk::Button,
}

/// One checklist row and the marker beside it.
pub struct Step {
    pub row: adw::ActionRow,
    marker: gtk::Stack,
}

#[derive(Debug, Clone, Copy)]
pub enum Mark {
    Working,
    Done,
    Failed,
}

impl Step {
    fn new(title: &str) -> Step {
        let marker = gtk::Stack::new();
        marker.add_named(&adw::Spinner::new(), Some("working"));
        marker.add_named(
            &marker_icon("object-select-symbolic", "success"),
            Some("done"),
        );
        marker.add_named(
            &marker_icon("dialog-error-symbolic", "error"),
            Some("failed"),
        );
        let row = adw::ActionRow::builder().title(title).build();
        row.add_suffix(&marker);
        Step { row, marker }
    }

    pub fn mark(&self, mark: Mark) {
        let name = match mark {
            Mark::Working => "working",
            Mark::Done => "done",
            Mark::Failed => "failed",
        };
        self.marker.set_visible_child_name(name);
    }
}

fn marker_icon(name: &str, class: &str) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(name);
    icon.add_css_class(class);
    icon
}

pub struct Assistant {
    pub dialog: adw::Dialog,
    pub nav: adw::NavigationView,
    /// Connect your AI is the Providers list itself, drawn a second time.
    pub providers: ProvidersPage,
    pub connect_next: gtk::Button,
    pub about: AboutFields,
    pub applying: ApplyingRows,
    pub ready: adw::StatusPage,
}

impl Assistant {
    pub fn new() -> Assistant {
        let providers = ProvidersPage::new();
        let connect_next = pill("Continue", "win.assistant-next");
        let (about, about_page) = about_page();
        let (applying, applying_page) = applying_page();
        let (ready, ready_page) = ready_page();
        let nav = adw::NavigationView::new();
        nav.add(&welcome_page());
        nav.add(&page(
            "connect",
            "Connect your AI",
            &providers.root,
            Some(&connect_next),
        ));
        nav.add(&about_page);
        nav.add(&applying_page);
        nav.add(&ready_page);
        let dialog = adw::Dialog::builder()
            .title("Set up Fermix")
            .content_width(640)
            .content_height(620)
            .child(&nav)
            .build();
        Assistant {
            dialog,
            nav,
            providers,
            connect_next,
            about,
            applying,
            ready,
        }
    }

    /// Shows one stage, keeping Back to the stages before it.
    pub fn show(&self, tag: &str) {
        if self.nav.visible_page().and_then(|p| p.tag()).as_deref() == Some(tag) {
            return;
        }
        self.nav.push_by_tag(tag);
    }
}

fn pill(label: &str, action: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    button.set_action_name(Some(action));
    button
}

/// A stage: its header, its content, and the one button that moves on.
fn page(
    tag: &str,
    title: &str,
    content: &impl IsA<gtk::Widget>,
    next: Option<&gtk::Button>,
) -> adw::NavigationPage {
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(content));
    if let Some(next) = next {
        let bar = gtk::Box::builder()
            .halign(gtk::Align::Center)
            .margin_top(12)
            .margin_bottom(18)
            .build();
        bar.append(next);
        toolbar.add_bottom_bar(&bar);
    }
    adw::NavigationPage::builder()
        .tag(tag)
        .title(title)
        .child(&toolbar)
        .build()
}

fn welcome_page() -> adw::NavigationPage {
    let status = adw::StatusPage::builder()
        .icon_name(crate::APP_ID)
        .title("Welcome to Fermix")
        .description(
            "Your assistant, running on this computer. Connect an AI and tell it who you \
             are, and it is ready to answer.",
        )
        .child(&pill("Begin", "win.assistant-begin"))
        .build();
    page("welcome", "Welcome", &status, None)
}

fn about_page() -> (AboutFields, adw::NavigationPage) {
    let name = adw::EntryRow::builder()
        .title("Your name")
        .text(account_name())
        .build();
    let zone = adw::EntryRow::builder()
        .title("Time zone")
        .text(local_zone())
        .build();
    let labels: Vec<&str> = STYLES.iter().map(|(label, _)| *label).collect();
    let style = adw::ComboRow::builder()
        .title("Style")
        .model(&gtk::StringList::new(&labels))
        .selected(u32::try_from(DEFAULT_STYLE).expect("small"))
        .subtitle(STYLES[DEFAULT_STYLE].1)
        .build();
    style.connect_selected_notify(|row| {
        let chosen = usize::try_from(row.selected()).unwrap_or(DEFAULT_STYLE);
        row.set_subtitle(STYLES.get(chosen).map_or("", |(_, sentence)| sentence));
    });
    let assistant = adw::EntryRow::builder()
        .title("Call the assistant")
        .text("Fermix")
        .build();
    let group = adw::PreferencesGroup::builder()
        .description("Fermix uses these to address you and keep time straight. Change any of them later in Settings.")
        .build();
    for row in [
        name.upcast_ref::<gtk::Widget>(),
        zone.upcast_ref(),
        style.upcast_ref(),
        assistant.upcast_ref(),
    ] {
        group.add(row);
    }
    let error = error_label();
    let content = adw::PreferencesPage::new();
    content.add(&group);
    let errors = adw::PreferencesGroup::new();
    errors.add(&error);
    content.add(&errors);
    let fields = AboutFields {
        name,
        zone,
        style,
        assistant,
        error,
    };
    let page = page(
        "about",
        "About you",
        &content,
        Some(&pill("Continue", "win.assistant-apply")),
    );
    (fields, page)
}

fn applying_page() -> (ApplyingRows, adw::NavigationPage) {
    let save = Step::new("Saving your details");
    let restart = Step::new("Restarting Fermix to apply them");
    let group = adw::PreferencesGroup::new();
    group.add(&save.row);
    group.add(&restart.row);
    let error = error_label();
    let retry = gtk::Button::builder()
        .label("Try again")
        .halign(gtk::Align::Start)
        .visible(false)
        .build();
    // Retry re-reads where setup stands; it never writes About you again unasked.
    retry.set_action_name(Some("win.assistant-next"));
    let content = adw::PreferencesPage::new();
    content.add(&group);
    let errors = adw::PreferencesGroup::new();
    errors.add(&error);
    errors.add(&retry);
    content.add(&errors);
    let rows = ApplyingRows {
        save,
        restart,
        error,
        retry,
    };
    (rows, page("applying", "Applying", &content, None))
}

fn ready_page() -> (adw::StatusPage, adw::NavigationPage) {
    let next = adw::PreferencesGroup::builder()
        .title("Next, if you like")
        .build();
    for (title, pane) in [
        ("Reach Fermix from a chat app", "channels"),
        ("Set up voice", "voice"),
    ] {
        let row = adw::ActionRow::builder()
            .title(title)
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_action_name(Some("win.assistant-finish"));
        row.set_action_target_value(Some(&glib::Variant::from(pane)));
        next.add(&row);
    }
    let finish = gtk::Button::builder()
        .label("Start chatting")
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build();
    finish.set_action_name(Some("win.assistant-finish"));
    finish.set_action_target_value(Some(&glib::Variant::from("chat")));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 24);
    body.append(&finish);
    body.append(&next);
    let status = adw::StatusPage::builder()
        .icon_name(crate::APP_ID)
        .title("Fermix is live")
        .child(&adw::Clamp::builder().maximum_size(420).child(&body).build())
        .build();
    let page = page("ready", "Ready", &status, None);
    page.set_can_pop(false);
    (status, page)
}

fn error_label() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["error"])
        .visible(false)
        .build()
}

/// The account's full name, else its login name: what About you starts from.
fn account_name() -> String {
    let real = glib::real_name().to_string_lossy().trim().to_owned();
    if real.is_empty() || real == "Unknown" {
        return glib::user_name().to_string_lossy().into_owned();
    }
    real
}

/// This computer's time zone as an IANA id, else UTC.
fn local_zone() -> String {
    let id = glib::TimeZone::local().identifier().to_string();
    if id.contains('/') || id == "UTC" {
        return id;
    }
    "UTC".to_owned()
}
