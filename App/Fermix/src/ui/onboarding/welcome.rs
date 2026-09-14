//! Welcome.
//!
//! The mascot, the wordmark, one title and one sentence, and the one secondary
//! link that adopts a Fermix folder this computer already has. The primary
//! action is the bar's, so this screen carries no button of its own.
//!
//! The link picks a directory and holds it in memory for the next screen. This
//! application validates nothing about it and parses nothing inside it: the
//! home is an argument to the command line's own install, and the command line
//! owns every rule about it (M38 section 4.7).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk4 as gtk;
use gtk4::gio;
use gtk4::glib;
use libadwaita as adw;

use crate::copy::{self, Key};
use crate::metrics;
use crate::models::onboarding::{OnboardingModel, Snapshot};

use super::Screen;

/// How tall the mascot draws. It is artwork rather than an icon, so it has a
/// size of its own rather than the toolkit's.
const MASCOT_HEIGHT: i32 = 160;
/// How tall the wordmark draws beside it.
const WORDMARK_HEIGHT: i32 = 40;

/// The Welcome screen.
pub struct WelcomeScreen {
    root: gtk::Widget,
    chosen: gtk::Label,
}

impl WelcomeScreen {
    /// Build it over the assistant's model.
    pub fn new(model: &Rc<OnboardingModel>) -> Rc<Self> {
        let column = super::screen_column();
        column.set_valign(gtk::Align::Center);

        column.append(&mascot());
        column.append(&wordmark());
        column.append(&title());
        column.append(&sentence());

        let chosen = crate::ui::caption("");
        chosen.set_visible(false);
        chosen.set_halign(gtk::Align::Center);

        column.append(&choose_home(model, &chosen));
        column.append(&chosen);

        Rc::new(Self {
            root: super::screen(&column),
            chosen,
        })
    }
}

impl Screen for WelcomeScreen {
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }

    /// The only thing that moves on this screen is whether a home was picked.
    fn draw(self: Rc<Self>, snapshot: &Snapshot) {
        match snapshot.home.as_ref() {
            Some(home) => {
                self.chosen.set_label(&home.to_string_lossy());
                self.chosen.set_visible(true);
            }
            None => self.chosen.set_visible(false),
        }
    }
}

fn mascot() -> gtk::Picture {
    let picture = gtk::Picture::for_resource(&format!(
        "{}/brand/fermix-mascot.png",
        crate::app::RESOURCE_PREFIX
    ));
    picture.set_content_fit(gtk::ContentFit::ScaleDown);
    picture.set_height_request(MASCOT_HEIGHT);
    picture.set_can_shrink(true);
    picture.set_halign(gtk::Align::Center);
    picture
}

/// The wordmark, drawn in the colour its own file asks for.
///
/// Two inks: the letterform says `currentColor`, which is the author asking for
/// the surrounding text colour, and the two dots carry the brand's own blue. A
/// renderer with no text colour to read resolves the first to black, which is
/// the mark drawn black on black in the dark appearance.
///
/// So the one thing the file leaves to its surroundings is supplied from the
/// surroundings: the widget's own text colour, as the platform resolved it.
/// Nothing else in the file is touched, and no colour here is this
/// application's.
fn wordmark() -> gtk::Widget {
    let label = copy::text(Key::ProductName);
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Contain)
        .halign(gtk::Align::Center)
        .build();
    picture.update_property(&[gtk::accessible::Property::Label(&label)]);

    // Drawn when the widget has a style to read a colour from, and again when
    // the appearance changes under it. The handler is dropped with the widget
    // rather than left on the singleton.
    picture.connect_realize(draw_wordmark);

    let handler = adw::StyleManager::default().connect_dark_notify(glib::clone!(
        #[weak]
        picture,
        move |_| draw_wordmark(&picture)
    ));
    let held = RefCell::new(Some(handler));
    picture.connect_destroy(move |_| {
        if let Some(handler) = held.borrow_mut().take() {
            adw::StyleManager::default().disconnect(handler);
        }
    });

    picture.upcast()
}

