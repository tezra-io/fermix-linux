//! The chat connection: one ACP session over `~/.fermix/acp.sock`, driven on the
//! GTK main loop with GIO's async streams. The wire format lives in
//! `fermix_client::acp`; this file only moves lines.

use fermix_client::acp::{self, AckError, Incoming};
use gtk::gio::{self, prelude::*};
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

/// How long each opening step (connect, handshake, initialize, session) may take.
const OPEN_STEP: Duration = Duration::from_secs(10);
/// Lines the agent may send before answering an opening request; past it, the
/// opening fails instead of reading forever.
const OPEN_READ_CAP: usize = 32;
const INITIALIZE_ID: u64 = 1;
const SESSION_ID: u64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// No socket: Fermix is not running.
    NotRunning,
    /// Fermix answered and said no, in its own words.
    Refused(String),
    /// The connection broke or the agent answered nonsense; the detail is for the log.
    Broken(String),
}

impl LinkError {
    pub fn sentence(&self) -> String {
        match self {
            LinkError::NotRunning => "Fermix is not running. Start it from Home.".into(),
            LinkError::Refused(message) => format!("Fermix refused the chat: {message}"),
            LinkError::Broken(_) => "The chat connection to Fermix failed.".into(),
        }
    }
}

pub struct Link {
    connection: gio::SocketConnection,
    reader: gio::DataInputStream,
    output: gio::OutputStream,
    queue: RefCell<VecDeque<String>>,
    writing: Cell<bool>,
    next_id: Cell<u64>,
    pub session_id: String,
}

/// Connects, shakes hands and opens a session whose tools work in `cwd`.
pub async fn open(socket: &Path, cwd: &str, app_version: &str) -> Result<Rc<Link>, LinkError> {
    let connection = step(connect(socket)).await??;
    let reader = gio::DataInputStream::new(&connection.input_stream());
    let output = connection.output_stream();
    let mut link = Link {
        connection,
        reader,
        output,
        queue: RefCell::default(),
        writing: Cell::new(false),
        next_id: Cell::new(SESSION_ID + 1),
        session_id: String::new(),
    };
    step(link.write_now(&acp::handshake_line(app_version))).await??;
    let ack = step(link.read_line()).await??;
    acp::parse_ack(&ack).map_err(|e| match e {
        AckError::Refused(message) => LinkError::Refused(message),
        AckError::Malformed(detail) => LinkError::Broken(detail),
    })?;
    link.write_now(&acp::initialize(INITIALIZE_ID, app_version))
        .await?;
    step(link.answer_to(INITIALIZE_ID)).await??;
    link.write_now(&acp::new_session(SESSION_ID, cwd)).await?;
    let opened = step(link.answer_to(SESSION_ID)).await??;
    link.session_id = opened
        .get("sessionId")
        .and_then(|id| id.as_str())
        .ok_or_else(|| LinkError::Broken(format!("session/new answered {opened}")))?
        .to_owned();
    Ok(Rc::new(link))
}

async fn step<T>(future: impl std::future::Future<Output = T>) -> Result<T, LinkError> {
    glib::future_with_timeout(OPEN_STEP, future)
        .await
        .map_err(|_| LinkError::Broken("the chat socket did not answer in time".into()))
}

async fn connect(socket: &Path) -> Result<gio::SocketConnection, LinkError> {
    let address = gio::UnixSocketAddress::new(socket);
    gio::SocketClient::new()
        .connect_future(&address)
        .await
        .map_err(|e| match e.kind::<gio::IOErrorEnum>() {
            Some(gio::IOErrorEnum::NotFound | gio::IOErrorEnum::ConnectionRefused) => {
                LinkError::NotRunning
            }
            _ => LinkError::Broken(format!("connect: {e}")),
        })
}

impl Link {
    pub fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    /// Queues one line; lines go out in order, one write at a time.
    pub fn send(self: &Rc<Self>, line: String) {
        self.queue.borrow_mut().push_back(line);
        if self.writing.replace(true) {
            return;
        }
        let link = self.clone();
        glib::spawn_future_local(async move { link.drain().await });
    }

    async fn drain(&self) {
        loop {
            let Some(line) = self.queue.borrow_mut().pop_front() else {
                break;
            };
            if let Err(e) = self.write_now(&line).await {
                glib::g_warning!("fermix", "chat write failed: {e:?}");
                self.queue.borrow_mut().clear();
                break;
            }
        }
        self.writing.set(false);
    }

    async fn write_now(&self, line: &str) -> Result<(), LinkError> {
        self.output
            .write_all_future(line.as_bytes().to_vec(), glib::Priority::DEFAULT)
            .await
            .map(|_| ())
            .map_err(|(_, e)| LinkError::Broken(format!("write: {e}")))
    }

    /// One line, or `None` once Fermix has closed the connection.
    async fn next_line(&self) -> Result<Option<String>, LinkError> {
        let line = self
            .reader
            .read_line_utf8_future(glib::Priority::DEFAULT)
            .await
            .map_err(|e| LinkError::Broken(format!("read: {e}")))?;
        match line {
            Some(line) if line.len() > acp::MAX_LINE_BYTES => {
                Err(LinkError::Broken(format!("a {} byte line", line.len())))
            }
            Some(line) => Ok(Some(line.to_string())),
            None => Ok(None),
        }
    }

    async fn read_line(&self) -> Result<String, LinkError> {
        self.next_line()
            .await?
            .ok_or_else(|| LinkError::Broken("Fermix closed the chat connection".into()))
    }

    /// Reads until the response to request `id`, answering any agent request
    /// on the way. Used only while opening, when nothing else reads.
    async fn answer_to(&self, id: u64) -> Result<serde_json::Value, LinkError> {
        for _ in 0..OPEN_READ_CAP {
            let line = self.read_line().await?;
            match acp::parse_line(&line).map_err(LinkError::Broken)? {
                Incoming::Response { id: got, result } if got == id => return Ok(result),
                Incoming::Error { code, message, .. } => {
                    return Err(LinkError::Refused(format!("{message} ({code})")))
                }
                Incoming::Request { id: asked, .. } => {
                    self.write_now(&acp::reply_unsupported(&asked)).await?
                }
                _ => {}
            }
        }
        Err(LinkError::Broken(format!("no answer to request {id}")))
    }

    /// The next message from the agent; `None` once the connection has ended.
    pub async fn read(&self) -> Result<Option<Incoming>, LinkError> {
        match self.next_line().await? {
            Some(line) => acp::parse_line(&line).map(Some).map_err(LinkError::Broken),
            None => Ok(None),
        }
    }

    /// Ends the connection; a pending `read` then returns `None` or an error.
    pub fn close(&self) {
        let socket = self.connection.socket();
        if let Err(e) = socket.shutdown(true, true) {
            glib::g_debug!("fermix", "chat socket shutdown: {e}");
        }
    }
}
