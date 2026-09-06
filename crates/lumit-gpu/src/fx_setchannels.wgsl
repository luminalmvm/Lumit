// Set channels (docs/08-EFFECTS.md §3.94): every output channel is told which
// channel of which picture it comes from. Mirrors
// lumit_core::fx::cpu::set_channels op-for-op (§1.6: the CPU is the oracle).
//
// The Source layer is this effect's OWN layer input, not a matte, so it
// arrives on binding 4 through the same dispatch_matted seam Set matte's source
// uses. The universal Matte row stays beside the effect and does the generic
// strength dissolve outside this kernel — nothing here reads it.
//
// It runs on STRAIGHT values (§2.2), fused into this one pass: a premultiplied
// channel carries its own alpha inside it, and reading one as a colour would
// read that alpha twice.
//
// Mix 0 is the bit-exact identity.

struct Params {
    // SET_CHANNELS_OPTIONS indices for R, G, B and A: 0..4 this layer's
    // R/G/B/A/luma, 5..9 the source's, 10 full on, 11 full off.
    picks: vec4<u32>,
    source_on: f32,    // 0 = no layer bound; every source pick reads zero
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    _pad0: f32,
    _pad1: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
@group(0) @binding(4) var source: texture_2d<f32>;

// == cpu::unpremult, then the alpha kept beside it: a fully transparent pixel's
// colour is undefined and reads as black, the identical rule on both paths.
fn straight_of(px: vec4<f32>) -> vec4<f32> {
    if (px.a > 0.0) {
        return vec4<f32>(px.rgb / px.a, px.a);
    }
    return vec4<f32>(0.0, 0.0, 0.0, px.a);
}

// == cpu::set_channels_pick. `own` is this layer, `s` the Source row; both are
// already straight. Arithmetic only, no transcendentals (§1.6). The name is
// `own` rather than `this` because WGSL reserves `this`.
fn pick_of(pick: u32, own: vec4<f32>, s: vec4<f32>) -> f32 {
    switch (pick) {
        case 0u: { return own.r; }
        case 1u: { return own.g; }
        case 2u: { return own.b; }
        case 3u: { return own.a; }
        case 4u: { return own.r * 0.2126 + own.g * 0.7152 + own.b * 0.0722; }
        case 5u: { return s.r; }
        case 6u: { return s.g; }
        case 7u: { return s.b; }
        case 8u: { return s.a; }
        case 9u: { return s.r * 0.2126 + s.g * 0.7152 + s.b * 0.0722; }
        case 10u: { return 1.0; }
        default: { return 0.0; }
    }
}

@compute @workgroup_size(8, 8)
fn set_channels(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let own = straight_of(o);
    var s = vec4<f32>(0.0);
    if (p.source_on != 0.0) {
        s = straight_of(textureLoad(source, xy, 0));
    }
    let a = pick_of(p.picks.w, own, s);
    let rgb = vec3<f32>(
        pick_of(p.picks.x, own, s),
        pick_of(p.picks.y, own, s),
        pick_of(p.picks.z, own, s),
    );
    let out_rgb = o.rgb * (1.0 - p.mix_amt) + rgb * a * p.mix_amt;
    let out_a = o.a * (1.0 - p.mix_amt) + a * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(out_rgb, out_a));
}
