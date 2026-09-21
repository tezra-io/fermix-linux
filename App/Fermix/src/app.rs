//! The application object.
//!
//! It owns the single instance, the one action map with its accelerators, the
//! primary menu built from that map, the shortcuts dialog built from the same
//! map, and the paths this process runs against. It owns no surface state:
//! every window reads the shared model it is handed.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use std::rc::Rc;

use crate::actions::{self, Group};
use crate::copy::{self, Key};
use crate::management::ManagementClient;
use crate::models::api::ManagementApi;
use crate::models::SettingsModel;
use crate::paths::Paths;
use crate::runtime::RuntimeEnv;
use crate::service::runner::ServiceRunner;
use crate::tray::controller::TrayController;
use crate::ui::plain;
use crate::window::FermixWindow;

/// The one application identity, in the first of its six places.
pub const APPLICATION_ID: &str = "io.tezra.Fermix";
/// The resource prefix the bundle is registered under.
pub const RESOURCE_PREFIX: &str = "/io/tezra/Fermix";
/// The gettext domain. One `.pot` per release, English only.
pub const TEXT_DOMAIN: &str = "fermix-desktop";
/// Where the product lives.
pub const WEBSITE: &str = "https://fermix.ai";

mod imp {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    pub struct FermixApplication {
        pub paths: RefCell<Paths>,
        /// The environment this process was started with, for every child it
        /// spawns. `src/runtime.rs` says why a child must not inherit this
        /// process's own.
        pub runtime: RefCell<RuntimeEnv>,
        /// The one settings model, built when the application starts and shared
        /// by every surface. `tests/structure.rs` counts the constructions.
        pub settings: RefCell<Option<Rc<SettingsModel>>>,
        /// The hold keeping the application alive for a tray icon that is
        /// actually drawing, if one was taken.
        ///
        /// The guard IS the hold: GLib's binding releases on drop and offers no
        /// separate release call. That makes the asymmetry slice5 asked for
        /// structural rather than remembered -- taking is conditional on a
        /// confirmed registration, and giving back is `take()`, which is a
        /// no-op when nothing was held. There is no second fact to disagree
        /// with this one.
        pub tray_hold: RefCell<Option<gio::ApplicationHoldGuard>>,
        /// The status icon, for as long as the application runs. Held here
        /// because the controller owns the bus objects and the name watch:
        /// dropping it would take the icon off the panel.
        pub tray: RefCell<Option<Rc<TrayController>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FermixApplication {
        const NAME: &'static str = "FermixApplication";
        type Type = super::FermixApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for FermixApplication {}

    impl ApplicationImpl for FermixApplication {
        fn startup(&self) {
            self.parent_startup();
            self.obj().on_startup();
        }

        fn activate(&self) {
            self.parent_activate();
            self.obj().present_window();
        }

        fn open(&self, files: &[gio::File], _hint: &str) {
            // Routes arrive with the setup and lifecycle slice. Until then a
            // URI raises the window and says so in the journal: an unknown
            // slug must never be a silent fall to Home.
            for file in files {
                glib::g_warning!(TEXT_DOMAIN, "no route is registered for {}", file.uri());
            }
            self.obj().present_window();
        }
    }

    impl GtkApplicationImpl for FermixApplication {}
    impl AdwApplicationImpl for FermixApplication {}
}

glib::wrapper! {
    pub struct FermixApplication(ObjectSubclass<imp::FermixApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl FermixApplication {
    /// The application, single by construction: one identity, one instance, and
    /// a second launch raises the first window rather than opening another.
    /// `runtime` is what `main` captured before this process had a second
    /// thread, and it is passed rather than reached for: nothing in this crate
    /// reads the entry environment from a global.
    pub fn new(paths: Paths, runtime: RuntimeEnv) -> Self {
        // The identity, in the place the toolkit does not put it by itself. A
        // Wayland compositor takes a window's `app_id` from the application id;
        // X11 takes `WM_CLASS` from the program name, which is the name the
        // binary was invoked as, so a window on X11 announced itself as
        // `fermix-desktop` while the launcher entry says
        // `StartupWMClass=io.tezra.Fermix`, and a shell would group neither
        // under the other. Nothing reports that: the symptom is a generic icon
        // in the dash and a window that floats free of its launcher. Setting the
        // program name here is what makes the two display servers agree.
        glib::set_prgname(Some(APPLICATION_ID));

        let application: Self = glib::Object::builder()
            .property("application-id", APPLICATION_ID)
            .property("flags", gio::ApplicationFlags::HANDLES_OPEN)
            .property("resource-base-path", RESOURCE_PREFIX)
            .build();

        application.imp().paths.replace(paths);
        application.imp().runtime.replace(runtime);
        application
    }

    /// The paths this process runs against.
    pub fn paths(&self) -> Paths {
        self.imp().paths.borrow().clone()
    }

    /// The environment this process was started with, which is the environment
    /// every child it spawns is given.
    pub fn runtime_env(&self) -> RuntimeEnv {
        self.imp().runtime.borrow().clone()
    }

    /// The one settings model, built on first use and shared from then on.
    ///
    /// Exactly one instance exists in this process: every surface reads it, and
    /// nothing copies what it holds.
    pub fn settings(&self) -> Rc<SettingsModel> {
        if let Some(settings) = self.imp().settings.borrow().clone() {
            return settings;
        }

        let paths = self.paths();

        // Two configurations, and the difference is where the socket comes
        // from. A development run is pointed at a fixture home and stays there;
        // a packaged run has nowhere to speak to until the command line reports
        // the home this account is bound to, and the model points the client at
        // it when that read lands.
        let client = match paths.fixture_socket() {
            Some(socket) => ManagementClient::new(socket),
            None => ManagementClient::unbound(),
        };
        let api: Rc<dyn ManagementApi> = Rc::new(client);
        let service = Rc::new(ServiceRunner::new(paths.cli()).with_runtime_env(self.runtime_env()));

        let settings = SettingsModel::new(api, service);
        self.imp().settings.replace(Some(Rc::clone(&settings)));
        settings
    }

    /// The primary menu, built from the one action map.
    pub fn primary_menu(&self) -> gio::Menu {
        let menu = gio::Menu::new();
        for (action, label) in actions::PRIMARY_MENU {
            menu.append(Some(&copy::text(*label)), Some(action));
        }
        menu
    }

    fn on_startup(&self) {
        bind_text_domain();
        register_resources();
        load_stylesheet();
        self.add_application_actions();
        self.bind_accelerators();
        self.start_the_tray();
    }

    /// Put the status icon on the panel, if this desktop has one.
    ///
    /// After the actions, because the rows the tray offers activate them and a
    /// panel can call one the instant the item appears.
    ///
    /// A failure here is logged and nothing else: the tray is an addition, and
    /// an application that refused to start because a session bus was missing
    /// would be worse than one with no icon. What must NOT happen is holding
    /// the application open when there is no icon, and that decision lives in
    /// the controller rather than here.
    fn start_the_tray(&self) {
        match TrayController::start(self, self.settings()) {
            Ok(tray) => {
                self.imp().tray.replace(Some(tray));
            }
            Err(error) => {
                glib::g_warning!(
                    TEXT_DOMAIN,
                    "no status icon: the session bus could not be reached: {error}"
                );
            }
        }
    }

    fn add_application_actions(&self) {
        let quit = gio::SimpleAction::new("quit", None);
        quit.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.close_and_quit()
        ));

        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.present_about()
        ));

