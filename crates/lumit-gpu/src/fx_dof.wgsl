// Depth-of-field lens blur (foundation for docs/08-EFFECTS.md's planned DoF
// effects). A variable-radius "scatter-as-gather" blur: each output pixel's
// circle-of-confusion radius comes from how far its depth is from the focus
// plane, and it averages a disc of that radius from the source. Mirrors the
// CPU reference tap-for-tap (§1.6: the CPU is the oracle) — the same CoC
// maths, the same integer disc taps in the same row-major order, box
// weighted and normalised, edges clamped.
//
// The per-pixel depth is read from the RED channel of `depth` (docs/impl/
// layer-input.md §3): in production it is the referenced depth layer rendered
// alone in the working format (rgba16float), so depth = its red; in the §1.6
// oracle it is an exact R32Float map (same red read). Convention: 0 = near,
// 1 = far, though the effect is symmetric about Focus so either reading of the
// pass works. `depth` is the same size as the source, so `.x` at `xy` is that
// pixel's depth. binding 0 is the source (the taps sample it), binding 1 the
// unprocessed original read back for the host Mix, binding 2 the depth field —
// the shared three-sampled-input shape it borrows from Motion blur, with the
// depth texture the one extra binding over the two-input convention. Only its
// red channel is read, so any float texture (R32Float or the working rgba16f)
// binds unchanged (the bind layout is a non-filterable float sample, textureLoad
// not a sampler).

struct Params {
    focus: f32,          // in-focus depth in [0,1]
    range: f32,          // half-width of the sharp band, [0,1]
    near_aperture: f32,  // near-side (d < focus) max CoC radius, raster px
    far_aperture: f32,   // far-side (d >= focus) max CoC radius, raster px
    mix_amt: f32,        // 0..1, blended against the unprocessed input
    depth_invert: u32,   // 1 = invert the depth (d' = 1 - d) before the CoC
    display: u32,        // 0 = Rendered, 1 = Depth map, 2 = Focus map
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var depth: texture_2d<f32>;
@group(0) @binding(3) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var<uniform> p: Params;

// The smoothstep focus falloff s in [0,1]: 0 inside the sharp band
// |depth-focus| <= range, ramping to 1 as the depth distance reaches the far
// extreme (1.0). Shared by the CoC radius and the Focus-map view. Written with
// explicit min/max/mul/sub — NOT the built-in smoothstep, whose exact form is
// not guaranteed to match the CPU — so the oracle reproduces it bit-for-bit.
fn coc_falloff(d: f32) -> f32 {
    let dist = abs(d - p.focus);
    let denom = max(1.0 - p.range, 1e-4);
    let e = min(max((dist - p.range) / denom, 0.0), 1.0);
    return e * e * (3.0 - 2.0 * e); // smoothstep ramp
}

// Circle-of-confusion radius (raster px) for a depth sample: the falloff scaled
// by the per-side aperture. The near side (d < focus) uses `near_aperture`, the
// far side `far_aperture`; at d == focus the falloff is 0, so the aperture
// select never introduces a discontinuity and the §1.6 oracle holds.
fn coc_radius(d: f32) -> f32 {
    let s = coc_falloff(d);
    let ap = select(p.far_aperture, p.near_aperture, d < p.focus);
    return ap * s;
}

@compute @workgroup_size(8, 8)
fn dof(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let raw = textureLoad(depth, xy, 0).x;
    // Depth invert (swap near and far): d' = 1 - d, applied before the CoC.
    let d = select(raw, 1.0 - raw, p.depth_invert != 0u);

    // Diagnostic views (both continuous, so the §1.6 oracle covers them; they
    // ignore the disc gather and Mix and write the view directly).
    if (p.display == 1u) {
        // Depth map: the post-invert depth as opaque greyscale.
        textureStore(dst, xy, vec4<f32>(d, d, d, 1.0));
        return;
    }
    if (p.display == 2u) {
        // Focus map: 1 - s, white where sharp, darkening out of focus.
        let m = 1.0 - coc_falloff(d);
        textureStore(dst, xy, vec4<f32>(m, m, m, 1.0));
        return;
    }

    let coc = coc_radius(d);
    let coc2 = coc * coc;
    // Integer disc radius: every tap whose squared pixel distance is within
    // coc² is included, box weighted. The centre (r²=0 <= coc²>=0) is always
    // in, so the running weight is never zero.
    let ri = i32(ceil(coc));
    var acc = vec4<f32>(0.0);
    var wsum = 0.0;
    for (var dy = -ri; dy <= ri; dy++) {
        for (var dx = -ri; dx <= ri; dx++) {
            let r2 = f32(dx * dx + dy * dy);
            if (r2 <= coc2) {
                let sx = clamp(xy.x + dx, 0, size.x - 1);
                let sy = clamp(xy.y + dy, 0, size.y - 1);
                acc += textureLoad(src, vec2<i32>(sx, sy), 0);
                wsum += 1.0;
            }
        }
    }
    let v = acc / wsum;
    let o = textureLoad(orig, xy, 0);
    textureStore(dst, xy, mix(o, v, p.mix_amt));
}
