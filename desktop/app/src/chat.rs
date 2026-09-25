//! Chat: the conversation, drawn from `Transcript`, and the composer under it.
//! The page never talks to Fermix; its buttons fire window actions and
//! `conversation.rs` does the work.

use crate::state::{Connection, State};
use crate::status::{down_view, waiting, DownPage};
use adw::prelude::*;
use fermix_client::acp::ToolStatus;
use fermix_client::chat::{tool_label, Entry, Phase, Transcript};
use fermix_client::markdown::{render, Block};
use fermix_client::view::answers_with;
use gtk::gdk;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Past this distance from the bottom, new text no longer pulls the view down.
const STICK_DISTANCE: f64 = 48.0;

/// One drawn entry. An assistant reply keeps its blocks so a streamed chunk
/// updates the last paragraph in place instead of redrawing the reply.
struct Shown {
    entry: Entry,
    widget: gtk::Widget,
    blocks: Vec<(Block, gtk::Widget, Option<gtk::Label>)>,
}

pub struct ChatPage {
    pub root: gtk::Stack,
    conversation: gtk::Stack,
    empty: adw::StatusPage,
    list: gtk::Box,
    thinking: gtk::Box,
    shown: RefCell<Vec<Shown>>,
    input: gtk::TextView,
    send: gtk::Button,
    down: DownPage,
}

impl ChatPage {
    pub fn new() -> Self {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .build();
        let thinking = thinking_row();
        let (scroller, conversation, empty) = conversation_area(&list, &thinking);
        let (composer, input, send) = composer();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        body.append(&conversation);
        body.append(&composer);
        let down = DownPage::new();
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        root.add_named(&body, Some("chat"));
        root.add_named(&down.page, Some("down"));
        stick_to_bottom(&scroller);
        ChatPage {
            root,
            conversation,
            empty,
            list,
            thinking,
            shown: RefCell::default(),
            input,
            send,
            down,
        }
    }

    pub fn render(&self, transcript: &Transcript, state: &State) {
        let problem = match &state.connection {
            Connection::Down(problem) if !state.waking => Some(problem),
            _ => None,
        };
        if let Some(problem) = problem.filter(|_| transcript.entries().is_empty()) {
            let needs_it = "Chat needs Fermix running. Start it to ask anything.";
            self.down
                .show(down_view(problem, state.wake_failed, needs_it));
            self.root.set_visible_child_name("down");
            return;
        }
        if matches!(state.connection, Connection::Down(_)) && transcript.entries().is_empty() {
            self.down.show(waiting("Starting Fermix"));
            self.root.set_visible_child_name("down");
            return;
        }
        self.down.reset();
        self.root.set_visible_child_name("chat");
        self.render_conversation(transcript, state);
    }

    fn render_conversation(&self, transcript: &Transcript, state: &State) {
        let answers = state.snapshot().map(|s| answers_with(&s.state));
        self.empty.set_description(Some(&match answers {
            Some(line) => format!("Replies come from {line}."),
            None => String::new(),
        }));
        let empty = transcript.entries().is_empty();
        self.conversation
            .set_visible_child_name(if empty { "empty" } else { "messages" });
        self.show_entries(transcript.entries());
        self.thinking
            .set_visible(transcript.phase() == Phase::Waiting);
        self.show_composer(transcript.phase(), state.snapshot().is_some());
    }

    fn show_composer(&self, phase: Phase, up: bool) {
        let running = phase != Phase::Idle;
        let (icon, tooltip, action) = if running {
            ("media-playback-stop-symbolic", "Stop", "win.stop-reply")
        } else {
            ("go-up-symbolic", "Send", "win.send-message")
        };
        self.send.set_icon_name(icon);
        self.send.set_tooltip_text(Some(tooltip));
        self.send.set_action_name(Some(action));
        let has_text = self.input.buffer().char_count() > 0;
        self.send
            .set_sensitive(up && phase != Phase::Stopping && (running || has_text));
        self.input.set_editable(up);
    }

    /// The composer's text, trimmed; the composer keeps it until `clear_input`.
    pub fn input_text(&self) -> String {
        let buffer = self.input.buffer();
        buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .trim()
            .to_owned()
    }

    pub fn clear_input(&self) {
        self.input.buffer().set_text("");
    }

    pub fn focus_input(&self) {
        self.input.grab_focus();
    }

