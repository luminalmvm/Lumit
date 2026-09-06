//! What the machine has, for the settings that spend it.
//!
//! The cache budgets in Settings → Performance are typed numbers now rather
//! than a pick from a fixed list, so they need a real ceiling: asking for more
//! memory than the machine owns is not a setting, it is a way to make the
//! session swap. Both answers are **bytes, or 0 for "not known here"** — the
//! frontend falls back to its own documented ceiling on 0 rather than
//! pretending, so a platform without an implementation is honest rather than
//! wrong.
//!
//! Windows is the shipped target, but installed RAM is answerable on
//! every supported desktop target, so all three answer it:
//! `GlobalMemoryStatusEx` on Windows, `MemTotal:` from `/proc/meminfo` on
//! Linux, and the `hw.memsize` sysctl on macOS. Windows and macOS report the
//! installed total; Linux's `MemTotal` is *usable* RAM, which excludes what
//! firmware and an integrated GPU reserved before the kernel booted (about
//! 15.5 GB on a 16 GB host). That errs low, which is the safe direction for a
//! budget ceiling — the same choice `video_memory_bytes` makes below.
//!
//! Video memory is answered on all three too: the first DXGI adapter's
//! dedicated video memory on Windows, and on the other two through the graphics
//! API the engine already links — see [`video_memory_bytes`].

use flutter_rust_bridge::frb;

/// What this process is actually holding, in bytes, or 0 where it cannot be
/// asked — the number the operating system's own monitor shows.
///
/// **Why this exists.** Lumit has now twice been reported holding tens of
/// gigabytes, and each time the first question took days to answer: is a cache
/// doing exactly what it was told, or is something holding memory nobody is
/// counting? Every tier already reports its own bytes; what was missing was the
/// total to weigh them against. The difference between the two is the whole
/// diagnosis, so it is worth one syscall.
///
/// Each platform's nearest equivalent of "what the task manager says":
/// `PROCESS_MEMORY_COUNTERS.WorkingSetSize` on Windows, `VmRSS` from
/// `/proc/self/status` on Linux, and `phys_footprint` from `TASK_VM_INFO` on
/// macOS — which is the number Activity Monitor prints under **Memory**, and so
/// the one a user reads back to us. Resident set size would have been the
/// obvious macOS choice and is the wrong one: it omits the compressed pages and
/// the IOSurface and Metal allocations a graphics application lives on, which
/// is most of what we would be hunting.
#[frb(sync)]
#[must_use]
pub fn resident_memory_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows::Win32::System::Threading::GetCurrentProcess;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: `counters` is a correctly sized, zeroed struct with its `cb`
        // set, and the pseudo-handle from `GetCurrentProcess` needs no closing.
        // The result is checked before the struct is trusted.
        if unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) }.is_ok()
        {
            return counters.WorkingSetSize as u64;
        }
        0
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(status) = std::fs::read_to_string("/proc/self/status") else {
            return 0;
        };
        for line in status.lines() {
            let Some(rest) = line.strip_prefix("VmRSS:") else {
                continue;
            };
            // "VmRSS:   123456 kB"
            let kb = rest.split_whitespace().next().unwrap_or("");
            if let Ok(kb) = kb.parse::<u64>() {
                return kb * 1024;
            }
        }
        0
    }
    #[cfg(target_os = "macos")]
    {
        // `task_info` with `TASK_VM_INFO` fills a struct whose 40th and 41st
        // 64-bit fields are `phys_footprint` and `min_address`; only the first
        // is wanted, and the struct is read through its documented layout
        // rather than a binding crate this workspace does not otherwise need.
        #[repr(C)]
        #[derive(Default, Clone, Copy)]
        struct TaskVmInfo {
            virtual_size: u64,
            region_count: i32,
            page_size: i32,
            resident_size: u64,
            resident_size_peak: u64,
            device: u64,
            device_peak: u64,
            internal: u64,
            internal_peak: u64,
            external: u64,
            external_peak: u64,
            reusable: u64,
            reusable_peak: u64,
            purgeable_volatile_pmap: u64,
            purgeable_volatile_resident: u64,
            purgeable_volatile_virtual: u64,
            compressed: u64,
            compressed_peak: u64,
            compressed_lifetime: u64,
            phys_footprint: u64,
        }

        extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(
                target_task: u32,
                flavor: u32,
                task_info_out: *mut std::os::raw::c_void,
                task_info_count: *mut u32,
            ) -> std::os::raw::c_int;
        }

        /// `TASK_VM_INFO`, from `mach/task_info.h`.
        const TASK_VM_INFO: u32 = 22;

        let mut info = TaskVmInfo::default();
        // The count is in 32-bit words, which is what the flavour's
        // `TASK_VM_INFO_COUNT` macro computes.
        let mut count =
            u32::try_from(std::mem::size_of::<TaskVmInfo>() / std::mem::size_of::<u32>())
                .unwrap_or(0);
        // SAFETY: `info` is a zeroed struct of exactly the layout the flavour
        // fills and `count` says how much room it has, so the kernel writes at
        // most that; it truncates rather than overruns when the running kernel's
        // struct is longer. `mach_task_self` needs no release. The return code
        // is checked before the value is trusted.
        let ret = unsafe {
            task_info(
                mach_task_self(),
                TASK_VM_INFO,
                std::ptr::addr_of_mut!(info).cast(),
                &mut count,
            )
        };
        if ret == 0 && info.phys_footprint > 0 {
            return info.phys_footprint;
        }
        // A kernel that answered a shorter struct than `phys_footprint` reaches
        // still answered the resident size, which is better than nothing.
        if ret == 0 {
            return info.resident_size;
        }
        0
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

