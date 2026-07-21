// Layer compositing (docs/06-RENDER-PIPELINE.md render order, evaluator v0).
//
// Each layer draws as a textured quad. The vertex transform is a full 4×4
// (decision K-023: 4×4 from day one, so 3D bolts on without a rewrite).
// Blending is premultiplied-over in LINEAR light — the whole reason the
// working format exists: light adds correctly here.

struct LayerUniform {
    // comp pixel space → NDC, including the layer's transform.
    matrix: mat4x4<f32>,
    // x: opacity 0..1 · y: use_matte · z: matte luma (else alpha) · w: invert
    params: vec4<f32>,
    // xy: comp target size in pixels (normalises frag position to matte uv)
    // z: snapshot-blend selector (composite.rs Blend::snapshot_mode): 0 screen ·
    //    1 overlay · 2 soft light · 3 hard light · 4 lighten · 5 darken ·
    //    6 subtract · 7 colour burn · 8 linear burn · 9 darker colour ·
    //    10 colour dodge · 11 lighter colour · 12 vivid light · 13 linear light ·
    //    14 pin light · 15 hard mix · 16 difference · 17 exclusion · 18 divide ·
    //    19 hue · 20 saturation · 21 colour · 22 luminosity
    target_size: vec4<f32>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> layer: LayerUniform;
// Comp-space matte (a rendered layer); 1×1 white when unused.
@group(0) @binding(3) var matte: texture_2d<f32>;
// Snapshot of the accumulated comp so far (shader-computed blends read the
// destination themselves and write with blending off); 1×1 black when unused.
@group(0) @binding(4) var dst_snapshot: texture_2d<f32>;
// Layer-space mask coverage in the alpha channel (GPU-sourced layers such as
// Precomps, whose pixels never exist CPU-side); 1×1 white when unused.
@group(0) @binding(5) var layer_mask: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_layer(@builtin(vertex_index) i: u32) -> VsOut {
    // Unit quad 0..1 (two triangles, 6 vertices).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[i];
    var out: VsOut;
    out.pos = layer.matrix * vec4<f32>(c, 0.0, 1.0);
    out.uv = c;
    return out;
}

fn srgb_encode_c(v: vec3<f32>) -> vec3<f32> {
    let lo = v * 12.92;
    let hi = 1.055 * pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, v <= vec3<f32>(0.0031308));
}

fn srgb_decode_c(v: vec3<f32>) -> vec3<f32> {
    let lo = v / 12.92;
    let hi = pow((v + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, v <= vec3<f32>(0.04045));
}

// W3C soft-light D(d) helper.
fn soft_light_d(d: vec3<f32>) -> vec3<f32> {
    let poly = ((16.0 * d - 12.0) * d + 4.0) * d;
    return select(sqrt(d), poly, d <= vec3<f32>(0.25));
}

// --- Separable blend formulas (per channel, W3C/PDF; s = source, d = dst). ---

fn f_screen(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    return one - (one - s) * (one - d);
}
fn f_hard_light(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    return select(one - 2.0 * (one - s) * (one - d), 2.0 * s * d, s <= vec3<f32>(0.5));
}
fn f_overlay(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return f_hard_light(d, s); // hard light with src/dst swapped
}
fn f_soft_light(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    let darkened = d - (one - 2.0 * s) * d * (one - d);
    let lightened = d + (2.0 * s - one) * (soft_light_d(d) - d);
    return select(lightened, darkened, s <= vec3<f32>(0.5));
}
fn f_colour_burn(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    let base = one - min(one, (one - d) / max(s, vec3<f32>(1e-6)));
    let r = select(base, vec3<f32>(0.0), s <= vec3<f32>(0.0)); // s==0 → 0
    return select(r, one, d >= one); // d==1 → 1 (wins)
}
fn f_colour_dodge(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    let base = min(one, d / max(one - s, vec3<f32>(1e-6)));
    let r = select(base, one, s >= one); // s==1 → 1
    return select(r, vec3<f32>(0.0), d <= vec3<f32>(0.0)); // d==0 → 0 (wins)
}
fn f_linear_burn(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return clamp(s + d - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0));
}
fn f_linear_light(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return clamp(d + 2.0 * s - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0));
}
fn f_vivid_light(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let burn = f_colour_burn(2.0 * s, d);
    let dodge = f_colour_dodge(2.0 * s - vec3<f32>(1.0), d);
    return select(dodge, burn, s <= vec3<f32>(0.5));
}
fn f_pin_light(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let lo = min(d, 2.0 * s);
    let hi = max(d, 2.0 * s - vec3<f32>(1.0));
    return select(hi, lo, s <= vec3<f32>(0.5));
}
fn f_hard_mix(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    let v = f_vivid_light(s, d);
    return select(vec3<f32>(0.0), vec3<f32>(1.0), v >= vec3<f32>(0.5));
}
fn f_difference(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return abs(s - d);
}
fn f_exclusion(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return s + d - 2.0 * s * d;
}
fn f_divide(s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    return clamp(d / max(s, vec3<f32>(1e-6)), vec3<f32>(0.0), vec3<f32>(1.0));
}

