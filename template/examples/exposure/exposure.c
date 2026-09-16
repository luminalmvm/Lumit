/*
 * exposure.c - the smallest honest LFX plugin, in C.
 *
 * SPDX-License-Identifier: MIT
 * Copyright (c) 2026 The Lumit authors.
 *
 * In plain terms
 *
 * One effect, four controls, and every rule the frozen header asks of a
 * plugin. It multiplies the picture by two to the power of a Stops control,
 * tints it, optionally carries the alpha with it, and clips what the author
 * asked to clip. The maths is three lines; everything around it is the part
 * worth copying.
 *
 * Five things here are the ones a first plugin gets wrong:
 *
 *   1. **Every struct opens with its own `sizeof`.** That is how the host
 *      reads a bundle built against an older header, and how a later header
 *      reads this one.
 *   2. **The value array is walked by `value_stride`, never by
 *      `sizeof(lfx_value)`.** The host may write a wider element than this
 *      build was compiled against; striding by the local size would read
 *      correct-looking kind tags over the wrong values, which no check at
 *      runtime can see.
 *   3. **The buffer is the definition and the region is inside it.** A frame
 *      carries its own top-left corner, so the first requested pixel is at
 *      `roi_x0 - origin_x`. Drawing the region at the buffer's corner is the
 *      classic tile seam.
 *   4. **Both depths are mandatory.** fp16 and fp32 are the project's choice,
 *      not the plugin's, and the host never converts to accommodate one. C has
 *      no standard half type, so the two conversions are written out below.
 *   5. **Nothing is kept between frames.** The values the host hands over are
 *      the only truth; anything remembered here would be a cached frame the
 *      host is right to serve and the user is right to call stale.
 *
 * Build it with the CMakeLists at the top of this repository, which also lays
 * the result out as `Exposure.lfx.bundle`, and then run `lfx-validator` over
 * that bundle.
 */

#include "lfx.h"

#include <math.h>
#include <stdlib.h>
#include <string.h>

/* MSVC exports a symbol from a DLL only when it is asked to, and the frozen
 * header declares `lfx_entry_point` without an export attribute - so the ask
 * goes on the link line rather than on the declaration, where it would be an
 * inconsistent-linkage error. Everything else exports by default. */
#if defined(_MSC_VER)
#pragma comment(linker, "/EXPORT:lfx_entry_point,DATA")
#endif

/* ----------------------------------------------------------- the halves -- */

/* fp16 to fp32. Written out because C has no standard half type and the depth
 * is the project's choice rather than this plugin's. */
static float half_to_float(uint16_t bits) {
    uint32_t sign = (uint32_t)(bits & 0x8000u) << 16;
    uint32_t exponent = (uint32_t)((bits >> 10) & 0x1fu);
    uint32_t mantissa = (uint32_t)(bits & 0x3ffu);
    uint32_t whole;
    float out;

    if (exponent == 0u) {
        if (mantissa == 0u) {
            whole = sign;
        } else {
            /* A subnormal half is a normal float: shift the leading one up
             * into place and pay for it in the exponent. */
            int32_t shifted = 1;
            while ((mantissa & 0x400u) == 0u) {
                mantissa <<= 1;
                shifted--;
            }
            mantissa &= 0x3ffu;
            whole = sign | ((uint32_t)(shifted + 112) << 23) | (mantissa << 13);
        }
    } else if (exponent == 31u) {
        whole = sign | 0x7f800000u | (mantissa << 13);
    } else {
        whole = sign | ((exponent + 112u) << 23) | (mantissa << 13);
    }
    memcpy(&out, &whole, sizeof out);
    return out;
}

/* fp32 to fp16, rounding to nearest and ties to even - which is what the host
 * does on its own side of the boundary. */
