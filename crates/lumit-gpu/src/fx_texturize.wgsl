// Texturize (docs/08-EFFECTS.md §3.68): another layer pressed into this one as
// relief. Mirrors lumit_core::fx::cpu::texturize op-for-op (§1.6: the CPU is the
// oracle).
//
// The texture layer is embossed exactly as fx_emboss.wgsl embosses a picture,
// and the light and shade that come out MULTIPLY this layer's colour — which is
// why nothing is unpremultiplied here: scaling premultiplied colour by a number
// is the same operation as scaling straight colour by it, and the shape is
// untouched. The texture's own taps ARE unpremultiplied, so a texture with a
// soft edge does not read as black there.
//
// The texture arrives at THIS raster (docs/impl/layer-input.md), which is why
// Placement is a fitting rather than a resize (§3.68 decision 2): Scale says how
// big one copy is, and Placement says only what happens outside it — Stretch
// holds the edge, Tile wraps, Centre leaves the rest untextured.
//
// An unset Texture never reaches this kernel: the host renders the identity.
// Mix 0 is the bit-exact identity.

struct Params {
    offset: vec2<f32>,  // toward the light, raster pixels
    contrast: f32,      // Texture contrast / 100
    inv_scale: f32,     // 100 / Scale
    placement: u32,     // 0 Stretch, 1 Tile, 2 Centre
    mix_amt: f32,       // 0..1, blended against the unprocessed input
    matte_on: f32,      // 1 = the matte scales Relief per pixel
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
// The texture layer, not the unprocessed original: this effect is one pass and
// `src` is already its own input (fx_roughenedges.wgsl does the same).
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::bilinear_edge on the texture, Repeat policy.
fn bilinear_repeat(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let hi = size - vec2<i32>(1, 1);
    let c00 = textureLoad(tex, clamp(vec2<i32>(x0i, y0i), vec2<i32>(0, 0), hi), 0);
    let c10 = textureLoad(tex, clamp(vec2<i32>(x0i + 1, y0i), vec2<i32>(0, 0), hi), 0);
    let c01 = textureLoad(tex, clamp(vec2<i32>(x0i, y0i + 1), vec2<i32>(0, 0), hi), 0);
    let c11 = textureLoad(tex, clamp(vec2<i32>(x0i + 1, y0i + 1), vec2<i32>(0, 0), hi), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

// The texture's perceptual luma at a coordinate, and whether the coordinate is
// textured at all (Centre leaves everything outside one copy alone). Returned as
// a vec2 rather than a struct: .x is the value, .y is 1 when it exists.
fn tap(u: f32, v: f32, size: vec2<i32>) -> vec2<f32> {
    var su = u;
    var sv = v;
    if (p.placement == 1u) {
        // Subtract-the-floor rather than a modulo, the form the CPU spells
        // (§3.38's note).
        su = u - floor(u);
        sv = v - floor(v);
    } else if (p.placement == 2u) {
        if (u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0) {
            return vec2<f32>(0.0, 0.0);
        }
    }
    let c = unpremult(bilinear_repeat(su * f32(size.x), sv * f32(size.y), size));
    return vec2<f32>(sqrt(max(dot(c, LUMA), 0.0)), 1.0);
}

@compute @workgroup_size(8, 8)
fn texturize(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let fw = f32(size.x);
    let fh = f32(size.y);
    var du = p.offset.x * p.inv_scale / fw;
    var dv = p.offset.y * p.inv_scale / fh;
    // The matte pulls Relief toward 0 per pixel, before the taps are read:
    // the two taps land on a different pair of texture pixels, which
    // is not a weaker version of the same difference.
    if (p.matte_on != 0.0) {
        let k = matte_k(xy);
        du = matte_toward(du, 0.0, k);
        dv = matte_toward(dv, 0.0, k);
    }
    let u = ((f32(xy.x) + 0.5) / fw - 0.5) * p.inv_scale + 0.5;
    let v = ((f32(xy.y) + 0.5) / fh - 0.5) * p.inv_scale + 0.5;
    let hi = tap(u + du, v + dv, size);
    let lo = tap(u - du, v - dv, size);
    var r = 0.0;
    if (hi.y > 0.5 && lo.y > 0.5) {
        r = (hi.x - lo.x) * p.contrast;
    }
    let lit = max(o.rgb * (1.0 + r), vec3<f32>(0.0));
    let outv = o.rgb * (1.0 - p.mix_amt) + lit * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
