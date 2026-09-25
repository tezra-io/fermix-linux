//! Doctor: runs the daemon's local checks and shows what they found, problems
//! first. The page owns its session and makes its own daemon calls. A remediation
//! that opens a Settings pane goes through the callback it was given; Restart
//! opens the window's one Restart confirmation.

use crate::daemon::Daemon;
use adw::prelude::*;
use fermix_client::doctor::{
    check_title, detail, grouped, headline, poll_cap, progress, Check, CheckStatus, Fix, Group,
    Scope, Session, SessionStatus, Tone, POLL_MS, SERVICE_STEPS_BODY, SERVICE_STEPS_COMMAND,
    STALLED,
};
use fermix_client::management::CallError;
use fermix_client::view::{daemon_problem, DaemonProblem};
use gtk::glib;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// The groups in the order the page draws them.
const GROUPS: [Group; 3] = [Group::Attention, Group::Passed, Group::NotRun];

#[derive(Default)]
struct Run {
    /// The newest view of the last session this page started.
    session: Option<Session>,
    /// The running session being polled; `None` once it ends, stalls or is cancelled.
    following: Option<String>,
    /// A `doctor.start` is on the wire.
    starting: bool,
    /// The last call did not reach the daemon; only an answer shows the checks again.
    down: bool,
}

/// The rows on screen and the session they were drawn from, so an unchanged
/// poll never tears down a row under the pointer.
#[derive(Default)]
struct Drawn {
    session: Option<Session>,
    rows: Vec<(usize, gtk::Widget)>,
}

struct Banner {
    row: adw::ActionRow,
    progress: gtk::ProgressBar,
    run: gtk::Button,
    cancel: gtk::Button,
    /// The support line: busy, a refusal, or a reload that did not land.
    note: gtk::Label,
}

pub struct DoctorPage {
    pub root: gtk::Widget,
    stack: gtk::Stack,
    down: adw::StatusPage,
    banner: Banner,
    groups: Vec<adw::PreferencesGroup>,
    daemon: Daemon,
    on_open_pane: Box<dyn Fn(&str)>,
    run: RefCell<Run>,
    drawn: RefCell<Drawn>,
}

impl DoctorPage {
    /// `on_open_pane` opens the Settings pane with this slug.
    pub fn new(daemon: Daemon, on_open_pane: Box<dyn Fn(&str)>) -> Rc<Self> {
        let banner = banner();
        let checks = adw::PreferencesPage::new();
        let top = adw::PreferencesGroup::new();
        top.add(&banner.row);
        top.add(&banner.progress);
        top.add(&banner.note);
        checks.add(&top);
        let groups: Vec<adw::PreferencesGroup> = GROUPS
            .iter()
            .map(|g| {
                let group = adw::PreferencesGroup::builder()
                    .title(g.title())
                    .visible(false)
                    .build();
                checks.add(&group);
                group
            })
            .collect();
        let down = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .build();
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        stack.add_named(&checks, Some("checks"));
        stack.add_named(&down, Some("down"));
        let page = Rc::new(DoctorPage {
            root: stack.clone().upcast(),
            stack,
            down,
            banner,
            groups,
            daemon,
            on_open_pane,
            run: RefCell::default(),
            drawn: RefCell::default(),
        });
        page.wire_banner();
        page.render();
        page
    }

    /// Page entry runs the local checks, as macOS does, unless a run this page
    /// started is still going: at most two run at once, so it is followed, not doubled.
    pub fn shown(self: &Rc<Self>) {
        self.run_checks();
    }

