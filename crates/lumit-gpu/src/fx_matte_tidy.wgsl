// The screen matte's two neighbourhood stages (docs/08-EFFECTS.md §3.21):
// shrink/grow and despot. Mirrors lumit_core::fx::cpu::matte_morph and
// ::matte_despot op-for-op (§1.6: the CPU is the oracle).
//
// Both run on the matte, never on the picture: `src` is the four-channel matte
// the screen pass wrote, with the same number in every channel, and `dst` is the
// same again. Reading `.r` and writing `vec4(v)` keeps that invariant, which is
// what lets the Softness stage between them be the shared Gaussian blur.
//
// Clamp-to-edge on both, the same edge policy that blur runs on here.

struct Params {
    dx: i32,      // the separable pass's direction: (1,0) then (0,1)
    dy: i32,
    ri: i32,      // whole rings of the structuring element
    frac: f32,    // how far the outermost ring has eased in, 0..1
    grow: u32,    // 1 = grow (max), 0 = shrink (min)
    black: f32,   // despot black, 0..1
    white: f32,   // despot white, 0..1
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The matte at a pixel, clamped to the edge (== the CPU's `at`).
fn at(xy: vec2<i32>, size: vec2<i32>) -> f32 {
    let c = clamp(xy, vec2<i32>(0, 0), size - vec2<i32>(1, 1));
    return textureLoad(src, c, 0).r;
}

// Grow takes the brighter, shrink the darker.
fn opv(a: f32, b: f32) -> f32 {
    if (p.grow == 1u) {
        return max(a, b);
    }
    return min(a, b);
}

// STAGE 3: one separable morphological pass. The edge marches without
// softening -- that is the whole difference between this and Softness. The
// outermost ring eases in with `frac` so the control is continuous across a
// whole-pixel boundary and the §1.6 oracle still holds.
@compute @workgroup_size(8, 8)
fn matte_morph(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let d = vec2<i32>(p.dx, p.dy);
    var acc = at(xy, size);
    for (var k: i32 = 1; k <= p.ri; k = k + 1) {
        acc = opv(acc, at(xy - d * k, size));
        acc = opv(acc, at(xy + d * k, size));
    }
    let outer = opv(at(xy - d * (p.ri + 1), size), at(xy + d * (p.ri + 1), size));
    let v = acc + p.frac * (opv(acc, outer) - acc);
    textureStore(dst, xy, vec4<f32>(v));
}

// STAGE 5: remove isolated specks. A speck is a pixel that disagrees
// with ALL eight of its neighbours, so a pixel on a real edge -- which always
// has a neighbour on its own side -- is left alone. The two amounts are 0..1
// blends, so 0 is the bit-exact identity.
@compute @workgroup_size(8, 8)
fn matte_despot(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    // Not `inf`: WGSL has no infinity literal, and the CPU reference starts its
    // reduction from this same number.
    var mn = 1e30;
    var mx = -1e30;
    for (var oy: i32 = -1; oy <= 1; oy = oy + 1) {
        for (var ox: i32 = -1; ox <= 1; ox = ox + 1) {
            if (ox == 0 && oy == 0) {
                continue;
            }
            let v = at(xy + vec2<i32>(ox, oy), size);
            mn = min(mn, v);
            mx = max(mx, v);
        }
    }
    let m0 = at(xy, size);
    let filled = m0 + p.black * (max(m0, mn) - m0);
    let v = filled + p.white * (min(filled, mx) - filled);
    textureStore(dst, xy, vec4<f32>(v));
}
