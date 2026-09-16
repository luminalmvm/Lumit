/*
 * lfx.h - LFX, Lumit's native effect ABI, version 1.
 *
 * SPDX-License-Identifier: MIT
 * Copyright (c) 2026 The Lumit authors.
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in
 * all copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 *
 * The licence is deliberately more permissive than Lumit's own GPLv3
 * (docs/12-PLUGINS.md §3.6), so a proprietary vendor may adopt the header
 * without licence anxiety.
 *
 * ---------------------------------------------------------------------------
 *
 * In plain terms
 *
 * This file is the whole agreement between Lumit and an effect somebody else
 * wrote. A plugin exports one symbol, `lfx_entry_point`; through it Lumit asks
 * what effects the bundle holds, creates one, asks it to declare its controls,
 * and then hands it pictures to process. There is no property bag, no string
 * key, and no question the host can ask that a plugin can answer in the wrong
 * type: every control is declared by pushing a typed record, and every value
 * arrives back carrying the kind it was declared with.
 *
 * The layout is frozen. `LFX_ABI_VERSION` is the integer that intends to reach
 * 2 never: every struct opens with `uint32_t struct_size`, growth happens by
 * adding fields at the end, and fields are never re-ordered and never removed.
 * Version 1's fields are therefore always the first bytes of any later
 * version's struct, and `struct_size` is read in the direction growth happens:
 * a side handed a *longer* struct than it was built against reads the fields it
 * knows and ignores the tail.
 *
 * The other direction is not the same question, and version 1 answers it per
 * struct here rather than leaving it to whoever reads the bytes first. A struct
 * *shorter* than the reader's own is one missing fields that reader's version
 * requires, and they are never invented:
 *
 *   - `lfx_entry` and `lfx_plugin` are refused. They are tables of function
 *     pointers, and a pointer that is not there cannot be called.
 *   - `lfx_traits` reads as the pessimistic case, exactly as a `NULL` block
 *     does.
 *   - `lfx_descriptor` and the declaration records are declined, with a line in
 *     the host's scan report naming the size that arrived.
 *
 * The cost of that is stated rather than hidden: a field appended to `lfx_entry`
 * or `lfx_plugin` would leave every bundle built against this header short, so
 * those two tables do not grow. They gain capability through `get_extension`
 * instead, which is what extensions are for.
 *
 * `lfx_value` is the one named exemption, and `lfx_process.value_stride` is
 * why. Values cross as a dense array addressed by index, so what the two sides
 * must agree on is the element stride rather than where a field starts. A
 * plugin indexes the array by `value_stride`, never by `sizeof(lfx_value)`;
 * striding by its own `sizeof` after the struct grows would read correct-
 * looking kind tags over silently wrong values. The host guarantees that the
 * array's base and the stride are both aligned for `lfx_value`, so every
 * element is too and the natural spelling is an ordinary aligned read.
 *
 * Every enumeration crosses as a `uint32_t` rather than as a C `enum`, whose
 * width is the compiler's to choose. The constants are declared in anonymous
 * enums; the fields are declared in the fixed-width typedef.
 *
 * For a neighbouring reason, no answer the *plugin* gives crosses as a C
 * `bool`. A `bool` has only two valid representations, and the host cannot
 * inspect the byte a stranger's compiler left in the return register before it
 * reads it - so `lfx_entry.init`, `lfx_plugin.init` and `lfx_plugin.describe`
 * answer a `uint32_t` in which any non-zero value is true. It is the choice
 * `lfx_bool_param.default_value` already makes for a declared switch. The
 * `bool`s that remain are the host's own, read by the plugin rather than
 * written by it: `lfx_describe_sink`'s answers and `lfx_process.cancelled`.
 *
 * Every trait enumeration starts at `UNSET = 0` and the host lowers an unstated
 * trait to the *pessimistic* answer, never to the first real one: a zeroed or
 * short trait block, or none at all, must schedule as the most expensive thing
 * it could be (docs/impl/lfx.md §2.4).
 *
 * Threading, stated per callback below and summarised here: `process` may be
 * called from any worker thread, and on different instances of one plugin
 * concurrently; one instance is never re-entered; `describe` and the instance
 * lifecycle run on one host-designated control thread. A plugin that cannot
 * bear that declares `LFX_TRAIT_THREAD_UNSAFE` and is serialised bundle-wide.
 *
 * The canonical copy of this file lives in `crates/lumit-lfx-abi/include` and
 * is pinned from both sides: `tests/layout.rs` asserts the Rust mirror's sizes
 * and offsets by number and `tests/layout.c` asserts this header's with
 * `sizeof`/`offsetof`, and because an offset says nothing about what a value
 * means, the C half also emits every constant and every string here for the
 * Rust half to compare, and writes each struct of plain data from these very
 * declarations for it to read back. The four structs that are tables of
 * function pointers are *called* instead, through the mirror's own types, an
 * offset saying nothing about arity or argument order either. A moved field, a
 * renumbered enumerator, a misspelt extension id, a field whose type drifted
 * and a callback whose signature drifted all fail.
 */

#ifndef LFX_H
#define LFX_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The core ABI version. One integer, and it intends to reach 2 never:
 * everything that would have been a version bump is an extension instead. */
#define LFX_ABI_VERSION 1u

/* The one symbol a bundle exports, as a string for the host's dynamic loader.
 * The object itself is declared at the bottom of this file. */
#define LFX_ENTRY_SYMBOL "lfx_entry_point"

/* -------------------------------------------------------------- ceilings -- */

