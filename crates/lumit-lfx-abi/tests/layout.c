/*
 * The C half of the layout suite.
 *
 * SPDX-License-Identifier: MIT
 *
 * In plain terms: the same numbers `tests/layout.rs` asserts for the Rust
 * mirror, asserted here against the header itself. A field that moved, grew or
 * changed places fails one side or the other, so the two halves of the ABI
 * cannot drift apart quietly.
 *
 * Offsets are only half of the drift, though, so this file does three more
 * things. It emits the header's own constants in a fixed order, and its own
 * spelling of every string, for the Rust half to compare one by one - a
 * renumbered enumerator and a misspelt extension id move no field. It writes
 * each struct of plain data from the header's own declarations, for the Rust
 * half to read back through the mirror, because a type that drifted to another
 * of the same width - a `float` read as a `u32`, an `int32_t` read as a `u32` -
 * moves no field either. And, for the four structs that are tables of function
 * pointers rather than data, it defines one real instance of each from these
 * same declarations, for the Rust half to *call* through the mirror's own
 * function-pointer types: an offset pins where a pointer sits and says nothing
 * about its arity, its argument order, its argument types or what it answers.
 *
 * The assertions are static, so this is a build failure rather than a test
 * failure. `lfx_abi_layout_assertions_compiled` exists so the Rust side can
 * prove this translation unit was actually compiled and linked: a static
 * assertion nobody compiles asserts nothing.
 *
 * The numbers are for a 64-bit target - the only kind Lumit ships (docs/05) -
 * and the first assertion says so, so a 32-bit build fails on the pointer size
 * rather than on eleven confusing offsets.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "lfx.h"

/* `_Static_assert` needs C11 and MSVC only offers it under `/std:c11`; the
 * negative-array trick needs nothing at all and says the same thing. */
#define LFX_ASSERT_CONCAT_(a, b) a##b
#define LFX_ASSERT_CONCAT(a, b) LFX_ASSERT_CONCAT_(a, b)
#define LFX_STATIC_ASSERT(cond) \
    typedef char LFX_ASSERT_CONCAT(lfx_layout_assert_, __LINE__)[(cond) ? 1 : -1]

#define LFX_SIZE(type, bytes) LFX_STATIC_ASSERT(sizeof(type) == (bytes))
#define LFX_AT(type, field, bytes) LFX_STATIC_ASSERT(offsetof(type, field) == (bytes))

LFX_STATIC_ASSERT(sizeof(void *) == 8);
LFX_STATIC_ASSERT(sizeof(bool) == 1);
LFX_STATIC_ASSERT(LFX_ABI_VERSION == 1u);

/* ------------------------------------------------------------- the traits -- */

LFX_SIZE(lfx_traits, 36);
LFX_AT(lfx_traits, struct_size, 0);
LFX_AT(lfx_traits, cost, 4);
LFX_AT(lfx_traits, roi_kind, 8);
LFX_AT(lfx_traits, roi_padding_px, 12);
LFX_AT(lfx_traits, temporal_lo, 16);
LFX_AT(lfx_traits, temporal_hi, 20);
LFX_AT(lfx_traits, alpha, 24);
LFX_AT(lfx_traits, flags, 28);
LFX_AT(lfx_traits, scratch_bytes_per_megapixel, 32);

/* ------------------------------------------------- the parameter records -- */

LFX_SIZE(lfx_float_param, 72);
LFX_AT(lfx_float_param, struct_size, 0);
LFX_AT(lfx_float_param, unit, 4);
LFX_AT(lfx_float_param, flags, 8);
LFX_AT(lfx_float_param, bounds, 12);
LFX_AT(lfx_float_param, id, 16);
LFX_AT(lfx_float_param, label, 24);
LFX_AT(lfx_float_param, default_value, 32);
LFX_AT(lfx_float_param, slider_min, 40);
LFX_AT(lfx_float_param, slider_max, 48);
LFX_AT(lfx_float_param, hard_min, 56);
LFX_AT(lfx_float_param, hard_max, 64);

LFX_SIZE(lfx_slider_param, 56);
LFX_AT(lfx_slider_param, struct_size, 0);
LFX_AT(lfx_slider_param, unit, 4);
LFX_AT(lfx_slider_param, flags, 8);
LFX_AT(lfx_slider_param, log, 12);
LFX_AT(lfx_slider_param, id, 16);
LFX_AT(lfx_slider_param, label, 24);
LFX_AT(lfx_slider_param, default_value, 32);
LFX_AT(lfx_slider_param, range_min, 40);
LFX_AT(lfx_slider_param, range_max, 48);

LFX_SIZE(lfx_int_param, 72);
LFX_AT(lfx_int_param, struct_size, 0);
LFX_AT(lfx_int_param, unit, 4);
LFX_AT(lfx_int_param, flags, 8);
LFX_AT(lfx_int_param, bounds, 12);
LFX_AT(lfx_int_param, id, 16);
LFX_AT(lfx_int_param, label, 24);
LFX_AT(lfx_int_param, default_value, 32);
LFX_AT(lfx_int_param, slider_min, 40);
LFX_AT(lfx_int_param, slider_max, 48);
LFX_AT(lfx_int_param, hard_min, 56);
LFX_AT(lfx_int_param, hard_max, 64);

LFX_SIZE(lfx_angle_param, 48);
LFX_AT(lfx_angle_param, struct_size, 0);
LFX_AT(lfx_angle_param, unit, 4);
LFX_AT(lfx_angle_param, flags, 8);
LFX_AT(lfx_angle_param, reserved_0, 12);
LFX_AT(lfx_angle_param, id, 16);
LFX_AT(lfx_angle_param, label, 24);
LFX_AT(lfx_angle_param, default_value, 32);
LFX_AT(lfx_angle_param, dial_step, 40);

