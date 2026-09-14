//! Doctor.
//!
//! A summary line, then one row per check: the pill, the daemon's name for the
//! check, its summary, and for a row that needs someone the daemon's own
//! remediation with the one button its kind routes to. Evidence sits in an
//! expander beneath the row it belongs to.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::{
    DoctorScope, DoctorSessionStatus, Remediation, RemediationKind, SettingsPane,
};
use crate::models::doctor::{status_word, CheckName, CheckRow, DoctorModel};
use crate::models::{spawn, SettingsModel};

use super::widgets::status_pill;
use super::{caption, identifier_label, value_label, PageToolbar};

/// The Doctor surface.
pub struct DoctorPage {
    root: gtk::Widget,
    settings: Rc<SettingsModel>,
    doctor: Rc<DoctorModel>,
    summary: gtk::Label,
    checked: gtk::Label,
    spinner: adw::Spinner,
    refusal: gtk::Label,
    checks: adw::PreferencesGroup,
    rows: RefCell<Vec<adw::PreferencesRow>>,
    drawn: RefCell<Vec<CheckRow>>,
    toolbar: PageToolbar,
}

impl DoctorPage {
    /// Build Doctor over the one settings model.
    pub fn new(settings: Rc<SettingsModel>, doctor: Rc<DoctorModel>) -> Rc<Self> {
        let summary = gtk::Label::builder().xalign(0.0).build();
        summary.add_css_class("title-1");
        let checked = caption("");
        let spinner = adw::Spinner::new();
        spinner.set_visible(false);
        let refusal = caption("");
        refusal.set_visible(false);

        let checks = adw::PreferencesGroup::new();
        let column = super::column();
        column.add_css_class("fermix-gutter");
        column.append(&summary_block(&summary, &spinner, &checked, &refusal));
        column.append(&checks);

        let network = gtk::Button::builder()
            .label(copy::text(Key::DoctorRunNetworkChecks))
            .tooltip_text(copy::text(Key::DoctorNetworkChecksCost))
            .build();
        // The header bar spans the whole window, so a label that cannot shrink
        // in it is a floor under the window's own minimum width. The whole word
        // is what the accessible name says, at every width.
        super::shorten(&network);
        network.update_property(&[gtk::accessible::Property::Label(&copy::text(
            Key::DoctorRunNetworkChecks,
        ))]);
        let menu = support_menu();

        let page = Rc::new(Self {
            root: super::scrolled(&super::clamp(&column)).upcast(),
            settings,
            doctor,
            summary,
            checked,
            spinner,
            refusal,
            checks,
            rows: RefCell::new(Vec::new()),
            drawn: RefCell::new(Vec::new()),
            toolbar: PageToolbar {
                start: Vec::new(),
                end: vec![network.clone().upcast(), menu.clone().upcast()],
            },
        });

        page.connect(&network, &menu);
        page.draw();
        page
    }

    /// The widget to put in the window.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// What this page puts in the header bar.
    pub fn toolbar(&self) -> PageToolbar {
        self.toolbar.clone()
    }

    /// Run the local checks again. The window's Run Doctor action calls this.
    pub fn run_local(&self) {
        self.doctor.run(DoctorScope::Local);
    }

    fn connect(self: &Rc<Self>, network: &gtk::Button, menu: &gtk::MenuButton) {
        {
            let page = Rc::downgrade(self);
            self.doctor.observe(move || {
                if let Some(page) = page.upgrade() {
                    page.draw();
                }
            });
        }

        {
            let page = Rc::clone(self);
            network.connect_clicked(move |_| page.doctor.run(DoctorScope::Network));
        }

        self.install_support_actions(menu);

        // The local run starts on entering the surface and is cancelled on
        // leaving it: a session is a resource on the daemon, and at most two
        // exist at once.
        {
            let page = Rc::clone(self);
            super::on_shown(&self.root, move || page.doctor.run(DoctorScope::Local));
        }

        let page = Rc::clone(self);
        super::watch_visibility(&self.root, move |visible| {
            if visible {
                return;
            }
            let doctor = Rc::clone(&page.doctor);
            spawn(async move {
                doctor.cancel().await;
            });
        });
    }

    fn install_support_actions(self: &Rc<Self>, menu: &gtk::MenuButton) {
        let group = gio::SimpleActionGroup::new();

        let export = gio::SimpleAction::new("export-bundle", None);
        {
            let page = Rc::clone(self);
            export.connect_activate(move |_, _| page.export_bundle());
        }
        group.add_action(&export);

        let folder = gio::SimpleAction::new("show-log-folder", None);
        {
            let page = Rc::clone(self);
            folder.connect_activate(move |_, _| page.show_log_folder());
        }
        group.add_action(&folder);
        menu.insert_action_group("doctor", Some(&group));

        // The folder is only known once the command line has said where this
        // account's Fermix keeps its files. Until then the item cannot be used
        // and the menu says why rather than opening nothing.
        let apply = {
            let menu = menu.clone();
            let page = Rc::downgrade(self);
            move || {
                let Some(page) = page.upgrade() else {
                    return;
                };
                let known = page.log_folder().is_some();
                folder.set_enabled(known);

                let unavailable = copy::text(Key::DoctorLogFolderUnavailable);
                menu.set_tooltip_text(if known { None } else { Some(&unavailable) });
            }
        };

        apply();
        self.settings.observe(move |_| apply());
    }

