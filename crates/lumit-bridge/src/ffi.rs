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

/// Place a project footage item into `comp_id` as a new Footage layer (top of
/// the stack, the media's own duration/size when probed).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_footage_layer(
    comp_id: *const c_char,
    item_id: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(item)) = (c_str_to_string(comp_id), c_str_to_string(item_id)) else {
        return to_c_string(err_json(
            "add footage layer: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::add_footage_layer(b, &comp, &item)))
}

/// Reorder a layer within its composition to `new_index` (0 = top; the op clamps
/// an out-of-range value into range).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_reorder_layer(
    comp_id: *const c_char,
    layer_id: *const c_char,
    new_index: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "reorder layer: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::edits::reorder_layer(b, &comp, &layer, new_index)))
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

/// Whether this build offers the Windows zero-copy shared-texture Viewer path
/// (K-177): `true` only when compiled with the `shared-texture` feature on
/// Windows. Dart calls this once to decide whether to attempt the `Texture`
/// widget path at all; `false` keeps it on the read-back path (an old `.dll`,
/// a non-Windows build, or a build without the feature). Stateless, never fails.
#[no_mangle]
pub extern "C" fn lumit_bridge_shared_supported() -> bool {
    cfg!(all(windows, feature = "shared-texture"))
}

/// Render composition `comp_id` at `frame` into the Windows shared GPU texture
/// and report its NT handle and dimensions through the out-pointers (K-177).
/// Returns `true` on success; on any failure returns `false` and zeroes the
/// out-pointers, so Dart falls back to [`lumit_bridge_render_comp_frame`] for
/// that frame. Unlike the read-back calls this returns no buffer to free — the
/// pixels never leave the GPU. The reported handle is stable across frames (the
/// same texture is re-used) and changes only when the comp is resized. Without
/// the `shared-texture` feature, on a non-Windows build, or with no D3D12
/// adapter, this always returns `false` calmly (never a crash, never a retry
/// storm).
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string. `out_handle`
/// must be null or a valid, writable pointer to a `u64`; `out_w` and `out_h`
/// must each be null or a valid, writable pointer to a `u32`.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_render_to_shared(
    comp_id: *const c_char,
    frame: u64,
    out_handle: *mut u64,
    out_w: *mut u32,
    out_h: *mut u32,
) -> bool {
    let write_zero = || {
        if !out_handle.is_null() {
            *out_handle = 0;
        }
        if !out_w.is_null() {
            *out_w = 0;
        }
        if !out_h.is_null() {
            *out_h = 0;
        }
    };

    let Some(id) = c_str_to_string(comp_id) else {
        write_zero();
        return false;
    };

    // A panic anywhere in the compositor or D3D interop becomes `false`, never an
    // unwind into Dart. The renderer serialises itself behind its own lock.
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_to_shared_buffer(&id, frame)
    }))
    .ok()
    .flatten();

    match rendered {
        Some((handle, w, h)) => {
            if !out_handle.is_null() {
                *out_handle = handle;
            }
            if !out_w.is_null() {
                *out_w = w;
            }
            if !out_h.is_null() {
                *out_h = h;
            }
            true
        }
        None => {
            write_zero();
            false
        }
    }
}

/// Render `comp_id` at `frame` into the shared texture, returning
/// `(handle, width, height)`. `None` on any failure. Without the
/// `shared-texture` feature (or off Windows) this is always `None`.
#[cfg(all(windows, feature = "shared-texture"))]
fn render_to_shared_buffer(comp_id: &str, frame: u64) -> Option<(u64, u32, u32)> {
    crate::render::render_to_shared(comp_id, frame)
}

#[cfg(not(all(windows, feature = "shared-texture")))]
fn render_to_shared_buffer(_comp_id: &str, _frame: u64) -> Option<(u64, u32, u32)> {
    None
}