LFX_SIZE(lfx_bool_param, 32);
LFX_AT(lfx_bool_param, struct_size, 0);
LFX_AT(lfx_bool_param, unit, 4);
LFX_AT(lfx_bool_param, flags, 8);
LFX_AT(lfx_bool_param, default_value, 12);
LFX_AT(lfx_bool_param, id, 16);
LFX_AT(lfx_bool_param, label, 24);

LFX_SIZE(lfx_choice_param, 56);
LFX_AT(lfx_choice_param, struct_size, 0);
LFX_AT(lfx_choice_param, unit, 4);
LFX_AT(lfx_choice_param, flags, 8);
LFX_AT(lfx_choice_param, default_index, 12);
LFX_AT(lfx_choice_param, option_count, 16);
LFX_AT(lfx_choice_param, divider_count, 20);
LFX_AT(lfx_choice_param, id, 24);
LFX_AT(lfx_choice_param, label, 32);
LFX_AT(lfx_choice_param, options, 40);
LFX_AT(lfx_choice_param, dividers_after, 48);

LFX_SIZE(lfx_colour_param, 80);
LFX_AT(lfx_colour_param, struct_size, 0);
LFX_AT(lfx_colour_param, unit, 4);
LFX_AT(lfx_colour_param, flags, 8);
LFX_AT(lfx_colour_param, reserved_0, 12);
LFX_AT(lfx_colour_param, id, 16);
LFX_AT(lfx_colour_param, label, 24);
LFX_AT(lfx_colour_param, default_rgba, 32);
LFX_AT(lfx_colour_param, range_min, 64);
LFX_AT(lfx_colour_param, range_max, 72);

LFX_SIZE(lfx_seed_param, 32);
LFX_AT(lfx_seed_param, struct_size, 0);
LFX_AT(lfx_seed_param, unit, 4);
LFX_AT(lfx_seed_param, flags, 8);
LFX_AT(lfx_seed_param, reserved_0, 12);
LFX_AT(lfx_seed_param, id, 16);
LFX_AT(lfx_seed_param, label, 24);

LFX_SIZE(lfx_point2_param, 64);
LFX_AT(lfx_point2_param, struct_size, 0);
LFX_AT(lfx_point2_param, unit, 4);
LFX_AT(lfx_point2_param, flags, 8);
LFX_AT(lfx_point2_param, reserved_0, 12);
LFX_AT(lfx_point2_param, id, 16);
LFX_AT(lfx_point2_param, label, 24);
LFX_AT(lfx_point2_param, default_x, 32);
LFX_AT(lfx_point2_param, default_y, 40);
LFX_AT(lfx_point2_param, slider_min, 48);
LFX_AT(lfx_point2_param, slider_max, 56);

LFX_SIZE(lfx_point3_param, 72);
LFX_AT(lfx_point3_param, struct_size, 0);
LFX_AT(lfx_point3_param, unit, 4);
LFX_AT(lfx_point3_param, flags, 8);
LFX_AT(lfx_point3_param, reserved_0, 12);
LFX_AT(lfx_point3_param, id, 16);
LFX_AT(lfx_point3_param, label, 24);
LFX_AT(lfx_point3_param, default_x, 32);
LFX_AT(lfx_point3_param, default_y, 40);
LFX_AT(lfx_point3_param, default_z, 48);
LFX_AT(lfx_point3_param, slider_min, 56);
LFX_AT(lfx_point3_param, slider_max, 64);

LFX_SIZE(lfx_curve_param, 40);
LFX_AT(lfx_curve_param, struct_size, 0);
LFX_AT(lfx_curve_param, unit, 4);
LFX_AT(lfx_curve_param, flags, 8);
LFX_AT(lfx_curve_param, point_count, 12);
LFX_AT(lfx_curve_param, id, 16);
LFX_AT(lfx_curve_param, label, 24);
LFX_AT(lfx_curve_param, points, 32);

LFX_SIZE(lfx_file_param, 48);
LFX_AT(lfx_file_param, struct_size, 0);
LFX_AT(lfx_file_param, unit, 4);
LFX_AT(lfx_file_param, flags, 8);
LFX_AT(lfx_file_param, filter_count, 12);
LFX_AT(lfx_file_param, id, 16);
LFX_AT(lfx_file_param, label, 24);
LFX_AT(lfx_file_param, filter, 32);
LFX_AT(lfx_file_param, filter_name, 40);

LFX_SIZE(lfx_action_param, 32);
LFX_AT(lfx_action_param, struct_size, 0);
LFX_AT(lfx_action_param, unit, 4);
LFX_AT(lfx_action_param, flags, 8);
LFX_AT(lfx_action_param, reserved_0, 12);
LFX_AT(lfx_action_param, id, 16);
LFX_AT(lfx_action_param, label, 24);

LFX_SIZE(lfx_group_param, 24);
LFX_AT(lfx_group_param, struct_size, 0);
LFX_AT(lfx_group_param, flags, 4);
LFX_AT(lfx_group_param, id, 8);
LFX_AT(lfx_group_param, label, 16);

/* ------------------------------------------------------ the describe sink -- */

LFX_SIZE(lfx_describe_sink, 136);
LFX_AT(lfx_describe_sink, struct_size, 0);
LFX_AT(lfx_describe_sink, sink_data, 8);
LFX_AT(lfx_describe_sink, declare_float, 16);
LFX_AT(lfx_describe_sink, declare_slider, 24);
LFX_AT(lfx_describe_sink, declare_int, 32);
LFX_AT(lfx_describe_sink, declare_angle, 40);
LFX_AT(lfx_describe_sink, declare_bool, 48);
LFX_AT(lfx_describe_sink, declare_choice, 56);
LFX_AT(lfx_describe_sink, declare_colour, 64);
LFX_AT(lfx_describe_sink, declare_seed, 72);
LFX_AT(lfx_describe_sink, declare_point2, 80);
LFX_AT(lfx_describe_sink, declare_point3, 88);
LFX_AT(lfx_describe_sink, declare_curve, 96);
LFX_AT(lfx_describe_sink, declare_file, 104);
LFX_AT(lfx_describe_sink, declare_action, 112);
LFX_AT(lfx_describe_sink, group_begin, 120);
LFX_AT(lfx_describe_sink, group_end, 128);

