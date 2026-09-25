//! The conversation's entries as widgets: the person's bubbles and Fermix's,
//! the tools it ran, its collapsed thoughts, pictures, and failures. A streamed
//! entry grows in place, so a chunk never redraws what is already on screen.

use adw::prelude::*;
use fermix_client::acp::{Image, ToolStatus};
use fermix_client::chat::{picture_file_name, picture_width, tool_label, tool_state, Entry};
use fermix_client::markdown::{render, Block};
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::rc::Rc;

/// A picture in the conversation fits a square this many pixels wide.
const PICTURE_BOUND: i32 = 420;
/// The picture viewer opens no larger than this, and no smaller than `VIEWER_MIN`.
const VIEWER_MAX: (i32, i32) = (1000, 760);
const VIEWER_MIN: (i32, i32) = (360, 280);
/// The viewer's header bar, which the picture's height does not include.
const HEADER_HEIGHT: i32 = 47;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Person,
    Fermix,
    Middle,
}

/// One drawn entry: its row in the list, the time under it, and the parts
/// that change after it is drawn.
pub struct Drawn {
    pub entry: Entry,
    pub row: gtk::Box,
    time: gtk::Label,
    parts: Parts,
}

enum Parts {
    Fixed,
    Reply(Reply),
    Thought { title: gtk::Label, text: gtk::Label },
    Failure { retry: gtk::Button },
}

struct Reply {
    body: gtk::Box,
    blocks: Vec<(Block, gtk::Widget, Option<gtk::Label>)>,
    /// The Markdown the copy button copies, kept current while the reply grows.
    source: Rc<RefCell<String>>,
}

impl Drawn {
    pub fn new(entry: &Entry) -> Drawn {
        let (side, body, copy, parts) = draw_body(entry);
        let (row, time) = row(side, &body, copy);
        Drawn {
            entry: entry.clone(),
            row,
            time,
            parts,
        }
    }

    /// Grows this entry into `entry`, the same entry with more streamed in;
    /// false when `entry` is a different one and has to be drawn anew.
    pub fn grow(&mut self, entry: &Entry) -> bool {
        match (&mut self.parts, entry) {
            (Parts::Reply(reply), Entry::Assistant(text)) => reply.grow(text),
            (Parts::Thought { text: label, .. }, Entry::Thought(text)) => label.set_text(text),
            _ => return false,
        }
        self.entry = entry.clone();
        true
    }

    /// What depends on the rest of the conversation: the time under the entry,
    /// whether a failure offers Retry, and whether a thought is still coming.
    pub fn decorate(&self, time: Option<&str>, retry: bool, live: bool) {
        self.time.set_visible(time.is_some());
        self.time.set_text(time.unwrap_or_default());
        match &self.parts {
            Parts::Failure { retry: button } => button.set_visible(retry),
            Parts::Thought { title, .. } => {
                title.set_text(if live { "Thinking…" } else { "Thought" })
            }
            Parts::Fixed | Parts::Reply(_) => {}
        }
    }
}

fn draw_body(entry: &Entry) -> (Side, gtk::Widget, Option<gtk::Button>, Parts) {
    match entry {
        Entry::User(text) => {
            let copy = copy_button(Rc::new(RefCell::new(text.clone())));
            (Side::Person, user_bubble(text), Some(copy), Parts::Fixed)
        }
        Entry::Assistant(text) => {
            let reply = Reply::new(text);
            let copy = copy_button(reply.source.clone());
            (
                Side::Fermix,
                reply.body.clone().upcast(),
                Some(copy),
                Parts::Reply(reply),
            )
        }
        Entry::Thought(text) => {
            let (widget, parts) = thought(text);
            (Side::Fermix, widget, None, parts)
        }
        Entry::Image(image) => (Side::Fermix, picture(image), None, Parts::Fixed),
        Entry::Attachment(name) => (Side::Fermix, attachment(name), None, Parts::Fixed),
        Entry::Tool { title, status, .. } => {
            (Side::Fermix, tool_line(title, *status), None, Parts::Fixed)
        }
        Entry::Notice(text) => (Side::Middle, notice_line(text), None, Parts::Fixed),
        Entry::Failure(text) => {
            let (widget, retry) = failure(text);
            (Side::Fermix, widget, None, Parts::Failure { retry })
        }
    }
}

