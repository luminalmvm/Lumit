//! Claiming a saved project for this process, so two copies of Lumit do not
//! write over each other.

use super::*;

impl OpenClaim {
    /// Claim the project saved at `project` for this process.
    ///
    /// `Ok(None)` when no claim can be made here, for want of a cache folder
    /// or the right to write in it. That is no reason to refuse an open.
    /// `Err` when somebody holds it, with the process id they wrote, or 0
    /// when that can't be read.
    pub fn take(project: &Path) -> Result<Option<Self>, u32> {
        match dirs::project_dirs() {
            Some(dirs) => Self::take_in(&dirs.cache_dir().join("open"), project),
            None => Ok(None),
        }
    }

    /// [`Self::take`], with the claims kept in `dir`.
    pub fn take_in(dir: &Path, project: &Path) -> Result<Option<Self>, u32> {
        let key = path_key(project);
        let path = dir.join(format!("{key}.lock"));
        let _ = fs::create_dir_all(dir);
        let holder = || {
            let id = fs::read_to_string(&path).ok();
            id.and_then(|id| id.trim().parse().ok()).unwrap_or(0)
        };
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 0x1;
            const FILE_SHARE_DELETE: u32 = 0x4;
            const GENERIC_WRITE: u32 = 0x4000_0000;
            const DELETE: u32 = 0x0001_0000;
            const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
            // Others may read the file and nothing more, and it is deleted
            // when this handle closes.
            options
                .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                .access_mode(GENERIC_WRITE | DELETE)
                .custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
        }
        let mut tries = 0;
        let mut file = loop {
            match options.open(&path) {
                Ok(file) => break file,
                // ERROR_SHARING_VIOLATION, which is the holder's handle saying
                // no. A claim being let go has no holder to read for a moment,
                // so that one is asked again.
                Err(e) if cfg!(windows) && e.raw_os_error() == Some(32) => {
                    let holder = holder();
                    if holder != 0 || tries == 3 {
                        return Err(holder);
                    }
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => return Ok(None),
            }
        };
        #[cfg(not(windows))]
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => return Err(holder()),
            Err(_) => return Ok(None),
        }
        let _ = file.set_len(0);
        let _ = write!(file, "{}", std::process::id());
        Ok(Some(Self { _file: file, key }))
    }

    /// Whether this is the claim on the project saved at `project`.
    #[must_use]
    pub fn covers(&self, project: &Path) -> bool {
        self.key == path_key(project)
    }
}
