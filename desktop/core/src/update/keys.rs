//! The public keys the app trusts the update feed from, in one reviewable
//! place (update-feed-spec, "Keys and signing"): the current key and a standby
//! key, so a key can be retired one release after the standby takes over. The
//! private halves are the release environment's secrets and never leave it.
//! The fixtures' `linux-feed-test` key is never one of these.

use crate::update::feed::FeedKey;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;

/// `(key_id, standard base64 of the 32-byte ed25519 public key)`.
pub const PRODUCTION: [(&str, &str); 2] = [
    // Current.
    (
        "linux-feed-1",
        "E51Uv2uksKNCEBeij/hJZ7dRgiUNFsz5pmzyX8iXwfY=",
    ),
    // Standby.
    (
        "linux-feed-2",
        "WLIAl6ZJLQcRV5RV87cIJ3L/ALzLZrJI0jEemRMJG2A=",
    ),
];

/// The keys above, decoded. Each is a constant the tests decode, so a bad one
/// is a build that must not ship.
pub fn production_keys() -> Vec<FeedKey> {
    PRODUCTION
        .iter()
        .map(|(id, public)| {
            let bytes = STANDARD
                .decode(public)
                .unwrap_or_else(|e| panic!("{id} is not base64: {e}"));
            let public: [u8; 32] = bytes
                .try_into()
                .unwrap_or_else(|_| panic!("{id} is not 32 bytes"));
            FeedKey {
                id: (*id).to_owned(),
                public,
            }
        })
        .collect()
}