/// The entry on its side of the conversation, its copy button on the inside
/// edge, and the time under it, hidden until there is one.
fn row(side: Side, body: &gtk::Widget, copy: Option<gtk::Button>) -> (gtk::Box, gtk::Label) {
    let align = match side {
        Side::Person => gtk::Align::End,
        Side::Fermix => gtk::Align::Start,
        Side::Middle => gtk::Align::Center,
    };
    let line = gtk::Box::builder().spacing(6).halign(align).build();
    match copy {
        Some(copy) if side == Side::Person => {
            line.append(&copy);
            line.append(body);
        }
        Some(copy) => {
            line.append(body);
            line.append(&copy);
        }
        None => line.append(body),
    }
    let time = gtk::Label::builder()
        .css_classes(["dim-label", "caption", "numeric"])
        .halign(align)
        .margin_start(10)
        .margin_end(10)
        .visible(false)
        .build();
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .css_classes(["chat-row"])
        .build();
    row.append(&line);
    row.append(&time);
    (row, time)
}

fn copy_button(source: Rc<RefCell<String>>) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy")
        .valign(gtk::Align::End)
        .css_classes(["flat", "circular", "chat-hover"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Copy message")]);
    button.connect_clicked(move |button| copy_text(button, &source.borrow()));
    button
}

fn copy_text(widget: &impl IsA<gtk::Widget>, text: &str) {
    widget.clipboard().set_text(text);
    toast(widget, "Copied");
}

fn toast(widget: &impl IsA<gtk::Widget>, text: &str) {
    if let Err(e) = widget.activate_action("win.toast", Some(&text.to_variant())) {
        glib::g_warning!("fermix", "could not show the toast {text:?}: {e}");
    }
}

fn user_bubble(text: &str) -> gtk::Widget {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .max_width_chars(60)
        .css_classes(["chat-user"])
        .build()
        .upcast()
}

impl Reply {
    fn new(text: &str) -> Reply {
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .css_classes(["chat-reply"])
            .build();
        let mut reply = Reply {
            body,
            blocks: Vec::new(),
            source: Rc::default(),
        };
        reply.grow(text);
        reply
    }

    /// Unchanged blocks stay, a changed text block is updated in place, and
    /// anything after it is redrawn.
    fn grow(&mut self, text: &str) {
        let fresh = render(text);
        let same = self
            .blocks
            .iter()
            .zip(&fresh)
            .take_while(|((old, _, _), new)| old == *new)
            .count();
        let mut next = same;
        if let (Some((old, _, Some(label))), Some(new)) =
            (self.blocks.get_mut(same), fresh.get(same))
        {
            if set_in_place(label, old, new) {
                *old = new.clone();
                next += 1;
            }
        }
        for (_, widget, _) in self.blocks.drain(next..) {
            self.body.remove(&widget);
        }
        for block in fresh.into_iter().skip(next) {
            let (widget, label) = block_widget(&block);
            self.body.append(&widget);
            self.blocks.push((block, widget, label));
        }
        self.source.replace(text.to_owned());
    }
}

fn set_in_place(label: &gtk::Label, old: &Block, new: &Block) -> bool {
    match (old, new) {
        (Block::Text(_), Block::Text(markup)) | (Block::Quote(_), Block::Quote(markup)) => {
            label.set_markup(markup);
            true
        }
        (Block::Code(_), Block::Code(text)) => {
            label.set_text(text);
            true
        }
        _ => false,
    }
}

fn text_label(markup: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .build();
    label.set_markup(markup);
    label
}

fn block_widget(block: &Block) -> (gtk::Widget, Option<gtk::Label>) {
    match block {
        Block::Text(markup) => {
            let label = text_label(markup);
            (label.clone().upcast(), Some(label))
        }
        Block::Heading(level, markup) => {
            let size = match level {
                1 => "x-large",
                2 => "large",
                _ => "medium",
            };
            let label = text_label(&format!(
                "<span size=\"{size}\" weight=\"bold\">{markup}</span>"
            ));
            (label.upcast(), None)
        }
        Block::Quote(markup) => {
            let label = text_label(markup);
            label.add_css_class("chat-quote");
            label.add_css_class("dim-label");
            (label.clone().upcast(), Some(label))
        }
        Block::Code(text) => code_block(text),
        Block::Rule => (
            gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
            None,
        ),
    }
}

/// Monospace, scrolled sideways when wide, with its own copy button.
fn code_block(text: &str) -> (gtk::Widget, Option<gtk::Label>) {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .propagate_natural_width(true)
        .hexpand(true)
        .child(&label)
        .build();
    let copy = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy code")
        .valign(gtk::Align::Start)
        .css_classes(["flat", "circular"])
        .build();
    copy.update_property(&[gtk::accessible::Property::Label("Copy code")]);
    let source = label.clone();
    copy.connect_clicked(move |button| copy_text(button, &source.text()));
    let block = gtk::Box::builder()
        .spacing(6)
        .css_classes(["chat-code"])
        .build();
    block.append(&scroller);
    block.append(&copy);
    (block.upcast(), Some(label))
}

/// Collapsed and dim: the reasoning is there for whoever wants it.
fn thought(text: &str) -> (gtk::Widget, Parts) {
    let title = gtk::Label::builder()
        .label("Thinking…")
        .css_classes(["dim-label", "caption"])
        .build();
    let body = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["dim-label", "chat-thought-text"])
        .build();
    let expander = gtk::Expander::builder()
        .label_widget(&title)
        .child(&body)
        .css_classes(["chat-thought"])
        .build();
    (expander.upcast(), Parts::Thought { title, text: body })
}

