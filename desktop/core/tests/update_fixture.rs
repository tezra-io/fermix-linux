//! The shared update feed fixture (tests/fixtures/update-feed, written by the
//! release's update_feed.py and signed by the test key `linux-feed-test`): the
//! app reads from it exactly what the writer says it wrote, refuses it under
//! the production keys, and applies the rules to it.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::SigningKey;
use fermix_client::update::feed::{
    parse_utc, read_feed, Arch, Engine, Feed, FeedError, FeedKey, Format, Package, Packages, Pair,
    Release,
};
use fermix_client::update::keys::{production_keys, read_keys};
use fermix_client::update::policy::{decide, Outcome};
use fermix_client::update::version::{Installed, Version};
use serde_json::Value;
use sha2::{Digest, Sha256};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/update-feed");

fn read(name: &str) -> Vec<u8> {
    std::fs::read(format!("{FIXTURE}/{name}")).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// The test key, from the fixture's own keys file.
fn test_keys() -> Vec<FeedKey> {
    let text = String::from_utf8(read("test-keys.json")).unwrap();
    let mut keys = Vec::new();
    let file: Value = serde_json::from_str(&text).unwrap();
    for entry in file["keys"].as_array().unwrap() {
        let public = STANDARD
            .decode(entry["public_key"].as_str().unwrap())
            .unwrap();
        keys.push(FeedKey {
            id: entry["key_id"].as_str().unwrap().to_owned(),
            public: public.try_into().unwrap(),
        });
    }
    assert!(read_keys(&text).is_err(), "a test keys file has no standby");
    keys
}

fn feed() -> Feed {
    read_feed(&read("appcast.json"), &test_keys()).expect("the fixture verifies")
}

fn text(value: &Value) -> String {
    value.as_str().unwrap().to_owned()
}

fn version(value: &Value) -> Version {
    Version::parse(value.as_str().unwrap()).unwrap()
}

fn time(value: &Value) -> u64 {
    parse_utc(value.as_str().unwrap()).unwrap()
}

fn package(value: &Value) -> Package {
    Package {
        file: text(&value["file"]),
        url: text(&value["url"]),
        size: value["size"].as_u64().unwrap(),
        sha256: text(&value["sha256"]),
    }
}

fn pair(value: &Value) -> Pair {
    Pair {
        deb: package(&value["deb"]),
        rpm: package(&value["rpm"]),
    }
}

/// A release as `update_feed.py verify --decoded` printed it.
fn expected_release(value: &Value) -> Release {
    let minimum = &value["minimum_installed_version"];
    Release {
        version: version(&value["version"]),
        published_at: time(&value["published_at"]),
        critical: value["critical"].as_bool().unwrap(),
        minimum_installed_version: (!minimum.is_null()).then(|| version(minimum)),
        engine: Engine {
            version: text(&value["engine"]["version"]),
            tag: text(&value["engine"]["tag"]),
            source_commit: text(&value["engine"]["source_commit"]),
        },
        notes: value["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(text)
            .collect(),
        packages: Packages {
            amd64: pair(&value["packages"]["amd64"]),
            arm64: pair(&value["packages"]["arm64"]),
        },
    }
}

#[test]
fn the_app_reads_what_the_writer_wrote() {
    let expected: Value = serde_json::from_slice(&read("expected-decode.json")).unwrap();
    assert_eq!(expected["key_id"], "linux-feed-test");
    let payload = &expected["payload"];
    let releases = payload["releases"].as_array().unwrap();
    let wanted = Feed {
        generated_at: time(&payload["generated_at"]),
        releases: releases.iter().map(expected_release).collect(),
    };
    let read = feed();
    assert_eq!(read, wanted);
    assert_eq!(
        read.generated_at,
        parse_utc("2026-11-20T18:05:00Z").unwrap()
    );
    let versions: Vec<String> = read
        .releases
        .iter()
        .map(|r| r.version.to_string())
        .collect();
    assert_eq!(versions, ["0.5.0", "0.4.0", "0.3.0"]);
    assert!(
        read.releases[0].notes[2].contains("café"),
        "non-ASCII notes survive"
    );
}

#[test]
fn the_test_key_signs_nothing_the_app_trusts() {
    assert_eq!(
        read_feed(&read("appcast.json"), &production_keys()),
        Err(FeedError::UnknownKey("linux-feed-test".into()))
    );
}

#[test]
fn the_fixture_s_key_is_the_seed_it_names() {
    let seed_hex = String::from_utf8(read("test-key-linux-feed-test.seed")).unwrap();
    let seed: [u8; 32] = Sha256::digest(b"linux-feed-test").into();
    let hex: String = seed.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        seed_hex.trim(),
        hex,
        "the seed is SHA-256 of the key's name"
    );
    let public = SigningKey::from_bytes(&seed).verifying_key().to_bytes();
    assert_eq!(test_keys()[0].public, public);
}

#[test]
fn a_changed_byte_of_the_signed_payload_is_refused() {
    let mut envelope: Value = serde_json::from_slice(&read("appcast.json")).unwrap();
    let mut payload = STANDARD
        .decode(envelope["payload"].as_str().unwrap())
        .unwrap();
    let at = payload.windows(5).position(|w| w == b"0.5.0").unwrap();
    payload[at + 4] = b'1';
    envelope["payload"] = Value::String(STANDARD.encode(&payload));
    let bytes = serde_json::to_vec(&envelope).unwrap();
    assert_eq!(read_feed(&bytes, &test_keys()), Err(FeedError::Signature));
}

#[test]
fn the_rules_on_the_fixture() {
    let feed = feed();
    let installed = |v: &str| Installed::Release(Version::parse(v).unwrap());
    let Outcome::Available(from_old) = decide(&feed, installed("0.3.0"), None) else {
        panic!("0.3.0 is offered an update");
    };
    assert_eq!(from_old.release.version.to_string(), "0.4.0");
    assert!(from_old.critical, "0.4.0 is critical");
    assert_eq!(
        from_old.held_back.map(|v| v.to_string()),
        Some("0.5.0".into())
    );
    let Outcome::Available(next) = decide(&feed, installed("0.4.0"), None) else {
        panic!("0.4.0 is offered an update");
    };
    assert_eq!(next.release.version.to_string(), "0.5.0");
    assert!(!next.critical);
    assert_eq!(decide(&feed, installed("0.5.0"), None), Outcome::UpToDate);
    let deb = next.release.packages.pick(Arch::Amd64, Format::Deb);
    assert_eq!(deb.file, "fermix-desktop_0.5.0_amd64.deb");
    let body = format!("test package {}\n", deb.file);
    assert_eq!(deb.size, body.len() as u64);
    let digest: String = Sha256::digest(body.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(deb.sha256, digest, "the package bytes SOURCE.md names");
}
