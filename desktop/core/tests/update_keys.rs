//! The public keys compiled into the app: the current and the standby
//! production keys of desktop/packaging/update-keys.json, and never the
//! fixtures' test key (update-feed-spec, "Keys").

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::VerifyingKey;
use fermix_client::update::keys::{production_keys, read_keys};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update-feed");

fn decoded(base64: &str) -> [u8; 32] {
    STANDARD.decode(base64).unwrap().try_into().unwrap()
}

#[test]
fn the_app_trusts_exactly_the_two_production_keys() {
    let keys = production_keys();
    let ids: Vec<&str> = keys.iter().map(|k| k.id.as_str()).collect();
    assert_eq!(
        ids,
        ["linux-feed-1", "linux-feed-2"],
        "current, then standby"
    );
    assert_eq!(
        keys[0].public,
        decoded("E51Uv2uksKNCEBeij/hJZ7dRgiUNFsz5pmzyX8iXwfY=")
    );
    assert_eq!(
        keys[1].public,
        decoded("WLIAl6ZJLQcRV5RV87cIJ3L/ALzLZrJI0jEemRMJG2A=")
    );
    for key in &keys {
        assert!(
            VerifyingKey::from_bytes(&key.public).is_ok(),
            "{} is an ed25519 point",
            key.id
        );
    }
}

#[test]
fn the_test_key_is_never_trusted() {
    let text = std::fs::read_to_string(format!("{FIXTURE}/test-keys.json")).unwrap();
    let test: serde_json::Value = serde_json::from_str(&text).unwrap();
    let test_key = &test["keys"][0];
    assert_eq!(test_key["key_id"], "linux-feed-test");
    let public = decoded(test_key["public_key"].as_str().unwrap());
    let keys = production_keys();
    assert!(keys.iter().all(|k| k.id != "linux-feed-test"));
    assert!(keys.iter().all(|k| !k.id.contains("test")));
    assert!(
        keys.iter().all(|k| k.public != public),
        "the fixture's public key is a production key"
    );
}

#[test]
fn a_keys_file_is_held_to_its_form() {
    let entry = |id: &str, role: &str| {
        format!(
            r#"{{"key_id":"{id}","role":"{role}","public_key":"E51Uv2uksKNCEBeij/hJZ7dRgiUNFsz5pmzyX8iXwfY="}}"#
        )
    };
    let file = |format: u32, keys: &[String]| {
        format!(r#"{{"format":{format},"keys":[{}]}}"#, keys.join(","))
    };
    let good = [entry("a", "current"), entry("b", "standby")];
    assert_eq!(read_keys(&file(1, &good)).unwrap().len(), 2);
    assert!(read_keys(&file(2, &good)).is_err(), "another format");
    assert!(read_keys(&file(1, &good[..1])).is_err(), "no standby");
    let swapped = [entry("a", "standby"), entry("b", "current")];
    assert!(read_keys(&file(1, &swapped)).is_err());
    let short = r#"{"format":1,"keys":[{"key_id":"a","role":"current","public_key":"AAAA"},{"key_id":"b","role":"standby","public_key":"AAAA"}]}"#;
    assert!(read_keys(short).is_err(), "a key of 3 bytes");
    let extra = file(1, &good).replace(r#""format":1"#, r#""format":1,"signer":"x""#);
    assert!(read_keys(&extra).is_err(), "an unknown field");
}
