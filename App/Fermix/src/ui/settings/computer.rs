//! The Computer pane.
//!
//! A switch at the head, the helper's install behind it, and the probe's own
//! two verdicts with the time they were taken. Nothing here claims to grant
//! anything, because on this platform there is nothing for this application to
//! grant: the helper asks the desktop for its own session, and the probe is the
//! only thing that knows the answer.
//!
//! Unavailable is a finished sentence with a reason, never a greyed control and
//! never an empty pane. Computer history is absent rather than empty: it is
//! macOS-only in the engine, so this pane renders no row for it at all.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::SettingsPane;
use crate::models::computer::{verdict, ComputerModel, Standing, ENABLED_KEY, SECTION};
use crate::models::jobs::{phase_word, JobRunner};
use crate::models::ledger::{PermissionLedger, Right};
use crate::models::{spawn, Change, SettingsModel};
use crate::ui::CaptionRow;

/// A statement with nothing in it yet: what it says is the standing the probe
/// answered with, which is read after the pane is built.
fn hidden_statement() -> adw::ActionRow {
    let row = crate::ui::statement_row("");
    row.set_visible(false);
    row
}

/// The Computer pane.
pub struct ComputerPane {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    model: Rc<ComputerModel>,
    enable: adw::SwitchRow,
    statements: adw::PreferencesGroup,
    statement: adw::ActionRow,
    session: adw::ActionRow,
    install: adw::ActionRow,
    install_button: gtk::Button,
    phase: gtk::Label,
    cancel: gtk::Button,
    rights: adw::PreferencesGroup,
    capture: adw::ActionRow,
    input: adw::ActionRow,
    refresh: gtk::Button,
    notice: CaptionRow,
    updating: Rc<Cell<bool>>,
    watching: Rc<Cell<bool>>,
    forms: RefCell<Vec<Rc<crate::ui::settings::descriptor_form::DescriptorForm>>>,
}

impl ComputerPane {
    /// Build the pane over the one settings model and the one ledger.
    pub fn new(settings: Rc<SettingsModel>, ledger: Rc<PermissionLedger>) -> Rc<Self> {
        let model = ComputerModel::new(Rc::clone(&settings), ledger);

        let enable = adw::SwitchRow::builder()
            .title(copy::text(Key::ComputerEnable))
            .build();
        let head = adw::PreferencesGroup::new();
        head.add(&enable);

        // Body copy in a row of its own, the way the Voice pane states what the
        // platform owes a person: this sentence is the whole answer on a
        // machine the helper cannot run on, and the smallest, dimmest text on
        // the pane is the wrong place for the one thing there is to read.
        let statement = hidden_statement();
        let session = hidden_statement();
        let statements = adw::PreferencesGroup::new();
        statements.add(&statement);
        statements.add(&session);
        statements.set_visible(false);

        let install_button = gtk::Button::builder()
            .label(copy::text(Key::ComputerInstall))
            .valign(gtk::Align::Center)
            .build();
        let phase = crate::ui::value_label("");
        let cancel = gtk::Button::builder()
            .label(copy::text(Key::ActionCancel))
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let install = adw::ActionRow::builder()
            .title(copy::text(Key::ComputerInstall))
            .activatable(false)
            .visible(false)
            .build();
        install.add_suffix(&phase);
        install.add_suffix(&cancel);
        install.add_suffix(&install_button);
        head.add(&install);

        let notice = CaptionRow::new();
        head.add(notice.row());

        let refresh = gtk::Button::builder()
            .label(copy::text(Key::ActionRefresh))
            .valign(gtk::Align::Center)
            .build();
        let rights = adw::PreferencesGroup::builder()
            .header_suffix(&refresh)
            .visible(false)
            .build();
        let capture = adw::ActionRow::builder()
            .title(copy::text(Key::ComputerScreenCapture))
            .activatable(false)
            .build();
        let input = adw::ActionRow::builder()
            .title(copy::text(Key::ComputerInputControl))
            .activatable(false)
            .build();
        rights.add(&capture);
        rights.add(&input);

        // The section's own rows, which are the daemon's. The enable row is the
        // pane's head, so the form draws whatever else the section carries.
        let form = crate::ui::settings::descriptor_form::DescriptorForm::restricted(
            Rc::clone(&settings),
            SettingsPane::Computer,
            vec![SECTION.to_string()],
            None,
        );
        form.set_filter(|row| row.key != ENABLED_KEY);

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&head);
        column.append(&statements);
        column.append(&rights);
        column.append(&form.widget());

