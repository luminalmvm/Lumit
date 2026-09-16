//! The Rust half of the layout suite: the frozen ABI, asserted by number.
//!
//! # In plain terms
//!
//! A frozen ABI is only frozen if something checks. Every struct's size and
//! every field's offset is written out here as a literal, and `tests/layout.c`
//! writes the same literals against the header itself with `sizeof` and
//! `offsetof`. A field that moved, grew, or changed places fails one side or
//! the other, so the header and its Rust mirror cannot drift apart quietly.
//!
//! Offsets are only half of the drift. An enumerator renumbered in one half, an
//! extension id misspelt in one half, and a field whose type changed to another
//! of the same width all move nothing, so the C half also emits the header's own
//! constants and strings - compared here one by one, in a fixed order - and
//! *writes* each struct of plain data from the header's own declarations, which
//! this side reads back through the mirror. A `float` read as a `u32` and an
//! `int32_t` read as a `u32` then fail on the value.
//!
//! Four of the ABI's structs are not data. `LfxDescribeSink`, `LfxEntry`,
//! `LfxPlugin` and `LfxHost` are tables of function pointers, and an offset says
//! where a pointer sits and nothing about its arity, its argument order, its
//! argument types or what it answers - swapping `LfxEntry::create`'s two
//! arguments moves no number at all. So the C half defines one real instance of
//! each from the header's own declarations and this side **calls** them through
//! the mirror's own `Option<unsafe extern "C" fn ...>` types, which is the only
//! thing that pins a signature.
//!
//! Two more things the offsets cannot reach have tests of their own: a host
//! writing an **oversized `value_stride`** is read correctly by a plugin built
//! against the smaller `lfx_value`, and a **short or zeroed trait block** reads
//! as the pessimistic case rather than the optimistic one.
//!
//! The numbers are for a 64-bit target, which is every desktop Lumit ships
//! (docs/05-ARCHITECTURE.md); the first assertion says so, so a 32-bit build
//! fails on the pointer size rather than on a hundred confusing offsets.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::mem::{offset_of, size_of};

use lumit_lfx_abi::*;

extern "C" {
    /// Defined by `tests/layout.c`, compiled by `build.rs`. It answers the
    /// header's own `LFX_ABI_VERSION`.
    fn lfx_abi_layout_assertions_compiled() -> u32;

    /// How many constants the header emits, and the array itself: every
    /// enumerator and every ceiling, in the header's own order.
    fn lfx_abi_constant_count() -> u32;
    fn lfx_abi_constants() -> *const u32;

    /// The header's own spelling of every string the ABI names, by index.
    fn lfx_abi_string_count() -> u32;
    fn lfx_abi_string(index: u32) -> *const std::ffi::c_char;

    /// Each of these fills a block of zeroes from the header's own
    /// declarations, so the read back through the mirror is a test of the field
    /// *types* rather than of where they sit.
    fn lfx_abi_write_traits(out: *mut LfxTraits);
    fn lfx_abi_write_float_param(out: *mut LfxFloatParam);
    fn lfx_abi_write_slider_param(out: *mut LfxSliderParam);
    fn lfx_abi_write_int_param(out: *mut LfxIntParam);
    fn lfx_abi_write_angle_param(out: *mut LfxAngleParam);
    fn lfx_abi_write_bool_param(out: *mut LfxBoolParam);
    fn lfx_abi_write_choice_param(out: *mut LfxChoiceParam);
    fn lfx_abi_write_colour_param(out: *mut LfxColourParam);
    fn lfx_abi_write_seed_param(out: *mut LfxSeedParam);
    fn lfx_abi_write_point2_param(out: *mut LfxPoint2Param);
    fn lfx_abi_write_point3_param(out: *mut LfxPoint3Param);
    fn lfx_abi_write_curve_param(out: *mut LfxCurveParam);
    fn lfx_abi_write_file_param(out: *mut LfxFileParam);
    fn lfx_abi_write_action_param(out: *mut LfxActionParam);
    fn lfx_abi_write_group_param(out: *mut LfxGroupParam);
    fn lfx_abi_write_descriptor(out: *mut LfxDescriptor);
    fn lfx_abi_write_frame(out: *mut LfxFrame);
    fn lfx_abi_write_process(out: *mut LfxProcess);
    fn lfx_abi_write_value(out: *mut LfxValue, kind: u32);

    /// One real instance of each of the ABI's four function-pointer tables,
    /// built from the header's own declarations. They are called through the
    /// mirror's types rather than read, because a signature is not an offset.
    fn lfx_abi_layout_sink() -> *mut LfxDescribeSink;
    fn lfx_abi_layout_entry() -> *const LfxEntry;
    fn lfx_abi_layout_plugin() -> *mut LfxPlugin;
    fn lfx_abi_layout_host() -> *const LfxHost;
    /// What those tables' `get_extension` answers when it has what was asked
    /// for: a real address, so the answer is compared rather than merely
    /// counted.
    fn lfx_abi_layout_extension() -> *const std::ffi::c_char;

    /// What the tables above recorded of the last call made to them. Every
    /// callback records rather than asserting, so a mismatch fails here, where
    /// it can name the field.
    fn lfx_abi_layout_forget();
    fn lfx_abi_recorded_calls() -> u32;
    fn lfx_abi_recorded_struct_size() -> u32;
    fn lfx_abi_recorded_sink_size() -> u32;
    fn lfx_abi_recorded_unit() -> u32;
    fn lfx_abi_recorded_whole_u32() -> u32;
    fn lfx_abi_recorded_number() -> f64;
    fn lfx_abi_recorded_whole() -> i64;
    fn lfx_abi_recorded_text() -> *const std::ffi::c_char;
}

/// Everything the C half wrote down about the last call it was handed.
#[derive(Debug)]
struct Recorded {
    calls: u32,
    struct_size: u32,
    sink_size: u32,
    unit: u32,
    whole_u32: u32,
    number: f64,
    whole: i64,
    text: Option<String>,
}

/// Empty the slots, so each call is read on its own.
fn forget() {
    // SAFETY: `lfx_abi_layout_forget` is `tests/layout.c`'s, takes nothing and
    // writes only its own file statics.
    unsafe { lfx_abi_layout_forget() }
}

/// Read them back.
fn recorded() -> Recorded {
    // SAFETY: each getter is `tests/layout.c`'s, takes nothing and answers a
    // scalar or a pointer to a literal that lives for the length of the
    // process.
    unsafe {
        let text = lfx_abi_recorded_text();
        Recorded {
            calls: lfx_abi_recorded_calls(),
            struct_size: lfx_abi_recorded_struct_size(),
            sink_size: lfx_abi_recorded_sink_size(),
            unit: lfx_abi_recorded_unit(),
            whole_u32: lfx_abi_recorded_whole_u32(),
            number: lfx_abi_recorded_number(),
            whole: lfx_abi_recorded_whole(),
            text: if text.is_null() {
                None
            } else {
                Some(
                    std::ffi::CStr::from_ptr(text)
                        .to_string_lossy()
                        .into_owned(),
                )
            },
        }
    }
}

/// A block of zeroes the header's own code fills in, read back through the
/// mirror. This is the plugin's read, spelled out.
///
/// # Safety
///
/// `fill` must be one of `tests/layout.c`'s writers for `T`, each of which sets
/// every field of the struct it was declared with. Every field of every `T` here
/// is a number, a raw pointer or an optional function pointer, so the zeroed
/// block is a valid value before the call as well as after it.
unsafe fn written_by_the_header<T>(fill: unsafe extern "C" fn(*mut T)) -> T {
    let mut block = std::mem::MaybeUninit::<T>::zeroed();
    fill(block.as_mut_ptr());
    block.assume_init()
}

