//! What each update surface shows, in the macOS app's words where it has
//! them (Localizable.strings 164-221, 588): the tray's row, Home's attention
//! row, Settings' Updates group, and every state of the dialog. A failed check
//! is never shown as up to date, and an install in progress cannot be closed.

use fermix_client::update::check::CheckError;
use fermix_client::update::feed::{Engine, FeedError, Format, Package, Packages, Pair, Release};
use fermix_client::update::fetch::FetchError;
use fermix_client::update::packagekit::{Ending, Fallback};
use fermix_client::update::policy::{Offer, Outcome};
use fermix_client::update::state::{Failure, Installation, Manual, Stage, Step, Updates, Waiting};
use fermix_client::update::store::{Checked, LastCheck, Remembered};
use fermix_client::update::version::Version;
use fermix_client::update::view::{
    check_failure, checked_words, dialog_view, download_failure, home_row, install_failure,
    note_spans, settings_view, size_words, tray_label, DialogAction, RowAction, Span,
};
use std::path::PathBuf;

fn v(text: &str) -> Version {
    Version::parse(text).unwrap()
}

fn release(version: &str, engine: &str) -> Release {
    let package = |file: &str| Package {
        file: file.into(),
        url: format!("https://example.com/{file}"),
        size: 52_428_800,
        sha256: "0".repeat(64),
    };
    let pair = Pair {
        deb: package("p.deb"),
        rpm: package("p.rpm"),
    };
    Release {
        version: v(version),
        published_at: 0,
        critical: false,
        minimum_installed_version: None,
        engine: Engine {
            version: engine.into(),
            tag: format!("v{engine}"),
            source_commit: "0".repeat(40),
        },
        notes: vec![
            "Updates install from the tray.".into(),
            "Run `fermix doctor` after.".into(),
        ],
        packages: Packages {
            amd64: pair.clone(),
            arm64: pair,
        },
    }
}

fn updates(format: Option<Format>) -> Updates {
    let mut updates = Updates::new(Remembered::default());
    updates.installation = Some(Installation::Package {
        version: v("0.3.0"),
        format,
    });
    updates
}

fn offering(critical: bool, skipped: bool) -> Updates {
    let mut u = updates(Some(Format::Deb));
    u.found = Some(Outcome::Available(Box::new(Offer {
        release: release("0.4.0", "0.17.0"),
        critical,
        held_back: None,
        skipped,
    })));
    u
}

fn actions(view: &fermix_client::update::view::DialogView) -> Vec<DialogAction> {
    view.buttons.iter().map(|b| b.action).collect()
}

#[test]
fn the_tray_offers_a_check_then_the_update_it_found() {
    let mut u = updates(Some(Format::Deb));
    assert_eq!(tray_label(&u), "Check for Updates…");
    u = offering(false, false);
    assert_eq!(tray_label(&u), "Update to 0.4.0…");
    assert_eq!(
        tray_label(&offering(false, true)),
        "Check for Updates…",
        "a skipped version is quiet"
    );
    let mut pending = updates(Some(Format::Deb));
    pending.restart_pending = true;
    assert_eq!(tray_label(&pending), "Restart to Finish Updating…");
}

#[test]
fn home_names_the_update_and_offers_to_show_it() {
    assert_eq!(home_row(&updates(Some(Format::Deb))), None);
    let row = home_row(&offering(false, false)).unwrap();
    assert_eq!(row.title, "Fermix 0.4.0 is available");
    assert_eq!(
        row.body,
        "Fermix installs an update only when you ask it to."
    );
    assert_eq!(row.button, "Show");
    assert_eq!(row.action, RowAction::Show);
    let critical = home_row(&offering(true, false)).unwrap();
    assert_eq!(
        critical.body,
        "This update fixes something important. Fermix still installs it only when you ask it to."
    );
    assert_eq!(home_row(&offering(false, true)), None);
    let mut pending = updates(Some(Format::Deb));
    pending.restart_pending = true;
    let restart = home_row(&pending).unwrap();
    assert_eq!(restart.title, "Restart to finish updating");
    assert_eq!(restart.action, RowAction::Restart);
}

