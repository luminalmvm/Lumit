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
//! Video memory is answered on all three too: the renderer's DXGI adapter's
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

/// The memory free on the machine right now, in bytes, or 0 where it cannot be
/// asked. Read once as the session opens, to keep the memory cache inside what
/// is actually free (see `framecache::size_to_machine`).
pub(crate) fn free_memory_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        let mut status = MEMORYSTATUSEX {
            dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: as in `system_memory_bytes`.
        if unsafe { GlobalMemoryStatusEx(&mut status) }.is_ok() {
            return status.ullAvailPhys;
        }
        0
    }
    // `MemAvailable`: what can be had without swapping, which counts the page
    // cache the kernel would give up, unlike `MemFree`.
    #[cfg(target_os = "linux")]
    {
        meminfo_bytes("MemAvailable:")
    }
    // macOS keeps memory full of cache by design, so a "free" figure there
    // reads low on a machine with plenty to spare; it answers "not known",
    // and its pressure level is asked instead ([`memory_low`]).
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        0
    }
}

/// Whether this computer is running on battery or saving power, which on a
/// laptop holds the graphics card at low clocks: a frame that takes a few
/// milliseconds plugged in can take ten times that. The status line says so,
/// because somebody whose editor is slow rarely suspects the power plan.
///
/// Windows: on battery, or Battery saver on (`GetSystemPowerStatus`). Linux:
/// a mains supply reporting offline beside a battery, or the platform
/// profile at `low-power`. False elsewhere and where it cannot be asked.
#[frb(sync)]
pub fn power_saving() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut status = SYSTEM_POWER_STATUS::default();
        // SAFETY: a zeroed, correctly sized struct the call fills; the result
        // is checked before it is read.
        if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
            return false;
        }
        // 0 is offline (on battery), 1 online, 255 unknown; flag 1 is
        // Battery saver.
        status.ACLineStatus == 0 || status.SystemStatusFlag == 1
    }
    #[cfg(target_os = "linux")]
    {
        let read = |path: std::path::PathBuf| {
            std::fs::read_to_string(path)
                .map(|s| s.trim().to_owned())
                .unwrap_or_default()
        };
        if read("/sys/firmware/acpi/platform_profile".into()) == "low-power" {
            return true;
        }
        let Ok(supplies) = std::fs::read_dir("/sys/class/power_supply") else {
            return false;
        };
        let (mut battery, mut mains_off) = (false, false);
        for supply in supplies.flatten() {
            let path = supply.path();
            match read(path.join("type")).as_str() {
                "Battery" => battery = true,
                "Mains" => mains_off |= read(path.join("online")) == "0",
                _ => {}
            }
        }
        battery && mains_off
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// One `/proc/meminfo` figure in bytes, or 0.
#[cfg(target_os = "linux")]
fn meminfo_bytes(field: &str) -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with(field))
                .and_then(|line| line.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map_or(0, |kb| kb * 1024)
}

