// lfx.hpp - a header-only C++ wrapper over the frozen LFX C ABI.
//
// SPDX-License-Identifier: MIT
// Copyright (c) 2026 The Lumit authors.
//
// In plain terms
//
// `include/lfx.h` is the agreement; this file is the part of it a C++ author
// would otherwise write again in every plugin. It carries no state of its own
// and adds no behaviour the header does not already promise:
//
//   * the two half-float conversions, because C++ has no standard half either;
//   * `lfx::describe`, one method per declaration kind, so a control is
//     declared in one line with its unit and its default beside it;
//   * `lfx::values`, which walks the dense value array **by the host's own
//     stride** and hands back the arm of the union the kind tag names;
//   * `lfx::request`, which knows that the buffer is the definition and the
//     region is inside it, and walks the region at either depth;
//   * `lfx::effect` and `LFX_BUNDLE`, which are the entry table written once.
//
// It needs C++17 and nothing else - no exceptions crossing the boundary, no
// runtime, no allocation beyond the one instance a `create` makes. An
// exception must never leave `process` or `describe`: the host is C on the
// other side of the call, and unwinding through it is undefined. Every
// callback below is `noexcept` for that reason.

#ifndef LFX_HPP
#define LFX_HPP

#include "lfx.h"

#include <cstdint>
#include <cstring>
#include <new>

namespace lfx {

// ------------------------------------------------------------ the halves --

/// One half-float as a `float`.
inline float from_half(std::uint16_t bits) noexcept {
    const std::uint32_t sign = static_cast<std::uint32_t>(bits & 0x8000u) << 16;
    std::uint32_t exponent = static_cast<std::uint32_t>((bits >> 10) & 0x1fu);
    std::uint32_t mantissa = static_cast<std::uint32_t>(bits & 0x3ffu);
    std::uint32_t whole = 0u;
    if (exponent == 0u) {
        if (mantissa == 0u) {
            whole = sign;
        } else {
            // A subnormal half is a normal float: shift the leading one into
            // place and pay for it in the exponent.
            std::int32_t shifted = 1;
            while ((mantissa & 0x400u) == 0u) {
                mantissa <<= 1;
                shifted--;
            }
            mantissa &= 0x3ffu;
            whole = sign | (static_cast<std::uint32_t>(shifted + 112) << 23) | (mantissa << 13);
        }
    } else if (exponent == 31u) {
        whole = sign | 0x7f800000u | (mantissa << 13);
    } else {
        whole = sign | ((exponent + 112u) << 23) | (mantissa << 13);
    }
    float out = 0.0f;
    std::memcpy(&out, &whole, sizeof out);
    return out;
}

/// One `float` as a half, rounding to nearest and ties to even - which is what
/// the host does on its own side of the boundary.
inline std::uint16_t to_half(float value) noexcept {
    std::uint32_t whole = 0u;
    std::memcpy(&whole, &value, sizeof whole);
    const std::uint32_t sign = (whole >> 16) & 0x8000u;
    const std::uint32_t biased = (whole >> 23) & 0xffu;
    std::uint32_t mantissa = whole & 0x7fffffu;
    const std::int32_t exponent = static_cast<std::int32_t>(biased) - 127 + 15;

    if (biased == 0xffu) {
        // Infinity keeps its sign; a NaN stays a NaN rather than becoming one.
        return static_cast<std::uint16_t>(sign | 0x7c00u |
                                          (mantissa != 0u ? (0x200u | (mantissa >> 13)) : 0u));
    }
    if (exponent >= 31) {
        return static_cast<std::uint16_t>(sign | 0x7c00u);
    }
    if (exponent <= 0) {
        if (exponent < -10) {
            return static_cast<std::uint16_t>(sign);
        }
        mantissa |= 0x800000u;
        const std::uint32_t shift = static_cast<std::uint32_t>(14 - exponent);
        std::uint32_t narrowed = mantissa >> shift;
        const std::uint32_t remainder = mantissa & ((1u << shift) - 1u);
        const std::uint32_t halfway = 1u << (shift - 1u);
        if (remainder > halfway || (remainder == halfway && (narrowed & 1u) != 0u)) {
            narrowed++;
        }
        return static_cast<std::uint16_t>(sign | narrowed);
    }
    std::uint32_t narrowed =
        (static_cast<std::uint32_t>(exponent) << 10) | (mantissa >> 13);
    const std::uint32_t remainder = mantissa & 0x1fffu;
    if (remainder > 0x1000u || (remainder == 0x1000u && (narrowed & 1u) != 0u)) {
        narrowed++;
    }
    return static_cast<std::uint16_t>(sign | narrowed);
}

// ------------------------------------------------------------ the picture --

/// One scene-linear, premultiplied pixel.
struct rgba {
    float r = 0.0f;
    float g = 0.0f;
    float b = 0.0f;
    float a = 0.0f;
};

/// A rectangle in the space the request's region and the frames' origins are
/// both given in: `x0`/`y0` inclusive, `x1`/`y1` exclusive.
struct rect {
    std::int32_t x0 = 0;
    std::int32_t y0 = 0;
    std::int32_t x1 = 0;
    std::int32_t y1 = 0;