/// Compute a scope trace (K-096 v1) for the frame the Viewer shows and return
/// the 256×256 RGBA8 trace bytes through `out_len` (256×256×4 = 262144 bytes),
/// or null with a zeroed length on any failure. `kind` selects the scope: `0`
/// luma waveform, `1` RGB waveform, `2` vectorscope, `3` histogram. `scale` must
/// match the Viewer's preview scale so the scope reads the very frame on screen
/// (the same rendered-frame cache key). The five trace colours are packed
/// `0x00RRGGBB` (the frontend's fixed `ScopeColours`, kept out of the engine so
/// no colour literal lives here).
///
/// The heavy binning runs on the GPU; only the tiny trace crosses the boundary.
/// The comp frame is served from the rendered-frame cache, so a frame already
/// banked for the Viewer traces without re-rendering the comp. The buffer is a
/// boxed slice — free it with [`lumit_bridge_free_buffer`] passing the exact
/// `out_len`. Without the `render` feature (no compositor linked) this is always
/// null. A panic anywhere becomes null, never an unwind into Dart.
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string. `out_len`
/// must be null or a valid, writable pointer to a `usize`.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_render_scope(
    kind: u32,
    comp_id: *const c_char,
    frame: u64,
    scale: f32,
    bg: u32,
    trace: u32,
    red: u32,
    green: u32,
    blue: u32,
    out_len: *mut usize,
) -> *mut u8 {
    let write_zero = || {
        if !out_len.is_null() {
            *out_len = 0;
        }
    };
    let Some(id) = c_str_to_string(comp_id) else {
        write_zero();
        return ptr::null_mut();
    };
    let colours = [
        unpack_rgb(bg),
        unpack_rgb(trace),
        unpack_rgb(red),
        unpack_rgb(green),
        unpack_rgb(blue),
    ];
    let traced = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_scope_buffer(kind, &id, frame, scale, colours)
    }))
    .ok()
    .flatten();
    match traced {
        Some(bytes) => {
            let len = bytes.len();
            let raw = Box::into_raw(bytes.into_boxed_slice()) as *mut u8;
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

/// Unpack a `0x00RRGGBB` colour into an `[r, g, b]` byte triple.
fn unpack_rgb(packed: u32) -> [u8; 3] {
    [
        ((packed >> 16) & 0xff) as u8,
        ((packed >> 8) & 0xff) as u8,
        (packed & 0xff) as u8,
    ]
}

/// Trace `comp_id` at `frame`/`scale` for scope `kind`. `None` on any failure.
/// Without the `render` feature there is no compositor linked, so this is always
/// `None`.
#[cfg(feature = "render")]
fn render_scope_buffer(
    kind: u32,
    comp_id: &str,
    frame: u64,
    scale: f32,
    colours: [[u8; 3]; 5],
) -> Option<Vec<u8>> {
    crate::render::render_scope(kind, comp_id, frame, scale, colours)
}

#[cfg(not(feature = "render"))]
fn render_scope_buffer(
    _kind: u32,
    _comp_id: &str,
    _frame: u64,
    _scale: f32,
    _colours: [[u8; 3]; 5],
) -> Option<Vec<u8>> {
    None
}

// ---------------------------------------------------------------------------
// Bridge v0.8 (ABI 8): the rendered-frame cache (K-176) and its controls,
// engine-side render cancellation, and the Project-panel thumbnail path.
// ---------------------------------------------------------------------------

/// Render composition `comp_id` at `frame` to RGBA8 with a latest-wins
/// `generation` for engine-side cancellation (K-176) — the generation-aware
/// sibling of [`lumit_bridge_render_comp_frame`]. Identical ownership contract
/// (free with [`lumit_bridge_free_buffer`] passing the exact length). A frame
/// already rendered under the current document is served from the bridge cache
/// without touching the GPU; a cache **miss** whose `generation` has been
/// superseded by a newer request returns null (the stale render is skipped
/// engine-side rather than stealing the renderer lock). The Dart worker passes
/// its own monotonic generation so a scrub aborts stale renders. Null with
/// zeroed outs on any failure (unknown comp, no adapter, or a superseded miss).
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string. `out_w`,
/// `out_h` and `out_len` must each be null or a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_render_comp_frame_gen(
    comp_id: *const c_char,
    frame: u64,
    scale: f32,
    generation: u64,
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

    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_to_buffer_gen(&id, frame, scale, generation)
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

#[cfg(feature = "render")]
fn render_to_buffer_gen(
    comp_id: &str,
    frame: u64,
    scale: f32,
    generation: u64,
) -> Option<(u32, u32, Vec<u8>)> {
    crate::render::render_comp_frame_gen(comp_id, frame, scale, generation)
}

#[cfg(not(feature = "render"))]
fn render_to_buffer_gen(
    _comp_id: &str,
    _frame: u64,
    _scale: f32,
    _generation: u64,
) -> Option<(u32, u32, Vec<u8>)> {
    None
}

/// Mark every render generation below `generation` as superseded (K-176), so a
/// stale comp render already queued behind the renderer lock is skipped when it
/// wakes. The Dart worker calls this when it abandons a request without issuing a
/// newer render. Stateless and always available (a no-op path in a build without
/// `render`). Returns `{"ok":true}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_render_cancel_stale(generation: u64) -> *mut c_char {
    guard(move || {
        crate::cancel::cancel_stale(generation);
        json_ok()
    })
}

/// Set the rendered-frame cache's RAM budget in **bytes** (Settings →
/// Performance → Memory budget). Evicts down to the new budget immediately.
/// Returns the fresh cache stats (see [`lumit_bridge_cache_stats`]).
#[no_mangle]
pub extern "C" fn lumit_bridge_set_cache_budget(bytes: u64) -> *mut c_char {
    guard(move || {
        let stats = crate::framecache::set_budget(bytes as usize);
        cache_stats_json(stats)
    })
}

/// Empty the rendered-frame cache now (Settings → Clear cache). Returns the
/// fresh cache stats.
#[no_mangle]
pub extern "C" fn lumit_bridge_clear_cache() -> *mut c_char {
    guard(|| cache_stats_json(crate::framecache::clear()))
}

/// The rendered-frame cache's current stats:
/// `{"ok":true,"used_bytes","budget_bytes","entries","hits","misses"}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_cache_stats() -> *mut c_char {
    guard(|| cache_stats_json(crate::framecache::stats()))
}

