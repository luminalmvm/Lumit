// Depth-of-field lens blur (docs/08-EFFECTS.md §3.22). A variable-radius
// "scatter-as-gather" blur: each output pixel's circle-of-confusion radius comes
// from how far its depth is from the focus plane, and it averages an aperture of
// that radius from the source. Mirrors the CPU reference tap-for-tap
// (`lumit_core::fx::cpu::dof`; §1.6: the CPU is the oracle) — the same CoC
// maths, the same integer taps in the same row-major order, the same edge
// policy.
//
// **In plain terms.** Open a hole the size of the blur around each pixel and
// average what you can see through it. Two things make that read as a lens
// rather than as a smudge, and both are off by default:
//
//   * the hole is a **polygon** — a real iris has blades, and a defocused
//     highlight is a picture of the hole, which is why bokeh balls are hexagons
//     on some lenses and circles on others;
//   * the average is a **power mean** — a flat average dissolves a small bright
//     thing into its dark surroundings, and raising each tap to a power first is
//     what lets it survive and bloom into a ball instead.
//
// **Neutral means bit-identical, and that is done by branching** (K-313).
// Roundness 1 takes the plain `r² ≤ coc²` circle test, Concentration 0 and
// Remove edge leak 0 take the unweighted accumulation, Exposure 0 takes the
// unsplit one. None of these multiplies by one and hopes: `Σ(c·w)/Σw` is not an
// identity in IEEE 754 even when every `w` is 1, `min(c,t) + max(c−t,0)` is not
// reliably `c`, and scaling both sides of a comparison by `apothem2` can flip a
// boundary tap. At their defaults the three branches leave exactly the
// box-weighted disc average this kernel has always computed, which is what let
// the aperture fold into the shipped effect rather than arrive beside it.
//
// The per-pixel depth is read from the channel the effect names (docs/impl/
// layer-input.md §3, `channel_of` — Red by default, the channel it always read).
// In production the depth is the referenced depth layer rendered alone in the
// working format (rgba16float); in the §1.6 oracle it is an exact R32Float map,
// whose red is the same red. Convention: 0 = near, 1 = far, though the effect is
// symmetric about Focus so either reading of the pass works. `depth` is the same
// size as the source, so the load at `xy` is that pixel's depth. Binding 0 is
// the source (the taps sample it), binding 1 the unprocessed original read back
// for the host Mix, binding 2 the depth field — the shared three-sampled-input
// shape it borrows from Motion blur. Any float texture binds (the layout is a
// non-filterable float sample, `textureLoad` not a sampler).

// The most aperture blades the polygon test carries. Mirrors
// `lumit_core::fx::MAX_BLADES` and `lumit_gpu::fx::MAX_BLADES`, pinned by a test
// there.
const MAX_BLADES: u32 = 8u;

