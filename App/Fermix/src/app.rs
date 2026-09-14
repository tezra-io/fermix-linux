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
use crate::service::runner::ServiceRunner;
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
        /// The one settings model, built when the application starts and shared
        /// by every surface. `tests/structure.rs` counts the constructions.
        pub settings: RefCell<Option<Rc<SettingsModel>>>,
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
    pub fn new(paths: Paths) -> Self {
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
        application
    }

    /// The paths this process runs against.
    pub fn paths(&self) -> Paths {
        self.imp().paths.borrow().clone()
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
        let service = Rc::new(ServiceRunner::new(paths.cli()));

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

    /// Quitting closes the window first, so it records its geometry on the way
    /// out. Ending the main loop under a window that never heard it is how a
    /// remembered size quietly stops being remembered.
    fn close_and_quit(&self) {
        for window in self.windows() {
            window.close();
        }
        self.quit();
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
        self.present_window();
        if let Some(window) = self.active_window() {
            let _ = gtk::prelude::WidgetExt::activate_action(&window, "win.home", None);
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
        let row = adw::ActionRow::builder()
            .title(copy::text(spec.label))
            .build();
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