    fn wire_banner(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.banner.run.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.run_checks();
            }
        });
        let weak = Rc::downgrade(self);
        self.banner.cancel.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                glib::spawn_future_local(page.cancel());
            }
        });
    }

    fn run_checks(self: &Rc<Self>) {
        let run = self.run.borrow();
        if run.starting || run.following.is_some() {
            return;
        }
        drop(run);
        glib::spawn_future_local(self.clone().start());
    }

    async fn start(self: Rc<Self>) {
        self.run.borrow_mut().starting = true;
        self.set_note(None);
        self.render();
        let answer = self.daemon.call(|m| m.doctor_start(Scope::Local)).await;
        self.run.borrow_mut().starting = false;
        let session = match answer {
            Ok(session) => session,
            Err(e) => return self.failed(e),
        };
        let (id, budget_ms) = (session.session_id.clone(), session.budget_ms);
        let running = session.status == SessionStatus::Running;
        self.land(session);
        if running {
            self.follow(&id, budget_ms).await;
        }
    }

    /// Polls one session until it ends, is cancelled, or the polls run out.
    /// Before and after each poll it checks the page still follows this session:
    /// a Cancel may have ended it while the poll was on the wire.
    async fn follow(self: &Rc<Self>, session_id: &str, budget_ms: u64) {
        for _ in 0..poll_cap(budget_ms) {
            glib::timeout_future(Duration::from_millis(POLL_MS)).await;
            if !self.follows(session_id) {
                return;
            }
            let id = session_id.to_owned();
            let answer = self.daemon.call(move |m| m.doctor_get(&id)).await;
            if !self.follows(session_id) {
                return;
            }
            match answer {
                Ok(view) if view.status == SessionStatus::Running => self.land(view),
                Ok(view) => return self.land(view),
                Err(e) => {
                    self.run.borrow_mut().following = None;
                    return self.failed(e);
                }
            }
        }
        self.run.borrow_mut().following = None;
        self.render();
    }

    fn follows(&self, session_id: &str) -> bool {
        self.run.borrow().following.as_deref() == Some(session_id)
    }

    /// A view of the session becomes what the page shows, followed while it runs.
    fn land(self: &Rc<Self>, session: Session) {
        let mut run = self.run.borrow_mut();
        run.following =
            (session.status == SessionStatus::Running).then(|| session.session_id.clone());
        run.session = Some(session);
        run.down = false;
        drop(run);
        self.render();
    }

    /// Cancels the followed run and shows how it ended: its last checks can land
    /// in the moment before the cancel does.
    async fn cancel(self: Rc<Self>) {
        let Some(id) = self.run.borrow_mut().following.take() else {
            return;
        };
        self.render();
        match self.daemon.call(move |m| m.doctor_cancel(&id)).await {
            Ok(view) => self.land(view),
            Err(e) => self.failed(e),
        }
    }

    /// `settings.reload`; once the file is read again the checks run again, so
    /// the row that asked for it redraws.
    async fn reload(self: Rc<Self>) {
        self.set_note(None);
        match self.daemon.call(|m| m.settings_reload()).await {
            Ok(_) => self.run_checks(),
            Err(e) => self.failed(e),
        }
    }

    /// A refusal (busy, a session no longer kept) is shown in the daemon's words.
    /// Losing the daemon, or one too old or too new for this app, shows the down page.
    fn failed(self: &Rc<Self>, e: CallError) {
        let problem = daemon_problem(&e);
        match e {
            CallError::Refused(refusal) if !matches!(problem, DaemonProblem::UpdateNeeded(_)) => {
                self.run.borrow_mut().down = false;
                self.set_note(Some(&refusal.sentence));
                self.render();
            }
            other => {
                glib::g_warning!("fermix", "Doctor could not reach the daemon: {other:?}");
                self.show_down(&problem);
            }
        }
    }

    fn show_down(&self, problem: &DaemonProblem) {
        let (title, description) = match problem {
            DaemonProblem::UpdateNeeded(sentence) => ("Update needed", sentence.as_str()),
            DaemonProblem::NotRunning => (
                "Fermix is not running",
                "Doctor runs its checks inside Fermix. Start it from Home.",
            ),
            DaemonProblem::NotResponding => (
                "Fermix is not responding",
                "Doctor runs its checks inside Fermix. Restart it from Home.",
            ),
            DaemonProblem::Broken(_) => (
                "Fermix answered in a way this app does not understand",
                "Doctor runs its checks inside Fermix. Restart it from Home.",
            ),
        };
        let description = glib::markup_escape_text(description);
        self.down.set_title(title);
        self.down.set_description(Some(description.as_str()));
        self.run.borrow_mut().down = true;
        self.stack.set_visible_child_name("down");
    }

    fn set_note(&self, sentence: Option<&str>) {
        self.banner.note.set_text(sentence.unwrap_or(""));
        self.banner.note.set_visible(sentence.is_some());
    }

    fn render(self: &Rc<Self>) {
        let run = self.run.borrow();
        self.stack
            .set_visible_child_name(if run.down { "down" } else { "checks" });
        let running = run.following.is_some();
        let (title, subtitle) = match &run.session {
            _ if run.starting => ("Running checks".to_owned(), String::new()),
            // Still running by its last view, but no longer followed: the polls ran out.
            Some(s) if s.status == SessionStatus::Running && !running => {
                (STALLED.to_owned(), detail(s))
            }
            Some(s) => (headline(s), detail(s)),
            None => ("No checks have run yet".to_owned(), String::new()),
        };
        self.banner.row.set_title(&title);
        self.banner.row.set_subtitle(&subtitle);
        let done = run.session.as_ref().filter(|_| running).and_then(progress);
        self.banner
            .progress
            .set_fraction(done.map_or(0.0, fraction));
        self.banner.progress.set_visible(running || run.starting);
        self.banner.run.set_visible(!running && !run.starting);
        self.banner.cancel.set_visible(running);
        let session = run.session.clone();
        drop(run);
        self.draw_rows(session);
    }

    fn draw_rows(self: &Rc<Self>, session: Option<Session>) {
        let mut drawn = self.drawn.borrow_mut();
        if drawn.session == session {
            return;
        }
        for (index, row) in drawn.rows.drain(..) {
            self.groups[index].remove(&row);
        }
        let checks = session.as_ref().map_or(&[][..], |s| s.checks.as_slice());
        for (group, rows) in grouped(checks) {
            let index = GROUPS
                .iter()
                .position(|g| *g == group)
                .expect("every group has a place on the page");
            for check in rows {
                let row = self.check_row(check);
                self.groups[index].add(&row);
                drawn.rows.push((index, row.upcast()));
            }
        }
        for (index, group) in self.groups.iter().enumerate() {
            group.set_visible(drawn.rows.iter().any(|(i, _)| *i == index));
        }
        drawn.session = session;
    }

    /// The check's title, the daemon's sentence and remediation as sent, the
    /// verdict, and the one button where the app can act on the remediation.
    fn check_row(self: &Rc<Self>, check: &Check) -> adw::ActionRow {
        // Plain text before any text is set, or a summary like `apt update && apt
        // upgrade` is parsed as markup and fails. A builder does not keep that order.
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&check_title(&check.id));
        row.set_subtitle(&row_text(check));
        row.set_subtitle_selectable(true);
        row.add_suffix(&verdict_label(check.status));
        if let (Some(fix), Some(remediation)) = (check.fix(), check.remediation.as_ref()) {
            row.add_suffix(&self.fix_button(fix, remediation.title.clone()));
        }
        row
    }

    fn fix_button(self: &Rc<Self>, fix: Fix, heading: String) -> gtk::Button {
        let button = gtk::Button::builder()
            .label(fix.label())
            .valign(gtk::Align::Center)
            .build();
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(page) = weak.upgrade() {
                page.perform(&fix, &heading);
            }
        });
        button
    }

    fn perform(self: &Rc<Self>, fix: &Fix, heading: &str) {
        match fix {
            Fix::OpenPane { slug, .. } => (self.on_open_pane)(slug),
            Fix::Restart => {
                if let Err(e) = self.root.activate_action("win.restart", None) {
                    glib::g_warning!("fermix", "Doctor could not open Restart: {e}");
                }
            }
            Fix::Reload => {
                glib::spawn_future_local(self.clone().reload());
            }
            Fix::ServiceSteps => service_steps(heading).present(Some(&self.root)),
        }
    }
}