fn picture(image: &Image) -> gtk::Widget {
    let bytes = glib::Bytes::from(&image.bytes[..]);
    match gdk::Texture::from_bytes(&bytes) {
        Ok(texture) => picture_button(&texture, image).upcast(),
        Err(e) => {
            glib::g_warning!("fermix", "a {} picture did not decode: {e}", image.mime);
            let line = format!(
                "Fermix sent a picture ({}) this app cannot show.",
                image.mime
            );
            quiet_line("image-missing-symbolic", &line)
        }
    }
}

/// The picture scaled to fit the conversation; a click opens it full size.
fn picture_button(texture: &gdk::Texture, image: &Image) -> gtk::Button {
    let picture = gtk::Picture::builder()
        .paintable(texture)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::ScaleDown)
        .alternative_text("Picture from Fermix")
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["chat-picture"])
        .build();
    let width = picture_width(texture.width(), texture.height(), PICTURE_BOUND);
    let clamp = adw::Clamp::builder()
        .maximum_size(width)
        .tightening_threshold(width)
        .child(&picture)
        .build();
    let button = gtk::Button::builder()
        .child(&clamp)
        .tooltip_text("Open full size")
        .css_classes(["flat", "chat-picture-button"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label(
        "Picture from Fermix. Open full size",
    )]);
    let (texture, image) = (texture.clone(), image.clone());
    button.connect_clicked(move |button| open_viewer(button, &texture, &image));
    button
}

fn open_viewer(anchor: &gtk::Button, texture: &gdk::Texture, image: &Image) {
    let picture = gtk::Picture::builder()
        .paintable(texture)
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .alternative_text("Picture from Fermix")
        .hexpand(true)
        .vexpand(true)
        .build();
    let header = adw::HeaderBar::new();
    header.pack_start(&save_picture_button(image));
    header.pack_start(&copy_picture_button(texture));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&picture));
    let dialog = adw::Dialog::builder()
        .title("Picture")
        .content_width(texture.width().clamp(VIEWER_MIN.0, VIEWER_MAX.0))
        .content_height((texture.height() + HEADER_HEIGHT).clamp(VIEWER_MIN.1, VIEWER_MAX.1))
        .child(&view)
        .build();
    dialog.present(Some(anchor));
}

fn copy_picture_button(texture: &gdk::Texture) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy")
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Copy picture")]);
    let texture = texture.clone();
    button.connect_clicked(move |button| {
        button.clipboard().set_texture(&texture);
        toast(button, "Copied");
    });
    button
}