    /// Where this account's Fermix keeps its files, as the command line
    /// reported it.
    fn log_folder(&self) -> Option<std::path::PathBuf> {
        let state = self.settings.state();
        let home = state.service.as_ref()?.bound_home()?.to_string();

        Some(std::path::PathBuf::from(home).join("logs"))
    }

    fn show_log_folder(self: &Rc<Self>) {
        let Some(folder) = self.log_folder() else {
            return;
        };
        let window = super::window_of(&self.root);

        spawn(async move {
            if let Err(error) =
                crate::session::DesktopSession::show_folder(window.as_ref(), &folder).await
            {
                gtk::glib::g_warning!("fermix-desktop", "the log folder did not open: {error}");
            }
        });
    }

    /// Build the bundle the daemon assembles, and write it where a person says.
    fn export_bundle(self: &Rc<Self>) {
        let page = Rc::clone(self);
        spawn(async move {
            let api = page.settings.api();
            let issued = crate::models::api::ask::<_, serde_json::Value>(
                api.as_ref(),
                "diagnostics.build",
                &serde_json::json!({}),
                crate::models::api::READ_DEADLINE,
            )
            .await;

            let Some(Ok(bundle)) = crate::models::api::accept(api.as_ref(), issued) else {
                return;
            };
            let Ok(bytes) = serde_json::to_vec_pretty(&bundle) else {
                return;
            };

            page.write_file(&bytes).await;
        });
    }

    async fn write_file(&self, bytes: &[u8]) {
        let dialog = gtk::FileDialog::builder()
            .title(copy::text(Key::DoctorExportSupportBundle))
            .initial_name(BUNDLE_NAME)
            .build();

        let window = super::window_of(&self.root);
        let Ok(file) = dialog.save_future(window.as_ref()).await else {
            // Cancelled. Nothing was written and nothing needs saying.
            return;
        };

        if let Err(error) = file
            .replace_contents_future(
                bytes.to_vec(),
                None,
                false,
                gio::FileCreateFlags::REPLACE_DESTINATION,
            )
            .await
        {
            gtk::glib::g_warning!("fermix-desktop", "the bundle was not written: {error:?}");
        }
    }

    /// Draw the session as it stands.
    pub fn draw(&self) {
        let running = self.doctor.is_running();
        let session = self.doctor.session();

        self.spinner.set_visible(running);
        self.checked.set_visible(!running && session.is_some());
        self.checked
            .set_label(&copy::text(Key::DoctorCheckedJustNow));

        let failed = self.doctor.failed();
        self.summary.set_label(&match session.as_ref() {
            Some(_) if running => copy::text(Key::DoctorRunning),
            Some(_) if failed > 0 => copy::fill(
                Key::DoctorSummaryFailed,
                &[("{count}", &failed.to_string())],
            ),
            Some(_) => copy::text(Key::DoctorSummaryHealthy),
            None => copy::text(Key::DoctorRunning),
        });

        match self.doctor.refusal() {
            Some(refusal) => {
                self.refusal.set_label(&refusal.text);
                self.refusal.set_visible(true);
            }
            None => self.refusal.set_visible(false),
        }

        if session
            .as_ref()
            .map(|session| session.status == DoctorSessionStatus::TimedOut)
            .unwrap_or(false)
        {
            self.checked.set_visible(true);
            self.checked
                .set_label(&copy::text(Key::DoctorStatusTimedOut));
        }

        self.draw_rows();
    }

    /// Redraw the check rows, and only when they changed.
    fn draw_rows(&self) {
        let rows = self.doctor.rows();
        if *self.drawn.borrow() == rows {
            return;
        }

        for row in self.rows.borrow_mut().drain(..) {
            self.checks.remove(&row);
        }

        let mut drawn = Vec::new();
        for row in &rows {
            let widget = self.check_row(row);
            self.checks.add(&widget);
            drawn.push(widget.upcast::<adw::PreferencesRow>());

            // Evidence belongs under the rows somebody has to act on. A row
            // that passed has nothing to look into.
            if row.needs_action() && !row.evidence.is_empty() {
                let evidence = evidence_row(row);
                self.checks.add(&evidence);
                drawn.push(evidence.upcast::<adw::PreferencesRow>());
            }
        }

        self.rows.replace(drawn);
        self.drawn.replace(rows);
    }

    fn check_row(&self, row: &CheckRow) -> adw::ActionRow {
        let widget = adw::ActionRow::builder().activatable(false).build();

        match &row.name {
            CheckName::Words(name) => widget.set_title(name),
            CheckName::Identifier(id) => {
                widget.set_title("");
                widget.add_prefix(&identifier_label(id));
            }
        }

        widget.set_subtitle(&subtitle(row));
        // The pill and the word travel together: status is never a letter on
        // its own, and a word in the suffix competes with the row's one button
        // for the same space.
        widget.add_prefix(&status_mark(row.status));

        if let Some(remediation) = row.remediation.as_ref() {
            if let Some(button) = self.remediation_button(remediation) {
                widget.add_suffix(&button);
            }
        }

        widget
    }

