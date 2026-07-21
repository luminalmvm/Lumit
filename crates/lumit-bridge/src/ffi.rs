//! The `extern "C"` surface Dart calls over `dart:ffi`.
//!
//! # In plain terms
//!
//! Every function here runs inside [`std::panic::catch_unwind`] so a panic
//! becomes an ordinary error reply and never unwinds into Dart
//! (docs/14-ENGINEERING-RULES: no panics across FFI). Two ownership contracts
//! live in this file:
//!
//! - **Strings** (`*mut c_char`): Rust allocates the JSON reply; Dart copies the
//!   bytes out and hands the pointer straight back to [`lumit_bridge_free_string`]
//!   so Rust frees it. Dart never frees Rust memory itself, and Rust never reads
//!   a freed pointer.
//! - **Frame buffers** (`*mut u8`): [`lumit_bridge_decode_frame`] returns a
//!   Rust-owned block of tightly-packed RGBA8 bytes (null on failure). Dart copies
//!   the pixels out and hands the pointer *and its length* back to
//!   [`lumit_bridge_free_buffer`]. The length must be exactly the `out_len` the
//!   decode wrote — the buffer is a boxed slice, freed as one.

use crate::err_json;
use crate::state::{self, with_bridge};
use std::ffi::{c_char, CStr, CString};
use std::ptr;

/// Turn a reply string into a Rust-owned C string. serde JSON never contains an
/// interior NUL, so the fallback is only a belt-and-braces guard.
fn to_c_string(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => match CString::new(err_json("internal: reply contained a NUL byte")) {
            Ok(c) => c.into_raw(),
            Err(_) => ptr::null_mut(),
        },
    }
}

/// Run a reply-producing closure, converting any panic into an error reply so
/// nothing unwinds across the C boundary.
fn guard(f: impl FnOnce() -> String + std::panic::UnwindSafe) -> *mut c_char {
    let reply = std::panic::catch_unwind(f)
        .unwrap_or_else(|_| err_json("internal error: a panic was caught at the bridge boundary"));
    to_c_string(reply)
}

/// Decode a caller-supplied C string to an owned `String`. `None` when the
/// pointer is null or the bytes are not valid UTF-8.
///
/// # Safety
/// `ptr` must be null or a valid NUL-terminated C string that stays alive for
/// the duration of the call.
unsafe fn c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(str::to_owned)
}

/// `{"ok":true,"version":"…","abi":3}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_version() -> *mut c_char {
    guard(state::version)
}

/// The built-in effect registry as `{"ok":true,"effects":[{"name","label"}]}` —
/// stateless (does not touch the document).
#[no_mangle]
pub extern "C" fn lumit_bridge_list_effects() -> *mut c_char {
    guard(crate::edits::list_effects)
}

/// Discard the current document and start an empty one. Returns a fresh snapshot.
#[no_mangle]
pub extern "C" fn lumit_bridge_new_project() -> *mut c_char {
    guard(|| with_bridge(state::new_project))
}

