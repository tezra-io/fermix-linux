//! One exchange over the management socket.
//!
//! Connect, write one frame, read one frame, close. One monotonic deadline
//! covers the whole exchange, and the connection is closed on every path
//! including the deadline and every refusal, because a Unix socket left open by
//! a timed-out exchange is a file descriptor this process never gets back.

use std::path::Path;
use std::time::Duration;

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

use super::errors::TransportError;
use super::framing;

/// Send one frame and read the answer.
///
/// `deadline` covers connect, write and read together. A peer that accepts and
/// never answers therefore ends the exchange rather than holding it forever.
pub async fn exchange(
    socket_path: &Path,
    request: &[u8],
    deadline: Duration,
) -> Result<Vec<u8>, TransportError> {
    match glib::future_with_timeout(deadline, exchange_inner(socket_path, request)).await {
        Ok(result) => result,
        Err(_elapsed) => Err(TransportError::Timeout),
    }
}

async fn exchange_inner(socket_path: &Path, request: &[u8]) -> Result<Vec<u8>, TransportError> {
    // A client that has not been told where the socket is has nothing to talk
    // to, which is the same fact to a person as a socket nobody answers on.
    if socket_path.as_os_str().is_empty() {
        return Err(TransportError::NotRunning);
    }

    let address = gio::UnixSocketAddress::new(socket_path);
    let client = gio::SocketClient::new();

    let connection = client
        .connect_future(&address)
        .await
        .map_err(connect_failure)?;

    let result = write_then_read(&connection, request).await;

    // Closed on every path: the success, the frame refusal and the stream
    // failure all arrive here before the answer is handed back.
    let _ = connection.close_future(glib::Priority::DEFAULT).await;
    result
}

async fn write_then_read(
    connection: &gio::SocketConnection,
    request: &[u8],
) -> Result<Vec<u8>, TransportError> {
    framing::write_frame(&connection.output_stream(), request).await?;
    let answer = framing::read_frame(&connection.input_stream()).await?;
    Ok(answer)
}

/// A missing socket file and a socket nothing is accepting on are the same fact
/// to a person: Fermix is not running. Everything else keeps its own shape.
fn connect_failure(error: glib::Error) -> TransportError {
    let not_running = error.matches(gio::IOErrorEnum::NotFound)
        || error.matches(gio::IOErrorEnum::ConnectionRefused)
        || error.matches(gio::IOErrorEnum::HostUnreachable);

    if not_running {
        TransportError::NotRunning
    } else {
        TransportError::Io(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::MainContext;

    #[test]
    fn a_socket_nobody_has_named_yet_is_not_running() {
        let result = MainContext::new()
            .block_on(async { exchange(Path::new(""), b"{}", Duration::from_millis(500)).await });

        match result {
            Err(TransportError::NotRunning) => {}
            other => panic!("expected NotRunning, got {other:?}"),
        }
    }

    #[test]
    fn a_socket_that_does_not_exist_is_not_running() {
        let missing = std::path::PathBuf::from("/nonexistent-fermix-test/daemon.sock");
        let result = MainContext::new()
            .block_on(async { exchange(&missing, b"{}", Duration::from_millis(500)).await });

        match result {
            Err(TransportError::NotRunning) => {}
            other => panic!("expected NotRunning, got {other:?}"),
        }
    }
}
