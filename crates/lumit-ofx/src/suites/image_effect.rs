//! `OfxImageEffectSuiteV1` — definition and instance both.
//!
//! # In plain terms
//!
//! Half of this suite is what a plugin uses while it is *describing itself*:
//! give me my own property bag, give me the bag my parameters go in, and here
//! is an image input I would like to have.
//!
//! The other half belongs to an **instance** — a live copy of the effect on a
//! layer, with values in its parameters and pictures to fetch. `clipGetImage`
//! is the one that matters: it is how a plugin actually gets pixels, and the
//! property set it answers with is the whole contract for a picture.
//!
//! Three things about that contract are worth saying out loud:
//!
//! * **The row bytes may be negative.** See [`crate::image`]; the host means
//!   it, and a plugin that assumes a positive stride writes its output
//!   upside-down.
//! * **The buffer is pinned until `clipReleaseImage`.** It belongs to the host
//!   for the whole of the render action that handed it over — the driver owns
//!   every picture and outlives every fetch of one — so a plugin holding the
//!   pointer for the duration of its render is holding something real.
//!   Releasing the same image twice is `kOfxStatErrBadHandle`, not a second
//!   free: the handle is struck off the first time and a struck-off handle is
//!   one of the things this host is built to survive.
//! * **There are pixels only during a render.** A plugin that squirrels a clip
//!   handle away and fetches an image between renders is told `kOfxStatFailed`,
//!   which is the truth: there is no frame to give it.

use std::ffi::{c_char, c_int, c_void};

use crate::describe::{ClipRef, EffectDescriptor};
use crate::ffi::{
    prop_keys as keys, prop_values as values, OfxImageClipHandle, OfxImageEffectHandle,
    OfxImageEffectSuiteV1, OfxImageMemoryHandle, OfxParamSetHandle, OfxPropertySetHandle, OfxRectD,
    OfxTime,
};
use crate::handles::{Handle, HandleKind};
use crate::host::state;
use crate::image::Image;
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
        state.effects.get_mut(handle)?.clips.push(ClipRef {
            name,
            props,
            handle: None,
        });
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
        let found = read(image_effect, |effect| {
            effect
                .clips
                .iter()
                .find(|clip| clip.name == name)
                .map(|found| (found.props, found.handle))
                .ok_or(Status::ErrUnknown)
        })?;
        let (props, clip_handle) = found;
        // A clip *handle* names a live image input, which only an instance has.
        // A descriptor's clip has none, and null is the honest answer there;
        // the property set is what a describing plugin is actually after.
        if !clip.is_null() {
            // SAFETY: the plugin's out-parameter, checked non-null.
            unsafe { *clip = clip_handle.map_or(std::ptr::null_mut(), Handle::as_ptr) };
        }
        // SAFETY: as above.
        unsafe { out_handle(property_set, props) }
    })
}

unsafe extern "C" fn clip_get_property_set(
    clip: OfxImageClipHandle,
    prop_handle: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        let handle = Handle::from_ptr(clip);
        if handle.kind() != Some(HandleKind::Clip) {
            return Err(Status::ErrBadHandle);
        }
        let props = state().clips.get(handle)?.props;
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(prop_handle, props) }
    })
}

/// The clip a handle names: which instance it belongs to, and which of its
/// clips it is.
fn clip_binding(clip: OfxImageClipHandle) -> Result<(Handle, String), Status> {
    let handle = Handle::from_ptr(clip);
    if handle.kind() != Some(HandleKind::Clip) {
        return Err(Status::ErrBadHandle);
    }
    let state = state();
    let binding = state.clips.get(handle)?;
    Ok((binding.effect, binding.name.clone()))
}

unsafe extern "C" fn clip_get_image(
    clip: OfxImageClipHandle,
    time: OfxTime,
    region: *const OfxRectD,
    image_handle: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        // The region a plugin may ask for is ignored, and honestly so: this
        // host says `kOfxImageEffectPropSupportsTiles` is nought, so the only
        // region there is is the whole image, and answering a smaller one
        // would be a tile by another name (docs/impl/ofx-host.md §2).
        let _ = region;
        let (effect, name) = clip_binding(clip)?;

        let props = {
            let state = state();
            let instance = state
                .effects
                .get(effect)?
                .instance
                .as_ref()
                .ok_or(Status::ErrBadHandle)?;
            // No render in flight means no picture. `kOfxStatFailed` is the
            // spec's "there is no image", which is different from an error.
            let image = instance.images.get(&name).ok_or(Status::Failed)?;
            image_property_set(image, time)
        };
        let handle = state().props.insert(props)?;
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(image_handle, handle) }
    })
}