struct Params {
    focus: f32,          // in-focus depth in [0,1], when use_focus_point is 0
    range: f32,          // half-width of the sharp band, [0,1]
    near_aperture: f32,  // near-side (d < focus) max CoC radius, raster px
    far_aperture: f32,   // far-side (d >= focus) max CoC radius, raster px
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    apothem2: f32,       // cos²(π/N)
    roundness: f32,      // -1 star … 0 polygon … 1 circle (the default)
    rim: f32,  // -1 centre-weighted … 0 flat … 1 rim-weighted
    aspect_x: f32,       // tap-offset multipliers, both >= 1, one == 1
    aspect_y: f32,
    threshold: f32,      // linear level each tap is split at
    bokeh_power: f32,    // 2^(Exposure/12); 1 = the plain arithmetic mean
    focus_x: f32,        // where to read focus depth, raster px
    focus_y: f32,
    gamma: f32,  // multiplier on the depth distance before the ramp
    remove_edge_leak: f32,
    detect_edge_threshold: f32,
    depth_invert: u32,   // 1 = d' = 1 - d before the CoC
    display: u32,        // 0 = Rendered, 1 = Depth map, 2 = Focus map
    blade_count: u32,    // 3..=MAX_BLADES; Roundness 1 is the circle
    depth_bound: u32,    // 0 = no depth layer: defocus uniformly
    depth_channel: u32,  // index into lumit_core::fx::CHANNEL_OPTIONS
    use_focus_point: u32,// 1 = focus is the depth under (focus_x, focus_y)
    repeat_edge: u32,    // 1 = clamp the gather to the frame edge
    weighted: u32,       // 1 = the tap-weighted path (host decides once)
    tonal: u32,          // 1 = the split-at-threshold power mean
    circle: u32,         // 1 = the plain r² <= coc² test (Roundness 1, no squeeze)
    _pad0: u32,
    // The seventeen floats and ten u32s above come to 27 words, which is not a
    // whole number of 16-byte rows — so one pad word takes it to 28, and the
    // array below lands 16-byte aligned
    // under both WGSL's rules and `repr(C)`'s. That is **load-bearing**: an
    // `array<vec4<f32>, N>` is 16-byte aligned in WGSL, so adding one scalar
    // above without restoring the count to a multiple of four moves the shader's
    // idea of this offset and not the host's, and every normal is then read from
    // the wrong place (measured at 17 920 fp16 ULP when it happened).
    // One normal per vec4 (only .xy read); a uniform array's element stride is
    // 16 bytes whatever the element type, so packing would save nothing.
    blade_normals: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<uniform> p: Params;

// One channel of the depth picture, by the shared CHANNEL_OPTIONS index.
// Mirrors `lumit_core::fx::cpu::channel_of` operation for operation.
fn channel_of(c: vec4<f32>) -> f32 {
    switch (p.depth_channel) {
        case 1u: { return c.a; }
        case 2u: { return c.r; }
        case 3u: { return c.g; }
        case 4u: { return c.b; }
        // 0 and anything unknown: Rec.709 luminance.
        default: { return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b; }
    }
}

fn depth_at(xy: vec2<i32>) -> f32 {
    let d = channel_of(textureLoad(depth, xy, 0));
    return select(d, 1.0 - d, p.depth_invert != 0u);
}

// The smoothstep focus falloff s in [0,1]: 0 inside the sharp band
// |depth-focus| <= range, ramping to 1 as the depth distance reaches the far
// extreme, SCALED BY THE PROFILE CONTROL first. Shared by the CoC radius and the
// Focus-map view. Written with explicit min/max/mul/sub — NOT the built-in
// smoothstep, whose exact form is not guaranteed to match the CPU — so the
// oracle reproduces it bit-for-bit.
//
// The Profile scale is what stops focus being all-or-nothing. Without it the
// ramp reaches full blur only at a depth distance of the whole range, and a real
// depth pass puts nearly all its content in a narrow band with one near object
// well outside it — so focusing anywhere leaves the scene almost sharp and that
// object almost fully blurred, with nothing in between. The host computes the
// multiplier (one `exp2`, off the per-pixel path), and its neutral is exactly 1,
// a multiply that is exact, so the historical ramp is preserved to the bit.
fn coc_falloff(d: f32, focus: f32) -> f32 {
    let dist = abs(d - focus);
    let denom = max(1.0 - p.range, 1e-4);
    let e = min(max(((dist - p.range) / denom) * p.gamma, 0.0), 1.0);
    return e * e * (3.0 - 2.0 * e); // smoothstep ramp
}

// Focus is either the number or whatever depth sits under the point — the reason
// Focus distance greys out in the panel. Clamped rather than wrapped: a point
// dragged off the frame focuses on the nearest edge. Read once per pixel by the
// entry point and passed down, so the extra depth sample is not paid twice.
fn focus_depth() -> f32 {
    if (p.depth_bound == 0u || p.use_focus_point == 0u) {
        return p.focus;
    }
    let size = vec2<i32>(textureDimensions(depth));
    let fx = clamp(i32(floor(p.focus_x)), 0, size.x - 1);
    let fy = clamp(i32(floor(p.focus_y)), 0, size.y - 1);
    return depth_at(vec2<i32>(fx, fy));
}

// Circle-of-confusion radius (raster px) for a depth sample: the falloff scaled
// by the per-side aperture. The near side (d < focus) uses `near_aperture`, the
// far side `far_aperture`; at d == focus the falloff is 0, so the aperture
// select never introduces a discontinuity and the §1.6 oracle holds.
fn coc_radius(d: f32, focus: f32) -> f32 {
    let s = coc_falloff(d, focus);
    let ap = select(p.far_aperture, p.near_aperture, d < focus);
    return ap * s;
}

// The tap's deformed r² when it is inside the aperture, else -1.
//
// **Roundness 1 with no Deform is the plain circle, and takes the plain test.**
// That is the back-compatibility guarantee, not an optimisation: the polygon
// form multiplies both sides of the comparison by `apothem2`, and scaling both
// sides of a floating-point comparison by the same positive constant can change
// its answer on a boundary tap.
//
// Below that: **Roundness reaches below zero.** Positive bows the blades outward
// toward the circle; negative is a star — at a vertex the tap has m = k·r, so
// the two terms cancel to r ≤ coc whatever the coefficient (the vertices stay
// exactly on the circle), while at an edge midpoint m = r, so a negative
// coefficient pulls the midpoint in. No new maths, no branch. **Deform squeezes
// one axis only:** the multipliers are always ≥ 1 and exactly one is > 1, so
// multiplying the tap offset before the inside test can only shrink the aperture
// on that axis. The region therefore stays INSCRIBED in the circle at every
// setting, which is what keeps `ceil(radius)` a correct bound on the taps and
// the effect's declared ROI honest.
fn aperture_r2(dxi: i32, dyi: i32, coc2: f32) -> f32 {
    if (p.circle != 0u) {
        let r2 = f32(dxi * dxi + dyi * dyi);
        return select(-1.0, r2, r2 <= coc2);
    }
    let ax = f32(dxi) * p.aspect_x;
    let ay = f32(dyi) * p.aspect_y;
    let r2 = ax * ax + ay * ay;
    var m = 0.0;
    for (var k = 0u; k < MAX_BLADES; k++) {
        if (k >= p.blade_count) {
            break;
        }
        let n = p.blade_normals[k];
        m = max(m, ax * n.x + ay * n.y);
    }
    let c = p.roundness;
    let inside = (1.0 - c) * m * m + c * p.apothem2 * r2 <= p.apothem2 * coc2;
    return select(-1.0, r2, inside);
}

// One tap's radial weight (Concentration). Multiplicative in coc2 so there is no
// division and no guard at coc = 0; the weights are only used as a ratio, so the
// common factor cancels in the mean.
fn tap_weight(r2: f32, coc2: f32) -> f32 {
    return max(coc2 + p.rim * (2.0 * r2 - coc2), 0.0);
}

@compute @workgroup_size(8, 8)
fn dof(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }

