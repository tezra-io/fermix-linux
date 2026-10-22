//! The downloads folder this app makes is 0700 whatever the umask. Alone in
//! its own test binary, since the umask is the whole process's.

use fermix_client::update::store::prepare_downloads;
use std::os::unix::fs::PermissionsExt;

#[test]
fn a_downloads_folder_made_under_a_narrow_umask_is_still_0700() {
    let dir = tempfile::tempdir().unwrap();
    let updates = dir.path().join("updates");
    // SAFETY: umask(2) cannot fail, and no other test runs in this process.
    let before = unsafe { libc::umask(0o277) };
    let made = prepare_downloads(&updates);
    unsafe { libc::umask(before) };
    made.unwrap();
    let mode = std::fs::metadata(&updates).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode, 0o700);
}
