//! The chat controller: sends what the composer holds, feeds the agent's
//! stream into the `Transcript`, stops a reply, and starts over. One ACP
//! session per conversation; Fermix cannot reload one, so a lost connection
//! means the next message starts a new conversation, and the page says so.

use crate::acp_link::{self, Link, LinkError};
use crate::app::App;
use crate::daemon::fermix_home;
use fermix_client::acp::{self, Incoming, StopReason};
use fermix_client::chat::{reply_error, Transcript};
use gtk::glib;
use std::rc::Rc;

#[derive(Default)]
pub struct Conversation {
    pub transcript: Transcript,
    link: Option<Rc<Link>>,
    /// The request id of the prompt whose reply is streaming.
    prompt_id: Option<u64>,
    /// Bumped whenever the link is dropped, so a reader of an older link stops.
    generation: u64,
}

/// Wall-clock seconds since the epoch, for the times under entries.
fn now() -> i64 {
    glib::real_time() / 1_000_000
}

impl App {
    pub async fn send_message(self: Rc<Self>) {
        let text = self.chat.input_text();
        if !self.conversation.borrow_mut().transcript.send(&text, now()) {
            return;
        }
        self.chat.clear_input();
        self.chat.follow_latest();
        self.render();
        // A click on Send leaves the keyboard where the next message goes.
        self.chat.focus_input();
        self.ask(&text).await;
    }

    /// Asks the question whose reply failed again, in place of that reply.
    pub async fn retry_reply(self: Rc<Self>) {
        let Some(text) = self.conversation.borrow_mut().transcript.retry() else {
            return;
        };
        self.chat.follow_latest();
        self.render();
        // The Retry button is gone with the failure it sat in.
        self.chat.focus_input();
        self.ask(&text).await;
    }

    /// Sends a question the transcript already shows, opening the chat
    /// connection first if there is none.
    async fn ask(self: &Rc<Self>, text: &str) {
        let link = match self.ensure_link().await {
            Ok(link) => link,
            Err(e) => {
                glib::g_warning!("fermix", "chat could not connect: {e:?}");
                return self.reply_failed(&e.sentence());
            }
        };
        let id = link.next_id();
        self.conversation.borrow_mut().prompt_id = Some(id);
        link.send(acp::prompt(id, &link.session_id, text));
    }

    async fn ensure_link(self: &Rc<Self>) -> Result<Rc<Link>, LinkError> {
        if let Some(link) = self.conversation.borrow().link.clone() {
            return Ok(link);
        }
        let socket = fermix_home().join("acp.sock");
        let home = glib::home_dir();
        let cwd = home
            .to_str()
            .ok_or_else(|| LinkError::Broken(format!("the home folder is not UTF-8: {home:?}")))?;
        let link = acp_link::open(&socket, cwd, env!("CARGO_PKG_VERSION")).await?;
        let generation = {
            let mut conversation = self.conversation.borrow_mut();
            conversation.link = Some(link.clone());
            conversation.generation
        };
        let reader = self.clone();
        let listening = link.clone();
        glib::spawn_future_local(async move { reader.read_replies(listening, generation).await });
        Ok(link)
    }

    /// Reads the agent's stream until the connection ends or a newer
    /// conversation replaces this one. Bounded by the connection's life.
    async fn read_replies(self: Rc<Self>, link: Rc<Link>, generation: u64) {
        loop {
            let message = link.read().await;
            if self.conversation.borrow().generation != generation {
                return;
            }
            match message {
                Ok(Some(incoming)) => self.on_incoming(&link, incoming),
                Ok(None) => return self.link_lost("Fermix closed the chat connection."),
                Err(e) => {
                    glib::g_warning!("fermix", "chat stream broke: {e:?}");
                    return self.link_lost("The chat connection to Fermix broke.");
                }
            }
        }
    }

    fn on_incoming(&self, link: &Rc<Link>, incoming: Incoming) {
        let prompt_id = self.conversation.borrow().prompt_id;
        match incoming {
            Incoming::Update { session_id, update } if session_id == link.session_id => {
                if !self.conversation.borrow_mut().transcript.apply(&update) {
                    glib::g_debug!("fermix", "chat update not shown: {update:?}");
                }
            }
            Incoming::Response { id, result } if Some(id) == prompt_id => {
                let mut conversation = self.conversation.borrow_mut();
                conversation.prompt_id = None;
                match StopReason::from_result(&result) {
                    Some(reason) => conversation.transcript.finish(&reason, now()),
                    None => conversation.transcript.fail(
                        "Fermix answered in a way this app does not understand.",
                        now(),
                    ),
                }
            }
            Incoming::Error { id, code, message } if id.is_some() && id == prompt_id => {
                glib::g_warning!("fermix", "the reply failed ({code}): {message}");
                let mut conversation = self.conversation.borrow_mut();
                conversation.prompt_id = None;
                let sentence = reply_error(code, &message).sentence();
                conversation.transcript.fail(&sentence, now());
            }
            Incoming::Request { id, method } => {
                glib::g_debug!("fermix", "declined the agent's {method} request");
                link.send(acp::reply_unsupported(&id));
            }
            other => glib::g_debug!("fermix", "chat message not for this page: {other:?}"),
        }
        self.render();
    }

    fn reply_failed(&self, sentence: &str) {
        self.conversation
            .borrow_mut()
            .transcript
            .fail(sentence, now());
        self.render();
    }

    /// The connection ended. A reply in progress fails with the reason; either
    /// way the next message opens a new session, which starts a new conversation.
    fn link_lost(&self, why: &str) {
        let replying = {
            let mut conversation = self.conversation.borrow_mut();
            conversation.link = None;
            conversation.prompt_id = None;
            conversation.generation += 1;
            !conversation.transcript.can_send()
        };
        if replying {
            let sentence = format!("{why} Your next message starts a new conversation.");
            return self.reply_failed(&sentence);
        }
        self.conversation.borrow_mut().transcript.note(
            "The connection to Fermix ended, so your next message starts a new conversation.",
        );
        self.render();
    }

    pub fn stop_reply(&self) {
        let mut conversation = self.conversation.borrow_mut();
        if !conversation.transcript.stop() {
            return;
        }
        if let Some(link) = conversation.link.clone() {
            link.send(acp::cancel(&link.session_id));
        }
        drop(conversation);
        self.render();
    }

    /// Closes the session and clears the page. Fermix stops a reply whose
    /// connection closes, so no separate cancel is sent.
    pub fn new_conversation(&self) {
        let link = {
            let mut conversation = self.conversation.borrow_mut();
            let link = conversation.link.take();
            let generation = conversation.generation + 1;
            *conversation = Conversation {
                generation,
                ..Conversation::default()
            };
            link
        };
        if let Some(link) = link {
            link.close();
        }
        self.chat.clear_input();
        self.chat.follow_latest();
        self.render();
        self.chat.focus_input();
    }
}
