//! The app's side of the grants (`fermix_client::chat_wire::grants`): a read of a phone-side
//! file awaits the daemon's answer and owns a `Hold` for as long as it reads, and dropping the
//! hold lets the file go. The connection's controller (`chat_session.rs`) hands this the
//! negotiated link and the daemon's answers; the chat page reads through it.

use crate::chat_link::Link;
use fermix_client::chat_wire::grants::{Answer, Answered, Grants, Outcome, Ticket};
use fermix_client::chat_wire::protocol::{loggable_id, ClientEvent, ServerEvent};
use futures_channel::oneshot;
use gtk::glib;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// The phone-side files the chat reads, shared by the page that reads them and the
/// connection that asks for them.
#[derive(Clone)]
pub struct Media {
    shared: Rc<Shared>,
}

struct Shared {
    grants: RefCell<Grants>,
    /// The reads waiting on an answer, by ticket.
    waiting: RefCell<HashMap<Ticket, oneshot::Sender<Outcome>>>,
    /// The negotiated connection the files are asked on, while there is one.
    link: RefCell<Option<Rc<Link>>>,
}

/// One read of a file: the daemon keeps the file while any read holds it, and dropping the
/// last hold lets it go.
pub struct Hold {
    shared: Rc<Shared>,
    r#ref: String,
    ticket: Ticket,
}

impl Media {
    /// `media_dir` is where the daemon makes its files: `$FERMIX_HOME/companion/media`.
    pub fn new(media_dir: PathBuf) -> Media {
        Media {
            shared: Rc::new(Shared {
                grants: RefCell::new(Grants::new(media_dir)),
                waiting: RefCell::default(),
                link: RefCell::default(),
            }),
        }
    }