    let d = select(0.0, depth_at(xy), p.depth_bound != 0u);
    let focus = focus_depth();

    // Diagnostic views (both continuous, so the §1.6 oracle covers them; they
    // ignore the gather and Mix and write the view directly). The host forces
    // Rendered when no depth is bound, so these never draw the stand-in texture
    // that occupies the depth slot.
    if (p.display == 1u) {
        // Depth map: the post-invert depth as opaque greyscale — after the
        // channel pick, so it is what the effect is actually reading.
        textureStore(dst, xy, vec4<f32>(d, d, d, 1.0));
        return;
    }
    if (p.display == 2u) {
        // Focus map: 1 - s, white where sharp, darkening out of focus.
        let m = 1.0 - coc_falloff(d, focus);
        textureStore(dst, xy, vec4<f32>(m, m, m, 1.0));
        return;
    }

    // With no depth bound the frame defocuses uniformly at the far-side radius,
    // which is what makes the effect usable as a plain aperture blur.
    let coc = select(p.far_aperture, coc_radius(d, focus), p.depth_bound != 0u);
    let o = textureLoad(orig, xy, 0);

    // In focus: the aperture is a point, so the pixel keeps itself — no gather,
    // no composite, no Mix. Also the only way a sharp pixel stays bit-exact
    // under a weighted gather (a single weighted tap computes (c·w)/w).
    if (coc <= 0.0) {
        textureStore(dst, xy, o);
        return;
    }

    let coc2 = coc * coc;
    // Integer aperture radius: every tap whose deformed squared distance is
    // inside the region is included. The centre (r²=0 <= coc²>=0) is always in,
    // so the running weight is never zero.
    let ri = i32(ceil(coc));
    let t = vec4<f32>(p.threshold);
    let weighted = p.weighted != 0u;
    let tonal = p.tonal != 0u;

