//! `OfxPropertySuiteV1` — the one every plugin uses for everything.

use std::ffi::{c_char, c_int, c_void, CString};
use std::mem::discriminant;

use crate::ffi::{OfxPropertySetHandle, OfxPropertySuiteV1};
use crate::handles::Handle;
use crate::host::state;
use crate::props::{Element, PropValue, PropertySet};
use crate::status::{Status, StatusResult};
use crate::suites::{cstr, guard};

/// The table handed out by `fetchSuite`.
pub static SUITE: OfxPropertySuiteV1 = OfxPropertySuiteV1 {
    prop_set_pointer,
    prop_set_string,
    prop_set_double,
    prop_set_int,
    prop_set_pointer_n,
    prop_set_string_n,
    prop_set_double_n,
    prop_set_int_n,
    prop_get_pointer,
    prop_get_string,
    prop_get_double,
    prop_get_int,
    prop_get_pointer_n,
    prop_get_string_n,
    prop_get_double_n,
    prop_get_int_n,
    prop_reset,
    prop_get_dimension,
};

/// Do something with the property set a handle names, for reading.
fn read<R>(
    handle: OfxPropertySetHandle,
    body: impl FnOnce(&PropertySet) -> Result<R, Status>,
) -> Result<R, Status> {
    let handle = Handle::from_ptr(handle);
    let state = state();
    body(state.props.get(handle)?)
}

/// The same, for writing.
fn write(
    handle: OfxPropertySetHandle,
    body: impl FnOnce(&mut PropertySet) -> StatusResult,
) -> StatusResult {
    let handle = Handle::from_ptr(handle);
    let mut state = state();
    body(state.props.get_mut(handle)?)
}

/// An index from the C API: never negative.
fn index_of(index: c_int) -> Result<usize, Status> {
    usize::try_from(index).map_err(|_| Status::ErrBadIndex)
}

/// A count from the C API: never negative.
fn count_of(count: c_int) -> Result<usize, Status> {
    usize::try_from(count).map_err(|_| Status::ErrValue)
}

/// Replace a whole property, refusing to change its type if it exists.
fn replace(set: &mut PropertySet, key: &str, value: PropValue) -> StatusResult {
    if let Ok(existing) = set.get(key) {
        if discriminant(existing) != discriminant(&value) {
            return Err(Status::ErrValue);
        }
    }
    set.set(key, value);
    Ok(())
}

unsafe extern "C" fn prop_set_pointer(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: *mut c_void,
) -> c_int {
    guard(|| {
        // SAFETY: an OFX string argument, checked for null inside `cstr`.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        write(properties, |set| {
            set.set_element(key, index, Element::Pointer(value as usize))
        })
    })
}

unsafe extern "C" fn prop_set_string(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: *const c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        // SAFETY: as above; the plugin's string is copied, not retained.
        let value = unsafe { cstr(value) }?;
        let value = CString::new(value).map_err(|_| Status::ErrValue)?;
        let index = index_of(index)?;
        write(properties, |set| {
            set.set_element(key, index, Element::String(value))
        })
    })
}

unsafe extern "C" fn prop_set_double(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: f64,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        write(properties, |set| {
            set.set_element(key, index, Element::Double(value))
        })
    })
}

unsafe extern "C" fn prop_set_int(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: c_int,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        write(properties, |set| {
            set.set_element(key, index, Element::Int(value))
        })
    })
}

/// Borrow `count` values the plugin passed as an array.
///
/// # Safety
///
/// `values` must point to `count` readable elements, which is the contract of
/// every `propSet*N` call.
unsafe fn input_slice<'a, T>(values: *const T, count: usize) -> Result<&'a [T], Status> {
    if count == 0 {
        return Ok(&[]);
    }
    if values.is_null() {
        return Err(Status::ErrValue);
    }
    // SAFETY: the caller's contract, plus the null and length checks above.
    Ok(unsafe { std::slice::from_raw_parts(values, count) })
}

/// The mutable counterpart, for `propGet*N`.
///
/// # Safety
///
/// As [`input_slice`], and the memory must be writable.
unsafe fn output_slice<'a, T>(values: *mut T, count: usize) -> Result<&'a mut [T], Status> {
    if count == 0 {
        return Ok(&mut []);
    }
    if values.is_null() {
        return Err(Status::ErrValue);
    }
    // SAFETY: the caller's contract, plus the null and length checks above.
    Ok(unsafe { std::slice::from_raw_parts_mut(values, count) })
}

unsafe extern "C" fn prop_set_pointer_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *const *mut c_void,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propSetPointerN` contract.
        let values = unsafe { input_slice(value, count) }?;
        let values = values.iter().map(|p| *p as usize).collect();
        write(properties, |set| {
            replace(set, key, PropValue::Pointer(values))
        })
    })
}

unsafe extern "C" fn prop_set_string_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *const *const c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propSetStringN` contract.
        let values = unsafe { input_slice(value, count) }?;
        let mut strings = Vec::with_capacity(values.len());
        for value in values {
            // SAFETY: each element is one of the C strings the plugin passed.
            let text = unsafe { cstr(*value) }?;
            strings.push(CString::new(text).map_err(|_| Status::ErrValue)?);
        }
        write(properties, |set| {
            replace(set, key, PropValue::String(strings))
        })
    })
}

