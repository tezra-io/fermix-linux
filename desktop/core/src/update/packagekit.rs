//! PackageKit's vocabulary for installing a downloaded package (proposal,
//! revision 5), and what each ending means for the update. The app makes the
//! D-Bus calls: CreateTransaction, then InstallFiles with only-trusted, and
//! once more without it when the package carries no GPG signature, as any
//! local .deb and the unsigned .rpm do. Whatever PackageKit cannot do here, a
//! missing daemon, no authorization, a held lock, leads to the command.
//! The numbers are libpackagekit-glib2's enums.

/// The daemon on the system bus.
pub const NAME: &str = "org.freedesktop.PackageKit";
pub const PATH: &str = "/org/freedesktop/PackageKit";
pub const MANAGER: &str = "org.freedesktop.PackageKit";
pub const TRANSACTION: &str = "org.freedesktop.PackageKit.Transaction";
/// InstallFiles' flags: PK_TRANSACTION_FLAG_ENUM_ONLY_TRUSTED as a bit, or none.
pub const ONLY_TRUSTED: u64 = 1 << 1;
pub const UNTRUSTED: u64 = 0;
/// `Percentage` when PackageKit cannot say.
pub const PERCENT_UNKNOWN: u32 = 101;

/// PkExitEnum, as `Finished` carries it.
pub mod exit {
    pub const SUCCESS: u32 = 1;
    pub const FAILED: u32 = 2;
    pub const CANCELLED: u32 = 3;
    pub const NEED_UNTRUSTED: u32 = 8;
}

/// PkErrorEnum, as `ErrorCode` carries it.
pub mod error {
    pub const TRANSACTION_CANCELLED: u32 = 17;
    pub const CANNOT_GET_LOCK: u32 = 26;
    pub const MISSING_GPG_SIGNATURE: u32 = 31;
    pub const NOT_AUTHORIZED: u32 = 48;
    /// Another package manager holds the lock, as unattended-upgrades does.
    pub const LOCK_REQUIRED: u32 = 67;
}

/// PkStatusEnum, as the `Status` property carries it.
pub mod status {
    pub const WAITING_FOR_LOCK: u32 = 30;
    pub const WAITING_FOR_AUTH: u32 = 31;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Installed,
    /// Only-trusted refused an unsigned local package: ask again without it.
    RetryUntrusted,
    /// PackageKit cannot install it here; the person runs the command.
    Fallback(Fallback),
    /// It ran and failed, in PackageKit's own words where it gave some.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    /// No PackageKit on the system bus, or none that installs this kind of file.
    Unavailable,
    /// polkit said no: no agent to ask, or the password prompt was dismissed.
    NotAuthorized,
    /// Another program is installing software.
    Locked,
}

/// How one transaction ended: its `Finished` exit, the last `ErrorCode` it
/// sent, and whether this pass asked for only-trusted.
pub fn ending(exit: u32, error: Option<(u32, &str)>, only_trusted: bool) -> Ending {
    if exit == exit::SUCCESS {
        return Ending::Installed;
    }
    let code = error.map(|(code, _)| code);
    let unsigned = exit == exit::NEED_UNTRUSTED || code == Some(error::MISSING_GPG_SIGNATURE);
    if unsigned && only_trusted {
        return Ending::RetryUntrusted;
    }
    match error {
        Some((error::NOT_AUTHORIZED, _)) => Ending::Fallback(Fallback::NotAuthorized),
        Some((error::CANNOT_GET_LOCK | error::LOCK_REQUIRED, _)) => {
            Ending::Fallback(Fallback::Locked)
        }
        Some((_, details)) if !details.trim().is_empty() => Ending::Failed(details.trim().into()),
        _ if exit == exit::CANCELLED => Ending::Failed("The installation was cancelled.".into()),
        _ => Ending::Failed("PackageKit gave no reason.".into()),
    }
}

/// A D-Bus call refused before any transaction ran, by its error's name
/// (`None` when the bus itself could not be reached) and message.
pub fn refusal(name: Option<&str>, message: &str) -> Ending {
    let Some(name) = name else {
        return Ending::Fallback(Fallback::Unavailable);
    };
    let unavailable = [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.Spawn.",
        "org.freedesktop.PackageKit.Transaction.NotSupported",
        "org.freedesktop.PackageKit.Transaction.MimeTypeNotSupported",
    ];
    let denied = [
        "org.freedesktop.DBus.Error.AccessDenied",
        "org.freedesktop.PackageKit.Transaction.RefusedByPolicy",
        "org.freedesktop.PackageKit.Transaction.PermissionDenied",
        "org.freedesktop.PackageKit.Transaction.NotAuthorized",
    ];
    if unavailable
        .iter()
        .any(|n| name == *n || (n.ends_with('.') && name.starts_with(n)))
    {
        return Ending::Fallback(Fallback::Unavailable);
    }
    if denied.contains(&name) {
        return Ending::Fallback(Fallback::NotAuthorized);
    }
    Ending::Failed(message.into())
}