    /// The one button a remediation's kind routes to.
    ///
    /// A kind this build cannot route has no button: a button that goes nowhere
    /// is worse than the sentence on its own, and the sentence is the daemon's.
    fn remediation_button(&self, remediation: &Remediation) -> Option<gtk::Button> {
        let label = match remediation.action.kind {
            RemediationKind::SettingsPane => Key::ActionOpenSettingsPane,
            RemediationKind::Restart => Key::MenuRestart,
            RemediationKind::Reload => Key::BannerReloadFromDisk,
            RemediationKind::Instructions => Key::ActionShowMeHow,
            _ => return None,
        };

        let button = gtk::Button::builder()
            .label(copy::text(label))
            .valign(gtk::Align::Center)
            .build();

        let settings = Rc::clone(&self.settings);
        let remediation = remediation.clone();
        button.connect_clicked(move |button| match remediation.action.kind {
            RemediationKind::SettingsPane => {
                if let Some(pane) = pane_named(remediation.action.target.as_deref()) {
                    settings.select_pane(pane);
                }
                let _ = WidgetExt::activate_action(button, "win.settings", None);
            }
            RemediationKind::Restart => {
                let _ = WidgetExt::activate_action(button, "win.restart", None);
            }
            RemediationKind::Reload => {
                let settings = Rc::clone(&settings);
                spawn(async move {
                    settings.reload().await;
                });
            }
            RemediationKind::Instructions => show_instructions(button, &remediation),
            _ => {}
        });

        Some(button)
    }
}

/// The summary line, the spinner that runs only while something is running,
/// and the two lines under them.
fn summary_block(
    summary: &gtk::Label,
    spinner: &adw::Spinner,
    checked: &gtk::Label,
    refusal: &gtk::Label,
) -> gtk::Box {
    let heading = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(crate::metrics::SPACE_HEADING)
        .build();
    heading.append(summary);
    heading.append(spinner);

    let block = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(crate::metrics::SPACE_TIGHT)
        .build();
    block.append(&heading);
    block.append(checked);
    block.append(refusal);
    block
}

/// The name the export is offered under.
const BUNDLE_NAME: &str = "fermix-diagnostics.json";

/// The width the status column reserves, in characters: the longest of the
/// eight words the daemon can answer with. Every row then starts its name at
/// the same place, whichever word it carries.
const STATUS_WORD_WIDTH: i32 = 14;

/// The pill and its word, as one prefix of one width.
fn status_mark(status: crate::management::types::CheckStatus) -> gtk::Box {
    let mark = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(crate::metrics::SPACE_TIGHT)
        .valign(gtk::Align::Center)
        .build();

    let word = gtk::Label::builder()
        .label(copy::text(status_word(status)))
        .width_chars(STATUS_WORD_WIDTH)
        .xalign(0.0)
        .build();
    word.add_css_class("dim-label");

    mark.append(&status_pill(status));
    mark.append(&word);
    mark
}

/// The row's own second line: the daemon's summary, and its remediation title
/// where it published one.
fn subtitle(row: &CheckRow) -> String {
    match row.remediation.as_ref() {
        Some(remediation) if row.needs_action() => {
            format!("{}\n{}", row.summary, remediation.title)
        }
        _ => row.summary.clone(),
    }
}

/// The evidence, in an expander under the row it belongs to.
fn evidence_row(row: &CheckRow) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::builder()
        .title(copy::text(Key::DoctorEvidence))
        .expanded(false)
        .build();

    for (label, value) in &row.evidence {
        let line = adw::ActionRow::builder().activatable(false).build();
        line.add_prefix(&identifier_label(label));
        line.add_suffix(&value_label(value));
        expander.add_row(&line);
    }

    expander
}

/// The daemon's own instructions, in a dialog that says them and closes.
fn show_instructions(widget: &impl IsA<gtk::Widget>, remediation: &Remediation) {
    let dialog = adw::AlertDialog::new(Some(&remediation.title), Some(&remediation.body));
    dialog.add_response("close", &copy::text(Key::ActionClose));
    dialog.set_default_response(Some("close"));
    dialog.set_close_response("close");
    dialog.present(Some(widget.as_ref()));
}

/// The pane a remediation names, where this build can route to it.
fn pane_named(target: Option<&str>) -> Option<SettingsPane> {
    crate::management::vocabulary::pane_for_slug(target?)
}

fn support_menu() -> gtk::MenuButton {
    let menu = gio::Menu::new();
    menu.append(
        Some(&copy::text(Key::DoctorExportSupportBundle)),
        Some("doctor.export-bundle"),
    );
    menu.append(
        Some(&copy::text(Key::DoctorShowLogFolder)),
        Some("doctor.show-log-folder"),
    );

    let button = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&copy::text(
        Key::ActionMoreAccessible,
    ))]);
    button
}
