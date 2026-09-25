//! Meetings and Computer: panes headed by a switch whose first turn-on installs
//! a helper, then the daemon's rows, then what the daemon reports about the
//! helper (M38 §5.7, §8.6). The words come from the daemon's rows and answers;
//! nothing here decides a capability.

use crate::descriptor::{Drawn, SectionView};
use crate::marks::{mark, Kind};
use adw::prelude::*;
use fermix_client::capabilities::{meetbot_state, phase_words, verdict, MeetbotState};
use fermix_client::capabilities::{ComputerPermissions, COMPUTER_SIDECAR, MEETBOT};
use fermix_client::ledger::MEETINGS_SLEEP_STATEMENT;
use fermix_client::model::DetectRow;
use fermix_client::settings::SectionRows;
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;
use std::rc::Rc;

pub const MEETINGS_ENABLED: &str = "meetings_enabled";
pub const COMPUTER_ENABLED: &str = "computer_use_enabled";
/// The job that signs the notetaker in; it has no target of its own.
pub const MEETINGS_SIGN_IN: &str = "meetings_signin";

const ARM64: &str = "Computer use is not available on this architecture. Fermix runs fully on \
    arm64 Linux, but the computer-use sidecar has no arm64 build, so there is nothing to install.";
const UNANSWERED: &str = "Fermix could not read the sign-in state on this computer.";

/// A job the pane is following: its name, id and current phase.
#[derive(Debug, Clone, PartialEq)]
pub struct Running {
    pub job_id: String,
    pub phase: Option<String>,
}

/// The switch row at the head of a pane, as drawn.
#[derive(Debug, Clone, PartialEq)]
struct Header {
    title: String,
    subtitle: String,
    on: bool,
    running: Option<Running>,
    failure: Option<String>,
    locked: bool,
}

/// One group whose rows are rebuilt whenever what they show changes.
struct Redrawn<T: PartialEq> {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
    shown: RefCell<Option<T>>,
}

impl<T: PartialEq> Redrawn<T> {
    fn new(group: adw::PreferencesGroup) -> Redrawn<T> {
        Redrawn {
            group,
            rows: RefCell::default(),
            shown: RefCell::default(),
        }
    }

    fn show(&self, value: T, build: impl FnOnce(&T) -> Vec<gtk::Widget>) {
        if self.shown.borrow().as_ref() == Some(&value) {
            return;
        }
        for old in self.rows.borrow_mut().drain(..) {
            self.group.remove(&old);
        }
        let fresh = build(&value);
        for row in &fresh {
            self.group.add(row);
        }
        *self.rows.borrow_mut() = fresh;
        *self.shown.borrow_mut() = Some(value);
    }
}

/// What a capability pane is drawn from, beyond the section rows.
pub struct Facts<'a> {
    pub rows: Option<&'a SectionRows>,
    pub running: Option<&'a Running>,
    pub failure: Option<&'a String>,
    pub locked: bool,
}

fn header(facts: &Facts<'_>, key: &str, fallback: &str) -> Header {
    let row = facts
        .rows
        .and_then(|s| s.rows.iter().find(|r| r.key == key));
    Header {
        title: row.map_or_else(|| fallback.to_owned(), |r| r.label.clone()),
        subtitle: row.and_then(|r| r.footer.clone()).unwrap_or_default(),
        on: row.and_then(|r| r.value.as_bool()).unwrap_or(false),
        running: facts.running.cloned(),
        failure: facts.failure.cloned(),
        locked: facts.locked,
    }
}