/* ------------------------------------------------------------- the values -- */

/* The one struct with no size prefix. What both sides walk by is
 * `lfx_process.value_stride`, and the union's own size is pinned here so a new
 * arm cannot widen the element behind a plugin's back. */
LFX_SIZE(lfx_value, 24);
LFX_AT(lfx_value, param, 0);
LFX_AT(lfx_value, kind, 4);
LFX_AT(lfx_value, v, 8);
LFX_STATIC_ASSERT(sizeof(((lfx_value *)0)->v) == 16);

/* The two arms with fields of their own. A size alone does not pin them: the
 * curve arm is sixteen bytes and eight-aligned whichever order its pointer and
 * its count are written in, so a swap would leave every assertion above green
 * and hand a plugin the count where the points belong. `offsetof` takes a
 * member designator, so the arms are named in full here rather than by their
 * struct, which the header does not give a tag. */
LFX_AT(lfx_value, v.curve.pt, 8);
LFX_AT(lfx_value, v.curve.n, 16);
LFX_STATIC_ASSERT(sizeof(((lfx_value *)0)->v.curve) == 16);
LFX_AT(lfx_value, v.file.path, 8);
LFX_STATIC_ASSERT(sizeof(((lfx_value *)0)->v.file) == 8);

/* -------------------------------------------------------------- the frame -- */

LFX_SIZE(lfx_frame, 48);
LFX_AT(lfx_frame, struct_size, 0);
LFX_AT(lfx_frame, format, 4);
LFX_AT(lfx_frame, width, 8);
LFX_AT(lfx_frame, height, 12);
LFX_AT(lfx_frame, row_bytes, 16);
LFX_AT(lfx_frame, origin_x, 20);
LFX_AT(lfx_frame, origin_y, 24);
LFX_AT(lfx_frame, reserved_0, 28);
LFX_AT(lfx_frame, data, 32);
LFX_AT(lfx_frame, time, 40);

LFX_SIZE(lfx_process, 96);
LFX_AT(lfx_process, struct_size, 0);
LFX_AT(lfx_process, pixel_format, 4);
LFX_AT(lfx_process, value_stride, 8);
LFX_AT(lfx_process, value_count, 12);
LFX_AT(lfx_process, roi_x0, 16);
LFX_AT(lfx_process, roi_y0, 20);
LFX_AT(lfx_process, roi_x1, 24);
LFX_AT(lfx_process, roi_y1, 28);
LFX_AT(lfx_process, dod_x0, 32);
LFX_AT(lfx_process, dod_y0, 36);
LFX_AT(lfx_process, dod_x1, 40);
LFX_AT(lfx_process, dod_y1, 44);
LFX_AT(lfx_process, time, 48);
LFX_AT(lfx_process, values, 56);
LFX_AT(lfx_process, input, 64);
LFX_AT(lfx_process, output, 72);
LFX_AT(lfx_process, cancelled, 80);
LFX_AT(lfx_process, host_context, 88);

/* --------------------------------------------------------------- the host -- */

LFX_SIZE(lfx_host, 32);
LFX_AT(lfx_host, struct_size, 0);
LFX_AT(lfx_host, abi_version, 4);
LFX_AT(lfx_host, host_data, 8);
LFX_AT(lfx_host, get_extension, 16);
LFX_AT(lfx_host, log, 24);

/* -------------------------------------------------------- the descriptor -- */

LFX_SIZE(lfx_descriptor, 88);
LFX_AT(lfx_descriptor, struct_size, 0);
LFX_AT(lfx_descriptor, id, 8);
LFX_AT(lfx_descriptor, name, 16);
LFX_AT(lfx_descriptor, vendor, 24);
LFX_AT(lfx_descriptor, major, 32);
LFX_AT(lfx_descriptor, minor, 36);
LFX_AT(lfx_descriptor, patch, 40);
LFX_AT(lfx_descriptor, categories, 48);
LFX_AT(lfx_descriptor, category_count, 56);
LFX_AT(lfx_descriptor, traits, 64);
LFX_AT(lfx_descriptor, required_extensions, 72);
LFX_AT(lfx_descriptor, required_extension_count, 80);

/* ------------------------------------------------------------ the plugin -- */

LFX_SIZE(lfx_plugin, 56);
LFX_AT(lfx_plugin, struct_size, 0);
LFX_AT(lfx_plugin, plugin_data, 8);
LFX_AT(lfx_plugin, init, 16);
LFX_AT(lfx_plugin, destroy, 24);
LFX_AT(lfx_plugin, describe, 32);
LFX_AT(lfx_plugin, process, 40);
LFX_AT(lfx_plugin, get_extension, 48);

/* ------------------------------------------------------------- the entry -- */

LFX_SIZE(lfx_entry, 48);
LFX_AT(lfx_entry, struct_size, 0);
LFX_AT(lfx_entry, abi_version, 4);
LFX_AT(lfx_entry, init, 8);
LFX_AT(lfx_entry, deinit, 16);
LFX_AT(lfx_entry, count, 24);
LFX_AT(lfx_entry, descriptor, 32);
LFX_AT(lfx_entry, create, 40);

/* ------------------------------------------------------- the linked proof -- */

/* Answers the header's own `LFX_ABI_VERSION`. `tests/layout.rs` calls it and
 * compares it with the mirror's constant, which proves two things at once: this
 * file was compiled (so its assertions were checked), and the two halves agree
 * on the version they are the two halves of. */
uint32_t lfx_abi_layout_assertions_compiled(void);

uint32_t lfx_abi_layout_assertions_compiled(void) {
    return LFX_ABI_VERSION;
}

/* ---------------------------------------------------- the header's values -- */

