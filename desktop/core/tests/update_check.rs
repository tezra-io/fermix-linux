//! One check from end to end against a feed served on 127.0.0.1, how this copy
//! of Fermix was installed, what a check leaves remembered, and the steps
//! around an install: the command to fall back on and whether the engine
//! restarts.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use fermix_client::update::check::{check, remember, CheckError};
use fermix_client::update::feed::{FeedError, FeedKey, Format};
use fermix_client::update::fetch::{FetchConfig, FetchError, Fetcher, Timeouts};
use fermix_client::update::identity::Alignment;
use fermix_client::update::policy::Outcome;
use fermix_client::update::state::{
    engine_needs_restart, install_command, installation, Installation, RELAUNCH,
};
use fermix_client::update::store::{Checked, LastCheck, Remembered};
use fermix_client::update::version::Version;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;
use std::time::Duration;

const SEED: [u8; 32] = [11; 32];
const KEY_ID: &str = "linux-feed-unit";

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn keys() -> Vec<FeedKey> {
    vec![FeedKey {
        id: KEY_ID.into(),
        public: SigningKey::from_bytes(&SEED).verifying_key().to_bytes(),
    }]
}

fn release(version: &str) -> Value {
    let package = |file: String| json!({"file": file, "url": format!("https://example.com/{file}"), "size": 10, "sha256": "b".repeat(64)});
    json!({
        "version": version, "published_at": "2026-10-30T18:00:00Z", "critical": false,
        "minimum_installed_version": null,
        "engine": {"version": "0.17.0", "tag": "v0.17.0", "source_commit": "c".repeat(40)},
        "notes": ["A note."],
        "packages": {
            "amd64": {"deb": package(format!("fermix-desktop_{version}_amd64.deb")),
                      "rpm": package(format!("fermix-desktop-{version}-1.x86_64.rpm"))},
            "arm64": {"deb": package(format!("fermix-desktop_{version}_arm64.deb")),
                      "rpm": package(format!("fermix-desktop-{version}-1.aarch64.rpm"))}
        }
    })
}

fn signed_feed(generated_at: &str, versions: &[&str]) -> Vec<u8> {
    let payload = json!({
        "format": 1, "product": "fermix-desktop", "generated_at": generated_at,
        "releases": versions.iter().map(|v| release(v)).collect::<Vec<_>>(),
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let signature = SigningKey::from_bytes(&SEED).sign(&bytes);
    serde_json::to_vec(&json!({
        "format": 1, "key_id": KEY_ID,
        "payload": STANDARD.encode(&bytes),
        "signature": STANDARD.encode(signature.to_bytes()),
    }))
    .unwrap()
}

/// Answers every request on one port with `body`, for a few connections.
fn serve(status: u16, body: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().take(4) {
            let Ok(mut stream) = stream else { return };
            let mut head = [0u8; 4096];
            let _ = stream.read(&mut head);
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.write_all(&body);
        }
    });
    format!("http://127.0.0.1:{port}/linux/appcast.json")
}

fn package(version: &str) -> Installation {
    Installation::Package {
        version: v(version),
        format: Some(Format::Deb),
    }
}

fn fetcher() -> Fetcher {
    let ca = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/update-tls/test-ca.der"
    ))
    .unwrap();
    Fetcher::new(FetchConfig {
        roots: vec![ca],
        https_only: false,
        proxy_from_env: false,
        timeouts: Timeouts {
            connect: Duration::from_secs(5),
            feed: Duration::from_secs(5),
            per_read: Duration::from_secs(2),
            download: Duration::from_secs(5),
        },
    })
    .unwrap()
}

#[test]
fn a_check_fetches_verifies_and_decides() {
    let url = serve(
        200,
        signed_feed("2026-10-22T03:00:00Z", &["0.4.0", "0.3.0"]),
    );
    let found = check(
        &fetcher(),
        &url,
        &keys(),
        &package("0.3.0"),
        &Remembered::default(),
    )
    .unwrap();
    assert_eq!(found.generated_at, Some(1_792_638_000));
    match found.outcome {
        Outcome::Available(offer) => assert_eq!(offer.release.version, v("0.4.0")),
        other => panic!("expected 0.4.0, got {other:?}"),
    }
}

#[test]
fn a_check_against_an_older_feed_could_not_check() {
    let url = serve(200, signed_feed("2026-10-22T03:00:00Z", &["0.4.0"]));
    let seen = Remembered {
        accepted_generated_at: Some(1_792_638_001),
        ..Remembered::default()
    };
    let answer = check(&fetcher(), &url, &keys(), &package("0.3.0"), &seen);
    assert_eq!(
        answer.unwrap_err(),
        CheckError::Stale {
            accepted: 1_792_638_001,
            generated_at: 1_792_638_000
        }
    );
}

#[test]
fn a_check_says_why_it_failed() {
    let missing = serve(404, b"no".to_vec());
    let answer = check(
        &fetcher(),
        &missing,
        &keys(),
        &package("0.3.0"),
        &Remembered::default(),
    );
    assert_eq!(
        answer.unwrap_err(),
        CheckError::Fetch(FetchError::Status(404))
    );
    let forged = serve(200, signed_feed("2026-10-22T03:00:00Z", &["0.4.0"]));
    let other_keys = vec![FeedKey {
        id: KEY_ID.into(),
        public: SigningKey::from_bytes(&[12; 32]).verifying_key().to_bytes(),
    }];
    let answer = check(
        &fetcher(),
        &forged,
        &other_keys,
        &package("0.3.0"),
        &Remembered::default(),
    );
    assert_eq!(answer.unwrap_err(), CheckError::Feed(FeedError::Signature));
}

