// Shadow highlight (docs/08-EFFECTS.md §3.63): the local rescue of a backlit
// shot. Mirrors lumit_core::fx::cpu::shadow_highlight op-for-op (§1.6: the CPU
// is the oracle).
//
// This is the SECOND pass. The first is the shipped §3.8 gaussian at Radius,
// run on the picture and bound here as `soft` — the third time that blur has
// paid for another effect, after §3.43's softening and §3.57's distance field.
// It answers one question only: how bright is this pixel's *neighbourhood*?
// Nothing of its colour is ever used, so nothing here softens the picture.
//
// A wholly neutral instance never reaches this kernel (the host short-circuits
// to the identity, and does not even run the blur).

struct Params {
    shadow: f32,           // Shadow amount / 100 * 2
    highlight: f32,        // Highlight amount / 100 * 2
    shadow_width: f32,     // Shadow tonal width / 100, floored
    highlight_width: f32,  // Highlight tonal width / 100, floored
    contrast: f32,         // 1 + Midtone contrast / 100
    colour_correction: f32, // Colour correction / 100
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    matte_on: f32,     // 1 = the matte drives the control below (K-395)
};

@group(0) @binding(0) var src: texture_2d<f32>;
// The blurred picture, not the unprocessed original: this effect is one pass
// and `src` is already its own input (fx_roughenedges.wgsl does the same).
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

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::smoothstep_between.
fn smoothstep_between(lo: f32, hi: f32, x: f32) -> f32 {
    let t = clamp((x - lo) / (hi - lo), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(8, 8)
fn shadow_highlight(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let s = textureLoad(soft, xy, 0);
    let u = unpremult(o);
    let ub = unpremult(s);
    let l = max(dot(u, LUMA), 0.0);
    let lb = dot(ub, LUMA);
    // Where the neighbourhood sits on the tone range, perceptually. Clamped at
    // 1 because a highlight mask has to saturate somewhere.
    let t = min(sqrt(max(lb, 0.0)), 1.0);
    let ms = 1.0 - smoothstep_between(0.0, p.shadow_width, t);
    let mh = smoothstep_between(1.0 - p.highlight_width, 1.0, t);
    // A multiply, not a gamma (§3.63): monotone, no clamp, no inverse.
    // The matte scales Shadow amount and Highlight amount per pixel (K-395).
    var shadow = p.shadow;
    var highlight = p.highlight;
    if (p.matte_on != 0.0) {
        let m = matte_k(xy);
        shadow = shadow * m;
        highlight = highlight * m;
    }
    let lifted = l * (1.0 + ms * shadow) / (1.0 + mh * highlight);
    let q = max((sqrt(max(lifted, 0.0)) - 0.5) * p.contrast + 0.5, 0.0);
    let out_l = q * q;
    var k = 1.0;
    if (l > 1e-6) {
        k = out_l / l;
    }
    let v = u * k;
    let g = dot(v, LUMA);
    let sat = 1.0 + p.colour_correction * min(abs(k - 1.0), 1.0);
    let corrected = max(vec3<f32>(g) + (v - vec3<f32>(g)) * sat, vec3<f32>(0.0));
    let outv = o.rgb * (1.0 - p.mix_amt) + corrected * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
