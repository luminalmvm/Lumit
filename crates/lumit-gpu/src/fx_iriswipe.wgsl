// Iris wipe (docs/08-EFFECTS.md §3.71): a polygon or a star opened out of the
// middle. Mirrors lumit_core::fx::cpu::iris_wipe and ::iris_wipe_keep op-for-op
// (§1.6: the CPU is the oracle).
//
// The polygon is never rasterised. A polygon and a star are both one wedge
// repeated round a circle, so the pixel's angle is folded into one wedge and
// mirrored about its bisector — and the whole boundary becomes the single
// straight edge the host already solved. The distance to it is one dot product,
// and it is a TRUE perpendicular distance in pixels, which is what lets Feather
// be a width rather than an angle.
//
// The fold uses floor(x + 0.5) and NOT round(), for fx_radialwipe.wgsl's reason.
//
// One atan2 a pixel — §3.47's admission again (K-399): the angle IS a function
// of the pixel and cannot be lifted host-side, so the oracle is judged on
// absolute difference rather than in fp16 ULPs.
//
// Mix 0 and Outer radius 0 are both the bit-exact identity.

const PI: f32 = 3.14159265358979323846;

struct Params {
    centre: vec2<f32>,   // raster px
    vertex: vec2<f32>,   // the sector's first vertex, radius along +x
    normal: vec2<f32>,   // the outward unit normal of the edge leaving it
    period: f32,         // one sector, radians
    rotation: f32,       // radians, from straight up, clockwise
    band: f32,           // the feather's width, raster px, floored above zero
    has_shape: f32,      // 0 when Outer radius is 0 — there is no polygon
                         // (`active` is a WGSL reserved keyword, K-405's `target` again)
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    matte_on: f32,       // 1 = the matte scales the polygon's radius per pixel (K-429)
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (K-395, docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl, K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// A control pulled toward its neutral by k (== cpu::matte_toward), spelled out
// rather than `mix()` so that k = 1 is the value to the bit.
fn matte_toward(value: f32, neutral: f32, k: f32) -> f32 {
    return neutral * (1.0 - k) + value * k;
}

@compute @workgroup_size(8, 8)
fn iris_wipe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let dx = f32(xy.x) + 0.5 - p.centre.x;
    let dy = f32(xy.y) + 0.5 - p.centre.y;
    // From straight up, clockwise, on a raster whose y grows downward, then
    // de-rotated so the sector's first vertex sits on the +x axis.
    let phi = atan2(dy, dx) + PI * 0.5 - p.rotation;
    let r = sqrt(dx * dx + dy * dy);
    let a = abs(phi - p.period * floor(phi / p.period + 0.5));
    let point = vec2<f32>(r * cos(a), r * sin(a));
    // The matte scales the whole polygon about its centre (K-429): the solved
    // vertex is the only place a radius survives into this expression, so one
    // multiply moves the outer and inner radii together and leaves the edge's
    // direction — and so the normal — alone. k = 1 multiplies by one.
    var k = 1.0;
    if (p.matte_on != 0.0) {
        k = matte_k(xy);
    }
    let dist = (point.x - p.vertex.x * k) * p.normal.x
             + (point.y - p.vertex.y * k) * p.normal.y;
    var keep = clamp(dist / p.band + 0.5, 0.0, 1.0);
    // No polygon, no edge: the frame passes through untouched (§3.71's fifth
    // note). A polygon the matte has scaled to nothing is the same identity,
    // and the same exact one Outer radius 0 is. A uniform test either way.
    keep = select(1.0, keep, p.has_shape > 0.5 && k > 0.0);
    textureStore(dst, xy, o * (1.0 - p.mix_amt * (1.0 - keep)));
}
