//! Pictures from the phone on screen: decoded on worker threads (`fermix_client::picture`, at the
//! size they are drawn, never past `media::PICTURE_PIXELS`), at most two at a time
//! (`media::Admission`), made into textures on the main loop; and the viewer a picture tile opens,
//! with Copy and Save As.

use super::entries::toast;
use crate::media_grants::Media;
use adw::prelude::*;
use fermix_client::chat_wire::grants::Outcome;
use fermix_client::media::{Admission, Fit};
use fermix_client::picture::{Decoded, Refused};
use futures_channel::oneshot;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The viewer opens no larger than this, and no smaller than `VIEWER_MIN`.
const VIEWER_MAX: (i32, i32) = (1000, 760);
const VIEWER_MIN: (i32, i32) = (360, 280);
/// The viewer's header bar, which its picture's height does not include.
const HEADER_HEIGHT: i32 = 47;
/// Round the viewer's picture, so it touches neither the header nor an edge.
const PICTURE_MARGIN: i32 = 12;
/// A square no picture fills: a decode for it is the picture at its own size, still under
/// `media::PICTURE_PIXELS`.
const WHOLE: u32 = u32::MAX;

/// The texture of decoded pixels, made on the main loop: straight RGBA, rows packed.
fn texture(decoded: Decoded) -> Result<gdk::Texture, Refused> {
    let size =
        |side: u32| i32::try_from(side).map_err(|_| Refused::Failed(format!("{side} pixels")));
    let (width, height) = (size(decoded.width)?, size(decoded.height)?);
    let stride = decoded.width as usize * 4;
    let bytes = glib::Bytes::from_owned(decoded.rgba);
    let texture =
        gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    Ok(texture.upcast())
}

/// A file a viewer or a tile reads: its ref, its name, the bare name to save it as, and the
/// grants.
#[derive(Clone)]
pub struct FileRef {
    pub r#ref: String,
    pub name: String,
    pub save_as: String,
    pub media: Media,
}

struct Job {
    path: PathBuf,
    side: u32,
    fit: Fit,
    answer: oneshot::Sender<Result<Decoded, Refused>>,
}

/// How a worker thread decodes: `fermix_client::picture::decode` in the app.
pub type Decode = fn(&Path, u32, Fit) -> Result<Decoded, Refused>;

/// The page's decodes: two run on worker threads, the rest wait their turn, and past
/// `MAX_WAITING` the oldest waiting gives way.
pub struct Decoder {
    admission: RefCell<Admission<Job>>,
    decode: Decode,
}

impl Decoder {
    pub fn new(decode: Decode) -> Rc<Decoder> {
        Rc::new(Decoder {
            admission: RefCell::default(),
            decode,
        })
    }

    /// `path`'s picture for a square of `side` pixels, once its turn comes.
    pub async fn picture(
        self: &Rc<Self>,
        path: PathBuf,
        side: u32,
        fit: Fit,
    ) -> Result<gdk::Texture, Refused> {
        self.decoded(path, side, fit).await.and_then(texture)
    }

    /// The pixels, once the turn comes, for the main loop to make a texture of.
    async fn decoded(
        self: &Rc<Self>,
        path: PathBuf,
        side: u32,
        fit: Fit,
    ) -> Result<Decoded, Refused> {
        let (answer, decoded) = oneshot::channel();
        let job = Job {
            path,
            side,
            fit,
            answer,
        };
        let (run, dropped) = self.admission.borrow_mut().ask(job);
        if let Some(dropped) = dropped {
            let gave_way = Err(Refused::Failed("too many pictures waiting".into()));
            if dropped.answer.send(gave_way).is_err() {
                glib::g_debug!("fermix", "a picture gave way after its reader left");
            }
        }
        if let Some(job) = run {
            self.start(job);
        }
        match decoded.await {
            Ok(decoded) => decoded,
            Err(oneshot::Canceled) => Err(Refused::Failed("the decode was dropped".into())),
        }
    }

    /// Runs `job`, then the next waiting, however the run ends.
    fn start(self: &Rc<Self>, job: Job) {
        let turn = Turn(self.clone());
        let decode = self.decode;
        glib::spawn_future_local(async move {
            run(decode, job).await;
            drop(turn);
        });
    }
}

/// A running decode's place in the admission, given back when it is dropped: at the end of its
/// run, or with its task, so no slot is ever lost.
struct Turn(Rc<Decoder>);

impl Drop for Turn {
    fn drop(&mut self) {
        let next = self.0.admission.borrow_mut().done();
        if let Some(next) = next {
            self.0.start(next);
        }
    }
}

