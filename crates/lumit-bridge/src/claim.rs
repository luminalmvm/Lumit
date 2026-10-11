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

/// Bring the Lumit that is the process `holder` forward. False when macOS
/// knows no application by that number or will not make it active.
///
/// macOS lets the application in front hand its place on, and whoever asked
/// to open a project has just used this one. A second Lumit is rare on a Mac,
/// where the Finder hands a document to the one already running: it takes
/// `open -n`, a second copy of the application, or the program started from a
/// terminal.
///
/// Spoken to the Objective-C runtime directly, as `api::system` does for the
/// machine's memory, rather than through a binding crate nothing else here
/// needs. Called from whichever thread is opening the project, which AppKit
/// allows of `NSRunningApplication` and of little else.
// ponytail: a window minimised to the Dock stays there, with its application
// in front. Restoring it takes a reopen Apple event sent to `holder`.
#[cfg(target_os = "macos")]
pub(crate) fn bring_forward(holder: u32) -> bool {
    use std::ffi::{c_char, c_void};

    type Id = *mut c_void;
    type Find = unsafe extern "C" fn(Id, Id, i32) -> Id;
    type Activate = unsafe extern "C" fn(Id, Id, usize) -> i8;

    // AppKit is named so that the class is there to be found in a program
    // that is not an application, a test above all.
    #[link(name = "AppKit", kind = "framework")]
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Id;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }
    /// `NSApplicationActivateAllWindows`, from NSRunningApplication.h.
    const ALL_WINDOWS: usize = 1;

    let Ok(holder) = i32::try_from(holder) else {
        return false;
    };
    // SAFETY: the names are NUL-terminated literals. `objc_msgSend` is called
    // through the signature of the method each selector names, which is how
    // the runtime means it to be called: a class method taking a `pid_t` and
    // returning an object, then an instance method taking an `NSUInteger` and
    // returning a `BOOL`. A message to nil does nothing and answers nil, and
    // `app` is checked all the same. The object found is autoreleased, and
    // the pool it goes into is opened and drained here, on this thread.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let send = objc_msgSend as unsafe extern "C" fn();
        let find = std::mem::transmute::<unsafe extern "C" fn(), Find>(send);
        let activate = std::mem::transmute::<unsafe extern "C" fn(), Activate>(send);
        let app = find(
            objc_getClass(c"NSRunningApplication".as_ptr()),
            sel_registerName(c"runningApplicationWithProcessIdentifier:".as_ptr()),
            holder,
        );
        let forward = !app.is_null()
            && activate(
                app,
                sel_registerName(c"activateWithOptions:".as_ptr()),
                ALL_WINDOWS,
            ) != 0;
        objc_autoreleasePoolPop(pool);
        forward
    }
}

/// Nothing here brings another program's window forward on Linux, where a
/// compositor decides that. The caller says the project is open in another
/// window instead.
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn bring_forward(_holder: u32) -> bool {
    false
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::bring_forward;

    /// The half a test can reach without taking the screen from whoever is
    /// running it: a process that is no application is not brought forward.
    /// This test program is one such, and has no window.
    #[test]
    fn a_process_that_is_no_application_is_not_brought_forward() {
        assert!(!bring_forward(std::process::id()));
        assert!(!bring_forward(u32::MAX));
    }
}
