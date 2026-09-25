//! Home: is Fermix working, what does it answer with, and what needs attention.

use crate::state::{Background, Connection, Snapshot, State};
use adw::prelude::*;
use fermix_client::overview::{channels_line, duration_words, tools_count, Overview};
use fermix_client::providers::{connection, Connection as Link};
use fermix_client::service::{background_switch, service_line};
use fermix_client::settings::pane;
use fermix_client::view::{
    answers_with, attention_rows, status_word, AttentionAction, AttentionRow, DaemonProblem,
};
use gtk::glib::{self, variant::ToVariant};
use std::cell::RefCell;

/// What the Attention group shows. It is rebuilt only when this changes, so a
/// refresh never tears down a row under the pointer.
#[derive(Debug, Clone, PartialEq)]
enum Attention {
    Rows(Vec<AttentionRow>),
    Down {
        problem: DaemonProblem,
        wake_failed: bool,
    },
    Waiting,
}

#[derive(Default)]
struct Shown {
    attention: Option<Attention>,
    widgets: Vec<gtk::Widget>,
}

pub struct HomePage {
    pub root: adw::PreferencesPage,
    status: gtk::Label,
    answers: gtk::Label,
    attention: adw::PreferencesGroup,
    shown: RefCell<Shown>,
    version: gtk::Label,
    pid: gtk::Label,
    signed_in: gtk::Label,
    uptime: gtk::Label,
    channels: gtk::Label,
    skills: gtk::Label,
    tools: gtk::Label,
    protocol: gtk::Label,
    background: BackgroundRows,
    service: gtk::Label,
}

fn value_label() -> gtk::Label {
    gtk::Label::builder()
        .css_classes(["dim-label"])
        .valign(gtk::Align::Center)
        .build()
}

fn value_row(title: &str, value: &gtk::Label) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    row.add_suffix(value);
    row
}

impl HomePage {
    pub fn new() -> Self {
        let (status, answers) = (value_label(), value_label());
        let (version, pid, signed_in) = (value_label(), value_label(), value_label());
        let (uptime, channels, skills) = (value_label(), value_label(), value_label());
        let (tools, protocol, session) = (value_label(), value_label(), value_label());
        let service = value_label();
        session.set_text(&session_line());
        let fermix = fermix_group(&status, &answers);

        let attention = adw::PreferencesGroup::builder()
            .title("Attention")
            .visible(false)
            .build();
        let details = adw::ExpanderRow::builder().title("Details").build();
        for (title, value) in [
            ("Engine version", &version),
            ("Running for", &uptime),
            ("Providers signed in", &signed_in),
            ("Channels on", &channels),
            ("Skills", &skills),
            ("Tools", &tools),
            ("Management protocol", &protocol),
            ("Process", &pid),
            ("Service", &service),
            ("This desktop session", &session),
        ] {
            details.add_row(&value_row(title, value));
        }
        let more = adw::PreferencesGroup::new();
        more.add(&details);

        let background = BackgroundRows::new();
        let root = adw::PreferencesPage::new();
        for group in [&fermix, &attention, &background.group, &more] {
            root.add(group);
        }
        HomePage {
            root,
            status,
            answers,
            attention,
            shown: RefCell::default(),
            version,
            pid,
            signed_in,
            uptime,
            channels,
            skills,
            tools,
            protocol,
            background,
            service,
        }
    }

    pub fn render(&self, state: &State) {
        self.background.render(&state.background);
        let line = state
            .background
            .service
            .as_ref()
            .map_or_else(|| "Reading…".to_owned(), service_line);
        self.service.set_text(&line);
        match &state.connection {
            Connection::Connecting => self.render_waiting("Connecting…"),
            Connection::Up(snapshot) => self.render_up(snapshot),
            Connection::Down(_) if state.waking => self.render_waiting("Starting…"),
            Connection::Down(problem) => self.render_down(problem, state.wake_failed),
        }
    }

    fn render_up(&self, snapshot: &Snapshot) {
        let state = &snapshot.state;
        self.status.set_text(status_word(state));
        self.answers.set_text(&answers_with(state));
        self.version.set_text(&snapshot.version);
        self.pid
            .set_text(snapshot.pid.as_deref().unwrap_or("Unknown"));
        let signed_in = state
            .providers
            .iter()
            .filter(|p| connection(p) == Link::Connected)
            .count();
        self.signed_in.set_text(&signed_in.to_string());
        self.protocol.set_text(&format!("v{}", snapshot.protocol));
        self.render_overview(snapshot.overview.as_ref());
        self.show_attention(Attention::Rows(attention_rows(state)));
    }

