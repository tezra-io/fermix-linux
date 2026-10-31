//! Chat: the conversation, drawn from `chat::view` of the chat wire's timeline,
//! the cards waiting for the owner's answer, and the composer under them. The
//! page never talks to Fermix; its buttons fire window actions and
//! `chat_session.rs` does the work.

mod activity;
mod approvals;
mod blocks;
mod entries;
mod levels;
mod picture;
mod places;
mod playback;
mod strip;
mod table_scroller;
mod voice_chip;
mod voice_note;

use crate::audio::Endpoints;
use crate::chat_session::ChatWire;
use crate::media_grants::Media;
use crate::state::{Connection, State};
use crate::status::{down_view, waiting, DownPage, DownView};
use activity::{Live, LiveLine};
use adw::prelude::*;
use entries::Drawn;
use fermix_client::chat::{view, Entry, Key, LiveTurn, View};
use fermix_client::chat_wire::session::{Shown, NO_CHAT};
use fermix_client::greeting::{greeting, time_of_day, QUIET_LINE};
use fermix_client::view::DaemonProblem;
use gtk::gdk;
use gtk::glib;
use picture::Decoder;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;
use strip::Readers;
use voice_note::Player;

/// Past this distance from the bottom, new text no longer pulls the view down.
const STICK_DISTANCE: f64 = 48.0;
/// What Chat cannot do without Fermix, on the down page.
const NEEDS_FERMIX: &str = "Chat needs Fermix running. Start it to ask anything.";

/// What the page says while Fermix is out of reach: its problem, or that it is
/// starting. `None` while it answers, or before the first read.
fn down_state(state: &State) -> Option<DownView> {
    match &state.connection {
        // Fermix here runs: chat says what its own connection says, the server's sentence.
        Connection::Down(DaemonProblem::PrimaryUnreachable(_)) => None,
        Connection::Down(problem) if !state.waking => Some(down_view(
            problem,
            state.wake_failed,
            state.background.binding,
            NEEDS_FERMIX,
        )),
        Connection::Down(_) => Some(waiting("Starting Fermix")),
        Connection::Up(_) | Connection::Connecting => None,
    }
}

/// Whether Fermix on this computer answers. After a move it does while the server it forwards
/// to is unreachable, and what is typed then waits in the outbox until the server is back.
fn fermix_here(state: &State) -> bool {
    matches!(
        &state.connection,
        Connection::Up(_) | Connection::Down(DaemonProblem::PrimaryUnreachable(_))
    )
}

pub struct ChatPage {
    pub root: gtk::Stack,
    conversation: gtk::Stack,
    empty: adw::StatusPage,
    list: gtk::Box,
    /// The live line for a turn with nothing else on screen showing the work.
    thinking: LiveLine,
    shown: Rc<RefCell<Vec<Placed>>>,
    on_screen: Rc<OnScreen>,
    /// The running turn, which the clock reads each second.
    turn: Rc<Cell<Option<Turn>>>,
    clock: Rc<RefCell<Option<glib::SourceId>>>,
    follow: Follow,
    input: gtk::TextView,
    send: gtk::Button,
    down: DownPage,
    banner: adw::Banner,
    approvals: approvals::Cards,
    /// The oldest seq drawn, to keep the reader's place when older rows come in above.
    first_seq: Cell<Option<u64>>,
    /// What the rows' strips read their files with.
    readers: Readers,
}

impl ChatPage {
    /// `endpoints` are the voice notes' speakers: the sound server's, or a fakesink under
    /// `FERMIX_AUDIO=test`.
    pub fn new(media: &Media, endpoints: Endpoints) -> Self {
        // Close within a turn; a question opens the next with more room (CSS).
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let thinking = LiveLine::new();
        thinking.row.set_halign(gtk::Align::Start);
        thinking.row.add_css_class("standalone");
        thinking.row.set_visible(false);
        let (follow, conversation, empty) = conversation_area(&list, &thinking.row);
        let shown = Rc::default();
        let on_screen = OnScreen::new(&shown, &follow.scroller);
        let (composer, input, send) = composer();
        let approvals = approvals::Cards::new();
        let body = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        let banner = adw::Banner::new("");
        body.append(&banner);
        body.append(&conversation);
        body.append(&approvals.widget);
        body.append(&composer);
        let down = DownPage::new();
        let root = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        root.add_named(&body, Some("chat"));
        root.add_named(&down.page, Some("down"));
        let player = Player::new(media, endpoints);
        // Leaving the page, or the window going, stops a voice note.
        let stops = Rc::downgrade(&player);
        root.connect_unmap(move |_| {
            if let Some(player) = stops.upgrade() {
                player.stop();
            }
        });
        ChatPage {
            root,
            conversation,
            empty,
            list,
            thinking,
            shown,
            on_screen,
            turn: Rc::default(),
            clock: Rc::default(),
            follow,
            input,
            send,
            down,
            banner,
            approvals,
            first_seq: Cell::default(),
            readers: Readers {
                media: media.clone(),
                decoder: Decoder::new(fermix_client::picture::decode),
                player,
            },
        }
    }

