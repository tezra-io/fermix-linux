//! The Always On pane (M75 §14): where Fermix runs, drawn from `home.status` alone. Before a move
//! it explains what moves and what stays, takes the owner through a code, the server that
//! enrolls with it, its review, the readiness report and the recovery passphrase, and starts the
//! move; it follows the move across the engine's restart, and says so plainly if the engine does
//! not come back. After the move it shows the server this computer reaches, the server's own
//! recovery, and the steps that replace the server; on a primary, its desktops and their
//! switches; always, recovery.
//!
//! The pane owns its daemon calls, as Integrations does, and reads `home.status` every few
//! seconds while it is on screen, at most `POLLS_PER_VISIT` times a visit. `home.status` is
//! answered by this computer's own engine, so the pane draws itself while the server it forwards
//! to is unreachable, and while the engine restarts into a move. The `snapshots` section, and any
//! other section the daemon files under this pane, is added below by `SettingsPage` and drawn by
//! the one settings model.
//!
//! Every sentence about the home is the daemon's; the labels are `fermix_client::always_on`'s.

use crate::daemon::Daemon;
use crate::dialogs::{form_dialog, keep_with, leave, SECRET_WIDTH};
use crate::status::{down_view, DownPage};
use adw::prelude::*;
use fermix_client::always_on::{
    self as words, grant_rows, move_to, passphrase, unconfirmed, view, CandidateRow, Checked,
    DesktopRow, DesktopStage, Destination, DeviceGrants, GrantRow, HomeStatus, ItemStatus,
    MoveFrom, Moving, Readiness, Recovery, Replacing, Review, Setup, Stage, Steps, View,
    POLLS_PER_VISIT, POLL_EVERY,
};
use fermix_client::management::CallError;
use fermix_client::secrets::refused_sentence;
use fermix_client::view::{daemon_problem, DaemonProblem};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

/// The groups and rows that stay for the pane's life, so a read every few seconds never
/// rebuilds what someone is typing in or reading.
struct Widgets {
    notice: adw::PreferencesGroup,
    explain: adw::PreferencesGroup,
    choose: adw::PreferencesGroup,
    aws: gtk::CheckButton,
    host: gtk::CheckButton,
    code: adw::PreferencesGroup,
    get_code: gtk::Button,
    guide: adw::ActionRow,
    command: adw::ActionRow,
    code_row: adw::ActionRow,
    code_note: adw::ActionRow,
    candidates: List,
    review: List,
    instance: List,
    /// The readiness report of the candidate under review, between the review and Move.
    readiness: List,
    check_again: gtk::Button,
    passphrase: adw::PreferencesGroup,
    first: adw::PasswordEntryRow,
    second: adw::PasswordEntryRow,
    start: adw::ButtonRow,
    refusal: gtk::Label,
    moving: adw::PreferencesGroup,
    moving_row: adw::ActionRow,
    moving_spinner: adw::Spinner,
    /// Start or restart Fermix, once the wait for it after a move's restart went past its grace.
    moving_action: gtk::Button,
    server: adw::PreferencesGroup,
    server_row: adw::ActionRow,
    server_state: gtk::Label,
    moved_row: adw::ActionRow,
    /// The device role with no server, or a candidate: one sentence alone.
    unlinked: adw::PreferencesGroup,
    /// On a desktop: what replacing the server does, that moving back is not in this version, and
    /// the button that opens (or closes) the steps.
    replace: adw::PreferencesGroup,
    replace_button: gtk::Button,
    /// On a desktop: the folders the server may read here beyond the sandbox, each removable.
    grants: List,
    desktops: List,
    recovery: List,
    back_up: gtk::Button,
    set_passphrase: gtk::Button,
    stopped: adw::PreferencesGroup,
}

/// A group whose rows follow a list in the view: rebuilt only when that list changed.
struct List {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk::Widget>>,
}

impl List {
    fn new(group: adw::PreferencesGroup) -> List {
        List {
            group,
            rows: RefCell::default(),
        }
    }

    fn replace(&self, rows: Vec<gtk::Widget>) {
        for row in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }
        for row in &rows {
            self.group.add(row);
        }
        *self.rows.borrow_mut() = rows;
    }
}

pub struct AlwaysOnPane {
    /// The pane itself, for the callbacks of the rows it draws.
    me: Weak<AlwaysOnPane>,
    /// The pane: its page, or the down page while this computer's Fermix cannot be read.
    pub root: gtk::Stack,
    /// Where `SettingsPage` adds the daemon's sections for this pane, below the pane's own groups.
    pub page: adw::PreferencesPage,
    daemon: Daemon,
    toast: Rc<dyn Fn(&str)>,
    read: RefCell<Option<Result<HomeStatus, DaemonProblem>>>,
    /// `device.grants.list` as last answered on a desktop, or why it was not.
    grants: RefCell<Option<Result<DeviceGrants, String>>>,
    /// The grants as last drawn, so a read every few seconds rebuilds nothing unchanged.
    drawn_grants: RefCell<Option<(String, Vec<GrantRow>)>>,
    steps: RefCell<Steps>,
    /// Each visit to the pane counts up, so the reads of an earlier visit stop.
    visit: Cell<u64>,
    /// This visit read `POLLS_PER_VISIT` times and stopped.
    stopped: Cell<bool>,
    down: DownPage,
    w: Widgets,
    drawn: RefCell<Option<View>>,
    /// The review on screen may start its move: its readiness report has nothing that blocks.
    /// The two passphrase entries must agree as well.
    can_start: Cell<bool>,
    /// Where the section that configures this computer goes: after the steps, before the
    /// desktops and recovery.
    this_computer_at: i32,
}

