// The Custom shader's host prologue (docs/impl/custom-shader.md §1.3). Constant
// text: the header the host fills, the seven bindings the host owns, and the
// helpers so that nobody has to write a wrong one. The user's text is appended
// after this, and `epilogue.wgsl` after that.
//
// The one generated line in here is the `Params` struct, spliced in at the
// marker below from the annotated block the user declared (§1.4) — lifted to
// the top so every binding and every helper is visible to every line the user
// writes, in a language where a declaration must precede its use.

struct LumitHeader {
    roi_offset: vec2<u32>,
    roi_size: vec2<u32>,
    comp_scale: f32,
    time: f32,
    seed: u32,
    mix_amt: f32,
    matte_on: f32,
    input2_on: f32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> lumit: LumitHeader;
@group(0) @binding(4) var matte: texture_2d<f32>;
@group(0) @binding(5) var input2: texture_2d<f32>;
//__LUMIT_PARAMS__
@group(0) @binding(6) var<uniform> p: Params;

fn lumit_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(src));
}

fn lumit_clamp_xy(xy: vec2<i32>) -> vec2<i32> {
    let last = vec2<i32>(textureDimensions(src)) - vec2<i32>(1, 1);
    return clamp(xy, vec2<i32>(0, 0), max(last, vec2<i32>(0, 0)));
}

fn lumit_load(xy: vec2<i32>) -> vec4<f32> {
    return textureLoad(src, lumit_clamp_xy(xy), 0);
}

// Bilinear from a texture there is no sampler for: the four loads and the two
// lerps, written once so that no two shaders disagree about the half-texel.
fn lumit_bilinear(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(src));
    let last = vec2<i32>(textureDimensions(src)) - vec2<i32>(1, 1);
    let hi = max(last, vec2<i32>(0, 0));
    let f = uv * size - vec2<f32>(0.5, 0.5);
    let base = floor(f);
    let frac = f - base;
    let i0 = clamp(vec2<i32>(base), vec2<i32>(0, 0), hi);
    let i1 = clamp(vec2<i32>(base) + vec2<i32>(1, 1), vec2<i32>(0, 0), hi);
    let c00 = textureLoad(t, vec2<i32>(i0.x, i0.y), 0);
    let c10 = textureLoad(t, vec2<i32>(i1.x, i0.y), 0);
    let c01 = textureLoad(t, vec2<i32>(i0.x, i1.y), 0);
    let c11 = textureLoad(t, vec2<i32>(i1.x, i1.y), 0);
    return mix(mix(c00, c10, frac.x), mix(c01, c11, frac.x), frac.y);
}

fn lumit_sample(uv: vec2<f32>) -> vec4<f32> {
    return lumit_bilinear(src, uv);
}

fn lumit_sample2(uv: vec2<f32>) -> vec4<f32> {
    return lumit_bilinear(input2, uv);
}

fn lumit_orig(uv: vec2<f32>) -> vec4<f32> {
    return lumit_bilinear(orig, uv);
}

// The K-395 matte's strength at a point: the premultiplied Rec. 709 luma the
// seam already prepared. `lumit.matte_on` says whether it means anything —
// binding 4 stands in as `src` when no matte is bound.
fn lumit_matte(uv: vec2<f32>) -> f32 {
    let m = lumit_bilinear(matte, uv);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// uv to px@comp: a distance written against this is right at every preview
// resolution, which a distance derived from `lumit_size()` is not.
fn lumit_px(uv: vec2<f32>) -> vec2<f32> {
    return uv * lumit_size() / max(lumit.comp_scale, 1e-6);
}

fn lumit_unpremult(c: vec4<f32>) -> vec4<f32> {
    if (c.a > 0.0) {
        return vec4<f32>(c.rgb / c.a, c.a);
    }
    return vec4<f32>(0.0, 0.0, 0.0, c.a);
}

fn lumit_premult(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}