    /// The daemon's own account of itself. Without one, the rows say so rather than guess.
    fn render_overview(&self, overview: Option<&Overview>) {
        let Some(overview) = overview else {
            for label in [&self.uptime, &self.channels, &self.skills, &self.tools] {
                label.set_text("Not reported");
            }
            return;
        };
        let uptime = overview.daemon.uptime_ms.filter(|ms| *ms > 0);
        self.uptime
            .set_text(&uptime.map_or_else(|| "Not reported".into(), duration_words));
        self.channels.set_text(&channels_line(overview));
        self.skills
            .set_text(&overview.capabilities.skill.to_string());
        self.tools.set_text(&tools_count(overview).to_string());
    }

    fn render_down(&self, problem: &DaemonProblem, wake_failed: bool) {
        self.status.set_text(problem.status_word());
        self.clear_values();
        self.show_attention(Attention::Down {
            problem: problem.clone(),
            wake_failed,
        });
    }

    fn render_waiting(&self, word: &str) {
        self.status.set_text(word);
        self.clear_values();
        self.show_attention(Attention::Waiting);
    }

    /// Values Fermix did not just give are left blank, never guessed.
    fn clear_values(&self) {
        let labels = [
            &self.answers,
            &self.version,
            &self.pid,
            &self.signed_in,
            &self.uptime,
            &self.channels,
            &self.skills,
            &self.tools,
            &self.protocol,
        ];
        for label in labels {
            label.set_text("");
        }
    }

    fn show_attention(&self, next: Attention) {
        let mut shown = self.shown.borrow_mut();
        if shown.attention.as_ref() == Some(&next) {
            return;
        }
        for old in shown.widgets.drain(..) {
            self.attention.remove(&old);
        }
        let widgets = attention_widgets(&next);
        for widget in &widgets {
            self.attention.add(widget);
        }
        self.attention.set_visible(!widgets.is_empty());
        *shown = Shown {
            attention: Some(next),
            widgets,
        };
    }
}

/// Status, and what Fermix answers with, which opens Providers.
fn fermix_group(status: &gtk::Label, answers: &gtk::Label) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title("Fermix").build();
    group.add(&value_row("Status", status));
    let answers_row = value_row("Answers with", answers);
    answers_row.set_activatable(true);
    answers_row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
    answers_row.set_action_name(Some("win.page"));
    answers_row.set_action_target_value(Some(&"providers".to_variant()));
    group.add(&answers_row);
    group
}

/// Two registrations, independent of each other: the service with systemd,
/// this window with the desktop's login list (spec §6.3, §6.6).
struct BackgroundRows {
    group: adw::PreferencesGroup,
    service: adw::SwitchRow,
    login: adw::SwitchRow,
}

impl BackgroundRows {
    fn new() -> BackgroundRows {
        let service = switch_row("Run in the background", "win.run-in-background");
        service.set_subtitle_selectable(true);
        let login = switch_row("Open at login", "win.open-at-login");
        login.set_subtitle("Opens this window when you log in.");
        let group = adw::PreferencesGroup::builder().title("Background").build();
        group.add(&service);
        group.add(&login);
        BackgroundRows {
            group,
            service,
            login,
        }
    }

    /// While a change runs, each switch shows where it is heading.
    fn render(&self, bg: &Background) {
        let busy = bg.service_change.is_some();
        let drawn = background_switch(bg.service.as_ref(), bg.binding, busy);
        let note = match (bg.service_change, &bg.service_error) {
            (Some(true), _) => "Turning on…",
            (Some(false), _) => "Turning off…",
            (None, Some(error)) => error.as_str(),
            (None, None) => drawn.note.as_str(),
        };
        self.service.set_subtitle(&glib::markup_escape_text(note));
        let refused = !busy && bg.service_error.is_some();
        if refused {
            self.service.add_css_class("setting-refused");
        } else {
            self.service.remove_css_class("setting-refused");
        }
        self.service.set_sensitive(drawn.sensitive);
        self.service
            .set_active(bg.service_change.unwrap_or(drawn.on));
        self.login.set_sensitive(bg.login_change.is_none());
        self.login
            .set_active(bg.login_change.unwrap_or(bg.opens_at_login));
    }
}