    fn show_entries(&self, entries: &[Entry]) {
        let mut shown = self.shown.borrow_mut();
        let mut same = shown
            .iter()
            .zip(entries)
            .take_while(|(drawn, entry)| drawn.entry == **entry)
            .count();
        if let (Some(drawn), Some(Entry::Assistant(text))) =
            (shown.get_mut(same), entries.get(same))
        {
            if matches!(drawn.entry, Entry::Assistant(_)) && same + 1 == entries.len() {
                grow_reply(drawn, text);
                same += 1;
            }
        }
        for stale in shown.drain(same..) {
            self.list.remove(&stale.widget);
        }
        for entry in &entries[same..] {
            let drawn = draw_entry(entry);
            self.list.append(&drawn.widget);
            shown.push(drawn);
        }
    }
}

fn conversation_area(
    list: &gtk::Box,
    thinking: &gtk::Box,
) -> (gtk::ScrolledWindow, gtk::Stack, adw::StatusPage) {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(18)
        .margin_end(18)
        .build();
    column.append(list);
    column.append(thinking);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .child(&column)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&clamp)
        .build();
    let empty = adw::StatusPage::builder()
        .icon_name("fermix-chat-symbolic")
        .title("Ask Fermix")
        .vexpand(true)
        .build();
    let stack = gtk::Stack::builder()
        .transition_type(gtk::StackTransitionType::Crossfade)
        .vexpand(true)
        .build();
    stack.add_named(&empty, Some("empty"));
    stack.add_named(&scroller, Some("messages"));
    (scroller, stack, empty)
}

/// Keeps the newest text in view while the reader is at the bottom, and leaves
/// the view alone once they scroll up to read.
fn stick_to_bottom(scroller: &gtk::ScrolledWindow) {
    let adjustment = scroller.vadjustment();
    let stuck = Rc::new(Cell::new(true));
    let watch = stuck.clone();
    adjustment.connect_value_changed(move |a| {
        watch.set(a.value() + a.page_size() >= a.upper() - STICK_DISTANCE);
    });
    adjustment.connect_changed(move |a| {
        if stuck.get() {
            a.set_value(a.upper() - a.page_size());
        }
    });
}

fn thinking_row() -> gtk::Box {
    let row = gtk::Box::builder().spacing(8).visible(false).build();
    row.append(&adw::Spinner::new());
    let label = gtk::Label::builder()
        .label("Thinking…")
        .css_classes(["dim-label"])
        .build();
    row.append(&label);
    row
}

fn composer() -> (adw::Clamp, gtk::TextView, gtk::Button) {
    let input = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .hexpand(true)
        .build();
    input.update_property(&[gtk::accessible::Property::Label("Message Fermix")]);
    let placeholder = gtk::Label::builder()
        .label("Message Fermix")
        .css_classes(["dim-label"])
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .margin_top(8)
        .can_target(false)
        .build();
    // An external scrollbar keeps a one-line composer one line tall; a visible
    // one would impose its own minimum height.
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .propagate_natural_height(true)
        .max_content_height(200)
        .child(&input)
        .build();
    let field = gtk::Overlay::builder()
        .child(&scroller)
        .hexpand(true)
        .build();
    field.add_overlay(&placeholder);
    let send = gtk::Button::builder()
        .icon_name("go-up-symbolic")
        .css_classes(["circular", "suggested-action"])
        .valign(gtk::Align::End)
        .tooltip_text("Send")
        .sensitive(false)
        .build();
    send.set_action_name(Some("win.send-message"));
    let row = gtk::Box::builder()
        .spacing(6)
        .css_classes(["chat-composer"])
        .build();
    row.append(&field);
    row.append(&send);
    wire_composer(&input, &placeholder, &send);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .margin_start(18)
        .margin_end(18)
        .margin_bottom(18)
        .child(&row)
        .build();
    (clamp, input, send)
}