/// Open a `.lum` project from `path`. Returns the loaded snapshot, or an error
/// reply if the file is missing, not a Lumit project, or too new.
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_open_project(path: *const c_char) -> *mut c_char {
    let Some(path) = c_str_to_string(path) else {
        return to_c_string(err_json(
            "open project: the path was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| state::open_project(b, &path)))
}

/// Save the project. An empty `path` saves to the loaded path (error if none).
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_save_project(path: *const c_char) -> *mut c_char {
    // A null pointer is treated as "save to the loaded path", the same as "".
    let path = c_str_to_string(path).unwrap_or_default();
    guard(move || with_bridge(|b| state::save_project(b, &path)))
}

/// The current document as `{"ok":true,"items":[…],"can_undo":…,"can_redo":…,"path":…}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_snapshot() -> *mut c_char {
    guard(|| with_bridge(|b| state::snapshot(b)))
}

/// Create a composition (filed in the Compositions folder, one undo step). An
/// empty `name` becomes "Comp N".
///
/// # Safety
/// `name` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_new_composition(name: *const c_char) -> *mut c_char {
    let name = c_str_to_string(name).unwrap_or_default();
    guard(move || with_bridge(|b| state::new_composition(b, &name)))
}

/// Add a footage item referencing the media file at `path` (one undo step). With
/// the `media` feature it also probes the file synchronously and carries the
/// metadata/status in the snapshot. An empty path returns a calm error reply.
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_import_footage(path: *const c_char) -> *mut c_char {
    let Some(path) = c_str_to_string(path) else {
        return to_c_string(err_json(
            "import footage: the path was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| state::import_footage(b, &path)))
}

/// Undo the last committed operation. A no-op still returns a valid snapshot.
#[no_mangle]
pub extern "C" fn lumit_bridge_undo() -> *mut c_char {
    guard(|| with_bridge(state::undo))
}

/// Redo the last undone operation. A no-op still returns a valid snapshot.
#[no_mangle]
pub extern "C" fn lumit_bridge_redo() -> *mut c_char {
    guard(|| with_bridge(state::redo))
}

/// Flip one of a layer's switches through the real op (undoable). `switch_name`
/// is the model's own field name (`visible`, `audible`, `locked`, `solo`,
/// `motion_blur`, `fx`, `three_d`, `collapse`).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_layer_switch(
    comp_id: *const c_char,
    layer_id: *const c_char,
    switch_name: *const c_char,
    value: bool,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(name)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(switch_name),
    ) else {
        return to_c_string(err_json(
            "set layer switch: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| state::set_layer_switch(b, &comp, &layer, &name, value)))
}

/// Edit a layer's span relative to the playhead `frame`. `edit` is one of
/// `move_in`, `move_out`, `trim_in`, `trim_out`.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_edit_layer_span(
    comp_id: *const c_char,
    layer_id: *const c_char,
    edit: *const c_char,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(edit)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(edit),
    ) else {
        return to_c_string(err_json(
            "edit layer span: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| state::edit_layer_span(b, &comp, &layer, &edit, frame)))
}

/// Set one transform property to a static `value`. `property` is a snake_case
/// name mirroring `TransformProp` (e.g. `position_x`, `rotation`, `opacity`).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_transform(
    comp_id: *const c_char,
    layer_id: *const c_char,
    property: *const c_char,
    value: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(property)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(property),
    ) else {
        return to_c_string(err_json(
            "set transform: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| state::set_transform(b, &comp, &layer, &property, value)))
}

/// Drop a user marker on the composition timeline at `frame`.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string alive for the
/// call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_marker(
    comp_id: *const c_char,
    frame: i64,
) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "add marker: the composition id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| state::add_marker(b, &comp, frame)))
}

// ---------------------------------------------------------------------------
// Bridge v0.3 ops (crate::edits). Each guards its body, routes through the one
// shared bridge, and returns the refreshed snapshot (or a calm error reply).
// ---------------------------------------------------------------------------

/// Add a Solid layer (a white, comp-sized solid asset) to `comp_id`.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_solid_layer(comp_id: *const c_char) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "add solid layer: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_solid_layer(b, &comp)))
}

/// Add a Text layer (the "Text" starter document) to `comp_id`.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_text_layer(comp_id: *const c_char) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "add text layer: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_text_layer(b, &comp)))
}

/// Add a Camera layer to `comp_id`.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_camera_layer(comp_id: *const c_char) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "add camera layer: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_camera_layer(b, &comp)))
}

/// Add an Adjustment layer to `comp_id`.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_adjustment_layer(comp_id: *const c_char) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "add adjustment layer: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_adjustment_layer(b, &comp)))
}

/// Add an (empty) Sequence layer to `comp_id`.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_sequence_layer(comp_id: *const c_char) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "add sequence layer: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_sequence_layer(b, &comp)))
}

/// Delete a layer from its composition.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_delete_layer(
    comp_id: *const c_char,
    layer_id: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "delete layer: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::delete_layer(b, &comp, &layer)))
}

/// Duplicate a layer (a copy above the original, with a fresh id).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_duplicate_layer(
    comp_id: *const c_char,
    layer_id: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "duplicate layer: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::duplicate_layer(b, &comp, &layer)))
}

/// Edit a composition's settings (name, size, rate, duration in frames) as one
/// undo step; the background is preserved.
///
/// # Safety
/// `comp_id` and `name` must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_comp_settings(
    comp_id: *const c_char,
    name: *const c_char,
    width: u32,
    height: u32,
    fps_num: i64,
    fps_den: i64,
    duration_frames: i64,
) -> *mut c_char {
    let (Some(comp), Some(name)) = (c_str_to_string(comp_id), c_str_to_string(name)) else {
        return to_c_string(err_json(
            "set comp settings: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::edits::set_comp_settings(
                b,
                &comp,
                &name,
                width,
                height,
                fps_num,
                fps_den,
                duration_frames,
            )
        })
    })
}

