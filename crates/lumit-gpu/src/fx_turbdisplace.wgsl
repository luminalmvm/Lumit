// Turbulent displace (docs/08-EFFECTS.md §3.38): the fractal-driven warp.
// Mirrors lumit_core::fx::cpu::turbulent_displace op-for-op (§1.6: the CPU is
// the oracle). The noise itself comes from fx_noise_core.wgsl, prepended to this
// file at compile time — the same twin fx_fractal_noise.wgsl reads, which is why
// a Fractal noise and a Turbulent displace at the same settings line up.
//
// One of the effects that claim the generic Matte inside their own maths
// (K-395): the matte's luma multiplies the DISPLACEMENT, so a grey matte warps
// the picture less rather than showing a warped copy over an unwarped one. With
// `matte_on == 0` the vector is used exactly as the field gave it, and the
// result is byte-for-byte the no-matte pass.

struct Params {
    offset_axes: vec4<f32>,   // xy = field origin (raster px), zw = axis multipliers
    pin_amount: vec4<f32>,    // xy = per-axis pin flags, z = Amount (raster px), w = 1 ÷ |Amount|
    inv_size: f32,            // 1 ÷ Size, raster pixels
    z: f32,                   // depth coordinate
    gain: f32,
    lacunarity: f32,
    seed_x: u32,
    seed_y: u32,
    octaves: u32,             // 1..10
    cycle: i32,               // depth loop length in cells; 0 = no loop
    mix_amt: f32,             // 0..1, blended against the unprocessed input
    matte_on: f32,            // 1 = scale the displacement by the matte's luma
    _pad1: f32,               // was Invert; applied once at the seam since K-425
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;
// Bound to `src` when there is no matte and gated by `matte_on` — a texture
// binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// == cpu::matte_strength / fx_blur.wgsl's matte_k: premultiplied Rec. 709 luma,
// clamped. One reading of "how much matte is here"; the Channel pick and
// Invert happened once already, in fx_matte_prepare.wgsl (K-425).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

// == cpu::bilinear_edge with the Repeat policy (edge == 1), which is the only
// one this effect uses: the pin ramp already stops a pinned edge being reached
// from outside, and an unpinned one holds its border pixel rather than fading.
fn tap(x: i32, y: i32, size: vec2<i32>) -> vec4<f32> {
    return textureLoad(src, vec2<i32>(clamp(x, 0, size.x - 1), clamp(y, 0, size.y - 1)), 0);
}

fn bilinear_repeat(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = tap(x0i, y0i, size);
    let c10 = tap(x0i + 1, y0i, size);
    let c01 = tap(x0i, y0i + 1, size);
    let c11 = tap(x0i + 1, y0i + 1, size);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn turbulent_displace(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let qx = (px - p.offset_axes.x) * p.inv_size;
    let qy = (py - p.offset_axes.y) * p.inv_size;
    let field_x = FractalField(p.seed_x, p.octaves, p.gain, p.lacunarity, 3u, p.cycle);
    let field_y = FractalField(p.seed_y, p.octaves, p.gain, p.lacunarity, 3u, p.cycle);
    let nx = nc_fractal(field_x, qx, qy, p.z);
    let ny = nc_fractal(field_y, qx, qy, p.z);

    // Pinning ramps the WHOLE vector to zero across the last |Amount| pixels
    // before a pinned edge, measured to the OUTERMOST PIXEL CENTRE so the border
    // row is exactly still. A lerp toward 1 rather than a branch, matching the
    // CPU reference's arithmetic.
    let fw = f32(size.x);
    let fh = f32(size.y);
    let ramp_x = clamp(min(px - 0.5, fw - 0.5 - px) * p.pin_amount.w, 0.0, 1.0);
    let ramp_y = clamp(min(py - 0.5, fh - 0.5 - py) * p.pin_amount.w, 0.0, 1.0);
    let pin_x = 1.0 + p.pin_amount.x * (ramp_x - 1.0);
    let pin_y = 1.0 + p.pin_amount.y * (ramp_y - 1.0);

    var k = 1.0;
    if (p.matte_on != 0.0) {
        k = matte_k(xy);
    }
    let s = p.pin_amount.z * pin_x * pin_y * k;
    let v = bilinear_repeat(px + nx * p.offset_axes.z * s,
                            py + ny * p.offset_axes.w * s,
                            size);
    textureStore(dst, xy, o * (1.0 - p.mix_amt) + v * p.mix_amt);
}
