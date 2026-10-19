//! What a row shows under its words (`fermix_client::media::Strip`): a lane of tiles, pictures
//! first, and its link cards, a list under any place cards. A picture tile reads its file when
//! it first comes on screen; every read holds its file for that read alone (`Media::read`): a
//! thumbnail's decode, the viewer's, an app opening it. Until voice notes play here, a note is
//! a file tile, first in the lane.

use super::entries::toast;
use super::picture::{self, pixels, Decoder, FileRef};
use super::places::{launched, turned_down, window_of, CARDS_MIN_WIDTH, CARDS_WIDTH};
use crate::media_grants::Media;
use adw::prelude::*;
use fermix_client::chat_wire::grants::Outcome;
use fermix_client::media::{save_name, tile, Fit, LinkCard, Strip, Tile, TileKind};
use fermix_client::picture::Refused;
use fermix_client::places::is_web_address;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

/// A tile, the Mac's 3:2 at its widest, and the gap between two.
const TILE: (i32, i32) = (180, 120);
const TILE_GAP: i32 = 6;
/// The most tiles the lane shows side by side before it scrolls (the Mac's three).
const LANE_TILES: i32 = 3;
/// A link card's picture.
const AVATAR: i32 = 40;
/// A picture is asked for at most this many times, once each time it comes on screen (Mac
/// `tries`).
const TRIES: u8 = 2;
const NOT_AVAILABLE: &str = "Not available";
/// The Mac's sentence for a file the daemon could not make ready (`browser.notice.fileNotReady`).
const NOT_READY: &str = "Fermix could not get this file ready to open. Try again.";
const NO_LINK: &str = "Your browser did not open the link.";

/// What a strip reads its files with: the grants, and the page's decoder.
#[derive(Clone)]
pub struct Readers {
    pub media: Media,
    pub decoder: Rc<Decoder>,
}

pub struct StripView {
    pub widget: gtk::Box,
    lane: gtk::ScrolledWindow,
    row: gtk::Box,
    tiles: Vec<TileView>,
    links: gtk::ListBox,
    links_clamp: adw::Clamp,
    cards: Vec<LinkRow>,
    shown: Strip,
    readers: Readers,
}

impl StripView {
    /// Hidden until its row shows anything; `align` is the row's side.
    pub fn new(align: gtk::Align, readers: &Readers) -> StripView {
        let row = gtk::Box::builder().spacing(TILE_GAP).build();
        let lane = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .max_content_width(LANE_TILES * TILE.0 + (LANE_TILES - 1) * TILE_GAP)
            .propagate_natural_width(true)
            .propagate_natural_height(true)
            .halign(align)
            .child(&row)
            .build();
        let links = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .width_request(CARDS_MIN_WIDTH)
            .css_classes(["boxed-list"])
            .build();
        links.update_property(&[gtk::accessible::Property::Label("Links")]);
        let links_clamp = adw::Clamp::builder()
            .maximum_size(CARDS_WIDTH)
            .halign(align)
            .child(&links)
            .build();
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .visible(false)
            .build();
        widget.append(&lane);
        widget.append(&links_clamp);
        StripView {
            widget,
            lane,
            row,
            tiles: Vec::new(),
            links,
            links_clamp,
            cards: Vec::new(),
            shown: Strip::default(),
            readers: readers.clone(),
        }
    }

    /// Shows `strip`, keeping every tile and card it shows already, with what each has read.
    pub fn show(&mut self, strip: &Strip) {
        if self.shown == *strip {
            return;
        }
        let tiles: Vec<Tile> = strip
            .voice
            .iter()
            .map(tile)
            .chain(strip.tiles.clone())
            .collect();
        self.show_tiles(&tiles);
        self.show_links(&strip.links);
        self.shown = strip.clone();
        self.widget.set_visible(!strip.is_empty());
    }

    /// The strip came on screen: each picture not read yet is read.
    pub fn on_screen(&self) {
        for view in &self.tiles {
            view.read_thumbnail(&self.readers);
        }
        for link in &self.cards {
            link.read_picture(&self.readers);
        }
    }

    fn show_tiles(&mut self, tiles: &[Tile]) {
        let mut old = std::mem::take(&mut self.tiles);
        let mut previous: Option<gtk::Widget> = None;
        for tile in tiles {
            let view = match old.iter().position(|view| view.tile == *tile) {
                Some(index) => old.swap_remove(index),
                None => TileView::new(tile, &self.readers),
            };
            super::place(&self.row, &view.button, previous.as_ref());
            previous = Some(view.button.clone().upcast());
            self.tiles.push(view);
        }
        // What is left is gone from the row; dropping it ends its read.
        for gone in old {
            self.row.remove(&gone.button);
        }
        self.lane.set_visible(!tiles.is_empty());
    }