/// The machine's installed memory in bytes, or 0 where it cannot be asked.
#[frb(sync)]
pub fn system_memory_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: `status` is a correctly sized, zeroed MEMORYSTATUSEX with
        // its `dwLength` set, which is the whole of this call's contract.
        if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() {
            return status.ullTotalPhys;
        }
        0
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
            for line in meminfo.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
        0
    }
    #[cfg(target_os = "macos")]
    {
        extern "C" {
            fn sysctlbyname(
                name: *const std::os::raw::c_char,
                oldp: *mut std::os::raw::c_void,
                oldlenp: *mut usize,
                newp: *mut std::os::raw::c_void,
                newlen: usize,
            ) -> std::os::raw::c_int;
        }

        let mut memsize: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let name = c"hw.memsize";
        // SAFETY: `name` is a NUL-terminated literal that outlives the call;
        // `memsize` is a zeroed u64 and `len` its size, which matches
        // `hw.memsize`'s uint64_t, so sysctl has room for exactly what it
        // writes; `newp`/`newlen` are the documented null/0 for a read. The
        // return code is checked before `memsize` is trusted.
        let ret = unsafe {
            sysctlbyname(
                name.as_ptr(),
                &mut memsize as *mut _ as *mut _,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if ret == 0 && memsize > 0 {
            return memsize;
        }
        0
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

/// The graphics card's memory in bytes, or 0 where it cannot be asked.
///
/// On Windows, the *first* adapter DXGI enumerates, which is the one the
/// renderer takes too. A machine with a discrete card behind an integrated one
/// would report the integrated adapter's memory; that is a smaller ceiling than
/// the truth, which errs the safe way for a budget.
///
/// macOS and Linux answer through the graphics API the engine already links —
/// Metal's recommended working-set size, or the largest device-local Vulkan
/// heap — so the answer comes from [`lumit_render::video_memory_bytes`] rather
/// than from a second copy of that knowledge here. See it for the details and
/// for what returns 0. Linux's answer needs an adapter, which only the renderer
/// opens, so it is 0 until one exists; the frontend's fallback covers that
/// exactly as it covers a platform with no implementation at all.
#[frb(sync)]
pub fn video_memory_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};
        // SAFETY: both calls are plain COM creation/enumeration, and every
        // result is checked before it is read.
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
                return 0;
            };
            let Ok(adapter) = factory.EnumAdapters(0) else {
                return 0;
            };
            let adapter: IDXGIAdapter = adapter;
            let Ok(desc) = adapter.GetDesc() else {
                return 0;
            };
            u64::try_from(desc.DedicatedVideoMemory).unwrap_or(0)
        }
    }
    #[cfg(not(windows))]
    {
        lumit_render::video_memory_bytes()
    }
}

