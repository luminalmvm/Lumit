//! `OfxParameterSuiteV1` — the definition half.
//!
//! # In plain terms
//!
//! A plugin declares its controls by asking the host for one property bag per
//! parameter and then filling it in: this one is a number, its label is
//! "Amount", it defaults to 0.5, it runs from 0 to 1. That is `paramDefine`,
//! and with `paramGetHandle` and the two property-set lookups it is the whole
//! of what a describing plugin needs.
//!
//! **Reading a value is answered from the host, never from the plugin.**
//! `paramGetValue` and `paramGetValueAtTime` look the control up in the
//! instance's snapshot — what every parameter read at the moment this
//! evaluation was scheduled ([`crate::instance::ParamSnapshot`]) — because
//! Lumit owns parameter storage, animation and expressions (docs/12 §2.2). A
//! plugin has no store of its own to be asked for, which is exactly why a
//! render is reproducible from the document.
//!
//! Writing a value, keyframing, and the derivative and integral of an animated
//! control are still `kOfxStatErrUnsupported`: they need the property system
//! the host has not been wired to yet, and answering them with a guess would
//! be worse than answering them with the truth.
//!
//! **A parameter's type is fixed at definition and never afterwards.** The bag
//! it hands back is seeded with the type, the standard defaults for that type,
//! and nothing else; the plugin overwrites what it cares about through the
//! property suite, which refuses to change a property's type (`props.rs`).

use std::ffi::{c_char, c_int, c_uint, c_void};

use crate::describe::{ParamRecord, ParamRef};
use crate::ffi::{
    double_types, param_types, prop_keys as keys, prop_values as values, string_modes,
    OfxParamHandle, OfxParamSetHandle, OfxParameterSuiteV1, OfxPropertySetHandle, OfxRangeD,
    OfxTime,
};
use crate::handles::{Handle, HandleKind};
use crate::host::state;
use crate::props::{PropValue, PropertySet};
use crate::status::Status;
use crate::suites::{cstr, guard, out_handle};

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxParameterSuiteV1 = OfxParameterSuiteV1 {
    param_define,
    param_get_handle,
    param_set_get_property_set,
    param_get_property_set,
    param_get_value,
    param_get_value_at_time,
    param_get_derivative,
    param_get_integral,
    param_set_value,
    param_set_value_at_time,
    param_get_num_keys,
    param_get_key_time,
    param_get_key_index,
    param_delete_key,
    param_delete_all_keys,
    param_copy,
    param_edit_begin,
    param_edit_end,
};

/// The effect a param-set handle belongs to (see [`HandleKind::ParamSet`]).
fn effect_of(param_set: OfxParamSetHandle) -> Result<Handle, Status> {
    let handle = Handle::from_ptr(param_set);
    if handle.kind() != Some(HandleKind::ParamSet) {
        return Err(Status::ErrBadHandle);
    }
    handle
        .recast(HandleKind::ImageEffect)
        .ok_or(Status::ErrBadHandle)
}

unsafe extern "C" fn param_define(
    param_set: OfxParamSetHandle,
    param_type: *const c_char,
    name: *const c_char,
    property_set: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        // SAFETY: OFX string arguments, checked for null inside `cstr`.
        let param_type = unsafe { cstr(param_type) }?;
        // SAFETY: as above.
        let name = unsafe { cstr(name) }?.to_owned();
        if !param_types::ALL.contains(&param_type) {
            // A type this host has never heard of. `kOfxStatErrUnsupported` is
            // the spec's answer, and a plugin that meets it usually falls back
            // to a type that exists.
            return Err(Status::ErrUnsupported);
        }
        let effect = effect_of(param_set)?;

        let mut state = state();
        // Two parameters of one name is the plugin's error, and OFX names the
        // code for it. Letting the second through would give the effect two
        // rows under one id — a silent `ParamId` collision
        // (docs/impl/effect-registry.md §5), which is the failure that never
        // announces itself.
        if state
            .effects
            .get(effect)?
            .params
            .iter()
            .any(|param| param.name == name)
        {
            return Err(Status::ErrExists);
        }

        let props = state.props.insert(param_property_set(&name, param_type))?;
        let handle = state.params.insert(ParamRecord {
            props,
            effect,
            name: name.clone(),
        })?;
        state.effects.get_mut(effect)?.params.push(ParamRef {
            name,
            param_type: param_type.to_owned(),
            handle,
            props,
        });
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(property_set, props) }
    })
}