        // The one route a notification takes back into the application: it
        // raises the window and shows Home, which is where the attention rows
        // are.
        let home = gio::SimpleAction::new("home", None);
        home.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.present_home()
        ));

        let shortcuts = gio::SimpleAction::new("shortcuts", None);
        shortcuts.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.present_shortcuts()
        ));

        // The rows the tray offers that the primary menu reaches through the
        // window. They exist as application actions because the tray is
        // clickable with no window open, and a `win.` action then goes nowhere.
        // Each one only presents the window and forwards to the window's own
        // action, so there is one implementation of each behaviour.
        let tray_settings = gio::SimpleAction::new("tray-settings", None);
        tray_settings.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.present_and_activate("win.settings")
        ));

        let tray_doctor = gio::SimpleAction::new("tray-doctor", None);
        tray_doctor.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.present_and_activate("win.run-doctor")
        ));

        let tray_restart = gio::SimpleAction::new("tray-restart", None);
        tray_restart.connect_activate(glib::clone!(
            #[weak(rename_to = application)]
            self,
            move |_, _| application.present_and_activate("win.restart")
        ));

        self.add_action(&tray_settings);
        self.add_action(&tray_doctor);
        self.add_action(&tray_restart);
        self.add_action(&quit);
        self.add_action(&about);
        self.add_action(&home);
        self.add_action(&shortcuts);
    }

    /// One table drives every accelerator, so the shortcuts dialog cannot list
    /// a binding the application does not have.
    fn bind_accelerators(&self) {
        for spec in actions::ACTIONS {
            if !spec.accels.is_empty() {
                self.set_accels_for_action(spec.name, spec.accels);
            }
        }
    }

    /// Let the application outlive its last window, because a tray icon is
    /// drawing somewhere that can bring it back.
    ///
    /// Taken only on a registration that was CONFIRMED, never on one that was
    /// merely attempted: see `tray::state::may_outlive_its_window`. On a
    /// desktop with no tray, and on one whose tray refused us, this is never
    /// called and closing the last window still ends the process -- otherwise
    /// the user is left with a process they cannot see and cannot reach.
    ///
    /// Idempotent by way of the flag, because the tray re-registers whenever
    /// the panel comes back and each of those is a confirmation. Two holds and
    /// one release would strand the process just as surely as holding with no
    /// icon.
    pub fn hold_for_tray(&self) {
        let mut hold = self.imp().tray_hold.borrow_mut();
        if hold.is_some() {
            return;
        }
        *hold = Some(self.upcast_ref::<gio::Application>().hold());
    }

    /// Quitting closes the window first, so it records its geometry on the way
    /// out. Ending the main loop under a window that never heard it is how a
    /// remembered size quietly stops being remembered.
    ///
    /// The tray hold is released here, on the one path every deliberate exit
    /// passes through, and the release is NOT conditional on anything but the
    /// flag this object itself set. The asymmetry with `hold_for_tray` is
    /// deliberate: a release that is harmless when nothing was held is safer
    /// than one that is correct only while two facts agree about whether a hold
    /// was taken. If those two ever disagree, this way loses an icon and that
    /// way strands a process with no window.
    fn close_and_quit(&self) {
        self.release_tray_hold();

        for window in self.windows() {
            window.close();
        }
        self.quit();
    }

    /// Give back the tray's hold because the status area went away.
    ///
    /// The icon was the only way back to an application with no window, so
    /// losing it has to undo the hold that the icon earned. Same rule as a
    /// desktop that never had a tray, reached from the other direction.
    pub fn release_tray_hold_for_lost_tray(&self) {
        self.release_tray_hold();
    }

    /// Give back the tray's hold.
    ///
    /// Unconditional: `take()` drops whatever is there, and dropping nothing is
    /// exactly nothing. Nothing here asks whether a hold was taken, because a
    /// release that is harmless when none was held is safer than one that is
    /// correct only while two facts agree.
    fn release_tray_hold(&self) {
        self.imp().tray_hold.borrow_mut().take();
    }

    fn present_window(&self) {
        match self.active_window() {
            Some(window) => window.present(),
            None => FermixWindow::new(self, self.settings()).present(),
        }
    }

    /// Raise the window and show Home. The window's own action is what actually
    /// shows the page, so there is one implementation of showing it.
    fn present_home(&self) {
        self.present_and_activate("win.home");
    }

    /// Raise the window, creating one if none is open, and hand the work to the
    /// window's own action.
    ///
    /// This is what every tray row needs and what `present_home` already did.
    /// The tray can be clicked when no window exists at all, which is the whole
    /// point of it, and a `win.` action with no window goes nowhere silently --
    /// so the window is presented first and the action activated second, in
    /// that order, every time.
    ///
    /// The action is named rather than reimplemented so that a row in the tray
    /// and the same row in the primary menu cannot drift into two behaviours.
    fn present_and_activate(&self, action: &str) {
        self.present_window();

        let Some(window) = self.active_window() else {
            // Presenting failed, which should not happen: present_window builds
            // a window when there is none. Said out loud rather than returned
            // into nothing, because the symptom would be a tray row that does
            // nothing at all.
            glib::g_warning!(TEXT_DOMAIN, "{action} had no window to act on");
            return;
        };

        if !gtk::prelude::WidgetExt::activate_action(&window, action, None).is_ok() {
            glib::g_warning!(TEXT_DOMAIN, "the window has no {action} to activate");
        }
    }

    fn present_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_icon(APPLICATION_ID)
            .application_name(copy::text(Key::ProductName))
            .developer_name(copy::text(Key::AboutDeveloper))
            .version(env!("CARGO_PKG_VERSION"))
            .website(WEBSITE)
            .comments(copy::text(Key::AboutComments))
            .license_type(gtk::License::MitX11)
            .build();

        dialog.present(self.active_window().as_ref());
    }

    /// The shortcuts reference, built from the same table the accelerators come
    /// from. It is an ordinary dialog rather than the toolkit's own shortcuts
    /// window, which is above the version floor this build targets.
    fn present_shortcuts(&self) {
        let page = adw::PreferencesPage::new();
        for which in [Group::General, Group::Navigation, Group::Actions] {
            page.add(&shortcuts_group(which));
        }

        let header = adw::HeaderBar::new();
        let toolbar = adw::ToolbarView::builder().content(&page).build();
        toolbar.add_top_bar(&header);

        let dialog = adw::Dialog::builder()
            .title(copy::text(Key::ShortcutsTitle))
            .child(&toolbar)
            .build();

        dialog.present(self.active_window().as_ref());
    }
}

