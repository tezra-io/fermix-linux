//! The Setup assistant.
//!
//! A presentation of the one window rather than a second window: the sidebar is
//! hidden, an `AdwNavigationView` holds the screens, and a stable bottom bar
//! carries the leading control on the left and the one suggested action on the
//! right. Content scrolls above the bar and the bar never moves.
//!
//! The window owns the header bar and the action map; the assistant hands it a
//! title widget and a bottom bar while it is showing, and takes them back when
//! it leaves. Enter reaches the suggested action through the window's default
//! widget, and Escape reaches the leading control through the one `win.back`
//! action every other surface uses.

pub mod about_you;
pub mod applying;
pub mod boot_failed;
pub mod connect_ai;
pub mod ready;
pub mod starting;
pub mod welcome;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::metrics;
use crate::models::onboarding::{Leading, OnboardingModel, Primary, Snapshot, Stage};
use crate::models::{spawn, Change, SettingsModel};

/// Every screen draws a snapshot and nothing else.
pub trait Screen {
    /// The widget the page holds.
    fn widget(&self) -> gtk::Widget;
    /// Draw the state the assistant is in.
    ///
    /// The receiver is the screen's own handle rather than a borrow, because a
    /// screen that builds a row builds the handler on it too, and a handler
    /// holds the screen it belongs to.
    fn draw(self: Rc<Self>, snapshot: &Snapshot);
    /// This screen is the one showing. Where a screen reads something of its
    /// own, this is where it asks.
    fn shown(self: Rc<Self>) {}
}

/// The Setup assistant, as a presentation of the window.
pub struct Assistant {
    view: adw::NavigationView,
    bar: gtk::ActionBar,
    title: gtk::Box,
    window_title: adw::WindowTitle,
    dots: Vec<gtk::Image>,
    leading: gtk::Button,
    primary: gtk::Button,
    model: Rc<OnboardingModel>,
    settings: Rc<SettingsModel>,
    screens: RefCell<BTreeMap<&'static str, Rc<dyn Screen>>>,
}

impl Assistant {
    /// Build the assistant over the one settings model.
    pub fn new(settings: Rc<SettingsModel>) -> Rc<Self> {
        let model = OnboardingModel::new(Rc::clone(&settings));
        let (title, window_title, dots) = header_title();

        let assistant = Rc::new(Self {
            view: adw::NavigationView::new(),
            bar: gtk::ActionBar::new(),
            title,
            window_title,
            dots,
            leading: gtk::Button::builder().visible(false).build(),
            primary: suggested_button(),
            model: Rc::clone(&model),
            settings: Rc::clone(&settings),
            screens: RefCell::new(BTreeMap::new()),
        });

        assistant.build(settings);
        assistant.connect();
        assistant.draw();
        assistant
    }

    /// The widget the window's content stack holds.
    pub fn widget(&self) -> gtk::Widget {
        self.view.clone().upcast()
    }

    /// The bar the window's toolbar view holds while the assistant is showing.
    pub fn bottom_bar(&self) -> gtk::ActionBar {
        self.bar.clone()
    }

    /// The title widget the header bar carries while the assistant is showing.
    pub fn title_widget(&self) -> gtk::Widget {
        self.title.clone().upcast()
    }

    /// The button Enter reaches, which is the window's default widget while the
    /// assistant is showing.
    pub fn default_widget(&self) -> gtk::Button {
        self.primary.clone()
    }

    /// Every screen the assistant holds, named by the stage it draws, for the
    /// gate that measures what each one asks for. All seven are built with the
    /// assistant, so this is the whole of it whichever screen is showing.
    pub fn screens(&self) -> Vec<(&'static str, gtk::Widget)> {
        self.screens
            .borrow()
            .iter()
            .map(|(slug, screen)| (*slug, screen.widget()))
            .collect()
    }

    /// The assistant's own model, for the window and the tests that walk it.
    pub fn model(&self) -> Rc<OnboardingModel> {
        Rc::clone(&self.model)
    }

    /// Whether Escape reaches the leading control on the screen showing.
    pub fn answers_escape(&self) -> bool {
        self.model
            .snapshot()
            .leading
            .is_some_and(Leading::answers_escape)
    }

    /// What Escape and the window's back control run.
    pub fn back(&self) {
        self.model.back();
    }

    /// Open at the screen the daemon's readiness says is the first one owed.
    pub fn resume(&self) {
        self.model.resume();
    }

    /// Tell me when the assistant is done with the window.
    pub fn on_leave(&self, leave: impl Fn() + 'static) {
        self.model.on_leave(leave);
    }

    fn build(self: &Rc<Self>, settings: Rc<SettingsModel>) {
        let screens: Vec<(Stage, Rc<dyn Screen>)> = vec![
            (Stage::Welcome, welcome::WelcomeScreen::new(&self.model)),
            (Stage::Starting, starting::StartingScreen::new()),
            (
                Stage::ConnectAi,
                connect_ai::ConnectAiScreen::new(Rc::clone(&settings), &self.model),
            ),
            (
                Stage::AboutYou,
                about_you::AboutYouScreen::new(Rc::clone(&settings), &self.model),
            ),
            (Stage::Applying, applying::ApplyingScreen::new(&self.model)),
            (
                Stage::Ready,
                ready::ReadyScreen::new(Rc::clone(&settings), &self.model),
            ),
            (
                Stage::BootFailed,
                boot_failed::BootFailedScreen::new(&self.model),
            ),
        ];

        for (stage, screen) in screens {
            let page = adw::NavigationPage::builder()
                .title(copy::text(stage.title()))
                .tag(stage.slug())
                .child(&screen.widget())
                .build();
            self.view.add(&page);
            self.screens.borrow_mut().insert(stage.slug(), screen);
        }

        self.build_bar();
    }

