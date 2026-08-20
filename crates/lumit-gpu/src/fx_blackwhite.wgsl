// Black and white (docs/08-EFFECTS.md §3.62): six weights, one grey. Mirrors
// lumit_core::fx::cpu::black_and_white and cpu::bw_grey op-for-op (§1.6: the
// CPU is the oracle).
//
// The colour is decomposed exactly into a grey, one secondary and one primary,
// and the two weights those parts name are applied. The six branches agree
// wherever two channels are equal, so a gradient has no seam.

struct Params {
    // Reds, Yellows, Greens, Cyans, Blues, Magentas -- each / 100, packed two
    // to a vec4 so the uniform stays 16-byte aligned.
    w0: vec4<f32>,   // reds, yellows, greens, cyans
    w1: vec4<f32>,   // blues, magentas, unused, unused
    tint: vec4<f32>, // .rgb only; already divided by its own luma
    tint_on: f32,    // 1 to tint, 0 to leave the grey grey
    mix_amt: f32,    // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

fn unpremult(c: vec4<f32>) -> vec3<f32> {
    if (c.a > 0.0) {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

// == cpu::bw_grey. (base, secondary amount, secondary weight, primary amount,
// primary weight), in the same six-branch order.
fn bw_grey(u: vec3<f32>) -> f32 {
    let r = u.r;
    let g = u.g;
    let b = u.b;
    let reds = p.w0.x;
    let yellows = p.w0.y;
    let greens = p.w0.z;
    let cyans = p.w0.w;
    let blues = p.w1.x;
    let magentas = p.w1.y;
    var base = 0.0;
    var sec = 0.0;
    var sw = 0.0;
    var pri = 0.0;
    var pw = 0.0;
    if (r >= g && g >= b) {
        base = b; sec = g - b; sw = yellows;  pri = r - g; pw = reds;
    } else if (g >= r && r >= b) {
        base = b; sec = r - b; sw = yellows;  pri = g - r; pw = greens;
    } else if (g >= b && b >= r) {
        base = r; sec = b - r; sw = cyans;    pri = g - b; pw = greens;
    } else if (b >= g && g >= r) {
        base = r; sec = g - r; sw = cyans;    pri = b - g; pw = blues;
    } else if (b >= r && r >= g) {
        base = g; sec = r - g; sw = magentas; pri = b - r; pw = blues;
    } else {
        base = g; sec = b - g; sw = magentas; pri = r - b; pw = reds;
    }
    return base + sec * sw + pri * pw;
}

@compute @workgroup_size(8, 8)
fn black_and_white(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let u = unpremult(o);
    let grey = max(bw_grey(u), 0.0);
    let tinted = vec3<f32>(grey) * p.tint.rgb;
    let v = vec3<f32>(grey) + (tinted - vec3<f32>(grey)) * p.tint_on;
    let outv = o.rgb * (1.0 - p.mix_amt) + v * o.a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