impl AlwaysOnPane {
    /// `toast` shows one short line in the window.
    pub fn new(daemon: Daemon, toast: Rc<dyn Fn(&str)>) -> Rc<AlwaysOnPane> {
        let page = adw::PreferencesPage::new();
        let w = widgets();
        // One order serves every stage: a primary sees the explanation and the steps; a desktop
        // its server, then the steps that replace it when they are open; a move, its sentence.
        let above = [
            &w.stopped,
            &w.notice,
            &w.explain,
            &w.moving,
            &w.server,
            &w.unlinked,
            &w.replace,
            &w.choose,
            &w.code,
            &w.candidates.group,
            &w.review.group,
            &w.instance.group,
            &w.readiness.group,
            &w.passphrase,
        ];
        for group in above {
            page.add(group);
        }
        let this_computer_at = i32::try_from(above.len()).expect("a few groups");
        page.add(&w.grants.group);
        page.add(&w.desktops.group);
        page.add(&w.recovery.group);
        let down = DownPage::new();
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        root.add_named(&page, Some("page"));
        root.add_named(&down.page, Some("down"));
        let pane = Rc::new_cyclic(|me| AlwaysOnPane {
            me: me.clone(),
            root,
            page,
            daemon,
            toast,
            read: RefCell::default(),
            grants: RefCell::default(),
            drawn_grants: RefCell::default(),
            steps: RefCell::default(),
            visit: Cell::new(0),
            stopped: Cell::new(false),
            down,
            w,
            drawn: RefCell::default(),
            can_start: Cell::new(false),
            this_computer_at,
        });
        pane.wire();
        pane.render();
        pane
    }

    /// Adds one of the daemon's sections filed under this pane, drawn by the one settings model:
    /// the section that configures this computer after the steps, any other after recovery.
    pub fn add_section(&self, id: &str, group: &adw::PreferencesGroup) {
        if id == words::THIS_COMPUTER {
            self.page.insert(group, self.this_computer_at);
        } else {
            self.page.add(group);
        }
    }

    /// Starts a visit: reads `home.status` now and every `POLL_EVERY` while the pane stays on
    /// screen, at most `POLLS_PER_VISIT` times. A visit already running stops at its next read.
    /// The pane starts one itself each time it comes on screen; Settings starts one as the pane
    /// opens and when Fermix restarts under it, as it does into a move.
    pub fn shown(self: &Rc<Self>) {
        let visit = self.visit.get() + 1;
        self.visit.set(visit);
        self.stopped.set(false);
        let pane = self.clone();
        glib::spawn_future_local(async move { pane.follow(visit).await });
    }

    async fn follow(self: Rc<Self>, visit: u64) {
        for _ in 0..POLLS_PER_VISIT {
            self.read_status().await;
            glib::timeout_future(POLL_EVERY).await;
            if self.visit.get() != visit || !self.root.is_mapped() {
                return;
            }
        }
        self.stopped.set(true);
        self.render();
    }

    /// Reads `home.status` once. A read asked before a move this app started lands after it at
    /// times; it says nothing about that move, so it is dropped.
    async fn read_status(&self) {
        let ticket = self.steps.borrow().ticket();
        let answer = self.daemon.call(|m| m.home_status()).await;
        let read = answer.map_err(|e| {
            glib::g_debug!("fermix", "home.status was not read: {e:?}");
            daemon_problem(&e)
        });
        let current = {
            let mut steps = self.steps.borrow_mut();
            match &read {
                Ok(status) => steps.settle(status, ticket, unix_now()),
                Err(_) => steps.unanswered(ticket, unix_now()),
            }
        };
        if !current {
            glib::g_debug!(
                "fermix",
                "a home.status read from before the move was dropped"
            );
            return;
        }
        let linked = read.as_ref().is_ok_and(HomeStatus::linked);
        *self.read.borrow_mut() = Some(read);
        if linked {
            self.read_grants().await;
        }
        self.render();
    }

    /// Reads the folders the server may read here. This computer answers it, so it reads while
    /// the server is unreachable too.
    async fn read_grants(&self) {
        let answer = self.daemon.call(|m| m.device_grants_list()).await;
        *self.grants.borrow_mut() = Some(answer.map_err(|e| sentence_for(e, "device.grants.list")));
    }

    fn render(&self) {
        let view = {
            let read = self.read.borrow();
            let read = read.as_ref().map(|r| r.as_ref());
            view(read, &self.steps.borrow(), unix_now())
        };
        self.w.stopped.set_visible(self.stopped.get());
        // Read beside `home.status`, so drawn whether or not the view changed.
        self.draw_grants(matches!(view.stage, Stage::Desktop(_)));
        let drawn = self.drawn.borrow().clone();
        if drawn.as_ref() == Some(&view) {
            return;
        }
        if let Stage::Down(problem) = &view.stage {
            self.down
                .show(down_view(problem, false, true, words::NEEDS_FERMIX));
            self.root.set_visible_child_name("down");
        } else {
            self.root.set_visible_child_name("page");
            self.draw(&view, drawn.as_ref());
        }
        *self.drawn.borrow_mut() = Some(view);
    }

    fn draw(&self, view: &View, before: Option<&View>) {
        let first_move = matches!(view.stage, Stage::Setup(_));
        self.w.explain.set_visible(first_move);
        self.draw_setup(
            setup_of(&view.stage),
            before.and_then(|v| setup_of(&v.stage)),
        );
        self.draw_moving(&view.stage);
        self.draw_desktop(&view.stage);
        if before.map(|v| &v.desktops) != Some(&view.desktops) {
            self.w
                .desktops
                .replace(view.desktops.iter().map(|d| self.desktop_row(d)).collect());
        }
        self.w.desktops.group.set_visible(!view.desktops.is_empty());
        if before.map(|v| &v.recovery) != Some(&view.recovery) {
            self.draw_recovery(view.recovery.as_ref());
        }
    }

    /// The folders the server may read here, on a desktop: the daemon's sentence, then each
    /// folder with when it was allowed and Remove.
    fn draw_grants(&self, desktop: bool) {
        let w = &self.w;
        let shown = match self.grants.borrow().as_ref().filter(|_| desktop) {
            Some(Ok(grants)) => Some((grants.sentence.clone(), grant_rows(grants, unix_now()))),
            Some(Err(sentence)) => Some((sentence.clone(), Vec::new())),
            None => None,
        };
        w.grants.group.set_visible(shown.is_some());
        if *self.drawn_grants.borrow() == shown {
            return;
        }
        let (sentence, rows) = shown.clone().unwrap_or_default();
        w.grants
            .group
            .set_description(Some(&glib::markup_escape_text(&sentence)));
        w.grants
            .replace(rows.iter().map(|row| self.grant_row(row)).collect());
        *self.drawn_grants.borrow_mut() = shown;
    }

