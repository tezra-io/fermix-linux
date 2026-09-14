//! The CLI's machine-mode envelope, and the results behind it.
//!
//! Every shape here is `contracts/cli/CONTRACT.md`, field for field and null for
//! null, and `tests/contract.rs` decodes all thirty-five vendored goldens
//! through these structs and fails on a field they drop. The envelope is
//! schema-versioned and carries either a result or a typed error with a display
//! sentence. Nothing here parses prose, and nothing here infers a state from an
//! exit code alone.
//!
//! A field the engine does not publish is not defined here, however convenient
//! a surface would find it: the contract is the whole of what can be rendered.

use serde::{Deserialize, Serialize};

use crate::management::types::EngineIdentity;

/// The envelope every `--json` operation answers with.
#[derive(Debug, Clone, Deserialize)]
pub struct Envelope {
    pub schema_version: u32,
    pub ok: bool,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<EnvelopeError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnvelopeError {
    pub code: String,
    /// The CLI's own operator-facing sentence.
    pub sentence: String,
}

/// The distribution identity a packaged engine publishes. Anything else came
/// from somewhere this application does not manage.
pub const PACKAGE_DISTRIBUTION: &str = "linux_package";

/// The sub-state systemd reports while it is restarting a unit that keeps
/// dying, which is the difference between a slow first start and a crash loop.
const AUTO_RESTART: &str = "auto-restart";

/// Every error code the packaged command line publishes, read as a type.
///
/// The spelling lives here, in the layer that decodes the command line, for the
/// same reason the wire atoms live in `management::vocabulary`: a model that
/// compared a code against a literal would be keeping its own copy of a
/// vocabulary the engine owns. The list is `MachineOutput.codes/0`, and
/// `tests/contract.rs` asserts that every golden under `cli/fixtures/errors/`
/// reads back as one of these rather than as `Unrecognized`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCode {
    AppManaged,
    UserManagerUnreachable,
    LingerDenied,
    LoginctlAbsent,
    NoIdentity,
    InvalidHome,
    HomeChangeRefused,
    ForeignUnit,
    ActivationTimeout,
    HealthUnavailable,
    ForeignDistribution,
    ServiceUnbound,
    DiagnosticsUnavailable,
    IdleRestartUnavailable,
    LifecycleRefused,
    InvalidPort,
    ConfigWriteFailed,
    SystemctlFailed,
    BindingWriteFailed,
    /// A code a newer command line publishes. Its sentence is still rendered;
    /// nothing is claimed about which state it names.
    Unrecognized,
}

impl ServiceCode {
    /// One published code, read.
    pub fn of(code: &str) -> Self {
        match code {
            "app_managed" => ServiceCode::AppManaged,
            "user_manager_unreachable" => ServiceCode::UserManagerUnreachable,
            "linger_denied" => ServiceCode::LingerDenied,
            "loginctl_absent" => ServiceCode::LoginctlAbsent,
            "no_identity" => ServiceCode::NoIdentity,
            "invalid_home" => ServiceCode::InvalidHome,
            "home_change_refused" => ServiceCode::HomeChangeRefused,
            "foreign_unit" => ServiceCode::ForeignUnit,
            "activation_timeout" => ServiceCode::ActivationTimeout,
            "health_unavailable" => ServiceCode::HealthUnavailable,
            "foreign_distribution" => ServiceCode::ForeignDistribution,
            "service_unbound" => ServiceCode::ServiceUnbound,
            "diagnostics_unavailable" => ServiceCode::DiagnosticsUnavailable,
            "idle_restart_unavailable" => ServiceCode::IdleRestartUnavailable,
            "lifecycle_refused" => ServiceCode::LifecycleRefused,
            "invalid_port" => ServiceCode::InvalidPort,
            "config_write_failed" => ServiceCode::ConfigWriteFailed,
            "systemctl_failed" => ServiceCode::SystemctlFailed,
            "binding_write_failed" => ServiceCode::BindingWriteFailed,
            _ => ServiceCode::Unrecognized,
        }
    }
}

/// How the installed and the running engine compare (M38 section 9.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    Aligned,
    PendingRestart,
    NotRunning,
    Unknown,
    OwnershipConflict,
    /// A verdict a newer CLI publishes. Nothing is claimed about it, which is
    /// the same posture `unknown` carries.
    #[serde(other)]
    Unrecognized,
}