#[test]
fn every_struct_is_laid_out_as_the_header_lays_it_out() {
    assert_eq!(size_of::<*const std::ffi::c_void>(), 8);
    assert_eq!(size_of::<bool>(), 1);

    // The traits block: nine four-byte fields and no padding anywhere, which is
    // what makes "a zeroed block" and "a short block" the same declaration.
    assert_eq!(size_of::<LfxTraits>(), 36);
    assert_eq!(offset_of!(LfxTraits, struct_size), 0);
    assert_eq!(offset_of!(LfxTraits, cost), 4);
    assert_eq!(offset_of!(LfxTraits, roi_kind), 8);
    assert_eq!(offset_of!(LfxTraits, roi_padding_px), 12);
    assert_eq!(offset_of!(LfxTraits, temporal_lo), 16);
    assert_eq!(offset_of!(LfxTraits, temporal_hi), 20);
    assert_eq!(offset_of!(LfxTraits, alpha), 24);
    assert_eq!(offset_of!(LfxTraits, flags), 28);
    assert_eq!(offset_of!(LfxTraits, scratch_bytes_per_megapixel), 32);

    // The parameter records. Each opens with the same four words - size, unit,
    // flags, and one kind-specific `u32` - so the pointers land at 16 and 24
    // and the payload starts at 32 in every one of them.
    assert_eq!(size_of::<LfxFloatParam>(), 72);
    assert_eq!(offset_of!(LfxFloatParam, struct_size), 0);
    assert_eq!(offset_of!(LfxFloatParam, unit), 4);
    assert_eq!(offset_of!(LfxFloatParam, flags), 8);
    assert_eq!(offset_of!(LfxFloatParam, bounds), 12);
    assert_eq!(offset_of!(LfxFloatParam, id), 16);
    assert_eq!(offset_of!(LfxFloatParam, label), 24);
    assert_eq!(offset_of!(LfxFloatParam, default_value), 32);
    assert_eq!(offset_of!(LfxFloatParam, slider_min), 40);
    assert_eq!(offset_of!(LfxFloatParam, slider_max), 48);
    assert_eq!(offset_of!(LfxFloatParam, hard_min), 56);
    assert_eq!(offset_of!(LfxFloatParam, hard_max), 64);

    assert_eq!(size_of::<LfxSliderParam>(), 56);
    assert_eq!(offset_of!(LfxSliderParam, struct_size), 0);
    assert_eq!(offset_of!(LfxSliderParam, unit), 4);
    assert_eq!(offset_of!(LfxSliderParam, flags), 8);
    assert_eq!(offset_of!(LfxSliderParam, log), 12);
    assert_eq!(offset_of!(LfxSliderParam, id), 16);
    assert_eq!(offset_of!(LfxSliderParam, label), 24);
    assert_eq!(offset_of!(LfxSliderParam, default_value), 32);
    assert_eq!(offset_of!(LfxSliderParam, range_min), 40);
    assert_eq!(offset_of!(LfxSliderParam, range_max), 48);

    assert_eq!(size_of::<LfxIntParam>(), 72);
    assert_eq!(offset_of!(LfxIntParam, struct_size), 0);
    assert_eq!(offset_of!(LfxIntParam, unit), 4);
    assert_eq!(offset_of!(LfxIntParam, flags), 8);
    assert_eq!(offset_of!(LfxIntParam, bounds), 12);
    assert_eq!(offset_of!(LfxIntParam, id), 16);
    assert_eq!(offset_of!(LfxIntParam, label), 24);
    assert_eq!(offset_of!(LfxIntParam, default_value), 32);
    assert_eq!(offset_of!(LfxIntParam, slider_min), 40);
    assert_eq!(offset_of!(LfxIntParam, slider_max), 48);
    assert_eq!(offset_of!(LfxIntParam, hard_min), 56);
    assert_eq!(offset_of!(LfxIntParam, hard_max), 64);

    assert_eq!(size_of::<LfxAngleParam>(), 48);
    assert_eq!(offset_of!(LfxAngleParam, struct_size), 0);
    assert_eq!(offset_of!(LfxAngleParam, unit), 4);
    assert_eq!(offset_of!(LfxAngleParam, flags), 8);
    assert_eq!(offset_of!(LfxAngleParam, reserved_0), 12);
    assert_eq!(offset_of!(LfxAngleParam, id), 16);
    assert_eq!(offset_of!(LfxAngleParam, label), 24);
    assert_eq!(offset_of!(LfxAngleParam, default_value), 32);
    assert_eq!(offset_of!(LfxAngleParam, dial_step), 40);

    assert_eq!(size_of::<LfxBoolParam>(), 32);
    assert_eq!(offset_of!(LfxBoolParam, struct_size), 0);
    assert_eq!(offset_of!(LfxBoolParam, unit), 4);
    assert_eq!(offset_of!(LfxBoolParam, flags), 8);
    assert_eq!(offset_of!(LfxBoolParam, default_value), 12);
    assert_eq!(offset_of!(LfxBoolParam, id), 16);
    assert_eq!(offset_of!(LfxBoolParam, label), 24);

    assert_eq!(size_of::<LfxChoiceParam>(), 56);
    assert_eq!(offset_of!(LfxChoiceParam, struct_size), 0);
    assert_eq!(offset_of!(LfxChoiceParam, unit), 4);
    assert_eq!(offset_of!(LfxChoiceParam, flags), 8);
    assert_eq!(offset_of!(LfxChoiceParam, default_index), 12);
    assert_eq!(offset_of!(LfxChoiceParam, option_count), 16);
    assert_eq!(offset_of!(LfxChoiceParam, divider_count), 20);
    assert_eq!(offset_of!(LfxChoiceParam, id), 24);
    assert_eq!(offset_of!(LfxChoiceParam, label), 32);
    assert_eq!(offset_of!(LfxChoiceParam, options), 40);
    assert_eq!(offset_of!(LfxChoiceParam, dividers_after), 48);

    assert_eq!(size_of::<LfxColourParam>(), 80);
    assert_eq!(offset_of!(LfxColourParam, struct_size), 0);
    assert_eq!(offset_of!(LfxColourParam, unit), 4);
    assert_eq!(offset_of!(LfxColourParam, flags), 8);
    assert_eq!(offset_of!(LfxColourParam, reserved_0), 12);
    assert_eq!(offset_of!(LfxColourParam, id), 16);
    assert_eq!(offset_of!(LfxColourParam, label), 24);
    assert_eq!(offset_of!(LfxColourParam, default_rgba), 32);
    assert_eq!(offset_of!(LfxColourParam, range_min), 64);
    assert_eq!(offset_of!(LfxColourParam, range_max), 72);

    assert_eq!(size_of::<LfxSeedParam>(), 32);
    assert_eq!(offset_of!(LfxSeedParam, struct_size), 0);
    assert_eq!(offset_of!(LfxSeedParam, unit), 4);
    assert_eq!(offset_of!(LfxSeedParam, flags), 8);
    assert_eq!(offset_of!(LfxSeedParam, reserved_0), 12);
    assert_eq!(offset_of!(LfxSeedParam, id), 16);
    assert_eq!(offset_of!(LfxSeedParam, label), 24);

    assert_eq!(size_of::<LfxPoint2Param>(), 64);
    assert_eq!(offset_of!(LfxPoint2Param, struct_size), 0);
    assert_eq!(offset_of!(LfxPoint2Param, unit), 4);
    assert_eq!(offset_of!(LfxPoint2Param, flags), 8);
    assert_eq!(offset_of!(LfxPoint2Param, reserved_0), 12);
    assert_eq!(offset_of!(LfxPoint2Param, id), 16);
    assert_eq!(offset_of!(LfxPoint2Param, label), 24);
    assert_eq!(offset_of!(LfxPoint2Param, default_x), 32);
    assert_eq!(offset_of!(LfxPoint2Param, default_y), 40);
    assert_eq!(offset_of!(LfxPoint2Param, slider_min), 48);
    assert_eq!(offset_of!(LfxPoint2Param, slider_max), 56);

    assert_eq!(size_of::<LfxPoint3Param>(), 72);
    assert_eq!(offset_of!(LfxPoint3Param, struct_size), 0);
    assert_eq!(offset_of!(LfxPoint3Param, unit), 4);
    assert_eq!(offset_of!(LfxPoint3Param, flags), 8);
    assert_eq!(offset_of!(LfxPoint3Param, reserved_0), 12);
    assert_eq!(offset_of!(LfxPoint3Param, id), 16);
    assert_eq!(offset_of!(LfxPoint3Param, label), 24);
    assert_eq!(offset_of!(LfxPoint3Param, default_x), 32);
    assert_eq!(offset_of!(LfxPoint3Param, default_y), 40);
    assert_eq!(offset_of!(LfxPoint3Param, default_z), 48);
    assert_eq!(offset_of!(LfxPoint3Param, slider_min), 56);
    assert_eq!(offset_of!(LfxPoint3Param, slider_max), 64);

    assert_eq!(size_of::<LfxCurveParam>(), 40);
    assert_eq!(offset_of!(LfxCurveParam, struct_size), 0);
    assert_eq!(offset_of!(LfxCurveParam, unit), 4);
    assert_eq!(offset_of!(LfxCurveParam, flags), 8);
    assert_eq!(offset_of!(LfxCurveParam, point_count), 12);
    assert_eq!(offset_of!(LfxCurveParam, id), 16);
    assert_eq!(offset_of!(LfxCurveParam, label), 24);
    assert_eq!(offset_of!(LfxCurveParam, points), 32);

    assert_eq!(size_of::<LfxFileParam>(), 48);
    assert_eq!(offset_of!(LfxFileParam, struct_size), 0);
    assert_eq!(offset_of!(LfxFileParam, unit), 4);
    assert_eq!(offset_of!(LfxFileParam, flags), 8);
    assert_eq!(offset_of!(LfxFileParam, filter_count), 12);
    assert_eq!(offset_of!(LfxFileParam, id), 16);
    assert_eq!(offset_of!(LfxFileParam, label), 24);
    assert_eq!(offset_of!(LfxFileParam, filter), 32);
    assert_eq!(offset_of!(LfxFileParam, filter_name), 40);

    assert_eq!(size_of::<LfxActionParam>(), 32);
    assert_eq!(offset_of!(LfxActionParam, struct_size), 0);
    assert_eq!(offset_of!(LfxActionParam, unit), 4);
    assert_eq!(offset_of!(LfxActionParam, flags), 8);
    assert_eq!(offset_of!(LfxActionParam, reserved_0), 12);
    assert_eq!(offset_of!(LfxActionParam, id), 16);
    assert_eq!(offset_of!(LfxActionParam, label), 24);

    assert_eq!(size_of::<LfxGroupParam>(), 24);
    assert_eq!(offset_of!(LfxGroupParam, struct_size), 0);
    assert_eq!(offset_of!(LfxGroupParam, flags), 4);
    assert_eq!(offset_of!(LfxGroupParam, id), 8);
    assert_eq!(offset_of!(LfxGroupParam, label), 16);

    // The sink: a size, an opaque pointer, and fifteen function pointers in the
    // order the header declares them. The order is the ABI, so the last one's
    // offset is the whole assertion.
    assert_eq!(size_of::<LfxDescribeSink>(), 136);
    assert_eq!(offset_of!(LfxDescribeSink, struct_size), 0);
    assert_eq!(offset_of!(LfxDescribeSink, sink_data), 8);
    assert_eq!(offset_of!(LfxDescribeSink, declare_float), 16);
    assert_eq!(offset_of!(LfxDescribeSink, declare_slider), 24);
    assert_eq!(offset_of!(LfxDescribeSink, declare_int), 32);
    assert_eq!(offset_of!(LfxDescribeSink, declare_angle), 40);
    assert_eq!(offset_of!(LfxDescribeSink, declare_bool), 48);
    assert_eq!(offset_of!(LfxDescribeSink, declare_choice), 56);
    assert_eq!(offset_of!(LfxDescribeSink, declare_colour), 64);
    assert_eq!(offset_of!(LfxDescribeSink, declare_seed), 72);
    assert_eq!(offset_of!(LfxDescribeSink, declare_point2), 80);
    assert_eq!(offset_of!(LfxDescribeSink, declare_point3), 88);
    assert_eq!(offset_of!(LfxDescribeSink, declare_curve), 96);
    assert_eq!(offset_of!(LfxDescribeSink, declare_file), 104);
    assert_eq!(offset_of!(LfxDescribeSink, declare_action), 112);
    assert_eq!(offset_of!(LfxDescribeSink, group_begin), 120);
    assert_eq!(offset_of!(LfxDescribeSink, group_end), 128);

    // The one struct with no size prefix. The union's own size is pinned so a
    // new arm cannot widen the element behind a plugin's back.
    assert_eq!(size_of::<LfxValue>(), 24);
    assert_eq!(offset_of!(LfxValue, param), 0);
    assert_eq!(offset_of!(LfxValue, kind), 4);
    assert_eq!(offset_of!(LfxValue, v), 8);
    assert_eq!(size_of::<LfxValuePayload>(), 16);
    // The two arms with fields of their own. A size alone does not pin them:
    // the curve arm is sixteen bytes and eight-aligned whichever order its
    // pointer and its count are written in, so a swap would leave every
    // assertion above green and hand a plugin the count where the points
    // belong. `tests/layout.c` asserts the same two by member designator,
    // `offsetof(lfx_value, v.curve.pt)` and `.n`.
    assert_eq!(size_of::<LfxCurveValue>(), 16);
    assert_eq!(offset_of!(LfxCurveValue, pt), 0);
    assert_eq!(offset_of!(LfxCurveValue, n), 8);
    assert_eq!(size_of::<LfxFileValue>(), 8);
    assert_eq!(offset_of!(LfxFileValue, path), 0);

    assert_eq!(size_of::<LfxFrame>(), 48);
    assert_eq!(offset_of!(LfxFrame, struct_size), 0);
    assert_eq!(offset_of!(LfxFrame, format), 4);
    assert_eq!(offset_of!(LfxFrame, width), 8);
    assert_eq!(offset_of!(LfxFrame, height), 12);
    assert_eq!(offset_of!(LfxFrame, row_bytes), 16);
    assert_eq!(offset_of!(LfxFrame, origin_x), 20);
    assert_eq!(offset_of!(LfxFrame, origin_y), 24);
    assert_eq!(offset_of!(LfxFrame, reserved_0), 28);
    assert_eq!(offset_of!(LfxFrame, data), 32);
    assert_eq!(offset_of!(LfxFrame, time), 40);

    assert_eq!(size_of::<LfxProcess>(), 96);
    assert_eq!(offset_of!(LfxProcess, struct_size), 0);
    assert_eq!(offset_of!(LfxProcess, pixel_format), 4);
    assert_eq!(offset_of!(LfxProcess, value_stride), 8);
    assert_eq!(offset_of!(LfxProcess, value_count), 12);
    assert_eq!(offset_of!(LfxProcess, roi_x0), 16);
    assert_eq!(offset_of!(LfxProcess, roi_y0), 20);
    assert_eq!(offset_of!(LfxProcess, roi_x1), 24);
    assert_eq!(offset_of!(LfxProcess, roi_y1), 28);
    assert_eq!(offset_of!(LfxProcess, dod_x0), 32);
    assert_eq!(offset_of!(LfxProcess, dod_y0), 36);
    assert_eq!(offset_of!(LfxProcess, dod_x1), 40);
    assert_eq!(offset_of!(LfxProcess, dod_y1), 44);
    assert_eq!(offset_of!(LfxProcess, time), 48);
    assert_eq!(offset_of!(LfxProcess, values), 56);
    assert_eq!(offset_of!(LfxProcess, input), 64);
    assert_eq!(offset_of!(LfxProcess, output), 72);
    assert_eq!(offset_of!(LfxProcess, cancelled), 80);
    assert_eq!(offset_of!(LfxProcess, host_context), 88);

    assert_eq!(size_of::<LfxHost>(), 32);
    assert_eq!(offset_of!(LfxHost, struct_size), 0);
    assert_eq!(offset_of!(LfxHost, abi_version), 4);
    assert_eq!(offset_of!(LfxHost, host_data), 8);
    assert_eq!(offset_of!(LfxHost, get_extension), 16);
    assert_eq!(offset_of!(LfxHost, log), 24);

    // The descriptor carries the padding its declaration order asks for - after
    // `struct_size`, after `patch`, and after `category_count` - and the numbers
    // are asserted rather than tidied away, because re-ordering to pack it would
    // be exactly the change the freeze forbids.
    assert_eq!(size_of::<LfxDescriptor>(), 88);
    assert_eq!(offset_of!(LfxDescriptor, struct_size), 0);
    assert_eq!(offset_of!(LfxDescriptor, id), 8);
    assert_eq!(offset_of!(LfxDescriptor, name), 16);
    assert_eq!(offset_of!(LfxDescriptor, vendor), 24);
    assert_eq!(offset_of!(LfxDescriptor, major), 32);
    assert_eq!(offset_of!(LfxDescriptor, minor), 36);
    assert_eq!(offset_of!(LfxDescriptor, patch), 40);
    assert_eq!(offset_of!(LfxDescriptor, categories), 48);
    assert_eq!(offset_of!(LfxDescriptor, category_count), 56);
    assert_eq!(offset_of!(LfxDescriptor, traits), 64);
    assert_eq!(offset_of!(LfxDescriptor, required_extensions), 72);
    assert_eq!(offset_of!(LfxDescriptor, required_extension_count), 80);

    assert_eq!(size_of::<LfxPlugin>(), 56);
    assert_eq!(offset_of!(LfxPlugin, struct_size), 0);
    assert_eq!(offset_of!(LfxPlugin, plugin_data), 8);
    assert_eq!(offset_of!(LfxPlugin, init), 16);
    assert_eq!(offset_of!(LfxPlugin, destroy), 24);
    assert_eq!(offset_of!(LfxPlugin, describe), 32);
    assert_eq!(offset_of!(LfxPlugin, process), 40);
    assert_eq!(offset_of!(LfxPlugin, get_extension), 48);

    assert_eq!(size_of::<LfxEntry>(), 48);
    assert_eq!(offset_of!(LfxEntry, struct_size), 0);
    assert_eq!(offset_of!(LfxEntry, abi_version), 4);
    assert_eq!(offset_of!(LfxEntry, init), 8);
    assert_eq!(offset_of!(LfxEntry, deinit), 16);
    assert_eq!(offset_of!(LfxEntry, count), 24);
    assert_eq!(offset_of!(LfxEntry, descriptor), 32);
    assert_eq!(offset_of!(LfxEntry, create), 40);
}