    constexpr std::int32_t width() const noexcept { return x1 - x0; }
    constexpr std::int32_t height() const noexcept { return y1 - y0; }
};

// ------------------------------------------------------------- the values --

/// The dense value array, read the one way the header admits.
///
/// Every getter takes the element's index and the number to answer with when
/// the element is not there or is not of the kind asked for. There is no
/// throwing getter and no default-constructed nonsense: a plugin that cannot
/// find its own control has been handed something it did not declare, and
/// carrying on with the declared default is what the host does too.
class values {
public:
    values() noexcept = default;

    explicit values(const lfx_process &request) noexcept
        : base_(reinterpret_cast<const char *>(request.values)),
          stride_(request.value_stride),
          count_(request.value_count) {}

    std::uint32_t size() const noexcept { return count_; }

    /// The element at `index`, or `nullptr`.
    ///
    /// **Strided, never `sizeof`-ed.** The host may write a wider element than
    /// this build was compiled against; `param` is the element's own index, so
    /// a mismatch is this plugin having walked the array wrongly rather than
    /// the host having filled it in wrongly.
    const lfx_value *at(std::uint32_t index) const noexcept {
        if (base_ == nullptr || stride_ == 0u || index >= count_) {
            return nullptr;
        }
        const auto *element = reinterpret_cast<const lfx_value *>(
            base_ + static_cast<std::size_t>(index) * static_cast<std::size_t>(stride_));
        return element->param == index ? element : nullptr;
    }

    /// A Float, a Slider or an Angle.
    double number(std::uint32_t index, double spare) const noexcept {
        const lfx_value *element = at(index);
        if (element == nullptr || (element->kind != LFX_PARAM_FLOAT &&
                                   element->kind != LFX_PARAM_SLIDER &&
                                   element->kind != LFX_PARAM_ANGLE)) {
            return spare;
        }
        return element->v.f;
    }

    /// An Int or a Seed.
    std::int64_t whole(std::uint32_t index, std::int64_t spare) const noexcept {
        const lfx_value *element = at(index);
        if (element == nullptr ||
            (element->kind != LFX_PARAM_INT && element->kind != LFX_PARAM_SEED)) {
            return spare;
        }
        return element->v.i;
    }

    bool flag(std::uint32_t index, bool spare) const noexcept {
        const lfx_value *element = at(index);
        return element != nullptr && element->kind == LFX_PARAM_BOOL ? element->v.b : spare;
    }

    std::uint32_t chosen(std::uint32_t index, std::uint32_t spare) const noexcept {
        const lfx_value *element = at(index);
        return element != nullptr && element->kind == LFX_PARAM_CHOICE ? element->v.choice
                                                                      : spare;
    }

    rgba colour(std::uint32_t index, rgba spare) const noexcept {
        const lfx_value *element = at(index);
        if (element == nullptr || element->kind != LFX_PARAM_COLOUR) {
            return spare;
        }
        return rgba{element->v.rgba[0], element->v.rgba[1], element->v.rgba[2],
                    element->v.rgba[3]};
    }

    /// **Both** axes of a Point2: one declaration is one element, however many
    /// rows the panel folded it into.
    void point(std::uint32_t index, float &x, float &y) const noexcept {
        const lfx_value *element = at(index);
        if (element == nullptr || element->kind != LFX_PARAM_POINT2) {
            return;
        }
        x = element->v.xy[0];
        y = element->v.xy[1];
    }

private:
    const char *base_ = nullptr;
    std::uint32_t stride_ = 0u;
    std::uint32_t count_ = 0u;
};

// ------------------------------------------------------------ the request --

/// One render request, with the geometry already read the right way round.
class request {
public:
    explicit request(const lfx_process &call) noexcept : call_(&call) {}

