// Motion blur (docs/08-EFFECTS.md §3.2), Guertin-class reconstruction
// (docs/impl/optical-flow.md §4.5 item 3). Mirrors
// lumit_core::fx::cpu::motion_blur op-for-op (§1.6: the CPU is the oracle) —
// the same tile reduction, the same tap count, the same weights accumulated in
// the same order, edges clamped.
//
// The motion is a dense flow field (per-pixel forward vectors, in raster
// pixels) the decode worker computed between the current source frame and the
// next (§3.1). It arrives as an rgba32float texture the same size as the input:
// .xy are the flow vectors, .z the per-pixel confidence in 0..1
// (lumit_flow::confidence). binding 2 samples it.
//
// Bindings follow the shared three-sampled-input shape (the one Datamosh uses):
// 0 the source — also the unprocessed original for the host Mix, since this is
// a single pass — 1 the dominant-motion tiles from fx_mb_tilemax.wgsl, 2 the
// flow field, 3 the storage output, 4 the uniform.
//
// What changed from v1, and why (both are the owner's stated goals):
//
//  * v1 gathered along each pixel's *own* vector, so a fast object never smeared
//    over the background it passed — the still sky behind an aeroplane stayed
//    razor sharp against a blurred fuselage. Here each pixel also gathers along
//    the *dominant* motion of its 3x3 tile neighbourhood, and every tap is
//    weighted by whether the sample it found could have travelled here: the
//    sample's own streak reaching out (cone), this pixel's streak reaching in
//    (cone), and a cylinder term for the ordinary case where the two agree,
//    which is what keeps uniform motion integrating like the box a shutter is.
//
//  * v1 multiplied the streak by confidence, so an uncertain pixel collapsed to
//    no blur and read as a frozen speck amid motion. Here confidence *blends*
//    between the pixel's own vector and the borrowed dominant one at
//    MB_DOM_TEMPER length: low confidence borrows its neighbourhood's motion
//    rather than freezing. Zero blur survives only where the tile itself is
//    still, in which case every tap lands on the pixel and the result is the
//    bit-exact input.

const DOM_TEMPER: f32 = 0.6; // lumit_core::fx::cpu::MB_DOM_TEMPER