    /// Reads `r#ref`'s file: the hold that keeps it, and, once the daemon has answered, where
    /// it is. A read dropped before the answer drops its hold with it.
    pub async fn read(&self, r#ref: &str) -> (Hold, Outcome) {
        let (ticket, answer, events) = self.shared.grants.borrow_mut().hold(r#ref);
        let hold = Hold {
            shared: self.shared.clone(),
            r#ref: r#ref.to_owned(),
            ticket,
        };
        // An answer only comes from a later read of the link, after the waiter is in place.
        self.shared.send(events);
        let receiver = match answer {
            Answer::Now(outcome) => return (hold, outcome),
            Answer::Later => {
                let (sender, receiver) = oneshot::channel();
                self.shared.waiting.borrow_mut().insert(ticket, sender);
                receiver
            }
        };
        let outcome = match receiver.await {
            Ok(outcome) => outcome,
            Err(oneshot::Canceled) => Outcome::Failed("the read was never answered".into()),
        };
        (hold, outcome)
    }

    /// A handshake completed on `link`: every file still read is asked for on it.
    pub fn connected(&self, link: Rc<Link>) {
        self.shared.link.replace(Some(link));
        let events = self.shared.grants.borrow_mut().negotiated();
        self.shared.send(events);
    }

    /// The connection ended, and with it every grant on it. Reads waiting wait for the next.
    pub fn disconnected(&self) {
        self.shared.link.take();
        self.shared.grants.borrow_mut().disconnected();
    }

    /// Hands the daemon's answer to a fetch to the reads waiting on it; false for any other
    /// event.
    pub fn answered(&self, event: &ServerEvent) -> bool {
        let Some(answered) = self.shared.grants.borrow_mut().answered(event) else {
            return false;
        };
        log_answer(&answered);
        let Answered {
            r#ref,
            outcome,
            tickets,
            events,
            ..
        } = answered;
        self.shared.send(events);
        let senders: Vec<_> = {
            let mut waiting = self.shared.waiting.borrow_mut();
            tickets.iter().filter_map(|t| waiting.remove(t)).collect()
        };
        for sender in senders {
            if sender.send(outcome.clone()).is_err() {
                let r#ref = loggable_id(&r#ref);
                glib::g_debug!("fermix", "a read of {ref} ended before its answer");
            }
        }
        true
    }
}

impl Shared {
    fn send(&self, events: Vec<ClientEvent>) {
        if events.is_empty() {
            return;
        }
        let Some(link) = self.link.borrow().clone() else {
            // The grants ask only while connected, and forget every grant when it ends.
            glib::g_warning!("fermix", "{} media events had no connection", events.len());
            return;
        };
        for event in events {
            if let Err(e) = link.send(&event) {
                glib::g_warning!("fermix", "a media event was not sent: {e:?}");
            }
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.shared.waiting.borrow_mut().remove(&self.ticket);
        let events = {
            let mut grants = self.shared.grants.borrow_mut();
            grants.forget(self.ticket, &self.r#ref);
            grants.release(&self.r#ref)
        };
        self.shared.send(events);
    }
}

/// The ref and this app's words for what came of it, never the daemon's text.
fn log_answer(answered: &Answered) {
    let r#ref = loggable_id(&answered.r#ref);
    if !answered.asked {
        glib::g_warning!("fermix", "the daemon answered for {ref}, never asked for");
    }
    match &answered.outcome {
        Outcome::Granted(_) => glib::g_debug!("fermix", "{ref} is granted"),
        Outcome::Missing => glib::g_debug!("fermix", "{ref} is not on this computer"),
        Outcome::Failed(why) => glib::g_warning!("fermix", "{ref} cannot be read: {why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fermix_client::chat_wire::protocol::read_line;

    const NOTE: &str = "9879f6d3c9efefe8c4f4aabe9d3269762264be52794e96f62ab8ffa53b7fadec";

    fn count(media: &Media) -> u32 {
        media.shared.grants.borrow().count(NOTE)
    }

    /// The daemon's answer. With no connection up nothing was fetched, so the grants take it as
    /// never asked for and log a warning; a reader waiting still gets it.
    fn missing() -> ServerEvent {
        let line = format!(r#"{{"type":"media_missing","ref":"{NOTE}"}}"#);
        let event = read_line(line.as_bytes()).expect("an event");
        assert!(!matches!(event, ServerEvent::Unreadable(_)), "{event:?}");
        event
    }

    /// Runs `test` on a main context of its own, as the window's loop runs reads.
    fn on_loop(test: impl FnOnce(&glib::MainContext)) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| test(&context))
            .expect("a context of the test's own");
    }

    #[test]
    fn a_read_waits_for_its_answer_and_holds_its_file_until_dropped() {
        on_loop(|context| {
            let media = Media::new(PathBuf::from("/home/me/.fermix/companion/media"));
            let reader = media.clone();
            let reading = context.spawn_local(async move { reader.read(NOTE).await });
            context.iteration(false);
            assert_eq!(count(&media), 1, "counted while it waits");
            assert!(media.answered(&missing()));
            let (hold, outcome) = context.block_on(reading).expect("answered");
            assert_eq!(outcome, Outcome::Missing);
            assert_eq!(count(&media), 1, "held while the read has it");
            drop(hold);
            assert_eq!(count(&media), 0);
        });
    }

    /// A tile dropped mid-read aborts its task: the read's hold goes with it, and the answer
    /// that comes later finds no reader.
    #[test]
    fn a_read_dropped_before_its_answer_lets_its_file_go() {
        on_loop(|context| {
            let media = Media::new(PathBuf::from("/home/me/.fermix/companion/media"));
            let reader = media.clone();
            let reading = context.spawn_local(async move { reader.read(NOTE).await });
            context.iteration(false);
            assert_eq!(count(&media), 1);
            reading.abort();
            drop(reading);
            assert_eq!(count(&media), 0, "the hold went with the read");
            assert!(media.shared.waiting.borrow().is_empty());
            assert!(
                media.answered(&missing()),
                "the answer is still the grants'"
            );
        });
    }

    /// The files whose log lines speak of the chat's media and files.
    const LOGGING: [(&str, &str); 6] = [
        ("media_grants.rs", include_str!("media_grants.rs")),
        ("chat/entries.rs", include_str!("chat/entries.rs")),
        ("chat/picture.rs", include_str!("chat/picture.rs")),
        ("chat/strip.rs", include_str!("chat/strip.rs")),
        ("chat/voice_chip.rs", include_str!("chat/voice_chip.rs")),
        ("chat/voice_note.rs", include_str!("chat/voice_note.rs")),
    ];

    /// What a log call of these files never formats: a row's file name, a path, a link's
    /// address, a toast's text, a GLib error's message, or a ref read straight from a row or a
    /// chip, unbounded by `loggable_id`.
    const UNLOGGED: [&str; 7] = [
        ".name",
        ".display()",
        "{opens}",
        "{text",
        ".message()",
        ".r#ref",
        "r#ref()",
    ];

    /// Each log call in `source`, from its macro's name to its closing parenthesis.
    fn log_calls(source: &str) -> Vec<&str> {
        let shipped = source.split("#[cfg(test)]").next().unwrap_or(source);
        let macros = [
            "g_warning!(",
            "g_debug!(",
            "g_critical!(",
            "g_message!(",
            "g_info!(",
        ];
        macros
            .iter()
            .flat_map(|name| shipped.match_indices(name))
            .map(|(start, _)| {
                let rest = &shipped[start..];
                &rest[..call_end(rest)]
            })
            .collect()
    }

    /// Where the macro call opening `rest` closes: its parenthesis matched, so a call that is a
    /// match arm's value, ending in `),`, ends there too.
    fn call_end(rest: &str) -> usize {
        let mut depth = 0i32;
        rest.char_indices()
            .find(|&(_, c)| {
                depth += i32::from(c == '(') - i32::from(c == ')');
                c == ')' && depth == 0
            })
            .map_or(rest.len(), |(at, _)| at)
    }

    fn unlogged(call: &str) -> Option<&'static str> {
        UNLOGGED.into_iter().find(|text| call.contains(text))
    }

    /// The log carries no daemon text from the chat's files: a log call names a file by its
    /// bounded ref, and an error by its code. The checker catches a planted line, and the scan
    /// reads files that log.
    #[test]
    fn no_media_log_line_names_a_file_a_path_an_address_or_an_unbounded_ref() {
        let planted = "glib::g_warning!(\"fermix\", \"copying {} failed\", file.name);";
        assert_eq!(log_calls(planted).len(), 1);
        assert_eq!(unlogged(log_calls(planted)[0]), Some(".name"));
        let mut calls = 0;
        for (file, source) in LOGGING {
            for call in log_calls(source) {
                calls += 1;
                assert_eq!(unlogged(call), None, "{file}: {call}");
            }
        }
        assert!(calls >= 20, "the scan reads the files' log lines ({calls})");
    }
}