/// Decodes on a worker thread, unless the job's reader already left. A decoder that panics on
/// the file, as an image crate can on hostile input, has refused it.
async fn run(decode: Decode, job: Job) {
    let Job {
        path,
        side,
        fit,
        answer,
    } = job;
    if answer.is_canceled() {
        return;
    }
    let decoded = match gio::spawn_blocking(move || decode(&path, side, fit)).await {
        Ok(decoded) => decoded,
        Err(panic) => {
            let why = panic
                .downcast_ref::<&str>()
                .map(|words| words.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no words".to_owned());
            glib::g_critical!("fermix", "the picture decoder panicked: {why}");
            Err(Refused::Failed(format!("the decoder panicked: {why}")))
        }
    };
    if answer.send(decoded).is_err() {
        glib::g_debug!("fermix", "a picture decoded after its reader left");
    }
}

/// The square, in pixels, a viewer's picture is decoded for on `widget`'s screen.
pub fn viewer_side(widget: &impl IsA<gtk::Widget>) -> u32 {
    pixels(VIEWER_MAX.0.max(VIEWER_MAX.1), widget)
}

/// `logical` pixels as `widget`'s screen draws them. Read when a decode starts: the scale is 1
/// until the widget is on screen.
pub fn pixels(logical: i32, widget: &impl IsA<gtk::Widget>) -> u32 {
    (logical * widget.scale_factor().max(1)).unsigned_abs()
}

/// The picture, decoded for the viewer, in a dialog over `anchor`'s window, with Save As and
/// Copy, which copies the picture whole.
pub fn open_viewer(
    anchor: &impl IsA<gtk::Widget>,
    texture: &gdk::Texture,
    file: &FileRef,
    decoder: &Rc<Decoder>,
) {
    let picture = gtk::Picture::builder()
        .paintable(texture)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .alternative_text(file.name.as_str())
        .hexpand(true)
        .vexpand(true)
        .margin_top(PICTURE_MARGIN)
        .margin_bottom(PICTURE_MARGIN)
        .margin_start(PICTURE_MARGIN)
        .margin_end(PICTURE_MARGIN)
        .build();
    let header = adw::HeaderBar::new();
    header.pack_start(&save_button(file));
    header.pack_start(&copy_button(file, decoder));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&picture));
    let scale = anchor.scale_factor().max(1);
    let (width, height) = viewer_size(texture.width() / scale, texture.height() / scale);
    let dialog = adw::Dialog::builder()
        .title(file.name.as_str())
        .content_width(width)
        .content_height(height)
        .child(&view)
        .build();
    dialog.present(Some(anchor));
}

/// The viewer's size for a picture of `width` by `height` logical pixels: the picture fitted, on
/// both axes and never enlarged, into the room under the header inside its margin.
fn viewer_size(width: i32, height: i32) -> (i32, i32) {
    let margins = 2 * PICTURE_MARGIN;
    let room = (
        VIEWER_MAX.0 - margins,
        VIEWER_MAX.1 - HEADER_HEIGHT - margins,
    );
    let (width, height) = (f64::from(width.max(1)), f64::from(height.max(1)));
    let scale = (f64::from(room.0) / width)
        .min(f64::from(room.1) / height)
        .min(1.0);
    let fitted = |length: f64| (length * scale).round() as i32;
    (
        (fitted(width) + margins).clamp(VIEWER_MIN.0, VIEWER_MAX.0),
        (fitted(height) + margins + HEADER_HEIGHT).clamp(VIEWER_MIN.1, VIEWER_MAX.1),
    )
}

fn copy_button(file: &FileRef, decoder: &Rc<Decoder>) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy")
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Copy picture")]);
    let (file, decoder) = (file.clone(), decoder.clone());
    button.connect_clicked(move |button| {
        let (button, file, decoder) = (button.clone(), file.clone(), decoder.clone());
        glib::spawn_future_local(async move { copy_picture(&button, &file, &decoder).await });
    });
    button
}

/// Copies the picture at its own size, decoded once more off the main loop under the same
/// ceiling and in the same line as every decode, holding its file for the decode.
async fn copy_picture(anchor: &gtk::Button, file: &FileRef, decoder: &Rc<Decoder>) {
    let (hold, outcome) = file.media.read(&file.r#ref).await;
    let whole = match outcome {
        Outcome::Granted(path) => decoder
            .picture(path, WHOLE, Fit::Contain)
            .await
            .map_err(|refused| refused.to_string()),
        Outcome::Missing => Err("it is no longer on this computer".to_owned()),
        Outcome::Failed(why) => Err(why),
    };
    drop(hold);
    match whole {
        Ok(texture) => {
            let (width, height) = (texture.width(), texture.height());
            glib::g_debug!("fermix", "copied {} whole, {width}x{height}", file.name);
            anchor.clipboard().set_texture(&texture);
            toast(anchor, "Copied");
        }
        Err(why) => {
            glib::g_warning!("fermix", "copying {} failed: {why}", file.name);
            toast(anchor, "The picture was not copied.");
        }
    }
}

fn save_button(file: &FileRef) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("document-save-as-symbolic")
        .tooltip_text("Save As…")
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Save picture as")]);
    let file = file.clone();
    button.connect_clicked(move |button| {
        let (button, file) = (button.clone(), file.clone());
        glib::spawn_future_local(async move { save_picture(&button, &file).await });
    });
    button
}

