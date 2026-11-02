//! Always On on the wire (Always On setup §3): `home.status`'s summary, the one screen every
//! surface draws, and `home.act`, the one call every step is. The engine words every sentence
//! and label, decides which steps run in what order, and answers each act with the summary after
//! it; the app collects what an action `needs` and sends it with the key the action was drawn
//! with. Decoding and the two calls only; what a screen shows of it is `always_on.rs`'s.

use crate::management::{CallError, Management};
use crate::provision::Pick;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::fmt;

pub const PASSPHRASE_MISMATCH: &str = "The two passphrases are not the same.";
/// The most bytes of an action's key, as the contract bounds it.
const KEY_BOUND: usize = 64;

/// `home.status`: the app reads its Always On summary and nothing else. An engine that serves
/// `home.act` always sends one.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct HomeStatus {
    pub summary: Summary,
}

/// The screen a summary maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryState {
    Off,
    Connect,
    Choose,
    Creating,
    Ready,
    Moving,
    On,
    Undone,
    Retired,
    Candidate,
    Primary,
    Problem,
    #[serde(other)]
    Unknown,
}

/// What one summary action is. Every one is a step `home.act` runs, but the three the surface
/// opens itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionId {
    CreateServer,
    Cancel,
    MoveNow,
    TryAgain,
    LearnMore,
    TurnOn,
    RestartServer,
    StartAgain,
    CreateIn,
    OpenConsole,
    PasteToken,
    Update,
    BackUpNow,
    ReplaceServer,
    CheckAgain,
    OwnServer,
    DeleteServer,
    /// On a computer the home moved from: the server gives the home back to it.
    MoveBack,
    /// Where the server Fermix created is gone: deleted first, then this computer's copy restored.
    RestoreHere,
    /// An action a newer engine offers that this app has no button for.
    #[serde(other)]
    Unknown,
}

impl ActionId {
    /// Learn more and Open console open their `url`, and Update is the app's own updater:
    /// `home.act` refuses all three.
    pub fn opened_here(self) -> bool {
        matches!(
            self,
            ActionId::LearnMore | ActionId::OpenConsole | ActionId::Update
        )
    }
}

/// What a surface collects before it acts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// The provider's token, from the token screens `detail.providers` lists.
    Token,
    /// The recovery passphrase set before: typed once, and the engine checks it.
    RecoveryPassphrase,
    /// None is set yet: typed twice and compared here.
    NewRecoveryPassphrase,
    /// The size and place, from `home.provision.options`; the recommendation by default.
    Choice,
    /// A need a newer engine asks for that this app has no way to ask.
    #[serde(other)]
    Unknown,
}

/// One action a summary offers, the same record wherever it sits: its label, what it needs, the
/// sentence the owner accepts first, the page a surface opens for it, and the key it is sent
/// with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
    pub needs: Vec<Need>,
    pub confirm: Option<String>,
    pub url: Option<String>,
    pub key: String,
}

/// One line of Details, finished by the engine.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Line {
    pub label: String,
    pub value: String,
}

/// One provider's token screen, in the engine's words: `id` is what a token is given for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// The token field's label.
    pub token_label: String,
    /// What the token screen says.
    pub sentence: String,
    /// The "How to get one" steps.
    pub steps: Vec<String>,
    pub console_url: String,
    /// The Always On docs page at this provider's part.
    pub docs_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Ok,
    Warn,
    Block,
    #[serde(other)]
    Unknown,
}

/// One warning or block of the move's review, shown as the engine words it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReadinessItem {
    pub status: ItemStatus,
    pub sentence: String,
}

/// What the state draws. A field the state has nothing for is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Detail {
    pub providers: Vec<Provider>,
    pub lines: Vec<Line>,
    /// The sentence of the step in hand, while a step's job runs.
    pub progress: Option<String>,
    pub alternatives: Vec<Action>,
    /// `on`'s other actions.
    pub actions: Vec<Action>,
    pub billing: Option<String>,
    pub readiness: Vec<ReadinessItem>,
    /// What lives on the server, on `on`.
    pub data: Option<String>,
    pub note: Option<String>,
}

/// The occurrence an app raises as a system notification; its id stays while it stands.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Notice {
    pub id: String,
    pub sentence: String,
}

/// `home.status.summary`: where Always On stands, in one sentence and at most one action.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Summary {
    pub key: String,
    pub state: SummaryState,
    pub sentence: String,
    pub action: Option<Action>,
    pub detail: Detail,
    pub notice: Option<Notice>,
}

impl Summary {
    /// Every action the summary offers, wherever it sits.
    pub fn every_action(&self) -> impl Iterator<Item = &Action> {
        self.action
            .iter()
            .chain(&self.detail.alternatives)
            .chain(&self.detail.actions)
    }

    /// Whether the summary still offers `action` under the key it was drawn with: the engine's
    /// rule for a press, which a step held between two screens follows too.
    pub fn offers(&self, action: &Action) -> bool {
        self.every_action()
            .any(|a| a.id == action.id && a.key == action.key)
    }
}

// ---- What a step carries ------------------------------------------------------------------------

/// A provider's API token as it was pasted, trimmed. It goes out on the wire and nowhere else: no
/// print of it spells it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// The pasted text without the space around it; None when nothing but space was pasted. How
    /// a token must look is the daemon's rule, and its refusal says so.
    pub fn pasted(text: &str) -> Option<Token> {
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| Token(trimmed.to_owned()))
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(withheld)")
    }
}