    fn grant_row(&self, grant: &GrantRow) -> gtk::Widget {
        let row = plain_row(&grant.path, &grant.granted);
        row.set_title_selectable(true);
        let remove = gtk::Button::builder()
            .label(words::REMOVE)
            .valign(gtk::Align::Center)
            .build();
        remove.update_property(&[gtk::accessible::Property::Description(&grant.path)]);
        let (pane, path) = (self.me.clone(), grant.path.clone());
        remove.connect_clicked(move |button| {
            if let Some(pane) = pane.upgrade() {
                button.set_sensitive(false);
                pane.remove_grant(path.clone());
            }
        });
        row.add_suffix(&remove);
        row.upcast()
    }

    /// The steps before a move: this computer's on a primary, the server's when a desktop
    /// replaces it.
    fn draw_setup(&self, setup: Option<&Setup>, before: Option<&Setup>) {
        let w = &self.w;
        w.choose.set_visible(setup.is_some());
        let notice = setup.and_then(|s| s.notice.as_deref());
        w.notice.set_visible(notice.is_some());
        w.notice
            .set_description(notice.map(glib::markup_escape_text).as_deref());
        let chosen = setup.and_then(|s| s.destination);
        set_choice(&w.aws, chosen == Some(Destination::Aws));
        set_choice(&w.host, chosen == Some(Destination::ExistingHost));
        self.draw_code(setup.filter(|s| s.destination.is_some()));
        let rows = setup.map(|s| &s.candidates);
        if before.map(|s| &s.candidates) != rows || before.is_none() {
            let rows =
                rows.map_or_else(Vec::new, |r| r.iter().map(|c| self.candidate(c)).collect());
            w.candidates.replace(rows);
        }
        w.candidates
            .group
            .set_visible(rows.is_some_and(|r| !r.is_empty()));
        let review = setup.and_then(|s| s.review.as_ref());
        let earlier = before.and_then(|s| s.review.as_ref());
        if earlier.map(|r| (&r.id, &r.host, &r.instance))
            != review.map(|r| (&r.id, &r.host, &r.instance))
            || before.is_none()
        {
            self.draw_review(review);
        }
        if earlier.map(|r| &r.readiness) != review.map(|r| &r.readiness) || before.is_none() {
            self.draw_readiness(review);
        }
        let starting = setup.is_some_and(|s| s.starting);
        w.start.set_title(&match review {
            Some(_) if starting => words::STARTING.to_owned(),
            Some(review) => move_to(&review.name),
            None => String::new(),
        });
        self.can_start.set(review.is_some_and(|r| r.can_start));
        self.check_passphrase();
    }

    fn draw_code(&self, setup: Option<&Setup>) {
        let w = &self.w;
        w.code.set_visible(setup.is_some());
        let Some(setup) = setup else {
            return;
        };
        let aws = setup.destination == Some(Destination::Aws);
        w.code.set_description(Some(if aws {
            words::AWS_STEPS
        } else {
            words::HOST_STEPS
        }));
        w.guide.set_visible(aws);
        w.command.set_visible(!aws);
        w.get_code.set_visible(setup.get_code.is_some());
        w.get_code
            .set_label(setup.get_code.unwrap_or(words::GET_CODE));
        w.code_row.set_visible(setup.code.is_some());
        if let Some(code) = &setup.code {
            w.code_row.set_title(&code.text);
            w.code_row.set_subtitle(&code.expiry);
        }
        let note = match (setup.opening, setup.waiting, setup.code_note) {
            (true, _, _) => Some(words::OPENING),
            (false, true, _) => Some(words::WAITING),
            (false, false, note) => note,
        };
        w.code_note.set_visible(note.is_some());
        w.code_note.set_title(note.unwrap_or_default());
    }

    fn draw_review(&self, review: Option<&Review>) {
        let w = &self.w;
        let host = review.map_or_else(Vec::new, |r| {
            r.host
                .iter()
                .map(|(label, value)| fact(label, value))
                .collect()
        });
        w.review.replace(host);
        w.review.group.set_visible(review.is_some());
        let instance = match review {
            Some(r) if r.instance.is_empty() => vec![note_row(words::NO_INSTANCE).upcast()],
            Some(r) => r.instance.iter().map(|(l, v)| unverified(l, v)).collect(),
            None => Vec::new(),
        };
        w.instance.replace(instance);
        w.instance.group.set_visible(review.is_some());
        w.passphrase.set_visible(review.is_some());
        if review.is_none() {
            self.forget_passphrase();
        }
    }

    /// What the owner reads before the move: each item's sentence as the daemon wrote it, with
    /// words beside it for its status; Move stays off while anything blocks.
    fn draw_readiness(&self, review: Option<&Review>) {
        let w = &self.w;
        w.readiness.group.set_visible(review.is_some());
        let checked = review.map(|r| &r.readiness);
        w.check_again
            .set_visible(checked.is_some_and(|c| *c != Checked::Checking));
        let (description, rows): (Option<&str>, Vec<gtk::Widget>) = match checked {
            None => (None, Vec::new()),
            Some(Checked::Checking) => (None, vec![checking_row().upcast()]),
            Some(Checked::Unread(sentence)) => (None, vec![refused_row(sentence).upcast()]),
            Some(Checked::Answered(report)) => (Some(summary(report)), report_rows(report)),
        };
        w.readiness.group.set_description(description);
        w.readiness.replace(rows);
    }

    fn draw_moving(&self, stage: &Stage) {
        let w = &self.w;
        let moving = match stage {
            Stage::Moving(moving) => Some(moving),
            _ => None,
        };
        w.moving.set_visible(moving.is_some());
        let Some(moving) = moving else {
            return;
        };
        w.moving_row.set_title(&moving.sentence);
        w.moving_row
            .set_subtitle(moving.detail.as_deref().unwrap_or_default());
        w.moving_spinner.set_visible(moving.waiting);
        match offer(moving) {
            Some((label, action)) => {
                w.moving_action.set_label(label);
                w.moving_action.set_action_name(Some(action));
                w.moving_action.set_visible(true);
            }
            None => w.moving_action.set_visible(false),
        }
    }