/// A static assertion nobody compiles asserts nothing, so the C half answers a
/// number and the Rust half reads it.
#[test]
fn the_c_half_of_the_layout_suite_is_compiled_and_agrees_on_the_version() {
    // SAFETY: `lfx_abi_layout_assertions_compiled` is defined in
    // `tests/layout.c`, compiled and linked by `build.rs`. It takes no
    // arguments, touches nothing, and returns a `uint32_t`.
    let from_the_header = unsafe { lfx_abi_layout_assertions_compiled() };
    assert_eq!(from_the_header, LFX_ABI_VERSION);
}

/// §2.1's second case, and the one the offsets cannot reach: a host built
/// against a newer header writes a **wider** element, says so in
/// `value_stride`, and a plugin built against the older `lfx_value` still reads
/// every value correctly - because it indexes by the stride it was handed
/// rather than by its own `size_of`.
#[test]
fn an_oversized_value_stride_is_read_by_stride_rather_than_by_sizeof() {
    // What a host built against a later header writes: the element this build
    // knows, plus eight bytes of a field it has never heard of.
    const GROWN: usize = size_of::<LfxValue>() + 8;
    const MARKER: u8 = 0xAA;
    const COUNT: usize = 3;

    let mut buffer = vec![MARKER; GROWN * COUNT];
    for (index, kind) in [LFX_PARAM_FLOAT, LFX_PARAM_CHOICE, LFX_PARAM_INT]
        .into_iter()
        .enumerate()
    {
        let value = LfxValue {
            param: index as u32,
            kind,
            v: match kind {
                LFX_PARAM_CHOICE => LfxValuePayload { choice: 7 },
                LFX_PARAM_INT => LfxValuePayload { i: -9 },
                _ => LfxValuePayload { f: 0.5 },
            },
        };
        // SAFETY: `buffer` holds `GROWN * COUNT` bytes and `index < COUNT`, so
        // the element at `index * GROWN` has `GROWN >= size_of::<LfxValue>()`
        // bytes behind it. The write is unaligned because the host's buffer is
        // a byte buffer, which is what the ring hands over.
        unsafe {
            std::ptr::write_unaligned(
                buffer.as_mut_ptr().add(index * GROWN).cast::<LfxValue>(),
                value,
            );
        }
    }

    // The plugin's read, which is the whole rule: `param * stride`.
    let stride = GROWN as u32;
    for index in 0..COUNT {
        // SAFETY: as above - `index * stride` is inside the buffer and at least
        // one whole `LfxValue` lies behind it.
        let read = unsafe {
            std::ptr::read_unaligned(
                buffer
                    .as_ptr()
                    .add(index * stride as usize)
                    .cast::<LfxValue>(),
            )
        };
        assert_eq!(read.param, index as u32);
        match read.kind {
            // SAFETY: the arm is the one `kind` names, which is the contract
            // the tag exists for.
            LFX_PARAM_FLOAT => assert_eq!(unsafe { read.v.f }, 0.5),
            LFX_PARAM_CHOICE => assert_eq!(unsafe { read.v.choice }, 7),
            LFX_PARAM_INT => assert_eq!(unsafe { read.v.i }, -9),
            other => panic!("a kind nothing wrote: {other}"),
        }
    }

    // And the failure it prevents, spelled out: a plugin that strode by its own
    // `size_of` would land in the middle of the host's second element and read
    // a plausible-looking number out of a field it has never heard of.
    // SAFETY: `size_of::<LfxValue>()` is inside the buffer by a wide margin.
    let strode_by_sizeof = unsafe {
        std::ptr::read_unaligned(
            buffer
                .as_ptr()
                .add(size_of::<LfxValue>())
                .cast::<LfxValue>(),
        )
    };
    assert_ne!(strode_by_sizeof.param, 1);
    assert_eq!(strode_by_sizeof.param, u32::from_ne_bytes([MARKER; 4]));
}

/// Trap 18: a zero in a trait block is a declaration, and the enumerations are
/// ordered so that it is the **pessimistic** one. Ordering them to mirror the
/// host's own enums would make a `memset` block claim trivial cost and an exact
/// ROI, which docs/13 calls a correctness bug.
#[test]
fn a_zero_in_a_trait_block_is_the_pessimistic_declaration() {
    assert_eq!(LFX_COST_UNSET, 0);
    assert_eq!(LFX_ROI_UNSET, 0);
    assert_eq!(LFX_ALPHA_UNSET, 0);
    assert_eq!(LFX_TRAIT_NONE, 0);
    assert_eq!(LFX_UNIT_UNSET, 0);
    assert_eq!(LFX_PARAM_UNSET, 0);
    assert_eq!(LFX_CATEGORY_UNSET, 0);
    assert_eq!(LFX_PIXEL_UNSET, 0);

    // The answers each of those unset values lowers to are *not* zero, which is
    // the whole mechanism: nothing can arrive at the pessimistic case by
    // accident, and nothing can arrive at the optimistic one by leaving a field
    // out.
    assert_ne!(LFX_COST_HEAVY, LFX_COST_UNSET);
    assert_ne!(LFX_ROI_FULL_FRAME, LFX_ROI_UNSET);
    assert_ne!(LFX_ALPHA_PREMULTIPLIED, LFX_ALPHA_UNSET);
    assert_ne!(LFX_COST_TRIVIAL, LFX_COST_UNSET);
    assert_ne!(LFX_ROI_EXACT, LFX_ROI_UNSET);

    // A vendor's `lfx_traits t = {0};` before filling two fields.
    let memset = LfxTraits::default();
    assert_eq!(memset.cost, LFX_COST_UNSET);
    assert_eq!(memset.roi_kind, LFX_ROI_UNSET);
    assert_eq!(memset.alpha, LFX_ALPHA_UNSET);
    assert_eq!(memset.flags, LFX_TRAIT_NONE);
    assert_eq!(memset.temporal_lo, 0);
    assert_eq!(memset.temporal_hi, 0);
    assert_eq!(memset.scratch_bytes_per_megapixel, 0);
}

/// What a block read to its own `struct_size` looks like: the fields it stated
/// are its own and the tail is nought, which is the unstated case every trait
/// enumeration lowers pessimistically.
///
/// It is the shape of the rule rather than a reader of it. Version 1 has no
/// reader that takes a short `lfx_traits` this way - the header says which
/// answer each struct gets when it arrives short, and a short trait block is
/// the pessimistic case exactly as a `NULL` one is. What this pins is why that
/// answer is safe either way: whichever of the two a reader takes, the tail it
/// has not been told about reads as unstated and never as trivial.
#[test]
fn a_block_read_to_its_own_struct_size_leaves_its_tail_zero() {
    // A plugin built when `lfx_traits` stopped after `temporal_hi`.
    const SHORT: usize = 24;
    let mut older = [0u8; SHORT];
    older[..4].copy_from_slice(&(SHORT as u32).to_ne_bytes());
    older[4..8].copy_from_slice(&LFX_COST_CHEAP.to_ne_bytes());
    older[8..12].copy_from_slice(&LFX_ROI_PADDED.to_ne_bytes());
    older[12..16].copy_from_slice(&8.0f32.to_ne_bytes());
    older[16..20].copy_from_slice(&(-1i32).to_ne_bytes());
    older[20..24].copy_from_slice(&1i32.to_ne_bytes());

    // What the host does with it: take the bytes the plugin wrote, leave the
    // rest as it found them, which is nought.
    let mut whole = [0u8; size_of::<LfxTraits>()];
    let declared = u32::from_ne_bytes([older[0], older[1], older[2], older[3]]) as usize;
    assert_eq!(declared, SHORT);
    whole[..declared].copy_from_slice(&older);
    // SAFETY: `whole` is exactly `size_of::<LfxTraits>()` bytes of a type whose
    // every field is a plain four-byte number, so every bit pattern is a valid
    // value. The read is unaligned because the bytes came off a wire.
    let read = unsafe { std::ptr::read_unaligned(whole.as_ptr().cast::<LfxTraits>()) };

    assert_eq!(read.struct_size, SHORT as u32);
    assert_eq!(read.cost, LFX_COST_CHEAP);
    assert_eq!(read.roi_kind, LFX_ROI_PADDED);
    assert_eq!(read.roi_padding_px, 8.0);
    assert_eq!(read.temporal_lo, -1);
    assert_eq!(read.temporal_hi, 1);
    // The three fields the older header had not got: unstated, not optimistic.
    assert_eq!(read.alpha, LFX_ALPHA_UNSET);
    assert_eq!(read.flags, LFX_TRAIT_NONE);
    assert_eq!(read.scratch_bytes_per_megapixel, 0);
}

