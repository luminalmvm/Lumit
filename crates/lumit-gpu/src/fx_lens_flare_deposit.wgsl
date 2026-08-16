// The lens flare's splat deposit and resolve (K-375).
//
// # Why this is a compute pass and not the blender
//
// Every ray's flux is deposited over a small footprint, and a bright pixel
// takes contributions from thousands of them. Until K-375 that accumulation
// was done by the raster blender, additively, straight into the flare buffer —
// which is `WORKING_FORMAT`, `Rgba16Float`. Adding a small increment to a large
// fp16 running sum loses anything below half an ULP of the sum, and that is a
// *systematic* loss rather than jitter that cancels: the brighter the pixel,
// the more of each further contribution disappears. Measured against the f32
// CPU reference the middle of the frame came out 4.5% dim, growing with the
// number of contributions per pixel.
//
// So the sum is accumulated at far higher precision, in a storage buffer, and
// written to the fp16 texture once at the end — one rounding instead of
// thousands. The texture stays fp16: a single stored value has precision to
// spare, and it was only ever the accumulation that was short.
//
// # Why the sum is fixed point, and not f32
//
// WGSL has no float atomics. The obvious substitute is a compare-and-swap loop
// over the f32's bit pattern, and it is exact per add — but the *order* of the
// adds is whatever the threads race to, and float addition is not associative,
// so the same document renders two different pictures. K-353 exists precisely
// to stop that, and CI caught it: "an area source must be bit-stable too".
//
// Integer addition IS associative and commutative, so `atomicAdd` on a u32
// gives a sum that does not depend on the order at all. The accumulator is
// therefore fixed point: every deposit is rounded to the nearest step (6e-8,
// see the scale below) and added exactly. That rounding is **unbiased**, where
// the fp16 blender's was a systematic truncation of everything under half an
// ULP of a large running sum — so this is better precision where it mattered,
// as well as reproducible, which the float version was not.
//
// Radiance is never negative (a deposit is `peak * k`, both non-negative), so
// the sign bit is spare range rather than a missing case.
//
// `Rgba32Float` blending would have been the other way to get f32 sums, and it
// needs `FLOAT32_BLENDABLE` — not universally available, so it would either
// raise the hardware floor or make the picture differ by machine, which is the
// same determinism problem from the other end.
//
// `deposit` mirrors `lumit_core::fx::lens_flare::splat_ray` op for op, and is
// now a closer twin than the raster ever was: same bounding box, same inverse
// 2x2, same kernel, same order of operations.

// One ray's footprint, as `build_splats` left it: centre and half-axes in
// flare-buffer pixels, and the peak colour with the density cap and the
// kernel's normalisation already folded in.
struct Splat {
    cx: f32,
    cy: f32,
    a1x: f32,
    a1y: f32,
    a2x: f32,
    a2y: f32,
    r: f32,
    g: f32,
    b: f32,
    live: f32,
    _pad0: f32,
    _pad1: f32,
};

struct Dims {
    // The flare buffer's width and height in pixels, how many splats this
    // dispatch covers, and how many pyramid levels the accumulator holds
    // (K-380). No level table: FXC cannot dynamically index a uniform
    // array without unrolling every loop that touches it (it refused), and
    // none is needed — level l is `ceil(raster / 2^l)` on both axes, which
    // `level_dim` computes below, exactly as the Rust twin sizes the
    // buffer (iterated ceil-halving and the closed form agree).
    head: vec4<u32>,
};

// One axis of pyramid level `level`: ceil(n / 2^level).
fn level_dim(n: u32, level: u32) -> u32 {
    return (n + (1u << level) - 1u) >> level;
}

// Pixel offset of level `level` in the accumulator: the sizes of every
// level before it. At most eleven iterations of plain arithmetic.
fn level_offset(level: u32) -> u32 {
    var off = 0u;
    for (var k = 0u; k < level; k = k + 1u) {
        off = off + level_dim(dims.head.x, k) * level_dim(dims.head.y, k);
    }
    return off;
}