#[test]
fn settings_says_this_version_and_how_the_last_check_went() {
    let mut u = updates(Some(Format::Deb));
    let view = settings_view(&u, Some("0.16.0"), 1_000);
    assert_eq!(view.version, "0.3.0 · engine 0.16.0");
    assert_eq!(view.status, "Fermix has not checked for updates yet");
    assert_eq!(view.detail, None);
    assert_eq!(view.button, Some(("Check Now", RowAction::CheckNow)));
    assert!(view.automatic && view.automatic_sensitive);
    u.checking = true;
    let checking = settings_view(&u, None, 1_000);
    assert_eq!(checking.status, "Checking for updates");
    assert_eq!(checking.button, None);
    assert!(checking.checking);
    assert_eq!(checking.version, "0.3.0");
    u.checking = false;
    u.found = Some(Outcome::UpToDate);
    u.remembered.last_check = Some(LastCheck {
        at: 1_000 - 5 * 60,
        result: Checked::UpToDate,
    });
    let current = settings_view(&u, Some("0.16.0"), 1_000);
    assert_eq!(current.status, "Up to date");
    assert_eq!(
        current.detail.as_deref(),
        Some("Last checked 5 minutes ago")
    );
}

#[test]
fn a_failed_check_is_never_shown_as_up_to_date() {
    let mut u = updates(Some(Format::Deb));
    u.found = Some(Outcome::UpToDate);
    u.failure = Some("Fermix could not reach fermix.ai.".into());
    let view = settings_view(&u, None, 1_000);
    assert_eq!(view.status, "Fermix could not check for updates");
    assert_eq!(
        view.detail.as_deref(),
        Some("Fermix could not reach fermix.ai.")
    );
    assert_eq!(view.button, Some(("Check Now", RowAction::CheckNow)));
    let mut later_run = updates(Some(Format::Deb));
    later_run.remembered.last_check = Some(LastCheck {
        at: 0,
        result: Checked::Failed {
            sentence: "fermix.ai answered with an error (503).".into(),
        },
    });
    let recalled = settings_view(&later_run, None, 3 * 86_400);
    assert_eq!(recalled.status, "Fermix could not check for updates");
    assert_eq!(
        recalled.detail.as_deref(),
        Some("fermix.ai answered with an error (503). Last tried 3 days ago.")
    );
}

#[test]
fn settings_recalls_an_earlier_run_until_this_one_checks() {
    let mut u = updates(Some(Format::Deb));
    u.remembered.last_check = Some(LastCheck {
        at: 0,
        result: Checked::Available {
            version: v("0.4.0"),
            critical: false,
        },
    });
    let view = settings_view(&u, None, 7_200);
    assert_eq!(view.status, "Fermix 0.4.0 is available");
    assert_eq!(view.detail.as_deref(), Some("Last checked 2 hours ago"));
    assert_eq!(view.button, Some(("Show…", RowAction::Show)));
    u.installation = Some(Installation::Package {
        version: v("0.4.0"),
        format: Some(Format::Deb),
    });
    assert_eq!(
        settings_view(&u, None, 7_200).status,
        "Up to date",
        "0.4.0 is what is installed now"
    );
}

#[test]
fn a_development_build_says_so_and_never_checks() {
    let mut u = Updates::new(Remembered::default());
    u.installation = Some(Installation::Development {
        version: "0.3.0+dev.20261021T120000Z.2d9e931".into(),
    });
    let view = settings_view(&u, Some("0.16.0"), 0);
    assert_eq!(
        view.version,
        "0.3.0+dev.20261021T120000Z.2d9e931, a development build · engine 0.16.0"
    );
    assert_eq!(
        view.status,
        "This is a development build, which does not update itself"
    );
    assert_eq!(view.button, None);
    assert!(!view.automatic_sensitive);
    let dialog = dialog_view(&u, None);
    assert_eq!(dialog.title, "This is a development build");
    assert_eq!(actions(&dialog), [DialogAction::Close]);
}

#[test]
fn the_dialog_shows_the_offer_with_its_notes() {
    let view = dialog_view(&offering(false, false), Some("0.16.0"));
    assert_eq!(view.title, "Fermix 0.4.0 is available");
    assert_eq!(
        view.subtitle.as_deref(),
        Some("You have 0.3.0 · engine 0.16.0 → 0.17.0")
    );
    assert_eq!(view.notice, None);
    assert_eq!(view.notes.len(), 2);
    assert_eq!(
        view.notes[1],
        [
            Span::Text("Run ".into()),
            Span::Code("fermix doctor".into()),
            Span::Text(" after.".into())
        ]
    );
    assert_eq!(
        actions(&view),
        [
            DialogAction::Skip,
            DialogAction::Later,
            DialogAction::Install
        ]
    );
    assert_eq!(view.buttons[2].label, "Install Update");
    assert!(view.buttons[2].suggested);
    assert_eq!(view.buttons[0].label, "Skip This Version");
    assert!(view.closable);
    let same_engine = dialog_view(&offering(false, false), Some("0.17.0"));
    assert_eq!(
        same_engine.subtitle.as_deref(),
        Some("You have 0.3.0 · engine 0.17.0")
    );
    let unknown_engine = dialog_view(&offering(false, false), None);
    assert_eq!(
        unknown_engine.subtitle.as_deref(),
        Some("You have 0.3.0 · engine 0.17.0 with the update")
    );
}