// --- Non-separable (HSL) helpers (W3C compositing §non-separable). ---

fn blend_lum(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.3, 0.59, 0.11));
}
fn clip_colour(c: vec3<f32>) -> vec3<f32> {
    let l = blend_lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    var r = c;
    if (n < 0.0) {
        r = l + (r - l) * (l / max(l - n, 1e-6));
    }
    if (x > 1.0) {
        r = l + (r - l) * ((1.0 - l) / max(x - l, 1e-6));
    }
    return r;
}
fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    return clip_colour(c + (l - blend_lum(c)));
}
fn blend_sat(c: vec3<f32>) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let mn = min(c.r, min(c.g, c.b));
    let mx = max(c.r, max(c.g, c.b));
    return select(vec3<f32>(0.0), (c - mn) * s / max(mx - mn, 1e-6), mx > mn);
}

// Dispatch every encoded-domain (perceptual) blend mode. `s` = source,
// `d` = destination, both in the encoded (display-referred) domain, [0,1].
fn blend_encoded(mode: f32, s: vec3<f32>, d: vec3<f32>) -> vec3<f32> {
    if (mode < 0.5) { return f_screen(s, d); }
    else if (mode < 1.5) { return f_overlay(s, d); }
    else if (mode < 2.5) { return f_soft_light(s, d); }
    else if (mode < 3.5) { return f_hard_light(s, d); }
    else if (mode < 7.5) { return f_colour_burn(s, d); }        // 7
    else if (mode < 8.5) { return f_linear_burn(s, d); }        // 8
    else if (mode < 9.5) {                                      // 9 darker colour
        return select(d, s, blend_lum(s) < blend_lum(d));
    }
    else if (mode < 10.5) { return f_colour_dodge(s, d); }      // 10
    else if (mode < 11.5) {                                     // 11 lighter colour
        return select(d, s, blend_lum(s) > blend_lum(d));
    }
    else if (mode < 12.5) { return f_vivid_light(s, d); }       // 12
    else if (mode < 13.5) { return f_linear_light(s, d); }      // 13
    else if (mode < 14.5) { return f_pin_light(s, d); }         // 14
    else if (mode < 15.5) { return f_hard_mix(s, d); }          // 15
    else if (mode < 16.5) { return f_difference(s, d); }        // 16
    else if (mode < 17.5) { return f_exclusion(s, d); }         // 17
    else if (mode < 18.5) { return f_divide(s, d); }            // 18
    else if (mode < 19.5) {                                     // 19 hue
        return set_lum(set_sat(s, blend_sat(d)), blend_lum(d));
    }
    else if (mode < 20.5) {                                     // 20 saturation
        return set_lum(set_sat(d, blend_sat(s)), blend_lum(d));
    }
    else if (mode < 21.5) {                                     // 21 colour
        return set_lum(s, blend_lum(d));
    }
    else {                                                      // 22 luminosity
        return set_lum(d, blend_lum(s));
    }
}

