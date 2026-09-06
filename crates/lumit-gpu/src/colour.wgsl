// The colour pipeline's one shader (docs/impl/gpu-foundation.md §2).
//
// Both passes draw a fullscreen triangle and copy texels; the colour maths
// lives in the TEXTURE FORMATS, which is the whole trick:
//  - linearise pass: source view is Rgba8UnormSrgb, so hardware decodes
//    sRGB → linear on sample; the render target is Rgba16Float, so linear
//    values land in the working format untouched.
//  - display pass: source is the linear Rgba16Float; the render target is
//    Rgba8UnormSrgb, so hardware encodes linear → sRGB on write.
// One shader, zero hand-rolled gamma curves, no chance of drift between
// decode and encode — the auditable single place the design doc demands.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> VsOut {
    // One triangle covering the screen: (-1,-1) (3,-1) (-1,3).
    var out: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> view: ViewParams;

// The two viewer-only controls (docs/07-UI-SPEC.md §2.2, docs/06 §3.3).
// Neither may ever reach an export: every export path passes the neutral value,
// which this shader short-circuits on so a neutral pass is bit-identical to the
// plain copy it used to be.
struct ViewParams {
    /// 2^stops, computed host-side so the Viewer's number and the Exposure
    /// effect's multiply by the identical float. 1.0 is neutral.
    gain: f32,
    /// 0 off, 1 on. A fixed curve, no measurement, no carried state — the
    /// picture at a frame never depends on which frame preceded it.
    tone_map: u32,
    _pad0: f32,
    _pad1: f32,
};

/// Rec. 709 luminance, the working space's own primaries (docs/06 §3).
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// Where the shoulder starts. Below this the curve is the identity, exactly —
/// which is the whole promise of the control: on an ordinary composite whose
/// values never pass 1, turning it on changes nothing at all.
const KNEE: f32 = 0.8;

/// Fold everything above the knee into the room left below 1.
///
/// `knee + room * (1 - exp(-(L - knee) / room))` has slope exactly 1 at the
/// knee, so the join is smooth, and approaches 1 without ever reaching it, so
/// no highlight — however bright — clips flat. It is a rolloff, not a grade:
/// scaling RGB by the luminance ratio keeps hue and saturation where the
/// author put them.
fn tone_map_rgb(c: vec3<f32>) -> vec3<f32> {
    let l = luma(c);
    if (l <= KNEE) {
        return c;
    }
    let room = 1.0 - KNEE;
    let mapped = KNEE + room * (1.0 - exp(-(l - KNEE) / room));
    return c * (mapped / l);
}

@fragment
fn fs_copy(in: VsOut) -> @location(0) vec4<f32> {
    return shade(in);
}

// ---------------------------------------------------------------------------
// The OCIO variants (docs/impl/ocio.md §5.2).
//
// Everything below reads a BAKED TABLE and nothing else: no logarithm of the
// config's, no power of the config's, no arithmetic this shader invented. That
// is the whole design. The maths a config describes was run once on the
// processor, at the bake, and what reaches here is the answers — so the only
// thing that can disagree between the Viewer and the export is a table lookup,
// and a table lookup is +, -, *, floor and clamp on both sides.
//
// The formulation is COPIED from `lumit-colour`'s `sample.rs` and `bake.rs` and
// must stay copied: six tetrahedra in the written order, `>=` as written, ties
// breaking top-first (ocio.md §4.3 — binding).
// ---------------------------------------------------------------------------

/// The curve table, `CURVE_WIDTH` samples per row, `p.curve_rows` rows per
/// stage, both stages in one texture.
@group(1) @binding(0) var curve: texture_2d<f32>;
/// The cube, red fastest, so `(x=r, y=g, z=b)` needs no transpose.
@group(1) @binding(1) var cube: texture_3d<f32>;
@group(1) @binding(2) var<uniform> p: OcioParams;

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

/// `f32::MIN_POSITIVE` — the floor `Shaper::forward` clamps to before taking a
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

/// The colour transform is not linear, so it has to see the colour the author
/// authored: unpremultiply, transform, put it back — the same discipline the
/// tone map above and every affine grade in the effect set use. An opaque
/// frame, which is every export, takes the cheap branch.
fn ocio_shade(in: VsOut) -> vec4<f32> {
    let s = shade(in);
    if (s.a > 0.0 && s.a != 1.0) {
        return vec4<f32>(ocio_apply(s.rgb / s.a) * s.a, s.a);
    }
    return vec4<f32>(ocio_apply(s.rgb), s.a);
}

/// The input variant, and the eight-bit display variants. The display targets
/// are viewed as plain `Unorm` rather than `UnormSrgb`, because a baked view's
/// output is ALREADY display-encoded — letting the hardware encode it a second
/// time is the pale-washed-out bug ocio.md §5.2 names, and it looks exactly
/// like a subtle grading mistake rather than like a bug.
@fragment
fn fs_ocio(in: VsOut) -> @location(0) vec4<f32> {
    return ocio_shade(in);
}

/// The sixteen-bit display variant. No `srgb_encode` here — the artefact has
/// already encoded — but the same clamp a unorm write applies, so the deep
/// target and the eight-bit one carry the same values.
@fragment
fn fs_ocio16(in: VsOut) -> @location(0) vec4<f32> {
    let s = ocio_shade(in);
    return vec4<f32>(clamp(s.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), s.a);
}

/// The display pass for a SIXTEEN-bit export target.
//
// The trick at the top of this file — let the texture format do the gamma —
// has one gap: there is no sixteen-bit sRGB format for the hardware to encode
// into. So the deep display target is Rgba16Float and this applies the same
// curve the hardware applies on an Rgba8UnormSrgb write, in the one place it
// can be compared against it (`the_deep_display_agrees_with_the_eight_bit_one`
// does exactly that, to within a code). Alpha is not encoded, matching the
// hardware, and the clamp matches what a unorm write does to a value past
// full scale.
@fragment
fn fs_display16(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    return vec4<f32>(srgb_encode(s.r), srgb_encode(s.g), srgb_encode(s.b), s.a);
}

/// Linear → sRGB, the IEC 61966-2-1 transfer the hardware writes.
fn srgb_encode(c: f32) -> f32 {
    let v = clamp(c, 0.0, 1.0);
    if (v <= 0.0031308) {
        return v * 12.92;
    }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}

/// What both fragment entry points draw, before any encoding.
fn shade(in: VsOut) -> vec4<f32> {
    let s = textureSample(src, samp, in.uv);
    // The neutral point, bit-exact: the linearise pass and every export take
    // this branch, so they are the copy they always were.
    if (view.gain == 1.0 && view.tone_map == 0u) {
        return s;
    }
    // A scene-linear gain is a scalar, so premultiplied alpha rides through it
    // untouched. The curve is not linear, so it has to see the colour
    // the author authored: unpremultiply, map, put it back.
    var rgb = s.rgb * view.gain;
    if (view.tone_map != 0u && s.a > 0.0) {
        rgb = tone_map_rgb(rgb / s.a) * s.a;
    } else if (view.tone_map != 0u) {
        rgb = tone_map_rgb(rgb);
    }
    return vec4<f32>(rgb, s.a);
}