/// Enter sends, Shift+Enter starts a new line, and the placeholder and Send
/// button follow whether there is anything to send.
fn wire_composer(input: &gtk::TextView, placeholder: &gtk::Label, send: &gtk::Button) {
    let (hint, button) = (placeholder.clone(), send.clone());
    input.buffer().connect_changed(move |buffer| {
        let empty = buffer.char_count() == 0;
        hint.set_visible(empty);
        if button.action_name().as_deref() == Some("win.send-message") {
            button.set_sensitive(!empty);
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|controller, key, _, modifiers| {
        let enter = matches!(key, gdk::Key::Return | gdk::Key::KP_Enter);
        let plain =
            !modifiers.intersects(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK);
        if !(enter && plain) {
            return glib::Propagation::Proceed;
        }
        let Some(input) = controller.widget() else {
            return glib::Propagation::Proceed;
        };
        if let Err(e) = input.activate_action("win.send-message", None) {
            glib::g_debug!("fermix", "nothing to send: {e}");
        }
        glib::Propagation::Stop
    });
    input.add_controller(keys);
}

fn draw_entry(entry: &Entry) -> Shown {
    let (widget, blocks) = match entry {
        Entry::User(text) => (user_bubble(text), Vec::new()),
        Entry::Assistant(text) => {
            let reply = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(10)
                .build();
            let blocks = append_blocks(&reply, render(text));
            (reply.upcast(), blocks)
        }
        Entry::Tool { title, status, .. } => (tool_line(title, *status), Vec::new()),
        Entry::Notice(text) => (notice_line(text), Vec::new()),
        Entry::Failure(text) => (failure_line(text), Vec::new()),
    };
    Shown {
        entry: entry.clone(),
        widget,
        blocks,
    }
}

/// Updates a streaming reply: unchanged blocks stay, a changed text block is
/// updated in place, and anything after it is redrawn.
fn grow_reply(drawn: &mut Shown, text: &str) {
    let reply = drawn
        .widget
        .downcast_ref::<gtk::Box>()
        .expect("a reply is drawn as a box")
        .clone();
    let fresh = render(text);
    let same = drawn
        .blocks
        .iter()
        .zip(&fresh)
        .take_while(|((old, _, _), new)| old == *new)
        .count();
    let mut next = same;
    if let (Some((old, _, Some(label))), Some(new)) = (drawn.blocks.get_mut(same), fresh.get(same))
    {
        if set_in_place(label, old, new) {
            *old = new.clone();
            next += 1;
        }
    }
    for (_, widget, _) in drawn.blocks.drain(next..) {
        reply.remove(&widget);
    }
    let added = append_blocks(&reply, fresh[next..].to_vec());
    drawn.blocks.extend(added);
    drawn.entry = Entry::Assistant(text.to_owned());
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

fn append_blocks(
    reply: &gtk::Box,
    blocks: Vec<Block>,
) -> Vec<(Block, gtk::Widget, Option<gtk::Label>)> {
    blocks
        .into_iter()
        .map(|block| {
            let (widget, label) = block_widget(&block);
            reply.append(&widget);
            (block, widget, label)
        })
        .collect()
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
        Block::Code(text) => {
            let label = gtk::Label::builder()
                .label(text)
                .xalign(0.0)
                .selectable(true)
                .css_classes(["monospace"])
                .build();
            let scroller = gtk::ScrolledWindow::builder()
                .vscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .css_classes(["chat-code"])
                .child(&label)
                .build();
            (scroller.upcast(), Some(label))
        }
        Block::Rule => (
            gtk::Separator::new(gtk::Orientation::Horizontal).upcast(),
            None,
        ),
    }
}

fn user_bubble(text: &str) -> gtk::Widget {
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .selectable(true)
        .max_width_chars(60)
        .css_classes(["chat-user"])
        .halign(gtk::Align::End)
        .build();
    label.upcast()
}

fn tool_line(title: &str, status: ToolStatus) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    let marker: gtk::Widget = match status {
        ToolStatus::Running => adw::Spinner::new().upcast(),
        ToolStatus::Completed => gtk::Image::from_icon_name("object-select-symbolic").upcast(),
        ToolStatus::Failed => {
            let image = gtk::Image::from_icon_name("dialog-warning-symbolic");
            image.add_css_class("warning");
            image.upcast()
        }
    };
    marker.add_css_class("dim-label");
    row.append(&marker);
    let label = gtk::Label::builder()
        .label(tool_label(title))
        .css_classes(["dim-label", "caption"])
        .build();
    row.append(&label);
    row.upcast()
}

fn notice_line(text: &str) -> gtk::Widget {
    gtk::Label::builder()
        .label(text)
        .css_classes(["dim-label", "caption"])
        .halign(gtk::Align::Center)
        .build()
        .upcast()
}

fn failure_line(text: &str) -> gtk::Widget {
    let row = gtk::Box::builder().spacing(8).build();
    let icon = gtk::Image::from_icon_name("dialog-error-symbolic");
    icon.add_css_class("error");
    icon.set_valign(gtk::Align::Start);
    row.append(&icon);
    let label = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .xalign(0.0)
        .selectable(true)
        .css_classes(["error"])
        .build();
    row.append(&label);
    row.upcast()
}