    /// `name` is the first name About you holds, for the greeting.
    pub fn render(&self, wire: &ChatWire, state: &State, name: Option<&str>) {
        let shown = view(&wire.chat);
        let down = down_state(state);
        if let Some(down) = down.clone().filter(|_| shown.entries.is_empty()) {
            self.down.show(down);
            self.root.set_visible_child_name("down");
            return;
        }
        self.down.reset();
        self.root.set_visible_child_name("chat");
        self.show_banner(down.as_ref(), &wire.session.shown());
        self.approvals.show(&wire.chat);
        let open = fermix_here(state) && wire.session.queues();
        self.render_conversation(&shown, open, name);
    }

    /// With a conversation on screen the page stays, and the banner says why the
    /// composer waits and offers the way out the down page would. With Fermix up,
    /// it says why the chat is not connected, in the session's words.
    fn show_banner(&self, down: Option<&DownView>, shown: &Shown) {
        if let Some(view) = down {
            self.banner.set_revealed(true);
            self.banner.set_title(view.title());
            let (label, action) = view.button().unzip();
            self.banner.set_button_label(label);
            self.banner.set_action_name(action);
            return;
        }
        let sentence = match shown {
            Shown::NoChat => Some(NO_CHAT.to_owned()),
            Shown::Disconnected(sentence) | Shown::Refused(sentence) => Some(sentence.clone()),
            Shown::Idle | Shown::Connecting | Shown::Connected => None,
        };
        self.banner.set_revealed(sentence.is_some());
        self.banner
            .set_title(sentence.as_deref().unwrap_or_default());
        self.banner.set_button_label(None);
        self.banner.set_action_name(None);
    }