        let pane = Rc::new(Self {
            root: crate::ui::scrolled(&crate::ui::clamp(&column)).upcast(),
            settings,
            model,
            enable,
            statements,
            statement,
            session,
            install,
            install_button,
            phase,
            cancel,
            rights,
            capture,
            input,
            refresh,
            notice,
            updating: Rc::new(Cell::new(false)),
            watching: Rc::new(Cell::new(false)),
            forms: RefCell::new(vec![form]),
        });

        pane.connect();
        pane.draw();
        pane
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// Read everything this pane draws. The probe runs here and on Refresh,
    /// and nowhere else.
    pub fn load(self: &Rc<Self>) {
        for form in self.forms.borrow().iter() {
            form.load();
        }
        let model = Rc::clone(&self.model);
        spawn(async move {
            model.refresh().await;
        });
    }

    fn connect(self: &Rc<Self>) {
        {
            let pane = Rc::downgrade(self);
            self.model.observe(move || {
                if let Some(pane) = pane.upgrade() {
                    pane.draw();
                }
            });
        }
        {
            let pane = Rc::downgrade(self);
            self.settings.observe(move |change| {
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                if matches!(change, Change::Section(section) if section == SECTION) {
                    pane.draw();
                }
            });
        }
        {
            let pane = Rc::clone(self);
            let updating = Rc::clone(&self.updating);
            self.enable.connect_active_notify(move |row| {
                if !updating.get() {
                    pane.switched(row.is_active());
                }
            });
        }
        {
            let pane = Rc::clone(self);
            self.install_button.connect_clicked(move |_| pane.install());
        }
        {
            let pane = Rc::clone(self);
            self.refresh.connect_clicked(move |_| {
                let model = Rc::clone(&pane.model);
                spawn(async move {
                    model.reprobe().await;
                });
            });
        }
        {
            let runner = self.model.install_job();
            let held = Rc::clone(&runner);
            self.cancel.connect_clicked(move |_| {
                let runner = Rc::clone(&held);
                spawn(async move {
                    runner.cancel().await;
                });
            });

            let pane = Rc::downgrade(self);
            let watched = Rc::clone(&runner);
            runner.observe(move || {
                if let Some(pane) = pane.upgrade() {
                    pane.draw_progress(&watched);
                }
            });
        }
    }

    fn draw(self: &Rc<Self>) {
        self.updating.set(true);
        self.enable.set_active(self.model.enabled());
        self.updating.set(false);

        let standing = self.model.standing();

        let statement = standing.statement();
        if let Some(key) = statement {
            self.statement.set_title(&copy::text(key));
        }
        self.statement.set_visible(statement.is_some());

        // The session is named so the sentence about it can be checked.
        let session = match &standing {
            Standing::UnsupportedSession(session) => Some(copy::fill(
                Key::ComputerSessionDetected,
                &[("{session}", session)],
            )),
            _ => None,
        };
        if let Some(sentence) = session.as_deref() {
            self.session.set_title(sentence);
        }
        self.session.set_visible(session.is_some());

        // A preferences group draws its own box, so an empty one is a hairline
        // around nothing. Read from what was just decided rather than from the
        // rows: a widget inside a group that is still hidden does not report
        // itself as visible, whatever its own flag says.
        self.statements
            .set_visible(statement.is_some() || session.is_some());

        self.install.set_visible(standing.offers_install());
        self.enable.set_sensitive(
            !matches!(standing, Standing::Unread) && standing != Standing::NoBuildForArchitecture,
        );

        self.rights
            .set_visible(matches!(standing, Standing::Installed));
        self.draw_rights();

        if let Some(sentence) = self.model.refusal() {
            self.notice.set(Some(&sentence.text));
        }
    }

