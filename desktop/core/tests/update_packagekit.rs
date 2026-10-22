//! What PackageKit's answers to InstallFiles mean for the update (proposal,
//! revision 5): only-trusted first, again untrusted on MISSING_GPG_SIGNATURE,
//! and the terminal command whenever PackageKit cannot do it here. The
//! numbers are libpackagekit-glib2's own (`pk_*_enum_from_string`).

use fermix_client::update::packagekit::{
    ending, error, exit, refusal, status, Ending, Fallback, ONLY_TRUSTED,
};
use fermix_client::update::state::{waiting, Waiting};

#[test]
fn the_numbers_are_packagekit_s() {
    assert_eq!(ONLY_TRUSTED, 1 << 1);
    assert_eq!(
        (
            exit::SUCCESS,
            exit::FAILED,
            exit::CANCELLED,
            exit::NEED_UNTRUSTED
        ),
        (1, 2, 3, 8)
    );
    assert_eq!(error::MISSING_GPG_SIGNATURE, 31);
    assert_eq!(error::NOT_AUTHORIZED, 48);
    assert_eq!(error::CANNOT_GET_LOCK, 26);
    assert_eq!(error::LOCK_REQUIRED, 67);
    assert_eq!(error::TRANSACTION_CANCELLED, 17);
    assert_eq!(status::WAITING_FOR_LOCK, 30);
    assert_eq!(status::WAITING_FOR_AUTH, 31);
}

#[test]
fn a_finished_success_is_installed() {
    assert_eq!(ending(exit::SUCCESS, None, true), Ending::Installed);
    assert_eq!(ending(exit::SUCCESS, None, false), Ending::Installed);
}

#[test]
fn an_unsigned_local_package_is_asked_for_again_untrusted() {
    let unsigned = Some((error::MISSING_GPG_SIGNATURE, "no signature"));
    assert_eq!(ending(exit::FAILED, unsigned, true), Ending::RetryUntrusted);
    assert_eq!(
        ending(exit::NEED_UNTRUSTED, None, true),
        Ending::RetryUntrusted
    );
    assert!(
        matches!(ending(exit::FAILED, unsigned, false), Ending::Failed(_)),
        "the untrusted pass is the last"
    );
}

#[test]
fn no_authorization_or_a_held_lock_falls_back_to_the_command() {
    let refused = Some((error::NOT_AUTHORIZED, "Failed to obtain authentication."));
    assert_eq!(
        ending(exit::FAILED, refused, false),
        Ending::Fallback(Fallback::NotAuthorized)
    );
    for code in [error::CANNOT_GET_LOCK, error::LOCK_REQUIRED] {
        assert_eq!(
            ending(
                exit::FAILED,
                Some((code, "unattended-upgrades holds it")),
                true
            ),
            Ending::Fallback(Fallback::Locked)
        );
    }
}

#[test]
fn any_other_failure_carries_packagekit_s_words() {
    let conflict = Some((35u32, "file /usr/bin/x conflicts with package y"));
    assert_eq!(
        ending(exit::FAILED, conflict, false),
        Ending::Failed("file /usr/bin/x conflicts with package y".into())
    );
    assert!(matches!(
        ending(exit::FAILED, None, false),
        Ending::Failed(_)
    ));
    assert!(matches!(
        ending(exit::CANCELLED, None, false),
        Ending::Failed(_)
    ));
    assert!(matches!(ending(0, None, false), Ending::Failed(_)));
}

#[test]
fn a_bus_without_packagekit_or_a_refusal_falls_back_to_the_command() {
    for missing in [
        None,
        Some("org.freedesktop.DBus.Error.ServiceUnknown"),
        Some("org.freedesktop.DBus.Error.NameHasNoOwner"),
        Some("org.freedesktop.DBus.Error.Spawn.ChildExited"),
        Some("org.freedesktop.PackageKit.Transaction.NotSupported"),
        Some("org.freedesktop.PackageKit.Transaction.MimeTypeNotSupported"),
    ] {
        assert_eq!(
            refusal(missing, "x"),
            Ending::Fallback(Fallback::Unavailable),
            "{missing:?}"
        );
    }
    for denied in [
        "org.freedesktop.DBus.Error.AccessDenied",
        "org.freedesktop.PackageKit.Transaction.RefusedByPolicy",
        "org.freedesktop.PackageKit.Transaction.PermissionDenied",
        "org.freedesktop.PackageKit.Transaction.NotAuthorized",
    ] {
        assert_eq!(
            refusal(Some(denied), "x"),
            Ending::Fallback(Fallback::NotAuthorized),
            "{denied}"
        );
    }
    assert_eq!(
        refusal(
            Some("org.freedesktop.PackageKit.Transaction.NoSuchFile"),
            "no such file"
        ),
        Ending::Failed("no such file".into())
    );
}

#[test]
fn the_dialog_says_what_packagekit_waits_for() {
    assert_eq!(waiting(status::WAITING_FOR_LOCK), Some(Waiting::Lock));
    assert_eq!(waiting(status::WAITING_FOR_AUTH), Some(Waiting::Password));
    assert_eq!(waiting(9), None, "installing");
    assert_eq!(waiting(0), None);
}