    /// Whether the newest entry is on screen in the window the person is using, so the shared
    /// read frontier may move: not while the window is behind another or on another workspace.
    pub fn reading_latest(&self) -> bool {
        let active = self
            .root
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| window.is_active());
        active && self.root.is_mapped() && self.follow.stuck.get()
    }

    /// Whether a voice call is up: while one is, no voice note plays.
    pub fn call_up(&self, up: bool) {
        self.readers.player.call(up);
    }

    /// The held rows drawn on screen now; none while the conversation is not.
    pub fn rows_on_screen(&self) -> BTreeSet<u64> {
        self.on_screen.seqs()
    }

    /// `open`: the composer takes a message, which Fermix here answers and the chat has not
    /// refused for good.
    fn render_conversation(&self, shown: &View, open: bool, name: Option<&str>) {
        // The time of day on this computer's clock; each render follows it.
        let time = match glib::DateTime::now_local() {
            Ok(now) => Some(time_of_day(now.hour() as u32)),
            Err(e) => {
                glib::g_warning!("fermix", "the local time cannot be read: {e}");
                None
            }
        };
        self.empty.set_title(&greeting(time, name));
        let empty = shown.entries.is_empty();
        self.conversation
            .set_visible_child_name(if empty { "empty" } else { "messages" });
        let turn = shown.turn.map(Turn::of);
        self.turn.set(turn);
        let live = turn.map(Turn::live);
        if self.first_seq.replace(shown.first_seq) > shown.first_seq {
            self.follow.keep_place();
        }
        self.show_entries(shown, live.as_ref());
        self.on_screen.notice();
        self.thinking.row.set_visible(shown.thinking);
        if let Some(live) = &live {
            self.thinking.thinking(live);
        }
        self.keep_time(turn.is_some());
        self.show_composer(shown.turn, open);
    }

    /// Send, or Stop while a turn this app started runs.
    fn show_composer(&self, turn: Option<LiveTurn>, up: bool) {
        let running = turn.is_some_and(|turn| turn.can_stop);
        let stopping = turn.is_some_and(|turn| turn.stopping);
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
            .set_sensitive(up && !stopping && (running || has_text));
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

    /// Draws what changed, each entry kept by its key: one already on screen stays where it
    /// is, one that streamed more in or changed is updated in place, a new one is drawn at its
    /// place, and one gone is removed. A page read backward is a prepend: nothing below it
    /// is drawn again.
    fn show_entries(&self, view: &View, live: Option<&Live>) {
        let mut shown = self.shown.borrow_mut();
        let mut kept = self.take_kept(&mut shown, &view.keys);
        let mut previous: Option<gtk::Widget> = None;
        let placed = view.keys.iter().zip(&view.entries).zip(&view.seqs);
        for ((key, entry), seq) in placed {
            let drawn = match kept.remove(key) {
                Some(drawn) => self.redrawn(drawn, entry),
                None => Drawn::new(entry, &self.readers),
            };
            place(&self.list, &drawn.row, previous.as_ref());
            previous = Some(drawn.row.clone().upcast());
            shown.push(Placed {
                key: key.clone(),
                seq: *seq,
                drawn,
            });
        }
        // Every key kept is one the view has, and the loop took each at its first place.
        debug_assert!(kept.is_empty(), "an entry kept that the view does not have");
        decorate(&shown, view, live);
    }

    /// Empties `shown` into its entries by key, and removes from the list each entry `keys`
    /// no longer has, and any second entry under one key.
    fn take_kept(&self, shown: &mut Vec<Placed>, keys: &[Key]) -> HashMap<Key, Drawn> {
        let wanted: HashSet<&Key> = keys.iter().collect();
        let mut kept = HashMap::with_capacity(shown.len());
        for placed in shown.drain(..) {
            if !wanted.contains(&placed.key) {
                self.list.remove(&placed.drawn.row);
                continue;
            }
            if let Some(twin) = kept.insert(placed.key, placed.drawn) {
                self.list.remove(&twin.row);
            }
        }
        kept
    }

    /// `drawn` grown into `entry`, or drawn anew when it cannot be: a different kind of entry
    /// under the same key, or a row read again that draws differently.
    fn redrawn(&self, mut drawn: Drawn, entry: &Entry) -> Drawn {
        if drawn.entry == *entry || drawn.grow(entry) {
            return drawn;
        }
        self.list.remove(&drawn.row);
        Drawn::new(entry, &self.readers)
    }

    /// Every second while a reply runs, the live line's time moves on. The
    /// clock stops itself at the first tick with no reply running.
    fn keep_time(&self, running: bool) {
        if !running || self.clock.borrow().is_some() {
            return;
        }
        let (turn, shown) = (self.turn.clone(), self.shown.clone());
        let (thinking, clock) = (self.thinking.clone(), self.clock.clone());
        let source = glib::timeout_add_seconds_local(1, move || {
            let Some(turn) = turn.get() else {
                clock.take();
                return glib::ControlFlow::Break;
            };
            let live = turn.live();
            thinking.thinking(&live);
            if let Some(last) = shown.borrow().last() {
                last.drawn.set_live(Some(&live));
            }
            glib::ControlFlow::Continue
        });
        self.clock.replace(Some(source));
    }
}

/// The running turn: when its question was asked, and whether Stop was pressed.
#[derive(Debug, Clone, Copy)]
struct Turn {
    started: i64,
    stopping: bool,
}

impl Turn {
    fn of(turn: LiveTurn) -> Turn {
        Turn {
            started: turn.started,
            stopping: turn.stopping,
        }
    }

    fn live(self) -> Live {
        Live {
            stopping: self.stopping,
            elapsed: glib::real_time() / 1_000_000 - self.started,
        }
    }
}

/// One entry on screen: its key, the held row it draws, and its widgets.
struct Placed {
    key: Key,
    seq: Option<u64>,
    drawn: Drawn,
}

/// Puts `row` straight after `after`, or first for `None`, moving it only when it is not
/// there already.
fn place(list: &gtk::Box, row: &impl IsA<gtk::Widget>, after: Option<&gtk::Widget>) {
    if row.parent().is_none() {
        list.insert_child_after(row, after);
        return;
    }
    if row.prev_sibling().as_ref() != after {
        list.reorder_child_after(row, after);
    }
}

