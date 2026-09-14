//! The management client.
//!
//! Nothing else in the application opens the socket. The client negotiates the
//! highest version both halves speak, stamps it on every request, refuses a
//! method whose published minimum is above it *before sending*, discards a
//! frame carrying somebody else's request id, and stamps every answer with the
//! connection epoch it was issued under so a result from a dead connection can
//! never reach a live model.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::contract;
use super::errors::{ManagementError, WireError};
use super::transport;
use super::types::{HelloResult, NoParams};

/// An answer and the connection epoch it was issued under.
#[derive(Debug)]
pub struct Issued<T> {
    pub epoch: u64,
    pub value: T,
}

impl<T> Issued<T> {
    /// The answer, or nothing when the connection epoch has moved since it was
    /// issued. A caller that uses this cannot apply a stale result by accident.
    pub fn accept(self, client: &ManagementClient) -> Option<T> {
        if client.epoch() == self.epoch {
            Some(self.value)
        } else {
            None
        }
    }
}

/// The framed client for one socket path.
pub struct ManagementClient {
    socket_path: RefCell<PathBuf>,
    /// Whether this client was given its socket for good.
    pinned: bool,
    negotiated: Cell<Option<u32>>,
    counter: Cell<u64>,
    epoch: Cell<u64>,
    last_hello: RefCell<Option<HelloResult>>,
}