    /// After the move: the server this computer reaches and the steps that replace it; or, with
    /// no server, one sentence.
    fn draw_desktop(&self, stage: &Stage) {
        let w = &self.w;
        let desktop: Option<&DesktopStage> = match stage {
            Stage::Desktop(desktop) => Some(desktop),
            _ => None,
        };
        w.server.set_visible(desktop.is_some());
        w.replace.set_visible(desktop.is_some());
        if let Some(desktop) = desktop {
            let server = &desktop.server;
            w.server_row.set_title(&server.name);
            w.server_row.set_subtitle(&server.sentence);
            w.server_state.set_label(server.state);
            if server.alarm {
                w.server_row.add_css_class("error");
            } else {
                w.server_row.remove_css_class("error");
            }
            w.moved_row.set_visible(server.moved.is_some());
            w.moved_row
                .set_title(server.moved.as_deref().unwrap_or_default());
            w.replace
                .set_description(Some(&glib::markup_escape_text(&desktop.explain)));
            w.replace_button.set_visible(desktop.can_replace);
            w.replace_button.set_label(if desktop.replace.is_some() {
                words::CANCEL
            } else {
                words::REPLACE
            });
        }
        let alone = match stage {
            Stage::Unlinked => Some(words::UNLINKED),
            Stage::Candidate(sentence) => Some(sentence.as_str()),
            _ => None,
        };
        w.unlinked.set_visible(alone.is_some());
        w.unlinked
            .set_description(alone.map(glib::markup_escape_text).as_deref());
    }

    fn draw_recovery(&self, recovery: Option<&Recovery>) {
        let w = &self.w;
        w.recovery.group.set_visible(recovery.is_some());
        let Some(recovery) = recovery else {
            return w.recovery.replace(Vec::new());
        };
        w.recovery
            .group
            .set_description(Some(&glib::markup_escape_text(&recovery.sentence)));
        w.back_up.set_visible(recovery.can_run);
        w.back_up.set_sensitive(!recovery.backing_up);
        w.back_up.set_label(if recovery.backing_up {
            words::BACKING_UP
        } else {
            words::BACK_UP_NOW
        });
        w.set_passphrase.set_visible(recovery.can_set_passphrase);
        let mut rows: Vec<gtk::Widget> = recovery
            .targets
            .iter()
            .map(|t| {
                let row = plain_row(&t.title, &t.verified);
                if let Some(error) = &t.error {
                    row.set_subtitle(&format!("{}\n{error}", t.verified));
                    row.add_css_class("error");
                } else if let Some(waiting) = &t.waiting {
                    row.set_subtitle(&format!("{}\n{waiting}", t.verified));
                } else if let Some(destination) = &t.destination {
                    row.set_subtitle(&format!("{}\n{destination}", t.verified));
                }
                row.upcast()
            })
            .collect();
        rows.extend(recovery.next.as_deref().map(|n| note_row(n).upcast()));
        if !recovery.secrets.is_empty() {
            rows.push(plain_row(words::SECRETS_TITLE, recovery.secrets_note).upcast());
            rows.extend(recovery.secrets.iter().map(|id| identifier_row(id)));
        }
        w.recovery.replace(rows);
    }

    fn candidate(&self, candidate: &CandidateRow) -> gtk::Widget {
        let row = plain_row(&candidate.title, &candidate.subtitle);
        let review = gtk::Button::builder()
            .label(words::REVIEW)
            .valign(gtk::Align::Center)
            .build();
        let (pane, id) = (self.me.clone(), candidate.id.clone());
        review.connect_clicked(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.review(&id);
            }
        });
        row.add_suffix(&review);
        row.upcast()
    }

    fn desktop_row(&self, desktop: &DesktopRow) -> gtk::Widget {
        let Some(on) = desktop.executor else {
            return plain_row(&desktop.name, desktop.state).upcast();
        };
        let switch = adw::SwitchRow::builder()
            .title(glib::markup_escape_text(&desktop.name))
            .subtitle(desktop.state)
            .active(on)
            .sensitive(!desktop.switching)
            .build();
        let (pane, id) = (self.me.clone(), desktop.device_id.clone());
        switch.connect_active_notify(move |switch| {
            if let Some(pane) = pane.upgrade() {
                pane.set_executor(id.clone(), switch.is_active());
            }
        });
        switch.upcast()
    }
}

// ---- What the owner does here ----

