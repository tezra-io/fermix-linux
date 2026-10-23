//! The public keys the app trusts the update feed from, in one reviewable
//! place (update-feed-spec, "Keys and signing"): desktop/packaging/update-keys.json,
//! the same file the release signs against, compiled in. It names the current
//! key and a standby key, so a key can be retired one release after the
//! standby takes over. The private halves are the release environment's
//! secrets and never leave it. The fixtures' `linux-feed-test` key is never
//! one of these.

use crate::update::feed::FeedKey;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::Deserialize;

/// desktop/packaging/update-keys.json, as built.
pub const KEYS_JSON: &str = include_str!("../../../packaging/update-keys.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeysFile {
    format: u32,
    keys: Vec<KeyEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyEntry {
    key_id: String,
    role: String,
    public_key: String,
}

/// The keys of [`KEYS_JSON`]: the current key, then the standby. The file is
/// a constant the tests read, so a bad one is a build that must not ship.
pub fn production_keys() -> Vec<FeedKey> {
    read_keys(KEYS_JSON).unwrap_or_else(|e| panic!("update-keys.json: {e}"))
}

/// A keys file in update-keys.json's form: format 1, a current key and a
/// standby key, each a 32-byte public key in standard base64.
pub fn read_keys(text: &str) -> Result<Vec<FeedKey>, String> {
    let file: KeysFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.format != 1 {
        return Err(format!("format {} is not 1", file.format));
    }
    let roles: Vec<&str> = file.keys.iter().map(|k| k.role.as_str()).collect();
    if roles != ["current", "standby"] {
        return Err(format!("the roles are {roles:?}, not current then standby"));
    }
    file.keys.iter().map(key).collect()
}

fn key(entry: &KeyEntry) -> Result<FeedKey, String> {
    let id = &entry.key_id;
    let bytes = STANDARD
        .decode(&entry.public_key)
        .map_err(|e| format!("{id} is not base64: {e}"))?;
    let public: [u8; 32] = bytes
        .try_into()
        .map_err(|_| format!("{id} is not 32 bytes"))?;
    Ok(FeedKey {
        id: id.clone(),
        public,
    })
}