/// The properties a freshly defined parameter starts with: its type, its
/// identity, and the spec's own defaults for the kind of thing it is.
///
/// Seeding the numeric properties matters more than it looks. A plugin that
/// sets only a maximum leaves the minimum for the host to answer, and a host
/// that answers "no such property" there has plugins that describe half a
/// control. So every numeric type arrives with a full set, at the values the
/// OFX spec names, and the plugin overwrites what it means to.
fn param_property_set(name: &str, param_type: &str) -> PropertySet {
    /// A literal with a NUL in it is not a thing this file contains; the
    /// property is simply not seeded and the golden test catches it.
    fn seed_string(set: &mut PropertySet, key: &str, value: &str) {
        if let Ok(value) = PropValue::string(value) {
            set.seed(key, value);
        }
    }

    let mut set = PropertySet::new();
    seed_string(&mut set, keys::TYPE, values::TYPE_PARAMETER);
    seed_string(&mut set, keys::NAME, name);
    seed_string(&mut set, keys::PARAM_TYPE, param_type);
    // The label a plugin never sets is its name, which is what every host
    // draws in that case.
    seed_string(&mut set, keys::LABEL, name);
    seed_string(&mut set, keys::PARAM_PARENT, "");
    seed_string(&mut set, keys::PARAM_HINT, "");

    let dimensions = match param_type {
        param_types::DOUBLE_2D | param_types::INTEGER_2D => 2,
        param_types::DOUBLE_3D | param_types::INTEGER_3D => 3,
        param_types::RGB => 3,
        param_types::RGBA => 4,
        _ => 1,
    };

    match param_type {
        // The spec's own defaults: unbounded, which is `±DBL_MAX` and is read
        // back as "no hard bound" (`schema.rs`), with a nought-to-one display
        // range for the slider.
        param_types::DOUBLE | param_types::DOUBLE_2D | param_types::DOUBLE_3D => {
            set.seed(
                keys::PARAM_DEFAULT,
                PropValue::Double(vec![0.0; dimensions]),
            );
            set.seed(
                keys::PARAM_MIN,
                PropValue::Double(vec![-f64::MAX; dimensions]),
            );
            set.seed(
                keys::PARAM_MAX,
                PropValue::Double(vec![f64::MAX; dimensions]),
            );
            set.seed(
                keys::PARAM_DISPLAY_MIN,
                PropValue::Double(vec![0.0; dimensions]),
            );
            set.seed(
                keys::PARAM_DISPLAY_MAX,
                PropValue::Double(vec![1.0; dimensions]),
            );
            seed_string(&mut set, keys::PARAM_DOUBLE_TYPE, double_types::PLAIN);
        }
        // A colour channel is nought to one until the plugin says otherwise.
        param_types::RGB | param_types::RGBA => {
            let mut default = vec![0.0; dimensions];
            if param_type == param_types::RGBA {
                if let Some(alpha) = default.last_mut() {
                    *alpha = 1.0;
                }
            }
            set.seed(keys::PARAM_DEFAULT, PropValue::Double(default));
            set.seed(keys::PARAM_MIN, PropValue::Double(vec![0.0; dimensions]));
            set.seed(keys::PARAM_MAX, PropValue::Double(vec![1.0; dimensions]));
            set.seed(
                keys::PARAM_DISPLAY_MIN,
                PropValue::Double(vec![0.0; dimensions]),
            );
            set.seed(
                keys::PARAM_DISPLAY_MAX,
                PropValue::Double(vec![1.0; dimensions]),
            );
        }
        param_types::INTEGER | param_types::INTEGER_2D | param_types::INTEGER_3D => {
            set.seed(keys::PARAM_DEFAULT, PropValue::Int(vec![0; dimensions]));
            set.seed(keys::PARAM_MIN, PropValue::Int(vec![i32::MIN; dimensions]));
            set.seed(keys::PARAM_MAX, PropValue::Int(vec![i32::MAX; dimensions]));
            set.seed(keys::PARAM_DISPLAY_MIN, PropValue::Int(vec![0; dimensions]));
            set.seed(
                keys::PARAM_DISPLAY_MAX,
                PropValue::Int(vec![100; dimensions]),
            );
        }
        param_types::BOOLEAN | param_types::CHOICE => {
            set.seed(keys::PARAM_DEFAULT, PropValue::int(0));
            if param_type == param_types::CHOICE {
                set.seed(keys::PARAM_CHOICE_OPTION, PropValue::String(Vec::new()));
            }
        }
        param_types::STRING | param_types::CUSTOM => {
            seed_string(&mut set, keys::PARAM_DEFAULT, "");
            seed_string(&mut set, keys::PARAM_STRING_MODE, string_modes::SINGLE_LINE);
        }
        param_types::GROUP => {
            set.seed(keys::PARAM_GROUP_OPEN, PropValue::int(1));
        }
        param_types::PAGE => {
            set.seed(keys::PARAM_PAGE_CHILD, PropValue::String(Vec::new()));
        }
        // A push button has no value and a parametric parameter's curve is its
        // own affair; neither carries a default.
        _ => {}
    }

    // Whether a parameter animates is the plugin's to say, and P3's to honour.
    set.seed(
        keys::PARAM_ANIMATES,
        PropValue::int(i32::from(!matches!(
            param_type,
            param_types::GROUP | param_types::PAGE | param_types::PUSH_BUTTON
        ))),
    );
    set
}

