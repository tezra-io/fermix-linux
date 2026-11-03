//! The app's side of the grants (`fermix_client::chat_wire::grants`): a read of a phone-side
//! file awaits the daemon's answer and owns a `Hold` for as long as it reads, and dropping the
//! hold lets the file go. The connection's controller (`chat_session.rs`) hands this the
//! negotiated link and the daemon's answers; the chat page reads through it.

use crate::chat_link::Link;
use fermix_client::chat_wire::grants::{Answer, Answered, Grants, Outcome, Ticket};
use fermix_client::chat_wire::protocol::{loggable_id, ClientEvent, LineTooLong, ServerEvent};
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
                let id = loggable_id(&r#ref);
                glib::g_debug!("fermix", "a read of {id} ended before its answer");
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
            let count = events.len();
            glib::g_warning!("fermix", "{count} media events had no connection");
            return;
        };
        for event in events {
            if let Err(LineTooLong(bytes)) = link.send(&event) {
                glib::g_warning!("fermix", "a media event of {bytes} bytes was not sent");
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
    let id = loggable_id(&answered.r#ref);
    if !answered.asked {
        glib::g_warning!("fermix", "the daemon answered for {id}, never asked for");
    }
    match &answered.outcome {
        Outcome::Granted(_) => glib::g_debug!("fermix", "{id} is granted"),
        Outcome::Missing => glib::g_debug!("fermix", "{id} is not on this computer"),
        Outcome::Failed(why) => glib::g_warning!("fermix", "{id} cannot be read: {why}"),
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

    /// The files whose log lines speak of the chat's connection, media, files, links, places and
    /// voice notes.
    const LOGGING: [(&str, &str); 10] = [
        ("chat_session.rs", include_str!("chat_session.rs")),
        ("media_grants.rs", include_str!("media_grants.rs")),
        ("chat/entries.rs", include_str!("chat/entries.rs")),
        ("chat/levels.rs", include_str!("chat/levels.rs")),
        ("chat/picture.rs", include_str!("chat/picture.rs")),
        ("chat/places.rs", include_str!("chat/places.rs")),
        ("chat/playback.rs", include_str!("chat/playback.rs")),
        ("chat/strip.rs", include_str!("chat/strip.rs")),
        ("chat/voice_chip.rs", include_str!("chat/voice_chip.rs")),
        ("chat/voice_note.rs", include_str!("chat/voice_note.rs")),
    ];

    /// The only names a log call in those files formats, inline (`{id}`) or as an argument after
    /// its format string: each with the files that log it, and where its value comes from there,
    /// which is why it holds no file name, path, address, message or sentence of the daemon's.
    const LOGGABLE: [(&str, &[&str], &str); 22] = [
        (
            "id",
            &[
                "chat_session.rs",
                "media_grants.rs",
                "chat/picture.rs",
                "chat/strip.rs",
                "chat/voice_chip.rs",
                "chat/voice_note.rs",
            ],
            "loggable_id: an id of 1 to 64 ASCII letters, digits, `_` or `-`, else \"(not logged)\"",
        ),
        (
            "turn",
            &["chat_session.rs"],
            "loggable_id of a turn's id",
        ),
        (
            "code",
            &["chat/picture.rs", "chat/places.rs", "chat/strip.rs"],
            "error_code: a GLib error's domain and number",
        ),
        (
            "code",
            &["chat/levels.rs"],
            "the Debug of GStreamer's PadLinkError, a C enum: its variant's name",
        ),
        (
            "code",
            &["chat/voice_chip.rs"],
            "the Debug of cairo's Error, a C enum: its variant's name",
        ),
        (
            "bytes",
            &["chat_session.rs", "media_grants.rs"],
            "LineTooLong's length of the line not sent",
        ),
        ("count", &["media_grants.rs"], "a number of media events"),
        (
            "messages",
            &["chat_session.rs"],
            "a history page's count of unreadable rows",
        ),
        (
            "skipped",
            &["chat_session.rs"],
            "a count of parts, blocks, places or commands a decode dropped",
        ),
        (
            "untimed",
            &["chat_session.rs"],
            "a count of rows whose time does not read",
        ),
        ("seq", &["chat_session.rs"], "a row's or a reply's server_seq, a number"),
        (
            "closing",
            &["chat_session.rs"],
            "a refusal's reason, logged only when closes() finds it among CLOSING_REASONS",
        ),
        (
            "kind",
            &["chat_session.rs"],
            "an event's type: ServerEvent::kind's fixed name, a known type (KNOWN_TYPES) of an \
             unreadable event, or loggable_id of an unknown one",
        ),
        (
            "kind",
            &["chat/levels.rs"],
            "the note's stream's caps name, a fixed media type: oggdemux's own, or gst-riff's for \
             an OGM stream, whose fourcc from the file goes into a field that is not logged",
        ),
        (
            "named",
            &["chat_session.rs"],
            "logged(): a hello's two version numbers, or an error's reason by loggable_id and its \
             direction as a fixed word or by loggable_id",
        ),
        ("width", &["chat/picture.rs"], "a decoded picture's width in pixels"),
        ("height", &["chat/picture.rs"], "a decoded picture's height in pixels"),
        (
            "words",
            &["chat/levels.rs"],
            "words(): the GStreamer element's name, which this app's pipeline sets, and \
             error_code of its error",
        ),
        (
            "refused",
            &["chat/strip.rs"],
            "picture::Refused's Display: fixed words, a size, or Failed's text, which is \
             io's or the image crate's error on a file core opened itself (neither names a \
             path), a side in pixels, or this app's sentence",
        ),
        (
            "unheard",
            &["chat/voice_chip.rs"],
            "waveform::Unheard's Display: sizes, fixed words, or \"not decoded: \" and the \
             voice note's `why` below",
        ),
        (
            "why",
            &[
                "media_grants.rs",
                "chat/picture.rs",
                "chat/strip.rs",
                "chat/voice_chip.rs",
                "chat/voice_note.rs",
                "chat/playback.rs",
            ],
            "grants: REFUSED, UNREADABLE or \"broken grant: \" and verify()'s fixed words; a \
             picture's `refused` above or error_code of its copy; this app's sentences; a voice \
             note's words(), STATE_REFUSED, BUS_UNWATCHED, error_code of GStreamer's start, \
             levels' fixed `what`, or audio::make's factory name and GStreamer's BoolError for it",
        ),
        (
            "why",
            &["chat_session.rs"],
            "Lost's Debug: Broken holds this app's sentences, a size, `named` above, or \
             chat_link's: \"connect: \", \"read: \" or \"write: \" and GIO's words for a \
             socket error, which name no path (chat_link's tests check connect's; GIO words a \
             failed read or write as \"Error receiving data: \" or \"Error sending data: \" \
             and the system's error), or a frame's or decode's error Debug, which read_line \
             keeps free of the line",
        ),
    ];

    /// What these files never call outside their tests: the GLib error's message and
    /// GStreamer's debug text. A GLib error's Display is its message too, so an error of
    /// GLib's is logged by `error_code`.
    const UNREAD: [&str; 2] = [".message()", ".debug()"];

    /// The names an error goes by in these files: one of them, or a value read by `.error()`,
    /// made text by `.to_string()` or `format!`, is an error's own words.
    const ERRORS: [&str; 3] = ["e", "err", "error"];

    /// Lines the scan must catch, each in the file it would be in: the forms past the earlier
    /// token scan, a raw format string holding `"` and `)`, an unlisted name in each kind of
    /// macro, and a listed name in a file it is not listed for.
    const PLANTED: [(&str, &str); 22] = [
        (
            r#"glib::g_warning!("fermix", "the link did not open: {}", e);"#,
            "chat/strip.rs",
        ),
        (
            r#"glib::g_debug!("fermix", "a voice note: {}", error.error());"#,
            "chat/levels.rs",
        ),
        (
            r#"glib::g_debug!("fermix", "no thumbnail of {ref:?}");"#,
            "chat/strip.rs",
        ),
        (
            r#"glib::g_warning!("fermix", "no web address: {opens:?}");"#,
            "chat/strip.rs",
        ),
        (
            r#"glib::g_warning!("fermix", "the place did not open: {e:#}");"#,
            "chat/places.rs",
        ),
        (
            r#"glib::g_debug!("fermix", "a voice note: {error:?}");"#,
            "chat/levels.rs",
        ),
        (r#"glib::g_debug!("fermix", "{r} opens");"#, "chat/strip.rs"),
        (
            r#"let copied = copied.await.map_err(|e| e.to_string());"#,
            "chat/picture.rs",
        ),
        (
            r#"let why = format!("GStreamer did not start: {e}");"#,
            "chat/levels.rs",
        ),
        (
            r#"let why = format!("{}: {}", source, error.error());"#,
            "chat/levels.rs",
        ),
        (r#"let why = e.message().to_owned();"#, "chat/picture.rs"),
        (r#"let details = error.debug();"#, "chat/levels.rs"),
        (
            r##"glib::g_warning!("fermix", r#"a "b) {name}"#);"##,
            "chat/strip.rs",
        ),
        (
            r#"glib::g_debug!("fermix", "{} at {}", id, path.display());"#,
            "chat/strip.rs",
        ),
        (
            r#"glib::g_critical!("fermix", "{text}");"#,
            "chat/entries.rs",
        ),
        (r#"glib::g_message!("fermix", "{name}");"#, "chat/strip.rs"),
        (r#"glib::g_info!("fermix", "{name}");"#, "chat/strip.rs"),
        (r#"glib::g_error!("fermix", "{name}");"#, "chat/strip.rs"),
        (
            r#"glib::g_log!("fermix", glib::LogLevel::Debug, "{name}");"#,
            "chat/strip.rs",
        ),
        (r#"println!("{name}");"#, "chat/strip.rs"),
        (r#"eprintln!("{name}");"#, "chat/strip.rs"),
        (r#"glib::g_debug!("fermix", "{words}");"#, "chat/strip.rs"),
    ];

    /// A source without its tests: cut at its one test-only attribute, which opens its test
    /// module. A second one, or one before anything else, would hide shipped lines from the scan.
    fn shipped<'a>(file: &str, source: &'a str) -> &'a str {
        // Built from two parts, so this file holds the marker once, where its tests begin.
        let marker = concat!("#[cfg", "(test)]");
        assert!(
            source.matches(marker).count() <= 1,
            "{file}: one {marker} at most"
        );
        let Some((shipped, tests)) = source.split_once(marker) else {
            return source;
        };
        let opens = tests.trim_start();
        let opens = opens.strip_prefix("pub(crate) ").unwrap_or(opens);
        assert!(
            opens.starts_with("mod tests {"),
            "{file}: {marker} opens mod tests"
        );
        shipped
    }

    /// Each log call in `source`, from its `(`, with the place of its format string among its
    /// arguments. `println!(` finds `eprintln!(` too.
    fn log_calls(source: &str) -> Vec<(&str, usize)> {
        let macros = [
            ("g_warning!(", 1),
            ("g_debug!(", 1),
            ("g_critical!(", 1),
            ("g_message!(", 1),
            ("g_info!(", 1),
            ("g_error!(", 1),
            ("g_log!(", 2),
            ("println!(", 0),
        ];
        macros
            .iter()
            .flat_map(|(name, at)| {
                let calls = source.match_indices(name);
                calls.map(|(start, _)| (&source[start + name.len() - 1..], *at))
            })
            .collect()
    }

    /// The arguments of the call whose `(` opens `call`, split at its top-level commas. A string
    /// (raw ones too) or a char literal is read whole, so a `,`, a bracket or a `"` in one is
    /// text. `None` when the call does not close.
    fn arguments(call: &str) -> Option<Vec<&str>> {
        let (mut at, mut depth, mut start, mut found) = (0, 0usize, 1, Vec::new());
        while at < call.len() {
            match call.as_bytes()[at] {
                b'"' => at = literal_end(call, at)?,
                b'r' if raw_opens(call, at) => at = literal_end(call, at)?,
                b'\'' => at = char_end(call, at),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 1 => {
                    found.push(call[start..at].trim());
                    found.retain(|arg| !arg.is_empty());
                    return Some(found);
                }
                b')' | b']' | b'}' => depth -= 1,
                b',' if depth == 1 => {
                    found.push(call[start..at].trim());
                    start = at + 1;
                }
                _ => {}
            }
            at += 1;
        }
        None
    }

    /// Whether a raw string (`r"`, `r#"`) opens at `at`, not an identifier's `r`.
    fn raw_opens(text: &str, at: usize) -> bool {
        let after = text[at + 1..].trim_start_matches('#');
        let word = at > 0
            && matches!(text.as_bytes()[at - 1], b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'_');
        !word && after.starts_with('"')
    }

    /// Where the string literal opening at `at` ends, at its last byte: a plain one at its
    /// unescaped `"`, a raw one at the `"` and the `#`s it opened with.
    fn literal_end(text: &str, at: usize) -> Option<usize> {
        if text.as_bytes()[at] == b'r' {
            let hashes = text[at + 1..].len() - text[at + 1..].trim_start_matches('#').len();
            let close = format!("\"{}", "#".repeat(hashes));
            let body = at + 2 + hashes;
            return text[body..]
                .find(&close)
                .map(|end| body + end + close.len() - 1);
        }
        let mut escaped = false;
        for (offset, c) in text[at + 1..].char_indices() {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => return Some(at + 1 + offset),
                _ => {}
            }
        }
        None
    }

    /// Where the char literal opening at `at` ends, at its closing `'`; a lifetime's `'` is
    /// left as it is.
    fn char_end(text: &str, at: usize) -> usize {
        let rest = &text[at + 1..];
        if let Some(escape) = rest.strip_prefix('\\') {
            return escape.find('\'').map_or(at, |end| at + 2 + end);
        }
        let width = rest.chars().next().map_or(0, char::len_utf8);
        if rest[width..].starts_with('\'') {
            return at + 1 + width;
        }
        at
    }

    /// The text of a string literal argument, plain or raw, between its quotes.
    fn literal(arg: &str) -> Option<&str> {
        let raw = arg.strip_prefix('r').map(|rest| rest.trim_matches('#'));
        let quoted = raw.unwrap_or(arg);
        quoted.strip_prefix('"')?.strip_suffix('"')
    }

    /// The names a format string formats inline, `{x}` or `{x:…}`, leaving out `{{`, `}}` and
    /// the positional `{}` and `{0}`.
    fn captures(format: &str) -> Vec<&str> {
        let text = format.replace("{{", "  ").replace("}}", "  ");
        let mut names = Vec::new();
        for (open, _) in text.match_indices('{') {
            let inner = text[open + 1..].split(['}', ':']).next().unwrap_or("");
            let name = inner.trim();
            if !name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit()) {
                names.push(&format[open + 1..open + 1 + inner.len()]);
            }
        }
        names
    }

    /// Whether `arg` is one bare name, `[a-z_][a-z0-9_]*`.
    fn bare(arg: &str) -> bool {
        let lead = arg.starts_with(|c: char| c.is_ascii_lowercase() || c == '_');
        lead && arg
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    }

    /// Whether LOGGABLE lists `name` for `file`.
    fn listed(file: &str, name: &str) -> bool {
        LOGGABLE
            .iter()
            .any(|(listed, files, _)| *listed == name && files.contains(&file))
    }

    /// What one log call formats that LOGGABLE does not list for `file`: each inline name, and
    /// each argument after the format string that is not a listed bare name.
    fn unlisted(file: &str, call: &str, format_at: usize) -> Vec<String> {
        let Some(args) = arguments(call) else {
            return vec![format!("a log call that does not close: {call:.60}")];
        };
        let Some(format) = args.get(format_at).and_then(|arg| literal(arg)) else {
            return vec![format!("a log call with no format string: {args:?}")];
        };
        let names = captures(format).into_iter().map(|name| name.trim());
        let named = args[format_at + 1..].iter().copied();
        names
            .chain(named)
            .filter(|name| !(bare(name) && listed(file, name)))
            .map(|name| format!("{file}: {name} in {format:?}"))
            .collect()
    }

    /// Whether `text` ends in one of ERRORS standing alone, or in a read of `.error()`.
    fn names_an_error(text: &str) -> bool {
        let text = text.trim();
        let alone = |name: &&str| {
            let before = text.strip_suffix(*name);
            before.is_some_and(|b| !b.ends_with(|c: char| c.is_alphanumeric() || c == '_'))
        };
        ERRORS.iter().any(alone) || text.ends_with(".error()")
    }

    /// An error made text where the scan sees it: a call in UNREAD, `.to_string()` on an error,
    /// or a `format!` that formats one, inline or as an argument.
    fn converted(file: &str, source: &str) -> Vec<String> {
        let mut found: Vec<String> = UNREAD
            .iter()
            .filter(|call| source.contains(*call))
            .map(|call| format!("{file}: {call}"))
            .collect();
        for (at, _) in source.match_indices(".to_string()") {
            if names_an_error(&source[..at]) {
                found.push(format!("{file}: .to_string() of an error"));
            }
        }
        for (at, _) in source.match_indices("format!(") {
            let Some(args) = arguments(&source[at + "format!".len()..]) else {
                found.push(format!("{file}: a format! that does not close"));
                continue;
            };
            let inline = args.first().and_then(|arg| literal(arg)).map(captures);
            let mut formatted = inline
                .unwrap_or_default()
                .into_iter()
                .chain(args.iter().copied().skip(1));
            if formatted.any(names_an_error) {
                found.push(format!("{file}: format! of an error: {:?}", args.first()));
            }
        }
        found
    }

    /// Every finding in `source` of `file`: its log calls' unlisted names, and its errors made
    /// text.
    fn findings(file: &str, source: &str) -> Vec<String> {
        let calls = log_calls(source);
        let mut found: Vec<String> = calls
            .iter()
            .flat_map(|(call, format_at)| unlisted(file, call, *format_at))
            .collect();
        found.extend(converted(file, source));
        found
    }

    /// The log carries no outside text from the chat's files: a log call formats only names
    /// LOGGABLE lists for its file, each traced to where its value comes from, and the files call
    /// no `.message()` or `.debug()` and make no error text where the scan can see one. The scan
    /// catches each planted line, every name it lists is logged where it is listed, and it reads
    /// every file it names, each of which logs.
    #[test]
    fn a_chat_log_line_formats_only_names_traced_to_no_outside_text() {
        for (line, file) in PLANTED {
            assert!(
                !findings(file, line).is_empty(),
                "{line} in {file} is caught"
            );
        }
        let mut calls = 0;
        for (file, source) in LOGGING {
            let source = shipped(file, source);
            let logged = log_calls(source);
            assert!(!logged.is_empty(), "{file} has log lines");
            calls += logged.len();
            assert_eq!(findings(file, source), Vec::<String>::new(), "{file}");
        }
        assert!(calls >= 60, "the scan reads the files' log lines ({calls})");
        for (name, files, source) in LOGGABLE {
            assert!(!source.is_empty(), "{name} says where it comes from");
            for file in files.iter() {
                let text = LOGGING.iter().find(|(logging, _)| logging == file);
                let logs = text.is_some_and(|(_, text)| logs_name(shipped(file, text), name));
                assert!(logs, "{name} is listed for {file}, which does not log it");
            }
        }
    }

    /// Whether a log call in `source` formats `name`.
    fn logs_name(source: &str, name: &str) -> bool {
        log_calls(source).iter().any(|(call, format_at)| {
            let args = arguments(call).unwrap_or_default();
            let format = args
                .get(*format_at)
                .and_then(|arg| literal(arg))
                .unwrap_or("");
            let inline = captures(format).into_iter().map(str::trim);
            inline
                .chain(args.iter().copied().skip(format_at + 1))
                .any(|n| n == name)
        })
    }
}