    /// The two verdicts, and when they were taken. Both are the daemon's.
    fn draw_rights(&self) {
        let ledger = self.model.ledger();
        let probed = ledger
            .probed_at()
            .map(|at| {
                copy::fill(
                    Key::ComputerProbedAt,
                    &[("{time}", &crate::ui::local_moment(&at))],
                )
            })
            .unwrap_or_else(|| copy::text(Key::ComputerProbeUnread));

        for (row, right) in [
            (&self.capture, Right::ScreenCapture),
            (&self.input, Right::InputSynthesis),
        ] {
            let word = copy::text(verdict(ledger.holds(right)));
            row.set_subtitle(&crate::ui::beside(&word, &probed));
        }

        if let Some(sentence) = ledger.refusal() {
            self.notice.set(Some(&sentence.text));
        }
    }

    /// The step the install is on, and the way to stop it.
    fn draw_progress(&self, runner: &Rc<JobRunner>) {
        let Some(job) = runner.job() else {
            return;
        };

        if let Some(failure) = job.failure.as_ref() {
            self.notice.set(Some(&failure.sentence));
        }

        let running = !runner.is_terminal();
        self.cancel.set_visible(running);
        self.install_button.set_sensitive(!running);
        self.install
            .set_visible(running || self.model.standing().offers_install());

        let word = job.phase.as_deref().and_then(phase_word).map(copy::text);
        let counted = job.progress.as_ref().map(|progress| match progress.total {
            Some(total) => format!("{}/{}", progress.done, total),
            None => progress.done.to_string(),
        });

        crate::ui::set_value(
            &self.phase,
            &match (word, counted) {
                (Some(word), Some(counted)) => crate::ui::beside(&word, &counted),
                (Some(word), None) => word,
                (None, Some(counted)) => counted,
                (None, None) => String::new(),
            },
        );
    }

    /// The switch, both ways.
    fn switched(self: &Rc<Self>, on: bool) {
        self.notice.set(None);

        let pane = Rc::clone(self);
        spawn(async move {
            if !on {
                pane.model.disable().await;
                return;
            }

            let installing = !matches!(pane.model.standing(), Standing::Installed);
            match pane.model.enable().await {
                Ok(()) if installing => pane.watch_install(),
                Ok(()) => {}
                Err(sentence) => {
                    pane.notice.set(Some(&sentence.text));
                    pane.updating.set(true);
                    pane.enable.set_active(false);
                    pane.updating.set(false);
                }
            }
        });
    }

    fn install(self: &Rc<Self>) {
        let pane = Rc::clone(self);
        spawn(async move {
            match pane.model.start_install().await {
                Ok(()) => pane.watch_install(),
                Err(sentence) => pane.notice.set(Some(&sentence.text)),
            }
        });
    }

    /// Follow the install to its end, then write what the switch asked for.
    fn watch_install(self: &Rc<Self>) {
        if self.watching.replace(true) {
            return;
        }

        let runner = self.model.install_job();
        let watched = Rc::clone(&runner);
        let pane = Rc::clone(self);
        let finished = Rc::new(Cell::new(false));

        runner.observe(move || {
            if !watched.is_terminal() || finished.replace(true) {
                return;
            }
            pane.watching.set(false);

            let pane = Rc::clone(&pane);
            spawn(async move {
                if let Err(sentence) = pane.model.install_finished().await {
                    pane.notice.set(Some(&sentence.text));
                    pane.updating.set(true);
                    pane.enable.set_active(false);
                    pane.updating.set(false);
                }
                pane.draw();
            });
        });
    }
}