static uint16_t float_to_half(float value) {
    uint32_t whole;
    uint32_t sign;
    uint32_t biased;
    int32_t exponent;
    uint32_t mantissa;

    memcpy(&whole, &value, sizeof whole);
    sign = (whole >> 16) & 0x8000u;
    biased = (whole >> 23) & 0xffu;
    mantissa = whole & 0x7fffffu;
    exponent = (int32_t)biased - 127 + 15;

    if (biased == 0xffu) {
        /* Infinity keeps its sign; a NaN stays a NaN rather than becoming
         * one. */
        return (uint16_t)(sign | 0x7c00u |
                          (mantissa != 0u ? (0x200u | (mantissa >> 13)) : 0u));
    }
    if (exponent >= 31) {
        return (uint16_t)(sign | 0x7c00u);
    }
    if (exponent <= 0) {
        uint32_t shift;
        uint32_t narrowed;
        uint32_t remainder;
        uint32_t halfway;
        if (exponent < -10) {
            return (uint16_t)sign;
        }
        mantissa |= 0x800000u;
        shift = (uint32_t)(14 - exponent);
        narrowed = mantissa >> shift;
        remainder = mantissa & ((1u << shift) - 1u);
        halfway = 1u << (shift - 1u);
        if (remainder > halfway || (remainder == halfway && (narrowed & 1u) != 0u)) {
            narrowed++;
        }
        return (uint16_t)(sign | narrowed);
    }
    {
        uint32_t narrowed = ((uint32_t)exponent << 10) | (mantissa >> 13);
        uint32_t remainder = mantissa & 0x1fffu;
        if (remainder > 0x1000u || (remainder == 0x1000u && (narrowed & 1u) != 0u)) {
            narrowed++;
        }
        return (uint16_t)(sign | narrowed);
    }
}

/* ------------------------------------------------------- the declarations -- */

/* The order these are pushed in is the order the panel draws them, and the
 * order the value array arrives in. A group takes no element of its own, so
 * the four controls are elements 0 to 3. */
enum {
    VALUE_STOPS = 0u,
    VALUE_TINT = 1u,
    VALUE_AFFECT_ALPHA = 2u,
    VALUE_CLIP = 3u
};

/* What the Clip dropdown's indices mean. A value the host sends that is not
 * one of these clips nothing: a plugin that indexed an array with a stranger's
 * number would be reading somebody else's memory on a typo. */
enum {
    CLIP_NOTHING = 0u,
    CLIP_BELOW_ZERO = 1u,
    CLIP_TO_THE_UNIT_RANGE = 2u
};

static const char *const clip_options[] = {
    "Nothing",
    "Below zero",
    "Below zero and above one"
};

/* The dropdown draws a rule after the first entry: the declaration says where,
 * rather than leaving the panel to guess it from the labels. */
static const uint32_t clip_dividers[] = {0u};

static const lfx_group_param exposure_group = {
    sizeof(lfx_group_param),
    LFX_PARAM_FLAG_NONE,
    "exposure",
    "Exposure"
};

static const lfx_slider_param stops_param = {
    sizeof(lfx_slider_param),
    /* A number of stops is a plain number: no unit dresses it up. */
    LFX_UNIT_RAW,
    LFX_PARAM_FLAG_NONE,
    0u, /* linear travel; a stop is already logarithmic */
    "stops",
    "Stops",
    0.0,
    -12.0,
    12.0
};

static const lfx_colour_param tint_param = {
    sizeof(lfx_colour_param),
    /* A colour carries no unit and the host says so: `LFX_UNIT_RAW` is the
     * only spelling that costs no line in the scan report. */
    LFX_UNIT_RAW,
    LFX_PARAM_FLAG_NONE,
    0u,
    "tint",
    "Tint",
    {1.0, 1.0, 1.0, 1.0},
    0.0,
    4.0
};

static const lfx_bool_param affect_alpha_param = {
    sizeof(lfx_bool_param),
    LFX_UNIT_RAW,
    LFX_PARAM_FLAG_NONE,
    0u,
    "affect_alpha",
    "Affect alpha"
};

static const lfx_choice_param clip_param = {
    sizeof(lfx_choice_param),
    LFX_UNIT_RAW,
    LFX_PARAM_FLAG_NONE,
    CLIP_BELOW_ZERO,
    (uint32_t)(sizeof clip_options / sizeof clip_options[0]),
    (uint32_t)(sizeof clip_dividers / sizeof clip_dividers[0]),
    "clip",
    "Clip",
    clip_options,
    clip_dividers
};

/* --------------------------------------------------------- the descriptor -- */