/* Every count and every string a plugin hands the host is bounded, and the
 * bounds are declared here rather than invented by whoever read the bytes
 * first. A frozen ABI cannot raise a limit that turned out too small - a
 * shipped vendor is already inside it - and cannot narrow one that turned out
 * too wide, so the numbers belong in the file both sides compile against.
 *
 * A declaration past any of these is refused by name, with the ceiling in the
 * host's scan report. The host enforces the same numbers where a stranger's
 * bytes arrive - `lumit-ingress`'s limits for the bundle manifest, and the
 * broker's own reads for everything the module answers once it is open - so
 * the two sides agree by construction rather than by coincidence, and
 * `lfx-validator` has a case for each of them.
 *
 * **Every `const char *` in this file is UTF-8 and NUL-terminated within
 * `LFX_MAX_STRING_BYTES`**, the NUL counted. The one exception is
 * `lfx_host.log`'s message, which is diagnostics rather than identity and is
 * allowed `LFX_MAX_LOG_BYTES`. A string with no NUL inside its ceiling is a
 * refusal rather than a longer read: the host never walks past the limit
 * looking for the end. */
#define LFX_MAX_STRING_BYTES 1024u
#define LFX_MAX_LOG_BYTES 4096u
/* One per member of `lfx_category`, which is the most an effect could claim. */
#define LFX_MAX_CATEGORIES 8u
#define LFX_MAX_OPTIONS 256u
/* A dropdown draws at most one rule per option, so the option ceiling is the
 * only divider ceiling that cannot turn out too small. A smaller number would
 * admit a dropdown and refuse the divider list that groups it, and a ceiling
 * here declines the whole declaration rather than trimming it: the plugin would
 * lose the control, not the rules. */
#define LFX_MAX_DIVIDERS LFX_MAX_OPTIONS
#define LFX_MAX_FILTERS 32u
#define LFX_MAX_REQUIRED_EXTENSIONS 16u
#define LFX_MAX_EFFECTS_PER_BUNDLE 1024u
/* The most declarations one effect may push into the describe sink - controls
 * and headings together, and counted whether the host accepts them or not.
 * Describe runs on the control thread and every row it mints lives as long as
 * the session, so the count is bounded here rather than left to whatever a
 * plugin's loop happens to produce. Counted over pushes rather than over rows
 * accepted because a declaration the host declines still costs it the sentence
 * saying so, and a loop of those would grow without bound while the panel
 * stayed empty. Exceeding it refuses the **effect** rather than costing one
 * row: a panel the host cannot draw is not a control lost. */
#define LFX_MAX_PARAMS 512u
/* A tone curve's control points: two, because one point is not a curve, up to
 * sixteen. */
#define LFX_MIN_CURVE_POINTS 2u
#define LFX_MAX_CURVE_POINTS 16u
/* How far either end of `lfx_traits`'s temporal window may reach, in comp
 * frames. Signed, because the window is: `[-LFX_MAX_TEMPORAL_WINDOW,
 * LFX_MAX_TEMPORAL_WINDOW]` is the whole of what a plugin may declare. */
#define LFX_MAX_TEMPORAL_WINDOW 64

/* ------------------------------------------------------------------ kinds -- */

/* What a declared control is. The vocabulary is Lumit's own: each kind lowers
 * onto a `ParamKind` the host already has, rather than onto a foreign standard
 * the host would have to map (docs/impl/lfx.md §2.3).
 *
 * `LFX_PARAM_PATH` and `LFX_PARAM_STRING` are in the frozen enum from day one
 * and are **refused by name** in version 1 - a bezier path with no on-Viewer
 * handles is a control nobody can edit, and the resolved value bag carries no
 * text at all, so a string row's value would never reach `process`. Admitting
 * them later adds no discriminant and breaks no compiled plugin.
 *
 * *ponytail:* `LFX_PARAM_FILE` is admitted and its payload is not settled. The
 * path a File row resolves to rides beside the op as an auxiliary slot, and the
 * generic file aux that would carry an arbitrary plugin's file does not exist
 * yet - a LUT and a lens file are the only two the host loads today. Until it
 * lands, `lfx_value.v.file.path` is NULL, and a plugin that cannot work without
 * the path is better off not declaring the row. */
typedef uint32_t lfx_param_kind;
enum {
    LFX_PARAM_UNSET = 0u,
    LFX_PARAM_FLOAT = 1u,  /* unbounded, with optional hard bounds           */
    LFX_PARAM_SLIDER = 2u, /* a closed range, optionally logarithmic         */
    LFX_PARAM_INT = 3u,
    LFX_PARAM_BOOL = 4u,
    LFX_PARAM_CHOICE = 5u,
    LFX_PARAM_COLOUR = 6u, /* RGBA, scene-linear                             */
    LFX_PARAM_ANGLE = 7u,  /* degrees, drawn as a dial                       */
    LFX_PARAM_SEED = 8u,   /* the randomness a seeded effect follows         */
    LFX_PARAM_POINT2 = 9u, /* two rows, `<id>_x` and `<id>_y`                */
    LFX_PARAM_POINT3 = 10u,
    LFX_PARAM_CURVE = 11u, /* the *tone* curve: points in the unit square    */
    LFX_PARAM_FILE = 12u,  /* a file from a dialog; the ponytail above  */
    LFX_PARAM_ACTION = 13u, /* a button: no value, no keyframe               */
    LFX_PARAM_GROUP = 14u,  /* a run of rows, no row of its own              */
    LFX_PARAM_PATH = 15u,   /* reserved; refused in version 1                */
    LFX_PARAM_STRING = 16u  /* reserved; refused in version 1                */
};