    /// The bar: the leading control, then the one suggested action. It is the
    /// same bar on every screen; what changes is which of its two controls a
    /// screen offers.
    fn build_bar(self: &Rc<Self>) {
        self.bar.pack_start(&self.leading);
        self.bar.pack_end(&self.primary);

        let assistant = Rc::clone(self);
        self.leading.connect_clicked(move |_| {
            let snapshot = assistant.model.snapshot();
            match snapshot.leading {
                Some(Leading::Cancel) => assistant.model.cancel(),
                Some(_) => assistant.model.back(),
                None => {}
            }
        });

        let assistant = Rc::clone(self);
        self.primary.connect_clicked(move |_| {
            if assistant.model.snapshot().primary == Some(Primary::Retry) {
                assistant.model.retry_applying();
                return;
            }
            assistant.model.advance();
        });
    }

    fn connect(self: &Rc<Self>) {
        let assistant = Rc::downgrade(self);
        self.model.observe(move || {
            if let Some(assistant) = assistant.upgrade() {
                assistant.draw();
            }
        });

        // A screen that draws the daemon's own answers redraws when they land,
        // whichever surface asked for them.
        let assistant = Rc::downgrade(self);
        self.settings.observe(move |change| {
            let Some(assistant) = assistant.upgrade() else {
                return;
            };
            if matches!(change, Change::Setup | Change::Daemon | Change::Detections) {
                assistant.draw();
            }
        });
    }

    /// Draw the screen showing: its page, the header title, the dots and the
    /// two controls.
    fn draw(&self) {
        let snapshot = self.model.snapshot();
        let stage = snapshot.stage;

        let screen = self.screens.borrow().get(stage.slug()).map(Rc::clone);

        if self.view.visible_page().and_then(|page| page.tag()) != Some(stage.slug().into()) {
            self.view.replace_with_tags(&[stage.slug()]);
            if let Some(screen) = screen.clone() {
                screen.shown();
            }
        }

        if let Some(screen) = screen {
            screen.draw(&snapshot);
        }

        self.draw_title(&snapshot);
        self.draw_bar(&snapshot);
    }

    /// The header carries the stage's title, except on the two screens that
    /// draw a heading of their own: a title in two places at once is the same
    /// words twice.
    fn draw_title(&self, snapshot: &Snapshot) {
        let title = match snapshot.stage {
            Stage::Welcome | Stage::Ready => String::new(),
            stage => copy::text(stage.title()),
        };
        self.window_title.set_title(&title);
        self.window_title.set_visible(!title.is_empty());

        let lit = snapshot.progress;
        for (index, dot) in self.dots.iter().enumerate() {
            dot.set_visible(lit.is_some());
            if Some(index) == lit {
                dot.remove_css_class("dim-label");
            } else {
                dot.add_css_class("dim-label");
            }
        }

        if let Some(lit) = lit {
            self.title
                .update_property(&[gtk::accessible::Property::Label(&copy::fill(
                    Key::SetupProgressAccessible,
                    &[
                        ("{step}", &(lit + 1).to_string()),
                        ("{total}", &Stage::PROGRESS_STEPS.to_string()),
                    ],
                ))]);
        }
    }

    fn draw_bar(&self, snapshot: &Snapshot) {
        match snapshot.leading {
            Some(leading) => {
                self.leading.set_label(&copy::text(leading.key()));
                self.leading.set_visible(true);
            }
            None => self.leading.set_visible(false),
        }

        match snapshot.primary {
            Some(primary) => {
                self.primary.set_label(&copy::text(primary.key()));
                self.primary.set_visible(true);
            }
            None => self.primary.set_visible(false),
        }
    }
}

/// The header's title widget: the stage's title with the four dots under it.
fn header_title() -> (gtk::Box, adw::WindowTitle, Vec<gtk::Image>) {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(metrics::SPACE_TIGHT)
        .valign(gtk::Align::Center)
        .build();

    let title = adw::WindowTitle::new("", "");
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(metrics::SPACE_TIGHT)
        .halign(gtk::Align::Center)
        .build();

    let dots: Vec<gtk::Image> = (0..Stage::PROGRESS_STEPS).map(|_| dot()).collect();
    for dot in &dots {
        row.append(dot);
    }

    column.append(&title);
    column.append(&row);
    (column, title, dots)
}

/// One progress dot.
///
/// The one glyph this application ships that the platform theme has no name
/// for: `media-record-symbolic` is a recording mark and draws red, and the
/// toolkit's own dot glyphs are squares at this size. It lives in the
/// application's own icon theme path, so it resolves on every desktop and is
/// recoloured by the toolkit like any other symbolic icon.
///
/// The lit one is the toolkit's ordinary foreground and the rest are dimmed,
/// which is the one distinction the toolkit itself makes between a thing in
/// force and a thing that is not.
fn dot() -> gtk::Image {
    let image = gtk::Image::from_icon_name("fermix-progress-symbolic");
    image.set_pixel_size(metrics::PROGRESS_DOT);
    image
}

fn suggested_button() -> gtk::Button {
    let button = gtk::Button::builder().visible(false).build();
    button.add_css_class("suggested-action");
    button
}

/// The column every assistant screen arranges itself in.
pub fn screen_column() -> gtk::Box {
    let column = crate::ui::column();
    column.add_css_class("fermix-gutter");
    column
}

/// One screen's widget: its column, clamped and scrolling above the bar.
pub fn screen(child: &impl IsA<gtk::Widget>) -> gtk::Widget {
    crate::ui::scrolled(&crate::ui::clamp(child)).upcast()
}

/// Run one future that belongs to a screen.
pub fn run(future: impl std::future::Future<Output = ()> + 'static) {
    spawn(future);
}