struct Params {
    shutter_frac: f32, // shutter / 360: streak length as a fraction of motion
    samples: i32,      // the *cap* on taps; the count adapts to the streak
    mix_amt: f32,      // 0..1, blended against the unprocessed input
    view: i32,         // 0 Rendered, 1 Motion vectors, 2 Confidence, 3 Dominant motion
    tile: i32,         // MB_TILE: tile side in pixels
    quality: i32,      // 0 Normal, 1 High (curved trails, half the tap spacing)
    matte_on: f32,     // 1 = the matte scales Shutter angle per pixel
    pad1: i32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var tiles: texture_2d<f32>;
@group(0) @binding(2) var flow: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<uniform> p: Params;

// The Matte (docs/08 §2.6) on this kernel's own layout, read only
// under `matte_on` — bound to `src` when there is none, since a texture
// binding cannot be left empty.
@group(0) @binding(5) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// Clamp-addressed bilinear at continuous pixel-centre coordinates (== the
// cpu::bilinear rule the reference uses, same arithmetic order): the texel at
// index x covers [x, x+1), centre x+0.5; out-of-frame taps read the edge.
fn bilinear_clamp(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c10 = textureLoad(src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c01 = textureLoad(src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let c11 = textureLoad(src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

// The same rule over the flow field, returning (u, v, conf) — the CPU oracle's
// bilinear_uv and bilinear_scalar read in this identical order.
fn bilinear_flow3(sx: f32, sy: f32, size: vec2<i32>) -> vec3<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(flow, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0).xyz;
    let c10 = textureLoad(flow, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0).xyz;
    let c01 = textureLoad(flow, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0).xyz;
    let c11 = textureLoad(flow, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0).xyz;
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

// The highest-scoring tile of the 3x3 neighbourhood, clamped at the frame edge
// (a border tile simply reads itself more than once) — Guertin's neighbour-max.
// A tile only knows about motion inside itself, and an object one tile away is
// exactly the thing whose smear should reach in.
//
// This answers "which way might something have flown into me", and an extremum
// is the right summary for that: the point is to catch the fast thing. It is
// NOT what an uncertain pixel borrows — see tile_bilinear.
fn neighbour_max(t: vec2<i32>, tdim: vec2<i32>) -> vec2<f32> {
    var dom = vec2<f32>(0.0);
    var best = -1.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let n = clamp(t + vec2<i32>(dx, dy), vec2<i32>(0), tdim - vec2<i32>(1));
            let c = textureLoad(tiles, n, 0);
            if (c.z > best) {
                best = c.z;
                dom = c.xy;
            }
        }
    }
    return dom;
}

// The motion an uncertain pixel BORROWS: the tile field sampled bilinearly
// between tile centres, rather than one tile's winning vector. The twin of
// lumit_core::fx::cpu::tile_bilinear, which carries the full reasoning.
//
// Short version: borrowing and scattering want different summaries. An extremum
// is right for "what is the fastest thing near me" and badly wrong for "what is
// my neighbourhood doing" — it is the single most unusual vector out of 256,
// picked exactly where the measurement is least trustworthy, so neighbouring
// tiles win unrelated wild vectors and the blur comes out in rectangular
// patches of different angles (measured on a fast zoom over cel animation).
// Interpolating makes the borrowed direction continuous, and makes disagreement
// cancel: tiles that agree reinforce, tiles that point at random average toward
// zero, so with no consensus the blur quietly backs off instead of inventing one.
fn tile_bilinear(pos: vec2<f32>, tdim: vec2<i32>) -> vec2<f32> {
    // Tile (tx, ty) speaks for the pixel at its centre, ((tx + 0.5) * tile).
    let f = pos / f32(p.tile) - vec2<f32>(0.5);
    let f0 = floor(f);
    let t = f - f0;
    let i0 = vec2<i32>(f0);
    let hi = tdim - vec2<i32>(1);
    let c00 = textureLoad(tiles, clamp(i0, vec2<i32>(0), hi), 0).xy;
    let c10 = textureLoad(tiles, clamp(i0 + vec2<i32>(1, 0), vec2<i32>(0), hi), 0).xy;
    let c01 = textureLoad(tiles, clamp(i0 + vec2<i32>(0, 1), vec2<i32>(0), hi), 0).xy;
    let c11 = textureLoad(tiles, clamp(i0 + vec2<i32>(1, 1), vec2<i32>(0), hi), 0).xy;
    let top = c00 * (1.0 - t.x) + c10 * t.x;
    let bottom = c01 * (1.0 - t.x) + c11 * t.x;
    return top * (1.0 - t.y) + bottom * t.y;
}

// The confidence blend — applied for this pixel, and again at every tap to
// learn that sample's reach, so both sides of the weighting speak the same
// language about how far a thing moves. `lend` is the borrowed motion, never
// the neighbour-max.
// `sf` is this pixel's shutter fraction, which the matte scales.
fn blended(uv: vec2<f32>, c: f32, lend: vec2<f32>, sf: f32) -> vec2<f32> {
    let cc = clamp(c, 0.0, 1.0);
    let borrow = DOM_TEMPER * (1.0 - cc);
    return lend * sf * borrow + uv * sf * cc;
}

// Spelled out rather than `length()` so the arithmetic is literally the CPU
// oracle's `sqrt(x*x + y*y)` — a driver is free to lower `length` to a fused
// or reassociated form, and this kernel is judged bit-for-bit against an f32
// reference (the trap the flare bake was bitten by).
fn mb_len(v: vec2<f32>) -> f32 {
    return sqrt(v.x * v.x + v.y * v.y);
}

fn mb_cone(d: f32, l: f32) -> f32 {
    return clamp(1.0 - d / max(l, 1e-4), 0.0, 1.0);
}

fn mb_cylinder(d: f32, l: f32) -> f32 {
    let ll = max(l, 1e-4);
    let e0 = 0.95 * ll;
    let e1 = 1.05 * ll;
    let t = clamp((d - e0) / (e1 - e0), 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

@compute @workgroup_size(8, 8)
fn motion_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    let fl = textureLoad(flow, xy, 0);
    let conf = fl.z;
    let tdim = vec2<i32>(textureDimensions(tiles));
    // Two summaries of the neighbourhood, for two different questions: the
    // fastest thing near me (where a smear could arrive from), and what the
    // neighbourhood agrees it is doing (what to borrow).
    let dom = neighbour_max(xy / p.tile, tdim);
    let lend = tile_bilinear(pos, tdim);
    // Diagnostic views (FX-19), matching cpu::motion_blur exactly.
    if (p.view == 1) {
        // Motion vectors: red = +x, green = +y, mid-grey = still. Opaque.
        let k = 1.0 / 32.0;
        let r = clamp(0.5 + fl.x * k, 0.0, 1.0);
        let g = clamp(0.5 + fl.y * k, 0.0, 1.0);
        textureStore(dst, xy, vec4<f32>(r, g, 0.5, 1.0));
        return;
    }
    if (p.view == 2) {
        let c = clamp(conf, 0.0, 1.0);
        textureStore(dst, xy, vec4<f32>(c, c, c, 1.0));
        return;
    }
    if (p.view == 3) {
        // The *borrowed* field, not the neighbour-max: this is what an uncertain
        // pixel is actually steered by, so a picture that looks wrong and this
        // view looking wrong are the same fact. On Motion vectors' exact scale.
        let k = 1.0 / 32.0;
        let r = clamp(0.5 + lend.x * k, 0.0, 1.0);
        let g = clamp(0.5 + lend.y * k, 0.0, 1.0);
        textureStore(dst, xy, vec4<f32>(r, g, 0.5, 1.0));
        return;
    }
    // The matte scales Shutter angle per pixel, read at the destination
    // and spent everywhere the shutter is: this pixel's own vector, the
    // neighbourhood's dominant sweep, and every tap's reach. k = 1 multiplies
    // by one, so an unbound row is the unmatted picture to the bit.
    var mk = 1.0;
    if (p.matte_on != 0.0) {
        mk = matte_k(xy);
    }
    let sf = p.shutter_frac * mk;
    let sv = blended(fl.xy, conf, lend, sf);
    let dom_s = dom * sf;
    let len_sv = mb_len(sv);
    let len_dom = mb_len(dom_s);
    let spacing = select(2.0, 1.0, p.quality == 1);
    let curved = p.quality == 1;
    // Adaptive taps (§4): enough to keep them `spacing` apart over whichever of
    // the two directions reaches furthest, never more than the user's cap.
    let n = clamp(i32(ceil(max(len_sv, len_dom) / spacing)), 1, max(p.samples, 1));
    let nf = f32(n);
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var k = 0; k < n; k++) {
        let t = (f32(k) + 0.5) / nf - 0.5;
        // Guertin's two directions per tile, alternating: the neighbourhood's
        // dominant sweep, then this pixel's own.
        var dir = dom_s;
        if (k % 2 != 0) {
            dir = sv;
            if (curved) {
                // Curved trail: re-read the field halfway along and steer by
                // what is there (§4's destination-flow fixed point, per tap).
                // Only the own-direction taps bend — the dominant sweep is one
                // direction by construction.
                let m = bilinear_flow3(pos.x + 0.5 * t * sv.x, pos.y + 0.5 * t * sv.y, size);
                dir = blended(m.xy, m.z, lend, sf);
            }
        }
        let off = t * dir;
        let d = mb_len(off);
        let s = pos + off;
        // What the sample found there is moving by — the term that lets a fast
        // object reach out over a still one.
        let tf = bilinear_flow3(s.x, s.y, size);
        let len_tap = mb_len(blended(tf.xy, tf.z, lend, sf));
        let wt = mb_cone(d, len_tap)
            + mb_cone(d, len_sv)
            + 2.0 * mb_cylinder(d, len_tap) * mb_cylinder(d, len_sv);
        acc += bilinear_clamp(s.x, s.y, size) * wt;
        wsum += wt;
    }
    let o = textureLoad(src, xy, 0);
    let v = acc / wsum;
    textureStore(dst, xy, mix(o, v, p.mix_amt));
}