/// Render the wordmark for the colour this widget's text is drawn in.
fn draw_wordmark(picture: &gtk::Picture) {
    let Some(texture) = wordmark_texture(&picture.color()) else {
        return;
    };

    // The room the letterform needs at the height it draws at.
    let ratio = f64::from(texture.width()) / f64::from(texture.height()).max(1.0);
    let width = (f64::from(WORDMARK_HEIGHT) * ratio).round() as i32;

    picture.set_size_request(width, WORDMARK_HEIGHT);
    picture.set_paintable(Some(&texture));
}

/// The wordmark's bytes with the one colour it leaves open supplied, decoded at
/// twice the height it draws at so a scaled display has pixels to draw with.
fn wordmark_texture(ink: &gtk::gdk::RGBA) -> Option<gtk::gdk::Texture> {
    let resource = format!("{}/brand/fermix-wordmark.svg", crate::app::RESOURCE_PREFIX);
    let bytes = gio::resources_lookup_data(&resource, gio::ResourceLookupFlags::NONE).ok()?;
    let artwork = String::from_utf8_lossy(&bytes).replace(OPEN_INK, &ink.to_str());

    // The size the mark is drawn at, in both dimensions: the loader takes no
    // "work it out from the other one", and the ratio is the file's own.
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    loader.set_size(
        (f64::from(WORDMARK_HEIGHT * 2) * WORDMARK_RATIO).round() as i32,
        WORDMARK_HEIGHT * 2,
    );

    let rendered = loader
        .write(artwork.as_bytes())
        .and_then(|()| loader.close())
        .ok()
        .and_then(|()| loader.pixbuf());

    match rendered {
        Some(pixbuf) => Some(gtk::gdk::Texture::for_pixbuf(&pixbuf)),
        None => {
            // The bytes are in this binary, so this is a host with no loader
            // for them.
            glib::g_warning!(
                "fermix-desktop",
                "the wordmark at {resource} was not drawn on this host, which has no loader for it"
            );
            None
        }
    }
}

/// What the artwork leaves to its surroundings, in the file's own spelling.
const OPEN_INK: &str = "currentColor";

/// The wordmark's own proportions, from the view box the file declares.
const WORDMARK_RATIO: f64 = 396.0 / 116.0;

fn title() -> gtk::Label {
    let label = gtk::Label::builder()
        .label(copy::text(Key::SetupWelcomeTitle))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    label.add_css_class("title-1");
    label
}

fn sentence() -> gtk::Label {
    let label = gtk::Label::builder()
        .label(copy::text(Key::SetupWelcomeBody))
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    label.set_max_width_chars(metrics::DETAIL_BUDGET / metrics::SPACE_TIGHT);
    label
}

/// The one secondary link, which picks a directory and nothing else.
fn choose_home(model: &Rc<OnboardingModel>, chosen: &gtk::Label) -> gtk::Button {
    let button = gtk::Button::builder()
        .label(copy::text(Key::SetupWelcomeChooseHome))
        .halign(gtk::Align::Center)
        .build();
    button.add_css_class("flat");

    let model = Rc::clone(model);
    let chosen = chosen.clone();
    button.connect_clicked(move |button| {
        let dialog = gtk::FileDialog::builder()
            .title(copy::text(Key::SetupWelcomeChooseHome))
            .modal(true)
            .build();

        let window = crate::ui::window_of(button);
        let model = Rc::clone(&model);
        let chosen = chosen.clone();
        super::run(async move {
            match dialog.select_folder_future(window.as_ref()).await {
                Ok(folder) => {
                    if let Some(path) = folder.path() {
                        model.choose_home(&path);
                    }
                }
                // The picker was dismissed. Nothing was chosen and nothing is
                // said about it.
                Err(_) => chosen.set_visible(model.home().is_some()),
            }
        });
    });

    button
}