/// D2 and D3: the two kinds version 1 refuses have discriminants from day one,
/// so admitting them when `lfx.overlay` lands adds no variant and breaks no
/// compiled plugin. Every discriminant is distinct, and the vocabulary is
/// closed.
#[test]
fn the_refused_kinds_carry_discriminants_from_day_one() {
    let kinds = [
        LFX_PARAM_UNSET,
        LFX_PARAM_FLOAT,
        LFX_PARAM_SLIDER,
        LFX_PARAM_INT,
        LFX_PARAM_BOOL,
        LFX_PARAM_CHOICE,
        LFX_PARAM_COLOUR,
        LFX_PARAM_ANGLE,
        LFX_PARAM_SEED,
        LFX_PARAM_POINT2,
        LFX_PARAM_POINT3,
        LFX_PARAM_CURVE,
        LFX_PARAM_FILE,
        LFX_PARAM_ACTION,
        LFX_PARAM_GROUP,
        LFX_PARAM_PATH,
        LFX_PARAM_STRING,
    ];
    let mut seen = kinds.to_vec();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), kinds.len(), "two kinds share a discriminant");

    // Distinct is not the property. The property is *these numbers*: a vendor
    // compiled against the header carries them in their binary, so swapping
    // PATH and STRING, or moving FILE along to make room, breaks every plugin
    // already built while leaving the enumeration just as distinct.
    assert_eq!(LFX_PARAM_UNSET, 0);
    assert_eq!(LFX_PARAM_FLOAT, 1);
    assert_eq!(LFX_PARAM_SLIDER, 2);
    assert_eq!(LFX_PARAM_INT, 3);
    assert_eq!(LFX_PARAM_BOOL, 4);
    assert_eq!(LFX_PARAM_CHOICE, 5);
    assert_eq!(LFX_PARAM_COLOUR, 6);
    assert_eq!(LFX_PARAM_ANGLE, 7);
    assert_eq!(LFX_PARAM_SEED, 8);
    assert_eq!(LFX_PARAM_POINT2, 9);
    assert_eq!(LFX_PARAM_POINT3, 10);
    assert_eq!(LFX_PARAM_CURVE, 11);
    assert_eq!(LFX_PARAM_FILE, 12);
    assert_eq!(LFX_PARAM_ACTION, 13);
    assert_eq!(LFX_PARAM_GROUP, 14);
    // The two reserved seats, held from day one so admitting them adds no
    // variant: the path waits for the overlay, the text row for a bag that can
    // carry text.
    assert_eq!(LFX_PARAM_PATH, 15);
    assert_eq!(LFX_PARAM_STRING, 16);
}

/// The extension ids are the spelling both sides agree on, NUL-terminated for
/// the C call that carries them. Only `lfx.temporal` is offered in version 1;
/// the rest are reserved so nobody mints a second spelling of them.
#[test]
fn the_extension_ids_are_nul_terminated_and_spelled_once() {
    for id in [
        LFX_EXT_TEMPORAL,
        LFX_EXT_GPU_FRAMES,
        LFX_EXT_OVERLAY,
        LFX_EXT_MOTION_VECTORS,
        LFX_EXT_AUDIO,
    ] {
        let text = std::ffi::CStr::from_bytes_with_nul(id).expect("one NUL, at the end");
        let text = text.to_str().expect("ASCII");
        assert!(text.starts_with("lfx."), "{text} is not in the namespace");
    }
    // And the spellings themselves, because a reserved id is only reserved if
    // it is the same string in every reader: `lfx-validator`, the broker's
    // negotiation and a vendor's `get_extension` call all carry these letters.
    assert_eq!(LFX_EXT_TEMPORAL, b"lfx.temporal\0");
    assert_eq!(LFX_EXT_GPU_FRAMES, b"lfx.gpu-frames\0");
    assert_eq!(LFX_EXT_OVERLAY, b"lfx.overlay\0");
    assert_eq!(LFX_EXT_MOTION_VECTORS, b"lfx.motion-vectors\0");
    assert_eq!(LFX_EXT_AUDIO, b"lfx.audio\0");
    assert_eq!(LFX_ENTRY_SYMBOL, b"lfx_entry_point\0");
    let entry = std::ffi::CStr::from_bytes_with_nul(LFX_ENTRY_SYMBOL).expect("one NUL, at the end");
    assert_eq!(entry.to_str().expect("ASCII"), "lfx_entry_point");
}

/// The rest of the frozen ladders, by number rather than by order. Every one of
/// these is a value a compiled plugin carries in its binary: a unit, a cost, a
/// status. Renumbering one leaves the enumeration just as well-formed and every
/// shipped bundle just as wrong, which is why the numbers are the assertion.
#[test]
fn every_ladder_is_frozen_at_the_numbers_it_shipped_with() {
    assert_eq!(LFX_ABI_VERSION, 1);

    assert_eq!(LFX_UNIT_UNSET, 0);
    assert_eq!(LFX_UNIT_RAW, 1);
    assert_eq!(LFX_UNIT_PERCENT, 2);
    assert_eq!(LFX_UNIT_PCT_DIAG, 3);
    assert_eq!(LFX_UNIT_PX, 4);
    assert_eq!(LFX_UNIT_DEGREES, 5);
    assert_eq!(LFX_UNIT_SECONDS, 6);
    assert_eq!(LFX_UNIT_FRAMES, 7);

    // Eight picture families, and no more: Audio, Drivers, Controls and
    // Compositing are deliberately not claimable.
    assert_eq!(LFX_CATEGORY_UNSET, 0);
    assert_eq!(LFX_CATEGORY_BLUR_SHARPEN, 1);
    assert_eq!(LFX_CATEGORY_COLOUR, 2);
    assert_eq!(LFX_CATEGORY_DISTORTION, 3);
    assert_eq!(LFX_CATEGORY_GENERATE, 4);
    assert_eq!(LFX_CATEGORY_STYLISE, 5);
    assert_eq!(LFX_CATEGORY_TEMPORAL, 6);
    assert_eq!(LFX_CATEGORY_TRANSITION, 7);
    assert_eq!(LFX_CATEGORY_UTILITY, 8);
    assert_eq!(LFX_MAX_CATEGORIES, LFX_CATEGORY_UTILITY);

    assert_eq!(LFX_PIXEL_UNSET, 0);
    assert_eq!(LFX_RGBA_F16, 1);
    assert_eq!(LFX_RGBA_F32, 2);

    assert_eq!(LFX_COST_UNSET, 0);
    assert_eq!(LFX_COST_TRIVIAL, 1);
    assert_eq!(LFX_COST_CHEAP, 2);
    assert_eq!(LFX_COST_MODERATE, 3);
    assert_eq!(LFX_COST_HEAVY, 4);

    assert_eq!(LFX_ROI_UNSET, 0);
    assert_eq!(LFX_ROI_EXACT, 1);
    assert_eq!(LFX_ROI_PADDED, 2);
    assert_eq!(LFX_ROI_FULL_FRAME, 3);

    assert_eq!(LFX_ALPHA_UNSET, 0);
    assert_eq!(LFX_ALPHA_PREMULTIPLIED, 1);
    assert_eq!(LFX_ALPHA_STRAIGHT, 2);

    // The flag words are bits, and a bit that moved is a plugin declaring one
    // trait and being scheduled for another.
    assert_eq!(LFX_TRAIT_NONE, 0);
    assert_eq!(LFX_TRAIT_SEEDED, 1);
    assert_eq!(LFX_TRAIT_THREAD_UNSAFE, 2);
    assert_eq!(LFX_TRAIT_CANCELLABLE, 4);

    assert_eq!(LFX_PARAM_FLAG_NONE, 0);
    assert_eq!(LFX_PARAM_FLAG_STATIC, 1);
    assert_eq!(LFX_PARAM_FLAG_HIDDEN, 2);

    assert_eq!(LFX_BOUND_NONE, 0);
    assert_eq!(LFX_BOUND_MIN, 1);
    assert_eq!(LFX_BOUND_MAX, 2);

    assert_eq!(LFX_LOG_UNSET, 0);
    assert_eq!(LFX_LOG_ERROR, 1);
    assert_eq!(LFX_LOG_WARN, 2);
    assert_eq!(LFX_LOG_INFO, 3);
    assert_eq!(LFX_LOG_DEBUG, 4);
    assert_eq!(LFX_LOG_TRACE, 5);

    assert_eq!(LFX_STATUS_OK, 0);
    assert_eq!(LFX_STATUS_FAILED, 1);
    assert_eq!(LFX_STATUS_CANCELLED, 2);
    assert_eq!(LFX_STATUS_OUT_OF_MEMORY, 3);
    assert_eq!(LFX_STATUS_UNSUPPORTED, 4);

    // The ceilings, which are frozen for the same reason and worse: a limit
    // cannot be raised once a vendor has shipped inside it, and cannot be
    // narrowed once one has shipped up against it.
    assert_eq!(LFX_MAX_STRING_BYTES, 1024);
    assert_eq!(LFX_MAX_LOG_BYTES, 4096);
    assert_eq!(LFX_MAX_OPTIONS, 256);
    // A dropdown draws at most one rule per option, so the divider ceiling is
    // the option ceiling: the relationship is the assertion, because a number
    // of its own could turn out too small and could not then be raised.
    assert_eq!(LFX_MAX_DIVIDERS, 256);
    assert_eq!(LFX_MAX_DIVIDERS, LFX_MAX_OPTIONS);
    assert_eq!(LFX_MAX_FILTERS, 32);
    assert_eq!(LFX_MAX_REQUIRED_EXTENSIONS, 16);
    assert_eq!(LFX_MAX_EFFECTS_PER_BUNDLE, 1024);
    assert_eq!(LFX_MAX_PARAMS, 512);
    assert_eq!(LFX_MIN_CURVE_POINTS, 2);
    assert_eq!(LFX_MAX_CURVE_POINTS, 16);
    assert_eq!(LFX_MAX_TEMPORAL_WINDOW, 64);
}