/// The property set that *is* an image, as far as OFX is concerned.
fn image_property_set(image: &Image, time: OfxTime) -> PropertySet {
    let mut set = PropertySet::new();
    let seed_string = |set: &mut PropertySet, key: &str, value: &str| {
        if let Ok(value) = PropValue::string(value) {
            set.seed(key, value);
        }
    };
    seed_string(&mut set, keys::TYPE, values::TYPE_IMAGE);
    seed_string(&mut set, keys::PIXEL_DEPTH, values::BIT_DEPTH_FLOAT);
    seed_string(&mut set, keys::COMPONENTS, values::COMPONENT_RGBA);
    seed_string(
        &mut set,
        keys::PRE_MULTIPLICATION,
        values::IMAGE_PRE_MULTIPLIED,
    );
    seed_string(&mut set, keys::IMAGE_FIELD, values::IMAGE_FIELD_NONE);
    seed_string(
        &mut set,
        keys::IMAGE_UNIQUE_IDENTIFIER,
        &format!("{:#x}@{time}", image.data_pointer() as usize),
    );

    let bounds = image.bounds().as_array().to_vec();
    set.seed(keys::IMAGE_BOUNDS, PropValue::Int(bounds.clone()));
    set.seed(keys::IMAGE_REGION_OF_DEFINITION, PropValue::Int(bounds));
    // The sign is the whole point; see `crate::image`.
    set.seed(keys::IMAGE_ROW_BYTES, PropValue::int(image.row_bytes()));
    set.seed(keys::PIXEL_ASPECT_RATIO, PropValue::double(1.0));
    set.seed(keys::RENDER_SCALE, PropValue::Double(vec![1.0, 1.0]));
    set.seed(
        keys::IMAGE_DATA,
        PropValue::Pointer(vec![image.data_pointer() as usize]),
    );
    set
}

unsafe extern "C" fn clip_release_image(image_handle: OfxPropertySetHandle) -> c_int {
    guard(|| {
        let handle = Handle::from_ptr(image_handle);
        let mut state = state();
        // Only a set this host minted as an image may be released as one, and
        // only once: the second release finds the handle struck off and is a
        // status rather than anything worse.
        let is_image = state
            .props
            .get(handle)?
            .get_string(keys::TYPE, 0)
            .map(|kind| kind.to_bytes() == values::TYPE_IMAGE.as_bytes())
            .unwrap_or(false);
        if !is_image {
            return Err(Status::ErrBadHandle);
        }
        state.props.remove(handle)?;
        Ok(())
    })
}

unsafe extern "C" fn clip_get_region_of_definition(
    clip: OfxImageClipHandle,
    time: OfxTime,
    bounds: *mut OfxRectD,
) -> c_int {
    guard(|| {
        let _ = time;
        if bounds.is_null() {
            return Err(Status::ErrValue);
        }
        let (effect, name) = clip_binding(clip)?;
        let state = state();
        let instance = state
            .effects
            .get(effect)?
            .instance
            .as_ref()
            .ok_or(Status::ErrBadHandle)?;
        let rect = instance
            .images
            .get(&name)
            .ok_or(Status::Failed)?
            .bounds()
            .as_array();
        // SAFETY: the plugin's out-parameter, checked non-null above.
        unsafe {
            *bounds = OfxRectD {
                x1: f64::from(rect[0]),
                y1: f64::from(rect[1]),
                x2: f64::from(rect[2]),
                y2: f64::from(rect[3]),
            };
        }
        Ok(())
    })
}

/// `abort` answers a plain int, and **nought is the answer that lets work
/// continue**. A plugin polls it inside its own render loop; the host answers
/// from the epoch token the render was given, which is set for the duration of
/// the render action ([`crate::render`]) — so a scrub that lands mid-render
/// stops the plugin at its next poll rather than at the end of the frame
/// (docs/13 §6).
unsafe extern "C" fn abort(image_effect: OfxImageEffectHandle) -> c_int {
    let _ = image_effect;
    c_int::from(crate::render::render_is_cancelled())
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