/* What a number *means*. Mandatory on every declaration: `LFX_UNIT_UNSET` is a
 * describe refusal, mirroring the build failure every one of Lumit's own
 * effects faces, because "dimensionless" and "nobody decided" must not look
 * alike. */
typedef uint32_t lfx_unit;
enum {
    LFX_UNIT_UNSET = 0u,
    LFX_UNIT_RAW = 1u,     /* a plain number: a gamma, a count, a threshold  */
    LFX_UNIT_PERCENT = 2u, /* 100 is the whole of whatever it is a share of  */
    /* A per cent of the composition diagonal. This ladder mirrors the host's
     * own `Unit` value for value so the two cannot drift, and this is the
     * member that carries no consumer of its own in version 1: it is a describe
     * **refusal** on a parameter, because every distance a control carries is
     * px@comp, and the ROI padding declaration does not take it either -
     * `lfx_traits.roi_padding_px` is px@comp and has no unit field at all. */
    LFX_UNIT_PCT_DIAG = 3u,
    /* Pixels at composition size - never pixels of whatever buffer the plugin
     * was handed. The host converts to the raster in play. */
    LFX_UNIT_PX = 4u,
    LFX_UNIT_DEGREES = 5u,
    LFX_UNIT_SECONDS = 6u, /* seconds of layer time                          */
    LFX_UNIT_FRAMES = 7u   /* comp-rate frames                               */
};

/* The picture families an LFX effect may claim, closed and frozen. The first
 * declared category is the heading the effect is browsed under; the rest are
 * search keywords.
 *
 * Audio, Drivers, Controls and Compositing are deliberately absent: each is a
 * family whose members answer a question a one-input picture effect does not
 * ask, and a plugin that claimed one would install, register, and then sit in
 * no menu a layer can reach. An unrecognised value lands in
 * `LFX_CATEGORY_UTILITY` plus a line in the scan report. */
typedef uint32_t lfx_category;
enum {
    LFX_CATEGORY_UNSET = 0u,
    LFX_CATEGORY_BLUR_SHARPEN = 1u,
    LFX_CATEGORY_COLOUR = 2u,
    LFX_CATEGORY_DISTORTION = 3u,
    LFX_CATEGORY_GENERATE = 4u,
    LFX_CATEGORY_STYLISE = 5u,
    LFX_CATEGORY_TEMPORAL = 6u,
    LFX_CATEGORY_TRANSITION = 7u,
    LFX_CATEGORY_UTILITY = 8u
};

/* The two working depths, **both mandatory**. The host sends whichever depth
 * the project is set to and never converts to accommodate a plugin; an 8 bpc
 * project sends fp16, which round-trips every 8-bit code value, so a plugin
 * never sees an integer buffer. */
typedef uint32_t lfx_pixel_format;
enum {
    LFX_PIXEL_UNSET = 0u,
    LFX_RGBA_F16 = 1u,
    LFX_RGBA_F32 = 2u
};

/* What one frame of this effect costs, read by the scheduler and the
 * degradation ladder. UNSET lowers to `LFX_COST_HEAVY`. */
typedef uint32_t lfx_cost;
enum {
    LFX_COST_UNSET = 0u,
    LFX_COST_TRIVIAL = 1u,
    LFX_COST_CHEAP = 2u,
    LFX_COST_MODERATE = 3u,
    LFX_COST_HEAVY = 4u
};

/* How far past an output pixel the effect reads. UNSET lowers to
 * `LFX_ROI_FULL_FRAME`: claiming less reach than the kernel uses produces tile
 * seams, which is a correctness bug, so the unstated answer is the expensive
 * one. `LFX_ROI_PADDED` with no padding is exact, and a line in the report. */
typedef uint32_t lfx_roi_kind;
enum {
    LFX_ROI_UNSET = 0u,
    LFX_ROI_EXACT = 1u,
    LFX_ROI_PADDED = 2u, /* with `lfx_traits.roi_padding_px`, in px@comp     */
    LFX_ROI_FULL_FRAME = 3u
};

/* Which alpha the effect's maths expects. Two states, not three: the host's own
 * declaration is a boolean, and a third state that read as scheduling
 * information would be a declaration the host has nowhere to put. UNSET lowers
 * to `LFX_ALPHA_PREMULTIPLIED`, the working form. */
typedef uint32_t lfx_alpha;
enum {
    LFX_ALPHA_UNSET = 0u,
    LFX_ALPHA_PREMULTIPLIED = 1u,
    LFX_ALPHA_STRAIGHT = 2u
};

/* `lfx_traits.flags`. Zero is the pessimistic case here too: not seeded, safe
 * on any thread, and not cancellable. */
typedef uint32_t lfx_trait_flags;
enum {
    LFX_TRAIT_NONE = 0u,
    /* The effect reads its Seed row and must stay bit-identical between two
     * exports of the same project. */
    LFX_TRAIT_SEEDED = 1u << 0,
    /* The sole, discouraged opt-out from instance-level concurrency: the host
     * serialises the whole bundle. This flag **is** the opt-out docs/12 §3.4
     * names as a capability: there is no `lfx.thread-unsafe` extension id, and
     * asking `get_extension` for one returns the NULL that means "not
     * offered" - which would leave the plugin scheduled concurrently. */
    LFX_TRAIT_THREAD_UNSAFE = 1u << 1,
    /* `lfx_process.cancelled` is worth calling: the effect polls it and returns
     * `LFX_STATUS_CANCELLED` promptly. */
    LFX_TRAIT_CANCELLABLE = 1u << 2
};