impl AlwaysOnPane {
    fn wire(self: &Rc<Self>) {
        let w = &self.w;
        for (check, destination) in [
            (&w.aws, Destination::Aws),
            (&w.host, Destination::ExistingHost),
        ] {
            let pane = self.me.clone();
            check.connect_toggled(move |check| {
                if let Some(pane) = pane.upgrade().filter(|_| check.is_active()) {
                    pane.choose(destination);
                }
            });
        }
        self.on_click(&w.get_code, |pane| pane.get_code());
        for entry in [&w.first, &w.second] {
            let pane = self.me.clone();
            entry.connect_changed(move |_| {
                if let Some(pane) = pane.upgrade() {
                    pane.w.refusal.set_visible(false);
                    pane.check_passphrase();
                }
            });
        }
        let pane = self.me.clone();
        w.start.connect_activated(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.start_move();
            }
        });
        self.on_click(&w.back_up, |pane| pane.back_up());
        self.on_click(&w.check_again, |pane| pane.check_readiness());
        self.on_click(&w.replace_button, |pane| pane.toggle_replace());
        self.on_click(&w.set_passphrase, |pane| pane.ask_passphrase());
        // Nothing keeps the passphrase: leaving the pane empties both entries. A visit starts
        // each time the pane comes on screen, the down page included, so a Fermix that comes
        // back is noticed.
        let pane = self.me.clone();
        self.root.connect_unmap(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.forget_passphrase();
            }
        });
        let pane = self.me.clone();
        self.root.connect_map(move |_| {
            if let Some(pane) = pane.upgrade() {
                pane.shown();
            }
        });
        self.wire_copies();
    }

    fn wire_copies(self: &Rc<Self>) {
        let w = &self.w;
        let copy_code = copy_button(words::COPY_CODE);
        let pane = self.me.clone();
        copy_code.connect_clicked(move |_| {
            let Some(pane) = pane.upgrade() else { return };
            let code = pane.steps.borrow().code().map(|c| c.text().to_owned());
            if let Some(code) = code {
                pane.copy(&code);
            }
        });
        w.code_row.add_suffix(&copy_code);
        let copy_command = copy_button(words::COPY_COMMAND);
        self.on_click(&copy_command, |pane| pane.copy(words::HOST_COMMAND));
        w.command.add_suffix(&copy_command);
        let open = gtk::Button::builder()
            .label(words::OPEN_GUIDE)
            .valign(gtk::Align::Center)
            .build();
        self.on_click(&open, |pane| pane.open_guide());
        w.guide.add_suffix(&open);
    }

    fn on_click(self: &Rc<Self>, button: &gtk::Button, act: fn(&Rc<AlwaysOnPane>)) {
        let pane = self.me.clone();
        button.connect_clicked(move |_| {
            if let Some(pane) = pane.upgrade() {
                act(&pane);
            }
        });
    }

    fn choose(&self, destination: Destination) {
        let mut steps = self.steps.borrow_mut();
        if steps.destination == Some(destination) {
            return;
        }
        steps.destination = Some(destination);
        drop(steps);
        self.render();
    }

    fn copy(&self, text: &str) {
        self.page.clipboard().set_text(text);
        (self.toast)(words::COPIED);
    }

    fn open_guide(self: &Rc<Self>) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let window = pane.page.root().and_downcast::<gtk::Window>();
            let launcher = gtk::UriLauncher::new(words::GUIDE_URL);
            if let Err(e) = launcher.launch_future(window.as_ref()).await {
                glib::g_warning!("fermix", "the Always On guide did not open: {e}");
                (pane.toast)(words::GUIDE_NOT_OPENED);
            }
        });
    }

    /// Opens an enrollment window. The code comes back once, so it lives only in `Steps`.
    fn get_code(self: &Rc<Self>) {
        self.steps.borrow_mut().opening = true;
        self.render();
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let answer = pane.daemon.call(|m| m.home_enroll_open()).await;
            match answer {
                Ok(code) => pane.steps.borrow_mut().code_opened(code),
                Err(e) => {
                    let mut steps = pane.steps.borrow_mut();
                    steps.opening = false;
                    steps.notice = Some(sentence_for(e, "home.enroll.open"));
                }
            }
            pane.read_status().await;
        });
    }

    /// The two entries agree, or the line under them says they do not; the move starts only
    /// from two that agree, once the readiness report has nothing that blocks.
    fn check_passphrase(&self) {
        let w = &self.w;
        let checked = passphrase(&w.first.text(), &w.second.text());
        w.start
            .set_sensitive(checked.is_ok() && self.can_start.get());
        if let Err(Some(sentence)) = checked {
            w.refusal.set_label(sentence);
            w.refusal.set_visible(true);
        }
    }

    /// Reviews a candidate: the review shows at once, and its readiness report is read.
    fn review(self: &Rc<Self>, candidate_id: &str) {
        self.steps.borrow_mut().review(candidate_id);
        self.render();
        self.read_readiness(candidate_id.to_owned());
    }

    fn check_readiness(self: &Rc<Self>) {
        let reviewing = self.steps.borrow().reviewing().map(str::to_owned);
        if let Some(id) = reviewing {
            self.review(&id);
        }
    }

    fn read_readiness(self: &Rc<Self>, candidate_id: String) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let id = candidate_id.clone();
            let answer = pane.daemon.call(move |m| m.home_move_readiness(&id)).await;
            let answer = answer.map_err(|e| sentence_for(e, "home.move.readiness"));
            pane.steps
                .borrow_mut()
                .readiness_read(&candidate_id, answer);
            pane.render();
        });
    }

    fn toggle_replace(&self) {
        let open = self.steps.borrow().replace_open;
        self.steps.borrow_mut().open_replace(!open);
        self.forget_passphrase();
        self.render();
    }

    fn forget_passphrase(&self) {
        self.w.first.set_text("");
        self.w.second.set_text("");
        self.w.refusal.set_label("");
        self.w.refusal.set_visible(false);
    }

    /// Starts the move to the candidate under review: this computer's home on a primary, which
    /// restarts into it and is followed across the restart (`Steps::started`), or, on a desktop,
    /// the server's, which is followed to the server that takes over (`Steps::replacing`).
    fn start_move(self: &Rc<Self>) {
        let w = &self.w;
        let Ok(typed) = passphrase(&w.first.text(), &w.second.text()) else {
            return self.check_passphrase();
        };
        let Some(candidate) = self.steps.borrow().reviewing().map(str::to_owned) else {
            return;
        };
        // On a desktop the move is the server's, followed by the plan taken now, while the
        // server's picture holds the candidate.
        let (from, plan, name) = match self.read.borrow().as_ref() {
            Some(Ok(status)) => (
                status.move_from(),
                status.replacement_by(&candidate),
                candidate_name(status, &candidate),
            ),
            _ => return,
        };
        if from == MoveFrom::TheServer && plan.is_none() {
            return;
        }
        self.steps.borrow_mut().starting = true;
        self.render();
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let answer = pane
                .daemon
                .call(move |m| m.home_move_start(&candidate, &typed, from))
                .await;
            pane.steps.borrow_mut().starting = false;
            match answer {
                Ok(started) => pane.move_started(plan, &started.candidate.name, &started.sentence),
                Err(e) => match unconfirmed(&e) {
                    // Sent on and not confirmed: no failure; the move is followed with the
                    // daemon's sentence beside it.
                    Some(sentence) => pane.move_unconfirmed(plan, &name, sentence),
                    None => pane.move_refused(e),
                },
            }
            pane.read_status().await;
        });
    }

    fn move_started(&self, plan: Option<Replacing>, name: &str, sentence: &str) {
        let mut steps = self.steps.borrow_mut();
        match plan {
            Some(replacing) => steps.replace_started(replacing),
            None => steps.move_started(name),
        }
        drop(steps);
        self.forget_passphrase();
        (self.toast)(sentence);
    }

    fn move_unconfirmed(&self, plan: Option<Replacing>, name: &str, sentence: &str) {
        self.steps
            .borrow_mut()
            .start_unconfirmed(plan, name, sentence);
        self.forget_passphrase();
    }

    /// Removes one folder the server may read here; the list is read again either way.
    fn remove_grant(self: &Rc<Self>, path: String) {
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let answer = pane
                .daemon
                .call(move |m| m.device_grants_remove(&path))
                .await;
            match answer {
                Ok(removed) => (pane.toast)(&removed.sentence),
                Err(e) => (pane.toast)(&sentence_for(e, "device.grants.remove")),
            }
            // The row's button was turned off; drawing the list anew brings it back.
            pane.drawn_grants.replace(None);
            pane.read_grants().await;
            pane.render();
        });
    }

    /// A refused passphrase is said under the entries; any other refusal above the steps.
    fn move_refused(&self, e: CallError) {
        let about_passphrase = matches!(&e, CallError::Refused(r)
            if r.field.as_deref() == Some("recovery_passphrase"));
        let sentence = sentence_for(e, "home.move.start");
        if about_passphrase {
            self.w.refusal.set_label(&sentence);
            self.w.refusal.set_visible(true);
        } else {
            self.steps.borrow_mut().notice = Some(sentence);
        }
    }

    fn set_executor(self: &Rc<Self>, device_id: String, on: bool) {
        self.steps.borrow_mut().executor = Some((device_id.clone(), on));
        self.render();
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let answer = pane
                .daemon
                .call(move |m| m.mobile_devices_set_executor(&device_id, on))
                .await;
            pane.steps.borrow_mut().executor = None;
            if let Err(e) = answer {
                (pane.toast)(&sentence_for(e, "mobile.devices.set_executor"));
            }
            pane.read_status().await;
        });
    }

    /// Asks for the recovery passphrase twice in a dialog of its own and sets it
    /// (`home.snapshots.passphrase`), so a primary that stays one can turn snapshots on here.
    fn ask_passphrase(self: &Rc<Self>) {
        let page = passphrase_page(self.me.clone());
        form_dialog(&page, SECRET_WIDTH, None).present(Some(&self.page));
    }

    fn back_up(self: &Rc<Self>) {
        self.steps.borrow_mut().backing_up = true;
        self.render();
        let pane = self.clone();
        glib::spawn_future_local(async move {
            let answer = pane.daemon.call(|m| m.home_snapshots_run()).await;
            pane.steps.borrow_mut().backing_up = false;
            if let Err(e) = answer {
                (pane.toast)(&sentence_for(e, "home.snapshots.run"));
            }
            pane.read_status().await;
        });
    }
}