/// Whether the machine is short of memory now: less free than
/// [`low_memory_floor`] of what it has on Windows and Linux, and the system's
/// own pressure level at warning or worse on macOS. False where it cannot be
/// asked.
pub(crate) fn memory_low() -> bool {
    #[cfg(target_os = "macos")]
    {
        // 1 normal, 2 warning, 4 critical.
        extern "C" {
            fn sysctlbyname(
                name: *const std::os::raw::c_char,
                oldp: *mut std::os::raw::c_void,
                oldlenp: *mut usize,
                newp: *mut std::os::raw::c_void,
                newlen: usize,
            ) -> std::os::raw::c_int;
        }
        let mut level: u32 = 0;
        let mut len = std::mem::size_of::<u32>();
        // SAFETY: a NUL-terminated name, a u32 the size the sysctl writes and
        // its length, and the null/0 pair for a read; the return code is
        // checked before the value is read.
        let ret = unsafe {
            sysctlbyname(
                c"kern.memorystatus_vm_pressure_level".as_ptr(),
                std::ptr::from_mut(&mut level).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        ret == 0 && level >= 2
    }
    #[cfg(not(target_os = "macos"))]
    {
        short_of_memory(system_memory_bytes(), free_memory_bytes())
    }
}

/// The free memory under which the machine counts as short: a tenth of it,
/// and never under 512 MiB. Close to what After Effects left the system by
/// default on a small machine, and well before Windows' own low-memory
/// signal, which comes when the page file is already busy.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn low_memory_floor(total: u64) -> u64 {
    (total / 10).max(512 << 20)
}

/// [`memory_low`]'s rule on two figures, either of them 0 for "not known".
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn short_of_memory(total: u64, free: u64) -> bool {
    total > 0 && free > 0 && free < low_memory_floor(total)
}

/// The graphics card's memory in bytes, or 0 where it cannot be asked.
///
/// On Windows, the dedicated memory of the adapter the renderer opened, matched
/// by its PCI ids ([`lumit_gpu::adapter_ids`]); before the renderer has opened
/// one, the hardware adapter with the most, which is the one it asks for.
///
/// It used to be the *first* adapter DXGI enumerates. On a laptop with an
/// integrated card and a discrete one (an Intel UHD and an Nvidia MX, say) the
/// first is the integrated card, whose dedicated memory is a token 128 MB,
/// while the renderer draws on the discrete one. The governor then held a 2 GB
/// card to about 90 MB, and every frame ran the degradation ladder: work
/// refused, frames tiled, kernels sent to the processor. That is not "errs the
/// safe way", it is a machine made many times slower than it is.
///
/// macOS and Linux answer through the graphics API the engine already links —
/// Metal's recommended working-set size, or the largest device-local Vulkan
/// heap — so the answer comes from [`lumit_gpu::video_memory_bytes`] rather
/// than from a second copy of that knowledge here. See it for the details and
/// for what returns 0. Linux's answer needs an adapter, which only the renderer
/// opens, so it is 0 until one exists; the frontend's fallback covers that
/// exactly as it covers a platform with no implementation at all.
#[frb(sync)]
pub fn video_memory_bytes() -> u64 {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
        };
        let mut cards = Vec::new();
        // SAFETY: plain COM creation and enumeration; every result is checked
        // before it is read, and enumeration stops at the first index DXGI
        // refuses (DXGI_ERROR_NOT_FOUND past the last adapter).
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else {
                return 0;
            };
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                index += 1;
                let Ok(desc) = adapter.GetDesc1() else {
                    continue;
                };
                cards.push(Card {
                    vendor: desc.VendorId,
                    device: desc.DeviceId,
                    dedicated: u64::try_from(desc.DedicatedVideoMemory).unwrap_or(0),
                    software: desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0,
                });
            }
        }
        card_memory(&cards, lumit_gpu::adapter_ids())
    }
    #[cfg(not(windows))]
    {
        lumit_gpu::video_memory_bytes()
    }
}

/// One graphics adapter as DXGI lists it, for [`card_memory`].
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy)]
struct Card {
    vendor: u32,
    device: u32,
    dedicated: u64,
    software: bool,
}

/// The dedicated memory of the card the renderer is on: the one whose ids are
/// `renderer`'s, or, where those are not known yet or match nothing, the
/// hardware card with the most (the one a high-performance request takes).
/// 0 when there is no hardware card at all, which the budget reads as "not
/// known here".
#[cfg_attr(not(windows), allow(dead_code))]
fn card_memory(cards: &[Card], renderer: Option<(u32, u32)>) -> u64 {
    let hardware = || cards.iter().filter(|c| !c.software);
    renderer
        .and_then(|(vendor, device)| hardware().find(|c| c.vendor == vendor && c.device == device))
        .map(|c| c.dedicated)
        .or_else(|| hardware().map(|c| c.dedicated).max())
        .unwrap_or(0)
}

