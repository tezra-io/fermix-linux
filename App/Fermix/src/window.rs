//! The one window.
//!
//! One `AdwApplicationWindow`, one `AdwToolbarView`, one `AdwHeaderBar`, one
//! `AdwNavigationSplitView`. Settings is a presentation of that same split view
//! rather than a second window or a second split view: entering swaps the
//! sidebar for the pane list and the detail for the pane, and leaving restores
//! the previous page, its scroll position and its focus.
//!
//! The window owns three things no page owns: the back control, the one
//! prominent action while its condition holds, and the primary menu. A page
//! hands it the controls that belong to the page, and the count of trailing
//! children never passes three.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use crate::app::FermixApplication;
use crate::copy::{self, Key};
use crate::metrics;
use crate::models::doctor::DoctorModel;
use crate::models::home::{HomeModel, ToolbarAction};
use crate::models::logs::LogsModel;
use crate::models::notices::{AttentionNotice, Notice};
use crate::models::recovery::RecoveryModel;
use crate::models::{pane, spawn, Change, SettingsModel};
use crate::session::state::{self, WindowState};
use crate::session::DesktopSession;
use crate::ui::doctor::DoctorPage;
use crate::ui::home::HomePage;
use crate::ui::logs::LogsPage;
use crate::ui::onboarding::Assistant;
use crate::ui::recovery::RecoveryPage;
use crate::ui::settings::SettingsPresentation;
use crate::ui::PageToolbar;

use imp::Previous;

/// The destinations the sidebar reaches, in sidebar order. Settings is the
/// pinned row at the bottom of the same list and the same focus order.
const DESTINATIONS: &[Destination] = &[
    Destination {
        page: "home",
        title: Key::PageHome,
        icon: "user-home-symbolic",
        action: "win.home",
    },
    Destination {
        page: "doctor",
        title: Key::PageDoctor,
        // Doctor examines this install, and the four sidebar glyphs have to be
        // four different things at a glance.
        icon: "system-search-symbolic",
        action: "win.doctor",
    },
    Destination {
        page: "logs",
        title: Key::PageLogs,
        icon: "view-list-symbolic",
        action: "win.logs",
    },
];

/// The pinned Settings row, which is in the same list and the same focus order
/// as the three above and is reached by the same action map.
const SETTINGS: Destination = Destination {
    page: "settings",
    title: Key::PageSettings,
    icon: "emblem-system-symbolic",
    action: "win.settings",
};

/// The page Recovery shows on. It is a state rather than a destination: nothing
/// in the sidebar reaches it, and the daemon's own answer is what puts someone
/// there.
const RECOVERY: &str = "recovery";

/// The page the Setup assistant shows on. Like Recovery it is a state rather
/// than a destination: the sidebar does not reach it, and Home's own prominent
/// action or an unfinished setup is what puts someone there.
const SETUP: &str = "setup";

#[derive(Clone, Copy)]
struct Destination {
    page: &'static str,
    title: Key,
    icon: &'static str,
    action: &'static str,
}

mod imp {
    use super::*;

    /// What leaving Settings has to put back.
    pub struct Previous {
        pub page: String,
        pub scroll: f64,
        pub focus: glib::WeakRef<gtk::Widget>,
    }

    /// Everything the window draws from. One settings model, and the surfaces
    /// built over it.
    pub struct Models {
        pub settings: Rc<SettingsModel>,
        pub home: Rc<HomePage>,
        pub doctor: Rc<DoctorPage>,
        pub logs: Rc<LogsPage>,
        pub recovery: Rc<RecoveryPage>,
        pub presentation: Rc<SettingsPresentation>,
        pub setup: Rc<Assistant>,
    }

