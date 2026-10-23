//! Whether a restart is pending (M38 §9.2): the engine installed on disk
//! against the one that answers hello, compared by build id, and the window's
//! own file replaced under it. A terminal `apt upgrade` or another account's
//! install is found the same way as the app's own.

use fermix_client::model::EngineIdentity;
use fermix_client::update::identity::{
    engine_alignment, installed_engine, restart_pending, window_replaced, Alignment,
};
use std::path::Path;

const STAGED: &str =
    include_str!("../../scripts/fixtures/engine/tree/usr/share/fermix/engine.json");

fn engine(build_id: Option<&str>, distribution: &str, architecture: &str) -> EngineIdentity {
    EngineIdentity {
        product_version: "0.16.0".into(),
        build_id: build_id.map(str::to_owned),
        pid: None,
        architecture: Some(architecture.into()),
        distribution_identity: Some(distribution.into()),
    }
}

#[test]
fn the_installed_engine_is_read_from_its_engine_json() {
    let staged = installed_engine(STAGED.as_bytes()).unwrap();
    assert_eq!(staged.build_id.as_deref(), Some("release-1"));
    assert_eq!(staged.product_version, "0.0.1");
    assert_eq!(
        staged.distribution_identity.as_deref(),
        Some("linux_package")
    );
    let released = br#"{"architecture": "x86_64", "artifact_target": "linux_x86_64",
        "binary_sha256": "9b0c", "build_id": "release-38495034881",
        "distribution_identity": "linux_package", "engine_id": "fermix-core",
        "loader_sha256": "71c3", "product_version": "0.15.0", "schema_version": 1,
        "source_commit": "b268d8e1d15688ec6015ae6ebd433da3db9392c5"}"#;
    let released = installed_engine(released).unwrap();
    assert_eq!(released.architecture.as_deref(), Some("x86_64"));
    assert_eq!(released.build_id.as_deref(), Some("release-38495034881"));
    assert!(installed_engine(b"{}").is_err(), "no product_version");
    assert!(installed_engine(b"not json").is_err());
}

#[test]
fn the_same_build_is_aligned_whatever_the_versions_say() {
    let on_disk = engine(Some("release-2"), "linux_package", "x86_64");
    let mut running = engine(Some("release-2"), "linux_package", "x86_64");
    running.product_version = "0.15.0".into();
    assert_eq!(
        engine_alignment(Some(&on_disk), Some(&running)),
        Alignment::Aligned,
        "a build id is compared with a build id, never a product version"
    );
}

#[test]
fn another_build_on_disk_is_a_pending_restart() {
    let on_disk = engine(Some("release-3"), "linux_package", "x86_64");
    let running = engine(Some("release-2"), "linux_package", "x86_64");
    assert_eq!(
        engine_alignment(Some(&on_disk), Some(&running)),
        Alignment::PendingRestart
    );
}

#[test]
fn no_daemon_is_not_running() {
    let on_disk = engine(Some("release-3"), "linux_package", "x86_64");
    assert_eq!(
        engine_alignment(Some(&on_disk), None),
        Alignment::NotRunning
    );
}

#[test]
fn a_missing_build_id_is_unknown_never_a_guess() {
    let on_disk = engine(Some("release-3"), "linux_package", "x86_64");
    let older = engine(None, "linux_package", "x86_64");
    assert_eq!(
        engine_alignment(Some(&on_disk), Some(&older)),
        Alignment::Unknown
    );
    let running = engine(Some("release-3"), "linux_package", "x86_64");
    assert_eq!(
        engine_alignment(None, Some(&running)),
        Alignment::Unknown,
        "no engine.json"
    );
    let unstamped = engine(None, "linux_package", "x86_64");
    assert_eq!(
        engine_alignment(Some(&unstamped), Some(&running)),
        Alignment::Unknown
    );
}

#[test]
fn another_kind_of_engine_is_an_ownership_conflict() {
    let on_disk = engine(Some("release-3"), "linux_package", "x86_64");
    let standalone = engine(Some("release-3"), "standalone", "x86_64");
    assert!(matches!(
        engine_alignment(Some(&on_disk), Some(&standalone)),
        Alignment::OwnershipConflict(_)
    ));
    let other_arch = engine(Some("release-3"), "linux_package", "aarch64");
    assert!(matches!(
        engine_alignment(Some(&on_disk), Some(&other_arch)),
        Alignment::OwnershipConflict(_)
    ));
}

#[test]
fn a_packaged_window_whose_file_was_replaced_is_stale() {
    let replaced = Path::new("/usr/lib/fermix-desktop/bin/fermix-desktop (deleted)");
    assert!(window_replaced(replaced));
    assert!(!window_replaced(Path::new(
        "/usr/lib/fermix-desktop/bin/fermix-desktop"
    )));
    assert!(
        !window_replaced(Path::new(
            "/home/ada/src/target-sdk/debug/fermix-desktop (deleted)"
        )),
        "a development build rebuilt under itself is not an update"
    );
}

#[test]
fn either_side_out_of_date_asks_for_the_restart() {
    assert!(restart_pending(&Alignment::PendingRestart, false));
    assert!(restart_pending(&Alignment::Aligned, true));
    assert!(restart_pending(&Alignment::NotRunning, true));
    assert!(!restart_pending(&Alignment::Aligned, false));
    assert!(!restart_pending(&Alignment::Unknown, false));
    assert!(!restart_pending(&Alignment::NotRunning, false));
    assert!(!restart_pending(
        &Alignment::OwnershipConflict("x".into()),
        false
    ));
}
