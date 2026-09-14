//! The Providers pane.
//!
//! A Primary group of the daemon's own routing rows, then one row per provider
//! the daemon published: its mark, its label, where it stands, the one verb it
//! leads with, and the way into everything else about it.
//!
//! Everything belonging to one provider lives on that provider's own page: its
//! sign-in method, its key, its address where it has one, its model, signing
//! out, making it primary and one metered call to check it answers. The rows on
//! the page are the daemon's descriptor for that section, so a key added in the
//! engine needs nothing here.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{SettingValue, SettingsPane};
use crate::models::providers::{
    detail_verbs, import_source, probe_sentence, ProviderRow, ProviderVerb, ProvidersModel,
};
use crate::models::{spawn, Change, SettingsModel};
use crate::ui::settings::descriptor_form::DescriptorForm;
use crate::ui::settings::descriptor_row::OpenChoice;
use crate::ui::settings::dialogs::model_picker::ModelPicker;
use crate::ui::settings::dialogs::sign_in::SignInDialog;
use crate::ui::widgets::mark::{self, MarkKind};
use crate::ui::{beside, CaptionRow};

/// The Providers pane.
pub struct ProvidersPane {
    view: adw::NavigationView,
    settings: Rc<SettingsModel>,
    model: Rc<ProvidersModel>,
    primary: Rc<DescriptorForm>,
    group: adw::PreferencesGroup,
    /// The rows on screen, by provider id, so a refresh updates rather than
    /// rebuilds: rebuilding takes the focus out of whatever a person is on.
    rows: RefCell<BTreeMap<String, Row>>,
    /// The provider ids the group was built from.
    shape: RefCell<Vec<String>>,
    /// One page per provider, built once and pushed again on every visit.
    pages: RefCell<BTreeMap<String, adw::NavigationPage>>,
}

/// One row and the parts of it that change.
struct Row {
    row: adw::ActionRow,
    verb: gtk::Button,
}

impl ProvidersPane {
    /// Build the pane over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let model = ProvidersModel::new(Rc::clone(&settings));

        let primary = DescriptorForm::restricted(
            Rc::clone(&settings),
            SettingsPane::Providers,
            vec![ROUTING.to_string()],
            Some(Key::ProvidersGroupPrimary),
        );