/// Build the cache-stats reply from a `(used, budget, entries, hits, misses)`
/// tuple.
fn cache_stats_json(stats: (usize, usize, usize, u64, u64)) -> String {
    let (used, budget, entries, hits, misses) = stats;
    serde_json::json!({
        "ok": true,
        "used_bytes": used,
        "budget_bytes": budget,
        "entries": entries,
        "hits": hits,
        "misses": misses,
    })
    .to_string()
}

/// `{"ok":true}` — the shared reply for a stateless success with no payload.
fn json_ok() -> String {
    serde_json::json!({ "ok": true }).to_string()
}

/// Decode a representative frame of footage item `item_id` and downscale it so
/// its longer edge is at most `max_edge`, returning tightly-packed RGBA8 (the
/// Project-panel thumbnail path). Same ownership contract as
/// [`lumit_bridge_decode_frame`] (free with [`lumit_bridge_free_buffer`] passing
/// the exact length). The result is cached on the bridge, so each thumbnail
/// decodes once. Null with zeroed outs on any failure (unknown/non-footage item,
/// missing/unreadable file), and always null without the `media` feature.
///
/// # Safety
/// `item_id` must be null or a valid NUL-terminated UTF-8 C string. `out_w`,
/// `out_h` and `out_len` must each be null or a valid, writable pointer.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_thumbnail(
    item_id: *const c_char,
    max_edge: u32,
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

    let Some(id) = c_str_to_string(item_id) else {
        write_zero();
        return ptr::null_mut();
    };

    let thumb = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        thumbnail_to_buffer(&id, max_edge)
    }))
    .ok()
    .flatten();

    match thumb {
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

/// Resolve `id` to a thumbnail `(width, height, rgba)`. `None` on any failure.
/// Without the `media` feature there is no decoder, so this is always `None`.
#[cfg(feature = "media")]
fn thumbnail_to_buffer(id: &str, max_edge: u32) -> Option<(u32, u32, Vec<u8>)> {
    with_bridge(|b| crate::media::thumbnail(b, id, max_edge))
}

#[cfg(not(feature = "media"))]
fn thumbnail_to_buffer(_id: &str, _max_edge: u32) -> Option<(u32, u32, Vec<u8>)> {
    None
}

// ---------------------------------------------------------------------------
// Bridge v0.4: export, keyframe interpolation, Retime, and the timeline columns.
// Each guards its body and returns a JSON reply (a refreshed snapshot for the
// mutating ops, or a small purpose-built object for the reads/export).
// ---------------------------------------------------------------------------

/// Resolve a delivery `preset_name` into the export-dialogue fields it stamps
/// plus its suggested file name — `{ok,codec,size,bitrate_mbps,include_audio,
/// default_name}`. `comp_name` and `template` drive the `{comp}`/`{preset}`/
/// `{date}` filename substitution (an empty template yields the preset's own
/// default). Stateless (does not touch the document).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_export_preset(
    preset_name: *const c_char,
    comp_name: *const c_char,
    template: *const c_char,
) -> *mut c_char {
    let preset = c_str_to_string(preset_name).unwrap_or_default();
    let comp_name = c_str_to_string(comp_name).unwrap_or_default();
    let template = c_str_to_string(template).unwrap_or_default();
    guard(move || crate::export::export_preset(&preset, &comp_name, &template))
}

/// Start an export of `comp_id` to `out_path` with the dialogue-shaped
/// `spec_json` (`preset`, `codec`, `size`, `bitrate_mbps`, `include_audio`,
/// `audio_bit_rate`). Returns `{ok:true}` on a clean start, or a calm `ok:false`
/// — "an export is already running" while one is in flight (the caller queues),
/// or a resolution/build error. The export runs on its own thread (K-017).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_start_export(
    comp_id: *const c_char,
    spec_json: *const c_char,
    out_path: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(spec), Some(out)) = (
        c_str_to_string(comp_id),
        c_str_to_string(spec_json),
        c_str_to_string(out_path),
    ) else {
        return to_c_string(err_json(
            "start export: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || crate::export::start_export(&comp, &spec, &out))
}

/// Poll the running export, draining its progress channel. Reply:
/// `{ok:true,state:"idle|running|done|failed",frame,total,encoder,path/error}`.
#[no_mangle]
pub extern "C" fn lumit_bridge_export_poll() -> *mut c_char {
    guard(crate::export::export_poll)
}

/// Ask the running export to cancel (a no-op when none is running).
#[no_mangle]
pub extern "C" fn lumit_bridge_export_cancel() -> *mut c_char {
    guard(crate::export::export_cancel)
}

/// Set the interpolation of the keyframe nearest the playhead `frame` on a
/// transform `property`. `interp_in`/`interp_out` are `Hold`/`Linear`/`Bezier`;
/// when a side is `Bezier` its `(speed, influence)` come from the matching pair
/// (`speed_in`/`influence_in`, `speed_out`/`influence_out`), otherwise ignored.
///
/// # Safety
/// The five string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_set_keyframe_interp(
    comp_id: *const c_char,
    layer_id: *const c_char,
    property: *const c_char,
    frame: i64,
    interp_in: *const c_char,
    interp_out: *const c_char,
    speed_in: f64,
    influence_in: f64,
    speed_out: f64,
    influence_out: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(property), Some(interp_in), Some(interp_out)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(property),
        c_str_to_string(interp_in),
        c_str_to_string(interp_out),
    ) else {
        return to_c_string(err_json(
            "set keyframe interp: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::edits::set_keyframe_interp(
                b,
                &comp,
                &layer,
                &property,
                frame,
                &interp_in,
                &interp_out,
                speed_in,
                influence_in,
                speed_out,
                influence_out,
            )
        })
    })
}