#[cfg(test)]
mod memory_tests {
    use super::short_of_memory;

    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;

    #[test]
    fn short_is_under_a_tenth_and_never_under_half_a_gigabyte() {
        // An 8 GB laptop at 92% has about 650 MB free: short.
        assert!(short_of_memory(8 * GB, 650 * MB));
        assert!(!short_of_memory(8 * GB, 1200 * MB));
        // A small machine still keeps half a gigabyte.
        assert!(short_of_memory(4 * GB, 450 * MB));
        assert!(!short_of_memory(32 * GB, 4 * GB));
        // Not known is never short.
        assert!(!short_of_memory(0, 100 * MB));
        assert!(!short_of_memory(8 * GB, 0));
    }
}

#[cfg(test)]
mod card_tests {
    use super::{card_memory, Card};

    const MB: u64 = 1 << 20;
    const INTEL: Card = Card {
        vendor: 0x8086,
        device: 0x9b41,
        dedicated: 128 * MB,
        software: false,
    };
    const MX230: Card = Card {
        vendor: 0x10de,
        device: 0x1d11,
        dedicated: 2048 * MB,
        software: false,
    };
    const WARP: Card = Card {
        vendor: 0x1414,
        device: 0x8c,
        dedicated: 0,
        software: true,
    };

    /// A laptop lists its integrated card first; the renderer is on the
    /// discrete one, and that is the card measured.
    #[test]
    fn the_renderers_card_is_measured_not_the_first() {
        let cards = [INTEL, MX230, WARP];
        assert_eq!(card_memory(&cards, Some((0x10de, 0x1d11))), 2048 * MB);
        // Pinned to the integrated card (WGPU_POWER_PREF=low), it is that one.
        assert_eq!(card_memory(&cards, Some((0x8086, 0x9b41))), 128 * MB);
    }

    /// Before the renderer opens, or for ids DXGI does not list, the hardware
    /// card with the most; the software rasteriser never counts.
    #[test]
    fn unknown_ids_take_the_largest_hardware_card() {
        assert_eq!(card_memory(&[INTEL, MX230, WARP], None), 2048 * MB);
        assert_eq!(card_memory(&[INTEL, MX230], Some((1, 2))), 2048 * MB);
        assert_eq!(card_memory(&[WARP], None), 0);
        assert_eq!(card_memory(&[], None), 0);
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
/// ignores (see the camera tools' layer). macOS and Linux under X11 do the
/// same ([`crate::cursor`]); Wayland lets nobody move the pointer.
static FROZEN_CURSOR: std::sync::Mutex<Option<(f64, f64)>> = std::sync::Mutex::new(None);

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
        *held = Some((f64::from(point.x), f64::from(point.y)));
        true
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let Some(at) = crate::cursor::position() else {
            return false;
        };
        let Ok(mut held) = FROZEN_CURSOR.lock() else {
            return false;
        };
        *held = Some(at);
        true
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
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
            let _ = unsafe { SetCursorPos(x as i32, y as i32) };
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let Ok(held) = FROZEN_CURSOR.lock() else {
            return;
        };
        if let Some((x, y)) = *held {
            crate::cursor::warp(x, y);
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

/// Open the browser's graphics card, before the first project opens. False
/// when the browser has none to give, which is the page's cue to say so.
///
/// Only the browser build has anything to do here. WebGPU hands the card over
/// as a promise, and the page's one thread can't wait for that in the middle
/// of opening a project, so it is awaited once, up front. Everywhere else the
/// card is opened when it is first needed.
pub async fn prepare_graphics() -> bool {
    // The first call the browser build makes, and it has no library loading
    // to arm the panic hook on.
    #[cfg(target_arch = "wasm32")]
    crate::faults::watch();
    #[cfg(target_arch = "wasm32")]
    return lumit_gpu::device::prepare_web().await.is_ok();
    #[cfg(not(target_arch = "wasm32"))]
    true
}