@group(0) @binding(0) var<storage, read> splats: array<Splat>;
// The fixed-point accumulator, three channels per pixel, laid out
// `(y * w + x) * 3 + channel`.
@group(0) @binding(1) var<storage, read_write> accum: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> dims: Dims;
@group(0) @binding(3) var out_tex: texture_storage_2d<rgba16float, write>;

// Fixed-point steps in one unit of radiance. Spelled in the Rust twin too and
// pinned against this text by test.
//
// **Sized from what the buffer actually holds, not from a guess.** Measured on
// the bundled default, the flare buffer peaks at 0.042 and its median lit pixel
// is 0.0028 — the auto-exposure normalises it there (K-258's
// `TARGET_PROBE_MEAN`). K-375 first chose 2^18, whose ceiling of 16384 was four
// hundred thousand times the peak: range spent on headroom no frame will ever
// use, paid for in resolution exactly where the picture is dark and banding
// shows. 2^24 keeps a ceiling of 256 — still six thousand times the measured
// peak, and a thousand times it at four-fold intensity — while the quantum
// falls to 6e-8, which is a fifty-thousandth of the median lit pixel.
//
// **Above the ceiling the sum wraps**, not saturates: `atomicAdd` has no
// saturating form, and detecting the overflow would need a compare-and-swap,
// which is the order dependence this design exists to avoid. A test measures
// the reference's brightest pixel against the ceiling so the margin is watched
// rather than assumed.
const ACCUM_SCALE: f32 = 16777216.0;
// The clamp on ONE deposit, which is a different job: it keeps an infinity out
// of the cast, whose result would otherwise be an arbitrary integer.
const ACCUM_MAX: f32 = 4294967040.0;

// The quadratic B-spline in units of one grid step — lumit_core's `bspline_q`.
// A partition of unity like the tent, but C1: no crease where one cell meets
// the next, which is the artefact K-373's tent still left on screen (K-376).
fn bspline_q(t: f32) -> f32 {
    let a = abs(t);
    if (a <= 0.5) {
        return 0.75 - a * a;
    }
    let e = 1.5 - a;
    return 0.5 * e * e;
}

// Order-independent by construction: integer addition is associative, so the
// sum does not depend on which thread got there first.
fn add_fixed(idx: u32, v: f32) {
    // Zero, negative and NaN all leave here: `NaN > 0.0` is false, so the
    // negation catches it, and a NaN through the cast would be nonsense.
    if (!(v > 0.0)) {
        return;
    }
    let fixed = min(round(v * ACCUM_SCALE), ACCUM_MAX);
    atomicAdd(&accum[idx], u32(fixed));
}

// Widest kernel span, px, a splat deposits at full resolution (K-380) —
// past this it moves to a coarser pyramid level. The twin of lumit_core's
// DEPOSIT_SPAN_PX, pinned by test.
const DEPOSIT_SPAN_PX: f32 = 48.0;

