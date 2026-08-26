//! Property sets: the string-keyed bags every OFX object is made of.
//!
//! # In plain terms
//!
//! OpenFX has almost no structs. Instead, everything an object knows is a
//! named value in a bag — "what components do you support", "what is your
//! label", "how many frames do you need". Every value is an array (a single
//! value is an array of one), and every value has exactly one type: whole
//! numbers, decimals, text, or an address.
//!
//! The one rule worth stating out loud: if a plugin asks for a decimal
//! property as a whole number, it gets `kOfxStatErrValue`, never a quietly
//! converted answer. Type confusion in a property bag is how a host ends up
//! reading a pointer as an integer, and a converted answer hides the plugin's
//! bug until it is a crash somewhere else.

use std::collections::BTreeMap;
use std::ffi::CString;

use crate::status::Status;

/// One property's value. Always an array; a scalar is an array of one.
#[derive(Clone, Debug, PartialEq)]
pub enum PropValue {
    /// `int`.
    Int(Vec<i32>),
    /// `double`.
    Double(Vec<f64>),
    /// A C string. Held owned, because the pointer we hand back has to stay
    /// valid until the property is next written.
    String(Vec<CString>),
    /// An address. Held as an integer, not as a pointer: it is the plugin's
    /// pointer, never ours to follow, and keeping it as a number is what lets
    /// a property set move between threads.
    Pointer(Vec<usize>),
}

impl PropValue {
    /// The number of elements — what `propGetDimension` answers.
    #[must_use]
    pub fn dimension(&self) -> usize {
        match self {
            Self::Int(v) => v.len(),
            Self::Double(v) => v.len(),
            Self::String(v) => v.len(),
            Self::Pointer(v) => v.len(),
        }
    }

    /// An empty value of the same type, for `propReset` on a property that
    /// has no default.
    #[must_use]
    fn emptied(&self) -> Self {
        match self {
            Self::Int(_) => Self::Int(Vec::new()),
            Self::Double(_) => Self::Double(Vec::new()),
            Self::String(_) => Self::String(Vec::new()),
            Self::Pointer(_) => Self::Pointer(Vec::new()),
        }
    }

    /// A single whole number, for the common case.
    #[must_use]
    pub fn int(value: i32) -> Self {
        Self::Int(vec![value])
    }

    /// A single decimal.
    #[must_use]
    pub fn double(value: f64) -> Self {
        Self::Double(vec![value])
    }

    /// A single string.
    ///
    /// # Errors
    ///
    /// [`Status::ErrValue`] if the text contains a NUL, which cannot cross a
    /// C boundary.
    pub fn string(value: &str) -> Result<Self, Status> {
        Ok(Self::String(vec![
            CString::new(value).map_err(|_| Status::ErrValue)?
        ]))
    }

    /// A list of strings.
    ///
    /// # Errors
    ///
    /// As [`PropValue::string`].
    pub fn strings(values: &[&str]) -> Result<Self, Status> {
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            out.push(CString::new(*value).map_err(|_| Status::ErrValue)?);
        }
        Ok(Self::String(out))
    }
}

/// One entry: what it is now, and what `propReset` should put back.
#[derive(Clone, Debug)]
struct Prop {
    value: PropValue,
    default: Option<PropValue>,
}

/// A property set.
///
/// Keys are ordered rather than hashed, so a dump of a set is the same on
/// every run and on every machine — which is what makes the host table
/// golden-testable (docs/14 §10.7).
#[derive(Clone, Debug, Default)]
pub struct PropertySet {
    props: BTreeMap<String, Prop>,
}

impl PropertySet {
    /// An empty set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a property **and** make that its default, which is what host-owned
    /// properties want: the value we seeded is the value `propReset` restores.
    pub fn seed(&mut self, key: &str, value: PropValue) {
        self.props.insert(
            key.to_owned(),
            Prop {
                default: Some(value.clone()),
                value,
            },
        );
    }

    /// Replace a property's whole value, leaving any default alone.
    pub fn set(&mut self, key: &str, value: PropValue) {
        match self.props.get_mut(key) {
            Some(prop) => prop.value = value,
            None => {
                self.props.insert(
                    key.to_owned(),
                    Prop {
                        value,
                        default: None,
                    },
                );
            }
        }
    }

    /// The raw value.
    ///
    /// # Errors
    ///
    /// [`Status::ErrUnknown`] if there is no such property, which is the code
    /// OFX uses for a name the object does not have.
    pub fn get(&self, key: &str) -> Result<&PropValue, Status> {
        self.props
            .get(key)
            .map(|prop| &prop.value)
            .ok_or(Status::ErrUnknown)
    }

