//! The Meetings pane.
//!
//! A switch at the head, the notetaker's install behind it, then the shared
//! settings and one section per platform. Turning it on for the first time
//! installs the notetaker and its browser, and the switch is written only once
//! that install has landed.
//!
//! Sign-in is state rather than a permanent button. The daemon's probe answers
//! it, and a missing answer is its own row: an absent notetaker, a null
//! `signed_in` and a refused probe all render "not answered" with the action
//! unavailable, because none of them means signed out.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{SettingsPane, SettingsRow};
use crate::models::jobs::{phase_word, JobRunner};
use crate::models::meetings::{MeetingsModel, SignInState, ENABLED_KEY, SECTION};
use crate::models::{spawn, Change, SettingsModel};
use crate::ui::plain;
use crate::ui::settings::descriptor_form::DescriptorForm;
use crate::ui::widgets::mark::{self, MarkKind};
use crate::ui::CaptionRow;

/// The Meetings pane.
pub struct MeetingsPane {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    model: Rc<MeetingsModel>,
    enable: adw::SwitchRow,
    progress: adw::ActionRow,
    phase: gtk::Label,
    cancel: gtk::Button,
    notice: CaptionRow,
    sign_in: adw::ActionRow,
    sign_in_button: gtk::Button,
    /// Set while the switch is being written from the daemon's own answer.
    updating: Rc<Cell<bool>>,
    /// Whether the install chain is being followed, so it is followed once.
    watching: Rc<Cell<bool>>,
    /// The forms this pane draws the daemon's own rows through.
    forms: RefCell<Vec<Rc<DescriptorForm>>>,
}

impl MeetingsPane {
    /// Build the pane over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let model = MeetingsModel::new(Rc::clone(&settings));

        let enable = plain(adw::SwitchRow::builder().build());
        let head = adw::PreferencesGroup::new();
        head.add(&enable);

        // Under the switch rather than over it: it is what happens when the
        // switch is turned on, and a person reads it after deciding to.
        let footer = CaptionRow::new();
        footer.set(Some(&copy::text(Key::MeetingsEnableFooter)));
        head.add(footer.row());

        let phase = crate::ui::value_label("");
        let cancel = gtk::Button::builder()
            .label(copy::text(Key::ActionCancel))
            .valign(gtk::Align::Center)
            .build();
        let progress = plain(
            adw::ActionRow::builder()
                .title(copy::text(Key::MeetingsInstall))
                .activatable(false)
                .visible(false)
                .build(),
        );
        progress.add_suffix(&phase);
        progress.add_suffix(&cancel);
        head.add(&progress);

        let notice = CaptionRow::new();
        head.add(notice.row());

        // The daemon publishes one section for this pane and the design shows
        // it in three groups, so the rows are split between two forms over the
        // same section. Every row lands in one of them.
        let shared = DescriptorForm::restricted(
            Rc::clone(&settings),
            SettingsPane::Meetings,
            vec![SECTION.to_string()],
            Some(Key::MeetingsSharedSettings),
        );
        shared.set_filter(is_shared);

        let google = adw::PreferencesGroup::builder()
            .title(copy::text(Key::MeetingsGoogleMeet))
            .header_suffix(&mark::slot(MarkKind::MeetingPlatform, GOOGLE_MEET))
            .build();

        let sign_in_button = gtk::Button::builder()
            .label(copy::text(Key::MeetingsSignIn))
            .valign(gtk::Align::Center)
            .build();
        let sign_in = plain(
            adw::ActionRow::builder()
                .title(copy::text(Key::MeetingsGoogleAccount))
                .activatable(false)
                .build(),
        );
        sign_in.add_suffix(&sign_in_button);
        google.add(&sign_in);