@compute @workgroup_size(64)
fn deposit(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= dims.head.z) {
        return;
    }
    let s = splats[i];
    if (s.live < 0.5) {
        return;
    }
    var a1 = vec2<f32>(s.a1x, s.a1y);
    var a2 = vec2<f32>(s.a2x, s.a2y);
    var centre = vec2<f32>(s.cx, s.cy);
    var det = a1.x * a2.y - a1.y * a2.x;
    if (abs(det) < 1e-12) {
        return;
    }
    let peak = vec3<f32>(s.r, s.g, s.b);

    // The quadratic B-spline reaches one and a half grid steps — three
    // half-axes — each way (K-373 widened it to one step, K-376 to this).
    var ext = 3.0 * (abs(a1) + abs(a2));

    // The pyramid level this splat can afford (K-380): halve the span until
    // it fits. Exact halving, so the CPU twin picks the same level for the
    // same splat. Everything then scales into the level's pixels; the
    // kernel below is unchanged by the scale — (u, v) solve the same system
    // when both sides carry it, and `peak` is a density per level-0 pixel,
    // which the resolve's upsample reads back out at level 0.
    var span = 2.0 * max(ext.x, ext.y);
    var level = 0u;
    while (span > DEPOSIT_SPAN_PX && level + 1u < dims.head.w) {
        span = span * 0.5;
        level = level + 1u;
    }
    let s2l = f32(1u << level);
    centre = centre / s2l;
    a1 = a1 / s2l;
    a2 = a2 / s2l;
    det = det / (s2l * s2l);
    ext = ext / s2l;
    let inv_det = 1.0 / det;
    let lw = level_dim(dims.head.x, level);
    let lh = level_dim(dims.head.y, level);
    let off = level_offset(level) * 3u;

    let x0 = i32(max(floor(centre.x - ext.x), 0.0));
    let x1 = i32(min(ceil(centre.x + ext.x), f32(lw) - 1.0));
    let y0 = i32(max(floor(centre.y - ext.y), 0.0));
    let y1 = i32(min(ceil(centre.y + ext.y), f32(lh) - 1.0));
    if (x1 < x0 || y1 < y0) {
        return;
    }

    for (var py = y0; py <= y1; py = py + 1) {
        for (var px = x0; px <= x1; px = px + 1) {
            let d = vec2<f32>(f32(px) + 0.5 - centre.x, f32(py) + 0.5 - centre.y);
            // (u, v) in the parallelogram's own frame: solve [a1 a2](u,v)^T = d.
            let u = (d.x * a2.y - d.y * a2.x) * inv_det;
            let v = (d.y * a1.x - d.x * a1.y) * inv_det;
            if (abs(u) >= 3.0 || abs(v) >= 3.0) {
                continue;
            }
            let k = bspline_q(u * 0.5) * bspline_q(v * 0.5);
            let base = off + (u32(py) * lw + u32(px)) * 3u;
            add_fixed(base, peak.x * k);
            add_fixed(base + 1u, peak.y * k);
            add_fixed(base + 2u, peak.z * k);
        }
    }
}

// One fixed-point texel of one level, as f32 density — all three channels
// at once, because FXC cannot index a local vector as an l-value and would
// otherwise try (and refuse) to unroll every loop around one.
fn level_tap(off: u32, lw: u32, x: u32, y: u32) -> vec3<f32> {
    let idx = off + (y * lw + x) * 3u;
    return vec3<f32>(
        f32(atomicLoad(&accum[idx])),
        f32(atomicLoad(&accum[idx + 1u])),
        f32(atomicLoad(&accum[idx + 2u])),
    ) / ACCUM_SCALE;
}

// Write the finished sum into the flare texture: every level bilinearly
// upsampled and summed (K-380 — level 0's tap is the identity, so a frame
// whose splats all fit level 0 reads exactly as it did before the pyramid),
// back to floating point, the one place the value meets fp16, and the alpha
// the combine reads is the luma of what landed.
@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= dims.head.x || gid.y >= dims.head.y) {
        return;
    }
    var rgb = vec3<f32>(0.0);
    // The running offset walks the levels in step with the loop, so no
    // level recomputes the sizes of everything before it.
    var off_px = 0u;
    for (var level = 0u; level < dims.head.w; level = level + 1u) {
        let lw = level_dim(dims.head.x, level);
        let lh = level_dim(dims.head.y, level);
        let off = off_px * 3u;
        off_px = off_px + lw * lh;
        let s2l = f32(1u << level);
        let pos_x = max((f32(gid.x) + 0.5) / s2l - 0.5, 0.0);
        let pos_y = max((f32(gid.y) + 0.5) / s2l - 0.5, 0.0);
        let x0 = min(u32(pos_x), lw - 1u);
        let y0 = min(u32(pos_y), lh - 1u);
        let x1 = min(x0 + 1u, lw - 1u);
        let y1 = min(y0 + 1u, lh - 1u);
        let fx = clamp(pos_x - f32(x0), 0.0, 1.0);
        let fy = clamp(pos_y - f32(y0), 0.0, 1.0);
        let t00 = level_tap(off, lw, x0, y0);
        let t10 = level_tap(off, lw, x1, y0);
        let t01 = level_tap(off, lw, x0, y1);
        let t11 = level_tap(off, lw, x1, y1);
        let top = t00 + (t10 - t00) * fx;
        let bot = t01 + (t11 - t01) * fx;
        rgb = rgb + top + (bot - top) * fy;
    }
    let luma = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
    textureStore(out_tex, vec2<i32>(gid.xy), vec4<f32>(rgb, luma));
}
