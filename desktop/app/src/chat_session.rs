//! The chat's controller: connects to `companion.sock`, shakes hands, feeds what the daemon
//! sends into the timeline (`fermix_client::chat_wire::timeline`) and the owner's lists into the
//! lists' library (`fermix_client::checklists::library`), sends what each answers, and reconnects
//! after the session's backoff. The page draws `chat::view` of it, with the lists cards and the
//! Lists pane, whose owner's open or close it keeps in `$XDG_STATE_HOME` (M85 lists §3.2). It keeps
//! the removals made in the pane for Undo while the app runs, and the toast that offers it (§3.7).

use crate::app::App;
use crate::chat::{Decision, ListAct};
use crate::chat_link::{self, Link, LinkError};
use crate::daemon::fermix_home;
use crate::shell::plain_toast;
use adw::prelude::*;
use fermix_client::chat_wire::protocol::{
    loggable_id, Attachments, ClientEvent, Decision as Word, LineTooLong, Proposal, Reply, Row,
    ServerError, ServerEvent, PROTOCOL_VERSION,
};
use fermix_client::chat_wire::session::{
    closes, logged, page_read, Handshake, Lost, Session, HANDSHAKE_TIMEOUT,
};
use fermix_client::chat_wire::timeline::{self, Chat};
use fermix_client::checklists::library::Library;
use fermix_client::checklists::pane;
use fermix_client::checklists::removals::{self, Removal, Removals};
use fermix_client::checklists::routing::lists_event;
use fermix_client::checklists::words::{self, UNDO};
use fermix_client::phone::unix_seconds;
use gtk::{gio, glib};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// Events read while waiting for `server_hello` before the attempt is given up.
const HANDSHAKE_READ_CAP: usize = 16;
/// How long a removal's toast offers Undo, in seconds (M85 lists §3.7).
const UNDO_SECONDS: u32 = 6;

#[derive(Default)]
pub struct ChatWire {
    pub chat: Chat,
    /// The owner's lists this connection draws, and the edits made here.
    pub lists: Library,
    /// The removals made here that Undo can still take back, kept while the app runs.
    pub removals: Removals,
    /// The toast offering Undo of the newest removal, by its token, until another replaces it.
    pub undo_toast: Option<(u64, adw::Toast)>,
    /// A handshake has served lists since the app started: the Lists toggle shows on Chat from
    /// then on.
    pub lists_served: bool,
    pub session: Session,
    link: Option<Rc<Link>>,
    /// Bumped whenever the link is dropped, so a reader of an older link stops.
    generation: u64,
    /// The backoff's timer while it waits; an attempt started sooner removes it.
    backoff: Option<glib::SourceId>,
    expiry: Option<glib::SourceId>,
    /// The one timer for the proposals' expiry, and the second it waits for: moved only when
    /// the soonest expiry of the cards that ask moves.
    proposal_clock: Option<(i64, glib::SourceId)>,
}

/// Wall-clock seconds since the epoch.
fn now() -> i64 {
    glib::real_time() / 1_000_000
}

/// Logs what a decode dropped: a page's unreadable messages (a warning), the media, previews,
/// blocks and places its rows skipped (debug: an unknown block type is expected), and the rows
/// whose time this app cannot read (a warning, as they arrive: the page draws them with none).
fn log_skipped<'a>(messages: usize, rows: impl Iterator<Item = &'a Row>) {
    if messages > 0 {
        glib::g_warning!("fermix", "a history page had {messages} unreadable rows");
    }
    let mut untimed = 0;
    for row in rows {
        untimed += usize::from(!time_reads(row.ts.as_deref()));
        let (seq, skipped) = (row.server_seq, row.skipped);
        if skipped > 0 {
            glib::g_debug!("fermix", "row {seq} dropped {skipped} unreadable parts");
        }
    }
    if untimed > 0 {
        glib::g_warning!(
            "fermix",
            "{untimed} rows came with a time this app cannot read"
        );
    }
}

