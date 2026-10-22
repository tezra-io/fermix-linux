//! The public keys compiled into the app: the current and the standby
//! production keys, and never the fixtures' test key (update-feed-spec, "Keys").

use ed25519_dalek::VerifyingKey;
use fermix_client::update::keys::{production_keys, PRODUCTION};

#[test]
fn the_app_trusts_exactly_the_two_production_keys() {
    assert_eq!(
        PRODUCTION,
        [
            (
                "linux-feed-1",
                "E51Uv2uksKNCEBeij/hJZ7dRgiUNFsz5pmzyX8iXwfY="
            ),
            (
                "linux-feed-2",
                "WLIAl6ZJLQcRV5RV87cIJ3L/ALzLZrJI0jEemRMJG2A="
            ),
        ]
    );
    let keys = production_keys();
    let ids: Vec<&str> = keys.iter().map(|k| k.id.as_str()).collect();
    assert_eq!(ids, ["linux-feed-1", "linux-feed-2"]);
    for key in &keys {
        assert!(
            VerifyingKey::from_bytes(&key.public).is_ok(),
            "{} is an ed25519 point",
            key.id
        );
    }
    assert_ne!(keys[0].public, keys[1].public);
}

#[test]
fn the_test_key_is_never_trusted() {
    let keys = production_keys();
    assert!(keys.iter().all(|k| k.id != "linux-feed-test"));
    assert!(keys.iter().all(|k| !k.id.contains("test")));
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/update-feed/linux-feed-test.pub");
    if let Ok(text) = std::fs::read_to_string(&fixture) {
        assert!(
            PRODUCTION.iter().all(|(_, public)| *public != text.trim()),
            "the fixture's public key is a production key"
        );
    }
}
