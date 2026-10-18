//! The chat's socket: `companion.sock`, driven on the GTK main loop with GIO's async streams.
//! The wire format lives in `fermix_client::chat_wire::protocol`; this file only moves lines.
//! It is the one owner of the chat's connection: `chat_session.rs` opens it, reads it in one
//! loop, and sends through it.

use fermix_client::chat_wire::protocol::{decode_line, line_buffer, ClientEvent, ServerEvent};
use fermix_client::realtime::protocol::LineBuffer;
use gtk::gio::{self, prelude::*};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes asked of the socket per read.
const READ_CHUNK: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// No socket file: an engine without chat.
    NotFound,
    /// The connection broke or the daemon sent what cannot be read; the detail is for the log.
    Broken(String),
}

pub struct Link {
    connection: gio::SocketConnection,
    input: gio::InputStream,
    output: gio::OutputStream,
    buffer: RefCell<LineBuffer>,
    /// Lines read and not yet handed out, oldest first.
    lines: RefCell<VecDeque<Vec<u8>>>,
    queue: RefCell<VecDeque<String>>,
    writing: Cell<bool>,
}

/// Connects to the chat socket. The handshake is the caller's.
pub async fn connect(socket: &Path) -> Result<Rc<Link>, LinkError> {
    let address = gio::UnixSocketAddress::new(socket);
    let connecting = gio::SocketClient::new().connect_future(&address);
    let connection = glib::future_with_timeout(CONNECT_TIMEOUT, connecting)
        .await
        .map_err(|_| LinkError::Broken("the chat socket did not answer in time".into()))?
        .map_err(|e| match e.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::NotFound) => LinkError::NotFound,
            _ => LinkError::Broken(format!("connect: {e}")),
        })?;
    Ok(Rc::new(Link {
        input: connection.input_stream(),
        output: connection.output_stream(),
        connection,
        buffer: RefCell::new(line_buffer()),
        lines: RefCell::default(),
        queue: RefCell::default(),
        writing: Cell::new(false),
    }))
}

impl Link {
    /// Queues one event; events go out in order, one write at a time. One too long for the
    /// daemon's cap is not sent, and says so.
    pub fn send(self: &Rc<Self>, event: &ClientEvent) -> Result<(), LinkError> {
        let line = event
            .line()
            .map_err(|e| LinkError::Broken(format!("a {} byte line", e.0)))?;
        self.queue.borrow_mut().push_back(line);
        if self.writing.replace(true) {
            return Ok(());
        }
        let link = self.clone();
        glib::spawn_future_local(async move { link.drain().await });
        Ok(())
    }

    async fn drain(&self) {
        loop {
            let Some(line) = self.queue.borrow_mut().pop_front() else {
                break;
            };
            let written = self
                .output
                .write_all_future(line.into_bytes(), glib::Priority::DEFAULT)
                .await;
            if let Err((_, e)) = written {
                // The read loop sees the broken connection and reconnects.
                glib::g_warning!("fermix", "chat write failed: {e}");
                self.queue.borrow_mut().clear();
                self.close();
                break;
            }
        }
        self.writing.set(false);
    }

    /// The next event; `None` once the daemon has closed the connection. Each pass reads one
    /// chunk, and the line buffer refuses a line past its cap, so a read cannot grow without
    /// bound.
    pub async fn read(&self) -> Result<Option<ServerEvent>, LinkError> {
        loop {
            if let Some(line) = self.lines.borrow_mut().pop_front() {
                return decode_line(&line)
                    .map(Some)
                    .map_err(|e| LinkError::Broken(format!("decode: {e:?}")));
            }
            let bytes = self
                .input
                .read_bytes_future(READ_CHUNK, glib::Priority::DEFAULT)
                .await
                .map_err(|e| LinkError::Broken(format!("read: {e}")))?;
            if bytes.is_empty() {
                return Ok(None);
            }
            let lines = self
                .buffer
                .borrow_mut()
                .push(&bytes)
                .map_err(|e| LinkError::Broken(format!("frame: {e:?}")))?;
            self.lines.borrow_mut().extend(lines);
        }
    }

    /// Ends the connection; a pending `read` then returns `None` or an error.
    pub fn close(&self) {
        if let Err(e) = self.connection.socket().shutdown(true, true) {
            glib::g_debug!("fermix", "chat socket shutdown: {e}");
        }
    }
}
