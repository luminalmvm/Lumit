// Stroke (**style**, docs/impl/layer-styles.md §4): an alpha-contour
// stroke. Mirrors lumit_core::fx::cpu::stroke_contour op-for-op (§1.6: the CPU
// is the oracle), whose two morphological copies are `cpu::matte_morph` — so
// this file's `stroke_morph` mirrors `matte_morph` twice over, once per copy.
//
// Deliberately NOT the Stroke *effect*, which paints a mask's own path: this one
// reads the layer's alpha and knows nothing about geometry.
//
// Three dispatches. `stroke_morph` runs twice, horizontally then vertically, and
// carries BOTH copies at once — the fattened alpha in .r and the thinned one in
// .g — because a running max and a running min over the same line read the same
// taps, and one pass that does both is one pass rather than four. `stage` says
// which of the two it is: the first reads the layer's alpha, the second reads
// the pair the first wrote. `stroke_combine` then cuts the band between the two
// copies, paints it and lays it over the layer.
//
// Clamp-to-edge on both copies, the same edge policy `matte_morph` runs on.
// Size 0 is the bit-exact identity (both copies are the alpha, so the band is
// empty), and so is Mix 0.

struct Params {
    colour: vec4<f32>,     // scene-linear; the alpha lane is ignored
    dx: i32,               // the separable pass's direction: (1,0) then (0,1)
    dy: i32,
    grow_ri: i32,          // whole rings of the fattening element
    grow_frac: f32,        // how far its outermost ring has eased in, 0..1
    shrink_ri: i32,        // and the same pair for the thinning one
    shrink_frac: f32,
    opacity: f32,          // 0..1
    mix_amt: f32,          // 0..1, blended against the unprocessed input
    stage: u32,            // 0 = first separable pass (read the alpha), 1 = second
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The two copies at a pixel, clamped to the edge (== the CPU's `at`). On the
// first pass both start as the layer's own alpha, which is what makes a zero
// half-width the identity on either of them.
fn pair(xy: vec2<i32>, size: vec2<i32>) -> vec2<f32> {
    let c = clamp(xy, vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    let t = textureLoad(src, c, 0);
    if (p.stage == 0u) {
        return vec2<f32>(t.a, t.a);
    }
    return vec2<f32>(t.r, t.g);
}

// One separable morphological pass, both copies at once: .x marches outward on
// a running max, .y inward on a running min. The outermost ring of each eases in
// with its own fraction, so dragging Size is continuous across a whole-pixel
// boundary and the §1.6 oracle stays continuous with it.
@compute @workgroup_size(8, 8)
fn stroke_morph(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let d = vec2<i32>(p.dx, p.dy);
    let here = pair(xy, size);
    var acc = here;
    for (var k: i32 = 1; k <= p.grow_ri; k = k + 1) {
        acc.x = max(acc.x, max(pair(xy - d * k, size).x, pair(xy + d * k, size).x));
    }
    for (var k: i32 = 1; k <= p.shrink_ri; k = k + 1) {
        acc.y = min(acc.y, min(pair(xy - d * k, size).y, pair(xy + d * k, size).y));
    }
    let go = max(pair(xy - d * (p.grow_ri + 1), size).x,
                 pair(xy + d * (p.grow_ri + 1), size).x);
    let so = min(pair(xy - d * (p.shrink_ri + 1), size).y,
                 pair(xy + d * (p.shrink_ri + 1), size).y);
    let grown = acc.x + p.grow_frac * (max(acc.x, go) - acc.x);
    let shrunk = acc.y + p.shrink_frac * (min(acc.y, so) - acc.y);
    textureStore(dst, xy, vec4<f32>(grown, shrunk, 0.0, 0.0));
}

// The band between the two copies, painted and laid OVER the layer. Because the
// fat copy is never smaller than the shape and the thin one never larger,
// Outside cannot put a pixel inside the shape and Inside cannot put one outside
// it — arithmetic, not a clip.
//
// binding 0 is the layer (which doubles as the unprocessed original for Mix,
// this being one logical pass); binding 1 is the pair the morph passes wrote.
@compute @workgroup_size(8, 8)
fn stroke_combine(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let m = textureLoad(orig, xy, 0);
    let band = clamp(m.r - m.g, 0.0, 1.0);
    let k = band * p.opacity;
    let over = vec4<f32>(p.colour.rgb * k + o.rgb * (1.0 - k), k + o.a * (1.0 - k));
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + over * p.mix_amt);
}
