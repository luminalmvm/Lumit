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
    // Accumulate combine (FX-17, T21), mirroring cpu::echo_blend
    // op-for-op: 0 = Behind, 1 = In front, 2 = Add, 3 = Screen, 4 = Multiply,
    // 5 = Overlay, 6 = Soft light, 7 = Hard light, 8 = Lighten, 9 = Darken,
    // 10 = Difference, 11 = Exclusion, 12 = Subtract, 13 = Divide. Unused by
    // echo_mix.
    mode: u32,
    // 1 = the matte scales Decay per pixel. Unused by echo_mix.
    matte_on: f32,
    // Which tap this dispatch is folding in: 0 for the echo one frame back,
    // 1 for two frames back, and so on. It is the exponent the per-pixel
    // decay is raised to, and it is why the matte can be read here at all --
    // the host runs one pass per tap, so the pass knows which it is.
    tap: i32,
}
@group(0) @binding(3) var<uniform> params: Params;

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

// W3C soft-light D(d) helper (== the compositor's `soft_light_d`).
fn echo_soft_light_d(d: vec4<f32>) -> vec4<f32> {
    let poly = ((16.0 * d - 12.0) * d + 4.0) * d;
    return select(sqrt(d), poly, d <= vec4<f32>(0.25));
}

// Fold the weighted neighbour tap `n` into the accumulator `a` (both
// premultiplied linear RGBA), per channel — the exact arithmetic order
// cpu::echo_blend uses, so the two agree bit-for-bit (§1.6).
fn echo_blend(mode: u32, a: vec4<f32>, n: vec4<f32>) -> vec4<f32> {
    let one = vec4<f32>(1.0);
    if (mode == 0u) {
        return a + n * (1.0 - a.a); // Behind: accumulator over the echo
    } else if (mode == 1u) {
        return n + a * (1.0 - n.a); // In front: the echo over the accumulator
    } else if (mode == 2u) {
        return a + n; // Add
    } else if (mode == 3u) {
        return a + n - a * n; // Screen
    } else if (mode == 4u) {
        return a * n; // Multiply
    } else if (mode == 5u) {
        // Overlay = hard light with the accumulator as the switch.
        let lo = 2.0 * a * n;
        let hi = one - 2.0 * (one - a) * (one - n);
        return select(hi, lo, a <= vec4<f32>(0.5));
    } else if (mode == 6u) {
        // Soft light (W3C), s = n, d = a.
        let darkened = a - (one - 2.0 * n) * a * (one - a);
        let lightened = a + (2.0 * n - one) * (echo_soft_light_d(a) - a);
        return select(lightened, darkened, n <= vec4<f32>(0.5));
    } else if (mode == 7u) {
        // Hard light: the echo is the switch.
        let lo = 2.0 * a * n;
        let hi = one - 2.0 * (one - a) * (one - n);
        return select(hi, lo, n <= vec4<f32>(0.5));
    } else if (mode == 8u) {
        return max(a, n); // Lighten (per-channel max)
    } else if (mode == 9u) {
        return min(a, n); // Darken (per-channel min)
    } else if (mode == 10u) {
        return abs(a - n); // Difference
    } else if (mode == 11u) {
        return a + n - 2.0 * a * n; // Exclusion
    } else if (mode == 12u) {
        return max(a - n, vec4<f32>(0.0)); // Subtract
    }
    return max(a / max(n, vec4<f32>(1e-6)), vec4<f32>(0.0)); // Divide
}

@compute @workgroup_size(8, 8)
fn echo_accumulate(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let a = textureLoad(src, p, 0);
    // The matte scales Decay per pixel: scaling decay by k scales tap
    // i by k^(i+1), so the whole-frame weight is multiplied by k exactly
    // tap + 1 times — the same running product cpu::echo_matted spells, and
    // exactly the whole-frame weight at k = 1. A tap the matte has taken to
    // zero is SKIPPED rather than folded in at zero, because a zero-weight tap
    // is not a no-op under every combine mode (Multiply by nothing is black),
    // which is the same `continue` the oracle takes.
    var weight = params.weight;
    if (params.matte_on != 0.0) {
        let k = matte_k(p);
        for (var j = 0; j <= params.tap; j++) {
            weight = weight * k;
        }
        if (weight <= 0.0) {
            textureStore(dst, p, a);
            return;
        }
    }
    let n = textureLoad(other, p, 0) * weight;
    textureStore(dst, p, echo_blend(params.mode, a, n));
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