/// Enable or disable a footage layer's Retime (the Time stopwatch).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_retime_enabled(
    comp_id: *const c_char,
    layer_id: *const c_char,
    enabled: bool,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "set retime enabled: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::retime::set_retime_enabled(b, &comp, &layer, enabled)))
}

/// Set a footage layer's constant playback speed (percent; 100 clears the
/// Retime).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_retime_speed(
    comp_id: *const c_char,
    layer_id: *const c_char,
    speed_percent: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "set retime speed: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::retime::set_retime_speed(b, &comp, &layer, speed_percent)))
}

/// Set the ease of the Retime segment covering the playhead `frame` — the graph
/// header's Lin/Slow/Fast/Smth/Shrp row.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_segment_preset(
    comp_id: *const c_char,
    layer_id: *const c_char,
    frame: i64,
    ease: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(ease)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(ease),
    ) else {
        return to_c_string(err_json(
            "set segment preset: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::retime::set_segment_preset(b, &comp, &layer, frame, &ease))
    })
}

/// Convert the Map segment covering the playhead `frame` to a Rate segment — the
/// graph's →Rate button. The reply is the refreshed snapshot with an added
/// `drift` field (the fit's source-position error in seconds).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_segment_to_rate(
    comp_id: *const c_char,
    layer_id: *const c_char,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "segment to rate: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::retime::segment_to_rate(b, &comp, &layer, frame)))
}

/// Move the value-lens Retime boundary at `index` to comp `frame`.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_drag_boundary(
    comp_id: *const c_char,
    layer_id: *const c_char,
    index: i64,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "drag boundary: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::retime::drag_boundary(b, &comp, &layer, index, frame)))
}

/// The blend-mode registry as `{ok:true,blend_modes:[{name,label}]}` — stateless.
#[no_mangle]
pub extern "C" fn lumit_bridge_list_blend_modes() -> *mut c_char {
    guard(crate::columns::list_blend_modes)
}

/// Set a layer's blend mode (the serde variant name, e.g. `Normal`, `Multiply`).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_blend_mode(
    comp_id: *const c_char,
    layer_id: *const c_char,
    mode: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(mode)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(mode),
    ) else {
        return to_c_string(err_json(
            "set blend mode: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::columns::set_blend_mode(b, &comp, &layer, &mode)))
}

/// Point a layer at another as its matte, or clear it when `source` is empty.
/// `channel` is `alpha`/`luma`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_matte(
    comp_id: *const c_char,
    layer_id: *const c_char,
    source: *const c_char,
    channel: *const c_char,
    inverted: bool,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(source), Some(channel)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(source),
        c_str_to_string(channel),
    ) else {
        return to_c_string(err_json(
            "set matte: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::columns::set_matte(b, &comp, &layer, &source, &channel, inverted))
    })
}

/// Point a layer at another as its transform parent, or clear it when `parent`
/// is empty.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_parent(
    comp_id: *const c_char,
    layer_id: *const c_char,
    parent: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(parent)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(parent),
    ) else {
        return to_c_string(err_json(
            "set parent: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::columns::set_parent(b, &comp, &layer, &parent)))
}

/// Set the comp's motion-blur master (enable, shutter angle/phase, samples).
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string alive for the
/// call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_motion_blur(
    comp_id: *const c_char,
    enabled: bool,
    shutter_angle: f64,
    shutter_phase: f64,
    samples: u32,
) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "set motion blur: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::columns::set_motion_blur(
                b,
                &comp,
                enabled,
                shutter_angle,
                shutter_phase,
                samples,
            )
        })
    })
}