/// Logs what a streamed reply's decode dropped, as `log_skipped` does a row's.
fn log_reply(reply: &Reply) {
    let (seq, skipped) = (reply.server_seq, reply.skipped);
    if skipped > 0 {
        glib::g_debug!("fermix", "reply {seq} dropped {skipped} blocks or places");
    }
    if !time_reads(reply.ts.as_deref()) {
        glib::g_warning!(
            "fermix",
            "reply {seq} came with a time this app cannot read"
        );
    }
}

/// Whether a daemon's time is absent or one this app can read: one the timeline places by
/// (`unix_seconds`) and the page writes (`chat::local`). Any other is drawn as none, or as the
/// daemon wrote it on a card; the callers here log it once, as it arrives.
pub fn time_reads(ts: Option<&str>) -> bool {
    ts.is_none_or(|ts| unix_seconds(ts).is_some() && crate::chat::local(ts).is_some())
}

/// Logs a card whose times this app cannot all read, by its id alone.
fn log_card(proposal: &Proposal) {
    let times = [
        Some(proposal.created_at.as_str()),
        Some(proposal.expires_at.as_str()),
        proposal.received_at.as_deref(),
    ];
    if !times.into_iter().all(time_reads) {
        let id = loggable_id(&proposal.proposal_id);
        glib::g_warning!("fermix", "card {id} came with a time this app cannot read");
    }
}

/// Logs an event this build shows nothing of yet, by its type and a plain id alone.
fn log_unused(event: &ServerEvent) {
    match event {
        // Always On stays behind its flag, so this app shows none of it.
        ServerEvent::HomeStatusChanged => glib::g_debug!("fermix", "Always On changed"),
        ServerEvent::Reasoning { turn_id, .. } => {
            let turn = loggable_id(turn_id);
            glib::g_debug!("fermix", "turn {turn} reasoned; not shown yet")
        }
        ServerEvent::AttachStatus { .. } => {
            glib::g_warning!(
                "fermix",
                "a file's status came that this app never handed over"
            )
        }
        _ => {}
    }
}

/// Logs what of a hello's offers did not fit, by count: the hello still connects.
fn log_offers(hello: &ServerEvent) {
    let ServerEvent::ServerHello {
        commands,
        attachments,
        ..
    } = hello
    else {
        return;
    };
    if commands.skipped > 0 {
        let skipped = commands.skipped;
        glib::g_warning!(
            "fermix",
            "the hello offered {skipped} commands that could not be read"
        );
    }
    if *attachments == Attachments::Unreadable {
        glib::g_warning!(
            "fermix",
            "the hello's offer of attachments could not be read"
        );
    }
}

/// Logs an event that could not be read by its type alone: what it held is the daemon's, and may
/// be the owner's. The first of a type on a connection is a warning, the rest are debug.
fn log_unreadable(chat: &mut Chat, kind: &str) {
    if timeline::first_unreadable(chat, kind) {
        glib::g_warning!(
            "fermix",
            "chat event {kind} could not be read and was dropped"
        );
    } else {
        glib::g_debug!(
            "fermix",
            "chat event {kind} could not be read and was dropped"
        );
    }
}

/// Logs a refusal of this app's answer to a card Fermix had already resolved (`late_refusal`):
/// the card keeps its resolution and shows nothing of the refusal, so this line is its one trace.
/// The card is named only by `loggable_id`.
fn log_late_refusal(chat: &Chat, event: &ServerEvent) {
    let Some(card) = timeline::late_refusal(chat, event) else {
        return;
    };
    let id = loggable_id(card);
    glib::g_debug!(
        "fermix",
        "card {id} was resolved before its answer was refused"
    );
}

/// Why the link ended the connection, in its own words: a missing socket file, or what broke.
fn link_lost(error: LinkError) -> Lost {
    match error {
        LinkError::NotFound => Lost::NoSocket,
        LinkError::Broken(why) => Lost::Broken(why),
    }
}

