//! The update feed, format 1 (update-feed-spec): a strict envelope whose
//! ed25519 signature is checked against the compiled-in keys before anything
//! else is read, then a lenient payload held to every limit the spec sets.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use fermix_client::update::feed::{
    parse_utc, read_feed, utc_date, Arch, FeedError, FeedKey, Format, FEED_CAP, FEED_URL,
};
use fermix_client::update::version::Version;
use serde_json::{json, Value};

/// This file's own key pair, for these tests only. The shared fixture's
/// `linux-feed-test` key is tested in update_fixture.rs.
const SEED: [u8; 32] = [7; 32];
const OTHER_SEED: [u8; 32] = [9; 32];
const KEY_ID: &str = "linux-feed-unit";
const STANDBY_ID: &str = "linux-feed-unit-standby";

fn public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

fn keys() -> Vec<FeedKey> {
    vec![
        FeedKey {
            id: KEY_ID.into(),
            public: public(&SEED),
        },
        FeedKey {
            id: STANDBY_ID.into(),
            public: public(&OTHER_SEED),
        },
    ]
}

fn package(file: &str, size: u64) -> Value {
    json!({
        "file": file,
        "url": format!("https://github.com/tezra-io/fermix-linux/releases/download/v1/{file}"),
        "size": size,
        "sha256": "a".repeat(64),
    })
}

fn release(version: &str) -> Value {
    json!({
        "version": version,
        "published_at": "2026-10-30T18:00:00Z",
        "critical": false,
        "minimum_installed_version": null,
        "engine": {"version": "0.17.0", "tag": "v0.17.0", "source_commit": "0123456789abcdef0123456789abcdef01234567"},
        "notes": ["Fermix checks for updates and installs them from the tray.", "Replies show `code` as code."],
        "packages": {
            "amd64": {
                "deb": package(&format!("fermix-desktop_{version}_amd64.deb"), 52_428_800),
                "rpm": package(&format!("fermix-desktop-{version}-1.x86_64.rpm"), 52_428_801),
            },
            "arm64": {
                "deb": package(&format!("fermix-desktop_{version}_arm64.deb"), 52_428_802),
                "rpm": package(&format!("fermix-desktop-{version}-1.aarch64.rpm"), 52_428_803),
            }
        }
    })
}

fn payload(releases: Vec<Value>) -> Value {
    json!({
        "format": 1,
        "product": "fermix-desktop",
        "generated_at": "2026-10-22T03:00:00Z",
        "releases": releases,
    })
}

fn envelope_over(bytes: &[u8], seed: &[u8; 32], key_id: &str) -> Value {
    let signature = SigningKey::from_bytes(seed).sign(bytes);
    json!({
        "format": 1,
        "key_id": key_id,
        "payload": STANDARD.encode(bytes),
        "signature": STANDARD.encode(signature.to_bytes()),
    })
}

fn signed(payload: &Value) -> Vec<u8> {
    let bytes = serde_json::to_vec(payload).unwrap();
    serde_json::to_vec(&envelope_over(&bytes, &SEED, KEY_ID)).unwrap()
}

fn read(bytes: &[u8]) -> Result<fermix_client::update::feed::Feed, FeedError> {
    read_feed(bytes, &keys())
}

/// The payload refused for a reason naming `why`.
fn refused_payload(payload: &Value, why: &str) {
    match read(&signed(payload)) {
        Err(FeedError::Payload(reason)) => assert!(
            reason.contains(why),
            "refused for {reason:?}, not for {why:?}"
        ),
        other => panic!("expected a refused payload ({why}), got {other:?}"),
    }
}

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

#[test]
fn the_feed_lives_on_fermix_ai() {
    assert_eq!(FEED_URL, "https://fermix.ai/linux/appcast.json");
    assert_eq!(FEED_CAP, 1024 * 1024);
}

#[test]
fn a_signed_feed_decodes_to_its_releases() {
    let feed = read(&signed(&payload(vec![release("0.4.0"), release("0.3.1")]))).unwrap();
    assert_eq!(feed.generated_at, 1_792_638_000);
    assert_eq!(feed.releases.len(), 2);
    let newest = &feed.releases[0];
    assert_eq!(newest.version, v("0.4.0"));
    assert_eq!(newest.published_at, 1_793_383_200);
    assert!(!newest.critical);
    assert_eq!(newest.minimum_installed_version, None);
    assert_eq!(newest.engine.version, "0.17.0");
    assert_eq!(newest.engine.tag, "v0.17.0");
    assert_eq!(newest.notes.len(), 2);
    let deb = newest.packages.pick(Arch::Amd64, Format::Deb);
    assert_eq!(deb.file, "fermix-desktop_0.4.0_amd64.deb");
    assert_eq!(deb.size, 52_428_800);
    assert_eq!(
        newest.packages.pick(Arch::Amd64, Format::Rpm).file,
        "fermix-desktop-0.4.0-1.x86_64.rpm"
    );
    assert_eq!(
        newest.packages.pick(Arch::Arm64, Format::Deb).file,
        "fermix-desktop_0.4.0_arm64.deb"
    );
    assert_eq!(
        newest.packages.pick(Arch::Arm64, Format::Rpm).size,
        52_428_803
    );
}