    #[derive(Default)]
    pub struct FermixWindow {
        pub split: adw::NavigationSplitView,
        pub sidebar_stack: gtk::Stack,
        pub content_stack: gtk::Stack,
        pub sidebar_list: gtk::ListBox,
        pub title: adw::WindowTitle,
        pub back: gtk::Button,
        pub prominent: gtk::Button,
        pub page_start: gtk::Box,
        pub page_end: gtk::Box,
        pub toolbar: RefCell<Option<adw::ToolbarView>>,
        pub header: RefCell<Option<adw::HeaderBar>>,
        pub in_settings: Cell<bool>,
        pub in_setup: Cell<bool>,
        /// How many things needed someone when this window last looked.
        pub notice: AttentionNotice,
        pub forced_collapsed: Cell<bool>,
        pub previous: RefCell<Option<Previous>>,
        pub scrollers: RefCell<HashMap<String, gtk::ScrolledWindow>>,
        pub models: RefCell<Option<Models>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FermixWindow {
        const NAME: &'static str = "FermixWindow";
        type Type = super::FermixWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for FermixWindow {}

    impl WidgetImpl for FermixWindow {}
    impl WindowImpl for FermixWindow {
        fn close_request(&self) -> glib::Propagation {
            self.obj().remember_geometry();
            self.obj().withdraw_notices();
            self.parent_close_request()
        }
    }
    impl ApplicationWindowImpl for FermixWindow {}
    impl AdwApplicationWindowImpl for FermixWindow {}
}

glib::wrapper! {
    pub struct FermixWindow(ObjectSubclass<imp::FermixWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
            gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl FermixWindow {
    /// The window, built over the one settings model and with its geometry
    /// restored.
    ///
    /// The tree is assembled here rather than in `constructed`, because the
    /// application is an ordinary property rather than a construct-only one: it
    /// is not set until after construction, and the window reads it while
    /// building its menu.
    pub fn new(application: &FermixApplication, settings: Rc<SettingsModel>) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", application)
            .build();

        let recorded = state::load();
        window.build(settings, recorded.last_pane.clone());
        window.restore_geometry(recorded);
        window.load();
        window
    }

    /// Which destination is showing.
    pub fn current_page(&self) -> String {
        self.imp()
            .content_stack
            .visible_child_name()
            .map(Into::into)
            .unwrap_or_default()
    }

    /// Whether the Settings presentation is showing.
    pub fn in_settings(&self) -> bool {
        self.imp().in_settings.get()
    }

    /// Whether the sidebar is on screen, either beside the detail or in front
    /// of it while navigation is collapsed.
    pub fn sidebar_shown(&self) -> bool {
        let split = &self.imp().split;
        !split.is_collapsed() || !split.shows_content()
    }

    /// The one settings model this window draws from.
    pub fn settings(&self) -> Rc<SettingsModel> {
        let models = self.imp().models.borrow();
        Rc::clone(&models.as_ref().expect("the window has its models").settings)
    }

    /// The Settings presentation, for the tests that walk it.
    pub fn presentation(&self) -> Rc<SettingsPresentation> {
        let models = self.imp().models.borrow();
        Rc::clone(
            &models
                .as_ref()
                .expect("the window has its models")
                .presentation,
        )
    }

    /// The content half of the window, for the gate that measures what each
    /// page asks for. Every page is a child of this one stack.
    pub fn content(&self) -> gtk::Widget {
        self.imp().content_stack.clone().upcast()
    }

    /// How many children the header bar carries at its trailing edge, which the
    /// redlines cap at three.
    pub fn trailing_children(&self) -> usize {
        let imp = self.imp();

        // The primary menu, the page's own controls, and the one prominent
        // action while its condition holds.
        1 + usize::from(imp.prominent.is_visible()) + count_children(&imp.page_end)
    }

    fn build(&self, settings: Rc<SettingsModel>, last_pane: Option<String>) {
        self.set_title(Some(&copy::text(Key::ProductName)));
        self.set_size_request(
            metrics::WINDOW_MINIMUM_WIDTH,
            metrics::WINDOW_MINIMUM_HEIGHT,
        );

        if let Some(pane) = last_pane.as_deref().and_then(pane::for_slug) {
            settings.select_pane(pane);
        }

        let models = imp::Models {
            home: HomePage::new(Rc::clone(&settings), HomeModel::new(Rc::clone(&settings))),
            doctor: DoctorPage::new(Rc::clone(&settings), DoctorModel::new(Rc::clone(&settings))),
            logs: LogsPage::new(Rc::clone(&settings), LogsModel::new(Rc::clone(&settings))),
            recovery: RecoveryPage::new(
                Rc::clone(&settings),
                RecoveryModel::new(Rc::clone(&settings)),
            ),
            presentation: SettingsPresentation::new(Rc::clone(&settings)),
            setup: Assistant::new(Rc::clone(&settings)),
            settings,
        };
        self.imp().models.replace(Some(models));

        self.follow_assistant();
        self.build_sidebar();
        self.build_content();
        self.build_split_view();
        self.build_chrome();
        self.add_window_actions();
        self.install_breakpoint();
        self.observe_model();
        self.show_page(DESTINATIONS[0]);
    }

    /// Follow the assistant's own screen.
    ///
    /// The window owns the back control and the assistant owns which screen is
    /// showing, and only one screen refuses to be left by Escape. Without this
    /// the action's enabled state is whatever it was when the assistant opened.
    fn follow_assistant(&self) {
        let window = self.downgrade();
        self.assistant().model().observe(move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            if window.in_setup() {
                window.update_back_action();
            }
        });
    }

    /// Read everything the window draws, once, at the start.
    fn load(&self) {
        let settings = self.settings();
        spawn(async move {
            settings.refresh_all().await;
        });

        let presentation = self.presentation();
        presentation.connect_banner();

        let window = self.clone();
        presentation.on_recovery(move || window.show_recovery());

        let window = self.clone();
        presentation.on_title_changed(move || window.update_title());

        // The ledger's background-service row points at Home rather than
        // drawing a second switch for one thing.
        let window = self.clone();
        presentation.on_home(move || window.leave_settings_then_show(DESTINATIONS[0]));
    }

    fn build_sidebar(&self) {
        let imp = self.imp();

        imp.sidebar_list
            .set_selection_mode(gtk::SelectionMode::Single);
        imp.sidebar_list.add_css_class("navigation-sidebar");
        imp.sidebar_list
            .update_property(&[gtk::accessible::Property::Label(&copy::text(
                Key::SidebarAccessible,
            ))]);

        for destination in DESTINATIONS.iter().chain(std::iter::once(&SETTINGS)) {
            imp.sidebar_list.append(&sidebar_row(*destination));
        }

        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&imp.sidebar_list)
            .vexpand(true)
            .build();

        imp.sidebar_stack.add_named(&scroller, Some("app"));
        imp.sidebar_stack
            .add_named(&self.presentation().sidebar(), Some("settings"));
        imp.sidebar_stack.set_visible_child_name("app");
    }

