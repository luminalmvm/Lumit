// Displacement map (docs/08-EFFECTS.md §3.49): another layer's channels push
// this one. Mirrors lumit_core::fx::cpu::displacement_map op-for-op (§1.6: the
// CPU is the oracle).
//
// **The matte IS the map** (the seventh override): what the Matte row
// supplies here is the displacement field itself, not an amount of one, so the
// texture comes into the kernel and no generic dissolve runs beside it. With
// none bound the kernel is a passthrough — the labelled no-op every layer-input
// effect follows.
//
// Mid-grey is the neutral: a map channel at 0.5 moves nothing, 1 pushes a full
// Amount one way and 0 a full Amount the other (AE's convention, §3.49).
//
// Mix 0, both Amounts at 0 and an unbound map are all the bit-exact identity.

struct Params {
    amount: vec2<f32>,   // farthest push per axis, raster px; signed
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    matte_on: f32,       // 0 = no map bound; the pass is a passthrough
    chan_x: u32,         // CHANNEL_OPTIONS index steering x
    chan_y: u32,         // ... and y
    edge: u32,           // 0 transparent, 1 repeat, 2 mirror
    invert: u32,         // 1 = read the map the other way round
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var matte: texture_2d<f32>;

// == cpu::channel_of: arithmetic only, no transcendentals (§1.6).
fn channel_of(m: vec4<f32>, which: u32) -> f32 {
    switch (which) {
        case 1u: { return m.a; }
        case 2u: { return m.r; }
        case 3u: { return m.g; }
        case 4u: { return m.b; }
        default: { return m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722; }
    }
}

// == fx_lensdistort.wgsl's edge_idx and cpu::edge_index. -1 means transparent.
fn edge_idx(i: i32, len: i32) -> i32 {
    if (i >= 0 && i < len) {
        return i;
    }
    if (p.edge == 1u) {
        return clamp(i, 0, len - 1);
    }
    if (p.edge == 2u) {
        var m = i;
        if (m < 0) {
            m = -m;
        } else {
            m = 2 * (len - 1) - m;
        }
        return clamp(m, 0, len - 1);
    }
    return -1;
}

// The tap NEVER loads out of bounds. A guard that early-returns before the
// textureLoad reads correctly on paper, but the load is side-effect-free and
// gets hoisted above the branch; on at least one Windows backend the hoisted
// out-of-range fetch comes back with a live alpha lane, so a pixel whose four
// taps are all outside the frame arrives opaque-and-wrong instead of empty.
// Clamping the coordinate and choosing the value afterwards has no such hazard,
// and costs one `select`. (Found by the §1.6 oracle for Polar coordinates,
// docs/08 §3.50 — the first kernel in the batch whose samples leave the frame.)
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    let xi = edge_idx(x, size.x);
    let yi = edge_idx(y, size.y);
    let c = clamp(vec2<i32>(xi, yi), vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return select(textureLoad(src, c, 0), vec4<f32>(0.0, 0.0, 0.0, 0.0), xi < 0 || yi < 0);
}

fn bilinear_edge(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
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
fn displacement_map(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    if (p.matte_on == 0.0) {
        textureStore(dst, xy, o);
        return;
    }
    let m = textureLoad(matte, xy, 0);
    var kx = channel_of(m, p.chan_x);
    var ky = channel_of(m, p.chan_y);
    if (p.invert != 0u) {
        kx = 1.0 - kx;
        ky = 1.0 - ky;
    }
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let v = bilinear_edge(
        px + (kx - 0.5) * 2.0 * p.amount.x,
        py + (ky - 0.5) * 2.0 * p.amount.y,
        size,
    );
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}