impl ManagementClient {
    /// A client for one socket, for good. Nothing is opened until a request is
    /// made, and nothing moves it afterwards: this is the configuration where
    /// the socket is known up front, which is a development run against a
    /// fixture home.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: RefCell::new(socket_path.into()),
            pinned: true,
            negotiated: Cell::new(None),
            counter: Cell::new(0),
            epoch: Cell::new(0),
            last_hello: RefCell::new(None),
        }
    }

    /// A client with nowhere to speak to yet.
    ///
    /// The packaged configuration: the socket lives inside the home this
    /// account is bound to, and the command line is what reports that. Every
    /// exchange refuses as a daemon that is not running until it does.
    pub fn unbound() -> Self {
        Self {
            socket_path: RefCell::new(PathBuf::new()),
            pinned: false,
            negotiated: Cell::new(None),
            counter: Cell::new(0),
            epoch: Cell::new(0),
            last_hello: RefCell::new(None),
        }
    }

    /// The socket this client speaks to.
    pub fn socket_path(&self) -> PathBuf {
        self.socket_path.borrow().clone()
    }

    /// Point this client at the socket the command line reported.
    ///
    /// A packaged run does not know where the daemon's socket is until the
    /// command line reports the home this account is bound to, and it moves
    /// again if that binding changes. Moving is a new connection, so it is also
    /// a new epoch: nothing issued against the old path can land on a model
    /// built from the new one. Answers whether anything moved.
    ///
    /// A client that was given its socket for good never moves.
    pub fn rebind(&self, socket_path: &Path) -> bool {
        if self.pinned || *self.socket_path.borrow() == socket_path {
            return false;
        }

        self.socket_path.replace(socket_path.to_path_buf());
        self.reset();
        true
    }

    /// The current connection epoch.
    pub fn epoch(&self) -> u64 {
        self.epoch.get()
    }

    /// The version both halves settled on, once `hello` has run.
    pub fn negotiated_version(&self) -> Option<u32> {
        self.negotiated.get()
    }

    /// Drop the session: a new epoch, and the negotiation is taken again.
    ///
    /// Called on disconnect, a home change, a lifecycle mutation and an
    /// observed daemon-generation change. Results issued under the old epoch
    /// stop being acceptable the moment this returns.
    pub fn reset(&self) {
        self.epoch.set(self.epoch.get().saturating_add(1));
        self.negotiated.set(None);
        self.last_hello.replace(None);
    }

    /// Negotiate, and answer with the daemon's own identity.
    pub async fn hello(&self, deadline: Duration) -> Issued<Result<HelloResult, ManagementError>> {
        let epoch = self.epoch();
        Issued {
            epoch,
            value: self.negotiate(deadline).await,
        }
    }

    /// The last `hello` answer, if one has landed under the current epoch.
    pub fn last_hello(&self) -> Option<HelloResult> {
        self.last_hello.borrow().clone()
    }

    /// One typed call.
    ///
    /// Refuses before sending when the method is not in the vendored contract,
    /// or when its published minimum is above the negotiated version: a daemon
    /// that cannot serve a surface is said so, rather than drawn as an empty
    /// pane.
    pub async fn request<P, R>(
        &self,
        method: &str,
        params: &P,
        deadline: Duration,
    ) -> Issued<Result<R, ManagementError>>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let epoch = self.epoch();
        Issued {
            epoch,
            value: self.request_inner(method, params, deadline).await,
        }
    }

    /// One typed call to a method that takes no parameters.
    pub async fn call<R>(
        &self,
        method: &str,
        deadline: Duration,
    ) -> Issued<Result<R, ManagementError>>
    where
        R: DeserializeOwned,
    {
        self.request(method, &NoParams {}, deadline).await
    }

    async fn request_inner<P, R>(
        &self,
        method: &str,
        params: &P,
        deadline: Duration,
    ) -> Result<R, ManagementError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let minimum = contract::method_minimum(method).ok_or_else(|| {
            ManagementError::MethodNotInContract {
                method: method.to_string(),
            }
        })?;

        let negotiated = self.ensure_negotiated(deadline).await?;
        if minimum > negotiated {
            return Err(ManagementError::MethodNeedsNewerDaemon {
                method: method.to_string(),
                requires: minimum,
                negotiated,
            });
        }

        let result = self.send(method, params, negotiated, deadline).await?;
        serde_json::from_value(result).map_err(|error| ManagementError::Decode {
            method: method.to_string(),
            reason: error.to_string(),
        })
    }

    async fn ensure_negotiated(&self, deadline: Duration) -> Result<u32, ManagementError> {
        if let Some(version) = self.negotiated.get() {
            return Ok(version);
        }
        self.negotiate(deadline).await?;
        self.negotiated
            .get()
            .ok_or_else(|| ManagementError::NoAnswer {
                method: "hello".to_string(),
            })
    }

    /// `hello` at this build's own maximum; if the daemon's ceiling is lower it
    /// answers `daemon_too_old` with its window attached, and the one retry
    /// below speaks the highest version both halves share. Two attempts, never
    /// more: the second either lands inside the daemon's window or the two
    /// halves genuinely share none.
    async fn negotiate(&self, deadline: Duration) -> Result<HelloResult, ManagementError> {
        let app = contract::supported_range();

        match self.hello_at(app.max, deadline).await {
            Ok(hello) => self.adopt(hello, app),
            Err(ManagementError::Wire(error)) if error.code == "daemon_too_old" => {
                let daemon = window_from(&error)?;
                let common = app.max.min(daemon.1);
                if common < app.min || common < daemon.0 {
                    return Err(ManagementError::VersionsDoNotOverlap {
                        app: (app.min, app.max),
                        daemon,
                    });
                }
                let hello = self.hello_at(common, deadline).await?;
                self.adopt(hello, app)
            }
            Err(ManagementError::Wire(error)) if error.code == "client_too_old" => {
                let daemon = window_from(&error)?;
                Err(ManagementError::VersionsDoNotOverlap {
                    app: (app.min, app.max),
                    daemon,
                })
            }
            Err(error) => Err(error),
        }
    }

    fn adopt(
        &self,
        hello: HelloResult,
        app: contract::VersionRange,
    ) -> Result<HelloResult, ManagementError> {
        let daemon = (
            hello.protocol.minimum_version,
            hello.protocol.maximum_version,
        );
        let common = app.max.min(daemon.1);

        if common < app.min || common < daemon.0 {
            return Err(ManagementError::VersionsDoNotOverlap {
                app: (app.min, app.max),
                daemon,
            });
        }

        self.negotiated.set(Some(common));
        self.last_hello.replace(Some(hello.clone()));
        Ok(hello)
    }

    async fn hello_at(
        &self,
        version: u32,
        deadline: Duration,
    ) -> Result<HelloResult, ManagementError> {
        let result = self.send("hello", &NoParams {}, version, deadline).await?;
        serde_json::from_value(result).map_err(|error| ManagementError::Decode {
            method: "hello".to_string(),
            reason: error.to_string(),
        })
    }

    async fn send<P>(
        &self,
        method: &str,
        params: &P,
        version: u32,
        deadline: Duration,
    ) -> Result<serde_json::Value, ManagementError>
    where
        P: Serialize,
    {
        let request_id = self.next_request_id();
        let encoded = self.encode(&request_id, method, params, version)?;
        let socket_path = self.socket_path.borrow().clone();
        let answer = transport::exchange(&socket_path, &encoded, deadline).await?;

        decode_response(&request_id, method, &answer)
    }

    fn encode<P>(
        &self,
        request_id: &str,
        method: &str,
        params: &P,
        version: u32,
    ) -> Result<Vec<u8>, ManagementError>
    where
        P: Serialize,
    {
        let params = serde_json::to_value(params).map_err(|error| ManagementError::Decode {
            method: method.to_string(),
            reason: error.to_string(),
        })?;

        let ceiling = contract::limits().max_params_bytes;
        let params_bytes = params.to_string().len();
        if params_bytes > ceiling {
            return Err(ManagementError::RequestTooLarge {
                method: method.to_string(),
                bytes: params_bytes,
                ceiling,
            });
        }

        let envelope = serde_json::json!({
            "request_id": request_id,
            "protocol_version": version,
            "method": method,
            "params": params,
        });

        Ok(envelope.to_string().into_bytes())
    }

    fn next_request_id(&self) -> String {
        let next = self.counter.get().saturating_add(1);
        self.counter.set(next);
        format!("fx-{next}")
    }
}

