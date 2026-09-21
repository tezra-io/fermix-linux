//! Connect your AI.
//!
//! One row per provider the daemon published, each leading with the one verb
//! the daemon will actually answer. The rule is `ProvidersModel`'s and the row
//! is drawn with the Providers pane's own helpers, so a detection that changes
//! a verb changes it in both places at once.
//!
//! Connecting a provider is the one required decision of the assistant, so the
//! screen has two shapes: the rows, or the connected state with the way back to
//! them. Nothing here decides that a provider is connected: the daemon's own
//! readiness does.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::models::onboarding::{Block, OnboardingModel, Snapshot};
use crate::models::providers::{
    import_source, probe_sentence, ProviderRow, ProviderVerb, ProvidersModel,
};
use crate::models::SettingsModel;
use crate::ui::settings::dialogs::secret::SecretDialog;
use crate::ui::settings::dialogs::sign_in::SignInDialog;
use crate::ui::settings::providers::{standing_line, verb_button, write_verb};
use crate::ui::widgets::mark::{self, MarkKind};
use crate::ui::CaptionRow;

use super::Screen;
use crate::ui::plain;

/// The Connect your AI screen.
pub struct ConnectAiScreen {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    model: Rc<OnboardingModel>,
    providers: Rc<ProvidersModel>,
    group: adw::PreferencesGroup,
    connected: adw::PreferencesGroup,
    connected_row: adw::ActionRow,
    /// Why the assistant would not go on, where it would not.
    blocked: CaptionRow,
    /// What the one metered check answered.
    probe: CaptionRow,
    rows: RefCell<Vec<(String, adw::ActionRow, gtk::Button)>>,
    shape: RefCell<Vec<String>>,
    /// Set while a person has asked to choose a different provider, which puts
    /// the rows back over a home the daemon already calls connected.
    changing: Cell<bool>,
}

impl ConnectAiScreen {
    /// Build it over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, model: &Rc<OnboardingModel>) -> Rc<Self> {
        let providers = ProvidersModel::new(Rc::clone(&settings));
        let column = super::screen_column();

        column.append(&crate::ui::caption(&copy::text(Key::SetupConnectAiBody)));

        let group = adw::PreferencesGroup::new();
        let (connected, connected_row, change) = connected_group();
        let blocked = CaptionRow::new();
        let probe = CaptionRow::new();

        column.append(&group);
        column.append(&connected);
        column.append(&crate::ui::caption_group(&blocked));
        column.append(&crate::ui::caption_group(&probe));

        let screen = Rc::new(Self {
            root: super::screen(&column),
            settings,
            model: Rc::clone(model),
            providers,
            group,
            connected,
            connected_row,
            blocked,
            probe,
            rows: RefCell::new(Vec::new()),
            shape: RefCell::new(Vec::new()),
            changing: Cell::new(false),
        });