unsafe extern "C" fn param_get_handle(
    param_set: OfxParamSetHandle,
    name: *const c_char,
    param: *mut OfxParamHandle,
    property_set: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        // SAFETY: an OFX string argument.
        let name = unsafe { cstr(name) }?;
        let effect = effect_of(param_set)?;
        let state = state();
        let found = state
            .effects
            .get(effect)?
            .params
            .iter()
            .find(|defined| defined.name == name)
            .ok_or(Status::ErrUnknown)?;
        let (handle, props) = (found.handle, found.props);
        drop(state);
        if !property_set.is_null() {
            // SAFETY: the plugin's out-parameter, checked non-null.
            unsafe { *property_set = props.as_ptr() };
        }
        // SAFETY: as above; `param` is the one the caller must have.
        unsafe { out_handle(param, handle) }
    })
}

unsafe extern "C" fn param_set_get_property_set(
    param_set: OfxParamSetHandle,
    prop_handle: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        let effect = effect_of(param_set)?;
        let props = state().effects.get(effect)?.props;
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(prop_handle, props) }
    })
}

unsafe extern "C" fn param_get_property_set(
    param: OfxParamHandle,
    prop_handle: *mut OfxPropertySetHandle,
) -> c_int {
    guard(|| {
        let props = state().params.get(Handle::from_ptr(param))?.props;
        // SAFETY: the plugin's out-parameter, checked non-null inside.
        unsafe { out_handle(prop_handle, props) }
    })
}

// ---------------------------------------------------------- reading values --

/// Write one parameter's value into the plugin's out-parameters.
///
/// The four slots are the widest a standard parameter can be — RGBA — and only
/// as many as the value has dimensions are written. See
/// [`crate::ffi::OfxParameterSuiteV1`] for why the arity is fixed rather than
/// variadic, and what that costs.
///
/// # Safety
///
/// Each of `slots[0..dimension]` must be null or point at writable storage of
/// the type the parameter declares, which is the contract of every OFX
/// `paramGetValue` call.
unsafe fn write_value(param: OfxParamHandle, slots: [*mut c_void; 4]) -> Result<(), Status> {
    let handle = Handle::from_ptr(param);

    let state = state();
    let record = state.params.get(handle)?;
    let param_type = state
        .props
        .get(record.props)?
        .get_string(keys::PARAM_TYPE, 0)?
        .to_string_lossy()
        .into_owned();
    // A push button has no value at all, and inventing one for it would have a
    // plugin read a number where the spec says there is nothing to read.
    if param_type == param_types::PUSH_BUTTON {
        return Err(Status::ErrUnsupported);
    }

    let instance = state
        .effects
        .get(record.effect)?
        .instance
        .as_ref()
        // A descriptor's parameters have no values: nothing has been evaluated.
        .ok_or(Status::ErrUnsupported)?;
    let value = instance
        .params
        .get(&record.name)
        .ok_or(Status::ErrUnknown)?;

    for (index, slot) in slots.into_iter().enumerate() {
        if slot.is_null() {
            continue;
        }
        match value {
            PropValue::Int(values) => {
                let Some(&found) = values.get(index) else {
                    break;
                };
                // SAFETY: the caller's contract — an integer parameter's slots
                // are `int*`.
                unsafe { *slot.cast::<c_int>() = found };
            }
            PropValue::Double(values) => {
                let Some(&found) = values.get(index) else {
                    break;
                };
                // SAFETY: the caller's contract — a double parameter's slots
                // are `double*`.
                unsafe { *slot.cast::<f64>() = found };
            }
            PropValue::String(values) => {
                let Some(found) = values.get(index) else {
                    break;
                };
                // The pointer belongs to the host and stays valid until the
                // value is next written — the same contract `propGetString`
                // gives, and the reason the snapshot owns its strings.
                // SAFETY: the caller's contract — a string parameter's slot is
                // `char**`.
                unsafe { *slot.cast::<*const c_char>() = found.as_ptr() };
            }
            // A custom parameter's blob is stored and round-tripped, never
            // interpreted (docs/12 §2.2), so there is nothing to hand back
            // through a typed pointer.
            PropValue::Pointer(_) => return Err(Status::ErrUnsupported),
        }
    }
    Ok(())
}