#[test]
fn the_standby_key_also_signs() {
    let bytes = serde_json::to_vec(&payload(vec![release("0.4.0")])).unwrap();
    let envelope = envelope_over(&bytes, &OTHER_SEED, STANDBY_ID);
    assert!(read(&serde_json::to_vec(&envelope).unwrap()).is_ok());
}

#[test]
fn a_feed_over_one_mebibyte_is_refused_unread() {
    let mut bytes = signed(&payload(vec![release("0.4.0")]));
    bytes.resize(FEED_CAP + 1, b' ');
    assert_eq!(read(&bytes), Err(FeedError::TooLarge));
    bytes.truncate(FEED_CAP);
    assert!(read(&bytes).is_ok(), "trailing spaces are still JSON");
}

#[test]
fn the_envelope_is_strict() {
    let good: Value = serde_json::from_slice(&signed(&payload(vec![release("0.4.0")]))).unwrap();
    let mut extra = good.clone();
    extra["comment"] = json!("hi");
    let mut format = good.clone();
    format["format"] = json!(2);
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("signature");
    for (what, envelope) in [
        ("an unknown field", extra),
        ("format 2", format),
        ("no signature", missing),
    ] {
        let answer = read(&serde_json::to_vec(&envelope).unwrap());
        assert!(
            matches!(answer, Err(FeedError::Envelope(_))),
            "{what}: {answer:?}"
        );
    }
    assert!(matches!(read(b"not json"), Err(FeedError::Envelope(_))));
    assert!(matches!(read(b"[1]"), Err(FeedError::Envelope(_))));
}

#[test]
fn the_key_id_must_be_a_known_and_well_formed_name() {
    let bytes = serde_json::to_vec(&payload(vec![release("0.4.0")])).unwrap();
    for bad in [
        "",
        "Linux-feed-1",
        "linux_feed",
        "linux feed",
        &"a".repeat(65),
    ] {
        let envelope = envelope_over(&bytes, &SEED, bad);
        let answer = read(&serde_json::to_vec(&envelope).unwrap());
        assert!(
            matches!(answer, Err(FeedError::Envelope(_))),
            "{bad:?}: {answer:?}"
        );
    }
    let longest = "a".repeat(64);
    let envelope = envelope_over(&bytes, &SEED, &longest);
    assert_eq!(
        read(&serde_json::to_vec(&envelope).unwrap()),
        Err(FeedError::UnknownKey(longest))
    );
    let envelope = envelope_over(&bytes, &SEED, "linux-feed-1");
    assert_eq!(
        read(&serde_json::to_vec(&envelope).unwrap()),
        Err(FeedError::UnknownKey("linux-feed-1".into())),
        "a production key is not in this test's set"
    );
}

#[test]
fn payload_and_signature_must_be_standard_padded_base64() {
    let good: Value = serde_json::from_slice(&signed(&payload(vec![release("0.4.0")]))).unwrap();
    let unpadded = good["payload"]
        .as_str()
        .unwrap()
        .trim_end_matches('=')
        .to_owned();
    let mut cases = Vec::new();
    if unpadded.len() != good["payload"].as_str().unwrap().len() {
        let mut envelope = good.clone();
        envelope["payload"] = json!(unpadded);
        cases.push(("unpadded payload", envelope));
    }
    let mut url_safe = good.clone();
    url_safe["signature"] = json!(
        good["signature"]
            .as_str()
            .unwrap()
            .replace('+', "-")
            .replace('/', "_")
            + "-"
    );
    cases.push(("url-safe signature", url_safe));
    let mut short = good.clone();
    short["signature"] = json!(STANDARD.encode([0u8; 63]));
    cases.push(("63-byte signature", short));
    let mut junk = good.clone();
    junk["payload"] = json!("!!!");
    cases.push(("junk payload", junk));
    for (what, envelope) in cases {
        let answer = read(&serde_json::to_vec(&envelope).unwrap());
        assert!(
            matches!(answer, Err(FeedError::Envelope(_))),
            "{what}: {answer:?}"
        );
    }
}

