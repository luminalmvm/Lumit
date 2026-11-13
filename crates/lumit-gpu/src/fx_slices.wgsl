// Slices: the picture cut into strips, each read back along its length by its
// own eased shift, averaged over up to 16 taps. Mirrors
// lumit_core::fx::effects::slices op-for-op (the CPU is the oracle). The noise
// core is prepended for its hash.

struct Params {
    t: f32,          // how far through the cut, 0..1
    stagger: f32,    // how far apart the strips set off, 0..0.95
    smear: f32,      // half the widest smear window, in a strip's own time
    mix: f32,
    count: u32,      // strips, 2..64
    vertical: u32,   // 1: columns that travel up and down
    alternate: u32,  // 1: every other strip travels the other way
    random: u32,     // 1: strips set off in a seeded order
    ease: u32,       // 0 Linear, 1 Smooth, 2 Sharp
    incoming: u32,   // 1: this is the incoming picture
    seed: u32,
    _pad: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

const TAPS: u32 = 16u;

// == eased.
fn eased(t: f32) -> f32 {
    if (p.ease == 0u) {
        return t;
    }
    if (p.ease == 2u) {
        if (t < 0.5) {
            return 16.0 * t * t * t * t * t;
        }
        let r = 1.0 - t;
        return 1.0 - 16.0 * r * r * r * r * r;
    }
    return t * t * (3.0 - 2.0 * t);
}

// One pixel of the picture at `a` along a strip and `b` across, nothing past
// the frame's edge.
fn read(a: i32, b: i32, len: i32) -> vec4<f32> {
    if (a < 0 || a >= len) {
        return vec4<f32>(0.0);
    }
    if (p.vertical == 1u) {
        return textureLoad(src, vec2<i32>(b, a), 0);
    }
    return textureLoad(src, vec2<i32>(a, b), 0);
}

@compute @workgroup_size(8, 8)
fn slices(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    var a = xy.x;
    var b = xy.y;
    var len = size.x;
    var across = size.y;
    if (p.vertical == 1u) {
        a = xy.y;
        b = xy.x;
        len = size.y;
        across = size.x;
    }
    let n = p.count;
    // == slices_shifts.
    let i = min(u32(b) * n / u32(across), n - 1u);
    var sign = 1.0;
    if (p.alternate == 1u && i % 2u == 1u) {
        sign = -1.0;
    }
    var order = f32(i) / f32(n - 1u);
    if (p.random == 1u) {
        order = nc_hash01(p.seed, 0u, i32(i), 0, 0);
    }
    let ti = clamp((p.t - order * p.stagger) / (1.0 - p.stagger), 0.0, 1.0);
    let half = min(min(p.smear, ti), 1.0 - ti);
    var count = 1u;
    if (half > 0.0) {
        count = TAPS;
    }
    let lenf = f32(len);
    var sum = vec4<f32>(0.0);
    for (var k = 0u; k < count; k = k + 1u) {
        var at = ti;
        if (count != 1u) {
            at = ti - half + 2.0 * half * f32(k) / f32(TAPS - 1u);
        }
        let e = eased(at);
        var shift = sign * lenf * e;
        if (p.incoming == 1u) {
            shift = -sign * lenf * (1.0 - e);
        }
        let f = f32(a) - shift;
        let i0 = floor(f);
        let frac = f - i0;
        let ia = i32(i0);
        sum = sum + ((1.0 - frac) * read(ia, b, len) + frac * read(ia + 1, b, len));
    }
    let inv = 1.0 / f32(count);
    let px = textureLoad(orig, xy, 0);
    textureStore(dst, xy, px * (1.0 - p.mix) + sum * inv * p.mix);
}