/* Per-parameter flags. Zero is the ordinary row - visible, and animatable -
 * because that is what almost every control is; the flags name the departures
 * rather than the norm. */
typedef uint32_t lfx_param_flags;
enum {
    LFX_PARAM_FLAG_NONE = 0u,
    /* The row never keyframes: one value for the whole of the effect's life,
     * as a file choice or a curve is.
     *
     * *ponytail:* on a tone curve, a file choice and a button this is what
     * Lumit already does, so the flag asks for nothing. On every other kind
     * this build reads it, has nowhere to put it and says so in the scan
     * report (`LfxRejection::StaticRowAnimatesAnyway`): `ParamSchema` carries
     * no non-animatable field, and one there - read by the panel and by the
     * keyframe menu - is the whole of what honouring the declaration needs. */
    LFX_PARAM_FLAG_STATIC = 1u << 0,
    /* Declared, kept, serialised - and not drawn. */
    LFX_PARAM_FLAG_HIDDEN = 1u << 1
};

/* Which hard bounds a Float or an Int declaration actually means. Zero is "this
 * parameter runs unbounded", which is the honest default: a threshold that
 * clamps at nought below and runs free above declares one side only. */
typedef uint32_t lfx_bounds;
enum {
    LFX_BOUND_NONE = 0u,
    LFX_BOUND_MIN = 1u << 0,
    LFX_BOUND_MAX = 1u << 1
};

/* The level a `lfx_host.log` line is filed at, matching the host's own ladder:
 * error is reserved for something a user would want reported. */
typedef uint32_t lfx_log_level;
enum {
    LFX_LOG_UNSET = 0u,
    LFX_LOG_ERROR = 1u,
    LFX_LOG_WARN = 2u,
    LFX_LOG_INFO = 3u,
    LFX_LOG_DEBUG = 4u,
    LFX_LOG_TRACE = 5u
};

/* What `lfx_plugin.process` returns. A typed refusal, never a message: the host
 * turns each of these into a sentence of its own in the user's language. */
typedef int32_t lfx_status;
enum {
    LFX_STATUS_OK = 0,
    /* The effect could not produce this frame. The host renders the input
     * unchanged and badges the layer. */
    LFX_STATUS_FAILED = 1,
    /* The host asked for the work to stop and the plugin obliged. */
    LFX_STATUS_CANCELLED = 2,
    LFX_STATUS_OUT_OF_MEMORY = 3,
    /* The request carried something this build does not do - a depth, a format,
     * a region. Both depths are mandatory, so this is not the answer to fp16. */
    LFX_STATUS_UNSUPPORTED = 4
};

/* --------------------------------------------------------- extension ids -- */

/* Extensions are typed function-pointer tables fetched by name and version:
 * `host->get_extension(host, LFX_EXT_TEMPORAL, 1)`. A missing one is `NULL`,
 * never a status. A plugin that *requires* one the host has not got is refused
 * before it is instantiated, rather than left to fail somewhere later.
 *
 * Only `lfx.temporal` is offered in version 1. The other four are reserved ids
 * with headers of their own to come, named here so nobody mints a second
 * spelling of them. */
#define LFX_EXT_TEMPORAL "lfx.temporal"
#define LFX_EXT_GPU_FRAMES "lfx.gpu-frames"
#define LFX_EXT_OVERLAY "lfx.overlay"
#define LFX_EXT_MOTION_VECTORS "lfx.motion-vectors"
#define LFX_EXT_AUDIO "lfx.audio"

/* ------------------------------------------------------------- the traits -- */

/* What the host schedules from, declared once in the descriptor and never
 * again. Reached by pointer rather than embedded by value: two size-prefixed
 * structs cannot both grow when one is nested inside the other, and an offset
 * that depended on the `struct_size` a plugin happened to be built with is not
 * a number the layout suite could assert.
 *
 * A `NULL` pointer, a zeroed block, or a short one all mean the same thing -
 * the pessimistic case. */
typedef struct lfx_traits {
    uint32_t struct_size;
    lfx_cost cost;         /* UNSET -> heavy                                  */
    lfx_roi_kind roi_kind; /* UNSET -> full frame                             */
    /* The dilation `LFX_ROI_PADDED` asks for, in px@comp. Sized from the
     * effect's own hard maximum: a kernel that reaches further than this
     * produces tile seams. */
    float roi_padding_px;
    /* The frame window the effect reads, in comp frames, relative to the frame
     * being rendered - `[-1, 1]` for an effect that looks one frame either
     * side, `[0, 0]` for one that does not. **This is the gate**: a plugin that
     * declares no window never sees a neighbour, however loudly `lfx.temporal`
     * asks at render time.
     *
     * Each end reaches at most `LFX_MAX_TEMPORAL_WINDOW` frames, and the window
     * must contain the frame being rendered: `temporal_lo <= 0 <= temporal_hi`.
     * A window that does not is a refusal rather than a clamp - it is a
     * declaration nobody can honour, not an ambitious one. */
    int32_t temporal_lo;
    int32_t temporal_hi;
    lfx_alpha alpha;       /* UNSET -> premultiplied                          */
    lfx_trait_flags flags;
    /* The working memory one megapixel of output costs. The host refuses to
     * dispatch a region whose declared scratch exceeds what the resource ledger
     * will grant - LFX has no host allocator, so this declaration is the
     * ceiling, and on Windows a Job Object enforces it for liars. */
    uint32_t scratch_bytes_per_megapixel;
} lfx_traits;

