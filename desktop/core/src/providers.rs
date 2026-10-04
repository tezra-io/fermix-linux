//! How each provider gets a credential ("doors") and whether it has one.
//!
//! `auth_modes` alone is not the answer: Anthropic publishes `oauth` there, but the
//! daemon has no browser sign-in for it and refuses `auth.start anthropic`. Its ways
//! in are an imported Claude Code login and a setup token. The macOS app keeps the
//! same table (`ProviderRowProjection` in fermix-macos).
//!
//! OpenAI Codex signs in with ChatGPT and runs on the person's ChatGPT plan (M57).
//! Its browser door is named by OpenAI's guidelines, and the daemon refuses the
//! Codex CLI import it used to offer.

use crate::model::ProviderRow;

/// Every provider `auth.start` will start a browser sign-in for.
const BROWSER_SIGN_IN: [&str; 2] = [CHATGPT_PLAN, "xai"];
/// The provider that signs in with ChatGPT and answers on the person's plan.
pub const CHATGPT_PLAN: &str = "openai_codex";
/// ChatGPT's own page for seeing and capping what Fermix uses of the plan.
pub const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/settings/usage";
/// The provider page's line while Fermix answers on the plan, in OpenAI's words.
pub const USING_PLAN: &str = "Using your ChatGPT plan";
/// The button that opens `CHATGPT_USAGE_URL`, and what it says to a screen reader.
pub const MANAGE_USAGE: &str = "Manage usage";
pub const MANAGE_USAGE_HINT: &str = "Opens ChatGPT settings in your browser.";
/// OpenAI's one-time notice for a first sign-in on the plan, word for word.
pub const PLAN_NOTICE_TITLE: &str = "You're using your ChatGPT plan";
pub const PLAN_NOTICE_BODY: &str =
    "Eligible usage in Fermix uses your ChatGPT plan. Manage usage in your ChatGPT settings.";
/// Token states that mean a credential is stored and no longer works.
const STALE_TOKEN_STATES: [&str; 3] = ["expired", "invalid", "revoked"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    ClaudeCode,
}

impl ImportSource {
    pub fn wire(self) -> &'static str {
        match self {
            ImportSource::ClaudeCode => "claude_code",
        }
    }

    /// The product the login is imported from, as people call it.
    pub fn product(self) -> &'static str {
        match self {
            ImportSource::ClaudeCode => "Claude Code",
        }
    }

    fn for_provider(id: &str) -> Option<ImportSource> {
        match id {
            "anthropic" => Some(ImportSource::ClaudeCode),
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
            "setup_token" => Door::SetupToken,
            "api_key" => Door::ApiKey,
            _ => return None,
        };
        Some((provider.to_owned(), door))
    }

    /// The button or menu words for this way into `provider`. `again` is for a
    /// provider that already holds a sign-in, working or expired. The ChatGPT door
    /// reads the same either way: it is also how a person reconnects.
    pub fn verb(self, provider: &str, again: bool) -> String {
        match self {
            Door::BrowserSignIn if provider == CHATGPT_PLAN => "Continue with ChatGPT".into(),
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

/// Whether Fermix answers on this row's ChatGPT plan: a working ChatGPT sign-in.
pub fn uses_chatgpt_plan(row: &ProviderRow) -> bool {
    row.id == CHATGPT_PLAN && connection(row) == Connection::Connected
}

/// What signing out of `provider` ends, said before it is done. ChatGPT's
/// sign-out revokes its session upstream; every other sign-in is only forgotten.
pub fn sign_out_sentence(provider: &str) -> &'static str {
    if provider == CHATGPT_PLAN {
        "Fermix will stop using your ChatGPT plan and disconnect from your ChatGPT account."
    } else {
        "Fermix forgets this sign-in on this computer. Nothing is revoked at the provider."
    }
}