    const lfx_process &raw() const noexcept { return *call_; }
    lfx::values values() const noexcept { return lfx::values(*call_); }
    double time() const noexcept { return call_->time; }
    lfx_pixel_format format() const noexcept { return call_->pixel_format; }

    /// The region asked for.
    rect region() const noexcept {
        return rect{call_->roi_x0, call_->roi_y0, call_->roi_x1, call_->roi_y1};
    }

    /// **The buffer**, which is the input's definition and not the region.
    ///
    /// A vignette's centre, a gradient's ramp and anything else that is a
    /// function of *where* a pixel is belongs in this rectangle. Computing it
    /// from the region instead is the tile seam: four quarters of one frame
    /// would each get their own centre.
    rect definition() const noexcept {
        return rect{call_->dod_x0, call_->dod_y0, call_->dod_x1, call_->dod_y1};
    }

    bool cancelled() const noexcept {
        return call_->cancelled != nullptr && call_->cancelled(call_);
    }

    /// Walk the region, handing each pixel to `shade` and writing back what it
    /// answers.
    ///
    /// `shade` is called as `shade(x, y, rgba)` with `x` and `y` in the space
    /// `definition()` is given in, and must not throw. Both depths are handled
    /// here so that the plugin's maths is written once, in `float`, which is
    /// the whole of what "every colour depth is mandatory" costs an author.
    /// Cancellation is polled once a row.
    template <typename Shade>
    lfx_status for_each_pixel(Shade shade) const noexcept {
        const lfx_frame *in = call_->input;
        lfx_frame *out = call_->output;
        if (in == nullptr || out == nullptr || in->data == nullptr || out->data == nullptr) {
            return LFX_STATUS_FAILED;
        }
        if (in->format != call_->pixel_format || out->format != call_->pixel_format) {
            return LFX_STATUS_UNSUPPORTED;
        }
        std::size_t sample_bytes = 0u;
        if (call_->pixel_format == LFX_RGBA_F32) {
            sample_bytes = sizeof(float);
        } else if (call_->pixel_format == LFX_RGBA_F16) {
            sample_bytes = sizeof(std::uint16_t);
        } else {
            return LFX_STATUS_UNSUPPORTED;
        }

        const rect buffer{in->origin_x, in->origin_y,
                          in->origin_x + static_cast<std::int32_t>(in->width),
                          in->origin_y + static_cast<std::int32_t>(in->height)};
        const rect wanted = region();
        const std::int32_t x0 = wanted.x0 > buffer.x0 ? wanted.x0 : buffer.x0;
        const std::int32_t y0 = wanted.y0 > buffer.y0 ? wanted.y0 : buffer.y0;
        const std::int32_t x1 = wanted.x1 < buffer.x1 ? wanted.x1 : buffer.x1;
        const std::int32_t y1 = wanted.y1 < buffer.y1 ? wanted.y1 : buffer.y1;

        for (std::int32_t y = y0; y < y1; y++) {
            if (cancelled()) {
                return LFX_STATUS_CANCELLED;
            }
            const char *source = static_cast<const char *>(in->data) +
                                 static_cast<std::size_t>(y - in->origin_y) * in->row_bytes;
            char *destination = static_cast<char *>(out->data) +
                                static_cast<std::size_t>(y - out->origin_y) * out->row_bytes;
            for (std::int32_t x = x0; x < x1; x++) {
                const std::size_t from =
                    static_cast<std::size_t>(x - in->origin_x) * 4u * sample_bytes;
                const std::size_t to =
                    static_cast<std::size_t>(x - out->origin_x) * 4u * sample_bytes;
                if (call_->pixel_format == LFX_RGBA_F32) {
                    const auto *read = reinterpret_cast<const float *>(source + from);
                    auto *write = reinterpret_cast<float *>(destination + to);
                    const rgba painted = shade(x, y, rgba{read[0], read[1], read[2], read[3]});
                    write[0] = painted.r;
                    write[1] = painted.g;
                    write[2] = painted.b;
                    write[3] = painted.a;
                } else {
                    const auto *read = reinterpret_cast<const std::uint16_t *>(source + from);
                    auto *write = reinterpret_cast<std::uint16_t *>(destination + to);
                    const rgba painted =
                        shade(x, y,
                              rgba{from_half(read[0]), from_half(read[1]), from_half(read[2]),
                                   from_half(read[3])});
                    write[0] = to_half(painted.r);
                    write[1] = to_half(painted.g);
                    write[2] = to_half(painted.b);
                    write[3] = to_half(painted.a);
                }
            }
        }
        return LFX_STATUS_OK;
    }

private:
    const lfx_process *call_;
};

// ----------------------------------------------------------- the describe --

/// The describe sink, one method per kind.
///
/// Each method answers whether the host took the declaration. A `false` is the
/// graceful refusal of that one row - the effect still loads and the row keeps
/// the default declared here - so declaring is worth carrying on with rather
/// than giving up at the first no. Two faults are not that and the answer does
/// not distinguish them: a duplicate id and an unset unit refuse the whole
/// effect, whatever is declared next.
class describe {
public:
    explicit describe(lfx_describe_sink *sink) noexcept : sink_(sink) {}