    fn build_content(&self) {
        let imp = self.imp();
        let models = imp.models.borrow();
        let models = models.as_ref().expect("the window has its models");

        let pages: Vec<(&str, gtk::Widget)> = vec![
            ("home", models.home.widget()),
            ("doctor", models.doctor.widget()),
            ("logs", models.logs.widget()),
            ("settings", models.presentation.detail()),
            (SETUP, models.setup.widget()),
            (RECOVERY, models.recovery.widget()),
        ];

        for (name, widget) in pages {
            if let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>() {
                imp.scrollers
                    .borrow_mut()
                    .insert(name.to_string(), scroller.clone());
            }
            imp.content_stack.add_named(&widget, Some(name));
        }

        imp.content_stack
            .set_transition_type(gtk::StackTransitionType::Crossfade);
        // The page being shown decides the window's minimum, not the widest of
        // the five.
        imp.content_stack.set_hhomogeneous(false);
        imp.content_stack.set_vhomogeneous(false);
        crate::motion::follow(&imp.content_stack, crate::motion::Transition::Crossfade);
    }

    fn build_split_view(&self) {
        let imp = self.imp();

        let sidebar = adw::NavigationPage::builder()
            .title(copy::text(Key::ProductName))
            .child(&imp.sidebar_stack)
            .build();

        let content = adw::NavigationPage::builder()
            .title(copy::text(Key::PageHome))
            .child(&imp.content_stack)
            .build();

        imp.split.set_sidebar(Some(&sidebar));
        imp.split.set_content(Some(&content));
        imp.split
            .set_min_sidebar_width(metrics::SIDEBAR_WIDTH as f64);
        imp.split
            .set_max_sidebar_width(metrics::SIDEBAR_WIDTH as f64);
        imp.split.set_sidebar_width_fraction(0.3);

        for property in ["collapsed", "show-content"] {
            imp.split.connect_notify_local(
                Some(property),
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |_, _| {
                        window.update_back_action();
                        window.update_gutter();
                    }
                ),
            );
        }
    }

