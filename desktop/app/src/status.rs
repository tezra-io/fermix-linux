//! The page a surface shows while it has nothing from Fermix to draw: connecting,
//! starting, not running, not responding, or too old or too new for this app.
//! Redrawn only when what it says changes.

use crate::home::{action_button, START_COMMAND};
use adw::prelude::*;
use fermix_client::view::DaemonProblem;
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq)]
pub struct DownView {
    icon: &'static str,
    title: &'static str,
    description: String,
    /// Shows the terminal command that tells why Fermix did not start.
    command: bool,
    waiting: bool,
    /// (label, action)
    button: Option<(&'static str, &'static str)>,
}

impl DownView {
    pub fn title(&self) -> &'static str {
        self.title
    }

    /// (label, action)
    pub fn button(&self) -> Option<(&'static str, &'static str)> {
        self.button
    }
}

pub fn waiting(title: &'static str) -> DownView {
    DownView {
        icon: "network-offline-symbolic",
        title,
        description: String::new(),
        command: false,
        waiting: true,
        button: None,
    }
}

/// `needs_it` says what this surface cannot do without Fermix, for the
/// not-running case ("Providers are read from Fermix. Start it to see them.").
pub fn down_view(problem: &DaemonProblem, wake_failed: bool, needs_it: &str) -> DownView {
    let (title, description, button) = match problem {
        DaemonProblem::UpdateNeeded(sentence) => ("Update needed", sentence.clone(), None),
        DaemonProblem::NotRunning => (
            "Fermix is not running",
            needs_it.to_owned(),
            Some(("Start Fermix", "win.start-service")),
        ),
        DaemonProblem::NotResponding | DaemonProblem::Broken(_) => (
            "Fermix is not responding",
            "Its socket is there but nothing answers. Restarting it usually fixes this.".to_owned(),
            Some(("Restart Fermix", "win.restart-service")),
        ),
    };
    let icon = match problem {
        DaemonProblem::UpdateNeeded(_) => "software-update-available-symbolic",
        _ => "network-offline-symbolic",
    };
    let view = DownView {
        icon,
        title,
        description,
        command: false,
        waiting: false,
        button,
    };
    match button {
        Some((_, action)) if wake_failed => DownView {
            title: "Fermix did not come back",
            description: "To see why, run this in a terminal:".to_owned(),
            command: true,
            button: Some(("Try again", action)),
            ..view
        },
        _ => view,
    }
}

pub struct DownPage {
    pub page: adw::StatusPage,
    shown: RefCell<Option<DownView>>,
}

impl DownPage {
    pub fn new() -> Self {
        DownPage {
            page: adw::StatusPage::new(),
            shown: RefCell::default(),
        }
    }

    pub fn show(&self, view: DownView) {
        if self.shown.borrow().as_ref() == Some(&view) {
            return;
        }
        self.page.set_icon_name(Some(view.icon));
        self.page.set_title(view.title);
        self.page.set_description(Some(&view.description));
        self.page.set_child(Some(&down_child(&view)));
        self.shown.replace(Some(view));
    }

    /// Forgets what was shown, so the next `show` redraws.
    pub fn reset(&self) {
        self.shown.replace(None);
    }
}

fn down_child(view: &DownView) -> gtk::Widget {
    let child = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .halign(gtk::Align::Center)
        .build();
    if view.command {
        let command = gtk::Label::builder()
            .label(START_COMMAND)
            .selectable(true)
            .css_classes(["monospace"])
            .build();
        child.append(&command);
    }
    if view.waiting {
        child.append(&adw::Spinner::builder().height_request(32).build());
    }
    if let Some((label, action)) = view.button {
        let button = action_button(label, action, None);
        button.set_halign(gtk::Align::Center);
        button.add_css_class("pill");
        button.add_css_class("suggested-action");
        child.append(&button);
    }
    child.upcast()
}