    bool ready() const noexcept {
        return sink_ != nullptr && sink_->declare_float != nullptr &&
               sink_->declare_slider != nullptr && sink_->declare_int != nullptr &&
               sink_->declare_angle != nullptr && sink_->declare_bool != nullptr &&
               sink_->declare_choice != nullptr && sink_->declare_colour != nullptr &&
               sink_->declare_seed != nullptr && sink_->declare_point2 != nullptr &&
               sink_->declare_point3 != nullptr && sink_->declare_curve != nullptr &&
               sink_->declare_file != nullptr && sink_->declare_action != nullptr &&
               sink_->group_begin != nullptr && sink_->group_end != nullptr;
    }

    /// An unbounded number with a slider's travel, and optional hard bounds.
    bool number(const char *id, const char *label, lfx_unit unit, double value, double low,
                double high, lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_float_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = unit;
        declaration.flags = flags;
        declaration.bounds = LFX_BOUND_NONE;
        declaration.id = id;
        declaration.label = label;
        declaration.default_value = value;
        declaration.slider_min = low;
        declaration.slider_max = high;
        return sink_ != nullptr && sink_->declare_float != nullptr &&
               sink_->declare_float(sink_, &declaration);
    }

    /// A bounded number: the range is the control's whole nature.
    bool slider(const char *id, const char *label, lfx_unit unit, double value, double low,
                double high, bool logarithmic = false,
                lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_slider_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = unit;
        declaration.flags = flags;
        declaration.log = logarithmic ? 1u : 0u;
        declaration.id = id;
        declaration.label = label;
        declaration.default_value = value;
        declaration.range_min = low;
        declaration.range_max = high;
        return sink_ != nullptr && sink_->declare_slider != nullptr &&
               sink_->declare_slider(sink_, &declaration);
    }

    bool whole(const char *id, const char *label, lfx_unit unit, std::int64_t value,
               std::int64_t low, std::int64_t high,
               lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_int_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = unit;
        declaration.flags = flags;
        declaration.bounds = LFX_BOUND_NONE;
        declaration.id = id;
        declaration.label = label;
        declaration.default_value = value;
        declaration.slider_min = low;
        declaration.slider_max = high;
        return sink_ != nullptr && sink_->declare_int != nullptr &&
               sink_->declare_int(sink_, &declaration);
    }

    /// An angle, in degrees because an angle is degrees by definition, and
    /// deliberately unbounded: it animates through full turns.
    bool angle(const char *id, const char *label, double degrees, double step = 15.0,
               lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_angle_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = LFX_UNIT_DEGREES;
        declaration.flags = flags;
        declaration.reserved_0 = 0u;
        declaration.id = id;
        declaration.label = label;
        declaration.default_value = degrees;
        declaration.dial_step = step;
        return sink_ != nullptr && sink_->declare_angle != nullptr &&
               sink_->declare_angle(sink_, &declaration);
    }

    bool flag(const char *id, const char *label, bool value,
              lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_bool_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = LFX_UNIT_RAW;
        declaration.flags = flags;
        declaration.default_value = value ? 1u : 0u;
        declaration.id = id;
        declaration.label = label;
        return sink_ != nullptr && sink_->declare_bool != nullptr &&
               sink_->declare_bool(sink_, &declaration);
    }

    /// A dropdown. The dividers are declared rather than guessed from the
    /// labels: each entry is the index after which the list draws a rule.
    bool choice(const char *id, const char *label, std::uint32_t value,
                const char *const *options, std::uint32_t option_count,
                const std::uint32_t *dividers_after = nullptr,
                std::uint32_t divider_count = 0u,
                lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_choice_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = LFX_UNIT_RAW;
        declaration.flags = flags;
        declaration.default_index = value;
        declaration.option_count = option_count;
        declaration.divider_count = divider_count;
        declaration.id = id;
        declaration.label = label;
        declaration.options = options;
        declaration.dividers_after = dividers_after;
        return sink_ != nullptr && sink_->declare_choice != nullptr &&
               sink_->declare_choice(sink_, &declaration);
    }