        let group = adw::PreferencesGroup::builder()
            .title(copy::text(Key::ProvidersGroupProviders))
            .build();

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&primary.widget());
        column.append(&group);

        let view = adw::NavigationView::new();
        view.add(
            &adw::NavigationPage::builder()
                .title(copy::text(Key::PaneProviders))
                .child(&crate::ui::scrolled(&crate::ui::clamp(&column)))
                .build(),
        );

        let pane = Rc::new(Self {
            view,
            settings,
            model,
            primary,
            group,
            rows: RefCell::new(BTreeMap::new()),
            shape: RefCell::new(Vec::new()),
            pages: RefCell::new(BTreeMap::new()),
        });

        pane.connect();
        pane.draw();
        pane
    }

    /// The widget the pane stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.view.clone().upcast()
    }

    /// Read everything this pane draws.
    pub fn load(self: &Rc<Self>) {
        self.primary.load();
        let model = Rc::clone(&self.model);
        spawn(async move {
            model.refresh().await;
        });
    }

    /// Go back one page, where a provider's own page is showing.
    pub fn pop(&self) -> bool {
        self.view.pop()
    }

    /// The title of the page showing, where it is not the pane's own.
    pub fn sub_page_title(&self) -> Option<String> {
        let page = self.view.visible_page()?;
        (self.view.navigation_stack().n_items() > 1).then(|| page.title().to_string())
    }

    /// Tell me when the page showing changes.
    pub fn on_page_changed(&self, changed: impl Fn() + 'static) {
        self.view.connect_visible_page_notify(move |_| changed());
    }

    fn connect(self: &Rc<Self>) {
        let pane = Rc::downgrade(self);
        self.model.observe(move || {
            if let Some(pane) = pane.upgrade() {
                pane.draw();
            }
        });

        let pane = Rc::downgrade(self);
        self.settings.observe(move |change| {
            let Some(pane) = pane.upgrade() else {
                return;
            };
            if matches!(change, Change::Setup | Change::Detections) {
                pane.draw();
            }
        });
    }

    /// Draw the provider rows: rebuilt when the set changes, updated otherwise.
    fn draw(self: &Rc<Self>) {
        let rows = self.model.rows();
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
        for held in self.rows.borrow_mut().values() {
            self.group.remove(&held.row);
        }
        self.rows.borrow_mut().clear();

        for row in rows {
            let built = self.build_row(row);
            self.group.add(&built.row);
            self.rows.borrow_mut().insert(row.id.clone(), built);
        }
    }

    fn build_row(self: &Rc<Self>, row: &ProviderRow) -> Row {
        let widget = adw::ActionRow::builder()
            .title(row.label.as_str())
            .subtitle(standing_line(row))
            .activatable(true)
            .build();

        widget.add_prefix(&mark::slot(MarkKind::Provider, &row.id));

        let verb = verb_button();
        widget.add_suffix(&verb);
        widget.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));

        {
            let pane = Rc::clone(self);
            let id = row.id.clone();
            widget.connect_activated(move |_| pane.open(&id));
        }
        {
            let pane = Rc::clone(self);
            let id = row.id.clone();
            verb.connect_clicked(move |button| pane.perform(&id, button));
        }

        let built = Row { row: widget, verb };
        write_row(&built, row);
        built
    }

    fn update(&self, row: &ProviderRow) {
        let held = self.rows.borrow();
        let Some(built) = held.get(&row.id) else {
            return;
        };
        write_row(built, row);
    }

    /// The row's one verb, whichever door it is.
    fn perform(self: &Rc<Self>, id: &str, anchor: &gtk::Button) {
        let Some(row) = self.model.row(id) else {
            return;
        };
        let Some(verb) = row.verb else {
            return;
        };

        match verb {
            ProviderVerb::AddKey | ProviderVerb::AddSetupToken => {
                let Some(secret) = row.secret_id.clone() else {
                    return;
                };
                crate::ui::settings::dialogs::secret::SecretDialog::present(
                    Rc::clone(&self.settings),
                    &row.section,
                    &secret,
                    row.present_key,
                    anchor,
                );
            }
            ProviderVerb::SignIn => self.start_sign_in(id, anchor),
            ProviderVerb::ImportClaudeCode | ProviderVerb::ImportCodexCli => {
                self.start_import(id, anchor)
            }
        }
    }

    fn start_sign_in(self: &Rc<Self>, id: &str, anchor: &impl IsA<gtk::Widget>) {
        let model = Rc::clone(&self.model);
        let anchor = anchor.as_ref().clone();
        let id = id.to_string();
        spawn(async move {
            if let Ok(started) = model.start_sign_in(&id).await {
                SignInDialog::present(model, started, &anchor);
            }
        });
    }

    fn start_import(self: &Rc<Self>, id: &str, anchor: &impl IsA<gtk::Widget>) {
        let Some(source) = import_source(id) else {
            return;
        };
        let model = Rc::clone(&self.model);
        let anchor = anchor.as_ref().clone();
        let id = id.to_string();
        spawn(async move {
            if let Ok(started) = model.import_sign_in(&id, source).await {
                SignInDialog::present(model, started, &anchor);
            }
        });
    }

    /// Open one provider's own page, building it the first time.
    fn open(self: &Rc<Self>, id: &str) {
        let page = self.pages.borrow().get(id).cloned();
        let page = match page {
            Some(page) => page,
            None => {
                let Some(row) = self.model.row(id) else {
                    return;
                };
                let page = self.build_page(&row);
                self.view.add(&page);
                self.pages.borrow_mut().insert(id.to_string(), page.clone());
                page
            }
        };

        self.view.push(&page);

        let model = Rc::clone(&self.model);
        let section = format!("providers.{id}");
        let settings = Rc::clone(&self.settings);
        spawn(async move {
            settings.refresh_section(&section).await;
            model.refresh().await;
        });
    }

    /// One provider's page: the daemon's own rows, then what can be done to it.
    fn build_page(self: &Rc<Self>, row: &ProviderRow) -> adw::NavigationPage {
        let form = DescriptorForm::restricted(
            Rc::clone(&self.settings),
            SettingsPane::Providers,
            vec![row.section.clone()],
            None,
        );

        // The listing the wire does not carry the whole of is this provider's
        // models, and the daemon publishes the method that pages through them.
        let pane = Rc::clone(self);
        let provider = row.id.clone();
        let holder = self.view.clone();
        form.set_picker(OpenChoice {
            label: Key::ProviderChooseModel,
            open: Rc::new(move |section: &str, key: &str, _current: Option<String>| {
                let settings = Rc::clone(&pane.settings);
                let section = section.to_string();
                let key = key.to_string();
                ModelPicker::present(
                    Rc::clone(&pane.model),
                    &provider,
                    &holder,
                    move |chosen: &str| {
                        let settings = Rc::clone(&settings);
                        let section = section.clone();
                        let key = key.clone();
                        let chosen = chosen.to_string();
                        spawn(async move {
                            settings
                                .apply(&section, &key, SettingValue::Text(chosen))
                                .await;
                        });
                    },
                );
            }),
        });

        let column = crate::ui::column();
        column.add_css_class("fermix-gutter");
        column.append(&form.widget());
        column.append(&self.actions(row));
        column.append(&self.probe(row));

        adw::NavigationPage::builder()
            .title(row.label.as_str())
            .child(&crate::ui::scrolled(&crate::ui::clamp(&column)))
            .build()
    }

    /// What can be done to one provider: its sign-in doors, making it primary,
    /// and signing it out.
    fn actions(self: &Rc<Self>, row: &ProviderRow) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        let notice = CaptionRow::new();

        let adoptable = import_source(&row.id)
            .and_then(|target| {
                self.settings
                    .state()
                    .detection(target)
                    .map(|result| result.present)
            })
            .unwrap_or(false);

        for verb in detail_verbs(&row.id, adoptable, None) {
            // One control per row: a row that is the button rather than a row
            // with the same word twice.
            let button = adw::ButtonRow::builder()
                .title(copy::text(verb.key()))
                .build();
            group.add(&button);

            let pane = Rc::clone(self);
            let id = row.id.clone();
            let section = row.section.clone();
            let present = row.present_key;
            let secret = row.secret_id.clone();
            button.connect_activated(move |button| match verb {
                ProviderVerb::SignIn => pane.start_sign_in(&id, button),
                ProviderVerb::ImportClaudeCode | ProviderVerb::ImportCodexCli => {
                    pane.start_import(&id, button)
                }
                ProviderVerb::AddKey | ProviderVerb::AddSetupToken => {
                    let Some(secret) = secret.clone() else {
                        return;
                    };
                    crate::ui::settings::dialogs::secret::SecretDialog::present(
                        Rc::clone(&pane.settings),
                        &section,
                        &secret,
                        present,
                        button,
                    );
                }
            });
        }

        group.add(&self.primary_row(row, notice.clone()));
        group.add(&self.sign_out_row(row, notice.clone()));
        group.add(notice.row());
        group
    }

    /// Making one provider the primary, which the daemon says what it changed
    /// about afterwards.
    fn primary_row(self: &Rc<Self>, row: &ProviderRow, notice: CaptionRow) -> adw::ButtonRow {
        let offered = row.configured && !row.primary;
        let widget = adw::ButtonRow::builder()
            .title(copy::text(Key::ProviderUseAsPrimary))
            .sensitive(offered)
            .build();
        // Suggested only while it can be taken: a filled control that cannot
        // be pressed reads as the one thing to do and is not.
        if offered {
            widget.add_css_class("suggested-action");
        }

        let pane = Rc::clone(self);
        let id = row.id.clone();
        widget.connect_activated(move |button| {
            let dialog = adw::AlertDialog::new(
                Some(&copy::text(Key::ProviderUseAsPrimaryConfirmTitle)),
                Some(&copy::text(Key::ProviderUseAsPrimaryConfirmBody)),
            );
            dialog.add_response(CANCEL, &copy::text(Key::ActionCancel));
            dialog.add_response(CONFIRM, &copy::text(Key::ProviderUseAsPrimary));
            dialog.set_response_appearance(CONFIRM, adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some(CONFIRM));
            dialog.set_close_response(CANCEL);

            let pane = Rc::clone(&pane);
            let id = id.clone();
            let notice = notice.clone();
            dialog.connect_response(None, move |_, response| {
                if response != CONFIRM {
                    return;
                }
                let model = Rc::clone(&pane.model);
                let id = id.clone();
                let notice = notice.clone();
                spawn(async move {
                    // The daemon names what it changed that nobody typed, and
                    // those sentences are its own.
                    match model.set_primary(&id).await {
                        Ok(effects) if !effects.is_empty() => notice.set(Some(&effects.join(" "))),
                        Ok(_) => notice.set(None),
                        Err(sentence) => notice.set(Some(&sentence.text)),
                    }
                });
            });

            dialog.present(Some(button));
        });

        widget
    }

    fn sign_out_row(self: &Rc<Self>, row: &ProviderRow, notice: CaptionRow) -> adw::ButtonRow {
        let widget = adw::ButtonRow::builder()
            .title(copy::text(Key::ProviderSignOut))
            .sensitive(row.configured)
            .build();
        widget.add_css_class("destructive-action");

        let model = Rc::clone(&self.model);
        let id = row.id.clone();
        widget.connect_activated(move |_| {
            let model = Rc::clone(&model);
            let id = id.clone();
            let notice = notice.clone();
            spawn(async move {
                match model.sign_out(&id).await {
                    Ok(()) => notice.set(None),
                    Err(sentence) => notice.set(Some(&sentence.text)),
                }
            });
        });

        widget
    }

    /// One metered call against the provider, and what came back.
    fn probe(self: &Rc<Self>, row: &ProviderRow) -> adw::PreferencesGroup {
        let group = adw::PreferencesGroup::new();
        let result = CaptionRow::new();

        let button = adw::ButtonRow::builder()
            .title(copy::text(Key::ProviderTestConnection))
            .build();
        group.add(&button);
        group.add(result.row());

        let runner = self.model.probe_job();
        {
            let result = result.clone();
            let runner = Rc::clone(&runner);
            let id = row.id.clone();
            let model = Rc::clone(&self.model);
            runner.clone().observe(move || {
                if model.probing().as_deref() != Some(id.as_str()) {
                    return;
                }
                let Some(job) = runner.job() else {
                    return;
                };
                result.set(probe_sentence(&job).as_deref());
            });
        }

        let model = Rc::clone(&self.model);
        let id = row.id.clone();
        button.connect_activated(move |_| {
            let model = Rc::clone(&model);
            let id = id.clone();
            let result = result.clone();
            spawn(async move {
                if let Err(sentence) = model.probe(&id).await {
                    result.set(Some(&sentence.text));
                }
            });
        });

        group
    }
}