/// Where the pane's open or close is kept: `$XDG_STATE_HOME/fermix-desktop/lists.json`.
fn lists_file() -> PathBuf {
    let state_home = std::env::var_os("XDG_STATE_HOME");
    pane::lists_path(state_home.as_deref(), &glib::home_dir())
}

/// A request id no other client uses: this app's prefix and 64 random bits.
fn message_id() -> String {
    format!("linux-{:08x}{:08x}", glib::random_int(), glib::random_int())
}

impl App {
    /// Starts the chat's connection now unless an attempt runs or it is connected: after a
    /// refusal, and before a waiting backoff is over, whose timer then goes.
    pub fn chat_connect(self: &Rc<Self>) {
        let generation = {
            let mut wire = self.wire.borrow_mut();
            if !wire.session.start() {
                return;
            }
            if let Some(backoff) = wire.backoff.take() {
                backoff.remove();
            }
            wire.generation
        };
        self.render();
        let app = self.clone();
        glib::spawn_future_local(async move { app.attempt(generation).await });
    }

    async fn attempt(self: Rc<Self>, generation: u64) {
        let socket = fermix_home().join("companion.sock");
        let link = match chat_link::connect(&socket).await {
            Ok(link) => link,
            Err(e) => return self.chat_lost(link_lost(e)),
        };
        if !self.wire.borrow_mut().session.connected() {
            // The attempt ended while the connect ran; its socket is not this session's.
            link.close();
            return;
        }
        if let Err(LineTooLong(bytes)) = link.send(&ClientEvent::hello()) {
            let why = format!("the hello of {bytes} bytes was not sent");
            return self.chat_lost(Lost::Broken(why));
        }
        let shaken = glib::future_with_timeout(HANDSHAKE_TIMEOUT, self.handshake(&link)).await;
        match shaken {
            Ok(Ok(())) => self.negotiated(link, generation),
            Ok(Err(lost)) => {
                link.close();
                self.chat_lost(lost);
            }
            Err(_) => {
                link.close();
                self.chat_lost(Lost::Broken("no server_hello in time".into()));
            }
        }
    }

    /// Reads until the handshake has an outcome. A refusal is shown and not retried.
    async fn handshake(&self, link: &Link) -> Result<(), Lost> {
        for _ in 0..HANDSHAKE_READ_CAP {
            let event = match link.read().await {
                Ok(Some(event)) => event,
                Ok(None) => return Err(Lost::Broken("closed during the handshake".into())),
                Err(e) => return Err(link_lost(e)),
            };
            let outcome = self.wire.borrow_mut().session.hello(&event);
            match outcome {
                Handshake::Ok => {
                    log_offers(&event);
                    return Ok(());
                }
                Handshake::Refused(_) => {
                    let named = logged(&event);
                    glib::g_warning!("fermix", "chat refused: {named}");
                    return Err(Lost::Broken(named));
                }
                Handshake::Lost(reason) => return Err(Lost::Broken(reason)),
                Handshake::Ignore => {
                    let kind = event.kind();
                    glib::g_debug!("fermix", "before hello: {kind}")
                }
            }
        }
        Err(Lost::Broken(
            "no server_hello among the first events".into(),
        ))
    }

    fn negotiated(self: &Rc<Self>, link: Rc<Link>, generation: u64) {
        // What the daemon changed while away is read again for the rows on screen; the rest
        // as the reader comes to them (`chat_rows_on_screen`).
        let on_screen = self.chat.rows_on_screen();
        let events = {
            let mut wire = self.wire.borrow_mut();
            wire.link = Some(link.clone());
            let mut events = timeline::negotiated(&mut wire.chat, &on_screen);
            events.extend(wire.lists.negotiated(PROTOCOL_VERSION));
            wire.lists_served |= wire.lists.index().served();
            events
        };
        self.chat_send(events);
        self.media.connected(link.clone());
        self.keep_proposal_clock();
        self.render();
        let app = self.clone();
        glib::spawn_future_local(async move { app.read_loop(link, generation).await });
    }

