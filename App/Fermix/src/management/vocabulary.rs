//! The closed vocabularies the wire publishes as atoms, read as types.
//!
//! Every atom the daemon publishes is spelled here and nowhere else. A model or
//! a view that had to compare a status against `"setup_required"` would be
//! keeping its own copy of a vocabulary the daemon owns, so the comparison
//! happens once, in the decoding layer, and everything above it matches on a
//! Rust enum. `tests/structure.rs` holds the line.
//!
//! Nothing here renders a word. The catalogue owns the English for these atoms;
//! this module owns only the reading.

use super::errors::WireError;
use super::types::{ReadinessFailure, SetupStateResult};

/// How ready the daemon says it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Everything configured is present.
    Ready,
    /// At least one gap the person has to close.
    SetupRequired,
    /// A word this build has never seen, or none at all. Nothing is claimed.
    Unknown,
}

impl Readiness {
    /// The status the daemon published, as a type.
    pub fn parse(status: Option<&str>) -> Self {
        match status {
            Some("ready") => Readiness::Ready,
            Some("setup_required") => Readiness::SetupRequired,
            _ => Readiness::Unknown,
        }
    }
}

/// What one daemon-reported gap is.
///
/// The wire carries a `detail_key` per readiness failure and, separately, the
/// standing coexistence descriptors of M38 section 5.8. Both are read into one
/// vocabulary here, because a row's wording is keyed on what the gap *is* and
/// never on the pane it happens to route to: five channels and voice collapse
/// onto two panes, so a pane cannot tell Telegram from Slack.
///
/// A key this build has never seen is kept rather than folded onto a
/// neighbour: it renders under the component the daemon named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gap {
    /// Fermix does not know who it is working for yet.
    Personalization,
    /// No provider it can answer with. The id is the daemon's, where it named
    /// one; the family is one row because the Linux deck carries one provider
    /// template (M38 section 6.5's casing column, not a per-reason deck).
    Provider(Option<String>),
    /// One channel is switched on and not finished.
    Channel(String),
    /// Voice is switched on and not finished.
    Realtime,
    /// Settings changed since the daemon started.
    RestartPending,
    /// The settings file was written by something other than Fermix.
    ExternalConfigChange,
    /// The settings file cannot be read.
    ConfigUnreadable,
    /// A generated unit is shadowing the packaged one.
    LegacyServiceUnit,
    /// The resolved PATH is not the one the packaged engine installs with.
    EnginePathBaseline,
    /// A gap this build has no template for. It renders under the daemon's own
    /// component name, never under a stranger's sentence.
    Unrecognized(String),
}

/// The two parameterised families, by their key prefix.
const PROVIDER_CREDENTIALS_PREFIX: &str = "provider:missing_credentials:";
const PROVIDER_PREFIX: &str = "provider:";
const CHANNEL_PREFIX: &str = "channel:";
const REALTIME_PREFIX: &str = "realtime:";

impl Gap {
    /// One `detail_key`, read.
    pub fn parse(detail_key: &str) -> Self {
        match detail_key {
            "personalization" => return Gap::Personalization,
            "restart_pending" => return Gap::RestartPending,
            "external_config_change" => return Gap::ExternalConfigChange,
            "config_unreadable" => return Gap::ConfigUnreadable,
            "legacy_service_unit" => return Gap::LegacyServiceUnit,
            "engine_path_baseline" => return Gap::EnginePathBaseline,
            _ => {}
        }

        if let Some(provider) = detail_key.strip_prefix(PROVIDER_CREDENTIALS_PREFIX) {
            return Gap::Provider(non_empty(provider));
        }
        if detail_key.starts_with(PROVIDER_PREFIX) {
            return Gap::Provider(None);
        }
        if let Some(channel) = detail_key.strip_prefix(CHANNEL_PREFIX) {
            return match non_empty(channel) {
                Some(name) => Gap::Channel(name),
                None => Gap::Unrecognized(detail_key.to_string()),
            };
        }
        if detail_key.starts_with(REALTIME_PREFIX) {
            return Gap::Realtime;
        }

        Gap::Unrecognized(detail_key.to_string())
    }