/// A switch whose every move goes to the controller, which ignores a move that
/// only mirrors what was drawn.
fn switch_row(title: &str, action: &'static str) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).build();
    row.connect_active_notify(move |row| {
        let wanted = row.is_active().to_variant();
        if let Err(e) = row.activate_action(action, Some(&wanted)) {
            glib::g_warning!("fermix", "{action} could not be sent: {e}");
        }
    });
    row
}

fn attention_widgets(attention: &Attention) -> Vec<gtk::Widget> {
    match attention {
        Attention::Rows(rows) => rows.iter().map(attention_widget).collect(),
        Attention::Down {
            problem,
            wake_failed,
        } => vec![down_row(problem, *wake_failed).upcast()],
        Attention::Waiting => vec![waiting_row().upcast()],
    }
}

fn waiting_row() -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title("Waiting for Fermix to answer")
        .build();
    row.add_suffix(&adw::Spinner::new());
    row
}

fn attention_widget(row: &AttentionRow) -> gtk::Widget {
    // Rows parse markup, and the body can carry the daemon's own sentences.
    let widget = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&row.title))
        .subtitle(glib::markup_escape_text(&row.body))
        .build();
    let button = match &row.action {
        Some(AttentionAction::Door { target, verb }) => {
            let b = action_button(verb, "win.door", Some(target));
            b.add_css_class("suggested-action");
            Some(b)
        }
        Some(AttentionAction::OpenPane(slug)) => {
            let title = pane(slug).map_or("Settings", |p| p.title);
            Some(action_button(
                &format!("Open {title}"),
                "win.page",
                Some(slug),
            ))
        }
        Some(AttentionAction::Restart) => {
            Some(action_button("Restart Fermix…", "win.restart", None))
        }
        Some(AttentionAction::OpenSetupPage) => Some(action_button(
            "Open setup page",
            "win.open-setup-page",
            None,
        )),
        None => None,
    };
    if let Some(button) = button {
        widget.add_suffix(&button);
    }
    widget.upcast()
}

/// A failed start or restart names the terminal command that shows why.
pub const START_COMMAND: &str = "systemctl --user start fermix";

type DownCopy = (&'static str, String, Option<(&'static str, &'static str)>);

fn down_row(problem: &DaemonProblem, wake_failed: bool) -> adw::ActionRow {
    let (title, subtitle, button) = match down_copy(problem) {
        (_, _, Some((_, action))) if wake_failed => (
            "Fermix did not come back",
            format!("To see why, run {START_COMMAND} in a terminal."),
            Some(("Try again", action)),
        ),
        copy => copy,
    };
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(glib::markup_escape_text(&subtitle))
        .subtitle_selectable(true)
        .build();
    if let Some((label, action)) = button {
        row.add_suffix(&action_button(label, action, None));
    }
    row
}

fn down_copy(problem: &DaemonProblem) -> DownCopy {
    match problem {
        DaemonProblem::UpdateNeeded(sentence) => ("Update needed", sentence.clone(), None),
        DaemonProblem::NotRunning => (
            "Fermix is not running",
            "Sign-in and chat need it.".to_owned(),
            Some(("Start Fermix", "win.start-service")),
        ),
        DaemonProblem::NotResponding => (
            "Fermix is not responding",
            "Its socket is there but nothing answers.".to_owned(),
            Some(("Restart Fermix", "win.restart-service")),
        ),
        DaemonProblem::Broken(detail) => (
            "Fermix answered in a way this app does not understand",
            detail.clone(),
            Some(("Restart Fermix", "win.restart-service")),
        ),
    }
}

pub fn action_button(label: &str, action: &str, target: Option<&str>) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(label)
        .valign(gtk::Align::Center)
        .build();
    button.set_action_name(Some(action));
    if let Some(target) = target {
        button.set_action_target_value(Some(&target.to_variant()));
    }
    button
}

/// What this app sees of the desktop it runs in: its own observation, never
/// the daemon's (M38 §5.6). "X11 · pop:GNOME", for example.
fn session_line() -> String {
    let display = gtk::gdk::Display::default().map(|d| d.type_().name().to_owned());
    let kind = match display.as_deref() {
        Some(name) if name.contains("Wayland") => "Wayland",
        Some(name) if name.contains("X11") => "X11",
        Some(name) if name.contains("Broadway") => "Broadway",
        _ => "Unknown display",
    };
    match std::env::var("XDG_CURRENT_DESKTOP") {
        Ok(desktop) if !desktop.is_empty() => format!("{kind} · {desktop}"),
        _ => kind.to_owned(),
    }
}