/* The first category is the heading this effect is browsed under; the rest are
 * search keywords. The vocabulary is closed, which is what lets an LFX effect
 * sit under Lumit's own headings rather than in a plugin ghetto. */
static const lfx_category exposure_categories[] = {
    LFX_CATEGORY_COLOUR,
    LFX_CATEGORY_UTILITY
};

/* What the host schedules from. Every field here is read once, at describe,
 * and never again - and every zero in it is a declaration of the pessimistic
 * case, so the block is filled in deliberately rather than memset.
 *
 * `LFX_ROI_EXACT` is the truthful answer for a per-pixel effect: it reads the
 * output pixel and nothing around it, so the host may tile the frame however
 * it likes. A kernel that reached further and said this would produce tile
 * seams, which is a correctness bug rather than a slow render. */
static const lfx_traits exposure_traits = {
    sizeof(lfx_traits),
    LFX_COST_CHEAP,
    LFX_ROI_EXACT,
    0.0f,
    0, /* no neighbouring frames are read … */
    0, /* … so the temporal window is this frame alone */
    LFX_ALPHA_PREMULTIPLIED,
    LFX_TRAIT_CANCELLABLE,
    0u /* no scratch: this effect writes straight into the output */
};

static const lfx_descriptor exposure_descriptor = {
    sizeof(lfx_descriptor),
    "com.example.lfx.exposure",
    "Exposure",
    "Example",
    1u,
    0u,
    0u,
    exposure_categories,
    (uint32_t)(sizeof exposure_categories / sizeof exposure_categories[0]),
    &exposure_traits,
    /* Nothing beyond the frozen core, so nothing to negotiate. A plugin that
     * asks here for an extension the host has not got is refused before it is
     * instantiated, with the extension named. */
    NULL,
    0u
};

/* ----------------------------------------------------------- the instance -- */

/* One live effect. It holds the host pointer - the one pointer in the ABI a
 * plugin may keep - and nothing else: there is nowhere here to put state the
 * host does not know about, because there is nothing the host would hash. */
typedef struct exposure_instance {
    lfx_plugin table;
    const lfx_host *host;
} exposure_instance;

static uint32_t exposure_init(lfx_plugin *plugin) {
    (void)plugin;
    return 1u;
}

static void exposure_destroy(lfx_plugin *plugin) {
    /* Freed through `plugin_data` rather than through the table's own address,
     * which is the same pointer only because the table happens to be the first
     * member. One of those two facts is a promise this file makes; the other
     * is a coincidence of the layout. */
    if (plugin != NULL) {
        free(plugin->plugin_data);
    }
}

static uint32_t exposure_describe(lfx_plugin *plugin, lfx_describe_sink *sink) {
    (void)plugin;
    if (sink == NULL || sink->declare_slider == NULL || sink->declare_colour == NULL ||
        sink->declare_bool == NULL || sink->declare_choice == NULL ||
        sink->group_begin == NULL || sink->group_end == NULL) {
        return 0u;
    }
    /* Each call answers whether the host took the declaration. A `false` is
     * the graceful refusal of that one row - the effect still loads and the
     * row keeps the default declared here - so declaring is worth carrying on
     * with rather than giving up at the first no. */
    sink->group_begin(sink, &exposure_group);
    sink->declare_slider(sink, &stops_param);
    sink->declare_colour(sink, &tint_param);
    sink->group_end(sink);
    sink->declare_bool(sink, &affect_alpha_param);
    sink->declare_choice(sink, &clip_param);
    return 1u;
}

/* ------------------------------------------------------------- the values -- */

/* One element of the dense value array, **read by the host's own stride**.
 *
 * `param` is the element's own index, carried so that a plugin walking the
 * array can check that it walked it correctly. A mismatch means this build
 * strode wrongly, which is a reason to refuse the frame rather than to paint
 * one from numbers that are not the ones asked for. */
static const lfx_value *value_at(const lfx_process *request, uint32_t index) {
    const lfx_value *element;
    if (request->values == NULL || index >= request->value_count ||
        request->value_stride == 0u) {
        return NULL;
    }
    element = (const lfx_value *)((const char *)request->values +
                                 (size_t)index * (size_t)request->value_stride);
    return element->param == index ? element : NULL;
}