    /// The identifier the daemon named inside this gap, where it named one.
    /// Rendered as the daemon wrote it: an id shown raw is visibly an id, and
    /// an id with its first letter raised is a spelling the app invented.
    pub fn identifier(&self) -> Option<&str> {
        match self {
            Gap::Provider(provider) => provider.as_deref(),
            Gap::Channel(name) => Some(name.as_str()),
            Gap::Unrecognized(key) => Some(key.as_str()),
            _ => None,
        }
    }
}

/// The wire's own spelling for one settings pane, and the reading back.
///
/// The daemon publishes a pane on every section and on every readiness failure,
/// and the application persists the selected one across launches. Both
/// directions are spelled here, so the one place that knows how a pane is
/// written down is the layer that decodes it.
pub fn pane_slug(pane: super::types::SettingsPane) -> Option<&'static str> {
    use super::types::SettingsPane as Pane;

    Some(match pane {
        Pane::Providers => "providers",
        Pane::Personality => "personality",
        Pane::Memory => "memory",
        Pane::Channels => "channels",
        Pane::Integrations => "integrations",
        Pane::Voice => "voice",
        Pane::Meetings => "meetings",
        Pane::Computer => "computer",
        Pane::Coding => "coding",
        Pane::Search => "search",
        Pane::Images => "images",
        Pane::Sandbox => "sandbox",
        Pane::Permissions => "permissions",
        // A pane a newer daemon publishes has no spelling this build can
        // persist or route to. The row still renders; only its deep link and
        // its place in the remembered selection are withheld.
        Pane::Unrecognized => return None,
    })
}

/// One pane, read back from what was persisted.
pub fn pane_for_slug(slug: &str) -> Option<super::types::SettingsPane> {
    use super::types::SettingsPane as Pane;

    [
        Pane::Providers,
        Pane::Personality,
        Pane::Memory,
        Pane::Channels,
        Pane::Integrations,
        Pane::Voice,
        Pane::Meetings,
        Pane::Computer,
        Pane::Coding,
        Pane::Search,
        Pane::Images,
        Pane::Sandbox,
        Pane::Permissions,
    ]
    .into_iter()
    .find(|pane| pane_slug(*pane) == Some(slug))
}

/// The configuration state, as a type the surfaces can match on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConfigCondition {
    /// Every write lands.
    #[default]
    Clear,
    /// The file was written by something else; every write refuses until it is
    /// read again.
    ExternalChange,
    /// The file cannot be read at all. There is no reload to offer, because the
    /// reload would re-run the read that failed.
    Unreadable,
    /// A state this build has never seen. Nothing is claimed.
    Unknown,
}

impl ConfigCondition {
    /// The state the daemon published, as a type.
    pub fn of(state: super::types::ConfigState) -> Self {
        match state {
            super::types::ConfigState::Clear => ConfigCondition::Clear,
            super::types::ConfigState::ExternalChange => ConfigCondition::ExternalChange,
            super::types::ConfigState::ConfigUnreadable => ConfigCondition::Unreadable,
            super::types::ConfigState::Unrecognized => ConfigCondition::Unknown,
        }
    }
}

/// The refusals a surface behaves differently about.
///
/// Everything else is one refusal with the daemon's own sentence, which is what
/// the surface renders. These are the four the product routes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The file was written by something else; every write refuses until it is
    /// read again.
    ExternalChange,
    /// The settings file cannot be read.
    ConfigUnreadable,
    /// The log cursor predates a rotation.
    CursorExpired,
    /// This host had nowhere to put a secret, so nothing was saved.
    SecretStoreFailed,
    /// Another operation of this kind is already running.
    Busy,
    /// Everything else: the sentence is the whole answer.
    Other,
}

impl Refusal {
    /// One wire error, read.
    pub fn of(error: &WireError) -> Self {
        Refusal::of_code(&error.code)
    }