/* An offset pins where a field sits and says nothing about what a value means.
 * Renumbering an enumerator or misspelling an extension id moves no field, so a
 * suite of offsets alone stays green while a plugin compiled against the header
 * declares a kind the host reads as something else, or asks for an extension
 * the host has never heard of.
 *
 * So the header's own constants are emitted here, in one fixed order, and
 * `tests/layout.rs` writes the mirror's constants in the same order and
 * compares them one by one. The order is the header's: each enumeration as it
 * is declared, each constant as it appears. A constant added to one half and
 * not the other changes the count, which is the first thing the Rust side
 * checks. */
static const uint32_t lfx_abi_constant_table[] = {
    LFX_ABI_VERSION,

    /* the ceilings */
    LFX_MAX_STRING_BYTES, LFX_MAX_LOG_BYTES, LFX_MAX_CATEGORIES, LFX_MAX_OPTIONS,
    LFX_MAX_DIVIDERS, LFX_MAX_FILTERS, LFX_MAX_REQUIRED_EXTENSIONS,
    LFX_MAX_EFFECTS_PER_BUNDLE, LFX_MAX_PARAMS, LFX_MIN_CURVE_POINTS,
    LFX_MAX_CURVE_POINTS, (uint32_t)LFX_MAX_TEMPORAL_WINDOW,

    /* lfx_param_kind */
    LFX_PARAM_UNSET, LFX_PARAM_FLOAT, LFX_PARAM_SLIDER, LFX_PARAM_INT,
    LFX_PARAM_BOOL, LFX_PARAM_CHOICE, LFX_PARAM_COLOUR, LFX_PARAM_ANGLE,
    LFX_PARAM_SEED, LFX_PARAM_POINT2, LFX_PARAM_POINT3, LFX_PARAM_CURVE,
    LFX_PARAM_FILE, LFX_PARAM_ACTION, LFX_PARAM_GROUP, LFX_PARAM_PATH,
    LFX_PARAM_STRING,

    /* lfx_unit */
    LFX_UNIT_UNSET, LFX_UNIT_RAW, LFX_UNIT_PERCENT, LFX_UNIT_PCT_DIAG,
    LFX_UNIT_PX, LFX_UNIT_DEGREES, LFX_UNIT_SECONDS, LFX_UNIT_FRAMES,

    /* lfx_category */
    LFX_CATEGORY_UNSET, LFX_CATEGORY_BLUR_SHARPEN, LFX_CATEGORY_COLOUR,
    LFX_CATEGORY_DISTORTION, LFX_CATEGORY_GENERATE, LFX_CATEGORY_STYLISE,
    LFX_CATEGORY_TEMPORAL, LFX_CATEGORY_TRANSITION, LFX_CATEGORY_UTILITY,

    /* lfx_pixel_format */
    LFX_PIXEL_UNSET, LFX_RGBA_F16, LFX_RGBA_F32,

    /* lfx_cost */
    LFX_COST_UNSET, LFX_COST_TRIVIAL, LFX_COST_CHEAP, LFX_COST_MODERATE,
    LFX_COST_HEAVY,

    /* lfx_roi_kind */
    LFX_ROI_UNSET, LFX_ROI_EXACT, LFX_ROI_PADDED, LFX_ROI_FULL_FRAME,

    /* lfx_alpha */
    LFX_ALPHA_UNSET, LFX_ALPHA_PREMULTIPLIED, LFX_ALPHA_STRAIGHT,

    /* lfx_trait_flags */
    LFX_TRAIT_NONE, LFX_TRAIT_SEEDED, LFX_TRAIT_THREAD_UNSAFE,
    LFX_TRAIT_CANCELLABLE,

    /* lfx_param_flags */
    LFX_PARAM_FLAG_NONE, LFX_PARAM_FLAG_STATIC, LFX_PARAM_FLAG_HIDDEN,

    /* lfx_bounds */
    LFX_BOUND_NONE, LFX_BOUND_MIN, LFX_BOUND_MAX,

    /* lfx_log_level */
    LFX_LOG_UNSET, LFX_LOG_ERROR, LFX_LOG_WARN, LFX_LOG_INFO, LFX_LOG_DEBUG,
    LFX_LOG_TRACE,

    /* lfx_status, which is signed and has no negative member */
    (uint32_t)LFX_STATUS_OK, (uint32_t)LFX_STATUS_FAILED,
    (uint32_t)LFX_STATUS_CANCELLED, (uint32_t)LFX_STATUS_OUT_OF_MEMORY,
    (uint32_t)LFX_STATUS_UNSUPPORTED
};

/* Every string the ABI spells, in the order `tests/layout.rs` compares them:
 * the entry symbol, then the five extension ids. A misspelling here is a
 * plugin asking for an extension by a name the host has never heard of. */
static const char *const lfx_abi_string_table[] = {
    LFX_ENTRY_SYMBOL, LFX_EXT_TEMPORAL, LFX_EXT_GPU_FRAMES, LFX_EXT_OVERLAY,
    LFX_EXT_MOTION_VECTORS, LFX_EXT_AUDIO
};

uint32_t lfx_abi_constant_count(void);
const uint32_t *lfx_abi_constants(void);
uint32_t lfx_abi_string_count(void);
const char *lfx_abi_string(uint32_t index);

uint32_t lfx_abi_constant_count(void) {
    return (uint32_t)(sizeof lfx_abi_constant_table / sizeof lfx_abi_constant_table[0]);
}

const uint32_t *lfx_abi_constants(void) {
    return lfx_abi_constant_table;
}

uint32_t lfx_abi_string_count(void) {
    return (uint32_t)(sizeof lfx_abi_string_table / sizeof lfx_abi_string_table[0]);
}

const char *lfx_abi_string(uint32_t index) {
    if (index >= lfx_abi_string_count()) {
        return NULL;
    }
    return lfx_abi_string_table[index];
}

/* ----------------------------------------------------- the header's types -- */

