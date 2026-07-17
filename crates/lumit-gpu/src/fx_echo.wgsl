// Echo / trails (docs/08 §3.13): accumulate decayed neighbour frames onto
// the current frame, then blend the resulting trail back toward the current
// frame by the host Mix. Two entries so the whole effect is a short chain of
// the shared 2-input pass (binding 0 = accumulator, 1 = the other input,
// 2 = output, 3 = uniform), one dispatch per echo tap:
//   * `echo_accumulate` folds one neighbour (scaled by its tap weight) into
//     the running accumulator, by the chosen combine mode.
//   * `echo_mix` lerps the finished accumulator toward the current frame.
// Working colour is premultiplied linear, so scaling a neighbour by its
// weight (rgb AND alpha) is the correct way to fade an echo.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var other: texture_2d<f32>;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;

struct Params {
    // Accumulate: the tap's intensity. Mix: the host Mix amount.
    weight: f32,
    // Accumulate combine: 0 = Add (sum light), 1 = Behind (accumulator over
    // the neighbour), 2 = Max (per-channel lighten). Unused by echo_mix.
    mode: u32,
    _pad0: f32,
    _pad1: f32,
}
@group(0) @binding(3) var<uniform> params: Params;

@compute @workgroup_size(8, 8)
fn echo_accumulate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let a = textureLoad(src, p, 0);
    let n = textureLoad(other, p, 0) * params.weight;
    var o: vec4<f32>;
    if (params.mode == 0u) {
        o = a + n; // Add: echoes sum light behind the leading frame
    } else if (params.mode == 1u) {
        o = a + n * (1.0 - a.a); // Behind: the accumulator composited over the echo
    } else {
        o = max(a, n); // Max: per-channel lighten
    }
    textureStore(dst, p, o);
}

@compute @workgroup_size(8, 8)
fn echo_mix(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let acc = textureLoad(src, p, 0);
    let cur = textureLoad(other, p, 0);
    textureStore(dst, p, mix(cur, acc, params.weight));
}