    /// Reads the daemon's events until the connection ends or a newer one replaces it.
    async fn read_loop(self: Rc<Self>, link: Rc<Link>, generation: u64) {
        loop {
            let read = link.read().await;
            if self.wire.borrow().generation != generation {
                return;
            }
            match read {
                Ok(Some(event)) => self.on_chat_event(event),
                Ok(None) => return self.chat_lost(Lost::Broken("Fermix closed the chat".into())),
                Err(e) => return self.chat_lost(link_lost(e)),
            }
        }
    }

    fn on_chat_event(self: &Rc<Self>, event: ServerEvent) {
        // A file's answer is for the reads waiting on it, never the timeline's.
        if self.media.answered(&event) {
            return;
        }
        self.log_event(&event);
        let listed = lists_event(&mut self.wire.borrow_mut().lists, &event);
        if let Some(events) = listed {
            self.chat_send(events);
            return self.render();
        }
        let read = page_read(&event);
        let events = timeline::receive(&mut self.wire.borrow_mut().chat, event, now());
        if timeline::history_unreadable(&self.wire.borrow().chat) {
            return self.chat_lost(Lost::History);
        }
        if read {
            self.wire.borrow_mut().session.history_read();
        }
        self.chat_send(events);
        self.chat_seen();
        self.keep_approval_clock();
        self.keep_proposal_clock();
        self.render();
        self.serve_reveal();
    }

    /// Logs what an event says of the wire, never the daemon's text: what did not decode, what
    /// was refused, and what this build shows nothing of. A type, an id or a code word the daemon
    /// chose is logged only as `loggable_id` lets it. Its raw value is bound under a name the
    /// log scan (media_grants.rs) does not allow, so a line without its `loggable_id` does not
    /// build.
    fn log_event(&self, event: &ServerEvent) {
        log_late_refusal(&self.wire.borrow().chat, event);
        match event {
            ServerEvent::Unknown(unknown) => {
                let kind = loggable_id(unknown);
                glib::g_warning!("fermix", "chat event {kind} unread")
            }
            ServerEvent::Unreadable(unread) => {
                log_unreadable(&mut self.wire.borrow_mut().chat, &unread.kind)
            }
            ServerEvent::HistoryPage(page) => log_skipped(page.skipped, page.messages.iter()),
            ServerEvent::Row(row) => log_skipped(0, std::iter::once(row)),
            ServerEvent::TextDone(reply) => log_reply(reply),
            ServerEvent::Proposal { proposal } => log_card(proposal),
            ServerEvent::Error(error) if closes(&error.reason) => {
                let closing = &error.reason;
                glib::g_warning!("fermix", "the chat was refused: {closing}")
            }
            ServerEvent::Error(ServerError {
                attach_id: Some(attach),
                ..
            }) => {
                let id = loggable_id(attach);
                glib::g_warning!("fermix", "an error came for file {id}, never handed over")
            }
            _ => log_unused(event),
        }
    }

    /// The connection ended: the timeline forgets what it asked, and the session waits out
    /// its backoff before the next attempt (none after a refusal).
    fn chat_lost(self: &Rc<Self>, why: Lost) {
        glib::g_debug!("fermix", "chat connection ended: {why:?}");
        let wait = {
            let mut wire = self.wire.borrow_mut();
            if let Some(link) = wire.link.take() {
                link.close();
            }
            wire.generation += 1;
            timeline::disconnected(&mut wire.chat);
            wire.lists.disconnected();
            wire.session.lost(why)
        };
        self.media.disconnected();
        self.render();
        let Some(wait) = wait else {
            return;
        };
        let app = self.clone();
        let source = glib::timeout_add_local_once(wait, move || {
            // Fired: there is no timer left to remove.
            app.wire.borrow_mut().backoff = None;
            app.chat_connect();
        });
        if let Some(stale) = self.wire.borrow_mut().backoff.replace(source) {
            stale.remove();
        }
    }