/// The mirror's constants, in the order `tests/layout.c` emits the header's.
/// The name travels with the number so a mismatch says which one moved.
fn the_mirrors_constants() -> Vec<(&'static str, u32)> {
    vec![
        ("LFX_ABI_VERSION", LFX_ABI_VERSION),
        // the ceilings
        ("LFX_MAX_STRING_BYTES", LFX_MAX_STRING_BYTES),
        ("LFX_MAX_LOG_BYTES", LFX_MAX_LOG_BYTES),
        ("LFX_MAX_CATEGORIES", LFX_MAX_CATEGORIES),
        ("LFX_MAX_OPTIONS", LFX_MAX_OPTIONS),
        ("LFX_MAX_DIVIDERS", LFX_MAX_DIVIDERS),
        ("LFX_MAX_FILTERS", LFX_MAX_FILTERS),
        ("LFX_MAX_REQUIRED_EXTENSIONS", LFX_MAX_REQUIRED_EXTENSIONS),
        ("LFX_MAX_EFFECTS_PER_BUNDLE", LFX_MAX_EFFECTS_PER_BUNDLE),
        ("LFX_MAX_PARAMS", LFX_MAX_PARAMS),
        ("LFX_MIN_CURVE_POINTS", LFX_MIN_CURVE_POINTS),
        ("LFX_MAX_CURVE_POINTS", LFX_MAX_CURVE_POINTS),
        ("LFX_MAX_TEMPORAL_WINDOW", LFX_MAX_TEMPORAL_WINDOW as u32),
        // lfx_param_kind
        ("LFX_PARAM_UNSET", LFX_PARAM_UNSET),
        ("LFX_PARAM_FLOAT", LFX_PARAM_FLOAT),
        ("LFX_PARAM_SLIDER", LFX_PARAM_SLIDER),
        ("LFX_PARAM_INT", LFX_PARAM_INT),
        ("LFX_PARAM_BOOL", LFX_PARAM_BOOL),
        ("LFX_PARAM_CHOICE", LFX_PARAM_CHOICE),
        ("LFX_PARAM_COLOUR", LFX_PARAM_COLOUR),
        ("LFX_PARAM_ANGLE", LFX_PARAM_ANGLE),
        ("LFX_PARAM_SEED", LFX_PARAM_SEED),
        ("LFX_PARAM_POINT2", LFX_PARAM_POINT2),
        ("LFX_PARAM_POINT3", LFX_PARAM_POINT3),
        ("LFX_PARAM_CURVE", LFX_PARAM_CURVE),
        ("LFX_PARAM_FILE", LFX_PARAM_FILE),
        ("LFX_PARAM_ACTION", LFX_PARAM_ACTION),
        ("LFX_PARAM_GROUP", LFX_PARAM_GROUP),
        ("LFX_PARAM_PATH", LFX_PARAM_PATH),
        ("LFX_PARAM_STRING", LFX_PARAM_STRING),
        // lfx_unit
        ("LFX_UNIT_UNSET", LFX_UNIT_UNSET),
        ("LFX_UNIT_RAW", LFX_UNIT_RAW),
        ("LFX_UNIT_PERCENT", LFX_UNIT_PERCENT),
        ("LFX_UNIT_PCT_DIAG", LFX_UNIT_PCT_DIAG),
        ("LFX_UNIT_PX", LFX_UNIT_PX),
        ("LFX_UNIT_DEGREES", LFX_UNIT_DEGREES),
        ("LFX_UNIT_SECONDS", LFX_UNIT_SECONDS),
        ("LFX_UNIT_FRAMES", LFX_UNIT_FRAMES),
        // lfx_category
        ("LFX_CATEGORY_UNSET", LFX_CATEGORY_UNSET),
        ("LFX_CATEGORY_BLUR_SHARPEN", LFX_CATEGORY_BLUR_SHARPEN),
        ("LFX_CATEGORY_COLOUR", LFX_CATEGORY_COLOUR),
        ("LFX_CATEGORY_DISTORTION", LFX_CATEGORY_DISTORTION),
        ("LFX_CATEGORY_GENERATE", LFX_CATEGORY_GENERATE),
        ("LFX_CATEGORY_STYLISE", LFX_CATEGORY_STYLISE),
        ("LFX_CATEGORY_TEMPORAL", LFX_CATEGORY_TEMPORAL),
        ("LFX_CATEGORY_TRANSITION", LFX_CATEGORY_TRANSITION),
        ("LFX_CATEGORY_UTILITY", LFX_CATEGORY_UTILITY),
        // lfx_pixel_format
        ("LFX_PIXEL_UNSET", LFX_PIXEL_UNSET),
        ("LFX_RGBA_F16", LFX_RGBA_F16),
        ("LFX_RGBA_F32", LFX_RGBA_F32),
        // lfx_cost
        ("LFX_COST_UNSET", LFX_COST_UNSET),
        ("LFX_COST_TRIVIAL", LFX_COST_TRIVIAL),
        ("LFX_COST_CHEAP", LFX_COST_CHEAP),
        ("LFX_COST_MODERATE", LFX_COST_MODERATE),
        ("LFX_COST_HEAVY", LFX_COST_HEAVY),
        // lfx_roi_kind
        ("LFX_ROI_UNSET", LFX_ROI_UNSET),
        ("LFX_ROI_EXACT", LFX_ROI_EXACT),
        ("LFX_ROI_PADDED", LFX_ROI_PADDED),
        ("LFX_ROI_FULL_FRAME", LFX_ROI_FULL_FRAME),
        // lfx_alpha
        ("LFX_ALPHA_UNSET", LFX_ALPHA_UNSET),
        ("LFX_ALPHA_PREMULTIPLIED", LFX_ALPHA_PREMULTIPLIED),
        ("LFX_ALPHA_STRAIGHT", LFX_ALPHA_STRAIGHT),
        // lfx_trait_flags
        ("LFX_TRAIT_NONE", LFX_TRAIT_NONE),
        ("LFX_TRAIT_SEEDED", LFX_TRAIT_SEEDED),
        ("LFX_TRAIT_THREAD_UNSAFE", LFX_TRAIT_THREAD_UNSAFE),
        ("LFX_TRAIT_CANCELLABLE", LFX_TRAIT_CANCELLABLE),
        // lfx_param_flags
        ("LFX_PARAM_FLAG_NONE", LFX_PARAM_FLAG_NONE),
        ("LFX_PARAM_FLAG_STATIC", LFX_PARAM_FLAG_STATIC),
        ("LFX_PARAM_FLAG_HIDDEN", LFX_PARAM_FLAG_HIDDEN),
        // lfx_bounds
        ("LFX_BOUND_NONE", LFX_BOUND_NONE),
        ("LFX_BOUND_MIN", LFX_BOUND_MIN),
        ("LFX_BOUND_MAX", LFX_BOUND_MAX),
        // lfx_log_level
        ("LFX_LOG_UNSET", LFX_LOG_UNSET),
        ("LFX_LOG_ERROR", LFX_LOG_ERROR),
        ("LFX_LOG_WARN", LFX_LOG_WARN),
        ("LFX_LOG_INFO", LFX_LOG_INFO),
        ("LFX_LOG_DEBUG", LFX_LOG_DEBUG),
        ("LFX_LOG_TRACE", LFX_LOG_TRACE),
        // lfx_status
        ("LFX_STATUS_OK", LFX_STATUS_OK as u32),
        ("LFX_STATUS_FAILED", LFX_STATUS_FAILED as u32),
        ("LFX_STATUS_CANCELLED", LFX_STATUS_CANCELLED as u32),
        ("LFX_STATUS_OUT_OF_MEMORY", LFX_STATUS_OUT_OF_MEMORY as u32),
        ("LFX_STATUS_UNSUPPORTED", LFX_STATUS_UNSUPPORTED as u32),
    ]
}

/// §14 item 1, the half the offsets cannot reach: **every** value that crosses
/// the wire, compared between the header and the mirror.
///
/// An offset says where a field sits and never what a number means, so a kind
/// renumbered in one half alone moves nothing and fails nothing - while a vendor
/// compiled against the header declares a File row the host reads as some other
/// kind entirely. The C half emits the header's own constants in one fixed
/// order and this walks them beside the mirror's, name by name.
#[test]
fn every_constant_is_the_number_the_header_declares() {
    // SAFETY: both are defined in `tests/layout.c`, compiled and linked by
    // `build.rs`. `lfx_abi_constants` answers a pointer to a `static const`
    // array of `lfx_abi_constant_count()` `uint32_t`, which lives for the
    // length of the process.
    let (count, from_the_header) = unsafe {
        let count = lfx_abi_constant_count() as usize;
        (
            count,
            std::slice::from_raw_parts(lfx_abi_constants(), count),
        )
    };

    let mirror = the_mirrors_constants();
    assert_eq!(
        count,
        mirror.len(),
        "one half declares a constant the other has not got"
    );
    for (index, (name, mine)) in mirror.iter().enumerate() {
        assert_eq!(
            from_the_header[index], *mine,
            "{name} is {} in the header and {mine} in the mirror",
            from_the_header[index]
        );
    }
}

/// The same, for the strings: the entry symbol the loader looks up and the five
/// extension ids. A misspelling moves no field either, and costs a plugin the
/// extension it asked for - refused as missing, which is the one refusal §4.3
/// makes final.
#[test]
fn every_string_constant_is_the_spelling_the_header_declares() {
    let mirror: [(&str, &[u8]); 6] = [
        ("LFX_ENTRY_SYMBOL", LFX_ENTRY_SYMBOL),
        ("LFX_EXT_TEMPORAL", LFX_EXT_TEMPORAL),
        ("LFX_EXT_GPU_FRAMES", LFX_EXT_GPU_FRAMES),
        ("LFX_EXT_OVERLAY", LFX_EXT_OVERLAY),
        ("LFX_EXT_MOTION_VECTORS", LFX_EXT_MOTION_VECTORS),
        ("LFX_EXT_AUDIO", LFX_EXT_AUDIO),
    ];

    // SAFETY: defined in `tests/layout.c`. `lfx_abi_string` answers a pointer
    // to a string literal, which lives for the length of the process, or null
    // past the end of its table.
    let count = unsafe { lfx_abi_string_count() } as usize;
    assert_eq!(
        count,
        mirror.len(),
        "one half spells a string the other has not got"
    );

    for (index, (name, mine)) in mirror.iter().enumerate() {
        // SAFETY: `index < count`, so the answer is one of the table's literals
        // and not null.
        let theirs = unsafe {
            let text = lfx_abi_string(index as u32);
            assert!(!text.is_null(), "{name} is null in the header");
            std::ffi::CStr::from_ptr(text)
        };
        let mine = std::ffi::CStr::from_bytes_with_nul(mine).expect("one NUL, at the end");
        assert_eq!(theirs, mine, "{name} is spelled differently in the header");
    }

    // SAFETY: as above; past the end is the documented null.
    assert!(unsafe { lfx_abi_string(count as u32) }.is_null());
}