/* ------------------------------------------------------ the describe sink -- */

/* One declared control, per kind. Every one of these opens with the same four
 * fields - size, unit, flags, and then whatever that kind alone needs - and
 * spells them out rather than nesting a shared header by value, for the reason
 * `lfx_traits` is reached by pointer.
 *
 * `id` is snake_case and stable for the control's life: the host hashes it, and
 * two rows that hash alike refuse the whole effect rather than ship one control
 * silently driving another. `label` is sentence case and is what a person
 * reads. */
typedef struct lfx_float_param {
    uint32_t struct_size;
    lfx_unit unit;     /* UNSET is a refusal                                  */
    lfx_param_flags flags;
    lfx_bounds bounds; /* which of the hard bounds are meant                  */
    const char *id;
    const char *label;
    double default_value;
    /* The slider's travel. Typing MAY exceed it - that is what makes this kind
     * a Float rather than a Slider - up to the hard bounds if any. */
    double slider_min;
    double slider_max;
    double hard_min; /* read only if `bounds` says so                         */
    double hard_max;
} lfx_float_param;

/* A **bounded** number: the range is the parameter's whole nature, so there is
 * no soft slider and hard bound to keep apart. */
typedef struct lfx_slider_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    /* Non-zero: the thumb moves through the range logarithmically, so a 20 Hz
     * to 20 kHz row spends half its travel below 1 kHz. Honest only above zero;
     * a range starting at nought draws linearly whatever this says. Lumit's own
     * effects can declare this and an OFX plugin cannot say it at all. */
    uint32_t log;
    const char *id;
    const char *label;
    double default_value;
    double range_min;
    double range_max;
} lfx_slider_param;

/* A whole number. It animates and serialises exactly as a Float does; the kind
 * tells the panel to step it and the host to round it. */
typedef struct lfx_int_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    lfx_bounds bounds;
    const char *id;
    const char *label;
    int64_t default_value;
    int64_t slider_min;
    int64_t slider_max;
    int64_t hard_min;
    int64_t hard_max;
} lfx_int_param;

/* An angle in degrees, drawn as a dial. Deliberately unbounded: an angle
 * animates through full turns rather than stopping at 360. `unit` must be
 * `LFX_UNIT_DEGREES`, an angle being degrees by definition. */
typedef struct lfx_angle_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t reserved_0; /* pad to the pointer's alignment; must be zero      */
    const char *id;
    const char *label;
    double default_value;
    double dial_step; /* the snapping increment while a modifier is held      */
} lfx_angle_param;

/* A switch. The default is `0` or `1`; nothing else is a boolean. */
typedef struct lfx_bool_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t default_value;
    const char *id;
    const char *label;
} lfx_bool_param;

/* A dropdown. The dividers are **declared** rather than guessed from the
 * labels: each entry is the index after which the list draws a rule. */
typedef struct lfx_choice_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t default_index;
    uint32_t option_count;  /* at most LFX_MAX_OPTIONS                        */
    uint32_t divider_count; /* at most LFX_MAX_DIVIDERS                       */
    const char *id;
    const char *label;
    const char *const *options;
    const uint32_t *dividers_after;
} lfx_choice_param;

/* Scene-linear RGBA. Channels animate independently, and the range is declared
 * per colour because a linear value may exceed one (an HDR tint) or dip below
 * nought (a lift). */
typedef struct lfx_colour_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t reserved_0; /* must be zero                                      */
    const char *id;
    const char *label;
    double default_rgba[4];
    double range_min;
    double range_max;
} lfx_colour_param;

/* An integer seed. There is deliberately **no declared default**: the host
 * draws one from the fresh instance's own id, so two copies of a seeded effect
 * never wobble in sync. */
typedef struct lfx_seed_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t reserved_0; /* must be zero                                      */
    const char *id;
    const char *label;
} lfx_seed_param;

/* A point. Lumit has deliberately no Point kind: this becomes two rows,
 * `<id>_x` and `<id>_y`, which the panel folds back into one crosshair row.
 * Both axes share the declared slider travel. */
typedef struct lfx_point2_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t reserved_0; /* must be zero                                      */
    const char *id;
    const char *label;
    double default_x;
    double default_y;
    double slider_min;
    double slider_max;
} lfx_point2_param;

/* Three rows: `<id>_x`, `<id>_y` and `<id>_z`. */
typedef struct lfx_point3_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t reserved_0; /* must be zero                                      */
    const char *id;
    const char *label;
    double default_x;
    double default_y;
    double default_z;
    double slider_min;
    double slider_max;
} lfx_point3_param;

/* A **tone** curve: the shape dragged in a Curves panel, as its own control
 * points. `LFX_MIN_CURVE_POINTS` to `LFX_MAX_CURVE_POINTS` points in the unit
 * square, `x` then `y`, sorted by `x`; `points` therefore holds
 * `2 * point_count` floats. Static in version 1, because a list that grows and
 * shrinks has nothing to interpolate between two keyframes.
 *
 * A bezier *path* is `LFX_PARAM_PATH` and waits for `lfx.overlay`. The two are
 * not the same control and never were. */
typedef struct lfx_curve_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t point_count; /* LFX_MIN_CURVE_POINTS..LFX_MAX_CURVE_POINTS       */
    const char *id;
    const char *label;
    const float *points;
} lfx_curve_param;

/* A file chosen from a dialog - a `.cube`, a trained table. The extensions are
 * lower case and carry no dot.
 *
 * The **payload rides beside the op, never in the value bag**: at `process` the
 * host fills `lfx_value.v.file.path` from the auxiliary slot it loaded, because
 * only the host knows which file actually opened. */