/// The daemon's sentence for a refusal, else what became of the daemon.
fn sentence_for(e: CallError, what: &str) -> String {
    match e {
        CallError::Refused(refusal) => refused_sentence(&refusal),
        other => {
            glib::g_warning!("fermix", "an Always On call ({what}) failed: {other:?}");
            daemon_problem(&other).sentence()
        }
    }
}

/// The name of the candidate `id` in the home whose move the pane offers.
fn candidate_name(status: &HomeStatus, id: &str) -> String {
    status
        .home_of_record()
        .and_then(|home| home.candidates.iter().find(|c| c.candidate_id == id))
        .map_or_else(|| id.to_owned(), |c| c.name.clone())
}

/// The steps on screen: before this computer's first move, or replacing the server.
fn setup_of(stage: &Stage) -> Option<&Setup> {
    match stage {
        Stage::Setup(setup) => Some(setup),
        Stage::Desktop(desktop) => desktop.replace.as_ref(),
        _ => None,
    }
}

/// Once the wait for this computer's engine after a move's restart is stalled, what the down
/// page would offer for what became of it: start it, or restart it.
fn offer(moving: &Moving) -> Option<(&'static str, &'static str)> {
    let problem = moving.problem.as_ref().filter(|_| moving.stalled)?;
    down_view(problem, false, true, "").button()
}

fn summary(report: &Readiness) -> &'static str {
    if report.ready {
        words::READY
    } else {
        words::NOT_READY
    }
}

fn unix_now() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past 1970");
    i64::try_from(since.as_secs()).expect("seconds fit")
}

/// Sets a choice without sending its toggle on again when it already reads so.
fn set_choice(check: &gtk::CheckButton, on: bool) {
    if check.is_active() != on {
        check.set_active(on);
    }
}

// ---- The widgets ----

fn widgets() -> Widgets {
    let (choose, aws, host) = choose_group();
    let (code, get_code, guide, command, code_row, code_note) = code_group();
    let (passphrase, first, second, start, refusal) = passphrase_group();
    let (server, server_row, server_state, moved_row) = server_group();
    let (moving, moving_row, moving_spinner, moving_action) = moving_group();
    let (recovery, back_up, set_passphrase) = recovery_group();
    let (readiness, check_again) = readiness_group();
    let (replace, replace_button) = replace_group();
    Widgets {
        notice: hidden_group(None),
        explain: explain_group(),
        choose,
        aws,
        host,
        code,
        get_code,
        guide,
        command,
        code_row,
        code_note,
        candidates: List::new(titled(words::CANDIDATES_TITLE, None)),
        review: List::new(titled(words::REVIEW_TITLE, None)),
        instance: List::new(titled(words::INSTANCE_TITLE, Some(words::INSTANCE_NOTE))),
        readiness: List::new(readiness),
        check_again,
        passphrase,
        first,
        second,
        start,
        refusal,
        moving,
        moving_row,
        moving_spinner,
        moving_action,
        server,
        server_row,
        server_state,
        moved_row,
        unlinked: hidden_group(None),
        replace,
        replace_button,
        grants: List::new(titled(words::GRANTS_TITLE, None)),
        desktops: List::new(titled(words::DESKTOPS_TITLE, Some(words::DESKTOPS_NOTE))),
        recovery: List::new(recovery),
        back_up,
        set_passphrase,
        stopped: hidden_group(Some(words::STOPPED_CHECKING)),
    }
}

/// A group hidden until the view needs it.
fn titled(title: &str, description: Option<&str>) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder()
        .title(title)
        .description(description.unwrap_or_default())
        .visible(false)
        .build()
}

fn hidden_group(description: Option<&str>) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder()
        .description(description.unwrap_or_default())
        .visible(false)
        .build()
}

fn explain_group() -> adw::PreferencesGroup {
    let group = titled(words::EXPLAIN_TITLE, Some(words::EXPLAIN));
    group.add(&plain_row(words::MOVES_TITLE, words::WHAT_MOVES));
    group.add(&plain_row(words::STAYS_TITLE, words::WHAT_STAYS));
    group
}