/// And the third thing an offset cannot see: a field's **type**, wherever two
/// types share a width. `float roi_padding_px` against a `u32` is four bytes at
/// twelve on both sides; `int32_t temporal_lo` against a `u32` is four bytes at
/// sixteen, and every small positive number reads alike. The C half writes each
/// field from the header's own declaration and this reads it back through the
/// mirror, so a type that drifted fails on the value.
#[test]
fn every_field_carries_the_type_the_header_gives_it() {
    // SAFETY: each writer is `tests/layout.c`'s, for the struct it names, and
    // sets every field of it. See `written_by_the_header`.
    let traits = unsafe { written_by_the_header(lfx_abi_write_traits) };
    assert_eq!(traits.struct_size as usize, size_of::<LfxTraits>());
    assert_eq!(traits.cost, LFX_COST_MODERATE);
    assert_eq!(traits.roi_kind, LFX_ROI_PADDED);
    // A `u32` here would read 1_056_964_608, which is what 0.5f is in bits.
    assert_eq!(
        traits.roi_padding_px, 0.5,
        "roi_padding_px is not the float the header declares"
    );
    // And a `u32` here would read 4_294_967_293.
    assert_eq!(
        traits.temporal_lo, -3,
        "temporal_lo is not the signed number the header declares"
    );
    assert_eq!(traits.temporal_hi, 4);
    assert_eq!(traits.alpha, LFX_ALPHA_STRAIGHT);
    assert_eq!(traits.flags, LFX_TRAIT_SEEDED | LFX_TRAIT_CANCELLABLE);
    assert_eq!(traits.scratch_bytes_per_megapixel, 12_345);

    // SAFETY: as above.
    let float = unsafe { written_by_the_header(lfx_abi_write_float_param) };
    assert_eq!(float.struct_size as usize, size_of::<LfxFloatParam>());
    assert_eq!(float.unit, LFX_UNIT_PX);
    assert_eq!(float.flags, LFX_PARAM_FLAG_STATIC);
    assert_eq!(float.bounds, LFX_BOUND_MIN | LFX_BOUND_MAX);
    assert_eq!(float.default_value, 0.25);
    assert_eq!(float.slider_min, -1.5);
    assert_eq!(float.slider_max, 2.5);
    assert_eq!(float.hard_min, -8.0);
    assert_eq!(float.hard_max, 8.0);

    // SAFETY: as above.
    let slider = unsafe { written_by_the_header(lfx_abi_write_slider_param) };
    assert_eq!(slider.unit, LFX_UNIT_SECONDS);
    assert_eq!(slider.log, 1);
    assert_eq!(slider.default_value, 0.75);
    assert_eq!(slider.range_min, 0.125);
    assert_eq!(slider.range_max, 4.0);

    // SAFETY: as above.
    let int = unsafe { written_by_the_header(lfx_abi_write_int_param) };
    assert_eq!(int.unit, LFX_UNIT_FRAMES);
    assert_eq!(int.flags, LFX_PARAM_FLAG_HIDDEN);
    assert_eq!(int.bounds, LFX_BOUND_MAX);
    // Wider than an `i32` either way, and signed, so a mirror that narrowed or
    // unsigned them fails on the value.
    assert_eq!(
        int.default_value, -5_000_000_000,
        "an Int's default is not the signed 64-bit number the header declares"
    );
    assert_eq!(int.slider_min, -9_000_000_000);
    assert_eq!(int.slider_max, 9_000_000_000);
    assert_eq!(int.hard_min, -1);
    assert_eq!(int.hard_max, 7_000_000_000);

    // SAFETY: as above.
    let angle = unsafe { written_by_the_header(lfx_abi_write_angle_param) };
    assert_eq!(angle.unit, LFX_UNIT_DEGREES);
    assert_eq!(angle.reserved_0, 0);
    assert_eq!(angle.default_value, 45.0);
    assert_eq!(angle.dial_step, 7.5);

    // SAFETY: as above. A switch's default is a `u32` rather than a C `bool`,
    // so nothing has to guess what byte a stranger's compiler left there - and
    // a mirror that read it as a `float` would read this one as 1.4e-45, which
    // the host's own `!= 0` would still call true and every panel would draw
    // as off.
    let switch = unsafe { written_by_the_header(lfx_abi_write_bool_param) };
    assert_eq!(switch.struct_size as usize, size_of::<LfxBoolParam>());
    assert_eq!(switch.unit, LFX_UNIT_RAW);
    assert_eq!(
        switch.default_value, 1,
        "a switch's default is not the unsigned number the header declares"
    );

    // SAFETY: as above.
    let choice = unsafe { written_by_the_header(lfx_abi_write_choice_param) };
    assert_eq!(choice.struct_size as usize, size_of::<LfxChoiceParam>());
    assert_eq!(choice.default_index, 2);
    // Past the top of an `i32`, so a count that drifted to a signed type reads
    // as minus two billion rather than as a number that happens to look alike.
    assert_eq!(
        choice.option_count, 0x8000_0001,
        "an option count is not the unsigned number the header declares"
    );
    assert_eq!(choice.divider_count, 2);
    // SAFETY: `dividers_after` is a file static of `tests/layout.c`'s holding
    // `divider_count` `uint32_t`, living for the length of the process.
    let dividers = unsafe { std::slice::from_raw_parts(choice.dividers_after, 2) };
    assert_eq!(dividers, [0, 1]);
    // SAFETY: `options` is a file static holding three NUL-terminated literals.
    let first = unsafe { std::ffi::CStr::from_ptr(*choice.options) };
    assert_eq!(first.to_str().expect("ASCII"), "First");

    // SAFETY: as above.
    let colour = unsafe { written_by_the_header(lfx_abi_write_colour_param) };
    assert_eq!(colour.default_rgba, [0.125, 0.25, 0.5, 1.0]);
    assert_eq!(colour.range_min, -0.5);
    assert_eq!(colour.range_max, 1.5);

    // SAFETY: as above.
    let seed = unsafe { written_by_the_header(lfx_abi_write_seed_param) };
    assert_eq!(seed.struct_size as usize, size_of::<LfxSeedParam>());
    assert_eq!(seed.unit, LFX_UNIT_RAW);
    assert_eq!(seed.flags, LFX_PARAM_FLAG_HIDDEN);
    assert_eq!(seed.reserved_0, 0);

    // SAFETY: as above.
    let point2 = unsafe { written_by_the_header(lfx_abi_write_point2_param) };
    assert_eq!(point2.default_x, 1.25);
    assert_eq!(point2.default_y, -2.75);
    assert_eq!(point2.slider_min, -10.0);
    assert_eq!(point2.slider_max, 10.0);

    // SAFETY: as above.
    let point3 = unsafe { written_by_the_header(lfx_abi_write_point3_param) };
    assert_eq!(point3.default_x, 1.25);
    assert_eq!(point3.default_y, -2.75);
    assert_eq!(point3.default_z, 3.5);

    // SAFETY: as above.
    let curve = unsafe { written_by_the_header(lfx_abi_write_curve_param) };
    assert_eq!(curve.struct_size as usize, size_of::<LfxCurveParam>());
    assert_eq!(curve.flags, LFX_PARAM_FLAG_STATIC);
    assert_eq!(
        curve.point_count, 0x8000_0001,
        "a point count is not the unsigned number the header declares"
    );
    // SAFETY: `points` is a file static of `tests/layout.c`'s holding
    // `2 * point_count` `float`, living for the length of the process. A mirror
    // that read it as a pointer to `f64` would read two of these as one.
    let points = unsafe { std::slice::from_raw_parts(curve.points, 4) };
    assert_eq!(points, [0.0, 0.25, 1.0, 0.75]);

    // SAFETY: as above.
    let file = unsafe { written_by_the_header(lfx_abi_write_file_param) };
    assert_eq!(file.struct_size as usize, size_of::<LfxFileParam>());
    assert_eq!(
        file.filter_count, 0x8000_0001,
        "a filter count is not the unsigned number the header declares"
    );
    // SAFETY: `filter` is a file static holding `filter_count` literals, and
    // `filter_name` is one.
    let (filter, filter_name) = unsafe {
        (
            std::ffi::CStr::from_ptr(*file.filter),
            std::ffi::CStr::from_ptr(file.filter_name),
        )
    };
    assert_eq!(filter.to_str().expect("ASCII"), "cube");
    assert_eq!(filter_name.to_str().expect("ASCII"), "Lookup tables");

    // SAFETY: as above.
    let action = unsafe { written_by_the_header(lfx_abi_write_action_param) };
    assert_eq!(action.struct_size as usize, size_of::<LfxActionParam>());
    assert_eq!(action.unit, LFX_UNIT_RAW);
    assert_eq!(action.reserved_0, 0);

    // SAFETY: as above. The one record with no `unit`: a heading is a run
    // rather than a row, so its flags sit where every other record's unit does,
    // and a mirror that gave it a unit field would read the flags as one.
    let group = unsafe { written_by_the_header(lfx_abi_write_group_param) };
    assert_eq!(group.struct_size as usize, size_of::<LfxGroupParam>());
    assert_eq!(
        group.flags, LFX_PARAM_FLAG_HIDDEN,
        "a heading's flags are not where the header puts them"
    );

    // SAFETY: as above.
    let descriptor = unsafe { written_by_the_header(lfx_abi_write_descriptor) };
    assert_eq!(descriptor.struct_size as usize, size_of::<LfxDescriptor>());
    // Past the top of an `i32`. The major version is the number the host's
    // frame key is stored in, so a signed mirror would key a cache on a
    // negative for any release past two billion.
    assert_eq!(
        descriptor.major, 0x8000_0001,
        "a major version is not the unsigned number the header declares"
    );
    assert_eq!(descriptor.minor, 999);
    assert_eq!(descriptor.patch, 1);
    assert_eq!(descriptor.category_count, 2);
    assert!(descriptor.traits.is_null(), "null is the pessimistic case");
    // SAFETY: `categories` is a file static holding `category_count`
    // `lfx_category`, and `required_extensions` one literal.
    let (categories, required) = unsafe {
        (
            std::slice::from_raw_parts(descriptor.categories, 2),
            std::ffi::CStr::from_ptr(*descriptor.required_extensions),
        )
    };
    assert_eq!(categories, [LFX_CATEGORY_COLOUR, LFX_CATEGORY_UTILITY]);
    assert_eq!(required.to_bytes_with_nul(), LFX_EXT_TEMPORAL);

    // SAFETY: as above.
    let frame = unsafe { written_by_the_header(lfx_abi_write_frame) };
    assert_eq!(frame.struct_size as usize, size_of::<LfxFrame>());
    assert_eq!(frame.format, LFX_RGBA_F32);
    assert_eq!(frame.width, 640);
    assert_eq!(frame.height, 360);
    assert_eq!(frame.row_bytes, 640 * 16);
    // Signed, because a frame's origin is a position in the raster in play and
    // the input's often sits above and to the left of it.
    assert_eq!(
        frame.origin_x, -7,
        "a frame's origin is not the signed number the header declares"
    );
    assert_eq!(frame.origin_y, -9);
    assert_eq!(frame.reserved_0, 0);
    assert!(frame.data.is_null());
    assert_eq!(
        frame.time, 12.5,
        "a frame's time is not the double the header declares"
    );

    // SAFETY: as above.
    let process = unsafe { written_by_the_header(lfx_abi_write_process) };
    assert_eq!(process.struct_size as usize, size_of::<LfxProcess>());
    assert_eq!(process.pixel_format, LFX_RGBA_F16);
    assert_eq!(process.value_stride as usize, size_of::<LfxValue>());
    assert_eq!(process.value_count, 2);
    assert_eq!(
        [
            process.roi_x0,
            process.roi_y0,
            process.roi_x1,
            process.roi_y1
        ],
        [-1, -2, 3, 4]
    );
    assert_eq!(
        [
            process.dod_x0,
            process.dod_y0,
            process.dod_x1,
            process.dod_y1
        ],
        [-5, -6, 7, 8]
    );
    assert_eq!(process.time, 24.5);
}