/// The stopwatch: toggle a transform property's animation at the playhead
/// `frame` (seed a key on enable, collapse to static on disable).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_toggle_property_animated(
    comp_id: *const c_char,
    layer_id: *const c_char,
    property: *const c_char,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(property)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(property),
    ) else {
        return to_c_string(err_json(
            "toggle property animated: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::edits::toggle_property_animated(b, &comp, &layer, &property, frame))
    })
}

/// Insert or replace a transform keyframe at the playhead `frame` with `value`.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_keyframe(
    comp_id: *const c_char,
    layer_id: *const c_char,
    property: *const c_char,
    frame: i64,
    value: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(property)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(property),
    ) else {
        return to_c_string(err_json(
            "add keyframe: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::edits::add_keyframe(b, &comp, &layer, &property, frame, value))
    })
}

/// Remove the transform keyframe at the playhead `frame` (collapses to static
/// when it was the last key).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_remove_keyframe(
    comp_id: *const c_char,
    layer_id: *const c_char,
    property: *const c_char,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(property)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(property),
    ) else {
        return to_c_string(err_json(
            "remove keyframe: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::edits::remove_keyframe(b, &comp, &layer, &property, frame))
    })
}

/// Slide the transform keyframes at comp `frames_json` (a JSON int array) by
/// `delta` frames.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_shift_keyframes(
    comp_id: *const c_char,
    layer_id: *const c_char,
    property: *const c_char,
    frames_json: *const c_char,
    delta: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(property), Some(frames)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(property),
        c_str_to_string(frames_json),
    ) else {
        return to_c_string(err_json(
            "shift keyframes: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::edits::shift_keyframes(b, &comp, &layer, &property, &frames, delta))
    })
}

/// Set one work-area edge to the playhead `frame` (`is_out` picks the out edge).
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string alive for the
/// call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_work_area_edge(
    comp_id: *const c_char,
    frame: i64,
    is_out: bool,
) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "set work area edge: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::set_work_area_edge(b, &comp, frame, is_out)))
}

/// Apply a built-in effect (by its match name) to a layer.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_effect(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_name: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(name)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_name),
    ) else {
        return to_c_string(err_json(
            "add effect: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_effect(b, &comp, &layer, &name)))
}

/// Remove an effect instance from a layer by its id.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_remove_effect(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
    ) else {
        return to_c_string(err_json(
            "remove effect: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::remove_effect(b, &comp, &layer, &effect)))
}

/// Enable or bypass an effect instance.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_effect_enabled(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    enabled: bool,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
    ) else {
        return to_c_string(err_json(
            "set effect enabled: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::edits::set_effect_enabled(b, &comp, &layer, &effect, enabled))
    })
}

/// Set a scalar (Float) effect parameter to a static `value`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_scalar(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    value: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "set effect param: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::edits::set_effect_param_scalar(b, &comp, &layer, &effect, &param, value)
        })
    })
}

/// Set a Colour effect parameter to a static scene-linear RGBA.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_colour(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    r: f64,
    g: f64,
    b: f64,
    a: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "set effect param: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|bridge| {
            crate::edits::set_effect_param_colour(
                bridge, &comp, &layer, &effect, &param, r, g, b, a,
            )
        })
    })
}

/// Decode one footage frame to tightly-packed RGBA8. On success returns a
/// Rust-owned buffer and writes its width/height/length into the out-pointers;
/// on any failure returns null and sets the out-pointers to 0. The buffer must
/// be freed with [`lumit_bridge_free_buffer`] passing exactly the written length
/// (this is the F2 CPU path; the shared-texture path stays future work). Without
/// the `media` feature this always returns null.
///
/// # Safety
/// `item_id` must be null or a valid NUL-terminated UTF-8 C string. `out_w`,
/// `out_h` and `out_len` must each be null or a valid, writable pointer to a
/// `u32`/`u32`/`usize` respectively.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_decode_frame(
    item_id: *const c_char,
    frame: u64,
    out_w: *mut u32,
    out_h: *mut u32,
    out_len: *mut usize,
) -> *mut u8 {
    // Any failure past this point sets the outs to zero and returns null.
    let write_zero = || {
        if !out_w.is_null() {
            *out_w = 0;
        }
        if !out_h.is_null() {
            *out_h = 0;
        }
        if !out_len.is_null() {
            *out_len = 0;
        }
    };

    let Some(id) = c_str_to_string(item_id) else {
        write_zero();
        return ptr::null_mut();
    };

    // The decode (frame index build + FFmpeg) is slow; resolve the path under
    // the lock, then decode with the lock released so no other call is blocked.
    let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_to_buffer(&id, frame)
    }))
    .ok()
    .flatten();

    match decoded {
        Some((w, h, bytes)) => {
            let len = bytes.len();
            let raw = Box::into_raw(bytes.into_boxed_slice()) as *mut u8;
            if !out_w.is_null() {
                *out_w = w;
            }
            if !out_h.is_null() {
                *out_h = h;
            }
            if !out_len.is_null() {
                *out_len = len;
            }
            raw
        }
        None => {
            write_zero();
            ptr::null_mut()
        }
    }
}

