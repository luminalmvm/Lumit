//! Where the pointer is on the screen and how to put it back, for the drags
//! that hold it still (`api::system::freeze_cursor`), on macOS and on Linux
//! under X11. Windows is done in place there. Wayland lets no application move
//! the pointer, so there the drag lets it travel, as it always did.

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const c_void) -> *mut c_void;
        fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
        fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
        fn CGAssociateMouseAndMouseCursorPosition(connected: u32) -> i32;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    pub fn position() -> Option<(f64, f64)> {
        // SAFETY: a null source asks for an event carrying the pointer's
        // current place; it is checked and released before returning.
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            if event.is_null() {
                return None;
            }
            let at = CGEventGetLocation(event);
            CFRelease(event);
            Some((at.x, at.y))
        }
    }

    pub fn warp(x: f64, y: f64) {
        // SAFETY: plain calls taking values. Joining the mouse to the pointer
        // again straight after the warp stops macOS holding the pointer still
        // for a quarter of a second after it.
        unsafe {
            CGWarpMouseCursorPosition(CGPoint { x, y });
            CGAssociateMouseAndMouseCursorPosition(1);
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};

    type Display = c_void;
    type Window = c_ulong;
    type OpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut Display;
    type RootWindow = unsafe extern "C" fn(*mut Display) -> Window;
    type QueryPointer = unsafe extern "C" fn(
        *mut Display,
        Window,
        *mut Window,
        *mut Window,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_int,
        *mut c_uint,
    ) -> c_int;
    type WarpPointer = unsafe extern "C" fn(
        *mut Display,
        Window,
        Window,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_int,
        c_int,
    ) -> c_int;
    type Flush = unsafe extern "C" fn(*mut Display) -> c_int;

    /// libX11, opened when first wanted rather than linked, and one display
    /// connection of our own.
    struct X11 {
        _lib: libloading::Library,
        display: *mut Display,
        root: Window,
        query: QueryPointer,
        warp: WarpPointer,
        flush: Flush,
    }

    // SAFETY: the connection is only ever used from the thread the Viewer's
    // drags run on, one call at a time.
    unsafe impl Send for X11 {}
    // SAFETY: as above.
    unsafe impl Sync for X11 {}

    fn x11() -> Option<&'static X11> {
        static X: std::sync::OnceLock<Option<X11>> = std::sync::OnceLock::new();
        X.get_or_init(|| {
            // Under Wayland the X server is someone else's and our window is
            // not on it.
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
                && std::env::var("GDK_BACKEND").map_or(true, |b| b != "x11");
            if wayland {
                return None;
            }
            // SAFETY: libX11's own symbols with their own signatures, and a
            // display checked for null before use.
            unsafe {
                let lib = libloading::Library::new("libX11.so.6").ok()?;
                let open = *lib.get::<OpenDisplay>(b"XOpenDisplay\0").ok()?;
                let root_of = *lib.get::<RootWindow>(b"XDefaultRootWindow\0").ok()?;
                let query = *lib.get::<QueryPointer>(b"XQueryPointer\0").ok()?;
                let warp = *lib.get::<WarpPointer>(b"XWarpPointer\0").ok()?;
                let flush = *lib.get::<Flush>(b"XFlush\0").ok()?;
                let display = open(std::ptr::null());
                if display.is_null() {
                    return None;
                }
                Some(X11 {
                    root: root_of(display),
                    _lib: lib,
                    display,
                    query,
                    warp,
                    flush,
                })
            }
        })
        .as_ref()
    }

    pub fn position() -> Option<(f64, f64)> {
        let x = x11()?;
        let (mut root, mut child) = (0, 0);
        let (mut rx, mut ry, mut wx, mut wy) = (0, 0, 0, 0);
        let mut mask = 0;
        // SAFETY: a live display, its root window and out-pointers to locals.
        let found = unsafe {
            (x.query)(
                x.display, x.root, &mut root, &mut child, &mut rx, &mut ry, &mut wx, &mut wy,
                &mut mask,
            )
        };
        (found != 0).then(|| (f64::from(rx), f64::from(ry)))
    }

    pub fn warp(px: f64, py: f64) {
        let Some(x) = x11() else {
            return;
        };
        // SAFETY: a live display and its root window; a warp the server
        // refuses is not an error worth acting on.
        unsafe {
            (x.warp)(x.display, 0, x.root, 0, 0, 0, 0, px as c_int, py as c_int);
            (x.flush)(x.display);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use platform::{position, warp};