/// Which home this account's service runs from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    Bound,
    Unbound,
    Invalid,
    #[serde(other)]
    Unrecognized,
}

/// Whether the account's services survive logout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Linger {
    Enabled,
    Disabled,
    Unknown,
    #[serde(other)]
    Unrecognized,
}

/// Where the listener's port came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerSource {
    Daemon,
    Config,
    Default,
    Unknown,
    #[serde(other)]
    Unrecognized,
}

/// The compiled identity against the manifest the package installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Integrity {
    Verified,
    Mismatched,
    Unreadable,
    #[serde(other)]
    Unrecognized,
}

/// `service status --json`, and `service install --json` on a packaged engine.
///
/// Every field the contract documents is decoded, and the ones it types as
/// nullable are the ones that are optional here. It is available with no daemon
/// running, which is the state it most has to answer in.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceStatus {
    pub binding: Binding,
    pub unit: Unit,
    pub enabled: bool,
    pub active: bool,
    pub sub_state: Option<String>,
    pub pid: Option<u32>,
    pub invocation_id: Option<String>,
    pub restart_count: u32,
    pub linger: Linger,
    /// Where the daemon's `PATH` comes from. The engine applies the shared
    /// baseline itself, so there is one answer for the service and the CLI.
    pub path_source: String,
    pub listener: Listener,
    pub installed: InstalledEngine,
    /// The identity the daemon that answered reported, where one did. It is
    /// `hello`'s own engine object, which is why it is that type.
    pub running: Option<EngineIdentity>,
    /// The typed update verdict.
    pub alignment: Alignment,
}

impl ServiceStatus {
    /// Whether the service manager is restarting a unit that keeps dying, which
    /// is the difference between a slow first start and a crash loop.
    pub fn is_restarting(&self) -> bool {
        self.sub_state.as_deref() == Some(AUTO_RESTART)
    }

    /// Whether the unit is running and the command line could not negotiate
    /// with what is on the socket. The running identity comes from `hello`, so
    /// its absence under an active unit is a daemon this build cannot manage.
    pub fn answers_nothing(&self) -> bool {
        self.active && self.running.is_none()
    }

    /// The home the service is bound to, where one is bound.
    ///
    /// An unbound or unreadable binding names no home, and the contract types
    /// it that way: `home` is null unless the state is `bound`.
    pub fn bound_home(&self) -> Option<&str> {
        if self.binding.state != BindingState::Bound {
            return None;
        }

        self.binding.home.as_deref().filter(|home| !home.is_empty())
    }

    /// The distribution of the engine that is running, where one answered and
    /// it is not the packaged one.
    pub fn foreign_distribution(&self) -> Option<&str> {
        let running = self.running.as_ref()?;
        let distribution = running.distribution_identity.as_str();

        (distribution != PACKAGE_DISTRIBUTION).then_some(distribution)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Binding {
    pub state: BindingState,
    pub home: Option<String>,
    /// Why the binding could not be read. The engine's own sentence, and null
    /// unless the state is `invalid`.
    pub reason: Option<String>,
}

/// Which service file is in force.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Unit {
    pub effective_path: Option<String>,
    /// Whether `effective_path` is the package's own unit.
    pub vendor: bool,
    /// Whether a unit an earlier Fermix wrote is present in the user unit
    /// directory.
    pub legacy_generated: bool,
    /// Whether a unit Fermix did not write is present there. Fermix never
    /// rewrites one.
    pub foreign: bool,
    /// Whether the unit on disk changed since the service manager read it.
    pub need_daemon_reload: bool,
}

/// The web listener the setup page is served on.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Listener {
    pub port: Option<u16>,
    pub origin: Option<String>,
    pub source: ListenerSource,
}

/// The engine identity compiled into the `fermix` on disk.
///
/// It is `hello`'s identity plus the integrity verdict and without a pid,
/// because nothing is running for it to have one.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstalledEngine {
    pub engine_id: String,
    pub product_version: String,
    pub build_id: Option<String>,
    pub source_commit: Option<String>,
    pub distribution_identity: String,
    pub artifact_target: Option<String>,
    pub architecture: String,
    pub integrity: Integrity,
}

/// What `service uninstall`, and `service install` on an engine that writes its
/// own unit file, answer with: what was done rather than a service state.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ActionResult {
    pub action: ActionKind,
    pub scope: ActionScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Installed,
    Uninstalled,
    #[serde(other)]
    Unrecognized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionScope {
    User,
    System,
    #[serde(other)]
    Unrecognized,
}