#[test]
fn a_critical_offer_has_no_skip() {
    let view = dialog_view(&offering(true, false), Some("0.16.0"));
    assert_eq!(
        view.notice.as_deref(),
        Some("This update fixes something important.")
    );
    assert_eq!(actions(&view), [DialogAction::Later, DialogAction::Install]);
}

#[test]
fn a_held_back_release_says_to_install_this_one_first() {
    let mut u = offering(false, false);
    if let Some(Outcome::Available(offer)) = &mut u.found {
        offer.held_back = Some(v("0.5.0"));
    }
    let view = dialog_view(&u, Some("0.16.0"));
    assert_eq!(
        view.notice.as_deref(),
        Some("Fermix 0.5.0 is out too. Install this update first.")
    );
}

#[test]
fn a_copy_no_package_database_owns_cannot_install() {
    let mut u = offering(false, false);
    u.installation = Some(Installation::Package {
        version: v("0.3.0"),
        format: None,
    });
    let view = dialog_view(&u, Some("0.16.0"));
    assert_eq!(
        view.notice.as_deref(),
        Some("This copy of Fermix was not installed from its package, so it cannot install updates itself.")
    );
    assert_eq!(actions(&view), [DialogAction::Skip, DialogAction::Later]);
}

#[test]
fn the_dialog_says_when_there_is_nothing_new_or_the_check_failed() {
    let mut u = updates(Some(Format::Deb));
    u.checking = true;
    let checking = dialog_view(&u, None);
    assert_eq!(checking.title, "Checking for updates…");
    assert!(checking.spinner);
    u.checking = false;
    u.found = Some(Outcome::UpToDate);
    let current = dialog_view(&u, Some("0.16.0"));
    assert_eq!(current.title, "Fermix is up to date");
    assert_eq!(
        current.subtitle.as_deref(),
        Some("You have 0.3.0 · engine 0.16.0")
    );
    assert_eq!(actions(&current), [DialogAction::Close]);
    u.failure = Some("Fermix could not reach fermix.ai.".into());
    let failed = dialog_view(&u, None);
    assert_eq!(failed.title, "Fermix could not check for updates");
    assert_eq!(
        failed.failure.as_deref(),
        Some("Fermix could not reach fermix.ai.")
    );
    assert_eq!(
        actions(&failed),
        [DialogAction::Close, DialogAction::CheckAgain]
    );
    u.failure = None;
    u.found = Some(Outcome::Stranded { latest: v("1.0.0") });
    let stranded = dialog_view(&u, None);
    assert_eq!(stranded.title, "Fermix 1.0.0 is available");
    assert_eq!(
        stranded.notice.as_deref(),
        Some("It cannot be installed over Fermix 0.3.0 directly.")
    );
}

#[test]
fn installing_runs_in_the_dialog_one_step_after_another() {
    let mut u = offering(false, false);
    u.step = Some(Step::Downloading {
        got: 12_345_678,
        size: 52_428_800,
    });
    let downloading = dialog_view(&u, None);
    assert_eq!(downloading.title, "Downloading Fermix 0.4.0");
    let progress = downloading.progress.clone().unwrap();
    assert_eq!(progress.text, "12.3 MB of 52.4 MB");
    assert!((progress.fraction.unwrap() - 0.2355).abs() < 0.001);
    assert_eq!(actions(&downloading), [DialogAction::Cancel]);
    assert!(downloading.closable, "nothing has changed yet");
    u.step = Some(Step::Verifying);
    let verifying = dialog_view(&u, None);
    assert_eq!(verifying.title, "Checking the download");
    assert!(!verifying.closable && verifying.buttons.is_empty());
    u.step = Some(Step::Asking);
    let asking = dialog_view(&u, None);
    assert_eq!(asking.title, "Installing Fermix 0.4.0");
    let notice = asking.notice.unwrap();
    assert!(notice.contains("asks for your password"), "{notice}");
    assert!(notice.contains("untrusted package"), "{notice}");
    assert!(!asking.closable);
    u.step = Some(Step::Installing {
        percent: Some(40),
        waiting: None,
    });
    let installing = dialog_view(&u, None);
    assert_eq!(installing.progress.clone().unwrap().fraction, Some(0.4));
    assert_eq!(installing.progress.unwrap().text, "Installing…");
    u.step = Some(Step::Installing {
        percent: None,
        waiting: Some(Waiting::Lock),
    });
    let locked = dialog_view(&u, None);
    assert_eq!(
        locked.progress.unwrap().text,
        "Waiting for another installation to finish…"
    );
    u.step = Some(Step::Restarting);
    let restarting = dialog_view(&u, None);
    assert_eq!(restarting.title, "Restarting Fermix");
    assert!(!restarting.closable && restarting.spinner);
}