fn save_picture_button(image: &Image) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("document-save-as-symbolic")
        .tooltip_text("Save As…")
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Save picture as")]);
    let image = image.clone();
    button.connect_clicked(move |button| {
        let (button, image) = (button.clone(), image.clone());
        glib::spawn_future_local(async move { save_picture(&button, &image).await });
    });
    button
}

/// Writes the picture's own bytes where the person chooses, through the portal.
async fn save_picture(anchor: &gtk::Button, image: &Image) {
    let window = anchor.root().and_downcast::<gtk::Window>();
    let dialog = gtk::FileDialog::builder()
        .title("Save Picture")
        .initial_name(picture_file_name(&image.mime))
        .modal(true)
        .build();
    let file = match dialog.save_future(window.as_ref()).await {
        Ok(file) => file,
        Err(e) if e.matches(gtk::DialogError::Dismissed) => return,
        Err(e) => {
            glib::g_warning!("fermix", "the save dialog failed: {e}");
            return toast(anchor, "The save dialog did not open.");
        }
    };
    let written = file
        .replace_contents_future(
            image.bytes.to_vec(),
            None,
            false,
            gio::FileCreateFlags::REPLACE_DESTINATION,
        )
        .await;
    match written {
        Ok(_) => toast(anchor, "Picture saved"),
        Err((_, e)) => {
            glib::g_warning!("fermix", "saving the picture failed: {e}");
            toast(
                anchor,
                &format!("The picture was not saved: {}", e.message()),
            );
        }
    }
}

/// A file Fermix sent that ACP cannot carry: named, and said plainly.
fn attachment(name: &str) -> gtk::Widget {
    let icon = gtk::Image::from_icon_name("mail-attachment-symbolic");
    icon.set_valign(gtk::Align::Center);
    let title = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .max_width_chars(40)
        .selectable(true)
        .css_classes(["heading"])
        .build();
    let note = gtk::Label::builder()
        .label("Files cannot come through this chat yet.")
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .build();
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .build();
    text.append(&title);
    text.append(&note);
    let chip = gtk::Box::builder()
        .spacing(12)
        .css_classes(["chat-file"])
        .build();
    chip.append(&icon);
    chip.append(&text);
    chip.upcast()
}

fn tool_line(title: &str, status: ToolStatus) -> gtk::Widget {
    let row = gtk::Box::builder()
        .spacing(8)
        .css_classes(["chat-tool"])
        .build();
    let marker: gtk::Widget = match status {
        ToolStatus::Running => adw::Spinner::new().upcast(),
        ToolStatus::Completed => gtk::Image::from_icon_name("object-select-symbolic").upcast(),
        ToolStatus::Failed => gtk::Image::from_icon_name("dialog-warning-symbolic").upcast(),
    };
    marker.add_css_class(if status == ToolStatus::Failed {
        "warning"
    } else {
        "dim-label"
    });
    row.append(&marker);
    let name = gtk::Label::builder()
        .label(tool_label(title))
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .css_classes(["caption", "dim-label"])
        .build();
    row.append(&name);
    let state = gtk::Label::builder()
        .label(tool_state(status))
        .css_classes(["caption", "dim-label"])
        .build();
    row.append(&state);
    row.upcast()
}

fn quiet_line(icon: &str, text: &str) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    let image = gtk::Image::from_icon_name(icon);
    image.add_css_class("dim-label");
    row.append(&image);
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build();
    row.append(&label);
    row.upcast()
}

fn notice_line(text: &str) -> gtk::Widget {
    gtk::Label::builder()
        .label(text)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .css_classes(["dim-label", "caption"])
        .build()
        .upcast()
}

/// Why the reply did not come, and a way to ask again while that can help.
fn failure(text: &str) -> (gtk::Widget, gtk::Button) {
    let icon = gtk::Image::from_icon_name("dialog-error-symbolic");
    icon.add_css_class("error");
    icon.set_valign(gtk::Align::Start);
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .build();
    let retry = gtk::Button::builder()
        .label("Retry")
        .halign(gtk::Align::Start)
        .action_name("win.retry-reply")
        .visible(false)
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    column.append(&label);
    column.append(&retry);
    let bubble = gtk::Box::builder()
        .spacing(10)
        .css_classes(["chat-failure"])
        .build();
    bubble.append(&icon);
    bubble.append(&column);
    (bubble.upcast(), retry)
}
