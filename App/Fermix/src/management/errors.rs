//! What a management exchange can refuse with.
//!
//! Two families, kept apart on purpose. A [`TransportError`] is this side of
//! the socket failing; a [`ManagementError::Wire`] is the daemon answering with
//! its own refusal, and the sentence inside it is the daemon's, never ours.

use super::framing::FrameError;

/// The socket failing, before any answer exists.
#[derive(Debug)]
pub enum TransportError {
    /// Nothing is listening: no socket file, or nothing accepting on it.
    NotRunning,
    /// The peer accepted and the exchange did not finish inside its deadline.
    Timeout,
    /// The framing refused the bytes.
    Frame(FrameError),
    /// Everything else the stream can do.
    Io(gtk4::glib::Error),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::NotRunning => write!(formatter, "nothing is listening on the socket"),
            TransportError::Timeout => write!(formatter, "the exchange passed its deadline"),
            TransportError::Frame(error) => write!(formatter, "{error}"),
            TransportError::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<FrameError> for TransportError {
    fn from(error: FrameError) -> Self {
        TransportError::Frame(error)
    }
}

/// The daemon's own refusal, decoded.
///
/// `sentence` is `details.sentence` when the daemon supplied one and `None`
/// otherwise. A surface renders the sentence when it is there and `message`
/// when it is not, because rendering `message` alone for the `invalid_params`
/// family shows an operator the one sentence in the catalogue that tells them
/// nothing.
#[derive(Debug, Clone)]
pub struct WireError {
    pub code: String,
    pub message: String,
    pub sentence: Option<String>,
    pub details: serde_json::Map<String, serde_json::Value>,
}

impl WireError {
    /// The sentence to render: the daemon's own where it has one, its fixed
    /// class sentence otherwise.
    pub fn rendered(&self) -> &str {
        match &self.sentence {
            Some(sentence) => sentence.as_str(),
            None => self.message.as_str(),
        }
    }

    /// One public scalar out of `details`, as a string.
    pub fn detail(&self, field: &str) -> Option<&str> {
        self.details.get(field).and_then(serde_json::Value::as_str)
    }

    /// One public scalar out of `details`, as an integer.
    pub fn detail_u32(&self, field: &str) -> Option<u32> {
        self.details
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    }
}

/// Everything one management call can answer with other than a result.
#[derive(Debug)]
pub enum ManagementError {
    /// The socket failed.
    Transport(TransportError),
    /// The daemon refused.
    Wire(WireError),
    /// The method's published minimum is above the negotiated version, so the
    /// call was refused here rather than sent. `requires` is the minimum.
    MethodNeedsNewerDaemon {
        method: String,
        requires: u32,
        negotiated: u32,
    },
    /// The method is not in the vendored contract at all, which is a defect in
    /// this application rather than a state of the daemon.
    MethodNotInContract { method: String },
    /// The two halves share no version.
    VersionsDoNotOverlap { app: (u32, u32), daemon: (u32, u32) },
    /// The answer did not decode into the shape the contract publishes.
    Decode { method: String, reason: String },
    /// Every frame that arrived carried a different request id.
    NoAnswer { method: String },
    /// The encoded parameters are above the published bound, refused here
    /// rather than sent and failed on the far side.
    RequestTooLarge {
        method: String,
        bytes: usize,
        ceiling: usize,
    },
}

impl std::fmt::Display for ManagementError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManagementError::Transport(error) => write!(formatter, "{error}"),
            ManagementError::Wire(error) => {
                write!(formatter, "{}: {}", error.code, error.rendered())
            }
            ManagementError::MethodNeedsNewerDaemon {
                method,
                requires,
                negotiated,
            } => write!(
                formatter,
                "{method} needs protocol {requires}, this session negotiated {negotiated}"
            ),
            ManagementError::MethodNotInContract { method } => {
                write!(
                    formatter,
                    "{method} is not published by the vendored contract"
                )
            }
            ManagementError::VersionsDoNotOverlap { app, daemon } => write!(
                formatter,
                "this app speaks {}-{} and the daemon speaks {}-{}",
                app.0, app.1, daemon.0, daemon.1
            ),
            ManagementError::Decode { method, reason } => {
                write!(formatter, "{method} did not decode: {reason}")
            }
            ManagementError::NoAnswer { method } => {
                write!(
                    formatter,
                    "{method} received no frame carrying its request id"
                )
            }
            ManagementError::RequestTooLarge {
                method,
                bytes,
                ceiling,
            } => {
                write!(
                    formatter,
                    "{method} encodes {bytes} bytes of params, bound is {ceiling}"
                )
            }
        }
    }
}

impl std::error::Error for ManagementError {}

impl From<TransportError> for ManagementError {
    fn from(error: TransportError) -> Self {
        ManagementError::Transport(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(code: &str, message: &str, details: serde_json::Value) -> WireError {
        WireError {
            code: code.to_string(),
            message: message.to_string(),
            sentence: details
                .get("sentence")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            details: details.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn the_daemons_own_sentence_wins_when_it_has_one() {
        let error = wire(
            "invalid_params",
            "Request parameters are invalid.",
            serde_json::json!({"field": "value", "sentence": "A secret cannot be empty."}),
        );
        assert_eq!(error.rendered(), "A secret cannot be empty.");
        assert_eq!(error.detail("field"), Some("value"));
    }

    #[test]
    fn the_class_sentence_is_what_is_left_when_it_does_not() {
        let error = wire(
            "busy",
            "Another operation is running.",
            serde_json::json!({}),
        );
        assert_eq!(error.rendered(), "Another operation is running.");
        assert_eq!(error.detail("field"), None);
    }

    #[test]
    fn an_integer_detail_decodes() {
        let error = wire(
            "method_not_found",
            "The method is not available.",
            serde_json::json!({"requires": 2}),
        );
        assert_eq!(error.detail_u32("requires"), Some(2));
    }
}