typedef struct lfx_file_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t filter_count; /* at most LFX_MAX_FILTERS                         */
    const char *id;
    const char *label;
    const char *const *filter;
    const char *filter_name;
} lfx_file_param;

/* A button: a row that asks the host to *do* something. No value, no keyframe,
 * and nothing in the value bag - it reaches the plugin as an event of its own,
 * never as a number. */
typedef struct lfx_action_param {
    uint32_t struct_size;
    lfx_unit unit;
    lfx_param_flags flags;
    uint32_t reserved_0; /* must be zero                                      */
    const char *id;
    const char *label;
} lfx_action_param;

/* A heading over the rows declared until the matching `group_end`. It is a run,
 * not a row: nothing is stored for it and nothing animates. */
typedef struct lfx_group_param {
    uint32_t struct_size;
    lfx_param_flags flags; /* HIDDEN hides the whole run                      */
    const char *id;
    const char *label;
} lfx_group_param;

struct lfx_describe_sink;

/* What `describe` pushes its declarations into. There is no key, no
 * string-valued answer, and no question the host can ask that a plugin can
 * answer in the wrong type - the plugin pushes typed records and the host
 * writes them down in the order they arrive.
 *
 * Every call returns `true` if the declaration was accepted. `false` is the
 * graceful refusal of **that one row**: a kind this version does not admit
 * (`LFX_PARAM_STRING`, `LFX_PARAM_PATH`), or a declaration the host cannot
 * represent. The plugin carries on declaring, each refusal is a line in the
 * host's scan report, the effect still loads, and the row keeps the default it
 * was declared with for the effect's life.
 *
 * **Two faults are not that, and the return value does not distinguish them.**
 * A duplicate `id` and an unset `unit` are structural, and they refuse the
 * whole effect: two rows hashing to one parameter id would ship one control
 * silently driving another, and a unit cannot be guessed at all. A `false` for
 * either means the effect will not be catalogued whatever the plugin declares
 * next. Carrying on after a refusal is allowed and costs nothing; it is simply
 * not always enough to save the effect.
 *
 * **Control thread only**: every function here may be called only from inside
 * `lfx_plugin.describe`, on the thread the host called it on. */
typedef struct lfx_describe_sink {
    uint32_t struct_size;
    /* The host's own, opaque. A plugin passes the sink back unchanged. */
    void *sink_data;
    bool (*declare_float)(struct lfx_describe_sink *, const lfx_float_param *);
    bool (*declare_slider)(struct lfx_describe_sink *, const lfx_slider_param *);
    bool (*declare_int)(struct lfx_describe_sink *, const lfx_int_param *);
    bool (*declare_angle)(struct lfx_describe_sink *, const lfx_angle_param *);
    bool (*declare_bool)(struct lfx_describe_sink *, const lfx_bool_param *);
    bool (*declare_choice)(struct lfx_describe_sink *, const lfx_choice_param *);
    bool (*declare_colour)(struct lfx_describe_sink *, const lfx_colour_param *);
    bool (*declare_seed)(struct lfx_describe_sink *, const lfx_seed_param *);
    bool (*declare_point2)(struct lfx_describe_sink *, const lfx_point2_param *);
    bool (*declare_point3)(struct lfx_describe_sink *, const lfx_point3_param *);
    bool (*declare_curve)(struct lfx_describe_sink *, const lfx_curve_param *);
    bool (*declare_file)(struct lfx_describe_sink *, const lfx_file_param *);
    bool (*declare_action)(struct lfx_describe_sink *, const lfx_action_param *);
    bool (*group_begin)(struct lfx_describe_sink *, const lfx_group_param *);
    bool (*group_end)(struct lfx_describe_sink *);
} lfx_describe_sink;

/* ------------------------------------------------------------- the values -- */

/* One resolved control value at the frame being rendered.
 *
 * **This is the one struct with no `struct_size`, and the stride is why.** The
 * values cross as a dense array in declaration order, addressed by index;
 * `lfx_process.value_stride` is the number of bytes between two elements and is
 * what both sides walk by. A plugin that strides by its own `sizeof` after this
 * struct has grown reads correct-looking kind tags over silently wrong values,
 * which no runtime check can see.
 *
 * `kind` is the tag the declaration minted, so a kind mismatch is impossible
 * rather than a runtime status. A declaration the host carries no value for -
 * an Action, a Group - is not in the array at all.
 *
 * **One declaration is one element**, including the ones the host spreads over
 * rows of its own: a `LFX_PARAM_POINT2` contributes a single element whose
 * `v.xy` carries both axes, and a `LFX_PARAM_POINT3` a single `v.xyz`. The
 * host's `<id>_x` / `<id>_y` rows are folded back before the array is written,
 * so the count here is the count of declarations and never the count of rows.
 *
 * **A colour and a point are declared wide and arrive narrow.**
 * `lfx_colour_param`, `lfx_point2_param` and `lfx_point3_param` state their
 * defaults as `double`, because a declaration is read once and precision there
 * costs nothing; the host's resolved value bag is single precision, so the
 * narrowing happens on the way in and `rgba`, `xy` and `xyz` are `float`. */