fn choose_group() -> (adw::PreferencesGroup, gtk::CheckButton, gtk::CheckButton) {
    let group = titled(words::CHOOSE_TITLE, None);
    let aws = gtk::CheckButton::new();
    let host = gtk::CheckButton::new();
    host.set_group(Some(&aws));
    for (check, title, detail) in [
        (&aws, words::AWS_TITLE, words::AWS_DETAIL),
        (&host, words::HOST_TITLE, words::HOST_DETAIL),
    ] {
        let row = plain_row(title, detail);
        check.update_property(&[gtk::accessible::Property::Label(title)]);
        row.add_prefix(check);
        row.set_activatable_widget(Some(check));
        group.add(&row);
    }
    (group, aws, host)
}

type CodeWidgets = (
    adw::PreferencesGroup,
    gtk::Button,
    adw::ActionRow,
    adw::ActionRow,
    adw::ActionRow,
    adw::ActionRow,
);

fn code_group() -> CodeWidgets {
    let group = titled(words::CODE_TITLE, None);
    let get_code = gtk::Button::builder()
        .label(words::GET_CODE)
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    group.set_header_suffix(Some(&get_code));
    let guide = plain_row(words::GUIDE_TITLE, words::GUIDE_URL);
    let command = plain_row(words::COMMAND_TITLE, words::HOST_COMMAND);
    command.add_css_class("monospace");
    command.set_subtitle_selectable(true);
    let code_row = adw::ActionRow::builder()
        .use_markup(false)
        .title_selectable(true)
        .css_classes(["monospace"])
        .visible(false)
        .build();
    let code_note = adw::ActionRow::builder()
        .use_markup(false)
        .title_lines(0)
        .visible(false)
        .build();
    for row in [&guide, &command, &code_row, &code_note] {
        group.add(row);
    }
    (group, get_code, guide, command, code_row, code_note)
}

type PassphraseWidgets = (
    adw::PreferencesGroup,
    adw::PasswordEntryRow,
    adw::PasswordEntryRow,
    adw::ButtonRow,
    gtk::Label,
);

fn passphrase_group() -> PassphraseWidgets {
    let group = titled(words::PASSPHRASE_TITLE, Some(words::PASSPHRASE_NOTE));
    let (first, second, refusal) = passphrase_entries();
    let start = adw::ButtonRow::builder()
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    group.add(&first);
    group.add(&second);
    group.add(&start);
    group.add(&refusal);
    (group, first, second, start, refusal)
}

/// The recovery passphrase typed twice, and the line a mismatch or a refusal is said on.
fn passphrase_entries() -> (adw::PasswordEntryRow, adw::PasswordEntryRow, gtk::Label) {
    let first = adw::PasswordEntryRow::builder()
        .title(words::PASSPHRASE)
        .build();
    let second = adw::PasswordEntryRow::builder()
        .title(words::PASSPHRASE_AGAIN)
        .build();
    let refusal = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .margin_top(6)
        .css_classes(["error"])
        .visible(false)
        .build();
    (first, second, refusal)
}

fn moving_group() -> (
    adw::PreferencesGroup,
    adw::ActionRow,
    adw::Spinner,
    gtk::Button,
) {
    let group = titled(words::MOVING_TITLE, None);
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title_lines(0)
        .subtitle_lines(0)
        .build();
    let spinner = adw::Spinner::new();
    row.add_prefix(&spinner);
    let action = gtk::Button::builder()
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .visible(false)
        .build();
    row.add_suffix(&action);
    group.add(&row);
    (group, row, spinner, action)
}

fn readiness_group() -> (adw::PreferencesGroup, gtk::Button) {
    let group = titled(words::READINESS_TITLE, None);
    let again = gtk::Button::builder()
        .label(words::CHECK_AGAIN)
        .valign(gtk::Align::Center)
        .build();
    group.set_header_suffix(Some(&again));
    (group, again)
}

fn replace_group() -> (adw::PreferencesGroup, gtk::Button) {
    let group = titled(words::REPLACE_TITLE, None);
    let open = gtk::Button::builder()
        .label(words::REPLACE)
        .valign(gtk::Align::Center)
        .build();
    group.set_header_suffix(Some(&open));
    group.add(&note_row(words::MOVE_BACK));
    (group, open)
}

fn server_group() -> (
    adw::PreferencesGroup,
    adw::ActionRow,
    gtk::Label,
    adw::ActionRow,
) {
    let group = titled(words::SERVER_TITLE, None);
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .subtitle_lines(0)
        .build();
    let state = gtk::Label::builder()
        .valign(gtk::Align::Center)
        .css_classes(["dimmed"])
        .build();
    row.add_suffix(&state);
    let moved = adw::ActionRow::builder()
        .use_markup(false)
        .css_classes(["dimmed"])
        .build();
    group.add(&row);
    group.add(&moved);
    (group, row, state, moved)
}

fn recovery_group() -> (adw::PreferencesGroup, gtk::Button, gtk::Button) {
    let group = titled(words::RECOVERY_TITLE, None);
    let set_passphrase = gtk::Button::builder()
        .label(words::SET_PASSPHRASE)
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    let back_up = gtk::Button::builder()
        .label(words::BACK_UP_NOW)
        .valign(gtk::Align::Center)
        .build();
    let buttons = gtk::Box::builder().spacing(6).build();
    buttons.append(&set_passphrase);
    buttons.append(&back_up);
    group.set_header_suffix(Some(&buttons));
    (group, back_up, set_passphrase)
}

