//! The Settings presentation.
//!
//! Not a second window and not a second split view: the same one, with the
//! sidebar showing the pane list and the detail showing the pane. The seven
//! descriptor panes render through one form; the six hand-built panes arrive
//! with their own slice and say so until then.
//!
//! Two things live here because they belong to the presentation rather than to
//! any pane: the banner that reports a settings file changed underneath us, and
//! the one Restart action, which is in the header bar and does not repeat
//! itself per pane.

pub mod channels;
pub mod computer;
pub mod descriptor_form;
pub mod descriptor_row;
pub mod dialogs;
pub mod integrations;
pub mod meetings;
pub mod pane_list;
pub mod permissions;
pub mod providers;
pub mod voice;

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::management::types::SettingsPane;
use crate::management::vocabulary::ConfigCondition;
use crate::models::ledger::PermissionLedger;
use crate::models::pane::{self, PaneKind};
use crate::models::{spawn, Change, SettingsModel};

use channels::ChannelsPane;
use computer::ComputerPane;
use descriptor_form::DescriptorForm;
use integrations::IntegrationsPane;
use meetings::MeetingsPane;
use pane_list::PaneList;
use permissions::PermissionsPane;
use providers::ProvidersPane;
use voice::VoicePane;

/// What one pane's content is.
///
/// Seven panes are the daemon's descriptor through one form, one is that form
/// under the statements the platform owes a person, and five are hand-built
/// because their flows are interaction rather than values.
enum Content {
    Descriptor(Rc<DescriptorForm>),
    Voice(Rc<VoicePane>),
    Providers(Rc<ProvidersPane>),
    Channels(Rc<ChannelsPane>),
    Integrations(Rc<IntegrationsPane>),
    Meetings(Rc<MeetingsPane>),
    Computer(Rc<ComputerPane>),
    Permissions(Rc<PermissionsPane>),
}

impl Content {
    fn widget(&self) -> gtk::Widget {
        match self {
            Content::Descriptor(form) => form.widget(),
            Content::Voice(pane) => pane.widget(),
            Content::Providers(pane) => pane.widget(),
            Content::Channels(pane) => pane.widget(),
            Content::Integrations(pane) => pane.widget(),
            Content::Meetings(pane) => pane.widget(),
            Content::Computer(pane) => pane.widget(),
            Content::Permissions(pane) => pane.widget(),
        }
    }

    /// Read what this pane draws. Called every time it is shown.
    fn load(&self) {
        match self {
            Content::Descriptor(form) => form.load(),
            Content::Voice(pane) => pane.load(),
            Content::Providers(pane) => pane.load(),
            Content::Channels(pane) => pane.load(),
            Content::Integrations(pane) => pane.load(),
            Content::Meetings(pane) => pane.load(),
            Content::Computer(pane) => pane.load(),
            Content::Permissions(pane) => pane.load(),
        }
    }

    /// Go back one page inside this pane, where it has pages of its own.
    fn pop(&self) -> bool {
        match self {
            Content::Providers(pane) => pane.pop(),
            Content::Channels(pane) => pane.pop(),
            Content::Integrations(pane) => pane.pop(),
            _ => false,
        }
    }

    /// The title of the page showing, where it is not the pane's own.
    fn sub_page_title(&self) -> Option<String> {
        match self {
            Content::Providers(pane) => pane.sub_page_title(),
            Content::Channels(pane) => pane.sub_page_title(),
            Content::Integrations(pane) => pane.sub_page_title(),
            _ => None,
        }
    }

    /// Whether this pane has a search of its own, which is what the window's
    /// search action reaches while the pane is showing.
    fn focus_search(&self) -> bool {
        match self {
            Content::Integrations(pane) => {
                pane.focus_search();
                true
            }
            _ => false,
        }
    }
}

/// The Settings presentation: a sidebar, a detail, and the two things that sit
/// above every pane.
pub struct SettingsPresentation {
    sidebar: Rc<PaneList>,
    detail: gtk::Widget,
    stack: gtk::Stack,
    banner: adw::Banner,
    settings: Rc<SettingsModel>,
    /// The one ledger, which Permissions, Voice and Computer all read.
    ledger: Rc<PermissionLedger>,
    panes: RefCell<BTreeMap<SettingsPane, Content>>,
    forms: RefCell<BTreeMap<SettingsPane, Rc<DescriptorForm>>>,
    /// Called when the page showing inside a pane changes, so the window's
    /// title follows a sub-page.
    on_title: RefCell<Option<Box<dyn Fn()>>>,
    /// Called when a surface asks for Home, which is where the background
    /// service is switched.
    on_home: RefCell<Option<Box<dyn Fn()>>>,
    /// Called when the settings file cannot be read at all, which is a state
    /// the window answers by showing Recovery rather than a pane.
    on_recovery: RefCell<Option<Box<dyn Fn()>>>,
    /// Whether the unreadable state has already been routed, so it is not
    /// routed again on every refresh.
    routed: Cell<bool>,
}