typedef struct lfx_value {
    /* Which declaration this is: its index among the declarations that carry a
     * value, in declaration order - counting an Action or a Group not at all,
     * and a point once. It is the element's own index too, so
     * `values[i].param == i` always; it is carried so a plugin walking a
     * strided array can check that it walked it correctly. */
    uint32_t param;
    lfx_param_kind kind; /* the tag, checked by the wrapper                   */
    union {
        double f;        /* LFX_PARAM_FLOAT, LFX_PARAM_SLIDER, LFX_PARAM_ANGLE */
        int64_t i;       /* LFX_PARAM_INT, LFX_PARAM_SEED                      */
        bool b;          /* LFX_PARAM_BOOL                                     */
        uint32_t choice; /* LFX_PARAM_CHOICE: the chosen index                 */
        float rgba[4]; /* LFX_PARAM_COLOUR, scene-linear                      */
        float xy[2];   /* **both** axes of a LFX_PARAM_POINT2, never one      */
        float xyz[3];  /* all three axes of a LFX_PARAM_POINT3                */
        struct {
            const float *pt; /* `2 * n` floats, x then y, in the unit square  */
            uint32_t n;
        } curve;
        struct {
            /* The file the host actually loaded, filled from the auxiliary slot
             * beside the op, because only the host knows which file opened.
             * NULL when nothing loaded - which, until the generic file aux
             * lands, is every File row (*ponytail:* see `LFX_PARAM_FILE`). */
            const char *path;
        } file;
    } v;
} lfx_value;

/* -------------------------------------------------------------- the frame -- */

/* One picture crossing the boundary: scene-linear, premultiplied, tightly
 * packed, top-down. The host never converts a depth to accommodate a plugin, so
 * `format` is the project's and both values are mandatory.
 *
 * **Both frames are the host's**, and in the shipping host each is a slot of a
 * shared-memory ring. The frame reached through `lfx_process.input` is read
 * only for the duration of the call: writing through its `data` is undefined
 * however permissive the mapping happens to be, and corrupts whatever the host
 * does with that slot next. Only `output->data` may be written, and only from
 * inside `process`.
 *
 * Neither frame outlives the `process` call that carried it. A pointer kept
 * past the return - for a worker of the plugin's own, for the next frame -
 * names a slot the host has since given to something else. */
typedef struct lfx_frame {
    uint32_t struct_size;
    lfx_pixel_format format;
    uint32_t width;
    uint32_t height;
    /* Bytes from the start of one row to the start of the next. Tightly packed
     * today; read it rather than computing it. */
    uint32_t row_bytes;
    /* Where this buffer's top-left pixel sits, in the same space the request's
     * region is given in. */
    int32_t origin_x;
    int32_t origin_y;
    uint32_t reserved_0; /* pad to the pointer's alignment; must be zero      */
    void *data;
    /* The comp frame this picture is, as a decimal. Equal to
     * `lfx_process.time` for the effect's own input; a neighbour fetched
     * through `lfx.temporal` carries its own. */
    double time;
} lfx_frame;

struct lfx_process;

/* One request to render one frame.
 *
 * **Any worker thread**, and two instances of one plugin may be inside
 * `process` at once. One instance is never re-entered.
 *
 * The request, its `values` array and both frames are valid **only for the
 * duration of the call**. A plugin may not keep the pointer and may not read it
 * from a thread of its own once `process` has returned - including to poll
 * `cancelled`, which answers only while the host is still waiting for this
 * frame. */
typedef struct lfx_process {
    uint32_t struct_size;
    lfx_pixel_format pixel_format; /* matches both frames                     */
    /* Bytes between two elements of `values`. **Index by this, never by
     * `sizeof(lfx_value)`.** It is at least `sizeof(lfx_value)` as the host
     * built it and a multiple of that struct's alignment, and `values` itself
     * is aligned for it - so
     * `*(const lfx_value *)((const char *)p->values + i * p->value_stride)` is
     * an aligned read on every target. */
    uint32_t value_stride;
    uint32_t value_count;
    /* The output region asked for, in pixels of the raster in play, `x0`/`y0`
     * inclusive and `x1`/`y1` exclusive. Full-frame is the degenerate case, not
     * the assumption. */
    int32_t roi_x0;
    int32_t roi_y0;
    int32_t roi_x1;
    int32_t roi_y1;
    /* The input's definition: where there are pixels at all. */
    int32_t dod_x0;
    int32_t dod_y0;
    int32_t dod_x1;
    int32_t dod_y1;
    double time; /* the comp frame being rendered, as a decimal              */
    /* `value_count` elements, in declaration order, strided. */
    const lfx_value *values;
    const lfx_frame *input;
    lfx_frame *output;
    /* True once the host has stopped wanting this frame. Worth calling only if
     * the effect declared `LFX_TRAIT_CANCELLABLE`; the honest answer to a true
     * is `LFX_STATUS_CANCELLED`, promptly. Never NULL. */
    bool (*cancelled)(const struct lfx_process *);
    /* The host's own, opaque, and the reason `cancelled` takes the request back
     * rather than nothing. */
    void *host_context;
} lfx_process;

/* --------------------------------------------------------------- the host -- */

struct lfx_host;

/* What the plugin is handed: the mirror image of its own `get_extension`, and
 * somewhere to say something. That is the whole of it.
 *
 * The pointer handed to `create` is valid until `deinit` and **may be kept**:
 * it is what a plugin logs and fetches extensions through from inside
 * `process`. It is the only pointer in this file with that lifetime - every
 * other one a plugin is handed lives for the length of one call. */