    /// Sends the timeline's answers on the live link. Without one, the timeline keeps what
    /// must arrive (the outbox) and asks again after the next handshake.
    fn chat_send(&self, events: Vec<ClientEvent>) {
        let Some(link) = self.wire.borrow().link.clone() else {
            return;
        };
        for event in events {
            if let Err(LineTooLong(bytes)) = link.send(&event) {
                glib::g_warning!("fermix", "a chat event of {bytes} bytes was not sent");
            }
        }
    }

    pub fn send_message(self: &Rc<Self>) {
        let text = self.chat.input_text();
        let (events, queued) = {
            let chat = &mut self.wire.borrow_mut().chat;
            let before = chat.pending.len();
            let events = timeline::send(chat, &text, message_id());
            (events, chat.pending.len() > before)
        };
        if queued {
            self.chat.clear_input();
        }
        self.chat_send(events);
        self.chat.follow_latest();
        self.render();
        // A click on Send leaves the keyboard where the next message goes.
        self.chat.focus_input();
    }

    pub fn stop_reply(&self) {
        let events = timeline::stop(&mut self.wire.borrow_mut().chat);
        self.chat_send(events);
        self.render();
    }

    /// Hands the page this controller, which its cards' buttons decide through by their own
    /// clicks. No window or application action decides (M60 cues §6 rule 3).
    pub fn chat_decide_here(self: &Rc<Self>) {
        let app = Rc::downgrade(self);
        self.chat.decide_with(move |decision| {
            if let Some(app) = app.upgrade() {
                app.chat_decide(decision);
            }
        });
    }

    /// Hands the page this controller, which the replies' lists cards send their edits and
    /// pulls to.
    pub fn chat_list_here(self: &Rc<Self>) {
        let app = Rc::downgrade(self);
        self.chat.list_with(move |asked| {
            if let Some(app) = app.upgrade() {
                app.chat_list(asked);
            }
        });
    }

    /// What a lists card or the Lists pane asks: lists pulled as they come on screen, an edit or a
    /// rewording drawn at once and sent, a new list sent, a removal or its Undo, the lists drawn
    /// again, the pane opened at a card's list, or the pane shown or hidden. A pull draws nothing:
    /// the page asks it while it holds its entries, and the list's arrival draws. A new list with a
    /// blank title sends nothing, nor one past 64 edits waiting, whose sentence the index then
    /// holds.
    fn chat_list(self: &Rc<Self>, asked: ListAct) {
        let events = match asked {
            ListAct::Drawn { list_ids, removed } => {
                let pulls = {
                    let lists = &mut self.wire.borrow_mut().lists;
                    list_ids
                        .iter()
                        .flat_map(|id| lists.draw(id, removed))
                        .collect()
                };
                return self.chat_send(pulls);
            }
            ListAct::Edit { list_id, op } => {
                let lists = &mut self.wire.borrow_mut().lists;
                lists.edit(&list_id, op, message_id())
            }
            ListAct::Create { title, folder } => {
                let lists = &mut self.wire.borrow_mut().lists;
                let made = lists.create(&title, folder.as_deref(), message_id());
                made.unwrap_or_default()
            }
            ListAct::Reword {
                list_id,
                began,
                words,
                note,
            } => {
                let lists = &mut self.wire.borrow_mut().lists;
                lists.reword(&list_id, &began, &words, note.as_deref(), message_id())
            }
            ListAct::Remove {
                list_id,
                row_id,
                text,
            } => return self.list_remove(&list_id, row_id.as_deref(), &text),
            ListAct::TakeBack(token) => return self.take_back(token),
            ListAct::Restore { list_id, row_id } => {
                return self.list_restore(&list_id, row_id.as_deref())
            }
            ListAct::Redraw => Vec::new(),
            ListAct::Open { list_id, by_key } => {
                self.chat.lists.open_at(Some(&list_id), by_key);
                Vec::new()
            }
            ListAct::Pane { shown, chosen } => return self.lists_shown(shown, chosen),
        };
        self.chat_send(events);
        self.render();
    }