#[test]
fn a_changed_byte_or_another_key_fails_the_signature() {
    let bytes = serde_json::to_vec(&payload(vec![release("0.4.0")])).unwrap();
    let mut envelope = envelope_over(&bytes, &SEED, KEY_ID);
    let mut tampered = bytes.clone();
    let at = tampered.iter().position(|b| *b == b'4').unwrap();
    tampered[at] = b'9';
    envelope["payload"] = json!(STANDARD.encode(&tampered));
    assert_eq!(
        read(&serde_json::to_vec(&envelope).unwrap()),
        Err(FeedError::Signature)
    );
    let wrong_key = envelope_over(&bytes, &OTHER_SEED, KEY_ID);
    assert_eq!(
        read(&serde_json::to_vec(&wrong_key).unwrap()),
        Err(FeedError::Signature)
    );
}

#[test]
fn only_a_verified_payload_is_read() {
    let garbage = b"{ this is not json";
    let unsigned = json!({
        "format": 1,
        "key_id": KEY_ID,
        "payload": STANDARD.encode(garbage),
        "signature": STANDARD.encode([0u8; 64]),
    });
    assert_eq!(
        read(&serde_json::to_vec(&unsigned).unwrap()),
        Err(FeedError::Signature)
    );
    let signed_garbage = envelope_over(garbage, &SEED, KEY_ID);
    assert!(matches!(
        read(&serde_json::to_vec(&signed_garbage).unwrap()),
        Err(FeedError::Payload(_))
    ));
}

#[test]
fn the_payload_names_this_product_and_format_one() {
    let mut product = payload(vec![release("0.4.0")]);
    product["product"] = json!("fermix");
    refused_payload(&product, "product");
    let mut format = payload(vec![release("0.4.0")]);
    format["format"] = json!(2);
    refused_payload(&format, "format");
    let mut when = payload(vec![release("0.4.0")]);
    when["generated_at"] = json!("2026-10-22 03:00:00");
    refused_payload(&when, "generated_at");
}

#[test]
fn releases_are_one_to_two_hundred_newest_first() {
    refused_payload(&payload(vec![]), "releases");
    let many: Vec<Value> = (0..201)
        .rev()
        .map(|n| release(&format!("0.{n}.0")))
        .collect();
    refused_payload(&payload(many), "releases");
    let most: Vec<Value> = (0..200)
        .rev()
        .map(|n| release(&format!("0.{n}.0")))
        .collect();
    assert_eq!(read(&signed(&payload(most))).unwrap().releases.len(), 200);
    refused_payload(
        &payload(vec![release("0.3.0"), release("0.4.0")]),
        "newest first",
    );
    refused_payload(
        &payload(vec![release("0.4.0"), release("0.4.0")]),
        "newest first",
    );
}

#[test]
fn versions_are_plain_x_y_z() {
    refused_payload(&payload(vec![release("0.04.0")]), "version");
    let mut minimum = release("0.4.0");
    minimum["minimum_installed_version"] = json!("0.3");
    refused_payload(&payload(vec![minimum]), "minimum_installed_version");
    let mut stepping = release("0.4.0");
    stepping["minimum_installed_version"] = json!("0.3.2");
    let feed = read(&signed(&payload(vec![stepping]))).unwrap();
    assert_eq!(feed.releases[0].minimum_installed_version, Some(v("0.3.2")));
}

#[test]
fn notes_are_one_to_eight_sentences_of_at_most_three_hundred_characters() {
    let with_notes = |notes: Value| {
        let mut r = release("0.4.0");
        r["notes"] = notes;
        payload(vec![r])
    };
    refused_payload(&with_notes(json!([])), "notes");
    refused_payload(&with_notes(json!(vec!["A note."; 9])), "notes");
    refused_payload(&with_notes(json!([""])), "note");
    refused_payload(&with_notes(json!(["x".repeat(301)])), "note");
    let wide = "é".repeat(300);
    let feed = read(&signed(&with_notes(json!(vec![wide.clone(); 8])))).unwrap();
    assert_eq!(
        feed.releases[0].notes,
        vec![wide; 8],
        "characters, not bytes"
    );
}

#[test]
fn every_release_carries_all_four_packages() {
    let mut missing = release("0.4.0");
    missing["packages"]["arm64"]
        .as_object_mut()
        .unwrap()
        .remove("rpm");
    refused_payload(&payload(vec![missing]), "rpm");
    let mut no_arch = release("0.4.0");
    no_arch["packages"].as_object_mut().unwrap().remove("arm64");
    refused_payload(&payload(vec![no_arch]), "arm64");
}

#[test]
fn a_package_is_named_as_the_build_names_it() {
    let with = |field: &str, value: Value| {
        let mut r = release("0.4.0");
        r["packages"]["amd64"]["deb"][field] = value;
        payload(vec![r])
    };
    refused_payload(
        &with("file", json!("fermix-desktop_0.4.1_amd64.deb")),
        "file",
    );
    refused_payload(
        &with("file", json!("fermix-desktop_0.4.0_arm64.deb")),
        "file",
    );
    let mut rpm = release("0.4.0");
    rpm["packages"]["arm64"]["rpm"]["file"] = json!("fermix-desktop-0.4.0-1.arm64.rpm");
    refused_payload(&payload(vec![rpm]), "file");
}