/* The tag says which arm of the union to read, which is what makes a kind
 * mismatch impossible here rather than a status at runtime. */
static double number_at(const lfx_process *request, uint32_t index, double spare) {
    const lfx_value *element = value_at(request, index);
    if (element == NULL || element->kind != LFX_PARAM_SLIDER) {
        return spare;
    }
    return element->v.f;
}

static int switched_on(const lfx_process *request, uint32_t index) {
    const lfx_value *element = value_at(request, index);
    if (element == NULL || element->kind != LFX_PARAM_BOOL) {
        return 0;
    }
    return element->v.b ? 1 : 0;
}

static uint32_t chosen_at(const lfx_process *request, uint32_t index, uint32_t spare) {
    const lfx_value *element = value_at(request, index);
    if (element == NULL || element->kind != LFX_PARAM_CHOICE) {
        return spare;
    }
    return element->v.choice;
}

static void colour_at(const lfx_process *request, uint32_t index, float out[4]) {
    const lfx_value *element = value_at(request, index);
    int channel;
    if (element == NULL || element->kind != LFX_PARAM_COLOUR) {
        for (channel = 0; channel < 4; channel++) {
            out[channel] = 1.0f;
        }
        return;
    }
    for (channel = 0; channel < 4; channel++) {
        out[channel] = element->v.rgba[channel];
    }
}

/* -------------------------------------------------------------- the frame -- */

/* What this effect does to one pixel. Everything above is plumbing; this is
 * the effect. */
static void shade(float rgba[4], const float gain[4], uint32_t clip) {
    int channel;
    for (channel = 0; channel < 4; channel++) {
        float sample = rgba[channel] * gain[channel];
        if (clip == CLIP_BELOW_ZERO || clip == CLIP_TO_THE_UNIT_RANGE) {
            if (!(sample > 0.0f)) {
                /* Written as a negated comparison so that a NaN clips to
                 * nought rather than travelling on into the picture. */
                sample = 0.0f;
            }
        }
        if (clip == CLIP_TO_THE_UNIT_RANGE && sample > 1.0f) {
            sample = 1.0f;
        }
        rgba[channel] = sample;
    }
}