    bool colour(const char *id, const char *label, rgba value, double low = 0.0,
                double high = 1.0, lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_colour_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = LFX_UNIT_RAW;
        declaration.flags = flags;
        declaration.reserved_0 = 0u;
        declaration.id = id;
        declaration.label = label;
        declaration.default_rgba[0] = value.r;
        declaration.default_rgba[1] = value.g;
        declaration.default_rgba[2] = value.b;
        declaration.default_rgba[3] = value.a;
        declaration.range_min = low;
        declaration.range_max = high;
        return sink_ != nullptr && sink_->declare_colour != nullptr &&
               sink_->declare_colour(sink_, &declaration);
    }

    /// Two rows the panel folds back into one crosshair, and one element of
    /// the value array.
    bool point(const char *id, const char *label, lfx_unit unit, double x, double y,
               double low, double high,
               lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_point2_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = unit;
        declaration.flags = flags;
        declaration.reserved_0 = 0u;
        declaration.id = id;
        declaration.label = label;
        declaration.default_x = x;
        declaration.default_y = y;
        declaration.slider_min = low;
        declaration.slider_max = high;
        return sink_ != nullptr && sink_->declare_point2 != nullptr &&
               sink_->declare_point2(sink_, &declaration);
    }

    /// The randomness a seeded effect follows. There is deliberately no
    /// default: the host draws one from the fresh instance's own id, so two
    /// copies never wobble in sync.
    bool seed(const char *id, const char *label,
              lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_seed_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.unit = LFX_UNIT_RAW;
        declaration.flags = flags;
        declaration.reserved_0 = 0u;
        declaration.id = id;
        declaration.label = label;
        return sink_ != nullptr && sink_->declare_seed != nullptr &&
               sink_->declare_seed(sink_, &declaration);
    }

    bool group_begin(const char *id, const char *label,
                     lfx_param_flags flags = LFX_PARAM_FLAG_NONE) noexcept {
        lfx_group_param declaration = {};
        declaration.struct_size = static_cast<std::uint32_t>(sizeof declaration);
        declaration.flags = flags;
        declaration.id = id;
        declaration.label = label;
        return sink_ != nullptr && sink_->group_begin != nullptr &&
               sink_->group_begin(sink_, &declaration);
    }

    bool group_end() noexcept {
        return sink_ != nullptr && sink_->group_end != nullptr && sink_->group_end(sink_);
    }

private:
    lfx_describe_sink *sink_;
};

// ------------------------------------------------------------- the bundle --

/// One live effect, written by the author. The host makes one per in-flight
/// frame and never re-enters one; `process` may still run on any worker
/// thread, and two instances of one effect may be inside it at once.
class effect {
public:
    virtual ~effect() = default;

    /// Declare every control, in the order they should be drawn.
    /// **Control thread.**
    virtual bool describe(lfx::describe &sink) noexcept = 0;

    /// Render one frame. **Any worker thread.**
    virtual lfx_status process(const lfx::request &call) noexcept = 0;

    /// This effect's answer to the extension handshake. Offering none is the
    /// honest default.
    virtual const void *extension(const char *id, std::uint32_t version) noexcept {
        (void)id;
        (void)version;
        return nullptr;
    }

    /// The host pointer handed to `create`, valid until the bundle is put
    /// away. It is the only pointer in the ABI a plugin may keep.
    const lfx_host *host() const noexcept { return host_; }