    /// Whether the set has this property at all.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.props.contains_key(key)
    }

    /// The number of elements in a property.
    ///
    /// # Errors
    ///
    /// As [`PropertySet::get`].
    pub fn dimension(&self, key: &str) -> Result<usize, Status> {
        Ok(self.get(key)?.dimension())
    }

    /// Put a property back to its default, or to empty if it never had one.
    ///
    /// # Errors
    ///
    /// As [`PropertySet::get`].
    pub fn reset(&mut self, key: &str) -> Result<(), Status> {
        let prop = self.props.get_mut(key).ok_or(Status::ErrUnknown)?;
        prop.value = match &prop.default {
            Some(default) => default.clone(),
            None => prop.value.emptied(),
        };
        Ok(())
    }

    /// One whole number.
    ///
    /// # Errors
    ///
    /// [`Status::ErrUnknown`] if absent, [`Status::ErrValue`] if the property
    /// is of another type, [`Status::ErrBadIndex`] if the index is past the
    /// end.
    pub fn get_int(&self, key: &str, index: usize) -> Result<i32, Status> {
        match self.get(key)? {
            PropValue::Int(values) => values.get(index).copied().ok_or(Status::ErrBadIndex),
            _ => Err(Status::ErrValue),
        }
    }

    /// One decimal. Errors as [`PropertySet::get_int`].
    ///
    /// # Errors
    ///
    /// As [`PropertySet::get_int`].
    pub fn get_double(&self, key: &str, index: usize) -> Result<f64, Status> {
        match self.get(key)? {
            PropValue::Double(values) => values.get(index).copied().ok_or(Status::ErrBadIndex),
            _ => Err(Status::ErrValue),
        }
    }

    /// One string, borrowed from the set. The borrow is what the C API hands
    /// back as a pointer, so it stays valid exactly as long as the property is
    /// not written again.
    ///
    /// # Errors
    ///
    /// As [`PropertySet::get_int`].
    pub fn get_string(&self, key: &str, index: usize) -> Result<&CString, Status> {
        match self.get(key)? {
            PropValue::String(values) => values.get(index).ok_or(Status::ErrBadIndex),
            _ => Err(Status::ErrValue),
        }
    }

    /// One address, as an integer.
    ///
    /// # Errors
    ///
    /// As [`PropertySet::get_int`].
    pub fn get_pointer(&self, key: &str, index: usize) -> Result<usize, Status> {
        match self.get(key)? {
            PropValue::Pointer(values) => values.get(index).copied().ok_or(Status::ErrBadIndex),
            _ => Err(Status::ErrValue),
        }
    }

    /// Write one element.
    ///
    /// Writing at the end extends the array by one, which is how a plugin
    /// builds up a list; writing further out is [`Status::ErrBadIndex`].
    /// Writing at a different type than the property already has is
    /// [`Status::ErrValue`] — a property does not change type once it exists.
    ///
    /// # Errors
    ///
    /// As above.
    pub fn set_element(&mut self, key: &str, index: usize, element: Element) -> Result<(), Status> {
        let Some(prop) = self.props.get_mut(key) else {
            // A property that does not exist yet is created, but only at
            // index nought: there is no array to be the second element of.
            if index != 0 {
                return Err(Status::ErrBadIndex);
            }
            self.set(key, element.into_singleton());
            return Ok(());
        };
        set_one(&mut prop.value, index, element)
    }

    /// Every key, in order, for the golden test and for diagnostics.
    #[must_use]
    pub fn keys(&self) -> Vec<&str> {
        self.props.keys().map(String::as_str).collect()
    }
}

/// One value to write, at one type.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    /// A whole number.
    Int(i32),
    /// A decimal.
    Double(f64),
    /// Text.
    String(CString),
    /// An address, as an integer.
    Pointer(usize),
}

impl Element {
    fn into_singleton(self) -> PropValue {
        match self {
            Self::Int(v) => PropValue::Int(vec![v]),
            Self::Double(v) => PropValue::Double(vec![v]),
            Self::String(v) => PropValue::String(vec![v]),
            Self::Pointer(v) => PropValue::Pointer(vec![v]),
        }
    }
}

/// Write one element into an existing value, keeping its type.
fn set_one(value: &mut PropValue, index: usize, element: Element) -> Result<(), Status> {
    /// Replace at `index`, or append when `index` is exactly the length.
    fn place<T>(values: &mut Vec<T>, index: usize, element: T) -> Result<(), Status> {
        let length = values.len();
        if index == length {
            values.push(element);
            return Ok(());
        }
        let slot = values.get_mut(index).ok_or(Status::ErrBadIndex)?;
        *slot = element;
        Ok(())
    }

    match (value, element) {
        (PropValue::Int(values), Element::Int(v)) => place(values, index, v),
        (PropValue::Double(values), Element::Double(v)) => place(values, index, v),
        (PropValue::String(values), Element::String(v)) => place(values, index, v),
        (PropValue::Pointer(values), Element::Pointer(v)) => place(values, index, v),
        _ => Err(Status::ErrValue),
    }
}
