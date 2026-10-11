//! The claim this Lumit holds on the saved project it has open
//! (`lumit_project::OpenClaim`), and bringing forward the window of a Lumit
//! that holds one already.
//!
//! One claim for the process, because one project is open at a time. Called
//! from whichever thread is opening, saving or closing, and the lock here is
//! held for one assignment.

use std::path::Path;
use std::sync::Mutex;

use lumit_project::OpenClaim;
use uuid::Uuid;

/// The claim held, and the project it is held for.
static HELD: Mutex<Option<(Uuid, OpenClaim)>> = Mutex::new(None);

/// Claim the project saved at `path`, without letting go of the claim held.
///
/// `Ok(None)` when this process has it open already, or when no claim can be
/// made, and neither stops an open. `Err` is the process that has it instead.
pub(crate) fn take(path: &Path) -> Result<Option<OpenClaim>, u32> {
    match OpenClaim::take(path) {
        Err(holder) if holder == std::process::id() => Ok(None),
        taken => taken,
    }
}

/// `project` is now the open project, saved at `path` or with no file yet,
/// and `fresh` is the claim taken for it. The claim held before is let go,
/// unless it is on this same file.
pub(crate) fn settle(project: Uuid, path: Option<&Path>, fresh: Option<OpenClaim>) {
    let Ok(mut held) = HELD.lock() else {
        return;
    };
    let kept = held
        .take()
        .filter(|(_, claim)| path.is_some_and(|path| claim.covers(path)));
    *held = fresh
        .or(kept.map(|(_, claim)| claim))
        .map(|claim| (project, claim));
}

/// `project` has closed, so its claim goes. One closed after another has
/// replaced it holds none.
pub(crate) fn release(project: Uuid) {
    if let Ok(mut held) = HELD.lock() {
        if held.as_ref().is_some_and(|(owner, _)| *owner == project) {
            *held = None;
        }
    }
}

/// Bring the main window of the process `holder` forward, restored if it was
/// minimised. False when it has no window to show or Windows will not move it.
///
/// Windows lets the program the person has just used hand the foreground on,
/// and whoever asked to open a project has just used this one.
#[cfg(windows)]
pub(crate) fn bring_forward(holder: u32) -> bool {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindow, GetWindowThreadProcessId, IsIconic, IsWindowVisible,
        SetForegroundWindow, ShowWindow, GW_OWNER, SW_RESTORE,
    };

    /// Whose window is wanted, and the one found.
    struct Search {
        process: u32,
        window: Option<HWND>,
    }

    unsafe extern "system" fn each(window: HWND, search: LPARAM) -> BOOL {
        // SAFETY: `search` is the `Search` given to `EnumWindows` below. It
        // outlives that call and nothing else reads it meanwhile.
        let search = unsafe { &mut *(search.0 as *mut Search) };
        let mut process = 0;
        // SAFETY: plain questions about a window the system has just named.
        // A main window is one that shows and that no other window owns.
        let main = unsafe {
            GetWindowThreadProcessId(window, Some(&mut process));
            process == search.process
                && IsWindowVisible(window).as_bool()
                && GetWindow(window, GW_OWNER).map_or(true, |owner| owner.is_invalid())
        };
        if main {
            search.window = Some(window);
        }
        // False ends the search.
        BOOL::from(!main)
    }

    let mut search = Search {
        process: holder,
        window: None,
    };
    // SAFETY: `each` is given a pointer to `search`, which lives until the
    // end of this function. The other calls take a window handle and nothing
    // else, and a window that has gone since it was found is refused.
    unsafe {
        // A search ended early reads as a failure, so the answer is the
        // window and not the result.
        let _ = EnumWindows(Some(each), LPARAM(&raw mut search as isize));
        let Some(window) = search.window else {
            return false;
        };
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window).as_bool()
    }
}

/// Nothing here brings another program's window forward on Linux, where a
/// compositor decides that, and macOS is its own piece of work. The caller
/// says the project is open in another window instead.
#[cfg(not(windows))]
pub(crate) fn bring_forward(_holder: u32) -> bool {
    false
}