#[test]
fn every_failure_says_what_went_wrong_offers_try_again_and_the_command() {
    let mut u = offering(false, false);
    let file =
        PathBuf::from("/home/ada/.cache/fermix-desktop/updates/fermix-desktop_0.4.0_amd64.deb");
    u.step = Some(Step::Failed(Failure {
        stage: Stage::Install,
        sentence: install_failure(&Ending::Fallback(Fallback::Unavailable)),
        manual: Manual::Command {
            command: format!("sudo apt install {}", file.display()),
            file: file.clone(),
        },
    }));
    let view = dialog_view(&u, None);
    assert_eq!(view.title, "The update did not install");
    assert_eq!(
        view.failure.as_deref(),
        Some("This system has no software installer Fermix can ask.")
    );
    assert_eq!(
        actions(&view),
        [DialogAction::Close, DialogAction::TryAgain]
    );
    assert!(view.closable);
    assert!(matches!(view.manual, Some(Manual::Command { .. })));
    for (stage, title) in [
        (Stage::Download, "The update did not download"),
        (Stage::Restart, "Fermix did not finish updating"),
    ] {
        u.step = Some(Step::Failed(Failure {
            stage,
            sentence: "x".into(),
            manual: Manual::Reopen,
        }));
        assert_eq!(dialog_view(&u, None).title, title);
    }
}

#[test]
fn a_pending_restart_offers_the_restart() {
    let mut u = updates(Some(Format::Deb));
    u.restart_pending = true;
    let view = dialog_view(&u, None);
    assert_eq!(view.title, "Restart to finish updating");
    assert_eq!(actions(&view), [DialogAction::Later, DialogAction::Restart]);
    let settings = settings_view(&u, None, 0);
    assert_eq!(settings.status, "Restart to finish updating");
    assert_eq!(settings.button, Some(("Restart…", RowAction::Restart)));
}

#[test]
fn notes_mark_inline_code_and_nothing_else() {
    assert_eq!(
        note_spans("Plain <b>text</b> & more."),
        [Span::Text("Plain <b>text</b> & more.".into())]
    );
    assert_eq!(
        note_spans("`fermix` runs `doctor`"),
        [
            Span::Code("fermix".into()),
            Span::Text(" runs ".into()),
            Span::Code("doctor".into())
        ]
    );
    assert_eq!(
        note_spans("An odd ` stays."),
        [Span::Text("An odd ` stays.".into())]
    );
}

#[test]
fn sizes_and_times_read_as_words() {
    assert_eq!(size_words(999), "999 bytes");
    assert_eq!(size_words(1_500), "1.5 kB");
    assert_eq!(size_words(52_428_800), "52.4 MB");
    assert_eq!(size_words(1_200_000_000), "1.2 GB");
    assert_eq!(checked_words(100, 130), "just now");
    assert_eq!(checked_words(0, 60), "1 minute ago");
    assert_eq!(checked_words(0, 3_600), "1 hour ago");
    assert_eq!(checked_words(0, 86_400 * 2), "2 days ago");
    assert_eq!(checked_words(500, 100), "just now", "a clock set back");
}

#[test]
fn failures_are_sentences_a_person_can_act_on() {
    assert_eq!(
        check_failure(&CheckError::Fetch(FetchError::Transport("x".into()))),
        "Fermix could not reach fermix.ai."
    );
    assert_eq!(
        check_failure(&CheckError::Fetch(FetchError::Status(503))),
        "fermix.ai answered with an error (503)."
    );
    assert_eq!(
        check_failure(&CheckError::Feed(FeedError::Signature)),
        "The update information is not signed by Fermix, so it was ignored."
    );
    assert_eq!(
        check_failure(&CheckError::Stale { accepted: 2, generated_at: 1 }),
        "The update information from fermix.ai is older than what Fermix has already seen, so it was ignored."
    );
    assert_eq!(
        download_failure(&FetchError::Digest),
        "The download was not the file the update names, so it was deleted."
    );
    assert_eq!(
        install_failure(&Ending::Fallback(Fallback::NotAuthorized)),
        "The system did not allow the installation."
    );
    assert_eq!(
        install_failure(&Ending::Fallback(Fallback::Locked)),
        "Another program is installing software right now."
    );
    assert_eq!(
        install_failure(&Ending::Failed("dependency problems".into())),
        "The system could not install it: dependency problems"
    );
}