/* A field's *type* is not pinned by its offset either, wherever two types share
 * a width: `float roi_padding_px` against a mirror's `u32` is four bytes at
 * twelve on both sides, and `int32_t temporal_lo` against a `u32` is worse,
 * because every small positive number reads alike.
 *
 * So the C half writes each of these fields from the header's own declaration
 * and the Rust half reads them back through the mirror. A float written as 0.5
 * reads as 1056964608 through a `u32`; a -3 reads as 4294967293; a double
 * written as 0.25 read as an `int64_t` is 4598175219545276416. A type that
 * drifted fails rather than passes.
 *
 * Some of the counts below are written past the top of an `int32_t`, which is
 * the only thing that tells a four-byte unsigned count from a signed one. They
 * are bit patterns chosen to distinguish a type, not declarations a plugin
 * could make: the ceilings are asked where a stranger's bytes actually arrive,
 * and nothing here is a declaration. */
void lfx_abi_write_traits(lfx_traits *out);
void lfx_abi_write_float_param(lfx_float_param *out);
void lfx_abi_write_slider_param(lfx_slider_param *out);
void lfx_abi_write_int_param(lfx_int_param *out);
void lfx_abi_write_angle_param(lfx_angle_param *out);
void lfx_abi_write_bool_param(lfx_bool_param *out);
void lfx_abi_write_choice_param(lfx_choice_param *out);
void lfx_abi_write_colour_param(lfx_colour_param *out);
void lfx_abi_write_seed_param(lfx_seed_param *out);
void lfx_abi_write_point2_param(lfx_point2_param *out);
void lfx_abi_write_point3_param(lfx_point3_param *out);
void lfx_abi_write_curve_param(lfx_curve_param *out);
void lfx_abi_write_file_param(lfx_file_param *out);
void lfx_abi_write_action_param(lfx_action_param *out);
void lfx_abi_write_group_param(lfx_group_param *out);
void lfx_abi_write_descriptor(lfx_descriptor *out);
void lfx_abi_write_frame(lfx_frame *out);
void lfx_abi_write_process(lfx_process *out);
void lfx_abi_write_value(lfx_value *out, lfx_param_kind kind);

/* What the pointer-valued fields of those records point at. Each is a file
 * static of the header's own element type, so a mirror that read
 * `const uint32_t *dividers_after` as a pointer to something else, or
 * `const float *points` as a pointer to a double, fails on the values rather
 * than on the pointer's width. */
static const char lfx_abi_text_id[] = "layout_id";
static const char lfx_abi_text_label[] = "Layout label";
static const char *const lfx_abi_options[3] = {"First", "Second", "Third"};
static const uint32_t lfx_abi_dividers[2] = {0u, 1u};
static const float lfx_abi_curve_points[4] = {0.0f, 0.25f, 1.0f, 0.75f};
static const char *const lfx_abi_filters[2] = {"cube", "exr"};
static const char lfx_abi_filter_name[] = "Lookup tables";
static const lfx_category lfx_abi_categories[2] = {LFX_CATEGORY_COLOUR, LFX_CATEGORY_UTILITY};
static const char *const lfx_abi_required[1] = {LFX_EXT_TEMPORAL};
static const char lfx_abi_file_path[] = "/layout/probe.cube";

void lfx_abi_write_traits(lfx_traits *out) {
    out->struct_size = (uint32_t)sizeof(lfx_traits);
    out->cost = LFX_COST_MODERATE;
    out->roi_kind = LFX_ROI_PADDED;
    out->roi_padding_px = 0.5f;
    out->temporal_lo = -3;
    out->temporal_hi = 4;
    out->alpha = LFX_ALPHA_STRAIGHT;
    out->flags = LFX_TRAIT_SEEDED | LFX_TRAIT_CANCELLABLE;
    out->scratch_bytes_per_megapixel = 12345u;
}

void lfx_abi_write_float_param(lfx_float_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_float_param);
    out->unit = LFX_UNIT_PX;
    out->flags = LFX_PARAM_FLAG_STATIC;
    out->bounds = LFX_BOUND_MIN | LFX_BOUND_MAX;
    out->id = NULL;
    out->label = NULL;
    out->default_value = 0.25;
    out->slider_min = -1.5;
    out->slider_max = 2.5;
    out->hard_min = -8.0;
    out->hard_max = 8.0;
}

void lfx_abi_write_slider_param(lfx_slider_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_slider_param);
    out->unit = LFX_UNIT_SECONDS;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->log = 1u;
    out->id = NULL;
    out->label = NULL;
    out->default_value = 0.75;
    out->range_min = 0.125;
    out->range_max = 4.0;
}

void lfx_abi_write_int_param(lfx_int_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_int_param);
    out->unit = LFX_UNIT_FRAMES;
    out->flags = LFX_PARAM_FLAG_HIDDEN;
    out->bounds = LFX_BOUND_MAX;
    out->id = NULL;
    out->label = NULL;
    /* Wider than an int32 either way, so a mirror that narrowed them fails on
     * the value rather than only on the size. */
    out->default_value = -5000000000LL;
    out->slider_min = -9000000000LL;
    out->slider_max = 9000000000LL;
    out->hard_min = -1LL;
    out->hard_max = 7000000000LL;
}

void lfx_abi_write_angle_param(lfx_angle_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_angle_param);
    out->unit = LFX_UNIT_DEGREES;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->reserved_0 = 0u;
    out->id = NULL;
    out->label = NULL;
    out->default_value = 45.0;
    out->dial_step = 7.5;
}

void lfx_abi_write_colour_param(lfx_colour_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_colour_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->reserved_0 = 0u;
    out->id = NULL;
    out->label = NULL;
    out->default_rgba[0] = 0.125;
    out->default_rgba[1] = 0.25;
    out->default_rgba[2] = 0.5;
    out->default_rgba[3] = 1.0;
    out->range_min = -0.5;
    out->range_max = 1.5;
}

void lfx_abi_write_point2_param(lfx_point2_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_point2_param);
    out->unit = LFX_UNIT_PX;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->reserved_0 = 0u;
    out->id = NULL;
    out->label = NULL;
    out->default_x = 1.25;
    out->default_y = -2.75;
    out->slider_min = -10.0;
    out->slider_max = 10.0;
}