fn shortcuts_group(which: Group) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(copy::text(which.title()))
        .build();

    for spec in actions::group(which) {
        let row = plain(
            adw::ActionRow::builder()
                .title(copy::text(spec.label))
                .build(),
        );
        for accel in spec.accels {
            row.add_suffix(&gtk::ShortcutLabel::new(accel));
        }
        group.add(&row);
    }

    group
}

/// Marks the application's strings for extraction and binds the catalogue
/// directory. v1 ships the `.pot` and no `.po`, so every lookup answers with
/// the English source and the casing column does the rest.
fn bind_text_domain() {
    // Setting the locale mutates process-wide state, which is why the binding
    // is unsafe. It runs once, inside startup, before this process has a
    // second thread or has drawn anything.
    let locale = unsafe { gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "") };
    if locale.is_none() {
        glib::g_debug!(TEXT_DOMAIN, "the locale was not set from the environment");
    }
    if let Err(error) = gettextrs::bindtextdomain(TEXT_DOMAIN, "/usr/share/locale") {
        glib::g_warning!(TEXT_DOMAIN, "the message catalogue was not bound: {error}");
    }
    if let Err(error) = gettextrs::textdomain(TEXT_DOMAIN) {
        glib::g_warning!(TEXT_DOMAIN, "the text domain was not set: {error}");
    }
}

fn register_resources() {
    gio::resources_register_include!("fermix.gresource")
        .expect("the compiled resource bundle is part of the binary");

    if let Some(display) = gtk::gdk::Display::default() {
        gtk::IconTheme::for_display(&display)
            .add_resource_path(&format!("{RESOURCE_PREFIX}/icons"));
        // The bundled Adwaita theme, behind whatever the host has. It is added
        // here rather than in `main` because an icon theme belongs to a display
        // and there is no display until the toolkit has started.
        crate::runtime::add_bundled_icons(&display);
    }
}

fn load_stylesheet() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };

    let provider = gtk::CssProvider::new();
    provider.load_from_resource(&format!("{RESOURCE_PREFIX}/style.css"));
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