    /// Remove, of the row `row_id` of `list_id` or of the list itself: drawn and sent as any edit,
    /// and a toast offers Undo (§3.7). Nothing is kept and no toast shows for a row the engine has
    /// not named yet, nor past 64 edits waiting, whose sentence the list then says.
    fn list_remove(self: &Rc<Self>, list_id: &str, row_id: Option<&str>, text: &str) {
        let made = {
            let wire = &mut *self.wire.borrow_mut();
            let (removals, lists) = (&mut wire.removals, &mut wire.lists);
            removals::remove(removals, lists, list_id, row_id, text, message_id())
        };
        let Some((token, events)) = made else {
            return self.render();
        };
        self.chat_send(events);
        self.render();
        self.undo_toast(token, text);
    }

    /// The toast offering Undo of the removal `token`, "Removed Lemons." with `text` as written,
    /// for 6 s at high priority, replacing the last Undo toast (§10). Its button takes the removal
    /// back by its own closure, not through `win.toast`, which carries only text.
    fn undo_toast(self: &Rc<Self>, token: u64, text: &str) {
        let toast = plain_toast(&words::removed(text));
        toast.set_button_label(Some(UNDO));
        toast.set_timeout(UNDO_SECONDS);
        toast.set_priority(adw::ToastPriority::High);
        let app = Rc::downgrade(self);
        toast.connect_button_clicked(move |_| {
            if let Some(app) = app.upgrade() {
                app.chat_list(ListAct::TakeBack(Some(token)));
            }
        });
        let last = self
            .wire
            .borrow_mut()
            .undo_toast
            .replace((token, toast.clone()));
        if let Some((_, last)) = last {
            last.dismiss();
        }
        self.shell.toasts.add_toast(toast);
    }

    /// Undo of the removal `token` names, a toast's, or with none the newest still kept, Ctrl+Z's:
    /// its restore is sent once, whichever asks first, and drawn when the engine pushes it.
    fn take_back(self: &Rc<Self>, token: Option<u64>) {
        let undone = {
            let wire = &mut *self.wire.borrow_mut();
            removals::undo(&mut wire.removals, &mut wire.lists, token, message_id())
        };
        let Some((removal, events)) = undone else {
            return;
        };
        self.taken_back(&removal);
        self.chat_send(events);
        self.render();
    }

    /// Restore of the removed row `row_id` of `list_id` from Recently removed, or of the list
    /// itself by the Undo on its removed page: its restore is sent, drawn when the engine pushes
    /// it, and its removal made here, if Undo can still take it back, is taken back with it.
    fn list_restore(self: &Rc<Self>, list_id: &str, row_id: Option<&str>) {
        let (taken, events) = {
            let wire = &mut *self.wire.borrow_mut();
            let (removals, lists) = (&mut wire.removals, &mut wire.lists);
            removals::restore(removals, lists, list_id, row_id, message_id())
        };
        if let Some(removal) = taken {
            self.taken_back(&removal);
        }
        self.chat_send(events);
        self.render();
    }

    /// A removal taken back: a list's turns the pane to it (§10), and its toast, if it still
    /// shows, is dismissed.
    fn taken_back(&self, removal: &Removal) {
        self.chat.lists.taken_back(removal);
        let mut wire = self.wire.borrow_mut();
        let shown = wire
            .undo_toast
            .take_if(|(token, _)| *token == removal.token);
        drop(wire);
        if let Some((_, toast)) = shown {
            toast.dismiss();
        }
    }