    void set_host(const lfx_host *host) noexcept { host_ = host; }

private:
    const lfx_host *host_ = nullptr;
};

/// One entry in a bundle's table: what the effect is, and how to make one.
struct registration {
    const lfx_descriptor *descriptor;
    effect *(*create)(const lfx_host *host);
};

/// Make one `T`, or `nullptr` - which is the ABI's own spelling of "this
/// instance could not be made".
template <typename T>
effect *make(const lfx_host *host) noexcept {
    auto *made = new (std::nothrow) T();
    if (made != nullptr) {
        made->set_host(host);
    }
    return made;
}

namespace detail {

/// Defined by `LFX_BUNDLE` in the plugin's own translation unit.
const registration *table() noexcept;
std::uint32_t table_size() noexcept;

/// The C table in front of one C++ instance. The `lfx_plugin` is first so
/// that the host's pointer is this object's, and `owned` is what `destroy`
/// deletes.
struct held {
    lfx_plugin table;
    effect *owned;
};

inline held *holder(lfx_plugin *plugin) noexcept {
    return plugin == nullptr ? nullptr : static_cast<held *>(plugin->plugin_data);
}

inline std::uint32_t plugin_init(lfx_plugin *plugin) noexcept {
    return holder(plugin) != nullptr ? 1u : 0u;
}

inline void plugin_destroy(lfx_plugin *plugin) noexcept {
    held *self = holder(plugin);
    if (self != nullptr) {
        delete self->owned;
        delete self;
    }
}

inline std::uint32_t plugin_describe(lfx_plugin *plugin, lfx_describe_sink *sink) noexcept {
    held *self = holder(plugin);
    if (self == nullptr || self->owned == nullptr) {
        return 0u;
    }
    lfx::describe declaring(sink);
    return declaring.ready() && self->owned->describe(declaring) ? 1u : 0u;
}

inline std::int32_t plugin_process(lfx_plugin *plugin, const lfx_process *call) noexcept {
    held *self = holder(plugin);
    if (self == nullptr || self->owned == nullptr || call == nullptr) {
        return LFX_STATUS_FAILED;
    }
    const lfx::request wrapped(*call);
    return self->owned->process(wrapped);
}

inline const void *plugin_extension(lfx_plugin *plugin, const char *id,
                                    std::uint32_t version) noexcept {
    held *self = holder(plugin);
    return self == nullptr || self->owned == nullptr ? nullptr
                                                     : self->owned->extension(id, version);
}

inline std::uint32_t entry_init(const char *bundle_path) noexcept {
    (void)bundle_path;
    return 1u;
}

inline void entry_deinit() noexcept {}

inline std::uint32_t entry_count() noexcept { return table_size(); }

inline const lfx_descriptor *entry_descriptor(std::uint32_t index) noexcept {
    return index < table_size() ? table()[index].descriptor : nullptr;
}

inline lfx_plugin *entry_create(const lfx_host *host, const char *id) noexcept {
    if (id == nullptr) {
        return nullptr;
    }
    for (std::uint32_t index = 0u; index < table_size(); index++) {
        const registration &row = table()[index];
        if (row.descriptor == nullptr || row.descriptor->id == nullptr ||
            std::strcmp(row.descriptor->id, id) != 0) {
            continue;
        }
        effect *made = row.create(host);
        if (made == nullptr) {
            return nullptr;
        }
        auto *self = new (std::nothrow) held();
        if (self == nullptr) {
            delete made;
            return nullptr;
        }
        self->owned = made;
        self->table.struct_size = static_cast<std::uint32_t>(sizeof(lfx_plugin));
        self->table.plugin_data = self;
        self->table.init = plugin_init;
        self->table.destroy = plugin_destroy;
        self->table.describe = plugin_describe;
        self->table.process = plugin_process;
        self->table.get_extension = plugin_extension;
        return &self->table;
    }
    return nullptr;
}

} // namespace detail

} // namespace lfx

/// Declare the bundle's entry table from an array of `lfx::registration`.
///
/// It goes at file scope, outside any namespace, once per bundle. MSVC exports
/// a symbol from a DLL only when it is asked to, and the frozen header declares
/// `lfx_entry_point` without an export attribute - so the ask goes on the link
/// line, where it is not an inconsistent-linkage error.
#if defined(_MSC_VER)
#define LFX_BUNDLE_EXPORT __pragma(comment(linker, "/EXPORT:lfx_entry_point,DATA"))
#else
#define LFX_BUNDLE_EXPORT
#endif

#define LFX_BUNDLE(TABLE)                                                                  \
    namespace lfx {                                                                        \
    namespace detail {                                                                     \
    const registration *table() noexcept { return (TABLE); }                               \
    std::uint32_t table_size() noexcept {                                                  \
        return static_cast<std::uint32_t>(sizeof(TABLE) / sizeof((TABLE)[0]));             \
    }                                                                                      \
    }                                                                                      \
    }                                                                                      \
    LFX_BUNDLE_EXPORT                                                                      \
    extern "C" const lfx_entry lfx_entry_point = {                                         \
        static_cast<std::uint32_t>(sizeof(lfx_entry)),                                     \
        LFX_ABI_VERSION,                                                                   \
        ::lfx::detail::entry_init,                                                         \
        ::lfx::detail::entry_deinit,                                                       \
        ::lfx::detail::entry_count,                                                        \
        ::lfx::detail::entry_descriptor,                                                   \
        ::lfx::detail::entry_create}

#endif // LFX_HPP