/// Add a starter mask shape (`rectangle`/`ellipse`/`star`) to a layer.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_mask(
    comp_id: *const c_char,
    layer_id: *const c_char,
    kind: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(kind)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(kind),
    ) else {
        return to_c_string(err_json(
            "add mask: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::columns::add_mask(b, &comp, &layer, &kind)))
}

// ---------------------------------------------------------------------------
// Bridge v0.5: the razor, beats, project-item and layer ops, Retime setters,
// asset properties, recovery, the boot log, the extra effect-param setters,
// effect reorder, and the linked-keyframe batch. Each guards its body, routes
// through the one shared bridge (or is stateless), and returns a JSON reply.
// ---------------------------------------------------------------------------

/// Razor: cut the Sequence layer's clip under the playhead `frame` into two.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_cut_clip_at_playhead(
    comp_id: *const c_char,
    layer_id: *const c_char,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "cut clip at playhead: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::sequence::cut_clip_at_playhead(b, &comp, &layer, frame)))
}

/// Razor: delete the Sequence layer's clip under the playhead `frame` (a gap).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_delete_clip_at_playhead(
    comp_id: *const c_char,
    layer_id: *const c_char,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "delete clip at playhead: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::sequence::delete_clip_at_playhead(b, &comp, &layer, frame))
    })
}

/// Detect beat markers for a composition from its audio (`sensitivity` 0..100).
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_detect_beats(
    comp_id: *const c_char,
    sensitivity: i64,
) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "detect beats: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::beats::detect_beats(b, &comp, sensitivity)))
}

/// Remove every detected Beat marker from a composition (keeping user/chapter).
///
/// # Safety
/// `comp_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_clear_beat_markers(comp_id: *const c_char) -> *mut c_char {
    let Some(comp) = c_str_to_string(comp_id) else {
        return to_c_string(err_json(
            "clear beat markers: the comp id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::beats::clear_beat_markers(b, &comp)))
}

/// Delete a project item.
///
/// # Safety
/// `item_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_delete_item(item_id: *const c_char) -> *mut c_char {
    let Some(item) = c_str_to_string(item_id) else {
        return to_c_string(err_json(
            "delete item: the item id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::items::delete_item(b, &item)))
}

/// Rename a project item.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_rename_item(
    item_id: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(item), Some(name)) = (c_str_to_string(item_id), c_str_to_string(name)) else {
        return to_c_string(err_json(
            "rename item: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::items::rename_item(b, &item, &name)))
}

/// Move a project item back to the panel root.
///
/// # Safety
/// `item_id` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_move_to_root(item_id: *const c_char) -> *mut c_char {
    let Some(item) = c_str_to_string(item_id) else {
        return to_c_string(err_json(
            "move to root: the item id was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::items::move_to_root(b, &item)))
}

/// Relink a missing footage item (and same-folder missing siblings) at `path`.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_relink(
    item_id: *const c_char,
    path: *const c_char,
) -> *mut c_char {
    let (Some(item), Some(path)) = (c_str_to_string(item_id), c_str_to_string(path)) else {
        return to_c_string(err_json("relink: an argument was null or not valid UTF-8"));
    };
    guard(move || with_bridge(|b| crate::items::relink(b, &item, &path)))
}

/// Rename a layer.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_rename_layer(
    comp_id: *const c_char,
    layer_id: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(name)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(name),
    ) else {
        return to_c_string(err_json(
            "rename layer: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::items::rename_layer(b, &comp, &layer, &name)))
}

/// Convert a footage layer into an editable Sequence layer (in place).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_convert_to_sequenced(
    comp_id: *const c_char,
    layer_id: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "convert to sequenced: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::items::convert_to_sequenced(b, &comp, &layer)))
}

/// Trim a retimed footage layer to where its source runs out.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_trim_to_source_end(
    comp_id: *const c_char,
    layer_id: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "trim to source end: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::items::trim_to_source_end(b, &comp, &layer)))
}

/// Set a footage layer's Retime reverse policy.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_retime_reverse(
    comp_id: *const c_char,
    layer_id: *const c_char,
    reverse: bool,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "set retime reverse: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::retime::set_retime_reverse(b, &comp, &layer, reverse)))
}

/// Set a footage layer's frame interpolation (`nearest`/`blend`/`flow`).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_retime_interpolation(
    comp_id: *const c_char,
    layer_id: *const c_char,
    interp: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(interp)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(interp),
    ) else {
        return to_c_string(err_json(
            "set retime interpolation: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::retime::set_retime_interpolation(b, &comp, &layer, &interp))
    })
}

/// Write a rotating autosave beside the project WITHOUT re-pointing the loaded
/// path. An empty `path` uses the loaded path; `keep` is the slot count.
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_autosave(path: *const c_char, keep: i64) -> *mut c_char {
    let path = c_str_to_string(path).unwrap_or_default();
    let keep = usize::try_from(keep).unwrap_or(1);
    guard(move || with_bridge(|b| crate::recovery::autosave(b, &path, keep)))
}

/// List the rotating autosaves beside a project (empty `path` = the loaded one).
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_list_autosaves(path: *const c_char) -> *mut c_char {
    let path = c_str_to_string(path).unwrap_or_default();
    guard(move || with_bridge(|b| crate::recovery::list_autosaves(b, &path)))
}

/// Open a project and replay its crash journal on top (empty `path` = loaded).
///
/// # Safety
/// `path` must be null or a valid NUL-terminated UTF-8 C string.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_restore_journal(path: *const c_char) -> *mut c_char {
    let path = c_str_to_string(path).unwrap_or_default();
    guard(move || with_bridge(|b| crate::recovery::restore_journal(b, &path)))
}

/// The engine's honest boot lines for the splash. Stateless.
#[no_mangle]
pub extern "C" fn lumit_bridge_boot_log() -> *mut c_char {
    guard(crate::recovery::boot_log)
}

/// Set a text layer's document (`text`, `size`, scene-linear RGBA `fill`).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_set_text_content(
    comp_id: *const c_char,
    layer_id: *const c_char,
    text: *const c_char,
    size: f64,
    r: f64,
    g: f64,
    b: f64,
    a: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(text)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(text),
    ) else {
        return to_c_string(err_json(
            "set text content: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|bridge| {
            crate::assets::set_text_content(bridge, &comp, &layer, &text, size, r, g, b, a)
        })
    })
}