    /// `win.lists`, from the toggle or Ctrl+Shift+L: on Chat it shows or hides the pane, and
    /// from any other page it opens Chat with the pane shown (M85 lists §3.2). Focus moves to the
    /// picker unless the toggle, which holds the focus after a click, opened it.
    pub fn toggle_lists(&self) {
        if self.shell.visible_page().as_deref() == Some("chat") {
            let by_key = !self.shell.lists.is_focus();
            self.chat.lists.toggle(by_key);
        } else {
            self.shell.show_page("chat");
            self.chat.lists.open_at(None, true);
        }
        self.render();
    }

    /// `win.lists` asked to be `open`, as a SetState on the session bus asks: the pane shows or
    /// hides to match, an open from another page showing Chat as the toggle's press does.
    pub fn set_lists(&self, open: bool) {
        let on_chat = self.shell.visible_page().as_deref() == Some("chat");
        let shown = self.chat.lists.shown();
        if open && !(shown && on_chat) {
            return self.toggle_lists();
        }
        if !open && shown {
            self.chat.lists.close();
        }
        self.render();
    }

    /// The Lists toggle shows on Chat once a handshake has served lists, and `win.lists` works
    /// from then on. Its state is whether the pane shows, which it never does while the down
    /// page has the chat's place, and the toggle hides then too.
    pub fn show_lists_toggle(&self) {
        let served = self.wire.borrow().lists_served;
        let on_chat = self.shell.visible_page().as_deref() == Some("chat");
        let down = self.chat.is_down();
        self.shell.lists.set_visible(served && on_chat && !down);
        if let Some(action) = self.lists_action() {
            action.set_enabled(served);
            let shown = self.chat.lists.shown() && !down;
            action.set_state(&shown.to_variant());
        }
    }

    /// The owner's last open or close of the pane, kept since the last run: closed when nothing
    /// was kept, and when the file does not load, which is logged by its code alone, since the
    /// path holds the home folder.
    pub fn load_lists_pane(&self) {
        let open = match pane::load(&lists_file()) {
            Ok(open) => open,
            Err(code) => {
                glib::g_warning!(
                    "fermix",
                    "the Lists pane's open or close did not load: {code:?}"
                );
                false
            }
        };
        self.chat.lists.keep_open(open);
        self.lists_shown(open, None);
    }

    /// The pane was shown or hidden: the toggle follows, and an open or close the owner made is
    /// kept for the next run.
    fn lists_shown(&self, shown: bool, chosen: Option<bool>) {
        let shown = shown && !self.chat.is_down();
        match self.lists_action() {
            Some(action) => action.set_state(&shown.to_variant()),
            None => glib::g_warning!(
                "fermix",
                "the Lists pane changed with no action to follow it"
            ),
        }
        let Some(open) = chosen else {
            return;
        };
        if let Err(e) = pane::save(&lists_file(), open) {
            let code = e.kind();
            glib::g_warning!(
                "fermix",
                "the Lists pane's open or close was not kept: {code:?}"
            );
        }
    }

    fn lists_action(&self) -> Option<gio::SimpleAction> {
        self.shell
            .window
            .lookup_action("lists")
            .and_downcast::<gio::SimpleAction>()
    }

    fn chat_decide(self: &Rc<Self>, decision: Decision) {
        match decision {
            Decision::Approval { id, approve } => self.answer_approval(&id, approve),
            Decision::Proposal { id, decision } => self.answer_proposal(&id, decision),
        }
    }

    /// Sends the owner's word on a proposal, once, through the outbox: the card's id and the word.
    fn answer_proposal(self: &Rc<Self>, proposal_id: &str, decision: Word) {
        let events = {
            let chat = &mut self.wire.borrow_mut().chat;
            timeline::answer_proposal(chat, proposal_id, decision, message_id())
        };
        self.chat_send(events);
        self.keep_proposal_clock();
        self.render();
    }