    fn build_chrome(&self) {
        let imp = self.imp();

        imp.back.set_icon_name("go-previous-symbolic");
        imp.back.set_action_name(Some("win.back"));
        imp.back.set_visible(false);
        imp.back
            .update_property(&[gtk::accessible::Property::Label(&copy::text(
                Key::BackToFermix,
            ))]);

        imp.prominent.add_css_class("suggested-action");
        imp.prominent.set_visible(false);
        imp.page_start.set_spacing(metrics::SPACE_TIGHT);
        imp.page_end.set_spacing(metrics::SPACE_TIGHT);

        let menu = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&self.application_object().primary_menu())
            .primary(true)
            .build();
        menu.update_property(&[gtk::accessible::Property::Label(&copy::text(
            Key::MenuPrimaryAccessible,
        ))]);

        let header = adw::HeaderBar::builder().title_widget(&imp.title).build();
        header.pack_start(&imp.back);
        header.pack_start(&imp.page_start);
        header.pack_end(&menu);
        header.pack_end(&imp.page_end);
        header.pack_end(&imp.prominent);

        let toolbar = adw::ToolbarView::builder().content(&imp.split).build();
        toolbar.add_top_bar(&header);
        self.set_content(Some(&toolbar));
        imp.toolbar.replace(Some(toolbar));
        imp.header.replace(Some(header));