    /// One published code, read. The sentence a surface carries keeps the code
    /// beside it, so a surface that routes on one never has to rebuild the
    /// envelope it came from.
    pub fn of_code(code: &str) -> Self {
        match code {
            "external_change" => Refusal::ExternalChange,
            "config_unreadable" => Refusal::ConfigUnreadable,
            "cursor_expired" => Refusal::CursorExpired,
            "secret_store_failed" => Refusal::SecretStoreFailed,
            "busy" => Refusal::Busy,
            _ => Refusal::Other,
        }
    }
}

/// Where one channel stands, as its own row publishes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStanding {
    /// Connected and answering.
    Working,
    /// Switched on with something still missing.
    NotFinished,
    /// The daemon published no status at all, which is not the same answer as
    /// a channel that is off.
    Unreported,
}

impl ChannelStanding {
    /// One published status, read.
    pub fn of(status: Option<&str>) -> Self {
        match status {
            Some("ok") => ChannelStanding::Working,
            Some("setup_required") => ChannelStanding::NotFinished,
            // A status a newer daemon mints has no word here, and a neighbour's
            // word would be a claim nothing on the wire made.
            _ => ChannelStanding::Unreported,
        }
    }
}

/// The targets `capabilities.install.start` takes, as the contract spells them.
pub const MEETBOT_TARGET: &str = "meetbot";
/// The computer-use helper, as the contract spells it.
pub const COMPUTER_USE_SIDECAR_TARGET: &str = "computer_use_sidecar";

/// The credential families `secret.set` takes, as the contract spells them.
///
/// Three of the four are composed rather than enumerated: a plugin's own token
/// is `plugin:<name>` and a sign-in client's secret is `oauth_client:<provider>`.
/// The fourth is one fixed id, because the daemon stores it through a different
/// mechanism and no descriptor row carries it.
pub const SETUP_TOKEN: &str = "anthropic_setup_token";

/// The `secret.set` and `auth.start` spelling for one plugin.
pub fn plugin_id(name: &str) -> String {
    format!("plugin:{name}")
}

/// The `secret.set` spelling for one sign-in client's secret.
pub fn oauth_client_id(provider: &str) -> String {
    format!("oauth_client:{provider}")
}

/// How a provider signs in, as its row publishes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// A browser hop.
    Oauth,
    /// A key the operator types.
    ApiKey,
    /// No credential at all.
    None,
    /// A mode a newer daemon publishes. Kept rather than folded onto a
    /// neighbour: a row offering the wrong door is worse than one offering
    /// none.
    Unrecognized,
}

impl AuthMode {
    /// One mode, read.
    pub fn parse(mode: &str) -> Self {
        match mode {
            "oauth" => AuthMode::Oauth,
            "api_key" => AuthMode::ApiKey,
            "none" => AuthMode::None,
            _ => AuthMode::Unrecognized,
        }
    }

    /// Every mode one provider publishes, read.
    pub fn of(modes: &[String]) -> Vec<AuthMode> {
        modes.iter().map(|mode| AuthMode::parse(mode)).collect()
    }

    /// How this mode is written on the wire, for the one row that writes a mode
    /// back.
    pub fn recorded(self) -> &'static str {
        match self {
            AuthMode::Oauth => "oauth",
            AuthMode::ApiKey => "api_key",
            AuthMode::None => "none",
            AuthMode::Unrecognized => "",
        }
    }
}

/// What a provider's stored token is worth, as the daemon reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    /// The credential is there and works as far as the daemon knows.
    Usable,
    /// The credential is there and no longer works.
    Stale,
    /// The daemon reported no token state at all, which is not the same answer
    /// as a stale one.
    Unreported,
}

impl TokenState {
    /// One token state, read.
    pub fn of(state: Option<&str>) -> Self {
        match state {
            None => TokenState::Unreported,
            Some("expired" | "invalid" | "revoked") => TokenState::Stale,
            Some(_) => TokenState::Usable,
        }
    }

