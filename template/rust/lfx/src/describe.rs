//! The describe sink, one method per declaration kind.
//!
//! # In plain terms
//!
//! At describe an effect pushes typed records into a sink the host owns. There
//! is no key, no string-valued answer and no question the host can ask that a
//! plugin can answer in the wrong type - which is the one design mistake LFX
//! most deliberately avoids. This module is that sink with Rust's spelling:
//! the `#[repr(C)]` record is filled in here so an author writes one line per
//! control.
//!
//! Every method answers whether the host took the declaration. A `false` is
//! the graceful refusal of **that one row**: a kind this version does not
//! admit, or a declaration the host cannot represent. The effect still loads,
//! the row keeps the default declared here, and each refusal is a line in the
//! host's scan report - so carrying on declaring is right, and giving up at
//! the first `false` is not.
//!
//! **Two faults are not that, and the answer does not distinguish them.** A
//! duplicate id and an unset unit are structural and refuse the whole effect:
//! two rows hashing to one parameter id would ship one control silently
//! driving another, and a unit cannot be guessed. Carrying on after one costs
//! nothing; it is simply not enough to save the effect.
//!
//! # Thread role and contract
//!
//! **Control thread only**, and only from inside
//! [`crate::Effect::describe`] - the sink is valid for that call and no
//! longer.

use std::ffi::{c_char, CStr};

use crate::{sys, Unit};

/// The host's declaration sink.
pub struct Describe<'a> {
    sink: &'a mut sys::LfxDescribeSink,
}

/// Fill in one declaration record, push it, and answer what the host said.
///
/// Every kind's method is the same four steps - find the hook, build the
/// record with this build's own `size_of`, call it, answer - so they are
/// written once here rather than thirteen times below.
macro_rules! push {
    ($self:ident, $hook:ident, $record:expr) => {{
        match $self.sink.$hook {
            None => false,
            Some(declare) => {
                let record = $record;
                let sink: *mut sys::LfxDescribeSink = $self.sink;
                // SAFETY: the hook is the host's own, called on the control
                // thread from inside `describe`, with the sink it belongs to
                // and a record that lives until the call returns.
                unsafe { declare(sink, &raw const record) }
            }
        }
    }};
}

impl<'a> Describe<'a> {
    /// Wrap a sink the host handed over.
    pub(crate) fn new(sink: &'a mut sys::LfxDescribeSink) -> Self {
        Self { sink }
    }

    /// Whether the sink carries every hook this build declares through.
    ///
    /// A host that offered a shorter table would be one built against an
    /// earlier header, and a hook that is not there cannot be called.
    pub(crate) fn complete(&self) -> bool {
        self.sink.declare_float.is_some()
            && self.sink.declare_slider.is_some()
            && self.sink.declare_int.is_some()
            && self.sink.declare_angle.is_some()
            && self.sink.declare_bool.is_some()
            && self.sink.declare_choice.is_some()
            && self.sink.declare_colour.is_some()
            && self.sink.declare_seed.is_some()
            && self.sink.declare_point2.is_some()
            && self.sink.declare_point3.is_some()
            && self.sink.declare_curve.is_some()
            && self.sink.declare_file.is_some()
            && self.sink.declare_action.is_some()
            && self.sink.group_begin.is_some()
            && self.sink.group_end.is_some()
    }

    /// An unbounded number with a slider's travel. Typing may exceed the
    /// travel, which is what makes this kind a Float rather than a Slider.
    pub fn number(
        &mut self,
        id: &CStr,
        label: &CStr,
        unit: Unit,
        default: f64,
        low: f64,
        high: f64,
    ) -> bool {
        push!(
            self,
            declare_float,
            sys::LfxFloatParam {
                struct_size: size_of::<sys::LfxFloatParam>() as u32,
                unit: unit as u32,
                flags: sys::LFX_PARAM_FLAG_NONE,
                bounds: sys::LFX_BOUND_NONE,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_value: default,
                slider_min: low,
                slider_max: high,
                hard_min: 0.0,
                hard_max: 0.0,
            }
        )
    }

    /// A **bounded** number: the range is the control's whole nature, so there
    /// is no soft travel and hard bound to keep apart.
    pub fn slider(
        &mut self,
        id: &CStr,
        label: &CStr,
        unit: Unit,
        default: f64,
        low: f64,
        high: f64,
    ) -> bool {
        push!(
            self,
            declare_slider,
            sys::LfxSliderParam {
                struct_size: size_of::<sys::LfxSliderParam>() as u32,
                unit: unit as u32,
                flags: sys::LFX_PARAM_FLAG_NONE,
                log: 0,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_value: default,
                range_min: low,
                range_max: high,
            }
        )
    }

    /// A bounded number whose thumb moves logarithmically, so a 20 Hz to
    /// 20 kHz row spends half its travel below 1 kHz. Honest only above nought.
    pub fn log_slider(
        &mut self,
        id: &CStr,
        label: &CStr,
        unit: Unit,
        default: f64,
        low: f64,
        high: f64,
    ) -> bool {
        push!(
            self,
            declare_slider,
            sys::LfxSliderParam {
                struct_size: size_of::<sys::LfxSliderParam>() as u32,
                unit: unit as u32,
                flags: sys::LFX_PARAM_FLAG_NONE,
                log: 1,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_value: default,
                range_min: low,
                range_max: high,
            }
        )
    }

