// The RGB split's Wavelength mode (docs/08-EFFECTS.md §3.6, K-090 quality
// tier; chromatic aberration's own Wavelength mode, K-144): wavelength-based
// dispersion. Mirrors lumit_core::fx::cpu::spectral_split op-for-op (§1.6:
// the CPU is the oracle): `count` spectral taps spread across ±offset, each
// carrying its rgb weight (xyz) and its offset fraction in [-1, +1] (the w
// lane), weighted and summed. The tap table arrives in the uniform,
// host-supplied from lumit_core::fx::spectral_basis_uniform — the kernel
// reads the very numbers the CPU reference does, exactly as the offset's
// cos/sin arrive host-computed. The colour columns are normalised, so a
// uniform image passes through unchanged; alpha follows the green channel's
// rule and stays put (§3.6), so mattes never fringe. More taps fill the same
// span more densely, so a large offset disperses smoothly. The classic
// three-channel mode is a separate kernel (fx_rgbsplit.wgsl), untouched.

struct Params {
    basis: array<vec4<f32>, 64>,  // per tap: rgb weight, w = offset fraction
    dx: f32,        // linear-mode offset, raster px (host-computed)
    dy: f32,
    amount: f32,    // radial-mode peak offset, raster px
    radial: u32,    // 1 = offsets grow from the frame centre
    count: u32,     // number of active taps (2..=64)
    mix_amt: f32,   // 0..1, blended against the unprocessed input
    matte_on: f32,  // 1 = the matte scales Amount (K-427)
    _pad1: f32,
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

// Clamp-addressed bilinear sample at continuous pixel-centre coordinates
// (== cpu::bilinear, same arithmetic order — and the classic kernel's).
fn bilinear(sx: f32, sy: f32, size: vec2<i32>) -> vec4<f32> {
    let fx = sx - 0.5;
    let fy = sy - 0.5;
    let x0 = floor(fx);
    let y0 = floor(fy);
    let tx = fx - x0;
    let ty = fy - y0;
    let x0i = i32(x0);
    let y0i = i32(y0);
    let c00 = textureLoad(
        src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c10 = textureLoad(
        src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i, 0, size.y - 1)), 0);
    let c01 = textureLoad(
        src, vec2<i32>(clamp(x0i, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let c11 = textureLoad(
        src, vec2<i32>(clamp(x0i + 1, 0, size.x - 1), clamp(y0i + 1, 0, size.y - 1)), 0);
    let top = c00 * (1.0 - tx) + c10 * tx;
    let bottom = c01 * (1.0 - tx) + c11 * tx;
    return top * (1.0 - ty) + bottom * ty;
}

@compute @workgroup_size(8, 8)
fn spectral_split(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let pos = vec2<f32>(xy) + vec2<f32>(0.5);
    var off = vec2<f32>(p.dx, p.dy);
    if (p.radial == 1u) {
        let fsize = vec2<f32>(size);
        let diag = sqrt(fsize.x * fsize.x + fsize.y * fsize.y);
        let k = p.amount / (0.5 * diag);
        off = vec2<f32>((pos.x - fsize.x * 0.5) * k, (pos.y - fsize.y * 0.5) * k);
    }
    // The matte scales Amount per pixel (K-427, == cpu::spectral_split_matted).
    if (p.matte_on != 0.0) {
        off = off * matte_k(xy);
    }
    let o = textureLoad(src, xy, 0);
    var acc = vec3<f32>(0.0);
    for (var i = 0u; i < p.count; i++) {
        // The tap's offset fraction rides in the w lane (host-computed, so no
        // WGSL division re-derives it), the rgb weight in xyz.
        let t = p.basis[i].w;
        let s = bilinear(pos.x + t * off.x, pos.y + t * off.y, size);
        acc = acc + p.basis[i].rgb * s.rgb;
    }
    let split = vec4<f32>(acc, o.a);
    let outv = o * (1.0 - p.mix_amt) + split * p.mix_amt;
    textureStore(dst, xy, outv);
}
