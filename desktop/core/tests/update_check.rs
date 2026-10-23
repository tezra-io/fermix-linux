//! One check from end to end against a feed served on 127.0.0.1, how this copy
//! of Fermix was installed, what a check leaves remembered, and the steps
//! around an install: the command to fall back on and whether the engine
//! restarts.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ed25519_dalek::{Signer, SigningKey};
use fermix_client::update::check::{
    check, due, remember, CheckError, AHEAD_BY, CHECK_EVERY, STALE_AFTER,
};
use fermix_client::update::feed::{FeedError, FeedKey, Format};
use fermix_client::update::fetch::{FetchConfig, FetchError, Fetcher, Timeouts};
use fermix_client::update::identity::Alignment;
use fermix_client::update::policy::Outcome;
use fermix_client::update::state::{
    engine_needs_restart, install_command, installation, may_restart, Failure, Installation,
    Manual, Restart, Stage, Step, Updates, RELAUNCH,
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
/// 2026-10-22T03:00:00Z, when the tests' feeds are written.
const GENERATED: u64 = 1_792_638_000;
const DAY: u64 = 86_400;
/// The local clock, an hour after the feed was written.
const NOW: u64 = GENERATED + 3_600;
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
        NOW,
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
    let answer = check(&fetcher(), &url, &keys(), &package("0.3.0"), &seen, NOW);
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
        NOW,
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
        NOW,
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
        NOW,
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
        NOW,
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
    let found = check(
        &fetcher(),
        &url,
        &keys(),
        &package("0.3.0"),
        &remembered,
        NOW,
    );
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
fn a_confirmed_install_goes_on_to_its_restart_and_an_asked_one_waits_its_turn() {
    let mut updates = Updates::new(Remembered::default());
    updates.installation = Some(package("0.4.0"));
    // Where an install stands when the package database confirms it.
    updates.step = Some(Step::Installing {
        percent: Some(100),
        waiting: None,
    });
    assert!(updates.busy(), "nothing else may start");
    assert!(may_restart(&updates, Restart::AfterInstall));
    assert!(
        !may_restart(&updates, Restart::Asked),
        "an asked restart never cuts into an install"
    );
    updates.step = Some(Step::Asking);
    assert!(
        may_restart(&updates, Restart::AfterInstall),
        "PackageKit sent no progress"
    );
    for running in [
        Step::Downloading { got: 1, size: 2 },
        Step::Verifying,
        Step::Restarting,
    ] {
        updates.step = Some(running);
        assert!(!may_restart(&updates, Restart::AfterInstall));
        assert!(!may_restart(&updates, Restart::Asked));
    }
    let failed = |stage| {
        Some(Step::Failed(Failure {
            stage,
            sentence: "It stopped.".into(),
            manual: Manual::Reopen,
        }))
    };
    updates.step = failed(Stage::Restart);
    assert!(
        may_restart(&updates, Restart::Asked),
        "Try Again after a failed restart"
    );
    assert!(!may_restart(&updates, Restart::AfterInstall));
    updates.step = failed(Stage::Install);
    assert!(!may_restart(&updates, Restart::Asked));
    updates.step = None;
    assert!(
        !may_restart(&updates, Restart::Asked),
        "nothing to restart into"
    );
    updates.restart_pending = true;
    assert!(
        may_restart(&updates, Restart::Asked),
        "Restart to finish updating"
    );
    updates.installation = Some(Installation::Development {
        version: "0.4.0+dev.1".into(),
    });
    assert!(!may_restart(&updates, Restart::Asked));
}

#[test]
fn the_window_comes_back_as_the_installed_binary_taking_over_its_name() {
    assert_eq!(
        RELAUNCH,
        ["/usr/bin/fermix-desktop", "--gapplication-replace"]
    );
}

#[test]
fn an_automatic_check_is_due_a_day_after_the_last() {
    let at = |seconds: u64| Remembered {
        last_check: Some(LastCheck {
            at: seconds,
            result: Checked::UpToDate,
        }),
        ..Remembered::default()
    };
    assert_eq!(CHECK_EVERY, 24 * 60 * 60);
    assert!(due(&Remembered::default(), 1_000), "never checked");
    assert!(!due(&at(1_000), 1_000 + CHECK_EVERY - 1));
    assert!(due(&at(1_000), 1_000 + CHECK_EVERY));
    assert!(
        due(&at(5_000), 1_000),
        "a last check in the future is a clock that was wrong"
    );
    let off = Remembered {
        automatic: false,
        ..Remembered::default()
    };
    assert!(!due(&off, 1_000), "the switch is off");
}

#[test]
fn a_feed_more_than_180_days_old_or_a_day_ahead_is_out_of_date() {
    assert_eq!((STALE_AFTER, AHEAD_BY), (180 * DAY, DAY));
    let feed = signed_feed("2026-10-22T03:00:00Z", &["0.4.0", "0.3.0"]);
    let at = |now: u64| {
        let url = serve(200, feed.clone());
        check(
            &fetcher(),
            &url,
            &keys(),
            &package("0.3.0"),
            &Remembered::default(),
            now,
        )
    };
    let out_of_date = |now: u64| CheckError::OutOfDate {
        generated_at: GENERATED,
        now,
    };
    assert!(at(GENERATED + 179 * DAY).is_ok(), "179 days behind");
    assert!(
        at(GENERATED + 180 * DAY).is_ok(),
        "180 days is not more than 180"
    );
    let behind = GENERATED + 181 * DAY;
    assert_eq!(
        at(behind).unwrap_err(),
        out_of_date(behind),
        "181 days behind"
    );
    assert!(at(GENERATED - DAY).is_ok(), "1 day ahead is clock skew");
    let ahead = GENERATED - 2 * DAY;
    assert_eq!(at(ahead).unwrap_err(), out_of_date(ahead), "2 days ahead");
}

#[test]
fn an_out_of_date_feed_is_remembered_as_a_failed_check() {
    let mut remembered = Remembered {
        accepted_generated_at: Some(GENERATED),
        ..Remembered::default()
    };
    let now = GENERATED + 200 * DAY;
    let frozen: Result<_, CheckError> = Err(CheckError::OutOfDate {
        generated_at: GENERATED,
        now,
    });
    remember(&mut remembered, now, &frozen);
    assert_eq!(remembered.accepted_generated_at, Some(GENERATED));
    match &remembered.last_check {
        Some(LastCheck {
            result: Checked::Failed { sentence },
            ..
        }) => assert_eq!(
            sentence,
            "The update information is out of date. It is from 22 October 2026."
        ),
        other => panic!("an out-of-date feed is a failed check, not {other:?}"),
    }
}
