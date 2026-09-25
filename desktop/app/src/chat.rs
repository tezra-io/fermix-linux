//! Chat: the conversation, drawn from `Transcript`, and the composer under it.
//! The page never talks to Fermix; its buttons fire window actions and
//! `conversation.rs` does the work.

mod entries;

use crate::state::{Connection, State};
use crate::status::{down_view, waiting, DownPage};
use adw::prelude::*;
use entries::Drawn;
use fermix_client::chat::{Phase, Transcript};
use fermix_client::view::answers_with;
use gtk::gdk;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Past this distance from the bottom, new text no longer pulls the view down.
const STICK_DISTANCE: f64 = 48.0;

pub struct ChatPage {
    pub root: gtk::Stack,
    conversation: gtk::Stack,
    empty: adw::StatusPage,
    list: gtk::Box,
    orb: gtk::Box,
    shown: RefCell<Vec<Drawn>>,
    follow: Follow,
    input: gtk::TextView,
    send: gtk::Button,
    down: DownPage,
}

impl ChatPage {
    pub fn new() -> Self {
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(12)
            .build();
        let orb = orb_row();
        let (follow, conversation, empty) = conversation_area(&list, &orb);
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
        ChatPage {
            root,
            conversation,
            empty,
            list,
            orb,
            shown: RefCell::default(),
            follow,
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
        self.show_entries(transcript);
        self.orb.set_visible(transcript.thinking());
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

    /// Scrolls to the newest entry and keeps following it, as after sending.
    pub fn follow_latest(&self) {
        self.follow.latest();
    }

    /// Draws what changed: entries already on screen stay, one that streamed
    /// more in or changed its status is updated in place, and everything from
    /// the first entry that cannot be is drawn anew.
    fn show_entries(&self, transcript: &Transcript) {
        let entries = transcript.entries();
        let mut shown = self.shown.borrow_mut();
        let mut same = 0;
        for (drawn, entry) in shown.iter_mut().zip(entries) {
            if drawn.entry != *entry && !drawn.grow(entry) {
                break;
            }
            same += 1;
        }
        for stale in shown.drain(same..) {
            self.list.remove(&stale.row);
        }
        for entry in &entries[same..] {
            let drawn = Drawn::new(entry);
            self.list.append(&drawn.row);
            shown.push(drawn);
        }
        decorate(&shown, transcript);
    }
}

/// The parts of each entry that follow the rest of the conversation.
fn decorate(shown: &[Drawn], transcript: &Transcript) {
    let last = shown.len().checked_sub(1);
    let replying = transcript.phase() != Phase::Idle;
    for (index, drawn) in shown.iter().enumerate() {
        let is_last = Some(index) == last;
        let time = transcript.time(index).and_then(clock);
        drawn.decorate(
            time.as_deref(),
            is_last && transcript.can_retry(),
            is_last && replying,
        );
    }
}

/// "14:05" in the local time zone.
fn clock(at: i64) -> Option<String> {
    match glib::DateTime::from_unix_local(at).and_then(|time| time.format("%H:%M")) {
        Ok(text) => Some(text.into()),
        Err(e) => {
            glib::g_warning!("fermix", "could not show the time {at}: {e}");
            None
        }
    }
}

fn conversation_area(list: &gtk::Box, orb: &gtk::Box) -> (Follow, gtk::Stack, adw::StatusPage) {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(18)
        .margin_end(18)
        .build();
    column.append(list);
    column.append(orb);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .child(&column)
        .build();
    // Natural heights: by default a viewport lays its content out at minimum
    // height, which squeezes every picture once the conversation outgrows it.
    let viewport = gtk::Viewport::builder()
        .vscroll_policy(gtk::ScrollablePolicy::Natural)
        .child(&clamp)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&viewport)
        .build();
    let jump = jump_button();
    let overlay = gtk::Overlay::builder().child(&scroller).build();
    overlay.add_overlay(&jump);
    let follow = Follow::new(&scroller, &jump);
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
    stack.add_named(&overlay, Some("messages"));
    (follow, stack, empty)
}

/// Floats over the conversation's corner while the reader is scrolled up.
fn jump_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name("go-bottom-symbolic")
        .tooltip_text("Jump to Latest")
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .margin_end(18)
        .margin_bottom(12)
        .visible(false)
        .css_classes(["circular", "osd"])
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Jump to latest message")]);
    button
}

/// Keeps the newest entry in view while the reader is at the bottom, leaves
/// the view alone once they scroll up to read, and offers a way back down.
struct Follow {
    stuck: Rc<Cell<bool>>,
    adjustment: gtk::Adjustment,
}

impl Follow {
    fn new(scroller: &gtk::ScrolledWindow, jump: &gtk::Button) -> Follow {
        let follow = Follow {
            stuck: Rc::new(Cell::new(true)),
            adjustment: scroller.vadjustment(),
        };
        let (stuck, button) = (follow.stuck.clone(), jump.clone());
        follow.adjustment.connect_value_changed(move |a| {
            let at_bottom = a.value() + a.page_size() >= a.upper() - STICK_DISTANCE;
            stuck.set(at_bottom);
            button.set_visible(!at_bottom);
        });
        let stuck = follow.stuck.clone();
        follow
            .adjustment
            .connect_changed(move |a| scroll_to_end_when_idle(a, &stuck));
        let (stuck, adjustment) = (follow.stuck.clone(), follow.adjustment.clone());
        jump.connect_clicked(move |_| {
            stuck.set(true);
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
        });
        follow
    }

    fn latest(&self) {
        self.stuck.set(true);
        let a = &self.adjustment;
        a.set_value(a.upper() - a.page_size());
    }
}

/// The content grew: follow it down while the reader is at the bottom. The
/// scroll waits for an idle: this signal comes while the viewport lays out,
/// and a value set then is only drawn at the next layout.
fn scroll_to_end_when_idle(adjustment: &gtk::Adjustment, stuck: &Rc<Cell<bool>>) {
    if !stuck.get() {
        return;
    }
    let (a, stuck) = (adjustment.clone(), stuck.clone());
    glib::idle_add_local_once(move || {
        if stuck.get() {
            a.set_value(a.upper() - a.page_size());
        }
    });
}

/// A small breathing orb in Fermix's place while it works with nothing else
/// on screen moving.
fn orb_row() -> gtk::Box {
    let orb = entries::orb();
    let row = gtk::Box::builder()
        .accessible_role(gtk::AccessibleRole::Status)
        .tooltip_text("Fermix is thinking")
        .halign(gtk::Align::Start)
        .visible(false)
        .build();
    row.update_property(&[gtk::accessible::Property::Label("Fermix is thinking")]);
    row.append(&orb);
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
    // Its side margins sit inside the clamp, as the conversation column's do,
    // so the composer lines up with the bubbles above it.
    let row = gtk::Box::builder()
        .spacing(6)
        .margin_start(18)
        .margin_end(18)
        .css_classes(["chat-composer"])
        .build();
    row.append(&field);
    row.append(&send);
    wire_composer(&input, &placeholder, &send);
    let clamp = adw::Clamp::builder()
        .maximum_size(760)
        .margin_bottom(18)
        .child(&row)
        .build();
    (clamp, input, send)
}

/// Enter or Ctrl+Enter sends, Shift+Enter starts a new line, and the
/// placeholder and Send button follow whether there is anything to send.
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
        let plain = !modifiers.contains(gdk::ModifierType::SHIFT_MASK);
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
