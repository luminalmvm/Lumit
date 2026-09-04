// Hue and saturation (docs/08-EFFECTS.md §3.33): a master adjustment plus six
// colour ranges, each hue/saturation/lightness, through HSV on unpremultiplied
// colour (§2.2, the wrap fused into the kernel) and re-premultiplied on the
// way out. Mirrors lumit_core::fx::cpu::hue_saturation op-for-op (§1.6: the
// CPU is the oracle).
//
// Each range's weight is a hat function 120 degrees wide centred every 60, so
// the six sum to exactly 1 for any hue and there is no boundary to cross; the
// weights are then scaled by the pixel's own saturation, so a grey (whose hue
// reads 0, which is red) takes the Master adjustment alone. V is unbounded
// above throughout, so scene-linear headroom survives the round trip.

struct Params {
    // [master, reds, yellows, greens, cyans, blues, magentas], each
    // (hue degrees, saturation %, lightness %, unused).
    bands: array<vec4<f32>, 7>,
    mix_amt: f32,  // 0..1, blended against the unprocessed input
    matte_on: f32,     // 1 = the matte drives the control below
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
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

// The unpremultiplied colour of a premultiplied pixel (== cpu::unpremult).
fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// The HSV hue in degrees 0..360 (== cpu::hsv_hue). A neutral colour has no
// hue and answers 0; the range weights scale by saturation, so that costs a
// grey nothing.
fn hsv_hue(u: vec3<f32>, v: f32, c: f32) -> f32 {
    if (c <= 0.0) {
        return 0.0;
    }
    var sixth: f32;
    if (v == u.r) {
        sixth = (u.g - u.b) / c;
    } else if (v == u.g) {
        sixth = (u.b - u.r) / c + 2.0;
    } else {
        sixth = (u.r - u.g) / c + 4.0;
    }
    let h = sixth * 60.0;
    if (h < 0.0) {
        return h + 360.0;
    }
    return h;
}

// HSV back to RGB with V unbounded above (== cpu::hsv_to_rgb). The sector is
// wrapped, not clamped: the fold can land on exactly 360 when a turn rounds,
// and a clamp would answer that with magenta where the colour is red.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> vec3<f32> {
    let hh = h / 60.0;
    let sector = floor(hh);
    let f = hh - sector;
    let pv = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    let si = ((i32(sector) % 6) + 6) % 6;
    if (si == 0) { return vec3<f32>(v, t, pv); }
    if (si == 1) { return vec3<f32>(q, v, pv); }
    if (si == 2) { return vec3<f32>(pv, v, t); }
    if (si == 3) { return vec3<f32>(pv, q, v); }
    if (si == 4) { return vec3<f32>(t, pv, v); }
    return vec3<f32>(v, pv, q);
}

@compute @workgroup_size(8, 8)
fn hue_saturation(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    // Neutral short-circuit (== the CPU reference's early return): all
    // twenty-one adjustments at zero.
    var any = false;
    for (var i = 0; i < 7; i = i + 1) {
        if (p.bands[i].x != 0.0 || p.bands[i].y != 0.0 || p.bands[i].z != 0.0) {
            any = true;
        }
    }
    if (!any) {
        textureStore(dst, xy, o);
        return;
    }
    let u = unpremult(o);
    let v = max(u.r, max(u.g, u.b));
    let mn = min(u.r, min(u.g, u.b));
    let chroma = v - mn;
    var s = 0.0;
    if (v > 0.0) {
        s = clamp(chroma / v, 0.0, 1.0);
    }
    let h = hsv_hue(u, v, chroma);
    var dh = p.bands[0].x;
    var ds = p.bands[0].y;
    var dl = p.bands[0].z;
    for (var i = 1; i < 7; i = i + 1) {
        let centre = f32(i - 1) * 60.0;
        var d = abs(h - centre);
        if (d > 180.0) {
            d = 360.0 - d;
        }
        let w = max(1.0 - d / 60.0, 0.0) * s;
        dh = dh + w * p.bands[i].x;
        ds = ds + w * p.bands[i].y;
        dl = dl + w * p.bands[i].z;
    }
    // Folded into 0..360 by subtracting the floor — the form the CPU
    // reference spells, so the two agree op-for-op.
    // The matte scales every range's Hue, Saturation and Lightness toward 0
    // per pixel — applied to the sum, which is the same number.
    if (p.matte_on != 0.0) {
        let k = matte_k(xy);
        dh = dh * k;
        ds = ds * k;
        dl = dl * k;
    }
    let turned = h + dh;
    let h2 = turned - floor(turned / 360.0) * 360.0;
    let s2 = clamp(s * (1.0 + ds / 100.0), 0.0, 1.0);
    let v2 = max(v * (1.0 + dl / 100.0), 0.0);
    let graded = hsv_to_rgb(h2, s2, v2) * o.a;
    let outv = o.rgb * (1.0 - p.mix_amt) + graded * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
