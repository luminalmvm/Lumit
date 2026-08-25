// The texture entry point to the pyramid (docs/impl/optical-flow.md §1).
//
// In plain terms: everything else in this crate starts from a luma image that
// arrived from the CPU. A composite never does — an adjustment layer's picture
// and a Precomp's picture exist only as textures on the card, and reading one
// back to make a grey copy of it would cost more than the whole measurement.
// This kernel makes that grey copy where the picture already is: one texel in,
// one f32 out, straight into the buffer level 0 of the pyramid reads.
//
// It mirrors the CPU `to_gray` exactly in what it computes — BT.709 luma of
// *encoded* values, because correlation happens on perceptual numbers — with
// the one extra step a texture needs: the working space is scene-linear, so
// each channel is put back through the sRGB transfer first. When `d > 1` it
// also box-averages a `d × d` block per output texel, which is the repeated
// halving `flow_grays` does on the CPU, done in one pass.

struct LumaParams {
    // Output (working) dimensions.
    w: u32,
    h: u32,
    // Source texture dimensions.
    sw: u32,
    sh: u32,
    // Box factor: source texels per output texel, each way.
    d: u32,
    pad0: u32,
    pad1: u32,
    pad2: u32,
};

@group(0) @binding(0) var<uniform> p: LumaParams;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var<storage, read_write> out: array<f32>;

/// Scene-linear to sRGB-encoded, the inverse of the transfer the pictures were
/// linearised through on the way in.
fn encode(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    if (x <= 0.0031308) {
        return x * 12.92;
    }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}

@compute @workgroup_size(8, 8)
fn to_luma(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= p.w || gid.y >= p.h) {
        return;
    }
    let d = max(p.d, 1u);
    var acc = 0.0;
    for (var j = 0u; j < d; j = j + 1u) {
        for (var i = 0u; i < d; i = i + 1u) {
            let x = min(gid.x * d + i, p.sw - 1u);
            let y = min(gid.y * d + j, p.sh - 1u);
            // Premultiplied, deliberately: a composite carries its own alpha,
            // and un-premultiplying would make a transparent region's colour
            // noise into motion the scene does not have.
            let c = textureLoad(src, vec2<i32>(i32(x), i32(y)), 0);
            acc = acc + 0.2126 * encode(c.r) + 0.7152 * encode(c.g) + 0.0722 * encode(c.b);
        }
    }
    out[gid.y * p.w + gid.x] = acc / f32(d * d);
}
