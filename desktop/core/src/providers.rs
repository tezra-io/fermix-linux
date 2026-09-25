//! How each provider gets a credential ("doors") and whether it has one.
//!
//! `auth_modes` alone is not the answer: Anthropic publishes `oauth` there, but the
//! daemon has no browser sign-in for it and refuses `auth.start anthropic`. Its ways
//! in are an imported Claude Code login and a setup token. The macOS app keeps the
//! same table (`ProviderRowProjection` in fermix-macos).

use crate::model::ProviderRow;

/// Every provider `auth.start` will start a browser sign-in for.
const BROWSER_SIGN_IN: [&str; 2] = ["openai_codex", "xai"];
/// Token states that mean a credential is stored and no longer works.
const STALE_TOKEN_STATES: [&str; 3] = ["expired", "invalid", "revoked"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    ClaudeCode,
    CodexCli,
}

impl ImportSource {
    pub fn wire(self) -> &'static str {
        match self {
            ImportSource::ClaudeCode => "claude_code",
            ImportSource::CodexCli => "codex_cli",
        }
    }

    /// The product the login is imported from, as people call it.
    pub fn product(self) -> &'static str {
        match self {
            ImportSource::ClaudeCode => "Claude Code",
            ImportSource::CodexCli => "Codex CLI",
        }
    }

    fn for_provider(id: &str) -> Option<ImportSource> {
        match id {
            "anthropic" => Some(ImportSource::ClaudeCode),
            "openai_codex" => Some(ImportSource::CodexCli),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Door {
    BrowserSignIn,
    Import(ImportSource),
    SetupToken,
    ApiKey,
}

impl Door {
    pub const ANTHROPIC_SETUP_TOKEN_ID: &'static str = "anthropic_setup_token";

    pub fn api_key_secret_id(provider: &str) -> String {
        format!("{provider}_api_key")
    }

    /// `"<provider>|<door>"`: the string a GTK action carries for "open this way in".
    pub fn target(self, provider: &str) -> String {
        let door = match self {
            Door::BrowserSignIn => "browser",
            Door::Import(ImportSource::ClaudeCode) => "import:claude_code",
            Door::Import(ImportSource::CodexCli) => "import:codex_cli",
            Door::SetupToken => "setup_token",
            Door::ApiKey => "api_key",
        };
        format!("{provider}|{door}")
    }

    pub fn parse_target(target: &str) -> Option<(String, Door)> {
        let (provider, door) = target.split_once('|')?;
        let door = match door {
            "browser" => Door::BrowserSignIn,
            "import:claude_code" => Door::Import(ImportSource::ClaudeCode),
            "import:codex_cli" => Door::Import(ImportSource::CodexCli),
            "setup_token" => Door::SetupToken,
            "api_key" => Door::ApiKey,
            _ => return None,
        };
        Some((provider.to_owned(), door))
    }

    /// The button or menu words for this way in. `again` is for a sign-in that expired.
    pub fn verb(self, again: bool) -> String {
        match self {
            Door::BrowserSignIn if again => "Sign in again".into(),
            Door::BrowserSignIn => "Sign in".into(),
            Door::Import(source) => format!("Import from {}", source.product()),
            Door::SetupToken => "Paste a setup token…".into(),
            Door::ApiKey => "Use an API key…".into(),
        }
    }
}

/// The ways in, most direct first.
pub fn doors(row: &ProviderRow) -> Vec<Door> {
    let offers = |mode: &str| row.auth_modes.iter().any(|m| m == mode);
    let mut doors = Vec::new();
    if offers("oauth") && BROWSER_SIGN_IN.contains(&row.id.as_str()) {
        doors.push(Door::BrowserSignIn);
    }
    if let Some(source) = ImportSource::for_provider(&row.id) {
        doors.push(Door::Import(source));
    }
    if row.id == "anthropic" {
        doors.push(Door::SetupToken);
    }
    if offers("api_key") {
        doors.push(Door::ApiKey);
    }
    doors
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Connection {
    Connected,
    Expired,
    NotConnected,
}

pub fn connection(row: &ProviderRow) -> Connection {
    if !row.configured {
        return Connection::NotConnected;
    }
    match row.auth_mode.as_deref() {
        Some("none") => Connection::Connected,
        Some("api_key") if row.present_key => Connection::Connected,
        Some("oauth") => match row.token_state.as_deref() {
            Some("valid") => Connection::Connected,
            Some(state) if STALE_TOKEN_STATES.contains(&state) => Connection::Expired,
            _ => Connection::NotConnected,
        },
        _ => Connection::NotConnected,
    }
}