typedef struct lfx_host {
    uint32_t struct_size;
    uint32_t abi_version; /* the host's, which may be newer than the plugin's */
    void *host_data;      /* opaque; passed back on every call                */
    /* The typed table for `id` at `version`, or NULL. **Any thread.** */
    const void *(*get_extension)(const struct lfx_host *, const char *id, uint32_t version);
    /* One line of diagnostics, filed at `level`, at most `LFX_MAX_LOG_BYTES`
     * long. It is not a user-facing message: the host's own sentences are
     * translated, and this is not. **Any thread**, and it must not be called
     * from a signal handler. */
    void (*log)(const struct lfx_host *, lfx_log_level level, const char *message);
} lfx_host;

/* -------------------------------------------------------- the descriptor -- */

/* What one effect in the bundle *is*, answered without creating it. The bundle
 * also states all of this in its manifest, which the host reads before it opens
 * the module at all; the manifest is the cheap listing and this struct is the
 * truth, and a disagreement once the module is open refuses the plugin. */
typedef struct lfx_descriptor {
    uint32_t struct_size;
    /* Reverse-DNS, stable for the plugin's life: it is half of the identity the
     * host's frame keys are minted from. */
    const char *id;
    const char *name;   /* what a person reads in the Add-effect menu         */
    const char *vendor; /* who to blame, shown in the row's context menu      */
    /* All three numbers re-key the host's cached frames, so a release whose
     * maths moved must move one of them. Minor and patch below 1000. */
    uint32_t major;
    uint32_t minor;
    uint32_t patch;
    /* The **first is the heading** the effect is browsed under; the rest are
     * search keywords. */
    const lfx_category *categories;
    uint32_t category_count; /* 1..LFX_MAX_CATEGORIES                         */
    /* NULL is the pessimistic case, and means it. */
    const lfx_traits *traits;
    /* The extensions without which this effect cannot work - the code's own
     * answer, checked against what the manifest declared. A plugin that asks
     * for one the host has not got is refused before it is instantiated, with
     * the extension named. */
    const char *const *required_extensions;
    uint32_t required_extension_count; /* at most LFX_MAX_REQUIRED_EXTENSIONS */
} lfx_descriptor;

/* ------------------------------------------------------------ the plugin -- */

struct lfx_plugin;

/* One live effect. The host creates one per in-flight frame from its instance
 * pool; a plugin holds **no opaque state of its own** - the host's values are
 * the only truth, which is what makes a frame key complete and a restart an
 * exact replay.
 *
 * `plugin_data` just below is not a contradiction of that, and what it is for
 * is worth spelling out. It is the instance's own `this`: scratch, a
 * working buffer, a table decoded once - anything **derivable from the values
 * in `lfx_process` alone**. Nothing in it reaches the host's frame key, so
 * anything in it that can change the output is a stale-frame bug: the host will
 * serve a cached frame from before it changed, and be right to. What "no opaque
 * state" refuses is the host-persisted blob OFX calls plugin state - there is
 * nothing here the host saves for a plugin and nothing it hands back, which is
 * what makes a restart an exact replay of the values. */
typedef struct lfx_plugin {
    uint32_t struct_size;
    /* The instance's own, untouched by the host and invisible to it. Its
     * contents must be derivable from the values `process` is handed. */
    void *plugin_data;
    /* Prepare this instance. Non-zero accepts it; zero refuses it, and the host
     * badges the layer rather than failing the frame. A `uint32_t` because it
     * is the plugin's answer and not the host's - see the top of this file.
     * **Control thread.** */
    uint32_t (*init)(struct lfx_plugin *);
    /* **Control thread**, and never while a `process` is running. */
    void (*destroy)(struct lfx_plugin *);
    /* Declare every control, in the order they should be drawn. Non-zero when
     * the plugin declared what it meant to; zero refuses the effect, which the
     * host reports by name. A `uint32_t` for the reason `init` is.
     * **Control thread.** */
    uint32_t (*describe)(struct lfx_plugin *, lfx_describe_sink *);
    /* Render one frame. Returns an `lfx_status`. **Any worker thread**, one
     * call per instance at a time. */
    int32_t (*process)(struct lfx_plugin *, const lfx_process *);
    /* The plugin's side of the extension handshake: its typed table for `id` at
     * `version`, or NULL. **Any thread.** */
    const void *(*get_extension)(struct lfx_plugin *, const char *id, uint32_t version);
} lfx_plugin;

/* ------------------------------------------------------------- the entry -- */

/* The one exported object. Everything else in this file is reached from here.
 *
 * `init` is called once per process, **after** the manifest has been read and
 * before anything else; it is handed the bundle's own directory so a plugin can
 * find its resources without guessing. */
typedef struct lfx_entry {
    uint32_t struct_size;
    uint32_t abi_version; /* LFX_ABI_VERSION as this bundle was built against */
    /* **Control thread.** Non-zero loads; zero means the bundle declines to
     * load, and the host says so without opening anything else. A `uint32_t`
     * for the reason `lfx_plugin.init` is. */
    uint32_t (*init)(const char *bundle_path);
    /* **Control thread**, once, last. */
    void (*deinit)(void);
    /* How many effects this bundle holds, at most
     * `LFX_MAX_EFFECTS_PER_BUNDLE`. **Control thread.** */
    uint32_t (*count)(void);
    /* The descriptor at `index`, or NULL past the end. It must stay valid and
     * unchanged until `deinit`. **Control thread.** */
    const lfx_descriptor *(*descriptor)(uint32_t index);
    /* One instance of the effect named `id`, or NULL. **Control thread.** */
    struct lfx_plugin *(*create)(const lfx_host *, const char *id);
} lfx_entry;

/* The symbol every bundle exports, spelled `LFX_ENTRY_SYMBOL` for the host's
 * loader. */
extern const lfx_entry lfx_entry_point;

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* LFX_H */
