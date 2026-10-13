//! The page a surface shows while it has nothing from Fermix to draw: connecting,
//! starting, not running, not responding, or too old or too new for this app.
//! Redrawn only when what it says changes.

use crate::home::action_button;
use adw::prelude::*;
use fermix_client::service::start_command;
use fermix_client::view::DaemonProblem;
use std::cell::RefCell;

#[derive(Debug, Clone, PartialEq)]
pub struct DownView {
    icon: &'static str,
    title: &'static str,
    description: String,
    /// The terminal command that tells why Fermix did not start.
    command: Option<&'static str>,
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
        command: None,
        waiting: true,
        button: None,
    }
}

/// `needs_it` says what this surface cannot do without Fermix, for the
/// not-running case ("Providers are read from Fermix. Start it to see them.").
/// `bound` picks the command a failed start names, as Home does.
pub fn down_view(
    problem: &DaemonProblem,
    wake_failed: bool,
    bound: bool,
    needs_it: &str,
) -> DownView {
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
        command: None,
        waiting: false,
        button,
    };
    match button {
        Some((_, action)) if wake_failed => DownView {
            title: "Fermix did not come back",
            description: "To see why, run this in a terminal:".to_owned(),
            command: Some(start_command(bound)),
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
    if let Some(command) = view.command {
        let command = gtk::Label::builder()
            .label(command)
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

#[cfg(test)]
mod tests {
    use super::down_view;
    use fermix_client::service::{INSTALL_COMMAND, START_COMMAND};
    use fermix_client::view::DaemonProblem;

    #[test]
    fn a_failed_start_names_the_command_the_account_starts_with() {
        let unbound = down_view(&DaemonProblem::NotRunning, true, false, "");
        assert_eq!(unbound.command, Some(INSTALL_COMMAND));
        let bound = down_view(&DaemonProblem::NotRunning, true, true, "");
        assert_eq!(bound.command, Some(START_COMMAND));
        assert_eq!(bound.title(), "Fermix did not come back");
    }

    #[test]
    fn no_command_is_named_until_a_start_has_failed() {
        for bound in [true, false] {
            let view = down_view(&DaemonProblem::NotRunning, false, bound, "");
            assert_eq!(view.command, None);
        }
        let update = down_view(&DaemonProblem::UpdateNeeded("x".into()), true, false, "");
        assert_eq!(update.command, None);
    }
}