/// A response carries exactly one of `result` or `error`, and it must carry the
/// request id it is answering. A frame with a different id is discarded, which
/// is the protocol's own rule.
fn decode_response(
    request_id: &str,
    method: &str,
    answer: &[u8],
) -> Result<serde_json::Value, ManagementError> {
    let envelope: serde_json::Value =
        serde_json::from_slice(answer).map_err(|error| ManagementError::Decode {
            method: method.to_string(),
            reason: error.to_string(),
        })?;

    let answered = envelope
        .get("request_id")
        .and_then(serde_json::Value::as_str);
    if answered != Some(request_id) {
        return Err(ManagementError::NoAnswer {
            method: method.to_string(),
        });
    }

    decode_envelope(method, &envelope)
}

/// One response envelope, read: the result, or the daemon's own refusal.
///
/// Public because the in-memory peer the model tests run against answers whole
/// envelopes too, and a second reader of the same shape would be a second place
/// for the refusal path to drift.
pub fn decode_envelope(
    method: &str,
    envelope: &serde_json::Value,
) -> Result<serde_json::Value, ManagementError> {
    if let Some(error) = envelope.get("error") {
        return Err(ManagementError::Wire(wire_error(error)));
    }

    match envelope.get("result") {
        Some(result) => Ok(result.clone()),
        None => Err(ManagementError::Decode {
            method: method.to_string(),
            reason: "the response carried neither a result nor an error".to_string(),
        }),
    }
}

fn wire_error(error: &serde_json::Value) -> WireError {
    let details = error
        .get("details")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();

    WireError {
        code: string_at(error, "code"),
        message: string_at(error, "message"),
        sentence: details
            .get("sentence")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        details,
    }
}

