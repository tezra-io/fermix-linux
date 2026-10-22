//! The rules the app applies to a verified feed (update-feed-spec, "Rules the
//! app applies"), one test or more per rule. Rule 7, the download's size and
//! digest, is update_fetch.rs's.

use fermix_client::update::feed::{Arch, Engine, Feed, Format, Package, Packages, Pair, Release};
use fermix_client::update::policy::{decide, fresh, origin, Offer, Outcome};
use fermix_client::update::version::{Installed, Version};

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn package(file: &str) -> Package {
    Package {
        file: file.into(),
        url: format!("https://example.com/{file}"),
        size: 10,
        sha256: "0".repeat(64),
    }
}

fn release(version: &str, critical: bool, minimum: Option<&str>) -> Release {
    let pair = |arch: &str| Pair {
        deb: package(&format!("{version}-{arch}.deb")),
        rpm: package(&format!("{version}-{arch}.rpm")),
    };
    Release {
        version: v(version),
        published_at: 0,
        critical,
        minimum_installed_version: minimum.map(v),
        engine: Engine {
            version: "0.17.0".into(),
            tag: "v0.17.0".into(),
            source_commit: "0".repeat(40),
        },
        notes: vec!["A note.".into()],
        packages: Packages {
            amd64: pair("amd64"),
            arm64: pair("arm64"),
        },
    }
}

fn feed(releases: Vec<Release>) -> Feed {
    Feed {
        generated_at: 100,
        releases,
    }
}

fn installed(version: &str) -> Installed {
    Installed::Release(v(version))
}

fn offer(outcome: Outcome) -> Offer {
    match outcome {
        Outcome::Available(offer) => *offer,
        other => panic!("expected an offer, got {other:?}"),
    }
}

#[test]
fn rule_1_a_development_build_never_offers_an_update() {
    let releases = feed(vec![release("9.0.0", true, None)]);
    let development = Installed::Development(v("0.3.0"));
    assert_eq!(decide(&releases, development, None), Outcome::Development);
}

#[test]
fn rule_2_an_older_feed_is_refused_and_an_equal_one_accepted() {
    assert!(fresh(100, None), "the first feed ever seen");
    assert!(fresh(100, Some(99)));
    assert!(fresh(100, Some(100)), "an equal generated_at is accepted");
    assert!(!fresh(99, Some(100)), "a replayed or frozen feed");
}

#[test]
fn rule_3_nothing_newer_is_up_to_date() {
    let releases = feed(vec![
        release("0.4.0", false, None),
        release("0.3.0", false, None),
    ]);
    assert_eq!(
        decide(&releases, installed("0.4.0"), None),
        Outcome::UpToDate
    );
    assert_eq!(
        decide(&releases, installed("0.5.0"), None),
        Outcome::UpToDate,
        "an installed version newer than the feed's newest"
    );
}

#[test]
fn rule_3_the_newest_installable_release_is_offered() {
    let releases = feed(vec![
        release("0.6.0", false, None),
        release("0.5.0", false, None),
        release("0.4.0", false, None),
    ]);
    let offered = offer(decide(&releases, installed("0.4.0"), None));
    assert_eq!(offered.release.version, v("0.6.0"));
    assert_eq!(offered.held_back, None);
    assert!(!offered.critical);
    assert!(!offered.skipped);
}

#[test]
fn rule_3_a_release_needing_a_stepping_stone_waits_for_it() {
    let releases = feed(vec![
        release("0.6.0", false, Some("0.5.0")),
        release("0.5.0", false, Some("0.3.0")),
        release("0.4.0", false, None),
    ]);
    let offered = offer(decide(&releases, installed("0.3.0"), None));
    assert_eq!(
        offered.release.version,
        v("0.5.0"),
        "a minimum equal to the installed version allows it"
    );
    assert_eq!(offered.held_back, Some(v("0.6.0")));
    let from_older = offer(decide(&releases, installed("0.2.0"), None));
    assert_eq!(from_older.release.version, v("0.4.0"));
    assert_eq!(
        from_older.held_back,
        Some(v("0.6.0")),
        "the newest release is the one named"
    );
}

#[test]
fn rule_3_no_installable_release_strands_the_installed_version() {
    let releases = feed(vec![release("0.6.0", false, Some("0.5.0"))]);
    assert_eq!(
        decide(&releases, installed("0.4.0"), None),
        Outcome::Stranded { latest: v("0.6.0") }
    );
}

#[test]
fn rule_4_a_critical_release_on_the_way_makes_the_offer_critical() {
    let releases = feed(vec![
        release("0.7.0", true, Some("0.6.0")),
        release("0.6.0", false, None),
        release("0.5.0", true, None),
        release("0.4.0", false, None),
        release("0.3.0", true, None),
    ]);
    let crossed = offer(decide(&releases, installed("0.4.0"), None));
    assert_eq!(crossed.release.version, v("0.6.0"));
    assert!(
        crossed.critical,
        "0.5.0, between 0.4.0 and 0.6.0, is critical"
    );
    let past = offer(decide(&releases, installed("0.5.0"), None));
    assert_eq!(past.release.version, v("0.6.0"));
    assert!(
        !past.critical,
        "0.5.0 is installed (exclusive), and the held-back 0.7.0 is past the offer"
    );
    let itself = offer(decide(&releases, installed("0.6.0"), None));
    assert_eq!(itself.release.version, v("0.7.0"));
    assert!(itself.critical, "the offer itself counts");
}

#[test]
fn rule_5_a_skipped_version_is_remembered_for_that_version_only() {
    let releases = feed(vec![
        release("0.5.0", false, None),
        release("0.4.0", false, None),
    ]);
    let skipped = offer(decide(&releases, installed("0.3.0"), Some(v("0.5.0"))));
    assert!(skipped.skipped);
    let other = offer(decide(&releases, installed("0.3.0"), Some(v("0.4.0"))));
    assert!(!other.skipped, "skipping 0.4.0 does not skip 0.5.0");
    let critical = feed(vec![release("0.5.0", true, None)]);
    let never = offer(decide(&critical, installed("0.3.0"), Some(v("0.5.0"))));
    assert!(never.critical);
    assert!(!never.skipped, "a critical offer cannot be skipped");
}

#[test]
fn rule_6_the_package_follows_the_machine_and_the_package_manager() {
    let r = release("0.5.0", false, None);
    assert_eq!(
        r.packages.pick(Arch::Amd64, Format::Deb).file,
        "0.5.0-amd64.deb"
    );
    assert_eq!(
        r.packages.pick(Arch::Arm64, Format::Rpm).file,
        "0.5.0-arm64.rpm"
    );
    let deb = "fermix-desktop: /usr/bin/fermix-desktop\n";
    let rpm = "fermix-desktop\n";
    assert_eq!(origin(Some(deb), None), Some(Format::Deb));
    assert_eq!(origin(None, Some(rpm)), Some(Format::Rpm));
    assert_eq!(
        origin(
            Some("dpkg-query: no path found matching pattern\n"),
            Some("file /usr/bin/fermix-desktop is not owned by any package\n")
        ),
        None
    );
    assert_eq!(
        origin(Some("fermix: /usr/bin/fermix-desktop\n"), None),
        None,
        "another package owns it"
    );
    assert_eq!(origin(None, Some("fermix-desktop-extra\n")), None);
    assert_eq!(
        origin(Some(deb), Some(rpm)),
        None,
        "both databases claiming it is not an answer"
    );
    assert_eq!(origin(None, None), None);
}