        let window = self.clone();
        imp.prominent
            .connect_clicked(move |_| window.run_prominent());
    }

    fn install_breakpoint(&self) {
        let condition = adw::BreakpointCondition::parse(&metrics::collapse_condition())
            .expect("the breakpoint condition is a compile-time constant");

        let breakpoint = adw::Breakpoint::new(condition);
        breakpoint.add_setter(&self.imp().split, "collapsed", Some(&true.to_value()));
        self.add_breakpoint(breakpoint);
    }

    /// Follow the model: the prominent action and the window title.
    fn observe_model(&self) {
        let window = self.downgrade();
        self.settings().observe(move |change| {
            let Some(window) = window.upgrade() else {
                return;
            };
            match change {
                Change::Daemon | Change::Setup | Change::Service => {
                    window.update_prominent();
                    window.raise_attention_notice();
                }
                Change::Pane => window.update_title(),
                _ => {}
            }
        });
    }

    // ---- Actions --------------------------------------------------------

    fn add_window_actions(&self) {
        self.add_navigation_actions();
        self.add_surface_actions();
    }

    /// Getting somewhere: the four destinations, back, the sidebar, the window.
    fn add_navigation_actions(&self) {
        self.add_action(&self.page_action("home", DESTINATIONS[0]));
        self.add_action(&self.page_action("doctor", DESTINATIONS[1]));
        self.add_action(&self.page_action("logs", DESTINATIONS[2]));

        let settings = gio::SimpleAction::new("settings", None);
        settings.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.enter_settings()
        ));
        self.add_action(&settings);

        let back = gio::SimpleAction::new("back", None);
        back.set_enabled(false);
        back.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.go_back()
        ));
        self.add_action(&back);

        let sidebar = gio::SimpleAction::new("toggle-sidebar", None);
        sidebar.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.set_sidebar_shown(!window.sidebar_shown())
        ));
        self.add_action(&sidebar);

        let close = gio::SimpleAction::new("close", None);
        close.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.close()
        ));
        self.add_action(&close);
    }

    /// Doing something on the surface that is showing.
    fn add_surface_actions(&self) {
        let search = gio::SimpleAction::new("search", None);
        search.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.focus_search()
        ));
        self.add_action(&search);

        let doctor = gio::SimpleAction::new("run-doctor", None);
        doctor.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.run_doctor()
        ));
        self.add_action(&doctor);

        let restart = gio::SimpleAction::new("restart", None);
        restart.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.present_restart()
        ));
        self.add_action(&restart);
    }

    fn page_action(&self, name: &str, destination: Destination) -> gio::SimpleAction {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.leave_settings_then_show(destination)
        ));
        action
    }

    /// Run Doctor, wherever it is asked for: the menu, the accelerator or the
    /// surface itself.
    fn run_doctor(&self) {
        if self.current_page() != "doctor" {
            self.leave_settings_then_show(DESTINATIONS[1]);
        }

        let models = self.imp().models.borrow();
        if let Some(models) = models.as_ref() {
            models.doctor.run_local();
        }
    }

    /// Put the focus in the search of whichever surface has one.
    ///
    /// Inside Settings a pane may own one of its own, and it wins while that
    /// pane is showing: the pane list's search is what the sidebar answers
    /// with, and Integrations searches integrations.
    fn focus_search(&self) {
        if self.in_settings() {
            let presentation = self.presentation();
            if !presentation.focus_pane_search() {
                presentation.pane_list().focus_search();
            }
            return;
        }
        if self.current_page() == "logs" {
            let models = self.imp().models.borrow();
            if let Some(models) = models.as_ref() {
                models.logs.focus_search();
            }
        }
    }

    /// The one restart confirmation, from every route that reaches it.
    fn present_restart(&self) {
        crate::ui::settings::dialogs::restart::RestartDialog::present(self.settings(), self);
    }

    /// The prominent action does whatever its condition is: continue the setup
    /// that is not finished, or finish the update that is waiting.
    fn run_prominent(&self) {
        if self.in_settings() {
            self.present_restart();
            return;
        }

        let settings = self.settings();
        let action = crate::models::home::toolbar_action(&settings.state());
        match action {
            Some(ToolbarAction::FinishUpdating) => self.present_restart(),
            Some(ToolbarAction::ContinueSetup) => self.show_setup(),
            None => {}
        }
    }

    /// The Setup assistant: a presentation of this window with the sidebar
    /// hidden, its own bottom bar, and its own title widget in the header.
    ///
    /// It opens at the screen the daemon's readiness says is still owed, which
    /// is what Home's Continue setup means.
    pub fn show_setup(&self) {
        if self.in_setup() {
            return;
        }
        if self.in_settings() {
            self.leave_settings();
        }

        let imp = self.imp();
        imp.in_setup.set(true);

        let assistant = self.assistant();
        let window = self.clone();
        assistant.on_leave(move || window.leave_setup());

        // The sidebar is hidden while the assistant is showing: there is one
        // task on the screen and nothing to navigate to beside it.
        self.set_sidebar_shown(false);

        imp.content_stack.set_visible_child_name(SETUP);
        imp.title.set_title(&copy::text(Key::PageSetup));
        self.set_title(Some(&copy::text(Key::PageSetup)));

        if let Some(header) = imp.header.borrow().as_ref() {
            header.set_title_widget(Some(&assistant.title_widget()));
        }
        if let Some(toolbar) = imp.toolbar.borrow().as_ref() {
            toolbar.add_bottom_bar(&assistant.bottom_bar());
        }
        self.set_default_widget(Some(&assistant.default_widget()));

        self.update_page_toolbar(SETUP);
        self.update_back_action();
        self.update_prominent();
        assistant.resume();
    }

    /// Leave the assistant, however it ended. Nothing is marked complete by
    /// leaving: the daemon's own readiness is what Home reads afterwards.
    pub fn leave_setup(&self) {
        let imp = self.imp();
        if !imp.in_setup.replace(false) {
            return;
        }

        let assistant = self.assistant();
        if let Some(toolbar) = imp.toolbar.borrow().as_ref() {
            toolbar.remove(&assistant.bottom_bar());
        }
        if let Some(header) = imp.header.borrow().as_ref() {
            header.set_title_widget(Some(&imp.title));
        }
        self.set_default_widget(None::<&gtk::Widget>);
        self.set_title(Some(&copy::text(Key::ProductName)));

        self.set_sidebar_shown(true);
        self.show_page(DESTINATIONS[0]);
        self.reveal_content();

        // What the assistant did or did not do is a question for the daemon.
        let settings = self.settings();
        spawn(async move {
            settings.refresh_setup().await;
            settings.refresh_overview().await;
        });
    }

    /// Whether the Setup assistant is showing.
    pub fn in_setup(&self) -> bool {
        self.imp().in_setup.get()
    }

    /// The assistant, for the window's own routing and the tests that walk it.
    pub fn assistant(&self) -> Rc<Assistant> {
        let models = self.imp().models.borrow();
        Rc::clone(&models.as_ref().expect("the window has its models").setup)
    }

    // ---- Presentations --------------------------------------------------

    fn leave_settings_then_show(&self, destination: Destination) {
        if self.in_setup() {
            self.leave_setup();
        }
        if self.in_settings() {
            self.leave_settings();
        }
        self.show_page(destination);
        self.reveal_content();
    }

    fn show_page(&self, destination: Destination) {
        let imp = self.imp();
        imp.content_stack.set_visible_child_name(destination.page);
        imp.title.set_title(&copy::text(destination.title));
        self.select_sidebar_row(destination.page);
        self.update_back_action();
        self.update_page_toolbar(destination.page);
        self.update_prominent();
    }

    /// Show Recovery, which is a state rather than a destination.
    pub fn show_recovery(&self) {
        if self.in_setup() {
            self.leave_setup();
        }
        if self.in_settings() {
            self.leave_settings();
        }

        let imp = self.imp();
        imp.content_stack.set_visible_child_name(RECOVERY);
        imp.title.set_title(&copy::text(Key::PageRecovery));
        self.update_page_toolbar(RECOVERY);
        self.update_back_action();
        self.update_prominent();
        self.reveal_content();
    }

    fn select_sidebar_row(&self, page: &str) {
        let position = DESTINATIONS
            .iter()
            .chain(std::iter::once(&SETTINGS))
            .position(|destination| destination.page == page);

        let Some(position) = position else {
            return;
        };
        let index = i32::try_from(position).unwrap_or(0);
        if let Some(row) = self.imp().sidebar_list.row_at_index(index) {
            self.imp().sidebar_list.select_row(Some(&row));
        }
    }

    /// Entering Settings remembers where it came from, so leaving can put back
    /// the page, its scroll position and its focus rather than dropping the
    /// person at the top of Home.
    fn enter_settings(&self) {
        if self.in_settings() {
            return;
        }

        let imp = self.imp();
        let page = self.current_page();
        imp.previous.replace(Some(Previous {
            scroll: self.scroll_of(&page),
            focus: weak_focus(self),
            page,
        }));

        imp.in_settings.set(true);
        imp.sidebar_stack.set_visible_child_name("settings");
        self.show_page(SETTINGS);
        self.update_title();
        self.presentation().load();
        self.reveal_content();
    }

    fn leave_settings(&self) {
        let imp = self.imp();
        if !imp.in_settings.get() {
            return;
        }

        imp.in_settings.set(false);
        imp.sidebar_stack.set_visible_child_name("app");

        let Some(previous) = imp.previous.replace(None) else {
            self.show_page(DESTINATIONS[0]);
            return;
        };

        if let Some(destination) = destination_named(&previous.page) {
            self.show_page(destination);
        }
        self.restore_scroll(&previous.page, previous.scroll);
        if let Some(widget) = previous.focus.upgrade() {
            widget.grab_focus();
        }
    }

    /// Back, in the order the redlines fix: leave Settings if it is showing,
    /// otherwise leave Recovery, otherwise reveal the sidebar while navigation
    /// is collapsed.
    fn go_back(&self) {
        if self.in_setup() {
            // The assistant's bottom bar owns the leading control, and this is
            // the same control: one action map, two ways to reach it.
            self.assistant().back();
            return;
        }
        if self.in_settings() {
            // A pane with pages of its own goes back one page first: leaving
            // Settings from a provider's page would skip the pane it is in.
            if self.presentation().pop() {
                return;
            }
            self.leave_settings();
            return;
        }
        if self.current_page() == RECOVERY {
            self.leave_settings_then_show(DESTINATIONS[0]);
            return;
        }
        if self.imp().split.is_collapsed() {
            self.imp().split.set_show_content(false);
        }
    }

    fn update_back_action(&self) {
        let split = &self.imp().split;
        let available = self.in_settings()
            || self.current_page() == RECOVERY
            || (split.is_collapsed() && split.shows_content());

        if self.in_setup() {
            // The assistant draws its own leading control in its bottom bar, so
            // the header carries none. Escape still reaches it through the one
            // action, and only on a screen that answers Escape at all.
            self.imp().back.set_visible(false);
            self.enable_back(self.assistant().answers_escape());
            return;
        }

        self.imp().back.set_visible(available);
        self.enable_back(available);
    }

    fn enable_back(&self, enabled: bool) {
        if let Some(action) = self.lookup_action("back") {
            if let Ok(action) = action.downcast::<gio::SimpleAction>() {
                action.set_enabled(enabled);
            }
        }
    }

    /// The title is the page's, and inside Settings it is the pane's.
    fn update_title(&self) {
        if !self.in_settings() {
            return;
        }
        self.imp().title.set_title(&self.presentation().title());
    }

    /// The one prominent action, and only while its condition holds.
    ///
    /// Home carries the setup and update actions; Settings carries the restart.
    /// No other surface carries a prominent action at all.
    /// Write the one prominent action's word, and let it shorten.
    ///
    /// The header bar spans the whole window, so a label in it that cannot
    /// shrink is a floor under the window's own minimum width. The shortening
    /// is applied here rather than where the button is built, because a button
    /// with no label yet has no label to shorten. The whole word is the
    /// accessible name at every width, and the reasons a restart is owed are
    /// the description.
    fn write_prominent(&self, word: &str, reasons: Option<&str>) {
        let button = &self.imp().prominent;
        button.set_label(word);
        crate::ui::shorten(button);
        button.set_tooltip_text(Some(reasons.unwrap_or(word)));
        button.update_property(&[gtk::accessible::Property::Label(word)]);
        if let Some(reasons) = reasons {
            button.update_property(&[gtk::accessible::Property::Description(reasons)]);
        }
    }

    fn update_prominent(&self) {
        let imp = self.imp();
        let settings = self.settings();
        let page = self.current_page();

        // The assistant carries its own actions in its own bar, and one task on
        // the screen has one prominent action.
        if self.in_setup() {
            imp.prominent.set_visible(false);
            return;
        }

        if page == "settings" {
            let pending = restart_pending(&settings);
            imp.prominent.set_visible(pending);
            if pending {
                let reasons = restart_reasons(&settings);
                self.write_prominent(&copy::text(Key::MenuRestart), Some(&reasons));
            }
            return;
        }

        let action = if page == "home" {
            crate::models::home::toolbar_action(&settings.state())
        } else {
            None
        };

        match action {
            Some(action) => {
                self.write_prominent(&copy::text(action.key()), None);
                imp.prominent.set_visible(true);
            }
            None => imp.prominent.set_visible(false),
        }
    }

    /// Say once, while this application is open, that the number of things
    /// needing someone has changed.
    ///
    /// The state the window opened on raises nothing: it was already true. The
    /// notification is withdrawn when nothing is left and when the window
    /// closes, so nothing this process raised outlives it.
    fn raise_attention_notice(&self) {
        let count = self.home_page().snapshot().attention.len();
        let Some(application) = self.application() else {
            return;
        };

        match self.imp().notice.observe(count) {
            Notice::Post => DesktopSession::notify(
                &application,
                AttentionNotice::ID,
                &copy::text(AttentionNotice::TITLE),
                &copy::text(AttentionNotice::BODY),
                Some("app.home"),
            ),
            Notice::Withdraw => DesktopSession::withdraw(&application, AttentionNotice::ID),
            Notice::Nothing => {}
        }
    }

    /// Take back anything this window raised. A notification that outlived the
    /// application would point at a window that is not there.
    fn withdraw_notices(&self) {
        if let Some(application) = self.application() {
            DesktopSession::withdraw(&application, AttentionNotice::ID);
        }
    }

    fn home_page(&self) -> Rc<HomePage> {
        let models = self.imp().models.borrow();
        Rc::clone(&models.as_ref().expect("the window has its models").home)
    }

    /// Hand the header bar the controls that belong to the page being shown.
    fn update_page_toolbar(&self, page: &str) {
        let imp = self.imp();

        clear_children(&imp.page_start);
        clear_children(&imp.page_end);

        let toolbar: PageToolbar = {
            let models = imp.models.borrow();
            match (models.as_ref(), page) {
                (Some(models), "doctor") => models.doctor.toolbar(),
                (Some(models), "logs") => models.logs.toolbar(),
                _ => PageToolbar::default(),
            }
        };

        for widget in toolbar.start {
            imp.page_start.append(&widget);
        }
        for widget in toolbar.end {
            imp.page_end.append(&widget);
        }

        if let Some(action) = self.lookup_action("search") {
            if let Ok(action) = action.downcast::<gio::SimpleAction>() {
                action.set_enabled(page == "logs" || page == "settings");
            }
        }
    }

    fn reveal_content(&self) {
        if self.imp().split.is_collapsed() {
            self.imp().split.set_show_content(true);
        }
    }

    /// The outer gutter: 24 with the sidebar shown, 18 while collapsed.
    fn update_gutter(&self) {
        let Some(toolbar) = self.imp().toolbar.borrow().clone() else {
            return;
        };

        if self.imp().split.is_collapsed() {
            toolbar.add_css_class("fermix-collapsed");
        } else {
            toolbar.remove_css_class("fermix-collapsed");
        }
    }

    /// Show or hide the sidebar. Hiding collapses the split view and keeps the
    /// detail in front; showing puts the sidebar back and undoes only a
    /// collapse this action itself asked for, so the breakpoint stays in charge
    /// of the responsive one.
    fn set_sidebar_shown(&self, shown: bool) {
        let imp = self.imp();

        if shown {
            imp.split.set_show_content(false);
            if imp.forced_collapsed.replace(false) {
                imp.split.set_collapsed(false);
            }
            return;
        }

        if !imp.split.is_collapsed() {
            imp.forced_collapsed.set(true);
            imp.split.set_collapsed(true);
        }
        imp.split.set_show_content(true);
    }

    // ---- Geometry -------------------------------------------------------

    fn restore_geometry(&self, state: WindowState) {
        self.set_default_size(state.width, state.height);
        if state.maximized {
            self.maximize();
        }
        self.set_sidebar_shown(state.sidebar_visible);
    }

    fn remember_geometry(&self) {
        let (width, height) = self.default_size();
        let state = WindowState {
            width,
            height,
            maximized: self.is_maximized(),
            sidebar_visible: self.sidebar_shown(),
            last_pane: pane::slug(self.settings().pane()).map(str::to_string),
        };

        if let Err(error) = state::save(&state) {
            glib::g_warning!("fermix-desktop", "the window state was not saved: {error}");
        }
    }

    fn scroll_of(&self, page: &str) -> f64 {
        self.imp()
            .scrollers
            .borrow()
            .get(page)
            .map(|scroller| scroller.vadjustment().value())
            .unwrap_or_default()
    }

    fn restore_scroll(&self, page: &str, value: f64) {
        if let Some(scroller) = self.imp().scrollers.borrow().get(page) {
            scroller.vadjustment().set_value(value);
        }
    }

    fn application_object(&self) -> FermixApplication {
        self.application()
            .and_then(|application| application.downcast::<FermixApplication>().ok())
            .expect("the window is built by the application that owns it")
    }
}

