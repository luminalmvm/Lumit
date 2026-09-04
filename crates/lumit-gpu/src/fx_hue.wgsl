// Hue shift (docs/08-EFFECTS.md §3.17). Mirrors lumit_core::fx::cpu::hue_shift
// op-for-op (§1.6: the CPU is the oracle): a row-major linear 3×3 colour
// matrix on RGB, alpha untouched. The matrix is computed host-side
// (lumit_core::fx::hue_matrix) so the CPU and this kernel multiply by
// identical coefficients. The nine coefficients are passed as individual f32
// fields, not a WGSL array/matrix, so their tight 4-byte packing matches the
// Rust `[f32; 9]` uniform exactly (a uniform array would stride at 16 bytes).

struct Params {
    m0: f32, m1: f32, m2: f32,
    m3: f32, m4: f32, m5: f32,
    m6: f32, m7: f32, m8: f32,
    mix_amt: f32,
    matte_on: f32,     // 1 = the matte drives the control below
    angle_rad: f32,    // the Angle, read only under a matte
    preserve: f32,     // 1 = constant-luminance matrix, 0 = plain-RGB spin
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

// The Matte (docs/08 §2.6), bound for every kernel on this layout and
// read only under `matte_on` — bound to `src` when there is none, since a
// texture binding cannot be left empty.
@group(0) @binding(4) var matte: texture_2d<f32>;

// This pixel's matte strength (== cpu::matte_strength): premultiplied Rec. 709
// luma, clamped. The Channel pick and Invert already happened, once, at the
// seam (fx_matte_prepare.wgsl).
fn matte_k(xy: vec2<i32>) -> f32 {
    let m = textureLoad(matte, xy, 0);
    return clamp(m.r * 0.2126 + m.g * 0.7152 + m.b * 0.0722, 0.0, 1.0);
}

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

@compute @workgroup_size(8, 8)
fn hue_shift(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    var m = array<f32, 9>(p.m0, p.m1, p.m2, p.m3, p.m4, p.m5, p.m6, p.m7, p.m8);
    if (p.matte_on != 0.0) {
        // The matte scales Angle toward 0 per pixel, and the matrix for that
        // angle is built here (== cpu::hue_matrix_px): a half-grey
        // matte on 90° turns the hue 45°, where a fade would desaturate it.
        let rad = p.angle_rad * matte_k(xy);
        let s = sin(rad);
        let cs = cos(rad);
        if (p.preserve != 0.0) {
            let lr = LUMA.r;
            let lg = LUMA.g;
            let lb = LUMA.b;
            m = array<f32, 9>(
                lr + cs * (1.0 - lr) - s * lr,
                lg - cs * lg - s * lg,
                lb - cs * lb + s * (1.0 - lb),
                lr - cs * lr + s * 0.143,
                lg + cs * (1.0 - lg) + s * 0.140,
                lb - cs * lb - s * 0.283,
                lr - cs * lr - s * (1.0 - lr),
                lg - cs * lg + s * lg,
                lb + cs * (1.0 - lb) + s * lb,
            );
        } else {
            let a = (1.0 - cs) / 3.0;
            let b = s / sqrt(3.0);
            m = array<f32, 9>(cs + a, a - b, a + b, a + b, cs + a, a - b, a - b, a + b, cs + a);
        }
    }
    let c = vec3<f32>(
        m[0] * o.r + m[1] * o.g + m[2] * o.b,
        m[3] * o.r + m[4] * o.g + m[5] * o.b,
        m[6] * o.r + m[7] * o.g + m[8] * o.b,
    );
    let outv = o.rgb * (1.0 - p.mix_amt) + c * p.mix_amt;
    textureStore(dst, xy, vec4<f32>(outv, o.a));
}
