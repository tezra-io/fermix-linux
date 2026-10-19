//! The chat's controller: connects to `companion.sock`, shakes hands, feeds what the daemon
//! sends into the timeline (`fermix_client::chat_wire::timeline`), sends what the timeline
//! answers, and reconnects after the session's backoff. The page draws `chat::view` of it.

use crate::app::App;
use crate::chat_link::{self, Link, LinkError};
use crate::daemon::fermix_home;
use fermix_client::chat_wire::protocol::{ClientEvent, Row, ServerEvent};
use fermix_client::chat_wire::session::{closes, Handshake, Lost, Session, HANDSHAKE_TIMEOUT};
use fermix_client::chat_wire::timeline::{self, Chat};
use gtk::glib;
use std::rc::Rc;

/// Events read while waiting for `server_hello` before the attempt is given up.
const HANDSHAKE_READ_CAP: usize = 16;

#[derive(Default)]
pub struct ChatWire {
    pub chat: Chat,
    pub session: Session,
    link: Option<Rc<Link>>,
    /// Bumped whenever the link is dropped, so a reader of an older link stops.
    generation: u64,
    /// The backoff's timer while it waits; an attempt started sooner removes it.
    backoff: Option<glib::SourceId>,
    expiry: Option<glib::SourceId>,
}

/// Wall-clock seconds since the epoch.
fn now() -> i64 {
    glib::real_time() / 1_000_000
}

/// Logs what a decode dropped: a page's unreadable messages (a warning), and the media,
/// previews, blocks and places its rows skipped (debug: an unknown block type is expected).
fn log_skipped<'a>(messages: usize, rows: impl Iterator<Item = &'a Row>) {
    if messages > 0 {
        glib::g_warning!("fermix", "a history page had {messages} unreadable rows");
    }
    for row in rows.filter(|row| row.skipped > 0) {
        let (seq, skipped) = (row.server_seq, row.skipped);
        glib::g_debug!("fermix", "row {seq} dropped {skipped} unreadable parts");
    }
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
            Err(LinkError::NotFound) => return self.chat_lost(Lost::NoSocket),
            Err(LinkError::Broken(why)) => return self.chat_lost(Lost::Broken(why)),
        };
        if !self.wire.borrow_mut().session.connected() {
            // The attempt ended while the connect ran; its socket is not this session's.
            link.close();
            return;
        }
        if let Err(e) = link.send(&ClientEvent::hello()) {
            return self.chat_lost(Lost::Broken(format!("{e:?}")));
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
                Err(e) => return Err(Lost::Broken(format!("{e:?}"))),
            };
            let outcome = self.wire.borrow_mut().session.hello(&event);
            match outcome {
                Handshake::Ok => return Ok(()),
                Handshake::Refused(sentence) => {
                    glib::g_warning!("fermix", "chat refused: {sentence}");
                    return Err(Lost::Broken(sentence));
                }
                Handshake::Lost(reason) => return Err(Lost::Broken(reason)),
                Handshake::Ignore => glib::g_debug!("fermix", "before hello: {event:?}"),
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
            timeline::negotiated(&mut wire.chat, &on_screen)
        };
        self.chat_send(events);
        self.media.connected(link.clone());
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
                Err(e) => return self.chat_lost(Lost::Broken(format!("{e:?}"))),
            }
        }
    }

    fn on_chat_event(self: &Rc<Self>, event: ServerEvent) {
        // A file's answer is for the reads waiting on it, never the timeline's.
        if self.media.answered(&event) {
            return;
        }
        match &event {
            ServerEvent::Unknown(kind) => glib::g_warning!("fermix", "chat event {kind} unread"),
            ServerEvent::HistoryPage(page) => log_skipped(page.skipped, page.messages.iter()),
            ServerEvent::Row(row) => log_skipped(0, std::iter::once(row)),
            ServerEvent::TextDone(reply) if reply.skipped > 0 => glib::g_debug!(
                "fermix",
                "reply {} dropped {} blocks or places",
                reply.server_seq,
                reply.skipped
            ),
            ServerEvent::Error(error) if closes(&error.reason) => {
                glib::g_warning!("fermix", "the chat was refused: {}", error.reason)
            }
            _ => {}
        }
        let events = timeline::receive(&mut self.wire.borrow_mut().chat, event, now());
        self.chat_send(events);
        self.chat_seen();
        self.keep_approval_clock();
        self.render();
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
            if let Err(e) = link.send(&event) {
                glib::g_warning!("fermix", "a chat event was not sent: {e:?}");
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

    pub fn answer_approval(&self, approval_id: &str, approve: bool) {
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