/// A Copy button named for what it copies.
fn copy_button(name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(words::COPY)
        .tooltip_text(name)
        .valign(gtk::Align::Center)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

/// A row of plain words, which may be the daemon's: never read as markup.
fn plain_row(title: &str, subtitle: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .use_markup(false)
        .title(title)
        .subtitle(subtitle)
        .subtitle_lines(0)
        .build()
}

fn note_row(text: &str) -> adw::ActionRow {
    adw::ActionRow::builder()
        .use_markup(false)
        .title(text)
        .title_lines(0)
        .css_classes(["dimmed"])
        .build()
}

/// A secret's id, as `secret.set` and Settings name it.
fn identifier_row(id: &str) -> gtk::Widget {
    let row = adw::ActionRow::builder()
        .use_markup(false)
        .title(id)
        .title_selectable(true)
        .css_classes(["monospace"])
        .build();
    row.upcast()
}

fn checking_row() -> adw::ActionRow {
    let row = note_row(words::CHECKING);
    row.add_prefix(&adw::Spinner::new());
    row
}

/// Why the readiness report could not be read, in the daemon's words.
fn refused_row(sentence: &str) -> adw::ActionRow {
    let row = note_row(sentence);
    row.add_css_class("error");
    row
}

/// One row per item: its sentence as the daemon wrote it, an icon, and the words for its status.
fn report_rows(report: &Readiness) -> Vec<gtk::Widget> {
    report
        .items
        .iter()
        .map(|item| {
            let row = adw::ActionRow::builder()
                .use_markup(false)
                .title(&item.sentence)
                .title_lines(0)
                .build();
            let (icon, style) = match item.status {
                ItemStatus::Ok => ("emblem-ok-symbolic", "dimmed"),
                ItemStatus::Warn => ("dialog-warning-symbolic", "warning"),
                ItemStatus::Block => ("dialog-error-symbolic", "error"),
                ItemStatus::Unknown => ("dialog-information-symbolic", "dimmed"),
            };
            row.add_prefix(&gtk::Image::from_icon_name(icon));
            if let Some(mark) = item.status.mark() {
                let label = gtk::Label::builder()
                    .label(mark)
                    .valign(gtk::Align::Center)
                    .css_classes(["caption", style])
                    .build();
                row.add_suffix(&label);
            }
            row.upcast()
        })
        .collect()
}

/// The recovery passphrase, twice, in a page of its own: Cancel and Set passphrase over the
/// entries, what the passphrase is for, and the line a mismatch or a refusal is said on. The
/// entries are emptied whenever the page is hidden; nothing keeps the passphrase.
fn passphrase_page(pane: Weak<AlwaysOnPane>) -> adw::NavigationPage {
    let (first, second, refusal) = passphrase_entries();
    let group = adw::PreferencesGroup::builder()
        .description(words::PASSPHRASE_NOTE)
        .build();
    group.add(&first);
    group.add(&second);
    group.add(&refusal);
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(12)
        .margin_end(12)
        .build();
    content.append(&group);
    let cancel = gtk::Button::with_label(words::CANCEL);
    let set = gtk::Button::builder()
        .label(words::SET)
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    let header = adw::HeaderBar::builder()
        .show_start_title_buttons(false)
        .show_end_title_buttons(false)
        .show_back_button(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&set);
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));
    let page = adw::NavigationPage::builder()
        .title(words::PASSPHRASE_TITLE)
        .child(&toolbar)
        .build();
    let form = Rc::new(PassphraseForm {
        first,
        second,
        refusal,
        cancel,
        set,
    });
    form.wire(&page, pane);
    keep_with(&page, form);
    page
}

struct PassphraseForm {
    first: adw::PasswordEntryRow,
    second: adw::PasswordEntryRow,
    refusal: gtk::Label,
    cancel: gtk::Button,
    set: gtk::Button,
}

impl PassphraseForm {
    fn wire(self: &Rc<Self>, page: &adw::NavigationPage, pane: Weak<AlwaysOnPane>) {
        for entry in [&self.first, &self.second] {
            let form = Rc::downgrade(self);
            entry.connect_changed(move |_| {
                if let Some(form) = form.upgrade() {
                    form.check();
                }
            });
        }
        self.cancel.connect_clicked(leave);
        let form = Rc::downgrade(self);
        page.connect_hidden(move |_| {
            if let Some(form) = form.upgrade() {
                form.first.set_text("");
                form.second.set_text("");
            }
        });
        // The first entry takes the focus as the page comes on screen, on the next idle turn.
        self.first.connect_map(|entry| {
            let entry = entry.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(entry) = entry.upgrade() {
                    entry.grab_focus();
                }
            });
        });
        let (form, from) = (Rc::downgrade(self), pane.clone());
        self.set.connect_clicked(move |_| {
            if let (Some(form), Some(pane)) = (form.upgrade(), from.upgrade()) {
                form.send(pane);
            }
        });
        // Enter in the second entry goes through the same check as the button.
        let form = Rc::downgrade(self);
        self.second.connect_entry_activated(move |_| {
            let Some(form) = form.upgrade().filter(|f| f.set.is_sensitive()) else {
                return;
            };
            if let Some(pane) = pane.upgrade() {
                form.send(pane);
            }
        });
    }

    /// The two entries agree, or the line under them says they do not.
    fn check(&self) {
        let checked = passphrase(&self.first.text(), &self.second.text());
        self.set.set_sensitive(checked.is_ok());
        match checked {
            Err(Some(sentence)) => {
                self.refusal.set_label(sentence);
                self.refusal.set_visible(true);
            }
            _ => self.refusal.set_visible(false),
        }
    }

    /// Sends the passphrase once: the page stays, with the daemon's sentence, on a refusal, and
    /// leaves once it is set, saying what the daemon said of it.
    fn send(self: Rc<Self>, pane: Rc<AlwaysOnPane>) {
        let Ok(typed) = passphrase(&self.first.text(), &self.second.text()) else {
            return self.check();
        };
        for widget in [&self.set, &self.cancel] {
            widget.set_sensitive(false);
        }
        glib::spawn_future_local(async move {
            let answer = pane
                .daemon
                .call(move |m| m.home_snapshots_passphrase(&typed))
                .await;
            self.cancel.set_sensitive(true);
            match answer {
                Ok(set) => {
                    (pane.toast)(&set.sentence);
                    leave(&self.set);
                    pane.read_status().await;
                }
                Err(e) => {
                    self.refusal
                        .set_label(&sentence_for(e, "home.snapshots.passphrase"));
                    self.refusal.set_visible(true);
                    self.set.set_sensitive(true);
                }
            }
        });
    }
}

fn fact(label: &str, value: &str) -> gtk::Widget {
    let row = plain_row(label, value);
    row.add_css_class("property");
    row.upcast()
}

/// A fact the server reported about itself, which nobody checked, and says so in words.
fn unverified(label: &str, value: &str) -> gtk::Widget {
    let row = plain_row(label, value);
    row.add_css_class("property");
    row.set_subtitle_selectable(true);
    let mark = gtk::Label::builder()
        .label(words::UNVERIFIED)
        .valign(gtk::Align::Center)
        .css_classes(["caption", "warning"])
        .build();
    row.add_suffix(&mark);
    row.upcast()
}