/// Where the pointer was when a drag took hold of it, for the tools that hold
/// it still.
///
/// **In plain terms.** Dragging a camera about is not a gesture with a *place*
/// — nothing on the picture is being aimed at, only the movement matters — so
/// the pointer running off the edge of the picture, and eventually off the edge
/// of the screen, is pure loss: the drag stops when the pointer runs out of
/// desk. Every 3D application answers this the same way: the pointer is pinned
/// where it was pressed and only its *movement* is read, and it reappears where
/// it started when the button comes up.
///
/// Windows has no "lock the pointer" call, so this is the way it is done
/// everywhere: remember where it was, and put it back after each movement.
/// Putting it back is itself a movement, which the frontend recognises and
/// ignores (see the camera tools' layer).
static FROZEN_CURSOR: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// Remember where the pointer is, and say whether it could be — a platform
/// with no implementation answers `false`, and the frontend then simply lets
/// the pointer travel as it always did.
#[frb(sync)]
pub fn freeze_cursor() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut point = POINT::default();
        // SAFETY: `point` is a live, correctly sized POINT; the call fills it.
        if unsafe { GetCursorPos(&mut point) }.is_err() {
            return false;
        }
        let Ok(mut held) = FROZEN_CURSOR.lock() else {
            return false;
        };
        *held = Some((point.x, point.y));
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Put the pointer back where [freeze_cursor] left it. Nothing at all when
/// nothing is frozen, so an extra call is harmless.
#[frb(sync)]
pub fn restore_frozen_cursor() {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::SetCursorPos;
        let Ok(held) = FROZEN_CURSOR.lock() else {
            return;
        };
        if let Some((x, y)) = *held {
            // SAFETY: a plain call with two integers; a refusal (a locked
            // desktop, another window holding the pointer) is not an error
            // worth acting on — the drag carries on either way.
            let _ = unsafe { SetCursorPos(x, y) };
        }
    }
}

/// Let the pointer go again, at the end of the drag.
#[frb(sync)]
pub fn thaw_cursor() {
    if let Ok(mut held) = FROZEN_CURSOR.lock() {
        *held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Freezing and thawing must be safe to call in any order, and thawing must
    /// actually let go — a restore after it moves nothing, which is what keeps a
    /// drag that ended from dragging the pointer back later.
    ///
    /// The pointer is never *moved* here: `restore_frozen_cursor` with nothing
    /// frozen is by construction a no-op, and a test that warped the developer's
    /// mouse would be a rude test.
    #[test]
    fn thawing_lets_the_pointer_go() {
        freeze_cursor();
        thaw_cursor();
        assert!(
            FROZEN_CURSOR
                .lock()
                .map(|held| held.is_none())
                .unwrap_or(false),
            "thawing leaves nothing to restore to"
        );
        restore_frozen_cursor();
    }

    /// Video memory is answered on all three desktops — but not all
    /// three answer at the same *moment*, which is the part worth pinning.
    /// Windows asks DXGI and macOS asks Metal, and both work with no renderer
    /// open, so a zero there is a broken implementation rather than a machine
    /// that will not say. Linux can only read the Vulkan heaps through an
    /// adapter, which only the renderer opens, so it answers 0 until one has
    /// been — the frontend's fallback covers that exactly as it covers a
    /// platform with no implementation at all, and asserting otherwise here
    /// would fail on a perfectly healthy machine.
    ///
    /// **Unless there is no card.** A runner with no graphics hardware has
    /// nothing for DXGI to report the memory of, and answering 0 there is the
    /// truth rather than a fault — which is why this asks the same question
    /// `lumit_gpu::no_adapter` does — `LUMIT_REQUIRE_GPU`, spelled out here
    /// because this crate does not depend on lumit-gpu: it says whether this
    /// machine is *supposed* to have a card. CI sets it on the jobs whose
    /// runners are confirmed to enumerate an adapter and deliberately not on
    /// the Windows one (ci.yml says so in its own words), so the gate holds
    /// wherever it means anything and stands down where it does not.
    #[test]
    fn the_card_says_how_big_it_is_where_it_can_be_asked_without_a_renderer() {
        #[cfg(any(windows, target_os = "macos"))]
        {
            let required =
                std::env::var("LUMIT_REQUIRE_GPU").is_ok_and(|v| !v.is_empty() && v != "0");
            if !required && video_memory_bytes() == 0 {
                eprintln!(
                    "skipping: no card to report the memory of, and \
                     LUMIT_REQUIRE_GPU is not set"
                );
                return;
            }
            assert!(
                video_memory_bytes() > 0,
                "the card's memory should be positive on Windows/macOS"
            );
        }
    }

    /// A platform with no implementation answers `false` rather than pretending,
    /// which is what lets the camera drag fall back to reading movement between
    /// events instead of measuring against an anchor that is not being held.
    #[test]
    fn freezing_says_whether_it_worked() {
        let held = freeze_cursor();
        assert_eq!(held, cfg!(windows));
        thaw_cursor();
    }
}