/// The union's arms, each written by the header and read through the one the
/// tag names. A point declares **one** element carrying every axis, which is
/// what `values[i].param == i` depends on: the host folds its own `_x` / `_y`
/// rows back before it writes the array.
#[test]
fn every_value_arm_is_read_through_the_kind_the_header_tagged_it_with() {
    let written = |kind: u32| {
        // SAFETY: `lfx_abi_write_value` is `tests/layout.c`'s, sets `param` and
        // `kind` always and the named arm for each kind this test passes.
        unsafe {
            let mut block = std::mem::MaybeUninit::<LfxValue>::zeroed();
            lfx_abi_write_value(block.as_mut_ptr(), kind);
            block.assume_init()
        }
    };

    let float = written(LFX_PARAM_FLOAT);
    assert_eq!(float.param, 2);
    assert_eq!(float.kind, LFX_PARAM_FLOAT);
    // SAFETY: the arm is the one `kind` names, which is the contract the tag
    // exists for - here and in each read below.
    assert_eq!(unsafe { float.v.f }, 0.5);

    assert_eq!(unsafe { written(LFX_PARAM_INT).v.i }, -5_000_000_000);
    assert!(unsafe { written(LFX_PARAM_BOOL).v.b });
    assert_eq!(unsafe { written(LFX_PARAM_CHOICE).v.choice }, 9);
    // Three kinds share the `f` arm and two the `i` arm, which the header says
    // beside each of them and a vendor has no other way of knowing: a SEED read
    // through `choice`, the only other unsigned arm, is right on this machine
    // for a small seed and wrong the moment one passes four billion.
    assert_eq!(unsafe { written(LFX_PARAM_SLIDER).v.f }, 0.5);
    assert_eq!(unsafe { written(LFX_PARAM_ANGLE).v.f }, 0.5);
    assert_eq!(unsafe { written(LFX_PARAM_SEED).v.i }, -5_000_000_000);
    assert_eq!(
        unsafe { written(LFX_PARAM_COLOUR).v.rgba },
        [0.25, 0.5, 0.75, 1.0]
    );
    // Both axes, in one element, from one declaration.
    assert_eq!(unsafe { written(LFX_PARAM_POINT2).v.xy }, [1.5, -2.5]);
    assert_eq!(unsafe { written(LFX_PARAM_POINT3).v.xyz }, [1.5, -2.5, 3.5]);

    // The two arms that carry fields of their own, read in the order the header
    // declares them. Swapped, `pt` would be the count and a plugin would
    // dereference the number two.
    let curve = written(LFX_PARAM_CURVE);
    // SAFETY: the curve arm, whose `pt` is a file static of `tests/layout.c`'s
    // holding `2 * n` floats and living for the length of the process.
    let points = unsafe {
        assert_eq!(curve.v.curve.n, 2);
        assert!(
            !curve.v.curve.pt.is_null(),
            "the points arrived as the count"
        );
        std::slice::from_raw_parts(curve.v.curve.pt, 2 * curve.v.curve.n as usize)
    };
    assert_eq!(points, [0.0, 0.25, 1.0, 0.75]);

    let file = written(LFX_PARAM_FILE);
    // SAFETY: the file arm, whose `path` is a string literal of
    // `tests/layout.c`'s.
    let path = unsafe {
        assert!(!file.v.file.path.is_null());
        std::ffi::CStr::from_ptr(file.v.file.path)
    };
    assert_eq!(path.to_str().expect("ASCII"), "/layout/probe.cube");

    // A kind the writer does not carry a value for is tagged unset rather than
    // left looking like something it is not.
    assert_eq!(written(LFX_PARAM_ACTION).kind, LFX_PARAM_UNSET);
}

/// The sink's fifteen slots, called through the mirror's own function-pointer
/// types with a record of each kind.
///
/// An offset pins where `declare_float` sits and says nothing about what it
/// takes: a mirror that paired it with an `LfxSliderParam` would move no number
/// and fail no assertion, while the host read a Float's `slider_min` as a
/// Slider's `range_min` and its `hard_min` sixteen bytes past the end of the
/// record the plugin passed. The call itself is the pin. Each record carries
/// its own `struct_size`, so a call handed the wrong kind fails on a number
/// rather than on a coincidence.
#[test]
fn every_sink_call_takes_the_record_the_header_pairs_it_with() {
    let id = std::ffi::CString::new("layout_id").expect("no interior NUL");
    let label = std::ffi::CString::new("Layout label").expect("no interior NUL");

    // SAFETY: `lfx_abi_layout_sink` is `tests/layout.c`'s, and answers the
    // address of one file-static `lfx_describe_sink` whose every slot that file
    // fills in. It lives for the length of the process.
    let sink = unsafe { lfx_abi_layout_sink() };
    let sink_size = size_of::<LfxDescribeSink>() as u32;
    // SAFETY: as above; the table is initialised before `main`.
    let table = unsafe { &*sink };
    assert_eq!(table.struct_size, sink_size);

    // What every declaration call has to show: it ran once, the sink arrived
    // first, and the record that arrived is the kind the mirror declared.
    let arrived = |bytes: usize| {
        let seen = recorded();
        assert_eq!(seen.calls, 1, "the sink slot was not the one called");
        assert_eq!(
            seen.sink_size, sink_size,
            "the sink did not arrive as the first argument"
        );
        assert_eq!(
            seen.struct_size as usize, bytes,
            "the record that arrived is not the kind the header pairs with this call"
        );
        assert_eq!(seen.text.as_deref(), Some("layout_id"));
        seen
    };

    let declare_float = table.declare_float.expect("the header fills every slot");
    let float = LfxFloatParam {
        struct_size: size_of::<LfxFloatParam>() as u32,
        unit: LFX_UNIT_PX,
        flags: LFX_PARAM_FLAG_STATIC,
        bounds: LFX_BOUND_MIN,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_value: 0.25,
        slider_min: -1.0,
        slider_max: 1.0,
        hard_min: -2.0,
        hard_max: 2.0,
    };
    forget();
    // SAFETY: the sink's own slot, called with the sink and a whole record of
    // the kind the header pairs it with - the plugin's call, spelled out. The
    // same holds for every call below.
    assert!(unsafe { declare_float(sink, &float) });
    let seen = arrived(size_of::<LfxFloatParam>());
    assert_eq!(seen.unit, LFX_UNIT_PX);
    assert_eq!(seen.number, 0.25);
    assert_eq!(seen.whole, i64::from(LFX_BOUND_MIN));

    let declare_slider = table.declare_slider.expect("the header fills every slot");
    let slider = LfxSliderParam {
        struct_size: size_of::<LfxSliderParam>() as u32,
        unit: LFX_UNIT_SECONDS,
        flags: LFX_PARAM_FLAG_NONE,
        log: 1,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_value: 0.75,
        range_min: 0.125,
        range_max: 4.0,
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_slider(sink, &slider) });
    let seen = arrived(size_of::<LfxSliderParam>());
    assert_eq!(seen.unit, LFX_UNIT_SECONDS);
    assert_eq!(seen.number, 4.0);
    assert_eq!(seen.whole, 1);

    let declare_int = table.declare_int.expect("the header fills every slot");
    let whole = LfxIntParam {
        struct_size: size_of::<LfxIntParam>() as u32,
        unit: LFX_UNIT_FRAMES,
        flags: LFX_PARAM_FLAG_NONE,
        bounds: LFX_BOUND_MAX,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_value: 0,
        slider_min: -1,
        slider_max: 1,
        hard_min: -1,
        hard_max: 7_000_000_000,
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_int(sink, &whole) });
    let seen = arrived(size_of::<LfxIntParam>());
    assert_eq!(seen.whole, 7_000_000_000);

    let declare_angle = table.declare_angle.expect("the header fills every slot");
    let angle = LfxAngleParam {
        struct_size: size_of::<LfxAngleParam>() as u32,
        unit: LFX_UNIT_DEGREES,
        flags: LFX_PARAM_FLAG_NONE,
        reserved_0: 0,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_value: 45.0,
        dial_step: 7.5,
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_angle(sink, &angle) });
    let seen = arrived(size_of::<LfxAngleParam>());
    assert_eq!(seen.unit, LFX_UNIT_DEGREES);
    assert_eq!(seen.number, 7.5);

    let declare_bool = table.declare_bool.expect("the header fills every slot");
    let switch = LfxBoolParam {
        struct_size: size_of::<LfxBoolParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_NONE,
        default_value: 1,
        id: id.as_ptr(),
        label: label.as_ptr(),
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_bool(sink, &switch) });
    let seen = arrived(size_of::<LfxBoolParam>());
    assert_eq!(seen.whole, 1);

    let declare_choice = table.declare_choice.expect("the header fills every slot");
    let dividers: [u32; 2] = [0, 1];
    let options: [*const std::ffi::c_char; 2] = [id.as_ptr(), label.as_ptr()];
    let choice = LfxChoiceParam {
        struct_size: size_of::<LfxChoiceParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_NONE,
        default_index: 1,
        option_count: 2,
        divider_count: 2,
        id: id.as_ptr(),
        label: label.as_ptr(),
        options: options.as_ptr(),
        dividers_after: dividers.as_ptr(),
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_choice(sink, &choice) });
    let seen = arrived(size_of::<LfxChoiceParam>());
    assert_eq!(seen.whole, 2);

    let declare_colour = table.declare_colour.expect("the header fills every slot");
    let colour = LfxColourParam {
        struct_size: size_of::<LfxColourParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_NONE,
        reserved_0: 0,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_rgba: [0.125, 0.25, 0.5, 1.0],
        range_min: -0.5,
        range_max: 1.5,
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_colour(sink, &colour) });
    let seen = arrived(size_of::<LfxColourParam>());
    assert_eq!(seen.number, 0.5);

    let declare_seed = table.declare_seed.expect("the header fills every slot");
    let seed = LfxSeedParam {
        struct_size: size_of::<LfxSeedParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_HIDDEN,
        reserved_0: 0,
        id: id.as_ptr(),
        label: label.as_ptr(),
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_seed(sink, &seed) });
    let seen = arrived(size_of::<LfxSeedParam>());
    assert_eq!(seen.whole, i64::from(LFX_PARAM_FLAG_HIDDEN));

    let declare_point2 = table.declare_point2.expect("the header fills every slot");
    let point2 = LfxPoint2Param {
        struct_size: size_of::<LfxPoint2Param>() as u32,
        unit: LFX_UNIT_PX,
        flags: LFX_PARAM_FLAG_NONE,
        reserved_0: 0,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_x: 1.25,
        default_y: -2.75,
        slider_min: -10.0,
        slider_max: 10.0,
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_point2(sink, &point2) });
    let seen = arrived(size_of::<LfxPoint2Param>());
    assert_eq!(seen.number, -2.75);

    let declare_point3 = table.declare_point3.expect("the header fills every slot");
    let point3 = LfxPoint3Param {
        struct_size: size_of::<LfxPoint3Param>() as u32,
        unit: LFX_UNIT_PX,
        flags: LFX_PARAM_FLAG_NONE,
        reserved_0: 0,
        id: id.as_ptr(),
        label: label.as_ptr(),
        default_x: 1.25,
        default_y: -2.75,
        default_z: 3.5,
        slider_min: -10.0,
        slider_max: 10.0,
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_point3(sink, &point3) });
    let seen = arrived(size_of::<LfxPoint3Param>());
    assert_eq!(seen.number, 3.5);

    let declare_curve = table.declare_curve.expect("the header fills every slot");
    let points: [f32; 4] = [0.0, 0.25, 1.0, 0.75];
    let tone = LfxCurveParam {
        struct_size: size_of::<LfxCurveParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_STATIC,
        point_count: 2,
        id: id.as_ptr(),
        label: label.as_ptr(),
        points: points.as_ptr(),
    };
    forget();
    // SAFETY: as above, and `points` outlives the call.
    assert!(unsafe { declare_curve(sink, &tone) });
    let seen = arrived(size_of::<LfxCurveParam>());
    assert_eq!(seen.number, 0.25);
    assert_eq!(seen.whole, 2);

    let declare_file = table.declare_file.expect("the header fills every slot");
    let filters: [*const std::ffi::c_char; 1] = [id.as_ptr()];
    let chooser = LfxFileParam {
        struct_size: size_of::<LfxFileParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_STATIC,
        filter_count: 1,
        id: id.as_ptr(),
        label: label.as_ptr(),
        filter: filters.as_ptr(),
        filter_name: label.as_ptr(),
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_file(sink, &chooser) });
    let seen = arrived(size_of::<LfxFileParam>());
    assert_eq!(seen.whole, 1);

    let declare_action = table.declare_action.expect("the header fills every slot");
    let button = LfxActionParam {
        struct_size: size_of::<LfxActionParam>() as u32,
        unit: LFX_UNIT_RAW,
        flags: LFX_PARAM_FLAG_NONE,
        reserved_0: 0,
        id: id.as_ptr(),
        label: label.as_ptr(),
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { declare_action(sink, &button) });
    arrived(size_of::<LfxActionParam>());

    // A heading, which carries no unit, and the call that closes it, which
    // carries nothing at all - and answers `false`, so both answers are read.
    let group_begin = table.group_begin.expect("the header fills every slot");
    let heading = LfxGroupParam {
        struct_size: size_of::<LfxGroupParam>() as u32,
        flags: LFX_PARAM_FLAG_HIDDEN,
        id: id.as_ptr(),
        label: label.as_ptr(),
    };
    forget();
    // SAFETY: as above.
    assert!(unsafe { group_begin(sink, &heading) });
    let seen = arrived(size_of::<LfxGroupParam>());
    assert_eq!(seen.whole, i64::from(LFX_PARAM_FLAG_HIDDEN));

    let group_end = table.group_end.expect("the header fills every slot");
    forget();
    // SAFETY: as above, and this one takes the sink alone.
    assert!(!unsafe { group_end(sink) });
    let seen = recorded();
    assert_eq!(seen.calls, 1);
    assert_eq!(seen.sink_size, sink_size);
}