#[test]
fn a_development_build_does_not_even_fetch() {
    let nowhere = "http://127.0.0.1:9/linux/appcast.json";
    let development = Installation::Development {
        version: "0.3.0+dev.1".into(),
    };
    let found = check(
        &fetcher(),
        nowhere,
        &keys(),
        &development,
        &Remembered::default(),
    )
    .unwrap();
    assert_eq!(found.outcome, Outcome::Development);
    assert_eq!(found.generated_at, None);
    let unknown = Installation::Unknown {
        version: "0.3.0-1ubuntu1".into(),
    };
    let answer = check(
        &fetcher(),
        nowhere,
        &keys(),
        &unknown,
        &Remembered::default(),
    );
    assert_eq!(
        answer.unwrap_err(),
        CheckError::Unknown("0.3.0-1ubuntu1".into())
    );
}

#[test]
fn a_check_remembers_when_and_how_it_ended() {
    let url = serve(200, signed_feed("2026-10-22T03:00:00Z", &["0.4.0"]));
    let mut remembered = Remembered {
        accepted_generated_at: Some(1_000),
        skipped: Some(v("0.3.5")),
        ..Remembered::default()
    };
    let found = check(&fetcher(), &url, &keys(), &package("0.3.0"), &remembered);
    remember(&mut remembered, 5_000, &found);
    assert_eq!(remembered.accepted_generated_at, Some(1_792_638_000));
    assert_eq!(
        remembered.last_check,
        Some(LastCheck {
            at: 5_000,
            result: Checked::Available {
                version: v("0.4.0"),
                critical: false
            }
        })
    );
    assert_eq!(
        remembered.skipped,
        Some(v("0.3.5")),
        "a check never forgets a skip"
    );
    let failed: Result<_, CheckError> =
        Err(CheckError::Fetch(FetchError::Transport("refused".into())));
    remember(&mut remembered, 6_000, &failed);
    assert_eq!(
        remembered.accepted_generated_at,
        Some(1_792_638_000),
        "a failure moves nothing forward"
    );
    match &remembered.last_check {
        Some(LastCheck {
            at: 6_000,
            result: Checked::Failed { sentence },
        }) => {
            assert!(sentence.contains("could not reach fermix.ai"), "{sentence}")
        }
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn this_copy_is_a_release_package_a_development_build_or_unknown() {
    assert_eq!(
        installation(true, Some(Format::Deb), Some("0.3.0"), "0.3.0"),
        Installation::Package {
            version: v("0.3.0"),
            format: Some(Format::Deb)
        }
    );
    assert_eq!(
        installation(true, Some(Format::Rpm), Some("0.4.0"), "0.3.0"),
        Installation::Package {
            version: v("0.4.0"),
            format: Some(Format::Rpm)
        },
        "the package database knows what is installed, even under a window it replaced"
    );
    assert_eq!(
        installation(
            true,
            Some(Format::Deb),
            Some("0.3.0+dev.20261021T120000Z.2d9e931"),
            "0.3.0"
        ),
        Installation::Development {
            version: "0.3.0+dev.20261021T120000Z.2d9e931".into()
        }
    );
    assert_eq!(
        installation(false, Some(Format::Deb), Some("0.3.0"), "0.3.0"),
        Installation::Development {
            version: "0.3.0".into()
        },
        "a window not run from the package"
    );
    assert_eq!(
        installation(true, None, None, "0.3.0"),
        Installation::Package {
            version: v("0.3.0"),
            format: None
        },
        "files no package database owns: told about updates, never installing them"
    );
    assert_eq!(
        installation(true, Some(Format::Deb), Some("0.3.0-1ubuntu1"), "0.3.0"),
        Installation::Unknown {
            version: "0.3.0-1ubuntu1".into()
        }
    );
    assert_eq!(
        installation(true, Some(Format::Deb), None, "0.3.0"),
        Installation::Package {
            version: v("0.3.0"),
            format: Some(Format::Deb)
        },
        "an unanswered version query falls back to the window's own"
    );
}

#[test]
fn the_fallback_is_the_package_manager_s_own_command() {
    let deb = Path::new("/home/ada/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb");
    assert_eq!(
        install_command(Format::Deb, deb),
        "sudo apt install /home/ada/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb"
    );
    let rpm =
        Path::new("/home/ada/.cache/fermix-desktop/updates/fermix-desktop-0.4.0-1.x86_64.rpm");
    assert_eq!(
        install_command(Format::Rpm, rpm),
        "sudo dnf install /home/ada/.cache/fermix-desktop/updates/fermix-desktop-0.4.0-1.x86_64.rpm"
    );
    let odd =
        Path::new("/home/Ada O'Neil/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb");
    assert_eq!(
        install_command(Format::Deb, odd),
        "sudo apt install '/home/Ada O'\\''Neil/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb'"
    );
}

#[test]
fn the_engine_restarts_only_when_another_build_is_installed() {
    assert!(engine_needs_restart(&Alignment::PendingRestart));
    assert!(
        engine_needs_restart(&Alignment::Unknown),
        "an older daemon that names no build"
    );
    assert!(
        !engine_needs_restart(&Alignment::Aligned),
        "a desktop-only release"
    );
    assert!(!engine_needs_restart(&Alignment::NotRunning));
    assert!(!engine_needs_restart(&Alignment::OwnershipConflict(
        "standalone".into()
    )));
}

#[test]
fn the_window_comes_back_as_the_installed_binary_taking_over_its_name() {
    assert_eq!(
        RELAUNCH,
        ["/usr/bin/fermix-desktop", "--gapplication-replace"]
    );
}
