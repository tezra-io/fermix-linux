//! Where secrets live, and whether one can be stored right now.
//!
//! These are two questions, and this module keeps them apart on purpose. The
//! message this work replaces answered both with one sentence: "This host has
//! no unlocked keyring" is true when a keyring exists and is locked and when
//! none exists at all, so the owner could not tell which they were in, and the
//! next action it offered was wrong for both. A locked keyring reported as an
//! absent one is the same conflation one layer down, which is why `StoreKind`
//! and `Availability` are separate types rather than one enum with four words.
//!
//! Everything here is a pure reading of what the engine published. Nothing
//! infers a state from a timeout or a exit code: the engine reads the
//! collection's `Locked` property and says so, and this module only routes
//! what it said.

use crate::management::vocabulary::Refusal;

use super::settings_model::Sentence;

/// Where values live, as the engine names it.
///
/// Two values, and deliberately not three. Nothing can answer "no store":
/// a refused save answers an error, which has no result to carry one, and a
/// machine with no keyring still SAVES TO the keyring, the save simply
/// refuses. Whether a secret can be stored right now is [`Availability`], a
/// different question with a different field. Collapsing the two is what
/// produced the message this work replaced.
///
/// This is also the first store kind the engine has ever published, so there
/// is no earlier spelling to accept, and `pass` was never a backend it could
/// write to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// The desktop keyring, through the Secret Service.
    Keyring,
    /// A private file under the Fermix home, which the owner consented to.
    File,
}

impl StoreKind {
    /// The kind the engine named, or nothing when it named something this
    /// build does not know.
    pub fn of(word: &str) -> Option<Self> {
        match word {
            "keyring" => Some(StoreKind::Keyring),
            "file" => Some(StoreKind::File),
            _ => None,
        }
    }

    /// Whether there is anything here to move back to the keyring.
    ///
    /// Only the file store has values to migrate. Offering the return from
    /// the keyring would be a move to where the owner already is, and from
    /// `none` there is nothing stored to move at all.
    pub fn offers_return_to_keyring(self) -> bool {
        matches!(self, StoreKind::File)
    }

    /// The words for this store, as a person reads them.
    pub fn label(self) -> crate::copy::Key {
        match self {
            StoreKind::Keyring => crate::copy::Key::SecretStoreDesktopKeyring,
            StoreKind::File => crate::copy::Key::SecretStoreThisComputer,
        }
    }
}

/// Whether a secret can be stored right now.
///
/// The same three words the refusal reasons use, so a locked keyring says it
/// is locked on the doctor row rather than reporting an absence that is not
/// true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// A store is reachable and a write would land.
    Ready,
    /// A Secret Service is present and its default collection is locked.
    Locked,
    /// No Secret Service, or no way to ask one.
    Unavailable,
}

impl Availability {
    /// The state the engine named, or nothing for a word this build does not
    /// know.
    pub fn of(word: &str) -> Option<Self> {
        match word {
            "ready" => Some(Availability::Ready),
            "locked" => Some(Availability::Locked),
            "unavailable" => Some(Availability::Unavailable),
            _ => None,
        }
    }

    /// The word the engine publishes for this state.
    pub fn word(self) -> &'static str {
        match self {
            Availability::Ready => "ready",
            Availability::Locked => "locked",
            Availability::Unavailable => "unavailable",
        }
    }
}

impl std::fmt::Display for Availability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.word())
    }
}

/// Which of the three store refusals the owner is looking at.
///
/// One variant per published reason, because each one wants a different
/// screen. They are not ordered by severity and nothing falls back to a
/// neighbour: a reason this build does not recognise is `None` rather than the
/// nearest guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRefusal {
    /// A keyring is there and locked. This is the only one with something to
    /// unlock.
    KeyringLocked,
    /// No keyring on this computer at all.
    NoKeyring,
    /// The helper did not answer while the collection was not locked, so
    /// there is nothing to unlock and retrying is the only honest offer.
    HelperDidNotAnswer,
}

impl StoreRefusal {
    /// The refusal this sentence names, or nothing when it is not a store
    /// refusal or carries a reason this build does not know.
    ///
    /// An unknown reason deliberately routes nowhere. A future engine may
    /// publish a fourth word, and sending it to the locked dialog would offer
    /// an unlock for a state nobody here understands.
    pub fn of(sentence: &Sentence) -> Option<Self> {
        // The code is read through the vocabulary rather than compared to a
        // literal, because the wire's own words live in the decoding layer.
        if Refusal::of_code(sentence.code.as_deref()?) != Refusal::SecretStoreFailed {
            return None;
        }

        match sentence.reason.as_deref()? {
            "locked" => Some(StoreRefusal::KeyringLocked),
            "unavailable" => Some(StoreRefusal::NoKeyring),
            "timeout" => Some(StoreRefusal::HelperDidNotAnswer),
            _ => None,
        }
    }

    /// Whether this refusal has something an unlock could act on.
    ///
    /// Only the locked keyring does. Offering the unlock anywhere else is a
    /// button that cannot work: there is no collection to unlock when none
    /// exists, and nothing is locked when a helper merely hung.
    pub fn offers_unlock(self) -> bool {
        matches!(self, StoreRefusal::KeyringLocked)
    }
}
