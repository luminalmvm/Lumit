// Drop shadow (docs/08-EFFECTS.md §3.43), the combine pass: the layer's shape,
// already softened by the shared §3.8 gaussian, read at the shifted position,
// painted in one colour and composited UNDERNEATH the layer. Mirrors the second
// half of lumit_core::fx::cpu::drop_shadow op-for-op (§1.6: the CPU is the
// oracle).
//
// binding 0 is the sharp source (which doubles as the unprocessed original for
// Mix, this being one logical pass); binding 1 is the blurred copy. The blur and
// the offset commute, so the shape is softened where it stands and read where
// the shadow goes — one gaussian instead of a gaussian plus a resample.
//
// Mix 0 is the bit-exact identity, and so is Opacity 0.

struct Params {
    colour: vec4<f32>,     // scene-linear; the alpha lane is ignored
    offset: vec2<f32>,     // where the shadow sits relative to the shape, raster px
    opacity: f32,          // 0..1
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    shadow_only: u32,
    matte_on: f32,         // 1 = the matte scales the shadow's Opacity (K-428)
    spread_scale: f32,     // Spread's threshold-remap slope (K-706); 1 = none
    knockout: u32,         // 1 = the layer's shape knocks the shadow out (K-706)
    invert: u32,           // 1 = read the coverage from the inverted alpha (K-706)
    inner: u32,            // 1 = composite inside the shape and over it (K-706)
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var soft: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

// == cpu::bilinear_edge with the Transparent policy (edge == 0): a shape
// touching the frame border casts a shadow that leaves the frame, and repeating
// the border pixel outward would smear it into a fan.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    if (x < 0 || x >= size.x || y < 0 || y >= size.y) {
        return vec4<f32>(0.0);
    }
    return textureLoad(soft, vec2<i32>(x, y), 0);
}

fn bilinear_transparent(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn drop_shadow(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // The matte pulls the shadow's Opacity toward 0 per pixel, read where the
    // shadow FALLS rather than where the shape stands (K-428), so the matte's
    // own picture is the picture of where the shadow lands.
    var opacity = p.opacity;
    if (p.matte_on != 0.0) {
        opacity = matte_toward(opacity, 0.0, matte_k(xy));
    }
    var cover = bilinear_transparent(f32(xy.x) + 0.5 - p.offset.x,
                                     f32(xy.y) + 0.5 - p.offset.y,
                                     size).a;
    // Inverted alpha (K-706, == cpu::drop_shadow_matted): the softened picture
    // of what the shape is NOT. Outside the frame the sample is 0, so this reads
    // 1 there — which is right, because outside the frame is outside the shape.
    if (p.invert != 0u) {
        cover = 1.0 - cover;
    }
    // Spread (K-706, == cpu::drop_shadow_matted): the gaussian's ramp re-cut
    // about its half-way line, which is where the original edge was. Skipped
    // whole at slope 1, so a shadow with no spread is the bytes it always was.
    if (p.spread_scale != 1.0) {
        cover = clamp((cover - 0.5) * p.spread_scale + 0.5, 0.0, 1.0);
    }
    // Layer knocks out shadow (K-706): the shape takes the shadow away first,
    // and the composite below then puts the layer over what is left.
    if (p.knockout != 0u) {
        cover = cover * (1.0 - o.a);
    }
    let k = cover * opacity;
    let shadow = vec4<f32>(p.colour.rgb * k, k);
    // Source OVER shadow, premultiplied — the shadow is BELOW, which is the
    // whole reason this is an effect and not a duplicated layer.
    var over = o + shadow * (1.0 - o.a);
    if (p.shadow_only != 0u) {
        over = shadow;
    }
    // Interior (K-706): the layer's colour carried toward the style's by the
    // coverage, alpha untouched. Both sides already carry the layer's alpha, so
    // an interior style cannot put a pixel where the layer was not.
    if (p.inner != 0u) {
        over = vec4<f32>(o.rgb * (1.0 - k) + p.colour.rgb * o.a * k, o.a);
    }
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + over * p.mix_amt);
}
