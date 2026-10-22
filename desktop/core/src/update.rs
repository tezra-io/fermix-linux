//! In-app updates for the `fermix-desktop` package (update-feed-spec, format 1):
//! the signed feed, the rules that pick what to offer, the download, what
//! PackageKit's answers mean, whether a restart is pending, and every
//! sentence the update's surfaces show. The app draws and runs the I/O the
//! main loop must not; nothing here touches GTK.

pub mod check;
pub mod feed;
pub mod fetch;
pub mod identity;
pub mod keys;
pub mod packagekit;
pub mod policy;
pub mod state;
pub mod store;
pub mod version;
pub mod view;