    // **Pass one: the brightest excess in the aperture, per channel** — and only
    // when the tonal split is on at all.
    //
    // The power mean cannot be computed as (Σ c^p / n)^(1/p) in f32. At a high
    // Exposure a channel at scene-linear 0.08 raises to 8e-36 and one at 0.05 to
    // 2e-42, below the smallest normal. Averaging those and rooting them back
    // yields zero, so every channel below roughly 0.116 linear collapses to
    // black, per channel independently — black holes and saturated speckle
    // rather than a blur. A floor on the *mean* cannot save it; the underflow
    // has already happened in the taps.
    //
    // Factoring the largest excess M out first is the standard fix and an exact
    // identity:
    //
    //     (Σ w·c^p / Σw)^(1/p)  =  M · (Σ w·(c/M)^p / Σw)^(1/p)
    //
    // Every c/M is then in [0, 1], the brightest tap contributes exactly 1, and
    // the mean is bounded below by that tap's share of the weight — nothing
    // underflows and no floor is needed. It costs a second walk of the aperture,
    // which is why this is two loops — and why an untonal gather skips it.
    var peak = vec4<f32>(0.0);
    if (tonal) {
        for (var dy = -ri; dy <= ri; dy++) {
            for (var dx = -ri; dx <= ri; dx++) {
                if (aperture_r2(dx, dy, coc2) < 0.0) {
                    continue;
                }
                let ox = xy.x + dx;
                let oy = xy.y + dy;
                var sx = ox;
                var sy = oy;
                if (p.repeat_edge != 0u) {
                    sx = clamp(ox, 0, size.x - 1);
                    sy = clamp(oy, 0, size.y - 1);
                } else if (ox < 0 || oy < 0 || ox >= size.x || oy >= size.y) {
                    continue;
                }
                let c = textureLoad(src, vec2<i32>(sx, sy), 0);
                peak = max(peak, max(c - t, vec4<f32>(0.0)));
            }
        }
    }

    // Pass two: the gather proper.
    var acc_lo = vec4<f32>(0.0);
    var acc_hi = vec4<f32>(0.0);
    var n = 0.0;

    for (var dy = -ri; dy <= ri; dy++) {
        for (var dx = -ri; dx <= ri; dx++) {
            let r2 = aperture_r2(dx, dy, coc2);
            if (r2 < 0.0) {
                continue;
            }
            var w = select(1.0, tap_weight(r2, coc2), weighted);

            let ox = xy.x + dx;
            let oy = xy.y + dy;
            var sx = ox;
            var sy = oy;
            if (p.repeat_edge != 0u) {
                sx = clamp(ox, 0, size.x - 1);
                sy = clamp(oy, 0, size.y - 1);
            } else if (ox < 0 || oy < 0 || ox >= size.x || oy >= size.y) {
                // Transparent contributes nothing AND keeps its weight, so a
                // gather running off the frame darkens toward the edge rather
                // than brightening — the reading the blur family already gives.
                n += w;
                continue;
            }

            // Edge leak: a tap across a depth discontinuity and in FRONT of this
            // pixel is sharp foreground colour bleeding into a defocused
            // background. Pull it back rather than drop it, so the suppression
            // is continuous in the slider.
            if (weighted && p.remove_edge_leak > 0.0 && p.depth_bound != 0u) {
                let dt = depth_at(vec2<i32>(sx, sy));
                if (abs(dt - d) > p.detect_edge_threshold && dt < d) {
                    w *= 1.0 - p.remove_edge_leak;
                }
            }

            let c = textureLoad(src, vec2<i32>(sx, sy), 0);
            if (tonal) {
                acc_lo += min(c, t) * w;
                // Normalised by the brightest excess, so the ratio is in [0, 1]
                // and its power cannot underflow (see pass one). A zero peak
                // means nothing in the aperture is above the threshold; the
                // excess term is then zero and the plain average is the whole
                // answer.
                let excess = max(c - t, vec4<f32>(0.0));
                let ratio = select(
                    vec4<f32>(0.0),
                    excess / max(peak, vec4<f32>(1e-30)),
                    peak > vec4<f32>(0.0),
                );
                acc_hi += pow(ratio, vec4<f32>(p.bokeh_power)) * w;
            } else {
                // The historical accumulation, unchanged: one sum, no split,
                // and with w a literal 1 on the unweighted path the multiply
                // is exact.
                acc_lo += c * w;
            }
            n += w;
        }
    }
    if (n <= 0.0) {
        textureStore(dst, xy, o);
        return;
    }

    // M · (mean of the normalised powers)^(1/p) — the identity pass one factored
    // out, put back together. No floor: the brightest tap contributes exactly 1
    // to the sum, so the mean is at least its share of the weight.
    var rooted = vec4<f32>(0.0);
    if (tonal) {
        rooted = select(
            vec4<f32>(0.0),
            peak * pow(acc_hi / n, vec4<f32>(1.0 / p.bokeh_power)),
            peak > vec4<f32>(0.0),
        );
    }
    let v = acc_lo / n + rooted;

    // The defocused result replaces the original, blended by Mix. There is no
    // composite menu: an effect that wants its balls added over a sharp plate is
    // an adjustment layer with a blend mode, which already exists.
    textureStore(dst, xy, mix(o, v, p.mix_amt));
}
