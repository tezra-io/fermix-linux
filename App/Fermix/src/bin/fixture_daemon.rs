//! The fixture daemon.
//!
//! It answers the vendored golden responses over a real Unix socket with the
//! real framing, so development, captures and tests drive the same client code
//! a packaged daemon does. It is not a simulator: it never composes a response,
//! it only serves one the engine published, optionally replaced by a scenario's
//! own golden. The goldens themselves are loaded by `fermix_desktop::fixtures`,
//! which the in-memory peer the model tests run against loads too, so the two
//! peers cannot answer differently.
//!
//! It also serves the one HTTP door the Setup assistant's finish gate asks for:
//! `GET /health/live`, answering 200 there and 404 anywhere else. That is the
//! whole of it. A scenario whose `hello` names port 0 has no door at all, which
//! is how the ladder's "checking that it answers" step is shown still running.
//!
//! The door is bound on a port the operating system chooses, and the `hello` it
//! then serves names that port rather than the goldens'. Nothing here may
//! depend on a fixed port: the number in the vendored golden is 4030, which is
//! the port a developer's own Fermix daemon holds, so binding it would fail on
//! the one machine this is most used on and two of these running at once could
//! never both succeed. The rewrite is the only change this peer makes to an
//! answer, and it exists so the assistant's finish gate asks a door that
//! belongs to this process.
//!
//! Usage: `fixture-daemon <directory>`, with the scenario in
//! `FIXTURE_DAEMON_SCENARIO`. The socket path is printed on stdout when it is
//! ready to answer.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

use fermix_desktop::fixtures::{self, Goldens};
use fermix_desktop::management::contract;