/// Resolve `id` to a path and decode `frame` to `(width, height, rgba)`. `None`
/// on any failure. Without the `media` feature there is no decoder, so this is
/// always `None`.
#[cfg(feature = "media")]
fn decode_to_buffer(id: &str, frame: u64) -> Option<(u32, u32, Vec<u8>)> {
    let path = with_bridge(|b| state::footage_path(b, id))?;
    let f = crate::media::decode_frame(&path, frame)?;
    Some((f.width, f.height, f.rgba))
}

#[cfg(not(feature = "media"))]
fn decode_to_buffer(_id: &str, _frame: u64) -> Option<(u32, u32, Vec<u8>)> {
    None
}

/// Render composition `comp_id` at `frame` to tightly-packed RGBA8, returning a
/// Rust-owned buffer and writing its width/height/length into the out-pointers;
/// null with zeroed outs on any failure. Same ownership contract as
/// [`lumit_bridge_decode_frame`] (free with [`lumit_bridge_free_buffer`] passing
/// the exact length). `scale` of 1.0 is the comp's own resolution; a smaller
/// positive value downsamples the output. Unlike `decode_frame`, this is the
/// *composited* comp — every layer, transform, blend and effect — so a missing
/// layer arrives already slated as colour bars inside the frame. Without the
/// `render` feature (no GPU compositor linked) this always returns null; a
/// machine with no GPU adapter returns null calmly on the first and every call.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string. `out_w`,
/// `out_h` and `out_len` must each be null or a valid, writable pointer to a
/// `u32`/`u32`/`usize` respectively.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_render_comp_frame(
    comp_id: *const c_char,
    frame: u64,
    scale: f32,
    out_w: *mut u32,
    out_h: *mut u32,
    out_len: *mut usize,
) -> *mut u8 {
    let write_zero = || {
        if !out_w.is_null() {
            *out_w = 0;
        }
        if !out_h.is_null() {
            *out_h = 0;
        }
        if !out_len.is_null() {
            *out_len = 0;
        }
    };

    let Some(id) = c_str_to_string(comp_id) else {
        write_zero();
        return ptr::null_mut();
    };

    // The GPU render is slow; the renderer serialises itself behind its own lock
    // (separate from the document lock), so nothing else here is blocked. A
    // panic anywhere in the compositor becomes null, never an unwind into Dart.
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_to_buffer(&id, frame, scale)
    }))
    .ok()
    .flatten();

    match rendered {
        Some((w, h, bytes)) => {
            let len = bytes.len();
            let raw = Box::into_raw(bytes.into_boxed_slice()) as *mut u8;
            if !out_w.is_null() {
                *out_w = w;
            }
            if !out_h.is_null() {
                *out_h = h;
            }
            if !out_len.is_null() {
                *out_len = len;
            }
            raw
        }
        None => {
            write_zero();
            ptr::null_mut()
        }
    }
}

/// Render `comp_id` at `frame` to `(width, height, rgba)`. `None` on any failure.
/// Without the `render` feature there is no compositor linked, so this is always
/// `None`.
#[cfg(feature = "render")]
fn render_to_buffer(comp_id: &str, frame: u64, scale: f32) -> Option<(u32, u32, Vec<u8>)> {
    crate::render::render_comp_frame(comp_id, frame, scale)
}

#[cfg(not(feature = "render"))]
fn render_to_buffer(_comp_id: &str, _frame: u64, _scale: f32) -> Option<(u32, u32, Vec<u8>)> {
    None
}

