// Linear wipe (docs/08-EFFECTS.md §3.46): a straight edge swept across the
// frame, everything behind it taken away. Mirrors
// lumit_core::fx::cpu::linear_wipe and ::linear_wipe_keep op-for-op (§1.6: the
// CPU is the oracle).
//
// The sweep direction arrives as a host-computed (sin, −cos) pair — this kernel
// runs no trigonometry (§1.6). The frame's extent along that direction is
// computed here rather than host-side, because it is a function of the raster
// the kernel already knows (fx_tile.wgsl's arrangement).
//
// Mix 0 and Completion 0 are both the bit-exact identity.

struct Params {
    centre_normal: vec4<f32>,  // xy = centre (raster px), zw = (sin θ, −cos θ)
    completion: f32,           // 0..1
    band: f32,                 // the feather's width in raster px, floored above zero
    mix_amt: f32,              // 0..1, blended against the unprocessed input
    _pad0: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var orig: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> p: Params;

@compute @workgroup_size(8, 8)
fn linear_wipe(@builtin(global_invocation_id) gid: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(gid.xy);
    if (xy.x >= size.x || xy.y >= size.y) {
        return;
    }
    let o = textureLoad(src, xy, 0);
    let px = f32(xy.x) + 0.5;
    let py = f32(xy.y) + 0.5;
    let d = (px - p.centre_normal.x) * p.centre_normal.z
          + (py - p.centre_normal.y) * p.centre_normal.w;
    // Half the frame's reach along the sweep direction.
    let extent = 0.5 * (abs(f32(size.x) * p.centre_normal.z)
                      + abs(f32(size.y) * p.centre_normal.w));
    // The edge travels half a feather PAST each end, so Completion 0 keeps the
    // whole frame exactly and 100 removes it exactly.
    let edge = -(extent + p.band * 0.5) + p.completion * (2.0 * extent + p.band);
    let keep = clamp((d - edge) / p.band + 0.5, 0.0, 1.0);
    // `1 − mix·(1 − keep)`, not `(1−mix) + keep·mix`: the second form rounds
    // twice and a fully kept pixel would stop scaling by exactly 1 below full
    // Mix.
    textureStore(dst, xy, o * (1.0 - p.mix_amt * (1.0 - keep)));
}