// Snapshot blends (docs/06-RENDER-PIPELINE.md §blend domains): the fragment
// reads the accumulated comp itself and writes the finished value (fixed-
// function blending off). The perceptual set (K-162, T24) runs encoded —
// encode both sides, apply the formula, decode. Lighten/Darken/Subtract are
// domain-invariant and run directly in linear.
@fragment
fn fs_layer_snapshot(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(src, samp, in.uv);
    var a = texel.a * layer.params.x * textureSample(layer_mask, samp, in.uv).a;
    let comp_uv = in.pos.xy / layer.target_size.xy;
    if (layer.params.y > 0.5) {
        let m = textureSample(matte, samp, comp_uv);
        var strength = m.a;
        if (layer.params.z > 0.5) {
            // Luma matte: Rec.709 Y of the sRGB-ENCODED signal (perceptual luma,
            // matching After Effects), not of linear light (docs/06 §3.5a).
            strength = dot(
                srgb_encode_c(clamp(m.rgb, vec3<f32>(0.0), vec3<f32>(1.0))),
                vec3<f32>(0.2126, 0.7152, 0.0722),
            );
        }
        if (layer.params.w > 0.5) {
            strength = 1.0 - strength;
        }
        a = a * clamp(strength, 0.0, 1.0);
    }
    let dst = textureSample(dst_snapshot, samp, comp_uv);
    let mode = layer.target_size.z;
    var blended: vec3<f32>;
    if (mode >= 3.5 && mode < 4.5) { // lighten: per-channel max, linear
        blended = max(texel.rgb, dst.rgb);
    } else if (mode >= 4.5 && mode < 5.5) { // darken: per-channel min, linear
        blended = min(texel.rgb, dst.rgb);
    } else if (mode >= 5.5 && mode < 6.5) { // subtract: dst − src, clamped, linear
        blended = max(dst.rgb - texel.rgb, vec3<f32>(0.0));
    } else { // the encoded (perceptual) set: 0–3 and 7–22
        let s_enc = srgb_encode_c(clamp(texel.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
        let d_enc = srgb_encode_c(clamp(dst.rgb, vec3<f32>(0.0), vec3<f32>(1.0)));
        blended = srgb_decode_c(blend_encoded(mode, s_enc, d_enc));
    }
    let rgb = mix(dst.rgb, blended, a);
    let out_a = a + dst.a * (1.0 - a);
    return vec4<f32>(rgb, out_a);
}

@fragment
fn fs_layer(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(src, samp, in.uv);
    // Straight-alpha source → premultiplied output, opacity folded in.
    var a = texel.a * layer.params.x * textureSample(layer_mask, samp, in.uv).a;
    if (layer.params.y > 0.5) {
        // Matte lives in comp space: sample at this fragment's comp position.
        let comp_uv = in.pos.xy / layer.target_size.xy;
        let m = textureSample(matte, samp, comp_uv);
        var strength = m.a;
        if (layer.params.z > 0.5) {
            // Luma matte: Rec.709 Y of the sRGB-ENCODED signal (perceptual luma,
            // matching After Effects), not of linear light (docs/06 §3.5a).
            strength = dot(
                srgb_encode_c(clamp(m.rgb, vec3<f32>(0.0), vec3<f32>(1.0))),
                vec3<f32>(0.2126, 0.7152, 0.0722),
            );
        }
        if (layer.params.w > 0.5) {
            strength = 1.0 - strength;
        }
        a = a * clamp(strength, 0.0, 1.0);
    }
    return vec4<f32>(texel.rgb * a, a);
}

// fp32 accumulation (docs/06 §4). The combine sums its weighted premultiplied
// sub-frames in an Rgba32Float target, so a still scene averages back to itself
// bit-for-bit (an fp16 target rounds the 0.75·v partial sum and drifts a LSB on
// fractional coverage). Each pass reads the running sum (accum_prev, an
// unfilterable float read by textureLoad — never sampled) and adds this
// sub-frame's weighted premultiplied texel; the host ping-pongs two fp32 targets.
@group(0) @binding(6) var accum_prev: texture_2d<f32>;

@fragment
fn fs_accumulate_f32(in: VsOut) -> @location(0) vec4<f32> {
    let prev = textureLoad(accum_prev, vec2<i32>(in.pos.xy), 0);
    return prev + textureSample(src, samp, in.uv) * layer.params.x;
}

// Resolve the fp32 running sum back into the working (fp16) format — the single,
// final rounding, for downstream compositing and display.
@fragment
fn fs_copy_f32(in: VsOut) -> @location(0) vec4<f32> {
    return textureLoad(accum_prev, vec2<i32>(in.pos.xy), 0);
}

// Per-layer motion blur (docs/06 §4) sums its sub-frame placements the same way,
// but each placement is a TRANSFORMED quad, not a full-frame one — so it can't
// carry the running sum through pixels it doesn't cover. Instead each placement
// is rendered at FULL alpha into a cleared fp16 temp (this pass's `src`, 0
// outside the quad) and this full-frame pass adds `weight · temp` to the fp32
// running sum. The 1/N weight is applied HERE, in f32 — baking it into the fp16
// temp first would round each contribution and lose the bit-exact still-scene
// identity. `weight` rides in params.x.
@fragment
fn fs_add_f32(in: VsOut) -> @location(0) vec4<f32> {
    let coord = vec2<i32>(in.pos.xy);
    return textureLoad(accum_prev, coord, 0) + layer.params.x * textureLoad(src, coord, 0);
}