impl SettingsPresentation {
    /// Build the presentation over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let sidebar = PaneList::new(Rc::clone(&settings));

        let banner = adw::Banner::new("");
        banner.set_revealed(false);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vexpand(true)
            // The stack asks for the pane being shown rather than for the
            // widest of the thirteen: a homogeneous stack would hold the whole
            // window open at the width of whichever pane needs the most.
            .hhomogeneous(false)
            .vhomogeneous(false)
            .build();
        crate::motion::follow(&stack, crate::motion::Transition::Crossfade);

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        column.append(&banner);
        column.append(&stack);

        let presentation = Rc::new(Self {
            sidebar,
            detail: column.upcast(),
            stack,
            banner,
            ledger: PermissionLedger::new(Rc::clone(&settings)),
            settings,
            panes: RefCell::new(BTreeMap::new()),
            forms: RefCell::new(BTreeMap::new()),
            on_title: RefCell::new(None),
            on_home: RefCell::new(None),
            on_recovery: RefCell::new(None),
            routed: Cell::new(false),
        });

        presentation.build_panes();
        presentation.connect();
        presentation.draw();
        presentation
    }

    /// The sidebar half.
    pub fn sidebar(&self) -> gtk::Widget {
        self.sidebar.widget()
    }

    /// The detail half.
    pub fn detail(&self) -> gtk::Widget {
        self.detail.clone()
    }

    /// The pane list, for the window's search action.
    pub fn pane_list(&self) -> Rc<PaneList> {
        Rc::clone(&self.sidebar)
    }

    /// The title of what is being shown: the pane, or the page inside it.
    pub fn title(&self) -> String {
        let pane = self.settings.pane();
        if let Some(title) = self
            .panes
            .borrow()
            .get(&pane)
            .and_then(Content::sub_page_title)
        {
            return title;
        }

        pane::row(pane)
            .map(|row| copy::text(row.title))
            .unwrap_or_else(|| copy::text(Key::PageSettings))
    }

    /// Go back one page inside the pane being shown, and say whether there was
    /// one. The window asks this before leaving Settings altogether.
    pub fn pop(&self) -> bool {
        let pane = self.settings.pane();
        let popped = self
            .panes
            .borrow()
            .get(&pane)
            .map(Content::pop)
            .unwrap_or(false);
        if popped {
            self.notify_title();
        }
        popped
    }

    /// Put the focus in the search of the pane being shown, where it has one.
    /// Answers false where the pane list's own search is the one to use.
    pub fn focus_pane_search(&self) -> bool {
        self.panes
            .borrow()
            .get(&self.settings.pane())
            .map(Content::focus_search)
            .unwrap_or(false)
    }

    /// What to do when the title of what is showing changes.
    pub fn on_title_changed(&self, changed: impl Fn() + 'static) {
        self.on_title.replace(Some(Box::new(changed)));
    }

    /// What to do when a pane sends someone to Home.
    pub fn on_home(&self, open: impl Fn() + 'static) {
        self.on_home.replace(Some(Box::new(open)));
    }

    /// Send someone to Home, which is where the background service is
    /// switched and where the ledger's row for it points.
    fn open_home(&self) {
        let open = self.on_home.borrow();
        if let Some(open) = open.as_ref() {
            open();
        }
    }

    fn notify_title(&self) {
        if let Some(changed) = self.on_title.borrow().as_ref() {
            changed();
        }
    }

    /// What to do when the settings file cannot be read.
    pub fn on_recovery(&self, route: impl Fn() + 'static) {
        self.on_recovery.replace(Some(Box::new(route)));
    }

    /// Read everything this presentation draws.
    pub fn load(&self) {
        let settings = Rc::clone(&self.settings);
        let pane = self.settings.pane();
        spawn(async move {
            settings.refresh_sections().await;
            settings.refresh_pane(pane).await;
        });
    }

    fn build_panes(self: &Rc<Self>) {
        for row in pane::PANES {
            let Some(name) = pane::slug(row.pane) else {
                continue;
            };

            let content = self.build_pane(row.pane, row.kind);
            self.stack.add_named(&content.widget(), Some(name));
            self.panes.borrow_mut().insert(row.pane, content);
        }
    }

    /// One pane's content, by what it is.
    fn build_pane(self: &Rc<Self>, pane: SettingsPane, kind: PaneKind) -> Content {
        let settings = Rc::clone(&self.settings);

        match (kind, pane) {
            (PaneKind::HandBuilt, SettingsPane::Providers) => {
                let built = ProvidersPane::new(settings);
                self.follow_pages(&built);
                Content::Providers(built)
            }
            (PaneKind::HandBuilt, SettingsPane::Channels) => {
                let built = ChannelsPane::new(settings);
                let presentation = Rc::downgrade(self);
                built.on_page_changed(move || {
                    if let Some(presentation) = presentation.upgrade() {
                        presentation.notify_title();
                    }
                });
                Content::Channels(built)
            }
            (PaneKind::HandBuilt, SettingsPane::Integrations) => {
                let built = IntegrationsPane::new(settings);
                let presentation = Rc::downgrade(self);
                built.on_page_changed(move || {
                    if let Some(presentation) = presentation.upgrade() {
                        presentation.notify_title();
                    }
                });
                let model = Rc::clone(&self.settings);
                built.on_open_pane(move |pane| model.select_pane(pane));
                Content::Integrations(built)
            }
            (PaneKind::HandBuilt, SettingsPane::Meetings) => {
                Content::Meetings(MeetingsPane::new(settings))
            }
            (PaneKind::HandBuilt, SettingsPane::Computer) => {
                Content::Computer(ComputerPane::new(settings, Rc::clone(&self.ledger)))
            }
            (PaneKind::HandBuilt, SettingsPane::Permissions) => {
                let built = PermissionsPane::new(Rc::clone(&self.ledger));
                let presentation = Rc::downgrade(self);
                built.on_open_home(move || {
                    if let Some(presentation) = presentation.upgrade() {
                        presentation.open_home();
                    }
                });
                Content::Permissions(built)
            }
            (PaneKind::StatedDescriptor, _) => {
                let built = VoicePane::new(settings, Rc::clone(&self.ledger));
                self.forms.borrow_mut().insert(pane, built.form());
                Content::Voice(built)
            }
            // Every other pane is the daemon's descriptor through one form.
            (_, _) => {
                let form = DescriptorForm::new(settings, pane);
                self.forms.borrow_mut().insert(pane, Rc::clone(&form));
                Content::Descriptor(form)
            }
        }
    }

    fn follow_pages(self: &Rc<Self>, pane: &Rc<ProvidersPane>) {
        let presentation = Rc::downgrade(self);
        pane.on_page_changed(move || {
            if let Some(presentation) = presentation.upgrade() {
                presentation.notify_title();
            }
        });
    }

    fn connect(self: &Rc<Self>) {
        let presentation = Rc::downgrade(self);
        self.settings.observe(move |change| {
            let Some(presentation) = presentation.upgrade() else {
                return;
            };
            match change {
                Change::Pane => presentation.draw(),
                Change::Setup | Change::Sections => presentation.draw_banner(),
                _ => {}
            }
        });
    }

    /// Show the selected pane, and read it if it has not been read.
    pub fn draw(self: &Rc<Self>) {
        let pane = self.settings.pane();
        if let Some(name) = pane::slug(pane) {
            self.stack.set_visible_child_name(name);
        }

        if let Some(content) = self.panes.borrow().get(&pane) {
            content.load();
        }

        self.draw_banner();
    }

    /// The banner: one action on a file that changed outside Fermix, and none
    /// at all on a file that cannot be read, because the reload would re-run
    /// the read that failed.
    fn draw_banner(self: &Rc<Self>) {
        let (config, sentence) = {
            let state = self.settings.state();
            (
                state.config,
                state.unreadable.as_ref().map(|reason| reason.text.clone()),
            )
        };

        match config {
            ConfigCondition::ExternalChange => {
                self.banner
                    .set_title(&copy::text(Key::BannerExternalChangeTitle));
                self.banner
                    .set_button_label(Some(&copy::text(Key::BannerReloadFromDisk)));
                self.banner.set_revealed(true);
                self.routed.set(false);
            }
            ConfigCondition::Unreadable => {
                self.banner
                    .set_title(sentence.as_deref().unwrap_or_default());
                self.banner.set_button_label(None);
                self.banner.set_revealed(true);

                if !self.routed.replace(true) {
                    if let Some(route) = self.on_recovery.borrow().as_ref() {
                        route();
                    }
                }
            }
            _ => {
                self.banner.set_revealed(false);
                self.routed.set(false);
            }
        }
    }

    /// Wire the banner's one action. Done once, by the window, because the
    /// banner outlives every pane.
    pub fn connect_banner(self: &Rc<Self>) {
        let settings = Rc::clone(&self.settings);
        self.banner.connect_button_clicked(move |_| {
            let settings = Rc::clone(&settings);
            spawn(async move {
                settings.reload().await;
            });
        });
    }
}