void lfx_abi_write_point3_param(lfx_point3_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_point3_param);
    out->unit = LFX_UNIT_PX;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->reserved_0 = 0u;
    out->id = NULL;
    out->label = NULL;
    out->default_x = 1.25;
    out->default_y = -2.75;
    out->default_z = 3.5;
    out->slider_min = -10.0;
    out->slider_max = 10.0;
}

void lfx_abi_write_bool_param(lfx_bool_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_bool_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_NONE;
    /* One, which is 1.401e-45 read as a float and nothing at all read as a
     * pointer: a switch declared on by default must not read as off. */
    out->default_value = 1u;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
}

void lfx_abi_write_choice_param(lfx_choice_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_choice_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->default_index = 2u;
    /* Past the top of an `int32_t`, so a count that drifted to a signed type
     * fails on the value rather than reading alike. */
    out->option_count = 0x80000001u;
    out->divider_count = 2u;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
    out->options = lfx_abi_options;
    out->dividers_after = lfx_abi_dividers;
}

void lfx_abi_write_seed_param(lfx_seed_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_seed_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_HIDDEN;
    out->reserved_0 = 0u;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
}

void lfx_abi_write_curve_param(lfx_curve_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_curve_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_STATIC;
    out->point_count = 0x80000001u;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
    out->points = lfx_abi_curve_points;
}

void lfx_abi_write_file_param(lfx_file_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_file_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_STATIC;
    out->filter_count = 0x80000001u;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
    out->filter = lfx_abi_filters;
    out->filter_name = lfx_abi_filter_name;
}

void lfx_abi_write_action_param(lfx_action_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_action_param);
    out->unit = LFX_UNIT_RAW;
    out->flags = LFX_PARAM_FLAG_NONE;
    out->reserved_0 = 0u;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
}

/* The one declaration record with no `unit`: a heading is a run rather than a
 * row, so `flags` sits where every other record's unit does. */
void lfx_abi_write_group_param(lfx_group_param *out) {
    out->struct_size = (uint32_t)sizeof(lfx_group_param);
    out->flags = LFX_PARAM_FLAG_HIDDEN;
    out->id = lfx_abi_text_id;
    out->label = lfx_abi_text_label;
}

void lfx_abi_write_descriptor(lfx_descriptor *out) {
    out->struct_size = (uint32_t)sizeof(lfx_descriptor);
    out->id = lfx_abi_text_id;
    out->name = lfx_abi_text_label;
    out->vendor = lfx_abi_text_label;
    /* Past the top of an `int32_t`. The major version is the number the host's
     * frame key is stored in, so a signed mirror would key a cache on a
     * negative number for any release past two billion. */
    out->major = 0x80000001u;
    out->minor = 999u;
    out->patch = 1u;
    out->categories = lfx_abi_categories;
    out->category_count = 2u;
    out->traits = NULL;
    out->required_extensions = lfx_abi_required;
    out->required_extension_count = 1u;
}

void lfx_abi_write_frame(lfx_frame *out) {
    out->struct_size = (uint32_t)sizeof(lfx_frame);
    out->format = LFX_RGBA_F32;
    out->width = 640u;
    out->height = 360u;
    out->row_bytes = 640u * 16u;
    out->origin_x = -7;
    out->origin_y = -9;
    out->reserved_0 = 0u;
    out->data = NULL;
    out->time = 12.5;
}

void lfx_abi_write_process(lfx_process *out) {
    out->struct_size = (uint32_t)sizeof(lfx_process);
    out->pixel_format = LFX_RGBA_F16;
    out->value_stride = (uint32_t)sizeof(lfx_value);
    out->value_count = 2u;
    out->roi_x0 = -1;
    out->roi_y0 = -2;
    out->roi_x1 = 3;
    out->roi_y1 = 4;
    out->dod_x0 = -5;
    out->dod_y0 = -6;
    out->dod_x1 = 7;
    out->dod_y1 = 8;
    out->time = 24.5;
    out->values = NULL;
    out->input = NULL;
    out->output = NULL;
    out->cancelled = NULL;
    out->host_context = NULL;
}

void lfx_abi_write_value(lfx_value *out, lfx_param_kind kind) {
    out->param = 2u;
    out->kind = kind;
    switch (kind) {
        /* Three tags, one arm, exactly as the header says beside it. */
        case LFX_PARAM_FLOAT:
        case LFX_PARAM_SLIDER:
        case LFX_PARAM_ANGLE:
            out->v.f = 0.5;
            break;
        case LFX_PARAM_INT:
        case LFX_PARAM_SEED:
            out->v.i = -5000000000LL;
            break;
        case LFX_PARAM_BOOL:
            out->v.b = true;
            break;
        case LFX_PARAM_CHOICE:
            out->v.choice = 9u;
            break;
        case LFX_PARAM_COLOUR:
            out->v.rgba[0] = 0.25f;
            out->v.rgba[1] = 0.5f;
            out->v.rgba[2] = 0.75f;
            out->v.rgba[3] = 1.0f;
            break;
        case LFX_PARAM_POINT2:
            /* Both axes, in one element: the host's two rows folded back. */
            out->v.xy[0] = 1.5f;
            out->v.xy[1] = -2.5f;
            break;
        case LFX_PARAM_POINT3:
            out->v.xyz[0] = 1.5f;
            out->v.xyz[1] = -2.5f;
            out->v.xyz[2] = 3.5f;
            break;
        case LFX_PARAM_CURVE:
            /* The points first and the count second. Read the other way round
             * a plugin dereferences its own count as an address. */
            out->v.curve.pt = lfx_abi_curve_points;
            out->v.curve.n = 2u;
            break;
        case LFX_PARAM_FILE:
            out->v.file.path = lfx_abi_file_path;
            break;
        default:
            out->kind = LFX_PARAM_UNSET;
            out->v.i = 0;
            break;
    }
}

/* ---------------------------------------------------- the header's tables -- */

