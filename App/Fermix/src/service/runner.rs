//! The five typed CLI operations.
//!
//! This is the one code path for anything that must happen while the daemon is
//! stopped, and it is the only caller of `fermix` in the application. Explicit
//! argv, no shell, no profile sourced, stdout and stderr captured separately
//! and capped, one deadline per operation, and a cancellation that force-exits
//! and reaps the child it owns and nothing else.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::future::try_join;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;

use super::types::{
    ActionResult, DiagnosticsExport, Envelope, InstallOutcome, Restart, ServiceStatus,
};

/// The most either stream may produce. A child that talks past it is refused
/// rather than allowed to grow this process without bound.
pub const OUTPUT_CEILING: usize = 1024 * 1024;

/// A read operation's deadline.
pub const READ_DEADLINE: Duration = Duration::from_secs(30);
/// A lifecycle mutation's deadline. The 90 second activation ceiling of M38
/// section 5.5 belongs to the assistant and covers more than one call.
pub const MUTATION_DEADLINE: Duration = Duration::from_secs(120);

/// What a CLI operation can answer with other than a result.
#[derive(Debug)]
pub enum ServiceError {
    /// The CLI refused, with its own code and sentence.
    Refused { code: String, sentence: String },
    /// The CLI could not be launched at all.
    Launch(glib::Error),
    /// The operation passed its deadline and the child was force-exited.
    Timeout,
    /// The output was not the envelope the contract publishes, or there was too
    /// much of it.
    Output(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServiceError::Refused { code, sentence } => write!(formatter, "{code}: {sentence}"),
            ServiceError::Launch(error) => {
                write!(formatter, "the Fermix command line did not run: {error}")
            }
            ServiceError::Timeout => write!(formatter, "the operation passed its deadline"),
            ServiceError::Output(reason) => write!(formatter, "unreadable output: {reason}"),
        }
    }
}

impl std::error::Error for ServiceError {}

/// One operation's typed answer.
pub type ServiceResult<T> = Result<T, ServiceError>;

/// The sole caller of the packaged `fermix`.
pub struct ServiceRunner {
    cli_path: PathBuf,
    read_deadline: Duration,
    mutation_deadline: Duration,
}

impl ServiceRunner {
    /// A runner for one CLI path, with the published deadlines. The path is
    /// absolute and is never resolved on `PATH`: a shell alias or a stale
    /// standalone install must not answer for the package this interface is
    /// bound to.
    pub fn new(cli_path: impl Into<PathBuf>) -> Self {
        Self {
            cli_path: cli_path.into(),
            read_deadline: READ_DEADLINE,
            mutation_deadline: MUTATION_DEADLINE,
        }
    }

    /// The same runner with different deadlines.
    ///
    /// The product uses the published ones; this exists so the deadline and
    /// cancellation paths can be proven in a second rather than in two minutes.
    /// It changes a value, never a code path: the same `capture` runs either
    /// way.
    pub fn with_deadlines(self, read: Duration, mutation: Duration) -> Self {
        Self {
            read_deadline: read,
            mutation_deadline: mutation,
            ..self
        }
    }

    /// The CLI this runner invokes.
    pub fn cli_path(&self) -> &Path {
        &self.cli_path
    }

    /// How long a read may take.
    pub fn read_deadline(&self) -> Duration {
        self.read_deadline
    }

    /// How long a lifecycle mutation may take.
    pub fn mutation_deadline(&self) -> Duration {
        self.mutation_deadline
    }

    /// `fermix service status --json`.
    pub async fn status(&self, cancellable: &gio::Cancellable) -> ServiceResult<ServiceStatus> {
        self.run(
            &["service", "status", "--json"],
            self.read_deadline,
            cancellable,
        )
        .await
    }