unsafe extern "C" fn param_get_value(
    param: OfxParamHandle,
    v0: *mut c_void,
    v1: *mut c_void,
    v2: *mut c_void,
    v3: *mut c_void,
) -> c_int {
    // SAFETY: the plugin's out-parameters, as OFX declares them.
    guard(|| unsafe { write_value(param, [v0, v1, v2, v3]) })
}

unsafe extern "C" fn param_get_value_at_time(
    param: OfxParamHandle,
    time: OfxTime,
    v0: *mut c_void,
    v1: *mut c_void,
    v2: *mut c_void,
    v3: *mut c_void,
) -> c_int {
    // The snapshot *is* the answer at a time: it was taken at the time this
    // evaluation was scheduled, with every curve and expression already
    // resolved (docs/12 §2.2). A plugin asking for another time — a retimer
    // reading its own control ahead — gets the same value, which is right for
    // a control that does not animate and is the ceiling for one that does.
    // Lifting it needs the property system on the far side of the bridge and
    // the frame prefetch `getFramesNeeded` already plans; until then the honest
    // thing is to answer from the snapshot rather than from nothing.
    let _ = time;
    // SAFETY: as `param_get_value`.
    guard(|| unsafe { write_value(param, [v0, v1, v2, v3]) })
}

unsafe extern "C" fn param_get_derivative(param: OfxParamHandle, time: OfxTime) -> c_int {
    let _ = (param, time);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_get_integral(
    param: OfxParamHandle,
    time1: OfxTime,
    time2: OfxTime,
) -> c_int {
    let _ = (param, time1, time2);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_set_value(param: OfxParamHandle) -> c_int {
    let _ = param;
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_set_value_at_time(param: OfxParamHandle, time: OfxTime) -> c_int {
    let _ = (param, time);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_get_num_keys(
    param: OfxParamHandle,
    number_of_keys: *mut c_uint,
) -> c_int {
    let _ = (param, number_of_keys);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_get_key_time(
    param: OfxParamHandle,
    nth_key: c_uint,
    time: *mut OfxTime,
) -> c_int {
    let _ = (param, nth_key, time);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_get_key_index(
    param: OfxParamHandle,
    time: OfxTime,
    direction: c_int,
    index: *mut c_int,
) -> c_int {
    let _ = (param, time, direction, index);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_delete_key(param: OfxParamHandle, time: OfxTime) -> c_int {
    let _ = (param, time);
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_delete_all_keys(param: OfxParamHandle) -> c_int {
    let _ = param;
    Status::ErrUnsupported.code()
}

unsafe extern "C" fn param_copy(
    param_to: OfxParamHandle,
    param_from: OfxParamHandle,
    dst_offset: OfxTime,
    frame_range: *const OfxRangeD,
) -> c_int {
    let _ = (param_to, param_from, dst_offset, frame_range);
    Status::ErrUnsupported.code()
}

/// An edit block is an undo grouping. Lumit's undo is the host's own
/// (docs/12 §1: the host owns parameter storage), and a describing plugin has
/// nothing to undo, so both ends succeed and record nothing. Answering an error
/// here would have plugins that wrap every change in one give up on the change.
unsafe extern "C" fn param_edit_begin(param_set: OfxParamSetHandle, name: *const c_char) -> c_int {
    let _ = (param_set, name);
    Status::Ok.code()
}

unsafe extern "C" fn param_edit_end(param_set: OfxParamSetHandle) -> c_int {
    let _ = param_set;
    Status::Ok.code()
}