    /// Brings a proposal's card into view in Chat, by its id: the window comes forward on Chat,
    /// and the card is scrolled to once it is drawn, now or when the daemon sends it. The Cues
    /// page's Show in chat and the tray's line call this; it never decides anything.
    pub fn chat_reveal(self: &Rc<Self>, proposal_id: &str) {
        timeline::reveal(&mut self.wire.borrow_mut().chat, proposal_id);
        self.shell.show_page("chat");
        self.shell.window.present();
        self.render();
        self.serve_reveal();
    }

    /// The page scrolls to a revealed card once it is laid out, and only then is the request
    /// taken back.
    fn serve_reveal(self: &Rc<Self>) {
        let app = Rc::downgrade(self);
        let served = move |id: &str| {
            if let Some(app) = app.upgrade() {
                timeline::revealed(&mut app.wire.borrow_mut().chat, id);
            }
        };
        self.chat.serve_reveal(&self.wire.borrow().chat, served);
    }

    fn answer_approval(&self, approval_id: &str, approve: bool) {
        let events = {
            let chat = &mut self.wire.borrow_mut().chat;
            timeline::answer(chat, approval_id, approve, message_id())
        };
        self.chat_send(events);
        self.render();
    }

    /// The reader reached the top of the conversation.
    pub fn chat_older(&self) {
        let events = timeline::pull_older(&mut self.wire.borrow_mut().chat);
        self.chat_send(events);
    }

    /// Held rows came on screen: those a reconnect may have changed are read again, once.
    pub fn chat_rows_on_screen(&self) {
        let seqs = self.chat.rows_on_screen();
        let events = timeline::rows_on_screen(&mut self.wire.borrow_mut().chat, &seqs);
        self.chat_send(events);
    }

    /// Moves the shared read frontier while the newest row is on screen in the active window.
    pub fn chat_seen(&self) {
        if !self.chat.reading_latest() {
            return;
        }
        let events = timeline::newest_seen(&mut self.wire.borrow_mut().chat);
        self.chat_send(events);
    }

    /// Keeps the one proposal timer on the soonest expiry of the cards that ask: left alone while
    /// that does not move, moved when it does, gone when no card asks. On firing it expires what
    /// it reached and waits for the next.
    fn keep_proposal_clock(self: &Rc<Self>) {
        let mut wire = self.wire.borrow_mut();
        let next = timeline::next_expiry(&wire.chat);
        if next == wire.proposal_clock.as_ref().map(|(at, _)| *at) {
            return;
        }
        if let Some((_, source)) = wire.proposal_clock.take() {
            source.remove();
        }
        let Some(at) = next else {
            return;
        };
        // Microseconds to the expiry's second, so the timer never fires before it.
        let wait = u64::try_from(at.saturating_mul(1_000_000) - glib::real_time()).unwrap_or(0);
        let app = Rc::downgrade(self);
        let source = glib::timeout_add_local_once(Duration::from_micros(wait), move || {
            let Some(app) = app.upgrade() else {
                return;
            };
            {
                let mut wire = app.wire.borrow_mut();
                // Fired: there is no timer left to remove.
                wire.proposal_clock = None;
                timeline::expire_proposals(&mut wire.chat, now());
            }
            app.keep_proposal_clock();
            app.render();
        });
        wire.proposal_clock = Some((at, source));
    }

    /// While cards wait, a second's clock drops each whose own ttl ran out.
    fn keep_approval_clock(self: &Rc<Self>) {
        let mut wire = self.wire.borrow_mut();
        if wire.chat.approvals.is_empty() || wire.expiry.is_some() {
            return;
        }
        let app = Rc::downgrade(self);
        let source = glib::timeout_add_seconds_local(1, move || {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut wire = app.wire.borrow_mut();
            timeline::expire_approvals(&mut wire.chat, now());
            let empty = wire.chat.approvals.is_empty();
            if empty {
                wire.expiry = None;
            }
            drop(wire);
            app.render();
            if empty {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        wire.expiry = Some(source);
    }
}