/// The recovery passphrase, held only between its entry and the act that carries it. No print
/// of it spells it.
#[derive(Clone, PartialEq, Eq)]
pub struct Passphrase(String);

impl Passphrase {
    /// The passphrase typed once; None while nothing is typed. How long it must be is the
    /// daemon's rule, and its refusal says so.
    pub fn typed(text: &str) -> Option<Passphrase> {
        (!text.is_empty()).then(|| Passphrase(text.to_owned()))
    }
}

impl fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Passphrase(withheld)")
    }
}

/// The passphrase two entries agree on, where none is set yet. `Err(None)` while either is
/// empty, which says nothing yet; `Err(Some(sentence))` once both have words and they differ.
pub fn passphrase(first: &str, second: &str) -> Result<Passphrase, Option<&'static str>> {
    if first.is_empty() || second.is_empty() {
        return Err(None);
    }
    if first != second {
        return Err(Some(PASSPHRASE_MISMATCH));
    }
    Ok(Passphrase(first.to_owned()))
}

/// What one act carries, as the action's `needs` asked: a token (naming its provider where the
/// engine lists several), a recovery passphrase, or the size and place the owner was shown.
/// No print of it spells the token or the passphrase.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Input {
    token: Option<Token>,
    provider: Option<String>,
    passphrase: Option<Passphrase>,
    choice: Option<Pick>,
}

impl Input {
    /// An act that needs nothing.
    pub fn none() -> Input {
        Input::default()
    }

    pub fn token(token: Token, provider: Option<&str>) -> Input {
        assert!(
            provider.is_none_or(|p| !p.is_empty()),
            "a provider is named by its id"
        );
        Input {
            token: Some(token),
            provider: provider.map(str::to_owned),
            ..Input::default()
        }
    }

    pub fn passphrase(passphrase: Passphrase) -> Input {
        Input {
            passphrase: Some(passphrase),
            ..Input::default()
        }
    }

    /// The row the owner chose, the recommended one by default: the engine makes no choice
    /// itself, so what it creates is what was shown.
    pub fn choice(pick: Pick) -> Input {
        assert!(
            !pick.size.is_empty() && !pick.location.is_empty(),
            "a choice is a size and a place"
        );
        Input {
            choice: Some(pick),
            ..Input::default()
        }
    }

    /// `home.act`'s `input`, or none when the act carries nothing.
    fn params(&self) -> Option<Value> {
        let mut input = Map::new();
        if let Some(token) = &self.token {
            input.insert("token".into(), json!(token.0));
        }
        if let Some(provider) = &self.provider {
            input.insert("provider".into(), json!(provider));
        }
        if let Some(passphrase) = &self.passphrase {
            input.insert("recovery_passphrase".into(), json!(passphrase.0));
        }
        if let Some(pick) = &self.choice {
            input.insert("type".into(), json!(pick.size));
            input.insert("location".into(), json!(pick.location));
        }
        (!input.is_empty()).then_some(Value::Object(input))
    }
}

impl fmt::Debug for Input {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Input")
            .field("token", &self.token)
            .field("provider", &self.provider)
            .field("passphrase", &self.passphrase)
            .field("choice", &self.choice)
            .finish()
    }
}

// ---- What a step answers ------------------------------------------------------------------------

/// `own_server`'s result: the code, answered once and never in a status, so no print of this
/// type spells it, and the command for the server, which carries none.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct OwnServer {
    code: String,
    pub expires_at: String,
    pub command: String,
}

impl OwnServer {
    /// The code as the daemon answered it, for the screen and the clipboard and nothing else.
    pub fn code(&self) -> &str {
        &self.code
    }
}

impl fmt::Debug for OwnServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OwnServer")
            .field("code", &"withheld")
            .field("expires_at", &self.expires_at)
            .field("command", &self.command)
            .finish()
    }
}

/// A step's own result, where it has one.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ActResult {
    OwnServer(OwnServer),
    /// Move now and Try again: the move started, and on the computer it leaves the engine
    /// restarts into it.
    Moved {
        move_id: String,
    },
}

/// `home.act`'s answer: the summary after the step, and its result.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Acted {
    pub summary: Summary,
    pub result: Option<ActResult>,
}

impl Management {
    pub fn home_status(&self) -> Result<HomeStatus, CallError> {
        decoded("home.status", self.call("home.status", json!({}))?)
    }

    /// Runs one Always On step: `action` as the summary offered it under `key`, with what its
    /// `needs` collected. Its answer is never quoted in an error, since it can hold the code.
    pub fn home_act(&self, action: ActionId, key: &str, input: &Input) -> Result<Acted, CallError> {
        assert!(
            action != ActionId::Unknown && !action.opened_here(),
            "{action:?} is no step home.act runs"
        );
        assert!(
            !key.is_empty() && key.len() <= KEY_BOUND,
            "an act carries the key its action was drawn with"
        );
        let mut params = json!({ "action": action, "key": key });
        if let Some(input) = input.params() {
            params["input"] = input;
        }
        let answer = self.call("home.act", params)?;
        serde_json::from_value(answer)
            .map_err(|_| CallError::Protocol("home.act answered an unexpected shape".into()))
    }
}

fn decoded<T: DeserializeOwned>(method: &str, answer: Value) -> Result<T, CallError> {
    serde_json::from_value(answer)
        .map_err(|e| CallError::Protocol(format!("{method} answered an unexpected shape: {e}")))
}
