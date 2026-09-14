//! The daemon's own web door, asked one question.
//!
//! The finish gate of M38 section 5.5 is `hello` *and* `/health/live` answering
//! 200 on the origin `hello` published. This is the second half: one TCP
//! connection to that origin, one hand-written HTTP/1.0 request, one status
//! line read, and the connection closed on every path.
//!
//! `/health/live` rather than `/health/ready`: readiness answers whether the
//! daemon has finished warming, which would keep the assistant waiting long
//! after the thing it is waiting for works.
//!
//! Nothing here parses a body, follows a redirect or speaks HTTP/1.1
//! keep-alive. HTTP/1.0 closes when the answer ends, which is exactly the
//! lifetime of the one question being asked.

use std::time::Duration;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

/// The path the daemon publishes its liveness on.
pub const HEALTH_PATH: &str = "/health/live";

/// The most a status line may be. A peer that answers more than this before its
/// first newline is not answering the question that was asked.
const STATUS_LINE_CEILING: usize = 8 * 1024;

/// What one look at the web door found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// The door answered 200: the daemon's web surface is up.
    Live,
    /// Something is accepting on that origin and it did not answer 200. On the
    /// origin the daemon needs, that is something else holding the port.
    Occupied(String),
    /// Nothing is accepting there at all.
    Unreachable,
}

/// Where to ask. Parsed from the origin `hello` publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub host: String,
    pub port: u16,
}

impl Origin {
    /// One `http://host:port` origin, read.
    ///
    /// Only the shape the daemon publishes is accepted: a scheme this door
    /// cannot speak, or a port it cannot read, is not guessed at.
    pub fn parse(origin: &str) -> Option<Self> {
        let rest = origin.strip_prefix("http://")?;
        let rest = rest.split('/').next().unwrap_or_default();
        let (host, port) = rest.rsplit_once(':')?;

        if host.is_empty() {
            return None;
        }

        Some(Self {
            host: host.to_string(),
            port: port.parse().ok()?,
        })
    }

    /// The `Host` header this request carries.
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Ask the web door once.
///
/// `deadline` covers the connect, the write and the read together, and the
/// connection is closed on every path including the deadline.
pub async fn probe(origin: &Origin, deadline: Duration) -> Health {
    match glib::future_with_timeout(deadline, ask(origin)).await {
        Ok(health) => health,
        // A peer that accepted and then said nothing is holding the port
        // without answering the question, which is the same fact to a person
        // as an answer that is not 200.
        Err(_elapsed) => Health::Occupied(String::new()),
    }
}

async fn ask(origin: &Origin) -> Health {
    let client = gio::SocketClient::new();
    let connection = match client
        .connect_to_host_future(&origin.host, origin.port)
        .await
    {
        Ok(connection) => connection,
        Err(_) => return Health::Unreachable,
    };

    let health = exchange(&connection, origin).await;
    let _ = connection.close_future(glib::Priority::DEFAULT).await;
    health
}

async fn exchange(connection: &gio::SocketConnection, origin: &Origin) -> Health {
    let request = format!(
        "GET {HEALTH_PATH} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        origin.authority()
    );

    if connection
        .output_stream()
        .write_all_future(request.into_bytes(), glib::Priority::DEFAULT)
        .await
        .is_err()
    {
        return Health::Occupied(String::new());
    }

    match read_status_line(&connection.input_stream()).await {
        Some(line) if is_ok(&line) => Health::Live,
        Some(line) => Health::Occupied(line),
        None => Health::Occupied(String::new()),
    }
}

/// The first line of the answer, and no more of it than a status line can be.
///
/// Bounded twice: every iteration either consumes bytes or ends the loop, and
/// the total is checked against the ceiling before the next read.
async fn read_status_line(stream: &gio::InputStream) -> Option<String> {
    let mut collected: Vec<u8> = Vec::new();

    loop {
        let buffer = vec![0u8; 1024];
        let (buffer, read) = stream
            .read_future(buffer, glib::Priority::DEFAULT)
            .await
            .ok()?;

        if read == 0 {
            break;
        }
        collected.extend_from_slice(&buffer[..read]);

        if let Some(at) = collected.iter().position(|byte| *byte == b'\n') {
            collected.truncate(at);
            break;
        }
        if collected.len() >= STATUS_LINE_CEILING {
            break;
        }
    }

    if collected.is_empty() {
        return None;
    }

    Some(String::from_utf8_lossy(&collected).trim_end().to_string())
}

/// `HTTP/1.x 200 …`, and nothing else. The engine's own health reader makes the
/// same comparison, so the two halves of this check agree about what 200 means.
fn is_ok(status_line: &str) -> bool {
    let mut parts = status_line.split(' ');
    let version = parts.next().unwrap_or_default();
    let code = parts.next().unwrap_or_default();

    matches!(version, "HTTP/1.0" | "HTTP/1.1") && code == "200"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_origin_is_read_from_the_shape_the_daemon_publishes() {
        let origin = Origin::parse("http://127.0.0.1:4030").expect("parses");
        assert_eq!(origin.host, "127.0.0.1");
        assert_eq!(origin.port, 4030);
        assert_eq!(origin.authority(), "127.0.0.1:4030");
    }

    #[test]
    fn an_origin_with_a_path_keeps_only_its_authority() {
        let origin = Origin::parse("http://localhost:4030/setup").expect("parses");
        assert_eq!(origin.host, "localhost");
        assert_eq!(origin.port, 4030);
    }

    #[test]
    fn an_origin_this_door_cannot_speak_is_refused_rather_than_guessed_at() {
        assert_eq!(Origin::parse("https://127.0.0.1:4030"), None);
        assert_eq!(Origin::parse("http://127.0.0.1"), None);
        assert_eq!(Origin::parse("http://:4030"), None);
        assert_eq!(Origin::parse("http://127.0.0.1:not-a-port"), None);
    }

    #[test]
    fn only_a_two_hundred_on_a_version_this_door_speaks_is_live() {
        assert!(is_ok("HTTP/1.0 200 OK"));
        assert!(is_ok("HTTP/1.1 200 OK"));
        assert!(!is_ok("HTTP/1.1 404 Not Found"));
        assert!(!is_ok("HTTP/1.1 500 Internal Server Error"));
        assert!(!is_ok("HTTP/2 200"));
        assert!(!is_ok(""));
    }
}
