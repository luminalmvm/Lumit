//! Files the bridge's tests make and clear away.

use std::path::Path;
use std::time::Duration;

/// Deletes `path`, waiting if something still has it open, as the probe of a
/// file just imported can for a moment.
pub(crate) fn remove_when_free(path: &Path) {
    let mut tries = 0;
    while let Err(e) = std::fs::remove_file(path) {
        assert!(e.raw_os_error() == Some(32) && tries < 200, "deleted: {e}");
        tries += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
}