#[test]
fn a_package_url_is_https_and_ends_in_its_file() {
    let with_url = |url: &str| {
        let mut r = release("0.4.0");
        r["packages"]["amd64"]["deb"]["url"] = json!(url);
        payload(vec![r])
    };
    let file = "fermix-desktop_0.4.0_amd64.deb";
    refused_payload(&with_url(&format!("http://example.com/{file}")), "url");
    refused_payload(&with_url(&format!("https://example.com/x{file}")), "url");
    refused_payload(&with_url(&format!("https://example.com/{file}?x=1")), "url");
    refused_payload(&with_url(&format!("https://exa mple.com/{file}")), "url");
    refused_payload(&with_url(&format!("https:///{file}")), "url");
    let elsewhere = with_url(&format!("https://downloads.fermix.ai/linux/0.4.0/{file}"));
    assert!(read(&signed(&elsewhere)).is_ok(), "any https origin");
}

#[test]
fn a_package_size_is_one_byte_to_512_mebibytes() {
    let with_size = |size: u64| {
        let mut r = release("0.4.0");
        r["packages"]["amd64"]["rpm"]["size"] = json!(size);
        payload(vec![r])
    };
    refused_payload(&with_size(0), "size");
    refused_payload(&with_size(536_870_913), "size");
    assert!(read(&signed(&with_size(536_870_912))).is_ok());
    assert!(read(&signed(&with_size(1))).is_ok());
}

#[test]
fn a_package_digest_is_64_lowercase_hex() {
    for bad in [
        "A".repeat(64),
        "a".repeat(63),
        "g".repeat(64),
        "a".repeat(65),
    ] {
        let mut r = release("0.4.0");
        r["packages"]["arm64"]["deb"]["sha256"] = json!(bad);
        refused_payload(&payload(vec![r]), "sha256");
    }
}

#[test]
fn the_engine_names_its_version_tag_and_commit() {
    let mut r = release("0.4.0");
    r["engine"]["source_commit"] = json!("0123");
    refused_payload(&payload(vec![r]), "source_commit");
    let mut r = release("0.4.0");
    r["engine"]["version"] = json!("");
    refused_payload(&payload(vec![r]), "engine");
}

#[test]
fn fields_the_app_does_not_know_are_read_past() {
    let mut p = payload(vec![release("0.4.0")]);
    p["mirrors"] = json!(["https://example.com"]);
    p["releases"][0]["engine"]["build_id"] = json!("release-123");
    p["releases"][0]["packages"]["riscv64"] = json!({"deb": 1});
    p["releases"][0]["packages"]["amd64"]["deb"]["signature"] = json!("x");
    p["releases"][0]["summary"] = json!("Later fields stay readable.");
    assert!(read(&signed(&p)).is_ok());
}

#[test]
fn timestamps_are_utc_to_the_second() {
    assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(parse_utc("2026-10-22T03:00:00Z"), Some(1_792_638_000));
    assert_eq!(parse_utc("2028-02-29T23:59:59Z"), Some(1_835_481_599));
    for bad in [
        "2026-10-22T03:00:00",
        "2026-10-22T03:00:00+00:00",
        "2026-10-22T03:00:00.5Z",
        "2026-13-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2026-10-32T00:00:00Z",
        "2026-10-22T24:00:00Z",
        "2026-10-22T03:60:00Z",
        "2026-10-22T03:00:60Z",
        "1969-12-31T23:59:59Z",
        "2026-1-22T03:00:00Z",
        "",
    ] {
        assert_eq!(parse_utc(bad), None, "{bad:?}");
    }
}

#[test]
fn the_machine_is_amd64_or_arm64() {
    assert_eq!(Arch::from_target("x86_64"), Some(Arch::Amd64));
    assert_eq!(Arch::from_target("aarch64"), Some(Arch::Arm64));
    assert_eq!(Arch::from_target("riscv64"), None);
    assert_eq!(Arch::Amd64.to_string(), "amd64");
    assert_eq!(Arch::Arm64.to_string(), "arm64");
}

#[test]
fn a_time_reads_back_as_its_utc_date() {
    assert_eq!(utc_date(0), (1970, 1, 1));
    assert_eq!(utc_date(1_792_638_000), (2026, 10, 22));
    for (text, date) in [
        ("2028-02-29T23:59:59Z", (2028, 2, 29)),
        ("2028-03-01T00:00:00Z", (2028, 3, 1)),
        ("2100-03-01T00:00:00Z", (2100, 3, 1)),
        ("2026-12-31T12:00:00Z", (2026, 12, 31)),
    ] {
        assert_eq!(utc_date(parse_utc(text).unwrap()), date, "{text}");
    }
}