static int32_t exposure_process(lfx_plugin *plugin, const lfx_process *request) {
    const lfx_frame *in;
    lfx_frame *out;
    float tint[4];
    float gain[4];
    double stops;
    uint32_t clip;
    int32_t x0, y0, x1, y1;
    int32_t y;
    size_t sample_bytes;

    (void)plugin;
    if (request == NULL || request->input == NULL || request->output == NULL) {
        return LFX_STATUS_FAILED;
    }
    in = request->input;
    out = request->output;
    if (in->data == NULL || out->data == NULL) {
        return LFX_STATUS_FAILED;
    }
    /* The host sends the project's depth and never converts to accommodate a
     * plugin, so a request whose three formats disagree is one this build
     * cannot honour rather than one to guess at. */
    if (in->format != request->pixel_format || out->format != request->pixel_format) {
        return LFX_STATUS_UNSUPPORTED;
    }
    if (request->pixel_format == LFX_RGBA_F32) {
        sample_bytes = sizeof(float);
    } else if (request->pixel_format == LFX_RGBA_F16) {
        sample_bytes = sizeof(uint16_t);
    } else {
        return LFX_STATUS_UNSUPPORTED;
    }

    stops = number_at(request, VALUE_STOPS, 0.0);
    colour_at(request, VALUE_TINT, tint);
    clip = chosen_at(request, VALUE_CLIP, CLIP_NOTHING);
    {
        float exposure = (float)pow(2.0, stops);
        gain[0] = exposure * tint[0];
        gain[1] = exposure * tint[1];
        gain[2] = exposure * tint[2];
        /* Lumit's frames are premultiplied, so scaling the alpha with the
         * colour is what keeps a picture premultiplied - and leaving it alone
         * is what makes the effect a light source rather than a fade. The
         * author gets to say which. */
        gain[3] = switched_on(request, VALUE_AFFECT_ALPHA) ? exposure : 1.0f;
    }

    /* The region asked for, clamped to the buffer it is asked out of. The
     * host promises the two agree; a plugin that reads its own promise is a
     * plugin that survives a host that ever stops keeping it. */
    x0 = request->roi_x0 > in->origin_x ? request->roi_x0 : in->origin_x;
    y0 = request->roi_y0 > in->origin_y ? request->roi_y0 : in->origin_y;
    x1 = request->roi_x1 < in->origin_x + (int32_t)in->width
             ? request->roi_x1
             : in->origin_x + (int32_t)in->width;
    y1 = request->roi_y1 < in->origin_y + (int32_t)in->height
             ? request->roi_y1
             : in->origin_y + (int32_t)in->height;

    for (y = y0; y < y1; y++) {
        const char *source;
        char *destination;
        int32_t x;

        /* Polled per row rather than per pixel: often enough that a cancelled
         * export stops promptly, rarely enough that the call is free. The
         * effect declared `LFX_TRAIT_CANCELLABLE`, which is what makes this
         * worth asking at all. */
        if (request->cancelled != NULL && request->cancelled(request)) {
            return LFX_STATUS_CANCELLED;
        }

        source = (const char *)in->data + (size_t)(y - in->origin_y) * in->row_bytes;
        destination = (char *)out->data + (size_t)(y - out->origin_y) * out->row_bytes;
        for (x = x0; x < x1; x++) {
            float rgba[4];
            int channel;
            size_t from = (size_t)(x - in->origin_x) * 4u * sample_bytes;
            size_t to = (size_t)(x - out->origin_x) * 4u * sample_bytes;

            if (request->pixel_format == LFX_RGBA_F32) {
                const float *read = (const float *)(const void *)(source + from);
                float *write = (float *)(void *)(destination + to);
                for (channel = 0; channel < 4; channel++) {
                    rgba[channel] = read[channel];
                }
                shade(rgba, gain, clip);
                for (channel = 0; channel < 4; channel++) {
                    write[channel] = rgba[channel];
                }
            } else {
                const uint16_t *read = (const uint16_t *)(const void *)(source + from);
                uint16_t *write = (uint16_t *)(void *)(destination + to);
                for (channel = 0; channel < 4; channel++) {
                    rgba[channel] = half_to_float(read[channel]);
                }
                shade(rgba, gain, clip);
                for (channel = 0; channel < 4; channel++) {
                    write[channel] = float_to_half(rgba[channel]);
                }
            }
        }
    }
    return LFX_STATUS_OK;
}

static const void *exposure_get_extension(lfx_plugin *plugin, const char *id,
                                          uint32_t version) {
    (void)plugin;
    (void)id;
    (void)version;
    /* NULL is "not offered", which is the honest answer for a plugin that
     * implements none of them. It is never a status. */
    return NULL;
}

/* -------------------------------------------------------------- the entry -- */

static uint32_t entry_init(const char *bundle_path) {
    /* The bundle's own directory, so resources are found rather than guessed
     * at. This effect has none, and says so by ignoring it. */
    (void)bundle_path;
    return 1u;
}

static void entry_deinit(void) {
}

static uint32_t entry_count(void) {
    return 1u;
}

static const lfx_descriptor *entry_descriptor(uint32_t index) {
    return index == 0u ? &exposure_descriptor : NULL;
}

static lfx_plugin *entry_create(const lfx_host *host, const char *id) {
    exposure_instance *instance;
    if (id == NULL || strcmp(id, exposure_descriptor.id) != 0) {
        return NULL;
    }
    instance = (exposure_instance *)calloc(1u, sizeof *instance);
    if (instance == NULL) {
        return NULL;
    }
    instance->table.struct_size = (uint32_t)sizeof(lfx_plugin);
    instance->table.plugin_data = instance;
    instance->table.init = exposure_init;
    instance->table.destroy = exposure_destroy;
    instance->table.describe = exposure_describe;
    instance->table.process = exposure_process;
    instance->table.get_extension = exposure_get_extension;
    instance->host = host;
    /* The instance table is the first member, so the host's pointer and this
     * allocation are the same address - which is what lets `destroy` free it. */
    return &instance->table;
}

const lfx_entry lfx_entry_point = {
    sizeof(lfx_entry),
    LFX_ABI_VERSION,
    entry_init,
    entry_deinit,
    entry_count,
    entry_descriptor,
    entry_create
};