        let zoom = DescriptorForm::restricted(
            Rc::clone(&settings),
            SettingsPane::Meetings,
            vec![SECTION.to_string()],
            Some(Key::MeetingsZoom),
        );
        zoom.set_filter(is_zoom);
        zoom.set_header_suffix(mark::slot(MarkKind::MeetingPlatform, ZOOM));

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&head);
        column.append(&shared.widget());
        column.append(&google);
        column.append(&zoom.widget());
        // The platform statement, in the shape Voice and Computer state theirs:
        // body copy in a box of the toolkit's rather than a dim caption.
        let sleep = adw::PreferencesGroup::new();
        sleep.add(
            &crate::ui::folded_statement_row(
                &copy::text(Key::MeetingsSleepLead),
                &copy::text(Key::MeetingsSleepStatement),
            )
            .row,
        );
        column.append(&sleep);

        let pane = Rc::new(Self {
            root: crate::ui::scrolled(&crate::ui::clamp(&column)).upcast(),
            settings,
            model,
            enable,
            progress,
            phase,
            cancel,
            notice,
            sign_in,
            sign_in_button,
            updating: Rc::new(Cell::new(false)),
            watching: Rc::new(Cell::new(false)),
            forms: RefCell::new(vec![shared, zoom]),
        });

        pane.connect();
        pane.draw();
        pane
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// Read everything this pane draws.
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
                if matches!(change, Change::Section(section) if section == SECTION)
                    || matches!(change, Change::Detections)
                {
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
            self.sign_in_button
                .connect_clicked(move |_| pane.start_sign_in());
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
        {
            let runner = self.model.sign_in_job();
            let pane = Rc::downgrade(self);
            let watched = Rc::clone(&runner);
            runner.observe(move || {
                let Some(pane) = pane.upgrade() else {
                    return;
                };
                pane.draw_progress(&watched);
                if watched.is_terminal() {
                    let model = Rc::clone(&pane.model);
                    spawn(async move {
                        // Job success is never taken for an account: the probe
                        // is what answers, and it is taken on every outcome.
                        model.sign_in_finished().await;
                    });
                }
            });
        }
    }

    fn draw(self: &Rc<Self>) {
        self.enable.set_title(&self.enable_label());

        self.updating.set(true);
        self.enable.set_active(self.model.enabled());
        self.updating.set(false);

        self.draw_sign_in();
    }

    /// The switch's own label is the daemon's own row label.
    fn enable_label(&self) -> String {
        self.settings
            .state()
            .rows(SECTION)
            .iter()
            .find(|row| row.key == ENABLED_KEY)
            .map(|row| row.label.clone())
            .unwrap_or_else(|| copy::text(Key::PaneMeetings))
    }

    /// Where the notetaker's Google sign-in stands.
    fn draw_sign_in(&self) {
        match self.model.sign_in_state() {
            SignInState::SignedIn(detail) => {
                self.sign_in.set_subtitle(&detail.unwrap_or_default());
                self.sign_in_button
                    .set_label(&copy::text(Key::MeetingsSignInAgain));
                self.sign_in_button.set_sensitive(true);
            }
            SignInState::SignedOut(detail) => {
                self.sign_in.set_subtitle(&detail.unwrap_or_default());
                self.sign_in_button
                    .set_label(&copy::text(Key::MeetingsSignIn));
                self.sign_in_button.set_sensitive(true);
            }
            // Nothing can be said, so nothing is offered either: the action
            // waits on a prerequisite the daemon has not reported.
            SignInState::Unanswered => {
                self.sign_in
                    .set_subtitle(&copy::text(Key::MeetingsSignInUnanswered));
                self.sign_in_button
                    .set_label(&copy::text(Key::MeetingsSignIn));
                self.sign_in_button.set_sensitive(false);
            }
        }
    }

    /// The step a job is on, its progress, and the way to stop it.
    fn draw_progress(&self, runner: &Rc<JobRunner>) {
        let Some(job) = runner.job() else {
            self.progress.set_visible(false);
            return;
        };

        if let Some(failure) = job.failure.as_ref() {
            self.notice.set(Some(&failure.sentence));
        }

        let running = !runner.is_terminal();
        self.progress.set_visible(running);
        self.cancel.set_visible(running);

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

            // An install starts where the notetaker is absent, and the write
            // the switch asked for waits for it.
            let installing = !pane.model.present();
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

    fn start_sign_in(self: &Rc<Self>) {
        let pane = Rc::clone(self);
        spawn(async move {
            if let Err(sentence) = pane.model.start_sign_in().await {
                pane.notice.set(Some(&sentence.text));
            }
        });
    }
}

/// The rows the Shared settings group draws: everything but the enable row,
/// which is the pane's own head, and the platform rows below.
fn is_shared(row: &SettingsRow) -> bool {
    row.key != ENABLED_KEY && !is_zoom(row)
}

/// The rows the Zoom group draws.
///
/// This prefix is the one thing this pane knows about how the section spells
/// its keys, and it is a grouping rather than an inventory: a key the engine
/// renames moves into Shared settings, where it is still written and still
/// visible, rather than disappearing.
fn is_zoom(row: &SettingsRow) -> bool {
    row.key.starts_with(ZOOM_PREFIX)
}

/// The prefix the daemon publishes Zoom's own keys under.
const ZOOM_PREFIX: &str = "meetings_zoom";

/// The two platforms the pane is sectioned by. Their keys are this door's own:
/// the daemon publishes settings and a sign-in job rather than a platform list.
const GOOGLE_MEET: &str = "google_meet";
const ZOOM: &str = "zoom";
