// Oklab conversions for effect kernels (decision K-034).
//
// SAME constants as oklab.rs — that file is the CPU oracle; a test compiles
// this module so it cannot rot. Effect kernels needing perceptual
// interpolation or hue-type operations include these functions.
// A dummy entry point exists solely so the module validates standalone.

fn linear_srgb_to_oklab(c: vec3<f32>) -> vec3<f32> {
    let l = 0.41222146 * c.r + 0.53633254 * c.g + 0.051445995 * c.b;
    let m = 0.2119035 * c.r + 0.6806995 * c.g + 0.10739696 * c.b;
    let s = 0.08830246 * c.r + 0.28171885 * c.g + 0.6299787 * c.b;
    let l_ = pow(l, 1.0 / 3.0);
    let m_ = pow(m, 1.0 / 3.0);
    let s_ = pow(s, 1.0 / 3.0);
    return vec3<f32>(
        0.21045426 * l_ + 0.7936178 * m_ - 0.004072047 * s_,
        1.9779985 * l_ - 2.4285922 * m_ + 0.4505937 * s_,
        0.025904037 * l_ + 0.78277177 * m_ - 0.80867577 * s_,
    );
}

fn oklab_to_linear_srgb(c: vec3<f32>) -> vec3<f32> {
    let l_ = c.x + 0.39633778 * c.y + 0.21580376 * c.z;
    let m_ = c.x - 0.105561346 * c.y - 0.06385417 * c.z;
    let s_ = c.x - 0.08948418 * c.y - 1.2914855 * c.z;
    let l = l_ * l_ * l_;
    let m = m_ * m_ * m_;
    let s = s_ * s_ * s_;
    return vec3<f32>(
        4.0767417 * l - 3.3077116 * m + 0.23096994 * s,
        -1.268438 * l + 2.6097574 * m - 0.34131938 * s,
        -0.0041960863 * l - 0.7034186 * m + 1.7076147 * s,
    );
}

fn oklab_lerp(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return oklab_to_linear_srgb(
        mix(linear_srgb_to_oklab(a), linear_srgb_to_oklab(b), t),
    );
}

fn oklab_hue_rotate(rgb: vec3<f32>, radians: f32) -> vec3<f32> {
    let lab = linear_srgb_to_oklab(rgb);
    let cs = cos(radians);
    let sn = sin(radians);
    return oklab_to_linear_srgb(vec3<f32>(
        lab.x,
        lab.y * cs - lab.z * sn,
        lab.y * sn + lab.z * cs,
    ));
}

fn oklch_from_linear(rgb: vec3<f32>) -> vec3<f32> {
    let lab = linear_srgb_to_oklab(rgb);
    return vec3<f32>(lab.x, length(lab.yz), atan2(lab.z, lab.y));
}

fn linear_from_oklch(lch: vec3<f32>) -> vec3<f32> {
    return oklab_to_linear_srgb(
        vec3<f32>(lch.x, lch.y * cos(lch.z), lch.y * sin(lch.z)),
    );
}

// THE gradient primitive (K-034): shortest-arc hue interpolation.
fn oklch_lerp(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    let tau = 6.2831853;
    var la = oklch_from_linear(a);
    var lb = oklch_from_linear(b);
    if (la.y < 1e-5) { la.z = lb.z; }
    if (lb.y < 1e-5) { lb.z = la.z; }
    var dh = (lb.z - la.z) % tau;
    if (dh > tau * 0.5) { dh -= tau; }
    if (dh < -tau * 0.5) { dh += tau; }
    return linear_from_oklch(vec3<f32>(
        mix(la.x, lb.x, t),
        mix(la.y, lb.y, t),
        la.z + dh * t,
    ));
}

// Validation-only entry point (effects include the functions above).
@compute @workgroup_size(1)
fn oklab_validate() {
    let probe = oklch_lerp(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), 0.5);
    let rotated = oklab_hue_rotate(probe, 1.0);
    _ = rotated;
}
