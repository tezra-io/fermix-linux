//! The management socket client. One request per connection, as the protocol
//! specifies; every call has a deadline and closes its socket on every path.

use crate::frame::{read_frame, write_frame, FrameError};
use crate::model::{
    AuthStart, DetectResult, Hello, JobList, JobStatus, JobView, Lease, RestartOnly,
    SecretSetResult, SetupSession, SetupState,
};
use crate::providers::ImportSource;
use crate::settings::{ApplyResult, ReloadResult, SectionRows, Sections};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};
use std::io::{self, ErrorKind};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const PROTOCOL_VERSION: u64 = 2;

#[derive(Debug)]
pub enum CallError {
    /// Nothing is listening: the daemon is stopped, or its socket is missing or stale.
    DaemonDown(ErrorKind),
    /// The daemon accepted the call but did not answer before the deadline.
    Timeout,
    /// The daemon said no. `sentence` is its own words, ready to show.
    Refused(Refusal),
    /// The connection failed mid-exchange.
    Io(String),
    /// The answer broke the protocol: not JSON, the wrong request, or the wrong shape.
    Protocol(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub code: String,
    pub sentence: String,
    /// The parameter or row key the refusal is about, where the daemon names one.
    pub field: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Management {
    socket: PathBuf,
    timeout: Duration,
    next_id: Arc<AtomicU64>,
}

impl Management {
    pub fn new(socket: PathBuf, timeout: Duration) -> Self {
        assert!(
            socket.is_absolute(),
            "the daemon socket path must be absolute"
        );
        assert!(!timeout.is_zero(), "a call needs a non-zero deadline");
        Management {
            socket,
            timeout,
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn call(&self, method: &str, params: Value) -> Result<Value, CallError> {
        assert!(!method.is_empty(), "a call must name its method");
        assert!(params.is_object(), "params must be a JSON object");
        let request_id = format!("desktop-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let request = json!({
            "request_id": request_id,
            "protocol_version": PROTOCOL_VERSION,
            "method": method,
            "params": params,
        });
        let body = serde_json::to_vec(&request).expect("a JSON value always serializes");
        let mut stream = self.connect()?;
        write_frame(&mut stream, &body).map_err(frame_error)?;
        let answer = read_frame(&mut stream).map_err(frame_error)?;
        drop(stream);
        decode_response(&answer, &request_id)
    }

    fn connect(&self) -> Result<UnixStream, CallError> {
        let stream = UnixStream::connect(&self.socket).map_err(|e| match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => CallError::DaemonDown(e.kind()),
            _ => CallError::Io(e.to_string()),
        })?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(io_error)?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(io_error)?;
        Ok(stream)
    }

    fn call_typed<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, CallError> {
        let result = self.call(method, params)?;
        serde_json::from_value(result)
            .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
    }

    pub fn hello(&self) -> Result<Hello, CallError> {
        self.call_typed("hello", json!({}))
    }

    /// Opens the daemon's drain window before systemd restarts it (M38 §4.1).
    /// A refusal here means nothing was stopped.
    pub fn prepare_restart(&self) -> Result<Lease, CallError> {
        self.call_typed("lifecycle.prepare", json!({}))
    }

    /// Hands back a drain lease when the restart it was taken for did not happen,
    /// so the running daemon carries on as before.
    pub fn cancel_restart(&self, lease_id: &str) -> Result<(), CallError> {
        assert!(!lease_id.is_empty(), "a lease is cancelled by its id");
        self.call("lifecycle.cancel", json!({ "lease_id": lease_id }))?;
        Ok(())
    }

    pub fn setup_session(&self) -> Result<SetupSession, CallError> {
        self.call_typed("setup.session.create", json!({}))
    }

    pub fn secret_clear(&self, id: &str) -> Result<SecretSetResult, CallError> {
        self.call_typed("secret.clear", json!({ "id": id }))
    }

    pub fn setup_state(&self) -> Result<SetupState, CallError> {
        self.call_typed("setup.state.get", json!({}))
    }

    pub fn auth_start(&self, provider: &str) -> Result<AuthStart, CallError> {
        let start: AuthStart = self.call_typed("auth.start", json!({ "provider": provider }))?;
        Ok(AuthStart {
            job: checked_job(start.job)?,
            ..start
        })
    }

    pub fn auth_import(&self, source: ImportSource) -> Result<JobView, CallError> {
        checked_job(self.call_typed("auth.import.start", json!({ "source": source.wire() }))?)
    }

    pub fn auth_logout(&self, provider: &str) -> Result<RestartOnly, CallError> {
        self.call_typed("auth.logout", json!({ "provider": provider }))
    }

    pub fn job_get(&self, job_id: &str) -> Result<JobView, CallError> {
        checked_job(self.call_typed("job.get", json!({ "job_id": job_id }))?)
    }

    pub fn job_cancel(&self, job_id: &str) -> Result<JobView, CallError> {
        checked_job(self.call_typed("job.cancel", json!({ "job_id": job_id }))?)
    }

    pub fn job_list(&self) -> Result<JobList, CallError> {
        let list: JobList = self.call_typed("job.list", json!({}))?;
        let jobs = list
            .jobs
            .into_iter()
            .map(checked_job)
            .collect::<Result<_, _>>()?;
        Ok(JobList { jobs })
    }

    /// Stores one secret. The value goes out on the wire and nowhere else:
    /// it is not logged, and no error built here can contain it.
    pub fn secret_set(&self, id: &str, value: &str) -> Result<SecretSetResult, CallError> {
        self.call_typed("secret.set", json!({ "id": id, "value": value }))
    }

    pub fn set_primary(&self, provider: &str) -> Result<RestartOnly, CallError> {
        self.call_typed("providers.set_primary", json!({ "provider": provider }))
    }

    pub fn detect(&self, targets: &[&str]) -> Result<DetectResult, CallError> {
        self.call_typed("setup.detect", json!({ "targets": targets }))
    }

    pub fn settings_sections(&self) -> Result<Sections, CallError> {
        self.call_typed("settings.sections", json!({}))
    }

    pub fn settings_get(&self, section: &str) -> Result<SectionRows, CallError> {
        assert!(!section.is_empty(), "settings.get needs a section id");
        self.call_typed("settings.get", json!({ "section": section }))
    }

    /// Writes the changed rows of one section. Validation is all-or-nothing:
    /// a refused key leaves nothing half-written.
    pub fn settings_apply(
        &self,
        section: &str,
        values: Map<String, Value>,
    ) -> Result<ApplyResult, CallError> {
        assert!(!section.is_empty(), "settings.apply needs a section id");
        assert!(
            !values.is_empty(),
            "settings.apply needs at least one value"
        );
        self.call_typed(
            "settings.apply",
            json!({ "section": section, "values": values }),
        )
    }

    pub fn settings_reload(&self) -> Result<ReloadResult, CallError> {
        self.call_typed("settings.reload", json!({}))
    }
}

/// A job that ended badly must say why; the UI renders that sentence and has no other.
fn checked_job(job: JobView) -> Result<JobView, CallError> {
    let ended_badly = matches!(job.status, JobStatus::Failed | JobStatus::TimedOut);
    if ended_badly && job.failure.is_none() {
        return Err(CallError::Protocol(format!(
            "job {} ended badly without a reason",
            job.job_id
        )));
    }
    Ok(job)
}

/// Reads one response envelope: exactly one of `result` or `error`, for this request.
pub fn decode_response(bytes: &[u8], request_id: &str) -> Result<Value, CallError> {
    let envelope: Value = serde_json::from_slice(bytes)
        .map_err(|e| CallError::Protocol(format!("the daemon's answer is not JSON: {e}")))?;
    let answered_id = envelope.get("request_id").and_then(Value::as_str);
    match (envelope.get("result"), envelope.get("error")) {
        (Some(result), None) if answered_id == Some(request_id) => Ok(result.clone()),
        // A request too malformed to carry an id is refused with a null id.
        (None, Some(error)) if answered_id.is_none() || answered_id == Some(request_id) => {
            Err(CallError::Refused(refusal(error)?))
        }
        (Some(_), Some(_)) | (None, None) => Err(CallError::Protocol(
            "the daemon's answer must carry exactly one of result or error".into(),
        )),
        _ => Err(CallError::Protocol(format!(
            "the daemon answered request {answered_id:?}, not {request_id}"
        ))),
    }
}

fn refusal(error: &Value) -> Result<Refusal, CallError> {
    let field = |name: &str| error.get(name).and_then(Value::as_str);
    let (Some(code), Some(message)) = (field("code"), field("message")) else {
        return Err(CallError::Protocol(
            "a refusal must carry a code and a message".into(),
        ));
    };
    // `message` is generic ("Request parameters are invalid."); the specific
    // sentence, where the daemon has one, travels in `details.sentence`.
    let specific = error.pointer("/details/sentence").and_then(Value::as_str);
    let field = error.pointer("/details/field").and_then(Value::as_str);
    Ok(Refusal {
        code: code.to_owned(),
        sentence: specific.unwrap_or(message).to_owned(),
        field: field.map(str::to_owned),
    })
}

fn frame_error(e: FrameError) -> CallError {
    match e {
        FrameError::TooLarge(n) => {
            CallError::Protocol(format!("a {n}-byte frame exceeds the ceiling"))
        }
        FrameError::Io(e) => io_error(e),
    }
}

fn io_error(e: io::Error) -> CallError {
    match e.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => CallError::Timeout,
        _ => CallError::Io(e.to_string()),
    }
}