/// `service install --json`, which answers in one of two shapes.
///
/// A packaged engine owns one user service and answers with the service result;
/// an engine that writes its own unit file answers with what it did. This
/// application drives a packaged engine, so the second arm is what it meets
/// when it is pointed at something else, and it is decoded rather than refused.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum InstallOutcome {
    Service(Box<ServiceStatus>),
    Action(ActionResult),
}

impl InstallOutcome {
    /// The service state an install left behind, where the answer carried one.
    pub fn status(&self) -> Option<&ServiceStatus> {
        match self {
            InstallOutcome::Service(status) => Some(status),
            InstallOutcome::Action(_) => None,
        }
    }
}

/// `restart --json`.
///
/// `pid` is always different from `previous_pid`: an answer alone is not a new
/// generation, because the restart job replies before the old process stops.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Restart {
    /// The generation that was replaced. Null when no daemon was answering,
    /// which is the recovery case rather than a refusal.
    pub previous_pid: Option<String>,
    pub pid: String,
    pub alignment: Alignment,
}

/// `diagnostics export --offline --json`.
///
/// The object is written to the file the person chose. One part of it is read:
/// the log tail, which the Boot failed screen and Recovery both show as their
/// offline evidence. Everything else is carried rather than interpreted, which
/// is why each source's `data` stays a value.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiagnosticsExport {
    /// The bundle's own version, which is not the envelope's and not the
    /// management protocol's.
    pub schema_version: u32,
    pub generated_at: String,
    pub mode: String,
    pub sources: std::collections::BTreeMap<String, DiagnosticsSource>,
}

