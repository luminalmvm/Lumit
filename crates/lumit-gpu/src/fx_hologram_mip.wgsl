// Hologram's halved copies: each texel the mean of the four below it, so the
// replay can read the layer and the Backdrop layer blurred by any amount.

@group(0) @binding(0) var mip_in: texture_2d<f32>;
@group(0) @binding(1) var mip_out: texture_storage_2d<rgba16float, write>;

@compute @workgroup_size(8, 8)
fn hologram_mip(@builtin(global_invocation_id) gid: vec3<u32>) {
    let od = textureDimensions(mip_out);
    if (gid.x >= od.x || gid.y >= od.y) {
        return;
    }
    let hi = vec2<i32>(textureDimensions(mip_in)) - vec2<i32>(1);
    let base = vec2<i32>(gid.xy) * 2;
    let a = textureLoad(mip_in, min(base, hi), 0);
    let b = textureLoad(mip_in, min(base + vec2<i32>(1, 0), hi), 0);
    let c = textureLoad(mip_in, min(base + vec2<i32>(0, 1), hi), 0);
    let d = textureLoad(mip_in, min(base + vec2<i32>(1, 1), hi), 0);
    textureStore(mip_out, vec2<i32>(gid.xy), 0.25 * (a + b + c + d));
}
