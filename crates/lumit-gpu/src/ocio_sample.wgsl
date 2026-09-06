// The OCIO sampler, shared by the colour pipeline's display and input
// passes (colour.wgsl) and the OCIO effects' kernel (fx_ocio.wgsl). It is
// prepended to both at pipeline creation, which is WGSL's only way of having
// one module, so the two read a baked table with the same arithmetic and
// preview equals export inside an effect stack as well as at its edges
// (docs/impl/ocio.md §5.2).
//
// Everything below reads a BAKED TABLE and nothing else: no logarithm of the
// config's, no power of the config's, no arithmetic this shader invented. The
// maths a config describes was run once on the processor, at the bake, and
// what reaches here is the answers - so the only thing that can disagree
// between the Viewer, the export and an effect is a table lookup, and a table
// lookup is +, -, *, floor and clamp on both sides.
//
// The formulation is COPIED from `lumit-colour`'s `sample.rs` and `bake.rs` and
// must stay copied: six tetrahedra in the written order, `>=` as written, ties
// breaking top-first (ocio.md §4.3 - binding).
//
// The consumer declares `curve`, `cube` and `p` (an OcioParams uniform) at
// whatever group and bindings its pass uses; WGSL resolves module-scope names
// in any order, so this file declares none of them.

struct OcioParams {
    /// 1 = factorised (curve, matrix, curve); 2 = shaper and cube.
    mode: u32,
    has_pre: u32,
    has_post: u32,
    /// 0 = lg2, 1 = uniform.
    shaper_kind: u32,
    shaper_a: f32,
    shaper_b: f32,
    shaper_c: f32,
    /// `CURVE_SAMPLES - 1`, as a float because that is what it multiplies.
    curve_last: f32,
    curve_rows: u32,
    /// `cube size - 1`.
    cube_last: f32,
    _pad0: u32,
    _pad1: u32,
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
};

/// The row length the host uploads at. A power of two, so the wrap is a shift
/// and a mask rather than a division.
const CURVE_WIDTH: u32 = 1024u;

/// `f32::MIN_POSITIVE` - the floor `Shaper::forward` clamps to before taking a
/// logarithm, spelled out because WGSL has no name for it.
const MIN_POSITIVE: f32 = 1.17549435e-38;

/// `Shaper::forward`: squeeze one linear value into 0-1.
fn shaper_forward(x: f32) -> f32 {
    var y: f32;
    if (p.shaper_kind == 0u) {
        let span = p.shaper_b - p.shaper_a;
        if (span == 0.0) {
            y = 0.0;
        } else {
            y = (log2(max(x + p.shaper_c, MIN_POSITIVE)) - p.shaper_a) / span;
        }
    } else {
        let span = p.shaper_b - p.shaper_a;
        if (span == 0.0) {
            y = 0.0;
        } else {
            y = (x - p.shaper_a) / span;
        }
    }
    if (y != y) {
        return 0.0;
    }
    return clamp(y, 0.0, 1.0);
}

/// `Shaper::forward_signed`: the same squeeze folded about zero, which is how a
/// factorised curve answers negatives and highlights from the table instead of
/// from code that would have to be written twice.
fn shaper_forward_signed(x: f32) -> f32 {
    if (x != x) {
        return 0.5;
    }
    if (x >= 0.0) {
        return 0.5 + 0.5 * shaper_forward(x);
    }
    return 0.5 - 0.5 * shaper_forward(-x);
}

/// One sample of one stage's table.
fn curve_at(stage: u32, i: u32) -> vec3<f32> {
    let k = i + stage * p.curve_rows * CURVE_WIDTH;
    return textureLoad(curve, vec2<i32>(i32(k % CURVE_WIDTH), i32(k / CURVE_WIDTH)), 0).rgb;
}

/// `Curve::sample` over the fixed domain 0-1: map into the grid, clamp, lerp
/// two neighbours, per channel. All three at once, because the arithmetic is
/// the same and only the table row differs.
fn curve_sample(stage: u32, t: vec3<f32>) -> vec3<f32> {
    let last = p.curve_last;
    let raw = t * last;
    let g = select(clamp(raw, vec3<f32>(0.0), vec3<f32>(last)), vec3<f32>(0.0), raw != raw);
    let base = floor(g);
    let f = g - base;
    let top = u32(last);
    let i0 = min(vec3<u32>(base), vec3<u32>(top));
    let i1 = min(i0 + vec3<u32>(1u), vec3<u32>(top));
    let a = vec3<f32>(
        curve_at(stage, i0.x).x,
        curve_at(stage, i0.y).y,
        curve_at(stage, i0.z).z,
    );
    let b = vec3<f32>(
        curve_at(stage, i1.x).x,
        curve_at(stage, i1.y).y,
        curve_at(stage, i1.z).z,
    );
    return a + f * (b - a);
}