/* Four of the ABI's structs are not data at all: `lfx_describe_sink`,
 * `lfx_entry`, `lfx_plugin` and `lfx_host` are tables of function pointers, and
 * every assertion above pins only where each pointer sits. Swapping
 * `lfx_entry.create`'s two arguments in the mirror moves nothing; neither does
 * making `declare_float` take an `lfx_slider_param`, or answer a `uint32_t`.
 * Either one hands a stranger's code an argument of a type it did not expect,
 * at the ABI's front door.
 *
 * So one real instance of each table is defined here from the header's own
 * declarations, and `tests/layout.rs` calls through the *mirror's* own
 * `Option<unsafe extern "C" fn ...>` types. The call itself is the assertion:
 * arity, argument order, argument type and answer all have to line up for the
 * recorded values to come back.
 *
 * Every callback records rather than asserts, because a failure reads far
 * better on the Rust side, where it can name the field. The slots are shared
 * and `lfx_abi_layout_forget` empties them, so each call is read on its own. */

static struct {
    uint32_t calls;
    uint32_t struct_size;
    uint32_t sink_size;
    uint32_t unit;
    uint32_t whole_u32;
    double number;
    int64_t whole;
    const char *text;
} lfx_layout_record;

void lfx_abi_layout_forget(void);
uint32_t lfx_abi_recorded_calls(void);
uint32_t lfx_abi_recorded_struct_size(void);
uint32_t lfx_abi_recorded_sink_size(void);
uint32_t lfx_abi_recorded_unit(void);
uint32_t lfx_abi_recorded_whole_u32(void);
double lfx_abi_recorded_number(void);
int64_t lfx_abi_recorded_whole(void);
const char *lfx_abi_recorded_text(void);

void lfx_abi_layout_forget(void) {
    lfx_layout_record.calls = 0u;
    lfx_layout_record.struct_size = 0u;
    lfx_layout_record.sink_size = 0u;
    lfx_layout_record.unit = 0u;
    lfx_layout_record.whole_u32 = 0u;
    lfx_layout_record.number = 0.0;
    lfx_layout_record.whole = 0;
    lfx_layout_record.text = NULL;
}

uint32_t lfx_abi_recorded_calls(void) { return lfx_layout_record.calls; }
uint32_t lfx_abi_recorded_struct_size(void) { return lfx_layout_record.struct_size; }
uint32_t lfx_abi_recorded_sink_size(void) { return lfx_layout_record.sink_size; }
uint32_t lfx_abi_recorded_unit(void) { return lfx_layout_record.unit; }
uint32_t lfx_abi_recorded_whole_u32(void) { return lfx_layout_record.whole_u32; }
double lfx_abi_recorded_number(void) { return lfx_layout_record.number; }
int64_t lfx_abi_recorded_whole(void) { return lfx_layout_record.whole; }
const char *lfx_abi_recorded_text(void) { return lfx_layout_record.text; }

/* The twelve declaration calls that share a shape. Each records the size prefix
 * of the record that arrived - which is the deterministic half of the pin, since
 * a caller passing the wrong kind of record writes a size this one does not
 * recognise - together with the sink it was handed back, the unit, and one field
 * of a type this kind alone carries. */
#define LFX_SINK_RECORDER(name, type, number_expr, whole_expr)      \
    static bool name(struct lfx_describe_sink *sink, const type *p) { \
        lfx_layout_record.calls += 1u;                              \
        lfx_layout_record.sink_size = sink->struct_size;            \
        lfx_layout_record.struct_size = p->struct_size;             \
        lfx_layout_record.unit = p->unit;                           \
        lfx_layout_record.text = p->id;                             \
        lfx_layout_record.number = (double)(number_expr);           \
        lfx_layout_record.whole = (int64_t)(whole_expr);            \
        return true;                                               \
    }

LFX_SINK_RECORDER(lfx_layout_declare_float, lfx_float_param, p->default_value, p->bounds)
LFX_SINK_RECORDER(lfx_layout_declare_slider, lfx_slider_param, p->range_max, p->log)
LFX_SINK_RECORDER(lfx_layout_declare_int, lfx_int_param, 0.0, p->hard_max)
LFX_SINK_RECORDER(lfx_layout_declare_angle, lfx_angle_param, p->dial_step, 0)
LFX_SINK_RECORDER(lfx_layout_declare_bool, lfx_bool_param, 0.0, p->default_value)
LFX_SINK_RECORDER(lfx_layout_declare_choice, lfx_choice_param, 0.0, p->option_count)
LFX_SINK_RECORDER(lfx_layout_declare_colour, lfx_colour_param, p->default_rgba[2], 0)
LFX_SINK_RECORDER(lfx_layout_declare_seed, lfx_seed_param, 0.0, p->flags)
LFX_SINK_RECORDER(lfx_layout_declare_point2, lfx_point2_param, p->default_y, 0)
LFX_SINK_RECORDER(lfx_layout_declare_point3, lfx_point3_param, p->default_z, 0)
LFX_SINK_RECORDER(lfx_layout_declare_curve, lfx_curve_param, p->points[1], p->point_count)
LFX_SINK_RECORDER(lfx_layout_declare_file, lfx_file_param, 0.0, p->filter_count)
LFX_SINK_RECORDER(lfx_layout_declare_action, lfx_action_param, 0.0, p->flags)

/* The two that do not: a heading carries no unit, and closing one carries
 * nothing at all. `group_end` answers `false` so the Rust half reads an answer
 * of each kind rather than only of one. */
static bool lfx_layout_group_begin(struct lfx_describe_sink *sink, const lfx_group_param *p) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.sink_size = sink->struct_size;
    lfx_layout_record.struct_size = p->struct_size;
    lfx_layout_record.text = p->id;
    lfx_layout_record.whole = (int64_t)p->flags;
    return true;
}

static bool lfx_layout_group_end(struct lfx_describe_sink *sink) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.sink_size = sink->struct_size;
    return false;
}

