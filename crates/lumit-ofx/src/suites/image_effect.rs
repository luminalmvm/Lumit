//! `OfxImageEffectSuiteV1` — the definition half.
//!
//! # In plain terms
//!
//! This is the suite a plugin uses while it is *describing itself*: give me my
//! own property bag, give me the bag my parameters go in, and here is an image
//! input I would like to have. Three entry points out of thirteen.
//!
//! The other ten belong to an **instance** — a live copy of the effect on a
//! layer, with values in its parameters and pictures to fetch. No instance
//! exists in this package, so each of those answers `kOfxStatErrUnsupported`
//! rather than pretending. That is a code every plugin is required to expect,
//! and it is true: the feature is not here yet, it is not broken.

use std::ffi::{c_char, c_int, c_void};

use crate::describe::{ClipRef, EffectDescriptor};
use crate::ffi::{
    prop_keys as keys, prop_values as values, OfxImageClipHandle, OfxImageEffectHandle,
    OfxImageEffectSuiteV1, OfxImageMemoryHandle, OfxParamSetHandle, OfxPropertySetHandle, OfxRectD,
    OfxTime,
};
use crate::handles::{Handle, HandleKind};
use crate::host::state;
use crate::props::{PropValue, PropertySet};
use crate::status::Status;
use crate::suites::{cstr, guard, out_handle};

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxImageEffectSuiteV1 = OfxImageEffectSuiteV1 {
    get_property_set,
    get_param_set,
    clip_define,
    clip_get_handle,
    clip_get_property_set,
    clip_get_image,
    clip_release_image,
    clip_get_region_of_definition,
    abort,
    image_memory_alloc,
    image_memory_free,
    image_memory_lock,
    image_memory_unlock,
};

/// The descriptor a handle names, for reading.
fn read<R>(
    handle: OfxImageEffectHandle,
    body: impl FnOnce(&EffectDescriptor) -> Result<R, Status>,
) -> Result<R, Status> {
    let handle = Handle::from_ptr(handle);
    let state = state();
    body(state.effects.get(handle)?)
}

unsafe extern "C" fn get_property_set(
    image_effect: OfxImageEffectHandle,
    prop_handle: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        let props = read(image_effect, |effect| Ok(effect.props))?;
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(prop_handle, props) }
    })
}

unsafe extern "C" fn get_param_set(
    image_effect: OfxImageEffectHandle,
    param_set: *mut OfxParamSetHandle,
) -> c_int {
    guard(|| {
        // The effect must exist before its param set can be named — otherwise
        // a forged effect handle would come back as a plausible param set.
        read(image_effect, |_| Ok(()))?;
        let handle = Handle::from_ptr(image_effect)
            .recast(HandleKind::ParamSet)
            .ok_or(Status::ErrBadHandle)?;
        // SAFETY: as above.
        unsafe { out_handle(param_set, handle) }
    })
}

unsafe extern "C" fn clip_define(
    image_effect: OfxImageEffectHandle,
    name: *const c_char,
    property_set: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        // SAFETY: an OFX string argument, checked for null inside `cstr`.
        let name = unsafe { cstr(name) }?.to_owned();
        let handle = Handle::from_ptr(image_effect);

        let mut state = state();
        // Defining a clip twice is the plugin's mistake, and answering with the
        // first one is what every host does: the second `clipDefine` of
        // "Source" means the same clip, not a second one.
        if let Some(existing) = state
            .effects
            .get(handle)?
            .clips
            .iter()
            .find(|clip| clip.name == name)
            .map(|clip| clip.props)
        {
            // SAFETY: the plugin's out-parameter.
            return unsafe { out_handle(property_set, existing) };
        }

        let props = state.props.insert(clip_property_set(&name))?;
        state
            .effects
            .get_mut(handle)?
            .clips
            .push(ClipRef { name, props });
        // SAFETY: as above.
        unsafe { out_handle(property_set, props) }
    })
}