    /// Whether the credential is there and no longer works.
    pub fn is_stale(self) -> bool {
        self == TokenState::Stale
    }
}

/// Every gap `setup.state.get` reports, in the order Home renders them:
/// the readiness failures the daemon listed, then the standing coexistence
/// descriptors it publishes separately.
///
/// `secret_acl_restricted` is deliberately absent: the Secret Service has no
/// per-item access-control list to be restricted by, so M38 section 5.8 refuses
/// the descriptor on Linux rather than carrying a row that always passed.
pub fn gaps(state: &SetupStateResult) -> Vec<Gap> {
    let mut found: Vec<Gap> = state
        .readiness
        .failures
        .iter()
        .map(|failure: &ReadinessFailure| Gap::parse(&failure.detail_key))
        .collect();

    if state.restart.required && !found.contains(&Gap::RestartPending) {
        found.push(Gap::RestartPending);
    }

    match ConfigCondition::of(state.coexistence.config_state) {
        ConfigCondition::ExternalChange => push_once(&mut found, Gap::ExternalConfigChange),
        ConfigCondition::Unreadable => push_once(&mut found, Gap::ConfigUnreadable),
        _ => {}
    }

    if state.coexistence.legacy_service_unit.present {
        push_once(&mut found, Gap::LegacyServiceUnit);
    }

    found
}

fn push_once(found: &mut Vec<Gap>, gap: Gap) {
    if !found.contains(&gap) {
        found.push(gap);
    }
}

