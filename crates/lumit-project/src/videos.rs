//! The person's own videos folder, where an export goes when nothing says
//! where.

use std::path::PathBuf;

/// The videos folder. `LUMIT_USER_DIRS` moves it under that folder, as it
/// does Lumit's own, so a test never writes into the real one.
#[must_use]
pub fn dir() -> Option<PathBuf> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let moved = std::env::var_os("LUMIT_USER_DIRS").map(PathBuf::from);
        if let Some(root) = moved.filter(|root| root.is_absolute()) {
            return Some(root.join("Videos"));
        }
        directories::UserDirs::new()?
            .video_dir()
            .map(std::path::Path::to_path_buf)
    }
    #[cfg(target_arch = "wasm32")]
    None
}