/// Recolour and resize a solid layer's backing asset.
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_set_solid(
    comp_id: *const c_char,
    layer_id: *const c_char,
    r: f64,
    g: f64,
    b: f64,
    a: f64,
    width: u32,
    height: u32,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "set solid: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|bridge| {
            crate::assets::set_solid(bridge, &comp, &layer, r, g, b, a, width, height)
        })
    })
}

/// Set a camera layer's zoom (pixels, static).
///
/// # Safety
/// The two string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_camera_zoom(
    comp_id: *const c_char,
    layer_id: *const c_char,
    zoom: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer)) = (c_str_to_string(comp_id), c_str_to_string(layer_id)) else {
        return to_c_string(err_json(
            "set camera zoom: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::assets::set_camera_zoom(b, &comp, &layer, zoom)))
}

/// Set an enum (`Choice`) effect parameter to an option `index`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_choice(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    index: u32,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "set effect choice: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxparams::set_effect_param_choice(b, &comp, &layer, &effect, &param, index)
        })
    })
}

/// Set a `Bool` effect parameter.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_bool(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    value: bool,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "set effect bool: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxparams::set_effect_param_bool(b, &comp, &layer, &effect, &param, value)
        })
    })
}

/// Set a `Seed` effect parameter.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_seed(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    seed: u32,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "set effect seed: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxparams::set_effect_param_seed(b, &comp, &layer, &effect, &param, seed)
        })
    })
}

/// Set a `Point` effect parameter to a static `(x, y)`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_point(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    x: f64,
    y: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "set effect point: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxparams::set_effect_param_point(b, &comp, &layer, &effect, &param, x, y)
        })
    })
}

/// Reorder an effect within a layer's stack to `new_index` (clamped).
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_reorder_effect(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    new_index: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
    ) else {
        return to_c_string(err_json(
            "reorder effect: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::fxparams::reorder_effect(b, &comp, &layer, &effect, new_index))
    })
}

/// Apply several transform-keyframe edits as one undo step. `ops_json` is a JSON
/// array of `{property, action, frame, value?}` objects on one layer.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_apply_keyframe_batch(
    comp_id: *const c_char,
    layer_id: *const c_char,
    ops_json: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(ops)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(ops_json),
    ) else {
        return to_c_string(err_json(
            "apply keyframe batch: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::fxparams::apply_keyframe_batch(b, &comp, &layer, &ops)))
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

// ---------------------------------------------------------------------------
// Bridge v0.9: mask geometry, effect-param keyframe ops, effect presets, and
// the realtime preview-tier readout. Each guards its body and returns JSON.
// ---------------------------------------------------------------------------

/// Add a mask built from a drawn drag rect (`rectangle`/`ellipse`/`star`) at
/// `(x, y)` sized `w`×`h` in comp pixels — the geometry-carrying mask op.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_mask_geometry(
    comp_id: *const c_char,
    layer_id: *const c_char,
    kind: *const c_char,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(kind)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(kind),
    ) else {
        return to_c_string(err_json(
            "add mask geometry: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| crate::columns::add_mask_geometry(b, &comp, &layer, &kind, x, y, w, h))
    })
}

/// Effect-param stopwatch: toggle keyframing on `(effect, param, channel)` at
/// the playhead `frame`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_toggle_effect_param_animated(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    channel: i64,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "toggle effect keyframing: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxkeys::toggle_effect_param_animated(
                b, &comp, &layer, &effect, &param, channel, frame,
            )
        })
    })
}

/// Insert or replace an effect-param keyframe at the playhead `frame` with
/// `value`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_add_effect_param_keyframe(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    channel: i64,
    frame: i64,
    value: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "add effect keyframe: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxkeys::add_effect_param_keyframe(
                b, &comp, &layer, &effect, &param, channel, frame, value,
            )
        })
    })
}