/// The row's second line: where it stands, and the account or model the daemon
/// named beside it.
pub fn standing_line(row: &ProviderRow) -> String {
    let word = copy::text(row.standing.key());
    match (row.account.as_deref(), row.model.as_deref()) {
        (Some(account), _) if !account.is_empty() => beside(&word, account),
        (_, Some(model)) if !model.is_empty() => beside(&word, model),
        _ => word,
    }
}

/// Write one row's changing parts: where it stands, and its one verb.
fn write_row(built: &Row, row: &ProviderRow) {
    built.row.set_subtitle(&standing_line(row));
    write_verb(&built.verb, row);
}

/// An empty verb button, at the size and alignment a row's suffix draws at.
///
/// The Setup assistant's Connect your AI screen draws the same rows this pane
/// does, so the button and the words on it are built here and used there: two
/// renderings of one rule would be two rules.
pub fn verb_button() -> gtk::Button {
    gtk::Button::builder()
        .valign(gtk::Align::Center)
        .visible(false)
        .build()
}

/// The one verb a row leads with, written onto its button.
pub fn write_verb(button: &gtk::Button, row: &ProviderRow) {
    match row.verb {
        Some(verb) => {
            let word = copy::text(verb.key());
            button.set_label(&word);
            crate::ui::shorten(button);
            // The whole word is one hover away, and the accessible name is
            // always the whole word rather than the shortened one.
            button.set_tooltip_text(Some(&word));
            button.update_property(&[gtk::accessible::Property::Label(&word)]);
            button.set_sensitive(row.can_perform());
            button.set_visible(true);
        }
        None => button.set_visible(false),
    }
}

/// The section the daemon publishes the model-behaviour rows under.
const ROUTING: &str = "routing";

const CANCEL: &str = "cancel";
const CONFIRM: &str = "confirm";