/// Whether a restart is owed: the daemon says so, or the installed engine and
/// the running one are known to differ.
fn restart_pending(settings: &SettingsModel) -> bool {
    let state = settings.state();
    state.restart.required
        || state
            .service
            .as_ref()
            .map(|status| status.alignment == crate::service::types::Alignment::PendingRestart)
            .unwrap_or(false)
}

/// The daemon's own reasons, as the accessible description of the one action
/// that answers them. Never a reason this application composed.
fn restart_reasons(settings: &SettingsModel) -> String {
    let state = settings.state();
    let reasons: Vec<String> = state
        .restart
        .reasons
        .iter()
        .map(|reason| reason.sentence.clone())
        .collect();

    if reasons.is_empty() {
        return copy::text(Key::RestartDialogBody);
    }
    reasons.join(" ")
}

fn destination_named(page: &str) -> Option<Destination> {
    DESTINATIONS
        .iter()
        .chain(std::iter::once(&SETTINGS))
        .find(|destination| destination.page == page)
        .copied()
}

/// A sidebar row activates its action and nothing else. The row is not also
/// wired to a signal handler: one gesture that reaches the same surface through
/// two paths is two paths to keep in step, and the action map is the one that
/// the menu, the accelerators and the shortcuts dialog already use.
fn sidebar_row(destination: Destination) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(copy::text(destination.title))
        .activatable(true)
        .action_name(destination.action)
        .build();

    row.add_prefix(&gtk::Image::from_icon_name(destination.icon));
    row
}

fn clear_children(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

fn count_children(container: &gtk::Box) -> usize {
    let mut count = 0;
    let mut child = container.first_child();
    while let Some(widget) = child {
        count += 1;
        child = widget.next_sibling();
    }
    count
}

fn weak_focus(window: &FermixWindow) -> glib::WeakRef<gtk::Widget> {
    let weak = glib::WeakRef::new();
    if let Some(widget) = gtk::prelude::GtkWindowExt::focus(window) {
        weak.set(Some(&widget));
    }
    weak
}