fn main() -> std::process::ExitCode {
    let Some(directory) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("fixture-daemon: a directory to hold daemon.sock is required");
        return std::process::ExitCode::FAILURE;
    };

    let scenario = std::env::var("FIXTURE_DAEMON_SCENARIO").unwrap_or_else(|_| "default".into());
    let socket_path = directory.join("daemon.sock");

    if let Err(reason) = std::fs::create_dir_all(&directory) {
        eprintln!(
            "fixture-daemon: {} could not be created: {reason}",
            directory.display()
        );
        return std::process::ExitCode::FAILURE;
    }

    if fixtures::is_silent(&scenario) {
        // The daemon is not running: the path exists as a fact, nothing
        // answers on it, and the client has to say so rather than hang.
        announce(&socket_path);
        return std::process::ExitCode::SUCCESS;
    }

    let goldens = match Goldens::load(&scenario) {
        Ok(goldens) => goldens,
        Err(reason) => {
            eprintln!("fixture-daemon: {reason}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match serve(&socket_path, &goldens) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(reason) => {
            eprintln!("fixture-daemon: {reason}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn announce(socket_path: &Path) {
    println!("{}", socket_path.display());
    let _ = std::io::stdout().flush();
}

/// The daemon's own web door, on a port the operating system chooses.
///
/// It answers `Some(port)` when it opened one, and `None` when this scenario
/// declares no door or the bind failed. A bind that fails is reported and the
/// socket goes on answering: the assistant then meets the state where the
/// daemon answers and its web door does not, which is one of the states this
/// peer exists to show.
fn serve_health(goldens: &Goldens) -> Option<u16> {
    declared_health_port(goldens)?;

    // Port 0 asks the operating system for a free one, and `local_addr` is what
    // it chose. Nothing downstream may assume a number.
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(reason) => {
            eprintln!("fixture-daemon: the health door could not be opened: {reason}");
            return None;
        }
    };

    let port = match listener.local_addr() {
        Ok(address) => address.port(),
        Err(reason) => {
            eprintln!("fixture-daemon: the health door has no address: {reason}");
            return None;
        }
    };

    // Its own thread, because the control socket's accept loop is the main one
    // and neither may wait on the other.
    std::thread::spawn(move || {
        for connection in listener.incoming().flatten() {
            if let Err(reason) = answer_health(connection) {
                eprintln!("fixture-daemon: {reason}");
            }
        }
    });

    Some(port)
}

/// Whether this scenario declares a web door at all.
///
/// The number it declares is not used: a scenario says "there is a door" by
/// naming a port above zero and "there is none" by naming zero, and which port
/// the door ends up on is this process's business rather than the fixture's.
fn declared_health_port(goldens: &Goldens) -> Option<u16> {
    let answer = goldens.answer(&serde_json::json!({
        "request_id": "health",
        "protocol_version": contract::supported_range().max,
        "method": "hello",
        "params": {},
    }));

    let origin = setup_origin(&answer)?;
    let port: u16 = origin.rsplit_once(':')?.1.parse().ok()?;
    (port > 0).then_some(port)
}

/// The origin one `hello` answer publishes.
fn setup_origin(answer: &serde_json::Value) -> Option<&str> {
    answer.get("result")?.get("setup")?.get("origin")?.as_str()
}

/// One `hello` answer, with its origin pointing at the door this process opened.
///
/// Every other answer is served exactly as the engine published it. This one is
/// rewritten because the golden names a port this process does not own, and a
/// finish gate that asked that port would be asking whatever else is listening
/// on the machine.
fn with_health_port(mut answer: serde_json::Value, port: u16) -> serde_json::Value {
    let Some(origin) = setup_origin(&answer) else {
        return answer;
    };
    let Some((authority, _declared)) = origin.rsplit_once(':') else {
        return answer;
    };

    let rewritten = format!("{authority}:{port}");
    if let Some(setup) = answer
        .get_mut("result")
        .and_then(|result| result.get_mut("setup"))
        .and_then(serde_json::Value::as_object_mut)
    {
        setup.insert("origin".to_string(), serde_json::json!(rewritten));
    }
    answer
}

/// One request, one answer, and the connection closed either way.
fn answer_health(mut connection: TcpStream) -> Result<(), String> {
    let mut request = [0u8; 1024];
    let read = connection
        .read(&mut request)
        .map_err(|error| format!("the health request was not read: {error}"))?;

    let live = String::from_utf8_lossy(&request[..read]).starts_with("GET /health/live ");
    let answer = if live {
        "HTTP/1.0 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    } else {
        "HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    };

    connection
        .write_all(answer.as_bytes())
        .and_then(|()| connection.flush())
        .map_err(|error| format!("the health answer was not written: {error}"))
}

fn serve(socket_path: &Path, goldens: &Goldens) -> Result<(), String> {
    let _ = std::fs::remove_file(socket_path);

    let listener = UnixListener::bind(socket_path)
        .map_err(|error| format!("{} could not be bound: {error}", socket_path.display()))?;

    // Announced before anything else is tried, and on stdout: a caller waits for
    // this line and nothing must reach it first. The web door is opened after
    // it, so a port that cannot be bound is a line about the web door rather
    // than a caller starting against a socket that is not there yet.
    announce(socket_path);
    let health_port = serve_health(goldens);

    // One request per connection, then the daemon closes it, which is the
    // exchange the protocol describes.
    for connection in listener.incoming() {
        let Ok(mut connection) = connection else {
            continue;
        };
        if let Err(reason) = answer(&mut connection, goldens, health_port) {
            eprintln!("fixture-daemon: {reason}");
        }
    }

    Ok(())
}

fn answer(
    connection: &mut UnixStream,
    goldens: &Goldens,
    health_port: Option<u16>,
) -> Result<(), String> {
    let request = read_frame(connection)?;
    let request: serde_json::Value =
        serde_json::from_slice(&request).map_err(|error| format!("unreadable request: {error}"))?;

    let mut response = goldens.answer(&request);
    if request.get("method").and_then(serde_json::Value::as_str) == Some("hello") {
        if let Some(port) = health_port {
            response = with_health_port(response, port);
        }
    }

    write_frame(
        connection,
        &serde_json::to_vec(&response).map_err(|error| error.to_string())?,
    )
}

fn read_frame(connection: &mut UnixStream) -> Result<Vec<u8>, String> {
    let mut header = [0u8; 4];
    connection
        .read_exact(&mut header)
        .map_err(|error| format!("no frame header: {error}"))?;

    let declared = u32::from_be_bytes(header) as usize;
    let ceiling = contract::limits().max_frame_bytes;
    if declared > ceiling {
        return Err(format!(
            "a frame declared {declared} bytes, ceiling is {ceiling}"
        ));
    }

    let mut body = vec![0u8; declared];
    connection
        .read_exact(&mut body)
        .map_err(|error| format!("short frame: {error}"))?;
    Ok(body)
}

fn write_frame(connection: &mut UnixStream, body: &[u8]) -> Result<(), String> {
    let length = u32::try_from(body.len()).map_err(|_| "the response does not fit a frame")?;
    connection
        .write_all(&length.to_be_bytes())
        .and_then(|()| connection.write_all(body))
        .and_then(|()| connection.flush())
        .map_err(|error| format!("the answer was not written: {error}"))
}