/// The other three tables, called the way the host and the plugin call them.
///
/// `LfxEntry::create` takes the host table first and the effect id second, and
/// nothing but a call says so: swapped, every plugin is handed the host where
/// it expects its own id and reads wildly on the first instantiation of any LFX
/// effect. The same goes for what each one answers - `process` is signed,
/// `get_extension` is a pointer, and the three the *plugin* supplies are `u32`s
/// in which non-zero is true rather than C `bool`s the host would have to
/// trust a stranger's compiler to have normalised.
#[test]
fn the_entry_plugin_and_host_tables_are_called_as_the_header_declares_them() {
    // SAFETY: each getter is `tests/layout.c`'s and answers the address of one
    // file-static table it fills in itself, living for the length of the
    // process.
    let (entry, plugin, host, sink) = unsafe {
        (
            &*lfx_abi_layout_entry(),
            lfx_abi_layout_plugin(),
            &*lfx_abi_layout_host(),
            lfx_abi_layout_sink(),
        )
    };
    assert_eq!(entry.struct_size as usize, size_of::<LfxEntry>());
    assert_eq!(entry.abi_version, LFX_ABI_VERSION);
    assert_eq!(host.struct_size as usize, size_of::<LfxHost>());

    // The entry, in the order the header pins: init, count, descriptor, create.
    let bundle = std::ffi::CString::new("/layout/bundle").expect("no interior NUL");
    forget();
    // SAFETY: the entry's own slot, called once with a NUL-terminated path that
    // outlives the call - the host's own call, spelled out. The same holds
    // below.
    let loaded = unsafe { entry.init.expect("the header fills every slot")(bundle.as_ptr()) };
    // Seven, and the comparison is the point: a mirror answering a `bool` here
    // could not be compared with a number at all, which is the whole of why the
    // plugin's answers are `u32`.
    assert_eq!(loaded, 7, "the entry's init does not answer a u32");
    assert_eq!(recorded().text.as_deref(), Some("/layout/bundle"));

    forget();
    // SAFETY: as above.
    assert_eq!(
        unsafe { entry.count.expect("the header fills every slot")() },
        2
    );

    let descriptor = entry.descriptor.expect("the header fills every slot");
    forget();
    // SAFETY: as above, with an index inside the count just read.
    let second = unsafe { descriptor(1) };
    assert!(!second.is_null());
    // SAFETY: a non-null answer is one of `tests/layout.c`'s own statics.
    assert_eq!(
        unsafe { (*second).major },
        22,
        "the descriptor call was handed an index it did not ask for"
    );
    assert_eq!(recorded().whole_u32, 1);
    // SAFETY: as above; past the end is the header's documented null.
    assert!(unsafe { descriptor(2) }.is_null());

    forget();
    let effect = std::ffi::CString::new("layout.first").expect("no interior NUL");
    // SAFETY: as above, with the host table and an id that both outlive it.
    let made = unsafe { entry.create.expect("the header fills every slot")(host, effect.as_ptr()) };
    assert!(!made.is_null());
    let seen = recorded();
    assert_eq!(
        seen.struct_size as usize,
        size_of::<LfxHost>(),
        "create read the effect id where the host table belongs"
    );
    assert_eq!(seen.text.as_deref(), Some("layout.first"));

    // The instance. `init` and `describe` are the other two answers a plugin
    // supplies, and `process` the one status it returns.
    // SAFETY: the table is `tests/layout.c`'s own static, initialised before
    // `main` and never reassigned.
    let table = unsafe { &*plugin };
    assert_eq!(table.struct_size as usize, size_of::<LfxPlugin>());

    forget();
    // SAFETY: as above, with the instance the header says it takes.
    let prepared = unsafe { table.init.expect("the header fills every slot")(plugin) };
    assert_eq!(prepared, 5, "an instance's init does not answer a u32");
    assert_eq!(recorded().struct_size as usize, size_of::<LfxPlugin>());

    forget();
    // SAFETY: as above, with the instance first and the sink second.
    let described = unsafe { table.describe.expect("the header fills every slot")(plugin, sink) };
    assert_eq!(described, 9, "describe does not answer a u32");
    let seen = recorded();
    assert_eq!(seen.struct_size as usize, size_of::<LfxPlugin>());
    assert_eq!(
        seen.sink_size as usize,
        size_of::<LfxDescribeSink>(),
        "describe read the instance where the sink belongs"
    );

    let mut request = std::mem::MaybeUninit::<LfxProcess>::zeroed();
    // SAFETY: every field of `LfxProcess` is a number, a raw pointer or an
    // optional function pointer, so a zeroed block is a valid value; the two
    // fields this call reads are written before it.
    let request = unsafe {
        let raw = request.as_mut_ptr();
        (*raw).struct_size = size_of::<LfxProcess>() as u32;
        (*raw).value_stride = size_of::<LfxValue>() as u32;
        (*raw).value_count = 4;
        (*raw).time = 24.5;
        request.assume_init()
    };
    forget();
    // SAFETY: as above, with a whole request that outlives the call.
    let status = unsafe { table.process.expect("the header fills every slot")(plugin, &request) };
    // Not an `LfxStatus`: it is here to say the answer is signed, which a
    // mirror reading a `u32` would turn into four billion and something.
    assert_eq!(status, -12_345, "process does not answer a signed i32");
    let seen = recorded();
    assert_eq!(seen.whole_u32, 4);
    assert_eq!(seen.number, 24.5);

    let plugin_extension = table.get_extension.expect("the header fills every slot");
    forget();
    // SAFETY: as above, with a NUL-terminated id from the mirror's own
    // constant.
    let offered = unsafe { plugin_extension(plugin, LFX_EXT_TEMPORAL.as_ptr().cast(), 3) };
    // SAFETY: `lfx_abi_layout_extension` answers the address the table above
    // answers when it has what was asked for.
    assert_eq!(offered.cast(), unsafe { lfx_abi_layout_extension() });
    let seen = recorded();
    assert_eq!(seen.whole_u32, 3, "the version did not arrive third");
    assert_eq!(seen.text.as_deref(), Some("lfx.temporal"));
    // SAFETY: as above. A missing extension is a null, never a status.
    assert!(unsafe { plugin_extension(plugin, LFX_EXT_OVERLAY.as_ptr().cast(), 1) }.is_null());

    forget();
    // SAFETY: as above; nothing is running on this instance.
    unsafe { table.destroy.expect("the header fills every slot")(plugin) };
    assert_eq!(recorded().struct_size as usize, size_of::<LfxPlugin>());

    // And the host's own two, which the plugin calls.
    forget();
    // SAFETY: as above, through the host table this test was handed.
    let from_the_host = unsafe {
        host.get_extension.expect("the header fills every slot")(
            host,
            LFX_EXT_TEMPORAL.as_ptr().cast(),
            3,
        )
    };
    // SAFETY: as above.
    assert_eq!(from_the_host.cast(), unsafe { lfx_abi_layout_extension() });
    let seen = recorded();
    assert_eq!(seen.struct_size as usize, size_of::<LfxHost>());
    assert_eq!(seen.whole_u32, 3);

    let message = std::ffi::CString::new("layout probe").expect("no interior NUL");
    forget();
    // SAFETY: as above, with a level and a NUL-terminated line that outlives
    // the call.
    unsafe {
        host.log.expect("the header fills every slot")(host, LFX_LOG_WARN, message.as_ptr());
    }
    let seen = recorded();
    assert_eq!(
        seen.whole_u32, LFX_LOG_WARN,
        "the level did not arrive second"
    );
    assert_eq!(seen.text.as_deref(), Some("layout probe"));

    forget();
    // SAFETY: as above, and last, as the header requires.
    unsafe { entry.deinit.expect("the header fills every slot")() };
    assert_eq!(recorded().calls, 1);
}
