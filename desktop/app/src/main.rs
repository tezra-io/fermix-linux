//! Fermix for Linux: set up Fermix and chat with it. A client of the daemon
//! that the `fermix` package runs; it holds no config and no secrets of its own.

mod acp_link;
mod app;
mod assistant;
mod assistant_flow;
mod audio;
mod background_flow;
mod capability_flow;
mod capability_panes;
mod channels;
mod chat;
mod companion;
mod conversation;
mod daemon;
mod descriptor;
mod dialogs;
mod doctor;
mod flows;
mod home;
mod integrations;
mod logs;
mod marks;
mod mascot;
mod portal;
mod providers;
mod service;
mod settings;
mod settings_flow;
mod shell;
mod state;
mod status;
mod systemd;
mod voice;
mod voice_call;
mod voice_flow;

use adw::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "io.tezra.Fermix";

fn main() -> glib::ExitCode {
    gio::resources_register_include!("fermix.gresource")
        .expect("the app's own resource bundle is built in");
    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(app::activate);
    install_app_actions(&application);
    application.run()
}

fn install_app_actions(application: &adw::Application) {
    let about = gio::SimpleAction::new("about", None);
    let app = application.clone();
    about.connect_activate(move |_, _| {
        let dialog = adw::AboutDialog::builder()
            .application_name("Fermix")
            .application_icon(APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("Tezra")
            .license_type(gtk::License::MitX11)
            .build();
        dialog.present(app.active_window().as_ref());
    });
    let shortcuts = gio::SimpleAction::new("shortcuts", None);
    let app = application.clone();
    shortcuts.connect_activate(move |_, _| {
        shortcuts_dialog().present(app.active_window().as_ref());
    });
    let quit = gio::SimpleAction::new("quit", None);
    let app = application.clone();
    quit.connect_activate(move |_, _| app.quit());
    application.add_action(&about);
    application.add_action(&shortcuts);
    application.add_action(&quit);
    application.set_accels_for_action("app.shortcuts", &["<Ctrl>question"]);
    application.set_accels_for_action("app.quit", &["<Ctrl>q"]);
    application.set_accels_for_action("window.close", &["<Ctrl>w"]);
}

/// Lists the accelerators set above and in `app.rs`; keep the three in step.
fn shortcuts_dialog() -> adw::ShortcutsDialog {
    let pages = adw::ShortcutsSection::new(Some("Pages"));
    for (index, (_, title, _)) in shell::PAGES.iter().enumerate() {
        pages.add(adw::ShortcutsItem::new(
            title,
            &format!("<Ctrl>{}", index + 1),
        ));
    }
    pages.add(adw::ShortcutsItem::new("Settings", "<Ctrl>comma"));
    pages.add(adw::ShortcutsItem::new("Search settings", "<Ctrl>f"));
    let chat = adw::ShortcutsSection::new(Some("Chat"));
    chat.add(adw::ShortcutsItem::new("Send", "Return"));
    chat.add(adw::ShortcutsItem::new("New line", "<Shift>Return"));
    chat.add(adw::ShortcutsItem::new("New conversation", "<Ctrl>n"));
    let general = adw::ShortcutsSection::new(Some("General"));
    general.add(adw::ShortcutsItem::new(
        "Keyboard shortcuts",
        "<Ctrl>question",
    ));
    general.add(adw::ShortcutsItem::new("Close the window", "<Ctrl>w"));
    general.add(adw::ShortcutsItem::new("Quit", "<Ctrl>q"));
    let dialog = adw::ShortcutsDialog::new();
    dialog.add(pages);
    dialog.add(chat);
    dialog.add(general);
    dialog
}