/// Remove the effect-param keyframe at the playhead `frame`.
///
/// # Safety
/// The four string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_remove_effect_param_keyframe(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    channel: i64,
    frame: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
    ) else {
        return to_c_string(err_json(
            "remove effect keyframe: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxkeys::remove_effect_param_keyframe(
                b, &comp, &layer, &effect, &param, channel, frame,
            )
        })
    })
}

/// Slide the effect-param keyframes at comp `frames_json` (a JSON array of comp
/// frame indices) by `delta` frames.
///
/// # Safety
/// The five string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_shift_effect_param_keyframes(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    channel: i64,
    frames_json: *const c_char,
    delta: i64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param), Some(frames)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
        c_str_to_string(frames_json),
    ) else {
        return to_c_string(err_json(
            "shift effect keyframes: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxkeys::shift_effect_param_keyframes(
                b, &comp, &layer, &effect, &param, channel, &frames, delta,
            )
        })
    })
}

/// Set the interpolation of the effect-param keyframe nearest the playhead
/// `frame`. Each side names `Hold`/`Linear`/`Bezier`; a Bezier side reads its
/// `(speed, influence)` from the matching pair.
///
/// # Safety
/// The six string pointers must each be null or a valid NUL-terminated UTF-8 C
/// string alive for the call.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn lumit_bridge_set_effect_param_keyframe_interp(
    comp_id: *const c_char,
    layer_id: *const c_char,
    effect_id: *const c_char,
    param_name: *const c_char,
    channel: i64,
    frame: i64,
    interp_in: *const c_char,
    interp_out: *const c_char,
    speed_in: f64,
    influence_in: f64,
    speed_out: f64,
    influence_out: f64,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(effect), Some(param), Some(int_in), Some(int_out)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(effect_id),
        c_str_to_string(param_name),
        c_str_to_string(interp_in),
        c_str_to_string(interp_out),
    ) else {
        return to_c_string(err_json(
            "set effect keyframe interp: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || {
        with_bridge(|b| {
            crate::fxkeys::set_effect_param_keyframe_interp(
                b,
                &comp,
                &layer,
                &effect,
                &param,
                channel,
                frame,
                &int_in,
                &int_out,
                speed_in,
                influence_in,
                speed_out,
                influence_out,
            )
        })
    })
}