fn string_at(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The window a version refusal carries, so the app can say which half to
/// update without re-deriving it.
fn window_from(error: &WireError) -> Result<(u32, u32), ManagementError> {
    match (
        error.detail_u32("minimum_version"),
        error.detail_u32("maximum_version"),
    ) {
        (Some(minimum), Some(maximum)) => Ok((minimum, maximum)),
        _ => Err(ManagementError::Decode {
            method: "hello".to_string(),
            reason: "a version refusal arrived without the window attached".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_answering_a_different_request_is_discarded() {
        let answer = br#"{"request_id":"fx-99","result":{}}"#;
        match decode_response("fx-1", "hello", answer) {
            Err(ManagementError::NoAnswer { method }) => assert_eq!(method, "hello"),
            other => panic!("expected the frame to be discarded, got {other:?}"),
        }
    }

    #[test]
    fn an_error_envelope_keeps_the_daemons_own_sentence() {
        let answer = br#"{"request_id":"fx-1","error":{"code":"invalid_params",
            "message":"Request parameters are invalid.",
            "details":{"field":"value","sentence":"A secret cannot be empty."}}}"#;

        match decode_response("fx-1", "secret.set", answer) {
            Err(ManagementError::Wire(error)) => {
                assert_eq!(error.code, "invalid_params");
                assert_eq!(error.rendered(), "A secret cannot be empty.");
            }
            other => panic!("expected a wire error, got {other:?}"),
        }
    }

    #[test]
    fn a_response_with_neither_half_is_a_decode_failure() {
        let answer = br#"{"request_id":"fx-1"}"#;
        match decode_response("fx-1", "hello", answer) {
            Err(ManagementError::Decode { .. }) => {}
            other => panic!("expected a decode failure, got {other:?}"),
        }
    }

    #[test]
    fn request_ids_are_unique_and_well_formed() {
        let client = ManagementClient::new("/nonexistent/daemon.sock");
        assert_eq!(client.next_request_id(), "fx-1");
        assert_eq!(client.next_request_id(), "fx-2");
    }

    #[test]
    fn a_reset_moves_the_epoch_and_stales_an_issued_answer() {
        let client = ManagementClient::new("/nonexistent/daemon.sock");
        let issued = Issued {
            epoch: client.epoch(),
            value: 7,
        };

        client.reset();

        assert!(issued.accept(&client).is_none());
        assert_eq!(client.negotiated_version(), None);
    }

    #[test]
    fn pointing_the_client_somewhere_else_is_a_new_connection() {
        let client = ManagementClient::unbound();
        let issued = Issued {
            epoch: client.epoch(),
            value: 7,
        };

        assert!(client.rebind(Path::new("/nonexistent/two/daemon.sock")));
        assert_eq!(
            client.socket_path(),
            PathBuf::from("/nonexistent/two/daemon.sock")
        );
        assert!(
            issued.accept(&client).is_none(),
            "an answer from the socket it left cannot land on the one it moved to"
        );
    }

    #[test]
    fn pointing_the_client_where_it_already_is_moves_nothing() {
        let client = ManagementClient::unbound();
        client.rebind(Path::new("/nonexistent/one/daemon.sock"));
        let before = client.epoch();

        assert!(!client.rebind(Path::new("/nonexistent/one/daemon.sock")));
        assert_eq!(client.epoch(), before);
    }

    #[test]
    fn a_client_given_its_socket_for_good_is_never_moved() {
        let client = ManagementClient::new("/nonexistent/fixture/daemon.sock");

        assert!(!client.rebind(Path::new("/nonexistent/somewhere-else/daemon.sock")));
        assert_eq!(
            client.socket_path(),
            PathBuf::from("/nonexistent/fixture/daemon.sock"),
            "a development run against a fixture home stays pointed at it"
        );
    }

    #[test]
    fn an_answer_from_the_current_epoch_is_accepted() {
        let client = ManagementClient::new("/nonexistent/daemon.sock");
        let issued = Issued {
            epoch: client.epoch(),
            value: 7,
        };
        assert_eq!(issued.accept(&client), Some(7));
    }
}