fn banner() -> Banner {
    let run = gtk::Button::builder()
        .label("Run checks")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    let cancel = gtk::Button::builder()
        .label("Cancel")
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    let row = adw::ActionRow::builder().use_markup(false).build();
    row.add_suffix(&run);
    row.add_suffix(&cancel);
    let progress = gtk::ProgressBar::builder()
        .margin_top(12)
        .visible(false)
        .build();
    let note = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .margin_top(12)
        .visible(false)
        .build();
    Banner {
        row,
        progress,
        run,
        cancel,
        note,
    }
}

fn fraction((done, total): (u32, u32)) -> f64 {
    if total == 0 {
        return 0.0;
    }
    (f64::from(done) / f64::from(total)).min(1.0)
}

/// The daemon's sentence, then its remediation's title and body where it sent one.
fn row_text(check: &Check) -> String {
    match &check.remediation {
        Some(r) => format!("{}\n{}\n{}", check.summary, r.title, r.body),
        None => check.summary.clone(),
    }
}

fn verdict_label(status: CheckStatus) -> gtk::Label {
    let tone = match status.tone() {
        Tone::Good => "success",
        Tone::Warn => "warning",
        Tone::Bad => "error",
        Tone::Quiet => "dim-label",
    };
    gtk::Label::builder()
        .label(status.verdict())
        .css_classes(["caption-heading", tone])
        .valign(gtk::Align::Center)
        .build()
}

/// Linux's answer to another Fermix service: the package's own command.
fn service_steps(heading: &str) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some(heading), Some(SERVICE_STEPS_BODY));
    let command = gtk::Label::builder()
        .label(SERVICE_STEPS_COMMAND)
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    dialog.set_extra_child(Some(&command));
    dialog.add_response("close", "Close");
    dialog.set_close_response("close");
    dialog
}