/// Copies the picture's own file where the person chooses, holding it for the copy.
async fn save_picture(anchor: &gtk::Button, file: &FileRef) {
    let window = anchor.root().and_downcast::<gtk::Window>();
    let dialog = gtk::FileDialog::builder()
        .title("Save Picture")
        .initial_name(file.save_as.as_str())
        .modal(true)
        .build();
    let target = match dialog.save_future(window.as_ref()).await {
        Ok(target) => target,
        Err(e) if e.matches(gtk::DialogError::Dismissed) => return,
        Err(e) => {
            glib::g_warning!("fermix", "the save dialog failed: {e}");
            return toast(anchor, "The save dialog did not open.");
        }
    };
    let (hold, outcome) = file.media.read(&file.r#ref).await;
    let copied = match outcome {
        Outcome::Granted(path) => copy(&path, &target).await,
        Outcome::Missing => Err("it is no longer on this computer".to_owned()),
        Outcome::Failed(why) => Err(why),
    };
    drop(hold);
    match copied {
        Ok(()) => toast(anchor, "Picture saved"),
        Err(why) => {
            glib::g_warning!("fermix", "saving {} failed: {why}", file.name);
            toast(anchor, "The picture was not saved.");
        }
    }
}

async fn copy(path: &Path, target: &gio::File) -> Result<(), String> {
    let source = gio::File::for_path(path);
    let (copied, _progress) = source.copy_future(
        target,
        gio::FileCopyFlags::OVERWRITE,
        glib::Priority::DEFAULT,
    );
    copied.await.map_err(|e| e.message().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fermix_client::media::MAX_DECODING;
    use std::time::{Duration, Instant};

    const PATIENCE: Duration = Duration::from_secs(5);

    /// A decoder that panics on a file named `panics`, as the crates have on fuzzed input, and
    /// gives one pixel otherwise.
    fn planted(path: &Path, _side: u32, _fit: Fit) -> Result<Decoded, Refused> {
        if path.ends_with("panics") {
            panic!("a planted decoder panic");
        }
        Ok(Decoded {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        })
    }

    fn on_loop(test: impl FnOnce(&glib::MainContext)) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| test(&context))
            .expect("a context of the test's own");
    }

    type Seen = Rc<RefCell<Vec<(String, Result<(), Refused>)>>>;

    fn ask(decoder: &Rc<Decoder>, name: &str, seen: &Seen) {
        let (decoder, name, seen) = (decoder.clone(), name.to_owned(), seen.clone());
        glib::spawn_future_local(async move {
            let decoded = decoder.decoded(PathBuf::from(&name), 1, Fit::Cover).await;
            seen.borrow_mut().push((name, decoded.map(|_| ())));
        });
    }

    /// Every decode slot a panic took is given back: the panics are refusals, and the picture
    /// asked after them still decodes.
    #[test]
    fn a_decoder_that_panics_gives_its_turn_back() {
        on_loop(|context| {
            let decoder = Decoder::new(planted);
            let seen: Seen = Rc::default();
            for _ in 0..MAX_DECODING {
                ask(&decoder, "panics", &seen);
            }
            ask(&decoder, "fine", &seen);
            let deadline = Instant::now() + PATIENCE;
            while seen.borrow().len() < MAX_DECODING + 1 && Instant::now() < deadline {
                context.iteration(false);
                std::thread::sleep(Duration::from_millis(5));
            }
            let seen = seen.borrow();
            assert_eq!(seen.len(), MAX_DECODING + 1, "{seen:?}");
            let panicked = Refused::Failed("the decoder panicked: a planted decoder panic".into());
            for (name, decoded) in seen.iter() {
                match name.as_str() {
                    "panics" => assert_eq!(decoded, &Err(panicked.clone())),
                    _ => assert_eq!(decoded, &Ok(())),
                }
            }
        });
    }

    /// The picture fits under the header with a margin round it on both axes, never larger than
    /// it is, and the dialog keeps its smallest size for a small one.
    #[test]
    fn the_viewer_fits_the_picture_under_its_header() {
        let margins = 2 * PICTURE_MARGIN;
        assert_eq!(viewer_size(4000, 3000), (919 + margins, VIEWER_MAX.1));
        assert_eq!(viewer_size(1000, 4000), (VIEWER_MIN.0, VIEWER_MAX.1));
        assert_eq!(viewer_size(8000, 1000), (VIEWER_MAX.0, VIEWER_MIN.1));
        assert_eq!(
            viewer_size(800, 500),
            (800 + margins, 500 + margins + HEADER_HEIGHT)
        );
        assert_eq!(viewer_size(200, 100), VIEWER_MIN);
        assert_eq!(viewer_size(0, 0), VIEWER_MIN);
    }
}