/// Free a string returned by any string-returning function above. Passing null
/// is safe and does nothing; passing the same pointer twice is undefined,
/// exactly as with C's `free`.
///
/// # Safety
/// `s` must be null or a pointer returned by one of this crate's string
/// functions and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_free_string(s: *mut c_char) {
    let _ = std::panic::catch_unwind(|| {
        if !s.is_null() {
            drop(CString::from_raw(s));
        }
    });
}

/// Free a buffer returned by [`lumit_bridge_decode_frame`]. `len` must be exactly
/// the length that decode wrote into `out_len`. Passing null (or length 0) is
/// safe and does nothing; passing the same pointer twice, or a wrong length, is
/// undefined.
///
/// # Safety
/// `ptr` must be null or a pointer returned by [`lumit_bridge_decode_frame`],
/// not yet freed, with `len` the exact length it reported.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_free_buffer(ptr: *mut u8, len: usize) {
    let _ = std::panic::catch_unwind(|| {
        if !ptr.is_null() && len > 0 {
            drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)));
        }
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).expect("reply is valid JSON")
    }

    #[test]
    fn exported_functions_return_freeable_strings() {
        // Drive the real C ABI: call, copy, free. A double-free or use of a
        // freed pointer would be caught by miri (not in CI) — this at least
        // exercises the alloc/free contract end to end.
        let ptr = lumit_bridge_version();
        assert!(!ptr.is_null());
        let copied = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        assert_eq!(parse(&copied)["ok"], json!(true));
        assert_eq!(parse(&copied)["abi"], json!(3));
        unsafe { lumit_bridge_free_string(ptr) };

        let snap_ptr = lumit_bridge_snapshot();
        assert!(!snap_ptr.is_null());
        let snap = unsafe { CStr::from_ptr(snap_ptr) }
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(parse(&snap)["ok"], json!(true));
        unsafe { lumit_bridge_free_string(snap_ptr) };

        // Freeing null is a no-op for both free functions.
        unsafe { lumit_bridge_free_string(ptr::null_mut()) };
        unsafe { lumit_bridge_free_buffer(ptr::null_mut(), 0) };
    }

    #[test]
    fn decode_frame_of_an_unknown_item_returns_null_and_zeroes_outs() {
        // An id that is not in the document (and not even a footage item) must
        // yield null with the out-params zeroed, never a panic — the null path
        // the F2 Viewer treats as "no frame".
        let id = std::ffi::CString::new("018f0e9a-0000-7000-8000-000000000001").unwrap();
        let mut w: u32 = 123;
        let mut h: u32 = 123;
        let mut len: usize = 123;
        let ptr = unsafe { lumit_bridge_decode_frame(id.as_ptr(), 0, &mut w, &mut h, &mut len) };
        assert!(ptr.is_null());
        assert_eq!((w, h, len), (0, 0, 0));
    }

    #[test]
    fn decode_frame_with_a_null_id_returns_null() {
        let mut w: u32 = 7;
        let mut h: u32 = 7;
        let mut len: usize = 7;
        let ptr = unsafe { lumit_bridge_decode_frame(ptr::null(), 0, &mut w, &mut h, &mut len) };
        assert!(ptr.is_null());
        assert_eq!((w, h, len), (0, 0, 0));
    }

    #[test]
    fn render_comp_frame_with_a_null_id_returns_null_and_zeroes_outs() {
        let mut w: u32 = 9;
        let mut h: u32 = 9;
        let mut len: usize = 9;
        let ptr = unsafe {
            lumit_bridge_render_comp_frame(ptr::null(), 0, 1.0, &mut w, &mut h, &mut len)
        };
        assert!(ptr.is_null());
        assert_eq!((w, h, len), (0, 0, 0));
    }

    #[test]
    fn render_comp_frame_of_an_unknown_comp_returns_null_and_zeroes_outs() {
        // An id that resolves to no composition in the (empty) global document
        // yields null with the outs zeroed — the null path the Viewer treats as
        // "no comp frame", whether the machine has a GPU adapter or not.
        let id = std::ffi::CString::new("018f0e9a-0000-7000-8000-0000000000aa").unwrap();
        let mut w: u32 = 5;
        let mut h: u32 = 5;
        let mut len: usize = 5;
        let ptr = unsafe {
            lumit_bridge_render_comp_frame(id.as_ptr(), 0, 1.0, &mut w, &mut h, &mut len)
        };
        assert!(ptr.is_null());
        assert_eq!((w, h, len), (0, 0, 0));
    }
}
