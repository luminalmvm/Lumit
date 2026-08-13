// Lens flare additive raster (docs/08 §3.27, K-366): one instanced quad per
// SPLAT, plain additive blend into the fp16 flare buffer.
//
// In plain terms: build_splats has already decided where each traced ray
// landed, how far its light spreads (the two half-axes of its footprint) and
// how bright the middle of that spread is. All this pass does is draw the
// footprint as a quad and fade the light off toward its edges — a tent,
// `(1−|u|)(1−|v|)` over the parallelogram, which integrates to exactly the
// flux the ray carried.
//
// **What is gone, and why nothing replaced it.** Through K-353 the raster
// drew the pupil grid as connected quads, and a caustic fold turned those
// quads into slivers thinner than a pixel — so the pipeline grew widening,
// analytic sample coverage, sub-pixel inflation and sliver parking to stop
// the fold's light disappearing. A tent is continuous and each ray is
// independent: a fold is simply many splats landing on top of one another,
// which is the integral the effect wanted all along. There is no coverage to
// compute and no geometry to rescue.

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

@group(0) @binding(0) var<storage, read> splats: array<Splat>;

struct DrawDims {
    raster: vec2<f32>,
    pad: vec2<f32>,
};
// The flare buffer's size — splats are in its pixels, and this is what maps
// them to clip space.
@group(0) @binding(1) var<uniform> dims: DrawDims;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) rgb: vec3<f32>,
    // The footprint's own coordinates, ±1 at the quad's corners — the tent's
    // argument, interpolated by the rasteriser at each pixel centre exactly
    // as the CPU twin solves for it with the inverse 2×2.
    @location(1) uv: vec2<f32>,
};

// The six vertices of the quad, as (u, v): triangles (0,1,2) and (0,2,3)
// round the corners (−1,−1), (1,−1), (1,1), (−1,1).
fn corner_uv(k: u32) -> vec2<f32> {
    if (k == 1u) {
        return vec2<f32>(1.0, -1.0);
    }
    if (k == 2u || k == 4u) {
        return vec2<f32>(1.0, 1.0);
    }
    if (k == 5u) {
        return vec2<f32>(-1.0, 1.0);
    }
    return vec2<f32>(-1.0, -1.0);
}

@vertex
fn vs_flare(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let s = splats[ii];
    let uv = corner_uv(vi % 6u);
    var out: VsOut;
    out.uv = uv;
    out.rgb = vec3<f32>(s.r, s.g, s.b);
    if (s.live < 0.5) {
        // A dead or unlit ray: every vertex off-screen and coincident, so
        // the rasteriser makes no fragment at all.
        out.pos = vec4<f32>(2.0, 2.0, 0.0, 1.0);
        return out;
    }
    // The quad reaches a FULL grid step each way — twice the half-axes
    // (K-373). At K-366's single half-axis, neighbouring tents met exactly
    // where both had fallen to zero, so the reconstruction printed a woven
    // grid of dark seams at the ray spacing over every ghost. A linear tent
    // partitions unity only when its support is twice the sample spacing.
    let px = vec2<f32>(s.cx, s.cy)
        + vec2<f32>(s.a1x, s.a1y) * uv.x * 2.0
        + vec2<f32>(s.a2x, s.a2y) * uv.y * 2.0;
    out.pos = vec4<f32>(
        px.x / dims.raster.x * 2.0 - 1.0,
        1.0 - px.y / dims.raster.y * 2.0,
        0.0,
        1.0,
    );
    return out;
}

@fragment
fn fs_flare(in: VsOut) -> @location(0) vec4<f32> {
    // The separable tent, lumit_core `splat_ray`'s kernel. Zero at the
    // quad's edge — and since the quad now spans a full grid step each way
    // (K-373), that edge sits on the NEXT ray along, where its own tent is at
    // full height. The two overlap and sum to one, which is what makes the
    // reconstruction seamless rather than merely continuous.
    let k = (1.0 - abs(in.uv.x)) * (1.0 - abs(in.uv.y));
    let rgb = in.rgb * max(k, 0.0);
    let luma = 0.2126 * rgb.x + 0.7152 * rgb.y + 0.0722 * rgb.z;
    return vec4<f32>(rgb, luma);
}