/// A clip descriptor's seeded properties: the same honest answers the host
/// table gives, because a clip cannot offer what the pipeline cannot carry
/// (docs/12 §2.1 — float RGBA and nothing else).
fn clip_property_set(name: &str) -> PropertySet {
    let mut set = PropertySet::new();
    if let Ok(value) = PropValue::string(values::TYPE_CLIP) {
        set.seed(keys::TYPE, value);
    }
    if let Ok(value) = PropValue::string(name) {
        set.seed(keys::NAME, value);
    }
    if let Ok(value) = PropValue::strings(&[values::COMPONENT_RGBA]) {
        set.seed(keys::SUPPORTED_COMPONENTS, value);
    }
    set.seed(keys::TEMPORAL_CLIP_ACCESS, PropValue::int(1));
    set.seed(keys::SUPPORTS_TILES, PropValue::int(0));
    set.seed(keys::CLIP_OPTIONAL, PropValue::int(0));
    set.seed(keys::CLIP_IS_MASK, PropValue::int(0));
    set
}

unsafe extern "C" fn clip_get_handle(
    image_effect: OfxImageEffectHandle,
    name: *const c_char,
    clip: *mut OfxImageClipHandle,
    property_set: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        // SAFETY: an OFX string argument.
        let name = unsafe { cstr(name) }?;
        let props = read(image_effect, |effect| {
            effect
                .clips
                .iter()
                .find(|clip| clip.name == name)
                .map(|clip| clip.props)
                .ok_or(Status::ErrUnknown)
        })?;
        // A clip *handle* names a live image input, which only an instance has;
        // the property set of the descriptor is what a describing plugin is
        // actually after, and it is answered honestly.
        if !clip.is_null() {
            // SAFETY: the plugin's out-parameter, checked non-null.
            unsafe { *clip = std::ptr::null_mut() };
        }
        // SAFETY: as above.
        unsafe { out_handle(property_set, props) }
    })
}

unsafe extern "C" fn clip_get_property_set(
    clip: OfxImageClipHandle,
    prop_handle: *mut OfxPropertySetHandle,
) -> c_int {
    let _ = (clip, prop_handle);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn clip_get_image(
    clip: OfxImageClipHandle,
    time: OfxTime,
    region: *const OfxRectD,
    image_handle: *mut OfxPropertySetHandle,
) -> c_int {
    let _ = (clip, time, region, image_handle);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn clip_release_image(image_handle: OfxPropertySetHandle) -> c_int {
    let _ = image_handle;
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn clip_get_region_of_definition(
    clip: OfxImageClipHandle,
    time: OfxTime,
    bounds: *mut OfxRectD,
) -> c_int {
    let _ = (clip, time, bounds);
    Status::ErrUnsupported.code()
}

/// `abort` answers a plain int, and **nought is the answer that lets work
/// continue**. Nothing is cancellable yet, so nothing is ever asked to stop —
/// but a plugin polling this in a render loop must never be told to abort by
/// an unimplemented entry point returning something arbitrary.
unsafe extern "C" fn abort(image_effect: OfxImageEffectHandle) -> c_int {
    let _ = image_effect;
    0
}

unsafe extern "C" fn image_memory_alloc(
    instance_handle: OfxImageEffectHandle,
    n_bytes: usize,
    memory_handle: *mut OfxImageMemoryHandle,
) -> c_int {
    let _ = (instance_handle, n_bytes, memory_handle);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn image_memory_free(memory_handle: OfxImageMemoryHandle) -> c_int {
    let _ = memory_handle;
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn image_memory_lock(
    memory_handle: OfxImageMemoryHandle,
    returned_ptr: *mut *mut c_void,
) -> c_int {
    let _ = (memory_handle, returned_ptr);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn image_memory_unlock(memory_handle: OfxImageMemoryHandle) -> c_int {
    let _ = memory_handle;
    Status::ErrUnsupported.code()
}
