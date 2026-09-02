//! `OfxInteractSuiteV1` — present, and never usable (K-757).
//!
//! **In plain terms.** An interact is the thing a plugin draws over the
//! viewer: a crosshair to drag, a box to size. Lumit tells every plugin it has
//! no overlays, so no plugin ever makes one — but the OpenFX support library
//! every commercial vendor builds on asks for this suite before it asks
//! anything else, and treats "not there" as a host missing a feature. HitFilm
//! and Red Giant Universe refused to describe on exactly that. The suite is
//! here to be found; there is no interact for any of its calls to act on, so
//! each answers `kOfxStatErrUnsupported` — the host does not do this — rather
//! than `kOfxStatErrBadHandle`, which the conformance tally counts as a
//! refused call. Red Giant Universe calls in here during describe whatever the
//! host said about overlays, and it carries on when told no.

use std::ffi::c_void;

use crate::ffi::{OfxInteractSuiteV1, OfxPropertySetHandle};
use crate::status::Status;
use crate::suites::guard_code;

pub static SUITE: OfxInteractSuiteV1 = OfxInteractSuiteV1 {
    interact_swap_buffers,
    interact_redraw,
    interact_get_property_set,
};

unsafe extern "C" fn interact_swap_buffers(_interact: *mut c_void) -> i32 {
    guard_code(|| Status::ErrUnsupported)
}

unsafe extern "C" fn interact_redraw(_interact: *mut c_void) -> i32 {
    guard_code(|| Status::ErrUnsupported)
}

unsafe extern "C" fn interact_get_property_set(
    _interact: *mut c_void,
    _property: *mut OfxPropertySetHandle,
) -> i32 {
    guard_code(|| Status::ErrUnsupported)
}
