// Set matte (docs/08-EFFECTS.md §3.44): the chosen channel of another layer
// becomes this layer's alpha. Mirrors lumit_core::fx::cpu::set_matte op-for-op
// (§1.6: the CPU is the oracle).
//
// **The matte IS the effect**: this is the sixth kernel to claim
// the universal Matte row inside its own maths rather than take the generic
// strength dissolve, because what its matte supplies is the coverage, not an
// amount of coverage. With none bound the kernel is a passthrough — the labelled
// no-op every layer-input effect follows.
//
// It runs on STRAIGHT values (§2.2), fused into this one pass: the job is to
// change how much of a pixel there is without changing what colour it is, and a
// premultiplied value multiplied by a new alpha would have been scaled twice.
//
// Mix 0 is the bit-exact identity.

struct Params {
    channel: u32,      // CHANNEL_OPTIONS index: 0 luma, 1 alpha, 2 R, 3 G, 4 B
    combine: u32,      // 1 = intersect with the existing alpha instead of replacing it
    matte_on: f32,     // 0 = no layer bound; the pass is a passthrough
    invert: f32,       // 1 = read the matte the other way round
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var matte: texture_2d<f32>;

// == cpu::channel_of: arithmetic only, no transcendentals (§1.6).
fn channel_of(m: vec4<f32>) -> f32 {
    switch (p.channel) {
        case 1u: { return m.a; }
        case 2u: { return m.r; }
        case 3u: { return m.g; }
        case 4u: { return m.b; }
        default: { return m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722; }
    }
}

@compute @workgroup_size(8, 8)
fn set_matte(@builtin(global_invocation_id) gid: vec3<u32>) {
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
    var k = channel_of(textureLoad(matte, xy, 0));
    if (p.invert != 0.0) {
        k = 1.0 - k;
    }
    var a = k;
    if (p.combine != 0u) {
        a = o.a * k;
    }
    // == cpu::unpremult: a fully transparent pixel's colour is undefined and
    // reads as black, the identical rule on both paths.
    var straight = vec3<f32>(0.0);
    if (o.a > 0.0) {
        straight = o.rgb / o.a;
    }
    let rgb = o.rgb * (1.0 - p.mix_amt) + straight * a * p.mix_amt;
    let outa = o.a * (1.0 - p.mix_amt) + a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(rgb, outa));
}
