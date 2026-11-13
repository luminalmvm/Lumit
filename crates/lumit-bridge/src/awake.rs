//! Keeping the computer awake while an export runs.
//!
//! A computer that sleeps halfway through an export stops it. So while
//! anything holds it, the system is asked not to sleep, and let go when the
//! last hold goes. The screen may still turn off.
//!
//! Threads: whichever asks. Holds are counted under one lock, and the only
//! call made inside it is the system's own, which answers at once.

use std::sync::Mutex;

/// Why the computer is kept awake, for whoever asks the system.
const WHY: &str = "Lumit is exporting";

/// One hold. Dropping it lets go.
pub(crate) struct Awake(());

/// How many holds there are, and what keeps the computer awake while there
/// are any.
static HOLDS: Mutex<(usize, Option<Keeper>)> = Mutex::new((0, None));

/// Keep the computer awake until the answer is dropped. Overlapping holds are
/// one.
pub(crate) fn hold() -> Awake {
    if let Ok(mut holds) = HOLDS.lock() {
        holds.0 += 1;
        if holds.1.is_none() {
            holds.1 = Keeper::begin();
        }
    }
    Awake(())
}

impl Drop for Awake {
    fn drop(&mut self) {
        let Ok(mut holds) = HOLDS.lock() else {
            return;
        };
        holds.0 = holds.0.saturating_sub(1);
        if holds.0 == 0 {
            if let Some(keeper) = holds.1.take() {
                keeper.end();
            }
        }
    }
}

/// The system's own request, while it stands.
#[cfg(windows)]
struct Keeper(isize);

#[cfg(windows)]
impl Keeper {
    fn begin() -> Option<Keeper> {
        use windows::Win32::System::Power::{
            PowerCreateRequest, PowerRequestSystemRequired, PowerSetRequest,
        };
        use windows::Win32::System::Threading::{
            POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
        };
        let mut why: Vec<u16> = WHY.encode_utf16().chain([0]).collect();
        let context = REASON_CONTEXT {
            Version: 0,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: windows::core::PWSTR(why.as_mut_ptr()),
            },
        };
        // SAFETY: `context` and the string it points at outlive the call,
        // which copies the string. The handle is checked before it is used,
        // and closed again if the request is refused.
        unsafe {
            let request = PowerCreateRequest(&context).ok()?;
            if PowerSetRequest(request, PowerRequestSystemRequired).is_err() {
                let _ = windows::Win32::Foundation::CloseHandle(request);
                return None;
            }
            Some(Keeper(request.0 as isize))
        }
    }

    fn end(self) {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        use windows::Win32::System::Power::{PowerClearRequest, PowerRequestSystemRequired};
        let request = HANDLE(self.0 as *mut std::ffi::c_void);
        // SAFETY: the handle is the one `begin` made and nothing else has
        // closed it. It is cleared and closed once, here.
        unsafe {
            let _ = PowerClearRequest(request, PowerRequestSystemRequired);
            let _ = CloseHandle(request);
        }
    }
}

/// A program of the system's that keeps it awake while it runs.
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct Keeper(std::process::Child);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Keeper {
    fn begin() -> Option<Keeper> {
        use std::process::{Command, Stdio};
        #[cfg(target_os = "macos")]
        let mut command = {
            // It also ends by itself should this process die first.
            let mut command = Command::new("caffeinate");
            command.args(["-i", "-w", &std::process::id().to_string()]);
            command
        };
        // ponytail: a Flatpak or a system without systemd has no
        // systemd-inhibit, and the export carries on without a hold. Ask the
        // portal's Inhibit if sleeping exports are ever reported from one.
        //
        // What it holds for is `cat` reading a pipe from here, not `sleep
        // infinity`: closing the pipe ends both, where killing the inhibitor
        // would leave a sleep behind each time, and the pipe closes by itself
        // should this process die first.
        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = Command::new("systemd-inhibit");
            let why = format!("--why={WHY}");
            command.args([
                "--what=sleep:idle",
                "--who=Lumit",
                why.as_str(),
                "--mode=block",
                "cat",
            ]);
            command
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command.spawn().ok().map(Keeper)
    }

    fn end(mut self) {
        // Closing its input ends `cat` on Linux. caffeinate takes no input.
        drop(self.0.stdin.take());
        #[cfg(target_os = "macos")]
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Nowhere else has a way to ask.
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
struct Keeper;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl Keeper {
    fn begin() -> Option<Keeper> {
        let _ = WHY;
        None
    }

    fn end(self) {}
}
