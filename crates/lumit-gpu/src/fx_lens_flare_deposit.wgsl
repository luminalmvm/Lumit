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
// So the sum is accumulated in **f32**, in a storage buffer, and written to the
// fp16 texture once at the end. One rounding instead of thousands. The texture
// stays fp16 — a single stored value has precision to spare; it was only ever
// the accumulation that was short.
//
// WGSL has no float atomics, so the add is the standard compare-and-swap loop
// over the f32's bit pattern. That is exact f32 addition and needs no device
// feature, which is what makes it portable — `Rgba32Float` blending would have
// needed `FLOAT32_BLENDABLE`, which is not universally available and would have
// made the picture differ by machine (the determinism K-353 fought for).
//
// `deposit` mirrors `lumit_core::fx::lens_flare::splat_ray` op for op, and is
// now a closer twin than the raster ever was: same bounding box, same inverse
// 2x2, same tent, same order of operations.

// One ray's footprint, as `build_splats` left it: centre and half-axes in
// flare-buffer pixels, and the peak colour with the density cap and the tent's
// normalisation already folded in.
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
    // The flare buffer's size in pixels.
    raster: vec2<u32>,
    // How many splats this dispatch covers, from zero.
    splat_count: u32,
    _pad: u32,
};

@group(0) @binding(0) var<storage, read> splats: array<Splat>;
// The f32 accumulator, three channels per pixel, held as bit patterns because
// WGSL atomics are integer-only. Laid out `(y * w + x) * 3 + channel`.
@group(0) @binding(1) var<storage, read_write> accum: array<atomic<u32>>;
@group(0) @binding(2) var<uniform> dims: Dims;
@group(0) @binding(3) var out_tex: texture_storage_2d<rgba16float, write>;

// Exact f32 add into an integer atomic, by compare-and-swap on the bit
// pattern. The loop retries only when another thread won the slot in between,
// which in a caustic — where a great many splats land on one pixel — is the
// price of getting the sum right.
fn add_f32(idx: u32, v: f32) {
    // Nothing to add, and never a NaN: a NaN would never compare equal and the
    // loop would not terminate.
    if (!(v != 0.0) || v != v) {
        return;
    }
    var old = atomicLoad(&accum[idx]);
    loop {
        let sum = bitcast<f32>(old) + v;
        let res = atomicCompareExchangeWeak(&accum[idx], old, bitcast<u32>(sum));
        if (res.exchanged) {
            break;
        }
        old = res.old_value;
    }
}

@compute @workgroup_size(64)
fn deposit(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= dims.splat_count) {
        return;
    }
    let s = splats[i];
    if (s.live < 0.5) {
        return;
    }
    let a1 = vec2<f32>(s.a1x, s.a1y);
    let a2 = vec2<f32>(s.a2x, s.a2y);
    let centre = vec2<f32>(s.cx, s.cy);
    let det = a1.x * a2.y - a1.y * a2.x;
    if (abs(det) < 1e-12) {
        return;
    }
    let inv_det = 1.0 / det;
    let peak = vec3<f32>(s.r, s.g, s.b);

    // The tent reaches a full grid step — two half-axes — each way (K-373).
    let ext = 2.0 * (abs(a1) + abs(a2));
    let w = f32(dims.raster.x);
    let h = f32(dims.raster.y);
    let x0 = i32(max(floor(centre.x - ext.x), 0.0));
    let x1 = i32(min(ceil(centre.x + ext.x), w - 1.0));
    let y0 = i32(max(floor(centre.y - ext.y), 0.0));
    let y1 = i32(min(ceil(centre.y + ext.y), h - 1.0));
    if (x1 < x0 || y1 < y0) {
        return;
    }

    for (var py = y0; py <= y1; py = py + 1) {
        for (var px = x0; px <= x1; px = px + 1) {
            let d = vec2<f32>(f32(px) + 0.5 - centre.x, f32(py) + 0.5 - centre.y);
            // (u, v) in the parallelogram's own frame: solve [a1 a2](u,v)^T = d.
            let u = (d.x * a2.y - d.y * a2.x) * inv_det;
            let v = (d.y * a1.x - d.x * a1.y) * inv_det;
            if (abs(u) >= 2.0 || abs(v) >= 2.0) {
                continue;
            }
            let k = (1.0 - abs(u) * 0.5) * (1.0 - abs(v) * 0.5);
            let base = (u32(py) * dims.raster.x + u32(px)) * 3u;
            add_f32(base, peak.x * k);
            add_f32(base + 1u, peak.y * k);
            add_f32(base + 2u, peak.z * k);
        }
    }
}

// Write the finished sum into the flare texture: the one place the value meets
// fp16, and the alpha the combine reads is the luma of what landed.
@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= dims.raster.x || gid.y >= dims.raster.y) {
        return;
    }
    let base = (gid.y * dims.raster.x + gid.x) * 3u;
    let rgb = vec3<f32>(
        bitcast<f32>(atomicLoad(&accum[base])),
        bitcast<f32>(atomicLoad(&accum[base + 1u])),
        bitcast<f32>(atomicLoad(&accum[base + 2u])),
    );
    let luma = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
    textureStore(out_tex, vec2<i32>(gid.xy), vec4<f32>(rgb, luma));
}