/// The switch row: installing shows its phase and a Cancel; a failure its sentence.
fn header_row(h: &Header, target: &str) -> Vec<gtk::Widget> {
    let subtitle = match (&h.running, &h.failure) {
        (Some(job), _) => phase_words(job.phase.as_deref()).to_owned(),
        (None, Some(sentence)) => sentence.clone(),
        (None, None) => h.subtitle.clone(),
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&h.title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .build();
    if h.failure.is_some() && h.running.is_none() {
        row.add_css_class("setting-refused");
    }
    if let Some(job) = &h.running {
        row.add_suffix(&adw::Spinner::new());
        row.add_suffix(&cancel_button(&job.job_id));
    }
    let switch = gtk::Switch::builder()
        .active(h.on || h.running.is_some())
        .sensitive(!h.locked && h.running.is_none())
        .valign(gtk::Align::Center)
        .build();
    switch.update_property(&[gtk::accessible::Property::Label(&h.title)]);
    let target = target.to_owned();
    switch.connect_active_notify(move |switch| {
        let wanted = (target.as_str(), switch.is_active()).to_variant();
        if let Err(e) = switch.activate_action("win.capability-switch", Some(&wanted)) {
            glib::g_warning!("fermix", "the capability switch could not be sent: {e}");
        }
    });
    row.add_suffix(&switch);
    row.set_activatable_widget(Some(&switch));
    vec![row.upcast()]
}

fn cancel_button(job_id: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label("Cancel")
        .valign(gtk::Align::Center)
        .build();
    button.set_action_name(Some("win.cancel-job"));
    button.set_action_target_value(Some(&job_id.to_variant()));
    button
}

fn intro_group(text: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_description(Some(&glib::markup_escape_text(text)));
    group
}

pub struct MeetingsPane {
    pub page: adw::PreferencesPage,
    pub shared: Rc<SectionView>,
    pub zoom: Rc<SectionView>,
    header: Redrawn<Header>,
    google: Redrawn<(MeetbotState, Option<Running>)>,
}

impl MeetingsPane {
    pub fn new() -> MeetingsPane {
        let page = adw::PreferencesPage::new();
        page.add(&intro_group(MEETINGS_SLEEP_STATEMENT));
        let header = Redrawn::new(adw::PreferencesGroup::new());
        page.add(&header.group);
        let shared = SectionView::keeping(
            "meetings",
            Some("Shared settings"),
            Box::new(|k| k != MEETINGS_ENABLED && !k.starts_with("meetings_zoom_")),
        );
        page.add(&shared.group);
        let google = Redrawn::new(
            adw::PreferencesGroup::builder()
                .title("Google Meet")
                .build(),
        );
        page.add(&google.group);
        let zoom = SectionView::keeping(
            "meetings",
            Some("Zoom"),
            Box::new(|k| k.starts_with("meetings_zoom_")),
        );
        // Its rows are the daemon's settings, so the mark heads the group instead.
        zoom.group
            .set_header_suffix(Some(&mark(Kind::MeetingPlatform, "zoom")));
        page.add(&zoom.group);
        MeetingsPane {
            page,
            shared,
            zoom,
            header,
            google,
        }
    }

    pub fn render(
        &self,
        facts: &Facts<'_>,
        drawn: &Drawn,
        meetbot: Option<&DetectRow>,
        sign_in: Option<&Running>,
    ) {
        self.header
            .show(header(facts, MEETINGS_ENABLED, "Meeting notetaker"), |h| {
                header_row(h, MEETBOT)
            });
        self.shared.show(drawn.clone());
        self.zoom.show(drawn.clone());
        let state = (meetbot_state(meetbot), sign_in.cloned());
        self.google.show(state, |(state, job)| {
            vec![google_row(state, job.as_ref()).upcast()]
        });
    }
}

/// The notetaker's Google account: signed in, signed out, absent or unanswered.
fn google_row(state: &MeetbotState, job: Option<&Running>) -> adw::ActionRow {
    let (subtitle, verb) = match state {
        MeetbotState::SignedIn(detail) => (detail.clone(), Some("Sign in again")),
        MeetbotState::SignedOut(detail) => (
            detail.clone().unwrap_or_else(|| "Not signed in".into()),
            Some("Sign in"),
        ),
        MeetbotState::Absent => ("Turn the notetaker on to install it first.".into(), None),
        MeetbotState::Unanswered => (UNANSWERED.into(), Some("Check again")),
    };
    let row = adw::ActionRow::builder()
        .title("Google account")
        .subtitle(glib::markup_escape_text(&subtitle))
        .build();
    row.add_prefix(&mark(Kind::MeetingPlatform, "google_meet"));
    if let Some(job) = job {
        row.set_subtitle(phase_words(job.phase.as_deref()));
        row.add_suffix(&adw::Spinner::new());
        row.add_suffix(&cancel_button(&job.job_id));
        return row;
    }
    let Some(verb) = verb else { return row };
    let action = if verb == "Check again" {
        "win.detect-meetbot"
    } else {
        "win.meetings-sign-in"
    };
    let button = gtk::Button::builder()
        .label(verb)
        .valign(gtk::Align::Center)
        .build();
    if matches!(state, MeetbotState::SignedOut(_)) {
        button.add_css_class("suggested-action");
    }
    button.set_action_name(Some(action));
    row.add_suffix(&button);
    row
}

pub struct ComputerPane {
    pub page: adw::PreferencesPage,
    pub rest: Rc<SectionView>,
    header: Redrawn<Header>,
    probe: Redrawn<Option<Result<ComputerPermissions, String>>>,
    arm64: adw::PreferencesGroup,
}

impl ComputerPane {
    pub fn new() -> ComputerPane {
        let page = adw::PreferencesPage::new();
        let arm64 = intro_group(ARM64);
        arm64.set_visible(false);
        page.add(&arm64);
        let header = Redrawn::new(adw::PreferencesGroup::new());
        page.add(&header.group);
        let rest = SectionView::keeping("computer_use", None, Box::new(|k| k != COMPUTER_ENABLED));
        page.add(&rest.group);
        let probe_group = adw::PreferencesGroup::builder()
            .title("What the helper can do")
            .description("As the helper last checked. Fermix asks for nothing on this page.")
            .build();
        probe_group.set_header_suffix(Some(&refresh_button()));
        page.add(&probe_group);
        ComputerPane {
            page,
            rest,
            header,
            probe: Redrawn::new(probe_group),
            arm64,
        }
    }

    pub fn render(
        &self,
        facts: &Facts<'_>,
        drawn: &Drawn,
        probe: Option<&Result<ComputerPermissions, String>>,
        architecture: Option<&str>,
    ) {
        // There is no helper to install on arm64 (M38 §8.2): say so, and offer nothing.
        let arm = matches!(architecture, Some("aarch64" | "arm64"));
        self.arm64.set_visible(arm);
        for group in [&self.header.group, &self.rest.group, &self.probe.group] {
            group.set_visible(!arm);
        }
        self.header
            .show(header(facts, COMPUTER_ENABLED, "Computer use"), |h| {
                header_row(h, COMPUTER_SIDECAR)
            });
        self.rest.show(drawn.clone());
        self.probe
            .show(probe.cloned(), |probe| probe_rows(probe.as_ref()));
    }
}

fn refresh_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Check again")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    button.set_action_name(Some("win.probe-computer"));
    button
}

fn probe_rows(probe: Option<&Result<ComputerPermissions, String>>) -> Vec<gtk::Widget> {
    let permissions = match probe {
        None => return vec![fact("Reading from Fermix…", "")],
        Some(Err(sentence)) => return vec![fact("The helper could not be checked", sentence)],
        Some(Ok(permissions)) => permissions,
    };
    let when = permissions
        .probed_at
        .as_deref()
        .map(|at| format!(" · Checked {}", at.replace('T', " ").trim_end_matches('Z')))
        .unwrap_or_default();
    let at = permissions.probed_at.as_deref();
    vec![
        fact(
            "Screen capture",
            &format!("{}{when}", verdict(permissions.screen_capture, at)),
        ),
        fact(
            "Input control",
            &format!("{}{when}", verdict(permissions.input_control, at)),
        ),
    ]
}

fn fact(title: &str, subtitle: &str) -> gtk::Widget {
    adw::ActionRow::builder()
        .title(title)
        .subtitle(glib::markup_escape_text(subtitle))
        .css_classes(["property"])
        .build()
        .upcast()
}