    /// A whole number. It animates and serialises exactly as a number does;
    /// the kind tells the panel to step it and the host to round it.
    pub fn whole(
        &mut self,
        id: &CStr,
        label: &CStr,
        unit: Unit,
        default: i64,
        low: i64,
        high: i64,
    ) -> bool {
        push!(
            self,
            declare_int,
            sys::LfxIntParam {
                struct_size: size_of::<sys::LfxIntParam>() as u32,
                unit: unit as u32,
                flags: sys::LFX_PARAM_FLAG_NONE,
                bounds: sys::LFX_BOUND_NONE,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_value: default,
                slider_min: low,
                slider_max: high,
                hard_min: 0,
                hard_max: 0,
            }
        )
    }

    /// An angle in degrees, drawn as a dial and deliberately unbounded: an
    /// angle animates through full turns rather than stopping at 360.
    pub fn angle(&mut self, id: &CStr, label: &CStr, default: f64, dial_step: f64) -> bool {
        push!(
            self,
            declare_angle,
            sys::LfxAngleParam {
                struct_size: size_of::<sys::LfxAngleParam>() as u32,
                unit: sys::LFX_UNIT_DEGREES,
                flags: sys::LFX_PARAM_FLAG_NONE,
                reserved_0: 0,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_value: default,
                dial_step,
            }
        )
    }

    /// A switch.
    pub fn flag(&mut self, id: &CStr, label: &CStr, default: bool) -> bool {
        push!(
            self,
            declare_bool,
            sys::LfxBoolParam {
                struct_size: size_of::<sys::LfxBoolParam>() as u32,
                unit: sys::LFX_UNIT_RAW,
                flags: sys::LFX_PARAM_FLAG_NONE,
                default_value: u32::from(default),
                id: id.as_ptr(),
                label: label.as_ptr(),
            }
        )
    }

    /// A dropdown. The dividers are **declared** rather than guessed from the
    /// labels: each entry is the index after which the list draws a rule.
    pub fn choice(
        &mut self,
        id: &CStr,
        label: &CStr,
        default: u32,
        options: &[&CStr],
        dividers_after: &[u32],
    ) -> bool {
        // The array of pointers the record points at. It lives until this
        // method returns, which is longer than the call the host reads it in.
        let pointers: Vec<*const c_char> = options.iter().map(|text| text.as_ptr()).collect();
        push!(
            self,
            declare_choice,
            sys::LfxChoiceParam {
                struct_size: size_of::<sys::LfxChoiceParam>() as u32,
                unit: sys::LFX_UNIT_RAW,
                flags: sys::LFX_PARAM_FLAG_NONE,
                default_index: default,
                option_count: pointers.len() as u32,
                divider_count: dividers_after.len() as u32,
                id: id.as_ptr(),
                label: label.as_ptr(),
                options: pointers.as_ptr(),
                dividers_after: dividers_after.as_ptr(),
            }
        )
    }

    /// Scene-linear RGBA. The range is declared per colour because a linear
    /// value may exceed one (an HDR tint) or dip below nought (a lift).
    pub fn colour(
        &mut self,
        id: &CStr,
        label: &CStr,
        default: [f64; 4],
        low: f64,
        high: f64,
    ) -> bool {
        push!(
            self,
            declare_colour,
            sys::LfxColourParam {
                struct_size: size_of::<sys::LfxColourParam>() as u32,
                unit: sys::LFX_UNIT_RAW,
                flags: sys::LFX_PARAM_FLAG_NONE,
                reserved_0: 0,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_rgba: default,
                range_min: low,
                range_max: high,
            }
        )
    }

    /// A point: two rows the panel folds back into one crosshair, and **one**
    /// element of the value array.
    pub fn point(
        &mut self,
        id: &CStr,
        label: &CStr,
        unit: Unit,
        default: (f64, f64),
        low: f64,
        high: f64,
    ) -> bool {
        push!(
            self,
            declare_point2,
            sys::LfxPoint2Param {
                struct_size: size_of::<sys::LfxPoint2Param>() as u32,
                unit: unit as u32,
                flags: sys::LFX_PARAM_FLAG_NONE,
                reserved_0: 0,
                id: id.as_ptr(),
                label: label.as_ptr(),
                default_x: default.0,
                default_y: default.1,
                slider_min: low,
                slider_max: high,
            }
        )
    }

    /// The randomness a seeded effect follows. There is deliberately **no**
    /// declared default: the host draws one from the fresh instance's own id,
    /// so two copies of a seeded effect never wobble in sync.
    pub fn seed(&mut self, id: &CStr, label: &CStr) -> bool {
        push!(
            self,
            declare_seed,
            sys::LfxSeedParam {
                struct_size: size_of::<sys::LfxSeedParam>() as u32,
                unit: sys::LFX_UNIT_RAW,
                flags: sys::LFX_PARAM_FLAG_NONE,
                reserved_0: 0,
                id: id.as_ptr(),
                label: label.as_ptr(),
            }
        )
    }

    /// A heading over the rows declared until the matching [`Self::group_end`].
    /// It is a run, not a row: nothing is stored for it and nothing animates.
    pub fn group_begin(&mut self, id: &CStr, label: &CStr) -> bool {
        push!(
            self,
            group_begin,
            sys::LfxGroupParam {
                struct_size: size_of::<sys::LfxGroupParam>() as u32,
                flags: sys::LFX_PARAM_FLAG_NONE,
                id: id.as_ptr(),
                label: label.as_ptr(),
            }
        )
    }

    /// Close the run [`Self::group_begin`] opened.
    pub fn group_end(&mut self) -> bool {
        match self.sink.group_end {
            None => false,
            Some(end) => {
                let sink: *mut sys::LfxDescribeSink = self.sink;
                // SAFETY: the host's own hook, called on the control thread
                // from inside `describe`, with the sink it belongs to.
                unsafe { end(sink) }
            }
        }
    }
}