    /// `fermix service install --json [--home PATH] [--port N]`.
    pub async fn install(
        &self,
        home: Option<&Path>,
        port: Option<u32>,
        cancellable: &gio::Cancellable,
    ) -> ServiceResult<InstallOutcome> {
        let mut argv: Vec<String> = ["service", "install", "--json"]
            .iter()
            .map(|part| part.to_string())
            .collect();
        if let Some(home) = home {
            argv.push("--home".to_string());
            argv.push(home.to_string_lossy().into_owned());
        }
        if let Some(port) = port {
            argv.push("--port".to_string());
            argv.push(port.to_string());
        }

        let borrowed: Vec<&str> = argv.iter().map(String::as_str).collect();
        self.run(&borrowed, self.mutation_deadline, cancellable)
            .await
    }

    /// `fermix service uninstall --json`.
    pub async fn uninstall(&self, cancellable: &gio::Cancellable) -> ServiceResult<ActionResult> {
        self.run(
            &["service", "uninstall", "--json"],
            self.read_deadline,
            cancellable,
        )
        .await
    }

    /// `fermix restart --json [--when-idle]`.
    pub async fn restart(
        &self,
        when_idle: bool,
        cancellable: &gio::Cancellable,
    ) -> ServiceResult<Restart> {
        let argv: &[&str] = if when_idle {
            &["restart", "--json", "--when-idle"]
        } else {
            &["restart", "--json"]
        };
        self.run(argv, self.mutation_deadline, cancellable).await
    }

    /// `fermix diagnostics export --offline --json`.
    pub async fn export_diagnostics(
        &self,
        cancellable: &gio::Cancellable,
    ) -> ServiceResult<DiagnosticsExport> {
        self.run(
            &["diagnostics", "export", "--offline", "--json"],
            self.read_deadline,
            cancellable,
        )
        .await
    }

    async fn run<T>(
        &self,
        arguments: &[&str],
        deadline: Duration,
        cancellable: &gio::Cancellable,
    ) -> ServiceResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let captured = self.capture(arguments, deadline, cancellable).await?;
        decode(&captured.stdout)
    }

    async fn capture(
        &self,
        arguments: &[&str],
        deadline: Duration,
        cancellable: &gio::Cancellable,
    ) -> ServiceResult<Captured> {
        let mut argv: Vec<&std::ffi::OsStr> = Vec::with_capacity(arguments.len() + 1);
        argv.push(self.cli_path.as_os_str());
        argv.extend(arguments.iter().map(std::ffi::OsStr::new));

        if cancellable.is_cancelled() {
            return Err(ServiceError::Timeout);
        }

        let child = gio::Subprocess::newv(
            &argv,
            gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
        )
        .map_err(ServiceError::Launch)?;

        // Cancelling force-exits this child and nothing else, which closes its
        // pipes and ends the reads. The handler is disconnected on every path,
        // so a cancellable that outlives the operation holds no reference to a
        // process that is already gone.
        let handler = cancellable.connect_cancelled_local(glib::clone!(
            #[strong]
            child,
            move |_| child.force_exit()
        ));

        let outcome = match glib::future_with_timeout(deadline, drain(&child, cancellable)).await {
            Ok(result) => result,
            Err(_elapsed) => {
                child.force_exit();
                Err(ServiceError::Timeout)
            }
        };

        // The child is reaped on every path, including the deadline, the
        // cancellation and a refusal to read its output.
        let _ = child.wait_future().await;
        if let Some(handler) = handler {
            cancellable.disconnect_cancelled(handler);
        }

        if cancellable.is_cancelled() {
            return Err(ServiceError::Timeout);
        }
        outcome
    }
}

struct Captured {
    stdout: Vec<u8>,
    #[allow(dead_code)]
    stderr: Vec<u8>,
}

/// Both pipes are drained at once. Draining one at a time deadlocks against a
/// child that fills the other's buffer while waiting to be read.
async fn drain(
    child: &gio::Subprocess,
    _cancellable: &gio::Cancellable,
) -> ServiceResult<Captured> {
    let stdout = child.stdout_pipe();
    let stderr = child.stderr_pipe();

    // `try_join` rather than `join`: when one pipe passes the ceiling the other
    // is usually still open, and waiting for it would turn a refusal into a
    // deadline.
    match try_join(read_capped(stdout), read_capped(stderr)).await {
        Ok((stdout, stderr)) => Ok(Captured { stdout, stderr }),
        Err(error) => {
            child.force_exit();
            Err(error)
        }
    }
}