static lfx_describe_sink lfx_layout_sink = {
    (uint32_t)sizeof(lfx_describe_sink),
    NULL,
    lfx_layout_declare_float,
    lfx_layout_declare_slider,
    lfx_layout_declare_int,
    lfx_layout_declare_angle,
    lfx_layout_declare_bool,
    lfx_layout_declare_choice,
    lfx_layout_declare_colour,
    lfx_layout_declare_seed,
    lfx_layout_declare_point2,
    lfx_layout_declare_point3,
    lfx_layout_declare_curve,
    lfx_layout_declare_file,
    lfx_layout_declare_action,
    lfx_layout_group_begin,
    lfx_layout_group_end
};

/* What `get_extension` answers when it is asked for something this table has:
 * a real address, so a mirror that read the answer as anything but a pointer
 * fails on the value rather than on the width. */
static const char lfx_layout_extension[] = "lfx.temporal@3";

static uint32_t lfx_layout_plugin_init(struct lfx_plugin *plugin) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = plugin->struct_size;
    /* Non-zero is true, and five is a non-zero no C `bool` would produce: the
     * mirror has to read a `u32` here for the Rust half to compare it. */
    return 5u;
}

static void lfx_layout_plugin_destroy(struct lfx_plugin *plugin) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = plugin->struct_size;
}

static uint32_t lfx_layout_plugin_describe(struct lfx_plugin *plugin, lfx_describe_sink *sink) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = plugin->struct_size;
    lfx_layout_record.sink_size = sink->struct_size;
    return 9u;
}

static int32_t lfx_layout_plugin_process(struct lfx_plugin *plugin, const lfx_process *request) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = plugin->struct_size;
    lfx_layout_record.whole_u32 = request->value_count;
    lfx_layout_record.number = request->time;
    /* Not an `lfx_status`, and deliberately: it is here to pin that `process`
     * answers a *signed* 32-bit number, which a mirror reading a `u32` would
     * turn into four billion and something. */
    return -12345;
}

static const void *lfx_layout_plugin_get_extension(struct lfx_plugin *plugin, const char *id,
                                                   uint32_t version) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = plugin->struct_size;
    lfx_layout_record.text = id;
    lfx_layout_record.whole_u32 = version;
    return (version == 3u) ? (const void *)lfx_layout_extension : NULL;
}

static lfx_plugin lfx_layout_plugin = {
    (uint32_t)sizeof(lfx_plugin),
    NULL,
    lfx_layout_plugin_init,
    lfx_layout_plugin_destroy,
    lfx_layout_plugin_describe,
    lfx_layout_plugin_process,
    lfx_layout_plugin_get_extension
};

static const void *lfx_layout_host_get_extension(const struct lfx_host *host, const char *id,
                                                 uint32_t version) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = host->struct_size;
    lfx_layout_record.text = id;
    lfx_layout_record.whole_u32 = version;
    return (version == 3u) ? (const void *)lfx_layout_extension : NULL;
}

static void lfx_layout_host_log(const struct lfx_host *host, lfx_log_level level,
                                const char *message) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.struct_size = host->struct_size;
    lfx_layout_record.whole_u32 = level;
    lfx_layout_record.text = message;
}

static lfx_host lfx_layout_host = {
    (uint32_t)sizeof(lfx_host),
    LFX_ABI_VERSION,
    NULL,
    lfx_layout_host_get_extension,
    lfx_layout_host_log
};

/* Two effects, so `descriptor` has an index that means something and an index
 * past the end that answers NULL. */
static const lfx_descriptor lfx_layout_descriptors[2] = {
    {(uint32_t)sizeof(lfx_descriptor), "layout.first", "First", "Layout", 11u, 0u, 0u,
     lfx_abi_categories, 2u, NULL, lfx_abi_required, 1u},
    {(uint32_t)sizeof(lfx_descriptor), "layout.second", "Second", "Layout", 22u, 0u, 0u,
     lfx_abi_categories, 1u, NULL, NULL, 0u}
};

static uint32_t lfx_layout_entry_init(const char *bundle_path) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.text = bundle_path;
    return 7u;
}

static void lfx_layout_entry_deinit(void) {
    lfx_layout_record.calls += 1u;
}

static uint32_t lfx_layout_entry_count(void) {
    lfx_layout_record.calls += 1u;
    return 2u;
}

static const lfx_descriptor *lfx_layout_entry_descriptor(uint32_t index) {
    lfx_layout_record.calls += 1u;
    lfx_layout_record.whole_u32 = index;
    if (index >= 2u) {
        return NULL;
    }
    return &lfx_layout_descriptors[index];
}

static lfx_plugin *lfx_layout_entry_create(const lfx_host *host, const char *id) {
    lfx_layout_record.calls += 1u;
    /* The host first and the id second. Reading them the other way round takes
     * the first four bytes of the id string for the host's own size prefix,
     * which is what the Rust half compares. */
    lfx_layout_record.struct_size = host->struct_size;
    lfx_layout_record.text = id;
    return &lfx_layout_plugin;
}

static const lfx_entry lfx_layout_entry = {
    (uint32_t)sizeof(lfx_entry),
    LFX_ABI_VERSION,
    lfx_layout_entry_init,
    lfx_layout_entry_deinit,
    lfx_layout_entry_count,
    lfx_layout_entry_descriptor,
    lfx_layout_entry_create
};

const lfx_describe_sink *lfx_abi_layout_sink(void);
const lfx_entry *lfx_abi_layout_entry(void);
lfx_plugin *lfx_abi_layout_plugin(void);
const lfx_host *lfx_abi_layout_host(void);
const char *lfx_abi_layout_extension(void);

const lfx_describe_sink *lfx_abi_layout_sink(void) { return &lfx_layout_sink; }
const lfx_entry *lfx_abi_layout_entry(void) { return &lfx_layout_entry; }
lfx_plugin *lfx_abi_layout_plugin(void) { return &lfx_layout_plugin; }
const lfx_host *lfx_abi_layout_host(void) { return &lfx_layout_host; }
const char *lfx_abi_layout_extension(void) { return lfx_layout_extension; }