/// The parts of each entry that follow the rest of the conversation; only the
/// last entry can be live.
fn decorate(shown: &[Placed], view: &View, live: Option<&Live>) {
    let last = shown.len().checked_sub(1);
    for (index, placed) in shown.iter().enumerate() {
        let is_last = Some(index) == last;
        let time = view.times.get(index).cloned().flatten();
        let time = time.as_deref().and_then(clock);
        placed
            .drawn
            .decorate(time.as_deref(), live.filter(|_| is_last));
    }
}

/// "14:05" in the local time zone, from the daemon's RFC 3339 time.
fn clock(at: &str) -> Option<String> {
    let parsed = glib::DateTime::from_iso8601(at, None)
        .and_then(|time| time.to_local())
        .and_then(|time| time.format("%H:%M"));
    match parsed {
        Ok(text) => Some(text.into()),
        Err(e) => {
            glib::g_warning!("fermix", "could not show the time {at}: {e}");
            None
        }
    }
}

fn conversation_area(list: &gtk::Box, live: &gtk::Box) -> (Follow, gtk::Stack, adw::StatusPage) {
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(18)
        .margin_end(18)
        .build();
    column.append(list);
    column.append(live);
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
    // The reader reached the oldest row drawn: ask for the page before it.
    scroller.connect_edge_reached(|scroller, edge| {
        if edge != gtk::PositionType::Top {
            return;
        }
        if let Err(e) = scroller.activate_action("win.chat-older", None) {
            glib::g_warning!("fermix", "could not ask for older messages: {e}");
        }
    });
    // The mark and a greeting (plan §3.4); the page fills in the title.
    let empty = adw::StatusPage::builder()
        .icon_name("io.tezra.Fermix-symbolic")
        .description(QUIET_LINE)
        .css_classes(["chat-empty"])
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
/// Coming back to the bottom marks the newest row read.
struct Follow {
    stuck: Rc<Cell<bool>>,
    adjustment: gtk::Adjustment,
    scroller: gtk::ScrolledWindow,
}

impl Follow {
    fn new(scroller: &gtk::ScrolledWindow, jump: &gtk::Button) -> Follow {
        let follow = Follow {
            stuck: Rc::new(Cell::new(true)),
            adjustment: scroller.vadjustment(),
            scroller: scroller.clone(),
        };
        let (stuck, button) = (follow.stuck.clone(), jump.clone());
        follow.adjustment.connect_value_changed(move |a| {
            let at_bottom = a.value() + a.page_size() >= a.upper() - STICK_DISTANCE;
            let was = stuck.replace(at_bottom);
            button.set_visible(!at_bottom);
            if at_bottom && !was {
                newest_seen(&button);
            }
        });
        let stuck = follow.stuck.clone();
        follow
            .adjustment
            .connect_changed(move |a| scroll_to_end_when_idle(a, &stuck));
        let (stuck, adjustment) = (follow.stuck.clone(), follow.adjustment.clone());
        jump.connect_clicked(move |button| {
            let was = stuck.replace(true);
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
            if !was {
                newest_seen(button);
            }
        });
        follow
    }

    /// Rows are about to come in above: keep the reader where they were, by their distance
    /// from the bottom, once the new rows are laid out.
    fn keep_place(&self) {
        if self.stuck.get() {
            return;
        }
        let a = self.adjustment.clone();
        let from_bottom = a.upper() - a.value();
        glib::idle_add_local_once(move || a.set_value(a.upper() - from_bottom));
    }

    fn latest(&self) {
        self.stuck.set(true);
        let a = &self.adjustment;
        a.set_value(a.upper() - a.page_size());
    }
}

/// The reader is back at the newest row: the window marks it read.
fn newest_seen(widget: &impl IsA<gtk::Widget>) {
    if let Err(e) = widget.activate_action("win.chat-seen", None) {
        glib::g_warning!("fermix", "could not mark the newest message read: {e}");
    }
}

/// Which held rows are drawn on screen, told to the window each time that changes, so a row a
/// reconnect may have changed is read again as the reader comes to it (macOS `rowsOnScreen`).
/// Each entry on screen also reads the pictures under it, once.
struct OnScreen {
    shown: Rc<RefCell<Vec<Placed>>>,
    scroller: gtk::ScrolledWindow,
    /// What the window was last told; empty while the conversation is off screen.
    told: RefCell<BTreeSet<u64>>,
    /// A look is due at the next idle, once the layout has settled.
    due: Cell<bool>,
}

impl OnScreen {
    fn new(shown: &Rc<RefCell<Vec<Placed>>>, scroller: &gtk::ScrolledWindow) -> Rc<OnScreen> {
        let on_screen = Rc::new(OnScreen {
            shown: shown.clone(),
            scroller: scroller.clone(),
            told: RefCell::default(),
            due: Cell::new(false),
        });
        let adjustment = scroller.vadjustment();
        let weak = Rc::downgrade(&on_screen);
        adjustment.connect_value_changed(move |_| notice(&weak));
        let weak = Rc::downgrade(&on_screen);
        adjustment.connect_changed(move |_| notice(&weak));
        let weak = Rc::downgrade(&on_screen);
        scroller.connect_map(move |_| notice(&weak));
        let told = Rc::downgrade(&on_screen);
        scroller.connect_unmap(move |_| {
            if let Some(on_screen) = told.upgrade() {
                on_screen.told.take();
            }
        });
        on_screen
    }

    /// Something moved: once the layout settles, the window hears of a change.
    fn notice(self: &Rc<Self>) {
        if self.due.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(on_screen) = weak.upgrade() else {
                return;
            };
            on_screen.due.set(false);
            on_screen.read();
            on_screen.tell();
        });
    }

    /// Each entry on screen reads what its strip shows, where it has not yet.
    fn read(&self) {
        let shown = self.shown.borrow();
        for placed in &shown[self.visible(&shown)] {
            placed.drawn.on_screen();
        }
    }

    fn tell(&self) {
        let seqs = self.seqs();
        if *self.told.borrow() == seqs {
            return;
        }
        self.told.replace(seqs);
        if let Err(e) = self.scroller.activate_action("win.chat-on-screen", None) {
            glib::g_warning!("fermix", "could not report the messages on screen: {e}");
        }
    }

    /// The held rows whose entries overlap the scroller's view now.
    fn seqs(&self) -> BTreeSet<u64> {
        let shown = self.shown.borrow();
        shown[self.visible(&shown)]
            .iter()
            .filter_map(|placed| placed.seq)
            .collect()
    }

    /// The entries of `shown` that overlap the scroller's view now; none while it is off
    /// screen. Entries stand in order down the list, so the first on screen is found by halving.
    fn visible(&self, shown: &[Placed]) -> Range<usize> {
        if !self.scroller.is_mapped() {
            return 0..0;
        }
        let height = self.scroller.height() as f32;
        let span = |placed: &Placed| {
            let bounds = placed.drawn.row.compute_bounds(&self.scroller)?;
            Some((bounds.y(), bounds.y() + bounds.height()))
        };
        let above = |placed: &Placed| span(placed).is_some_and(|(_, bottom)| bottom <= 0.0);
        let first = shown.partition_point(above);
        let count = shown[first..]
            .iter()
            .take_while(|placed| span(placed).is_some_and(|(top, _)| top < height))
            .count();
        first..first + count
    }
}

fn notice(on_screen: &std::rc::Weak<OnScreen>) {
    if let Some(on_screen) = on_screen.upgrade() {
        on_screen.notice();
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

fn composer() -> (adw::Clamp, gtk::TextView, gtk::Button) {
    let input = gtk::TextView::builder()
        .css_classes(["inline"])
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .hexpand(true)
        .build();
    input.update_property(&[gtk::accessible::Property::Label("Message Fermix")]);
    let placeholder = gtk::Label::builder()
        .label("Message Fermix")
        .css_classes(["dimmed"])
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

/// Enter or Ctrl+Enter sends, Shift+Enter starts a new line, Escape stops a
/// reply, and the placeholder and Send button follow whether there is anything
/// to send.
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
    let stop = send.clone();
    keys.connect_key_pressed(move |controller, key, _, modifiers| {
        // Escape does what the Stop button does, while it is Stop.
        if key == gdk::Key::Escape {
            let stopping = stop.action_name().as_deref() == Some("win.stop-reply");
            if stopping && stop.is_sensitive() {
                stop.activate();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
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
