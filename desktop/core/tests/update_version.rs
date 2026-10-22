//! Versions in the update feed and the installed package's version: plain X.Y.Z,
//! ordered by number, and a development package's X.Y.Z+dev… (update-feed-spec,
//! "Payload" and rule 1).

use fermix_client::update::version::{Installed, Version};

fn v(text: &str) -> Version {
    Version::parse(text).unwrap_or_else(|| panic!("{text} is a version"))
}

#[test]
fn a_version_is_three_plain_numbers() {
    let parsed = v("0.16.2");
    assert_eq!(parsed.to_string(), "0.16.2");
    assert_eq!(v("10.0.0").to_string(), "10.0.0");
    for refused in [
        "",
        "1",
        "1.2",
        "1.2.3.4",
        "01.2.3",
        "1.02.3",
        "1.2.03",
        "1.2.3-rc1",
        "1.2.3+dev.1",
        "v1.2.3",
        " 1.2.3",
        "1.2.3 ",
        "1..3",
        "1.2.",
        "-1.2.3",
        "1.2.x",
        "١.٢.٣",
        "99999999999999999999.0.0",
    ] {
        assert_eq!(Version::parse(refused), None, "{refused:?} is refused");
    }
}

#[test]
fn versions_order_by_number_not_by_text() {
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("1.0.0") > v("0.99.99"));
    assert!(v("0.3.10") > v("0.3.9"));
    assert_eq!(v("0.3.0"), v("0.3.0"));
    let mut sorted = [v("0.3.0"), v("0.10.1"), v("0.3.2"), v("0.10.0")];
    sorted.sort();
    let texts: Vec<String> = sorted.iter().map(Version::to_string).collect();
    assert_eq!(texts, ["0.3.0", "0.3.2", "0.10.0", "0.10.1"]);
}

#[test]
fn the_installed_package_is_a_release_or_a_development_build() {
    assert_eq!(
        Installed::parse("0.3.0"),
        Some(Installed::Release(v("0.3.0")))
    );
    assert_eq!(
        Installed::parse("0.3.0+dev.20261021T120000Z.2d9e931"),
        Some(Installed::Development(v("0.3.0")))
    );
    assert_eq!(
        Installed::parse("0.3.0+dev.20261021T120000Z.2d9e931.dirty"),
        Some(Installed::Development(v("0.3.0")))
    );
    for unknown in ["", "0.3", "0.3.0-1", "0.3.0+build.1", "0.3.0+", "dev"] {
        assert_eq!(Installed::parse(unknown), None, "{unknown:?} is not ours");
    }
}

#[test]
fn a_version_reads_back_from_its_text() {
    let json = serde_json::to_string(&v("0.4.1")).unwrap();
    assert_eq!(json, "\"0.4.1\"");
    assert_eq!(serde_json::from_str::<Version>(&json).unwrap(), v("0.4.1"));
    assert!(serde_json::from_str::<Version>("\"0.4\"").is_err());
}