    fn show_links(&mut self, cards: &[LinkCard]) {
        let mut old = std::mem::take(&mut self.cards);
        self.links.remove_all();
        for card in cards {
            let link = match old.iter().position(|link| link.card == *card) {
                Some(index) => old.swap_remove(index),
                None => LinkRow::new(card),
            };
            self.links.append(&link.row);
            self.cards.push(link);
        }
        self.links_clamp.set_visible(!cards.is_empty());
    }
}

/// A read started for a tile or a card, ended when its owner goes.
#[derive(Default)]
struct Task(RefCell<Option<glib::JoinHandle<()>>>);

impl Task {
    fn run(&self, future: impl std::future::Future<Output = ()> + 'static) {
        let task = glib::spawn_future_local(future);
        if let Some(earlier) = self.0.replace(Some(task)) {
            earlier.abort();
        }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

/// How a picture's reads went: tried at most `TRIES` times, never two at once, and not again
/// once it is drawn or known never to be.
#[derive(Default)]
struct Reads {
    tries: Cell<u8>,
    reading: Cell<bool>,
    settled: Cell<bool>,
}

impl Reads {
    /// Whether a read may start now; it is counted when it may.
    fn begin(&self) -> bool {
        if self.settled.get() || self.reading.get() || self.tries.get() >= TRIES {
            return false;
        }
        self.tries.set(self.tries.get() + 1);
        self.reading.set(true);
        true
    }

    fn end(&self, settled: bool) {
        self.reading.set(false);
        self.settled.set(self.settled.get() || settled);
    }
}

/// What reading a picture for the strip came to.
enum Thumbnail {
    Drawn(gdk::Texture),
    Gone,
    Refused(Refused),
}

impl Thumbnail {
    /// Whether nothing is left to read: it is drawn, or it never will be.
    fn settled(&self) -> bool {
        !matches!(self, Thumbnail::Refused(Refused::Failed(_)))
    }
}

/// Reads `r#ref`'s picture for a square of `side` pixels, holding the file for the decode.
async fn thumbnail(readers: &Readers, r#ref: &str, side: u32) -> Thumbnail {
    let (_hold, outcome) = readers.media.read(r#ref).await;
    let decoded = match outcome {
        Outcome::Granted(path) => readers.decoder.picture(path, side, Fit::Cover).await,
        Outcome::Missing => return Thumbnail::Gone,
        Outcome::Failed(why) => Err(Refused::Failed(why)),
    };
    match decoded {
        Ok(texture) => Thumbnail::Drawn(texture),
        Err(refused) => {
            glib::g_debug!("fermix", "no thumbnail of {ref}: {refused}");
            Thumbnail::Refused(refused)
        }
    }
}

struct TileView {
    tile: Tile,
    button: gtk::Button,
    face: Face,
    reads: Rc<Reads>,
    task: Task,
}

/// The parts of a tile its reads change.
#[derive(Clone)]
struct Face {
    /// A picture tile's thumbnail; none on a file tile.
    picture: Option<gtk::Picture>,
    /// The icon and name: a picture tile's until its thumbnail lands, a file tile's always.
    cover: gtk::Box,
    /// What the tile says under its name: a file's kind and size, or that it is gone.
    note: gtk::Label,
    /// The kind and size, which a picture that will not draw says as a file tile does.
    detail: String,
}

impl TileView {
    fn new(tile: &Tile, readers: &Readers) -> TileView {
        let (content, face) = match tile.kind {
            TileKind::Picture => picture_face(tile),
            TileKind::File => file_face(tile),
        };
        let button = gtk::Button::builder()
            .child(&content)
            .tooltip_text(tile.name.as_str())
            .valign(gtk::Align::Start)
            .css_classes(["card"])
            .overflow(gtk::Overflow::Hidden)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(&tile.spoken)]);
        let (face_of, tile_of, readers) = (face.clone(), tile.clone(), readers.clone());
        button.connect_clicked(move |button| open(button, &face_of, &tile_of, &readers));
        TileView {
            tile: tile.clone(),
            button,
            face,
            reads: Rc::default(),
            task: Task::default(),
        }
    }

    /// A picture tile on screen reads its thumbnail, until it has one.
    fn read_thumbnail(&self, readers: &Readers) {
        if self.tile.kind != TileKind::Picture || !self.reads.begin() {
            return;
        }
        let (face, reads, readers) = (self.face.clone(), self.reads.clone(), readers.clone());
        let (button, r#ref) = (self.button.clone(), self.tile.media.r#ref.clone());
        self.task.run(async move {
            let side = pixels(TILE.0.max(TILE.1), &button);
            let read = thumbnail(&readers, &r#ref, side).await;
            reads.end(read.settled());
            face.show(&button, read);
        });
    }
}

impl Face {
    fn show(&self, button: &gtk::Button, read: Thumbnail) {
        match read {
            Thumbnail::Drawn(texture) => {
                if let Some(picture) = &self.picture {
                    picture.set_paintable(Some(&texture));
                }
                self.cover.set_visible(false);
            }
            Thumbnail::Gone => self.gone(button),
            Thumbnail::Refused(_) => self.say(&self.detail),
        }
    }

    /// The daemon has nothing for the file: it is said, and the tile no longer opens.
    fn gone(&self, button: &gtk::Button) {
        if let Some(picture) = &self.picture {
            picture.set_paintable(None::<&gdk::Paintable>);
        }
        self.cover.set_visible(true);
        self.say(NOT_AVAILABLE);
        button.set_sensitive(false);
    }

    fn say(&self, note: &str) {
        self.note.set_label(note);
        self.note.set_visible(true);
    }
}

/// A picture tile: until its thumbnail lands, its icon and name over where it will be.
fn picture_face(tile: &Tile) -> (gtk::Widget, Face) {
    let icon = gtk::Image::from_icon_name("image-x-generic-symbolic");
    let (cover, note) = cover(icon, tile, None);
    // The overlay is the cover's size; the picture fills it, whatever its own size.
    let frame = gtk::Box::builder()
        .width_request(TILE.0)
        .height_request(TILE.1)
        .build();
    frame.append(&cover);
    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .can_target(false)
        .alternative_text(tile.name.as_str())
        .build();
    let overlay = gtk::Overlay::builder().child(&frame).build();
    overlay.add_overlay(&picture);
    let face = Face {
        picture: Some(picture),
        cover,
        note,
        detail: tile.detail.clone(),
    };
    (overlay.upcast(), face)
}

/// A file tile: what the desktop draws for its type, its name and its size.
fn file_face(tile: &Tile) -> (gtk::Widget, Face) {
    let icon = gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(&tile.media.mime));
    let (cover, note) = cover(icon, tile, Some(&tile.detail));
    cover.set_size_request(TILE.0, -1);
    cover.set_margin_top(12);
    cover.set_margin_bottom(12);
    let face = Face {
        picture: None,
        cover: cover.clone(),
        note,
        detail: tile.detail.clone(),
    };
    (cover.upcast(), face)
}

/// The icon, the name and the note under it, centred; the note is hidden while empty.
fn cover(icon: gtk::Image, tile: &Tile, note: Option<&str>) -> (gtk::Box, gtk::Label) {
    icon.set_pixel_size(32);
    icon.add_css_class("dimmed");
    icon.set_accessible_role(gtk::AccessibleRole::Presentation);
    let name = gtk::Label::builder()
        .label(tile.name.as_str())
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .max_width_chars(18)
        .css_classes(["caption"])
        .build();
    let note = gtk::Label::builder()
        .label(note.unwrap_or_default())
        .visible(note.is_some())
        .css_classes(["caption", "dimmed"])
        .build();
    let cover = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    cover.append(&icon);
    cover.append(&name);
    cover.append(&note);
    (cover, note)
}

/// A click: a picture opens in the viewer, a file in the computer's own app.
fn open(button: &gtk::Button, face: &Face, tile: &Tile, readers: &Readers) {
    let Some(window) = window_of(button) else {
        return;
    };
    let (button, face, readers) = (button.clone(), face.clone(), readers.clone());
    let file = FileRef {
        r#ref: tile.media.r#ref.clone(),
        name: tile.name.clone(),
        save_as: save_name(tile),
        media: readers.media.clone(),
    };
    let side = picture::viewer_side(&button);
    let kind = tile.kind;
    glib::spawn_future_local(async move {
        let (hold, outcome) = file.media.read(&file.r#ref).await;
        let path = match outcome {
            Outcome::Granted(path) => path,
            Outcome::Missing => return face.gone(&button),
            Outcome::Failed(why) => {
                glib::g_warning!("fermix", "{} did not open: {why}", file.r#ref);
                return toast(&window, NOT_READY);
            }
        };
        match kind {
            TileKind::Picture => view(&window, &readers.decoder, &path, side, &file).await,
            TileKind::File => open_outside(&window, &path, &file.name).await,
        }
        drop(hold);
    });
}

/// The picture in the viewer, decoded at its size; one that will not decode opens outside.
async fn view(window: &gtk::Window, decoder: &Rc<Decoder>, path: &Path, side: u32, file: &FileRef) {
    match decoder.picture(path.to_owned(), side, Fit::Contain).await {
        Ok(texture) => picture::open_viewer(window, &texture, file, decoder),
        Err(refused) => {
            glib::g_debug!("fermix", "{} opens outside: {refused}", file.r#ref);
            open_outside(window, path, &file.name).await;
        }
    }
}

/// The file in the app the desktop opens its type with: through the OpenURI portal inside a
/// sandbox, GIO's handler outside one. `name` is the row's, for the person.
async fn open_outside(window: &gtk::Window, path: &Path, name: &str) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    let Err(e) = launcher.launch_future(Some(window)).await else {
        return;
    };
    if !turned_down(&e) {
        glib::g_warning!("fermix", "{} did not open: {e}", path.display());
        toast(window, &format!("Could not open {name}"));
    }
}

/// A stored preview of a page the row links: a row that opens it in the browser.
struct LinkRow {
    card: LinkCard,
    row: adw::ActionRow,
    avatar: adw::Avatar,
    reads: Rc<Reads>,
    task: Task,
}

impl LinkRow {
    fn new(card: &LinkCard) -> LinkRow {
        // Markup off before any text: titles and sites are the page's own words.
        let row = adw::ActionRow::new();
        row.set_use_markup(false);
        row.set_title(&card.title);
        row.set_title_lines(1);
        row.set_subtitle(&card.site);
        row.set_subtitle_lines(1);
        row.set_tooltip_text(Some(&card.opens));
        row.set_activatable(true);
        let accessible = row.upcast_ref::<gtk::Widget>();
        accessible.update_property(&[gtk::accessible::Property::Label(&card.spoken)]);
        let avatar = adw::Avatar::new(AVATAR, Some(&card.initial), true);
        // As the row's, the avatar's type does not carry `Accessible` here; its widget does.
        let presentation = avatar.upcast_ref::<gtk::Widget>();
        presentation.set_accessible_role(gtk::AccessibleRole::Presentation);
        row.add_prefix(&avatar);
        let out = gtk::Image::from_icon_name("adw-external-link-symbolic");
        out.add_css_class("dimmed");
        out.set_accessible_role(gtk::AccessibleRole::Presentation);
        row.add_suffix(&out);
        let opens = card.opens.clone();
        row.connect_activated(move |row| open_link(row, &opens));
        LinkRow {
            card: card.clone(),
            row,
            avatar,
            reads: Rc::default(),
            task: Task::default(),
        }
    }

    /// A card on screen reads the page's picture, where it has one, until it has it.
    fn read_picture(&self, readers: &Readers) {
        let Some(r#ref) = self.card.image_ref.clone() else {
            return;
        };
        if !self.reads.begin() {
            return;
        }
        let (avatar, reads, readers) = (self.avatar.clone(), self.reads.clone(), readers.clone());
        self.task.run(async move {
            let side = pixels(AVATAR, &avatar);
            let read = thumbnail(&readers, &r#ref, side).await;
            reads.end(read.settled());
            if let Thumbnail::Drawn(texture) = read {
                avatar.set_custom_image(Some(&texture));
            }
        });
    }
}

fn open_link(row: &adw::ActionRow, opens: &str) {
    // The card was planned with a web address alone; anything else would open in its own app.
    if !is_web_address(opens) {
        glib::g_warning!("fermix", "a link card held no web address: {opens}");
        return;
    }
    let Some(window) = window_of(row) else {
        return;
    };
    let opens = opens.to_owned();
    glib::spawn_future_local(async move {
        let Err(e) = launched(&window, &opens).await else {
            return;
        };
        if !turned_down(&e) {
            glib::g_warning!("fermix", "the link did not open: {e}");
            toast(&window, NO_LINK);
        }
    });
}