/// Read one pipe to its end, refusing at the ceiling.
///
/// Bounded twice over: every iteration either consumes bytes or ends the loop,
/// and the total is checked against the ceiling before the next read.
async fn read_capped(stream: Option<gio::InputStream>) -> ServiceResult<Vec<u8>> {
    let Some(stream) = stream else {
        return Ok(Vec::new());
    };

    let mut collected: Vec<u8> = Vec::new();
    loop {
        let buffer = vec![0u8; 64 * 1024];
        let (buffer, read) = stream
            .read_future(buffer, glib::Priority::DEFAULT)
            .await
            .map_err(|(_, error)| ServiceError::Launch(error))?;

        if read == 0 {
            return Ok(collected);
        }

        collected.extend_from_slice(&buffer[..read]);
        if collected.len() > OUTPUT_CEILING {
            return Err(ServiceError::Output(format!(
                "the command produced more than {OUTPUT_CEILING} bytes"
            )));
        }
    }
}

/// The envelope, decoded. `ok: false` becomes a typed refusal carrying the
/// CLI's own sentence, which is the half a retry needs and the half that used
/// to be thrown away.
pub fn decode<T>(stdout: &[u8]) -> ServiceResult<T>
where
    T: serde::de::DeserializeOwned,
{
    let envelope: Envelope =
        serde_json::from_slice(stdout).map_err(|error| ServiceError::Output(error.to_string()))?;

    if !envelope.ok {
        let error = envelope.error.ok_or_else(|| {
            ServiceError::Output("a refusal arrived without an error object".to_string())
        })?;
        return Err(ServiceError::Refused {
            code: error.code,
            sentence: error.sentence,
        });
    }

    let result = envelope
        .result
        .ok_or_else(|| ServiceError::Output("a success arrived without a result".to_string()))?;

    serde_json::from_value(result).map_err(|error| ServiceError::Output(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::types::Alignment;

    /// One published golden, as the bytes the command line would print.
    fn golden(relative: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/cli/fixtures")
            .join(relative);
        std::fs::read(&path)
            .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()))
    }

    #[test]
    fn a_success_envelope_decodes_into_its_result() {
        let status: ServiceStatus =
            decode(&golden("service_status/active_aligned.json")).expect("decodes");
        assert_eq!(status.alignment, Alignment::Aligned);
    }

    #[test]
    fn a_published_refusal_decodes_with_its_code_and_its_sentence() {
        match decode::<ServiceStatus>(&golden("errors/user_manager_unreachable.json")) {
            Err(ServiceError::Refused { code, sentence }) => {
                assert_eq!(code, "user_manager_unreachable");
                assert!(sentence.starts_with("This session has no user service manager"));
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_refusal_keeps_the_command_lines_own_sentence() {
        let stdout = br#"{"schema_version":1,"ok":false,
            "error":{"code":"linger_denied","sentence":"Linux needs one more permission."}}"#;

        match decode::<ServiceStatus>(stdout) {
            Err(ServiceError::Refused { code, sentence }) => {
                assert_eq!(code, "linger_denied");
                assert_eq!(sentence, "Linux needs one more permission.");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn output_that_is_not_the_envelope_is_refused_rather_than_guessed_at() {
        match decode::<ServiceStatus>(b"starting service...\n") {
            Err(ServiceError::Output(_)) => {}
            other => panic!("expected an output failure, got {other:?}"),
        }
    }

    #[test]
    fn a_refusal_without_an_error_object_is_an_output_failure() {
        match decode::<ServiceStatus>(br#"{"schema_version":1,"ok":false}"#) {
            Err(ServiceError::Output(_)) => {}
            other => panic!("expected an output failure, got {other:?}"),
        }
    }
}