/// Serialise a layer's effect stack to a `.lumfx` preset JSON string (in the
/// reply's `preset` field) — the Dart side writes it to a file.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_save_effect_preset(
    comp_id: *const c_char,
    layer_id: *const c_char,
    name: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(name)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(name),
    ) else {
        return to_c_string(err_json(
            "save effect preset: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::preset::save_effect_preset(b, &comp, &layer, &name)))
}

/// Load a `.lumfx` preset (its JSON `text`, read from a file by Dart) onto a
/// layer, appending its effects with fresh ids as one undo step.
///
/// # Safety
/// The three string pointers must each be null or a valid NUL-terminated UTF-8
/// C string alive for the call.
#[no_mangle]
pub unsafe extern "C" fn lumit_bridge_load_effect_preset(
    comp_id: *const c_char,
    layer_id: *const c_char,
    text: *const c_char,
) -> *mut c_char {
    let (Some(comp), Some(layer), Some(text)) = (
        c_str_to_string(comp_id),
        c_str_to_string(layer_id),
        c_str_to_string(text),
    ) else {
        return to_c_string(err_json(
            "load effect preset: an argument was null or not valid UTF-8",
        ));
    };
    guard(move || with_bridge(|b| crate::preset::load_effect_preset(b, &comp, &layer, &text)))
}

/// The realtime preview tier currently in force: `{ok, tier, scale}` (tier
/// 1 = Full … 4 = Quarter; scale = 1/tier). Stateless read of the session
/// controller — the Viewer readout and Auto-mode scale source.
#[no_mangle]
pub extern "C" fn lumit_bridge_playback_tier() -> *mut c_char {
    guard(crate::realtime::playback_tier)
}

/// Reset the realtime tier controller to Full (called when playback stops, the
/// comp changes, or the user switches back to Auto). Returns the fresh tier.
#[no_mangle]
pub extern "C" fn lumit_bridge_reset_realtime() -> *mut c_char {
    guard(crate::realtime::reset_reply)
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
        assert_eq!(parse(&copied)["abi"], json!(9));
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
    fn render_scope_with_a_null_id_returns_null_and_zeroes_len() {
        let mut len: usize = 9;
        let ptr = unsafe {
            lumit_bridge_render_scope(
                0,
                ptr::null(),
                0,
                1.0,
                0x0a0b0c,
                0x86dd9a,
                0xe2555f,
                0x54cf6b,
                0x5387e0,
                &mut len,
            )
        };
        assert!(ptr.is_null());
        assert_eq!(len, 0);
    }

    #[test]
    fn render_scope_of_an_unknown_comp_is_null() {
        // A well-formed but unknown comp id: null (the document is empty here),
        // never a panic. On a GPU-less machine it is null via the renderer's
        // Failed state; either way the length is zeroed.
        let unknown = std::ffi::CString::new(uuid::Uuid::now_v7().to_string()).unwrap();
        let mut len: usize = 9;
        let ptr = unsafe {
            lumit_bridge_render_scope(
                0,
                unknown.as_ptr(),
                0,
                1.0,
                0x0a0b0c,
                0x86dd9a,
                0xe2555f,
                0x54cf6b,
                0x5387e0,
                &mut len,
            )
        };
        assert!(ptr.is_null());
        assert_eq!(len, 0);
    }

    #[test]
    fn unpack_rgb_splits_the_channels() {
        assert_eq!(unpack_rgb(0x0a0b0c), [0x0a, 0x0b, 0x0c]);
        assert_eq!(unpack_rgb(0xffffff), [0xff, 0xff, 0xff]);
        assert_eq!(unpack_rgb(0x000000), [0, 0, 0]);
    }

    #[test]
    fn shared_supported_matches_the_build() {
        // True only in the opt-in shared-texture build on Windows; false in the
        // default build every gate runs — never a panic either way.
        assert_eq!(
            lumit_bridge_shared_supported(),
            cfg!(all(windows, feature = "shared-texture"))
        );
    }

    #[test]
    fn render_to_shared_with_a_null_id_returns_false_and_zeroes_outs() {
        let mut handle: u64 = 7;
        let mut w: u32 = 7;
        let mut h: u32 = 7;
        let ok =
            unsafe { lumit_bridge_render_to_shared(ptr::null(), 0, &mut handle, &mut w, &mut h) };
        assert!(!ok);
        assert_eq!((handle, w, h), (0, 0, 0));
    }

    #[test]
    fn render_to_shared_of_an_unknown_comp_returns_false_and_zeroes_outs() {
        // Unknown comp in the empty global document, or no adapter — either way
        // `false` with the outs zeroed, the path Dart falls back from.
        let id = std::ffi::CString::new("018f0e9a-0000-7000-8000-0000000000bb").unwrap();
        let mut handle: u64 = 5;
        let mut w: u32 = 5;
        let mut h: u32 = 5;
        let ok =
            unsafe { lumit_bridge_render_to_shared(id.as_ptr(), 0, &mut handle, &mut w, &mut h) };
        assert!(!ok);
        assert_eq!((handle, w, h), (0, 0, 0));
    }

    #[test]
    fn cache_controls_report_stats_and_clear() {
        // clear_cache and cache_stats always return a well-formed stats reply,
        // and set_cache_budget round-trips the budget — in every build (the
        // cache is always compiled; only its population is render-gated).
        let cleared_ptr = lumit_bridge_clear_cache();
        let cleared = unsafe { CStr::from_ptr(cleared_ptr) }
            .to_str()
            .unwrap()
            .to_owned();
        unsafe { lumit_bridge_free_string(cleared_ptr) };
        let v = parse(&cleared);
        assert_eq!(v["ok"], json!(true));
        assert_eq!(v["used_bytes"], json!(0));

        let budget_ptr = lumit_bridge_set_cache_budget(64 * 1024 * 1024);
        let budget = unsafe { CStr::from_ptr(budget_ptr) }
            .to_str()
            .unwrap()
            .to_owned();
        unsafe { lumit_bridge_free_string(budget_ptr) };
        let v = parse(&budget);
        assert_eq!(v["budget_bytes"], json!(64 * 1024 * 1024));

        let stats_ptr = lumit_bridge_cache_stats();
        let stats = unsafe { CStr::from_ptr(stats_ptr) }
            .to_str()
            .unwrap()
            .to_owned();
        unsafe { lumit_bridge_free_string(stats_ptr) };
        assert_eq!(parse(&stats)["budget_bytes"], json!(64 * 1024 * 1024));

        // Restore a sane default so other tests are unaffected.
        let restore = lumit_bridge_set_cache_budget(512 * 1024 * 1024);
        unsafe { lumit_bridge_free_string(restore) };
    }

    #[test]
    fn render_cancel_stale_is_ok_and_freeable() {
        let ptr = lumit_bridge_render_cancel_stale(1);
        let reply = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_owned();
        unsafe { lumit_bridge_free_string(ptr) };
        assert_eq!(parse(&reply)["ok"], json!(true));
    }

    #[test]
    fn thumbnail_with_a_null_id_returns_null_and_zeroes_outs() {
        let mut w: u32 = 7;
        let mut h: u32 = 7;
        let mut len: usize = 7;
        let ptr = unsafe { lumit_bridge_thumbnail(ptr::null(), 128, &mut w, &mut h, &mut len) };
        assert!(ptr.is_null());
        assert_eq!((w, h, len), (0, 0, 0));
    }

    #[test]
    fn render_comp_frame_gen_with_a_null_id_returns_null_and_zeroes_outs() {
        let mut w: u32 = 9;
        let mut h: u32 = 9;
        let mut len: usize = 9;
        let ptr = unsafe {
            lumit_bridge_render_comp_frame_gen(ptr::null(), 0, 1.0, 1, &mut w, &mut h, &mut len)
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