fn non_empty(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(json: serde_json::Value) -> SetupStateResult {
        serde_json::from_value(json).expect("the fixture shape decodes")
    }

    fn minimal(failures: serde_json::Value, extra: serde_json::Value) -> SetupStateResult {
        let mut body = serde_json::json!({
            "readiness": {"status": "setup_required", "failures": failures},
            "restart": {"required": false, "reasons": []},
            "providers": [],
            "channels": [],
            "personalization": {"present": {
                "user_name": false, "timezone": false, "communication_style": false
            }},
            "features": {
                "voice": false, "voice_notes": false, "meetings": false, "computer_use": false,
                "computer_history": {"enabled": false, "installed": false, "ready": false}
            },
            "profile": null,
            "coexistence": {
                "legacy_service_unit": {"present": false, "scope": null, "path": null},
                "config_state": "clear",
                "secret_acl_restricted": {"present": null, "keys": []}
            }
        });

        merge(&mut body, extra);
        state(body)
    }

    fn merge(into: &mut serde_json::Value, from: serde_json::Value) {
        let (Some(target), Some(source)) = (into.as_object_mut(), from.as_object()) else {
            return;
        };
        for (key, value) in source {
            match target.get_mut(key) {
                Some(existing) if existing.is_object() && value.is_object() => {
                    merge(existing, value.clone())
                }
                _ => {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
    }

    #[test]
    fn readiness_is_read_from_the_word_the_daemon_published() {
        assert_eq!(Readiness::parse(Some("ready")), Readiness::Ready);
        assert_eq!(
            Readiness::parse(Some("setup_required")),
            Readiness::SetupRequired
        );
        assert_eq!(Readiness::parse(None), Readiness::Unknown);
        assert_eq!(
            Readiness::parse(Some("something_newer")),
            Readiness::Unknown
        );
    }

    #[test]
    fn every_published_detail_key_family_is_read() {
        assert_eq!(Gap::parse("personalization"), Gap::Personalization);
        assert_eq!(
            Gap::parse("provider:unknown_configured"),
            Gap::Provider(None)
        );
        assert_eq!(
            Gap::parse("provider:missing_credentials:openai"),
            Gap::Provider(Some("openai".into()))
        );
        assert_eq!(
            Gap::parse("channel:whatsapp"),
            Gap::Channel("whatsapp".into())
        );
        assert_eq!(Gap::parse("realtime:openai"), Gap::Realtime);
        assert_eq!(Gap::parse("restart_pending"), Gap::RestartPending);
        assert_eq!(
            Gap::parse("external_config_change"),
            Gap::ExternalConfigChange
        );
        assert_eq!(Gap::parse("legacy_service_unit"), Gap::LegacyServiceUnit);
        assert_eq!(Gap::parse("engine_path_baseline"), Gap::EnginePathBaseline);
    }

    #[test]
    fn a_key_this_build_has_never_seen_keeps_its_own_name() {
        assert_eq!(
            Gap::parse("a_gap_from_the_future"),
            Gap::Unrecognized("a_gap_from_the_future".into())
        );
        assert_eq!(
            Gap::parse("channel:").identifier(),
            Some("channel:"),
            "an empty parameter is not a channel called nothing"
        );
    }

    #[test]
    fn the_gaps_of_a_state_are_its_failures_then_its_standing_descriptors() {
        let state = minimal(
            serde_json::json!([
                {"component": "personalization", "gating": true, "pane": "personality",
                 "detail_key": "personalization"},
                {"component": "channel:whatsapp", "gating": false, "pane": "channels",
                 "detail_key": "channel:whatsapp"}
            ]),
            serde_json::json!({
                "restart": {"required": true, "reasons": []},
                "coexistence": {
                    "config_state": "external_change",
                    "legacy_service_unit": {"present": true, "scope": "user", "path": "/x"}
                }
            }),
        );

        assert_eq!(
            gaps(&state),
            vec![
                Gap::Personalization,
                Gap::Channel("whatsapp".into()),
                Gap::RestartPending,
                Gap::ExternalConfigChange,
                Gap::LegacyServiceUnit,
            ]
        );
    }

    #[test]
    fn a_restart_the_daemon_already_listed_is_not_listed_twice() {
        let state = minimal(
            serde_json::json!([
                {"component": "restart", "gating": false, "pane": "providers",
                 "detail_key": "restart_pending"}
            ]),
            serde_json::json!({"restart": {"required": true, "reasons": []}}),
        );

        assert_eq!(gaps(&state), vec![Gap::RestartPending]);
    }

    #[test]
    fn a_restricted_secret_list_is_never_a_row_on_this_platform() {
        let state = minimal(
            serde_json::json!([]),
            serde_json::json!({
                "coexistence": {"secret_acl_restricted": {"present": true, "keys": ["a"]}}
            }),
        );

        assert!(gaps(&state).is_empty());
    }

    #[test]
    fn every_pane_the_daemon_publishes_round_trips_through_its_slug() {
        use crate::management::types::SettingsPane as Pane;

        for pane in [
            Pane::Providers,
            Pane::Personality,
            Pane::Memory,
            Pane::Channels,
            Pane::Integrations,
            Pane::Voice,
            Pane::Meetings,
            Pane::Computer,
            Pane::Coding,
            Pane::Search,
            Pane::Images,
            Pane::Sandbox,
            Pane::Permissions,
        ] {
            let slug = pane_slug(pane).expect("every published pane is spelled");
            assert_eq!(pane_for_slug(slug), Some(pane));
            assert_eq!(
                serde_json::from_value::<Pane>(serde_json::json!(slug)).expect("decodes"),
                pane,
                "{slug} is not the spelling the wire uses"
            );
        }

        assert_eq!(pane_slug(Pane::Unrecognized), None);
        assert_eq!(pane_for_slug("a_pane_from_the_future"), None);
    }

    #[test]
    fn the_four_refusals_the_product_routes_on_are_read_and_the_rest_are_one() {
        let refusal = |code: &str| {
            Refusal::of(&WireError {
                code: code.to_string(),
                message: String::new(),
                sentence: None,
                details: serde_json::Map::new(),
            })
        };

        assert_eq!(refusal("external_change"), Refusal::ExternalChange);
        assert_eq!(refusal("config_unreadable"), Refusal::ConfigUnreadable);
        assert_eq!(refusal("cursor_expired"), Refusal::CursorExpired);
        assert_eq!(refusal("busy"), Refusal::Busy);
        assert_eq!(refusal("internal_error"), Refusal::Other);
    }
}
