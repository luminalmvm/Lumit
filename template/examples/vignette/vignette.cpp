// vignette.cpp - an LFX plugin in C++, through `cpp/lfx.hpp`.
//
// SPDX-License-Identifier: MIT
// Copyright (c) 2026 The Lumit authors.
//
// In plain terms
//
// It darkens the picture away from a centre. The effect itself is four lines;
// what it is here to show is the one thing a per-pixel effect gets wrong more
// often than the maths.
//
// **The geometry comes from the definition, never from the region.** The host
// may ask for any rectangle of the frame - one call for the whole picture, or
// four calls for its quarters, or one small region because that is all the
// Viewer can see. A vignette whose centre is worked out from the region asked
// for would put a different centre in every tile, and the seams would only
// show on the machines that tiled. `request::definition()` is the buffer, and
// the buffer is the picture; `request::region()` is the part of it wanted this
// time.
//
// The second thing is the declaration that lets the host tile at all:
// `LFX_ROI_EXACT` says this effect reads the output pixel and nothing around
// it. It is true here, and a kernel that reached further while saying it would
// produce exactly the seams above.

#include "lfx.hpp"

#include <cmath>

namespace {

// The order these are declared in is the order the panel draws them and the
// order the value array arrives in. A point is **one** element however many
// rows the panel folds it into, and a group takes none at all.
enum : std::uint32_t {
    value_centre = 0u,
    value_radius = 1u,
    value_softness = 2u,
    value_invert = 3u
};

// What the host schedules from. Every zero in a trait block is a declaration
// of the pessimistic case, so it is filled in deliberately rather than left to
// a memset.
const lfx_traits traits = {
    static_cast<std::uint32_t>(sizeof(lfx_traits)),
    LFX_COST_CHEAP,
    LFX_ROI_EXACT,
    0.0f,
    0, // no neighbouring frames are read …
    0, // … so the temporal window is this frame alone
    LFX_ALPHA_PREMULTIPLIED,
    LFX_TRAIT_CANCELLABLE,
    0u // no scratch: this effect writes straight into the output
};

// The first category is the heading the effect is browsed under; the rest are
// search keywords.
const lfx_category categories[] = {LFX_CATEGORY_STYLISE, LFX_CATEGORY_UTILITY};

const lfx_descriptor descriptor = {
    static_cast<std::uint32_t>(sizeof(lfx_descriptor)),
    "com.example.lfx.vignette",
    "Vignette",
    "Example",
    1u,
    0u,
    0u,
    categories,
    static_cast<std::uint32_t>(sizeof categories / sizeof categories[0]),
    &traits,
    nullptr,
    0u};

/// The falloff at a distance, given where the full-strength circle ends and
/// how wide the fade is. Smooth at both ends, so a still frame has no ring in
/// it and a moving one has no step.
float falloff(float distance, float radius, float fade) noexcept {
    if (!(fade > 0.0f)) {
        return distance <= radius ? 1.0f : 0.0f;
    }
    float t = (distance - radius) / fade;
    if (t <= 0.0f) {
        return 1.0f;
    }
    if (t >= 1.0f) {
        return 0.0f;
    }
    return 1.0f - (t * t * (3.0f - 2.0f * t));
}

class vignette final : public lfx::effect {
public:
    bool describe(lfx::describe &sink) noexcept override {
        // Per cent of the frame rather than pixels, so the same declared
        // default means the same picture whatever size the composition is.
        sink.point("centre", "Centre", LFX_UNIT_PERCENT, 50.0, 50.0, 0.0, 100.0);
        sink.slider("radius", "Radius", LFX_UNIT_PERCENT, 70.0, 0.0, 200.0);
        sink.slider("softness", "Softness", LFX_UNIT_PERCENT, 40.0, 0.0, 100.0);
        sink.flag("invert", "Invert", false);
        return true;
    }

    lfx_status process(const lfx::request &call) noexcept override {
        const lfx::values values = call.values();
        const lfx::rect picture = call.definition();
        const float width = static_cast<float>(picture.width());
        const float height = static_cast<float>(picture.height());
        if (!(width > 0.0f) || !(height > 0.0f)) {
            return LFX_STATUS_OK;
        }

        float centre_x = 50.0f;
        float centre_y = 50.0f;
        values.point(value_centre, centre_x, centre_y);
        const float cx = static_cast<float>(picture.x0) + width * centre_x * 0.01f;
        const float cy = static_cast<float>(picture.y0) + height * centre_y * 0.01f;

        // Everything is scaled by half the frame's diagonal, so the control
        // reads the same on a square frame and a wide one.
        const float reach = 0.5f * std::sqrt(width * width + height * height);
        const float radius =
            static_cast<float>(values.number(value_radius, 70.0)) * 0.01f * reach;
        const float fade =
            static_cast<float>(values.number(value_softness, 40.0)) * 0.01f * reach;
        const bool inverted = values.flag(value_invert, false);

        return call.for_each_pixel(
            [&](std::int32_t x, std::int32_t y, lfx::rgba pixel) noexcept -> lfx::rgba {
                const float dx = (static_cast<float>(x) + 0.5f) - cx;
                const float dy = (static_cast<float>(y) + 0.5f) - cy;
                float shade = falloff(std::sqrt(dx * dx + dy * dy), radius, fade);
                if (inverted) {
                    shade = 1.0f - shade;
                }
                // Every channel, alpha included: the frames are premultiplied,
                // so a picture scaled on three channels and not the fourth is
                // no longer one.
                return lfx::rgba{pixel.r * shade, pixel.g * shade, pixel.b * shade,
                                 pixel.a * shade};
            });
    }
};

const lfx::registration bundle[] = {{&descriptor, &lfx::make<vignette>}};

} // namespace

LFX_BUNDLE(bundle);
