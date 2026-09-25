//! Everything the window knows, in one place. Pages render from this; only the
//! controller in `app.rs` changes it.

use fermix_client::model::SetupState;
use fermix_client::overview::Overview;
use fermix_client::service::ServiceRead;
use fermix_client::view::{Activity, DaemonProblem, Recent};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long "just now" lasts on a row after the app changed it.
pub const RECENT_FOR: Duration = Duration::from_secs(120);

pub struct Snapshot {
    pub state: SetupState,
    pub version: String,
    pub pid: Option<String>,
    pub architecture: Option<String>,
    /// How the engine was installed (`linux_package` for this app to manage it).
    pub distribution: Option<String>,
    /// The management protocol version this app and the daemon speak.
    pub protocol: u64,
    /// `overview.get`, when the daemon answered it.
    pub overview: Option<Overview>,
}

/// The background service and the login registration, as last read.
#[derive(Default)]
pub struct Background {
    /// `None` until systemd has been asked.
    pub service: Option<ServiceRead>,
    /// A binding is known to exist (spec §6.5): its file is there, or a daemon
    /// answered since this window opened.
    pub binding: bool,
    /// Where a running "Run in the background" change is heading.
    pub service_change: Option<bool>,
    /// Why the last change was refused, shown under the switch until the next one.
    pub service_error: Option<String>,
    /// The last "Open at login" answer the desktop granted.
    pub opens_at_login: bool,
    /// Where a running "Open at login" request is heading.
    pub login_change: Option<bool>,
}

pub enum Connection {
    /// Nothing has been read yet.
    Connecting,
    Up(Box<Snapshot>),
    Down(DaemonProblem),
}

/// An authorize url the daemon handed out once, kept only while its sign-in runs.
pub struct Link {
    pub url: String,
    pub expires: Instant,
}

pub struct State {
    pub connection: Connection,
    pub activity: HashMap<String, Activity>,
    pub recent: HashMap<String, (Recent, Instant)>,
    pub links: HashMap<String, Link>,
    /// Set while a start or restart the app asked for is being waited on.
    pub waking: bool,
    /// Set when the last start or restart did not bring Fermix back.
    pub wake_failed: bool,
    pub background: Background,
}

impl State {
    pub fn new() -> Self {
        State {
            connection: Connection::Connecting,
            activity: HashMap::new(),
            recent: HashMap::new(),
            links: HashMap::new(),
            waking: false,
            wake_failed: false,
            background: Background::default(),
        }
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        match &self.connection {
            Connection::Up(snapshot) => Some(snapshot.as_ref()),
            _ => None,
        }
    }

    pub fn activity(&self, provider: &str) -> Activity {
        self.activity
            .get(provider)
            .cloned()
            .unwrap_or(Activity::Idle)
    }

    pub fn recent(&self, provider: &str, now: Instant) -> Option<Recent> {
        let (recent, at) = self.recent.get(provider)?;
        (now.duration_since(*at) < RECENT_FOR).then_some(*recent)
    }

    pub fn label(&self, provider: &str) -> String {
        self.snapshot()
            .and_then(|s| s.state.providers.iter().find(|p| p.id == provider))
            .map_or_else(|| provider.to_owned(), |p| p.label.clone())
    }

    /// The running job for this provider, if the app is following one.
    pub fn job_id(&self, provider: &str) -> Option<String> {
        match self.activity.get(provider) {
            Some(Activity::Job { job_id, .. }) if !job_id.is_empty() => Some(job_id.clone()),
            _ => None,
        }
    }

    /// Every job the app is following, on any row.
    pub fn followed_jobs(&self) -> Vec<String> {
        self.activity
            .keys()
            .filter_map(|provider| self.job_id(provider))
            .collect()
    }

    /// Whether any row is waiting on the daemon; a refresh then waits its turn.
    pub fn in_flight(&self) -> bool {
        self.waking
            || self.background.service_change.is_some()
            || self.activity.values().any(Activity::in_flight)
    }
}