        screen.connect(&change);
        screen
    }

    fn connect(self: &Rc<Self>, change: &adw::ButtonRow) {
        let screen = Rc::downgrade(self);
        self.providers.observe(move || {
            if let Some(screen) = screen.upgrade() {
                let snapshot = screen.model.snapshot();
                Rc::clone(&screen).draw(&snapshot);
            }
        });

        let screen = Rc::clone(self);
        change.connect_activated(move |_| {
            screen.changing.set(true);
            let snapshot = screen.model.snapshot();
            Rc::clone(&screen).draw(&snapshot);
        });
    }

    fn draw_rows(self: &Rc<Self>) {
        let rows = self.providers.rows();
        let shape: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();

        if *self.shape.borrow() != shape {
            self.rebuild(&rows);
            self.shape.replace(shape);
            return;
        }

        for row in &rows {
            self.update(row);
        }
    }

    fn rebuild(self: &Rc<Self>, rows: &[ProviderRow]) {
        for (_, row, _) in self.rows.borrow_mut().drain(..) {
            self.group.remove(&row);
        }

        let mut drawn = Vec::with_capacity(rows.len());
        for row in rows {
            let (widget, verb) = self.build_row(row);
            self.group.add(&widget);
            drawn.push((row.id.clone(), widget, verb));
        }
        self.rows.replace(drawn);
    }

    fn build_row(self: &Rc<Self>, row: &ProviderRow) -> (adw::ActionRow, gtk::Button) {
        let widget = plain(
            adw::ActionRow::builder()
                .title(row.label.as_str())
                .subtitle(standing_line(row))
                .activatable(false)
                .build(),
        );
        widget.add_prefix(&mark::slot(MarkKind::Provider, &row.id));

        let verb = verb_button();
        write_verb(&verb, row);
        widget.add_suffix(&verb);

        let screen = Rc::clone(self);
        let id = row.id.clone();
        verb.connect_clicked(move |button| screen.perform(&id, button));

        (widget, verb)
    }

    fn update(&self, row: &ProviderRow) {
        let held = self.rows.borrow();
        let Some((_, widget, verb)) = held.iter().find(|(id, _, _)| id == &row.id) else {
            return;
        };
        widget.set_subtitle(&standing_line(row));
        write_verb(verb, row);
    }

    /// The provider the daemon reports in use, drawn where the rows are not.
    fn draw_connected(&self) {
        let Some(primary) = self
            .providers
            .rows()
            .into_iter()
            .find(|row| row.primary || row.configured)
        else {
            return;
        };

        self.connected_row.set_title(&primary.label);
        self.connected_row.set_subtitle(&standing_line(&primary));
    }

    /// The row's one verb, whichever door it is.
    fn perform(self: &Rc<Self>, id: &str, anchor: &gtk::Button) {
        let Some(row) = self.providers.row(id) else {
            return;
        };
        let Some(verb) = row.verb else {
            return;
        };

        match verb {
            ProviderVerb::AddKey | ProviderVerb::AddSetupToken => self.add_key(&row, anchor),
            ProviderVerb::SignIn => self.sign_in(id, anchor),
            ProviderVerb::ImportClaudeCode | ProviderVerb::ImportCodexCli => {
                self.import(id, anchor)
            }
        }
    }

    /// A key is typed in the one dialog that owns a secret entry, and then it is
    /// checked: a key that was stored and never tried is a setup that looks
    /// finished and is not.
    fn add_key(self: &Rc<Self>, row: &ProviderRow, anchor: &gtk::Button) {
        let Some(secret) = row.secret_id.clone() else {
            return;
        };

        let screen = Rc::clone(self);
        let id = row.id.clone();
        SecretDialog::present_then(
            Rc::clone(&self.settings),
            &row.section,
            &secret,
            row.present_key,
            anchor,
            move || screen.verify(&id),
        );
    }

    /// One metered call, as a job, whose answer is the daemon's own.
    fn verify(self: &Rc<Self>, id: &str) {
        let providers = Rc::clone(&self.providers);
        let screen = Rc::clone(self);
        let id = id.to_string();

        super::run(async move {
            match providers.probe(&id).await {
                Ok(_) => screen.follow_probe(),
                Err(sentence) => screen.probe.set(Some(&sentence.text)),
            }
        });
    }

    /// Follow the probe until it stops, and say what it answered.
    fn follow_probe(self: &Rc<Self>) {
        let screen = Rc::downgrade(self);
        self.providers.probe_job().observe(move || {
            let Some(screen) = screen.upgrade() else {
                return;
            };
            let runner = screen.providers.probe_job();
            let Some(job) = runner.job() else {
                return;
            };

            screen.probe.set(probe_sentence(&job).as_deref());
            if runner.is_terminal() {
                screen.providers.reload();
            }
        });
    }

    fn sign_in(self: &Rc<Self>, id: &str, anchor: &gtk::Button) {
        let providers = Rc::clone(&self.providers);
        let anchor = anchor.clone();
        let id = id.to_string();

        super::run(async move {
            if let Ok(started) = providers.start_sign_in(&id).await {
                SignInDialog::present(providers, started, &anchor);
            }
        });
    }

    fn import(self: &Rc<Self>, id: &str, anchor: &gtk::Button) {
        let Some(source) = import_source(id) else {
            return;
        };
        let providers = Rc::clone(&self.providers);
        let anchor = anchor.clone();
        let id = id.to_string();

        super::run(async move {
            if let Ok(started) = providers.import_sign_in(&id, source).await {
                SignInDialog::present(providers, started, &anchor);
            }
        });
    }
}

impl Screen for ConnectAiScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        // The screen is one of two shapes, and the daemon's own readiness is
        // what decides which.
        let rows = self.changing.get() || snapshot.provider_gap;
        self.group.set_visible(rows);
        self.connected.set_visible(!rows);

        if rows {
            self.draw_rows();
        } else {
            self.draw_connected();
        }

        self.blocked.set(
            match snapshot.block {
                Some(Block::Provider) => Some(copy::text(Block::Provider.key())),
                _ => None,
            }
            .as_deref(),
        );
    }

    fn shown(self: Rc<Self>) {
        let providers = Rc::clone(&self.providers);
        super::run(async move {
            providers.refresh().await;
        });
    }
}

/// The connected state: one row for the provider in use, and one row that is
/// the button back to the list.
fn connected_group() -> (adw::PreferencesGroup, adw::ActionRow, adw::ButtonRow) {
    let group = adw::PreferencesGroup::new();
    let row = plain(adw::ActionRow::builder().activatable(false).build());
    let change = plain(
        adw::ButtonRow::builder()
            .title(copy::text(Key::SetupChangeProvider))
            .build(),
    );

    group.add(&row);
    group.add(&change);

    (group, row, change)
}