/// One stage: fold through the signed shaper, then read the table.
fn curve_stage(stage: u32, rgb: vec3<f32>) -> vec3<f32> {
    let t = vec3<f32>(
        shaper_forward_signed(rgb.x),
        shaper_forward_signed(rgb.y),
        shaper_forward_signed(rgb.z),
    );
    return curve_sample(stage, t);
}

/// `matrix::apply`: written out as `a*b + c` rather than fma, matching the
/// processor's refusal to fuse (ocio.md §4.2).
fn ocio_matrix(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        p.m0.x * rgb.x + p.m0.y * rgb.y + p.m0.z * rgb.z + p.m0.w,
        p.m1.x * rgb.x + p.m1.y * rgb.y + p.m1.z * rgb.z + p.m1.w,
        p.m2.x * rgb.x + p.m2.y * rgb.y + p.m2.z * rgb.z + p.m2.w,
    );
}

fn cube_at(r: u32, g: u32, b: u32) -> vec3<f32> {
    return textureLoad(cube, vec3<i32>(i32(r), i32(g), i32(b)), 0).rgb;
}

/// `Cube::sample`, tetrahedrally, over the fixed domain 0-1.
///
/// Six wedges chosen by ordering the fractions. The `>=` and the branch order
/// are load-bearing: ties must break here exactly as they break in
/// `lumit-colour`'s `Cube::sample`, or the two answers part company on the
/// wedge boundaries and nobody notices until a gradient bands.
fn cube_sample(rgb: vec3<f32>) -> vec3<f32> {
    let last = p.cube_last;
    let raw = rgb * last;
    let g = select(clamp(raw, vec3<f32>(0.0), vec3<f32>(last)), vec3<f32>(0.0), raw != raw);
    let base = floor(g);
    let f = g - base;
    let top = u32(last);
    let i0 = min(vec3<u32>(base), vec3<u32>(top));
    let i1 = min(i0 + vec3<u32>(1u), vec3<u32>(top));
    let fr = f.x;
    let fg = f.y;
    let fb = f.z;
    let c000 = cube_at(i0.x, i0.y, i0.z);
    let c111 = cube_at(i1.x, i1.y, i1.z);

    var a: vec3<f32>;
    var b: vec3<f32>;
    var wa: f32;
    var wb: f32;
    var wc: f32;
    if (fr >= fg && fg >= fb) {
        a = cube_at(i1.x, i0.y, i0.z);
        b = cube_at(i1.x, i1.y, i0.z);
        wa = fr; wb = fg; wc = fb;
    } else if (fr >= fb && fb >= fg) {
        a = cube_at(i1.x, i0.y, i0.z);
        b = cube_at(i1.x, i0.y, i1.z);
        wa = fr; wb = fb; wc = fg;
    } else if (fb >= fr && fr >= fg) {
        a = cube_at(i0.x, i0.y, i1.z);
        b = cube_at(i1.x, i0.y, i1.z);
        wa = fb; wb = fr; wc = fg;
    } else if (fg >= fr && fr >= fb) {
        a = cube_at(i0.x, i1.y, i0.z);
        b = cube_at(i1.x, i1.y, i0.z);
        wa = fg; wb = fr; wc = fb;
    } else if (fg >= fb && fb >= fr) {
        a = cube_at(i0.x, i1.y, i0.z);
        b = cube_at(i0.x, i1.y, i1.z);
        wa = fg; wb = fb; wc = fr;
    } else {
        a = cube_at(i0.x, i0.y, i1.z);
        b = cube_at(i0.x, i1.y, i1.z);
        wa = fb; wb = fg; wc = fr;
    }
    return c000 + wa * (a - c000) + wb * (b - a) + wc * (c111 - b);
}

/// `Artefact::eval`, on the card.
fn ocio_apply(rgb: vec3<f32>) -> vec3<f32> {
    if (p.mode == 2u) {
        return cube_sample(vec3<f32>(
            shaper_forward(rgb.x),
            shaper_forward(rgb.y),
            shaper_forward(rgb.z),
        ));
    }
    var c = rgb;
    if (p.has_pre != 0u) {
        c = curve_stage(0u, c);
    }
    c = ocio_matrix(c);
    if (p.has_post != 0u) {
        c = curve_stage(1u, c);
    }
    return c;
}