/// One source of the bundle. Nothing omitted is read as healthy: a source that
/// could not be read says so and says why.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiagnosticsSource {
    pub status: SourceStatus,
    pub observed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Available,
    Unavailable,
    NotApplicable,
    #[serde(other)]
    Unrecognized,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn golden(relative: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("contracts/cli/fixtures")
            .join(relative);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()));
        serde_json::from_slice(&bytes).expect("a golden parses")
    }

    fn status(relative: &str) -> ServiceStatus {
        serde_json::from_value(golden(relative)["result"].clone()).expect("the golden decodes")
    }

    #[test]
    fn the_state_a_fresh_host_publishes_is_unbound_with_nothing_running() {
        let status = status("service_status/fresh.json");

        assert_eq!(status.binding.state, BindingState::Unbound);
        assert_eq!(status.bound_home(), None);
        assert_eq!(status.alignment, Alignment::NotRunning);
        assert_eq!(status.linger, Linger::Disabled);
        assert_eq!(status.listener.source, ListenerSource::Unknown);
        assert!(status.listener.port.is_none());
        assert!(status.running.is_none());
        assert!(!status.answers_nothing(), "nothing is active to answer");
    }

    #[test]
    fn a_binding_that_could_not_be_read_carries_the_engines_own_sentence() {
        let status = status("service_status/invalid_binding.json");

        assert_eq!(status.binding.state, BindingState::Invalid);
        assert_eq!(
            status.bound_home(),
            None,
            "an invalid binding names no home"
        );
        assert!(
            status
                .binding
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("absolute path")),
            "the reason is the engine's, rendered rather than composed"
        );
    }

    #[test]
    fn the_unit_in_force_is_four_facts_and_a_path() {
        let vendor = status("service_status/active_aligned.json");
        assert!(vendor.unit.vendor);
        assert!(!vendor.unit.foreign);
        assert!(!vendor.unit.legacy_generated);
        assert!(!vendor.unit.need_daemon_reload);
        assert_eq!(
            vendor.unit.effective_path.as_deref(),
            Some("/usr/lib/systemd/user/fermix.service")
        );

        // A unit Fermix did not write is published in the status rather than
        // refusing the read: the refusal belongs to the verbs that would have
        // changed it.
        let foreign = status("service_status/foreign_unit.json");
        assert!(foreign.unit.foreign);
        assert!(!foreign.unit.vendor);

        let legacy = status("service_status/legacy_unit.json");
        assert!(legacy.unit.legacy_generated);
        assert!(!legacy.unit.foreign);
    }

    #[test]
    fn an_engine_from_another_distribution_is_named_rather_than_assumed_packaged() {
        let conflict = status("service_status/ownership_conflict.json");
        assert_eq!(conflict.alignment, Alignment::OwnershipConflict);
        assert_eq!(conflict.foreign_distribution(), Some("standalone"));

        let aligned = status("service_status/active_aligned.json");
        assert_eq!(aligned.foreign_distribution(), None);
        assert_eq!(aligned.installed.integrity, Integrity::Verified);
        assert_eq!(aligned.pid, Some(4711));
    }

    #[test]
    fn a_daemon_that_reported_no_generation_is_unknown_rather_than_stale() {
        let status = status("service_status/unknown_identity.json");

        assert_eq!(status.alignment, Alignment::Unknown);
        let running = status.running.as_ref().expect("a daemon answered");
        assert!(
            running.build_id.is_none(),
            "the golden omits the key entirely, which is the case this reads"
        );
        assert_eq!(running.pid, "4711");
    }

    #[test]
    fn a_generation_behind_the_installed_one_is_a_restart_rather_than_a_mismatch() {
        let status = status("service_status/pending_restart.json");

        assert_eq!(status.alignment, Alignment::PendingRestart);
        let installed = status.installed.build_id.as_deref();
        let running = status
            .running
            .as_ref()
            .and_then(|engine| engine.build_id.as_deref());
        assert_ne!(installed, running, "a build id is compared with a build id");
    }

    #[test]
    fn an_install_answers_either_a_service_state_or_what_it_did() {
        let packaged: InstallOutcome =
            serde_json::from_value(golden("service_install/packaged.json")["result"].clone())
                .expect("decodes");
        assert_eq!(
            packaged.status().map(|status| status.alignment),
            Some(Alignment::Aligned),
            "a packaged engine answers with the service result itself"
        );

        let unit: InstallOutcome =
            serde_json::from_value(golden("service_install/unit.json")["result"].clone())
                .expect("decodes");
        assert!(unit.status().is_none());
        match unit {
            InstallOutcome::Action(action) => {
                assert_eq!(action.action, ActionKind::Installed);
                assert_eq!(action.scope, ActionScope::User);
            }
            InstallOutcome::Service(_) => panic!("an action result is not a service state"),
        }
    }

    #[test]
    fn a_restart_with_no_previous_generation_is_a_recovery_rather_than_a_refusal() {
        let recovered: Restart =
            serde_json::from_value(golden("restart/recovered.json")["result"].clone())
                .expect("decodes");
        assert!(recovered.previous_pid.is_none());
        assert_eq!(recovered.pid, "4822");
        assert_eq!(recovered.alignment, Alignment::Aligned);

        let ordinary: Restart =
            serde_json::from_value(golden("restart/ok.json")["result"].clone()).expect("decodes");
        assert_eq!(ordinary.previous_pid.as_deref(), Some("4711"));
        assert_ne!(ordinary.pid, "4711");
    }

    #[test]
    fn a_source_that_could_not_be_read_says_so_rather_than_going_missing() {
        let export: DiagnosticsExport =
            serde_json::from_value(golden("diagnostics_export/degraded.json")["result"].clone())
                .expect("decodes");

        assert_eq!(export.mode, "offline");
        let doctor = export.sources.get("doctor").expect("a doctor source");
        assert_eq!(doctor.status, SourceStatus::Unavailable);
        assert!(doctor.reason.is_some());
        assert!(doctor.data.is_none());

        let engine = export.sources.get("engine").expect("an engine source");
        assert_eq!(engine.status, SourceStatus::Available);
        assert!(engine.data.is_some());
    }

    #[test]
    fn every_code_the_command_line_publishes_is_read_as_itself() {
        assert_eq!(ServiceCode::of("linger_denied"), ServiceCode::LingerDenied);
        assert_eq!(
            ServiceCode::of("idle_restart_unavailable"),
            ServiceCode::IdleRestartUnavailable
        );
        assert_eq!(
            ServiceCode::of("a_code_from_a_newer_command_line"),
            ServiceCode::Unrecognized
        );
    }

    #[test]
    fn a_status_missing_a_field_the_contract_publishes_is_refused() {
        let mut result = golden("service_status/active_aligned.json")["result"].clone();
        result
            .as_object_mut()
            .expect("an object")
            .remove("alignment");

        let refused: Result<ServiceStatus, _> = serde_json::from_value(result);
        assert!(
            refused.is_err(),
            "a half-written status is louder than a status with a guessed field"
        );
    }
}