unsafe extern "C" fn prop_set_double_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *const f64,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propSetDoubleN` contract.
        let values = unsafe { input_slice(value, count) }?.to_vec();
        write(properties, |set| {
            replace(set, key, PropValue::Double(values))
        })
    })
}

unsafe extern "C" fn prop_set_int_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *const c_int,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propSetIntN` contract.
        let values = unsafe { input_slice(value, count) }?.to_vec();
        write(properties, |set| replace(set, key, PropValue::Int(values)))
    })
}

unsafe extern "C" fn prop_get_pointer(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: *mut *mut c_void,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        if value.is_null() {
            return Err(Status::ErrValue);
        }
        let found = read(properties, |set| set.get_pointer(key, index))?;
        // SAFETY: `value` is the plugin's out-parameter, checked non-null.
        unsafe { *value = found as *mut c_void };
        Ok(())
    })
}

unsafe extern "C" fn prop_get_string(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        if value.is_null() {
            return Err(Status::ErrValue);
        }
        // The pointer handed back belongs to the host and stays valid until
        // that property is written again — the OFX contract for strings, and
        // the reason property sets own their strings rather than borrowing.
        let found = read(properties, |set| {
            Ok(set.get_string(key, index)?.as_ptr().cast_mut())
        })?;
        // SAFETY: `value` is the plugin's out-parameter, checked non-null.
        unsafe { *value = found };
        Ok(())
    })
}

unsafe extern "C" fn prop_get_double(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: *mut f64,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        if value.is_null() {
            return Err(Status::ErrValue);
        }
        let found = read(properties, |set| set.get_double(key, index))?;
        // SAFETY: `value` is the plugin's out-parameter, checked non-null.
        unsafe { *value = found };
        Ok(())
    })
}

unsafe extern "C" fn prop_get_int(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    index: c_int,
    value: *mut c_int,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let index = index_of(index)?;
        if value.is_null() {
            return Err(Status::ErrValue);
        }
        let found = read(properties, |set| set.get_int(key, index))?;
        // SAFETY: `value` is the plugin's out-parameter, checked non-null.
        unsafe { *value = found };
        Ok(())
    })
}

unsafe extern "C" fn prop_get_pointer_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *mut *mut c_void,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propGetPointerN` contract.
        let out = unsafe { output_slice(value, count) }?;
        let found = read(properties, |set| {
            (0..count)
                .map(|index| set.get_pointer(key, index))
                .collect::<Result<Vec<_>, _>>()
        })?;
        for (slot, found) in out.iter_mut().zip(found) {
            *slot = found as *mut c_void;
        }
        Ok(())
    })
}

unsafe extern "C" fn prop_get_string_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *mut *mut c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propGetStringN` contract.
        let out = unsafe { output_slice(value, count) }?;
        let found = read(properties, |set| {
            (0..count)
                .map(|index| Ok(set.get_string(key, index)?.as_ptr().cast_mut()))
                .collect::<Result<Vec<_>, Status>>()
        })?;
        for (slot, found) in out.iter_mut().zip(found) {
            *slot = found;
        }
        Ok(())
    })
}

unsafe extern "C" fn prop_get_double_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *mut f64,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propGetDoubleN` contract.
        let out = unsafe { output_slice(value, count) }?;
        let found = read(properties, |set| {
            (0..count)
                .map(|index| set.get_double(key, index))
                .collect::<Result<Vec<_>, _>>()
        })?;
        out.copy_from_slice(&found);
        Ok(())
    })
}

unsafe extern "C" fn prop_get_int_n(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: c_int,
    value: *mut c_int,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        let count = count_of(count)?;
        // SAFETY: the `propGetIntN` contract.
        let out = unsafe { output_slice(value, count) }?;
        let found = read(properties, |set| {
            (0..count)
                .map(|index| set.get_int(key, index))
                .collect::<Result<Vec<_>, _>>()
        })?;
        out.copy_from_slice(&found);
        Ok(())
    })
}

unsafe extern "C" fn prop_reset(
    properties: OfxPropertySetHandle,
    property: *const c_char,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        write(properties, |set| set.reset(key))
    })
}

unsafe extern "C" fn prop_get_dimension(
    properties: OfxPropertySetHandle,
    property: *const c_char,
    count: *mut c_int,
) -> c_int {
    guard(|| {
        // SAFETY: as above.
        let key = unsafe { cstr(property) }?;
        if count.is_null() {
            return Err(Status::ErrValue);
        }
        let dimension = read(properties, |set| set.dimension(key))?;
        let dimension = c_int::try_from(dimension).map_err(|_| Status::ErrValue)?;
        // SAFETY: `count` is the plugin's out-parameter, checked non-null.
        unsafe { *count = dimension };
        Ok(())
    })
}
