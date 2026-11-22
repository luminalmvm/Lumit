// The holographic core: maths the holographic kernels share. Not a kernel on
// its own; it is prepended to each one, which must define
// `holo_row(i: u32) -> vec4<f32>`, the i-th row of its colour terms.
//
// The colour of a band of light is summed in closed form over the CIE
// observer's Gaussian lobes (Wyman, Sloan and Shirley, JCGT 2013). The model
// follows holospot by Dmitry Kurash (MIT, Copyright (c) 2026 Dmitry Kurash).
// The highlight curves are the Khronos PBR Neutral tone mapper (Khronos
// Group, Apache 2.0, github.com/KhronosGroup/ToneMapping) and AgX as three.js
// writes it (three.js authors, MIT, tonemapping_pars_fragment.glsl.js), both
// transcribed close to their published code.

const HC_PI: f32 = 3.14159265;
const HC_TAU: f32 = 6.28318531;

// The standard normal CDF (Abramowitz and Stegun 7.1.26), written so the tail
// stays accurate relative to its own size.
fn holo_cdf(u: f32) -> f32 {
    let x = abs(u) * 0.70710678;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741
        + t * (-1.453152027 + t * 1.061405429))));
    let tail = 0.5 * poly * exp(-x * x);
    return select(1.0 - tail, tail, u < 0.0);
}

fn holo_pdf(u: f32) -> f32 {
    return 0.39894228 * exp(-0.5 * u * u);
}

// A box from -h to h, blurred by a Gaussian of width s, read at v.
fn holo_box(v: f32, h: f32, s: f32) -> f32 {
    let w = max(s, 1e-5);
    return holo_cdf((h - v) / w) - holo_cdf((-h - v) / w);
}

// The Bessel function J_m: eight terms of its series, enough for x up to the
// 5.5 the phase is held to.
fn holo_bessel(m: u32, x: f32) -> f32 {
    let h = 0.5 * x;
    let h2 = h * h;
    var term = 1.0;
    for (var k = 1u; k <= m; k = k + 1u) {
        term = term * h / f32(k);
    }
    var sum = term;
    let mf = f32(m);
    for (var k = 1u; k < 8u; k = k + 1u) {
        let kf = f32(k);
        term = term * (-h2 / (kf * (kf + mf)));
        sum = sum + term;
    }
    return sum;
}

// J_0 at four values at once, by the same series.
fn holo_bessel0(x: vec4<f32>) -> vec4<f32> {
    let h2 = 0.25 * x * x;
    var term = vec4<f32>(1.0);
    var sum = term;
    for (var k = 1u; k < 8u; k = k + 1u) {
        let kf = f32(k);
        term = term * (-h2 / (kf * kf));
        sum = sum + term;
    }
    return sum;
}

// A dielectric's unpolarised reflectance at cosine c, index n.
fn holo_fresnel(c: f32, n: f32) -> f32 {
    let cc = clamp(c, 1e-4, 1.0);
    let g = sqrt(max(n * n - 1.0 + cc * cc, 0.0));
    let a = (g - cc) / (g + cc);
    let b = (cc * (g + cc) - 1.0) / (cc * (g - cc) + 1.0);
    return 0.5 * a * a * (1.0 + b * b);
}

fn holo_ggx(nh: f32, a2: f32) -> f32 {
    let d = nh * nh * (a2 - 1.0) + 1.0;
    return a2 / (HC_PI * d * d);
}

// The height-correlated Smith visibility, with the 1 / (4 nl nv) folded in.
fn holo_smith(nl: f32, nv: f32, a2: f32) -> f32 {
    let gv = nl * sqrt(nv * nv * (1.0 - a2) + a2);
    let gl = nv * sqrt(nl * nl * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-6);
}

// The slow part of a term at its band's mean m and spread sd: aluminium, a
// Planck curve (planck = temperature, factor; temperature 0 for none) and the
// dye's edge (tint = kind, edge, width, floor; kind 1 long pass, 2 short).
fn holo_slow(m: f32, sd: f32, planck: vec2<f32>, tint: vec4<f32>) -> f32 {
    var f = 0.889 - 1.25e-4 * (clamp(m, 380.0, 780.0) - 450.0);
    if (planck.x > 0.0) {
        let lm = max(m, 250.0);
        let r = 560.0 / lm;
        f = f * planck.y * (r * r * r * r * r) / (exp(1.4388e7 / (lm * planck.x)) - 1.0);
    }
    if (tint.x > 0.5) {
        let z = (m - tint.y) / sqrt(0.25 * tint.z * tint.z + sd * sd);
        let s = holo_cdf(select(-z, z, tint.x < 1.5));
        f = f * (tint.w + (1.0 - tint.w) * s);
    }
    return f;
}

// The linear sRGB colour of a band of light centred at lam nm, sig nm wide,
// summed over `count` terms from `first`. Each term is two rows: amplitude
// and centre; width, cut and side (-1 below the cut, +1 above, 0 neither and
// no slow factor).
fn holo_band(lam: f32, sig: f32, first: u32, count: u32, planck: vec2<f32>,
             tint: vec4<f32>) -> vec3<f32> {
    var acc = vec3<f32>(0.0);
    let s2 = sig * sig;
    for (var i = 0u; i < count; i = i + 1u) {
        let a = holo_row(2u * (first + i));
        let b = holo_row(2u * (first + i) + 1u);
        let s = b.x;
        let v = s * s + s2;
        let dl = lam - a.w;
        let e = dl * dl / (2.0 * v);
        if (e > 14.0) {
            continue;
        }
        let g = s * inverseSqrt(v) * exp(-e);
        if (b.z == 0.0) {
            acc = acc + a.xyz * g;
            continue;
        }
        let mu = (a.w * s2 + lam * s * s) / v;
        let tau = s * sig * inverseSqrt(v);
        let u = b.z * (mu - b.y) / tau;
        let cp = holo_cdf(u);
        let r = holo_pdf(u) / max(cp, 1e-6);
        let m = mu + b.z * tau * r;
        let sd = tau * sqrt(max(1.0 - u * r - r * r, 0.02));
        acc = acc + a.xyz * (g * cp * holo_slow(m, sd, planck, tint));
    }
    return acc;
}

const HC_LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// Spectral colours fall outside sRGB. Pull a colour with a negative channel
// toward its own grey, keeping its luminance and hue.
fn holo_gamut(c: vec3<f32>) -> vec3<f32> {
    let y = dot(c, HC_LUMA);
    if (y <= 0.0) {
        return vec3<f32>(0.0);
    }
    if (min(c.x, min(c.y, c.z)) >= 0.0) {
        return c;
    }
    let neg = max(-c, vec3<f32>(0.0));
    let k = pow(dot(pow(neg, vec3<f32>(1.5)), vec3<f32>(1.0)), 2.0 / 3.0);
    return max(vec3<f32>(y) + (c - vec3<f32>(y)) * (y / (y + k)), vec3<f32>(0.0));
}

// The Khronos PBR Neutral highlight roll-off (Khronos Group, Apache 2.0).
fn holo_neutral(c0: vec3<f32>) -> vec3<f32> {
    var c = c0;
    let x = min(c.x, min(c.y, c.z));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    c = c - vec3<f32>(offset);
    let peak = max(c.x, max(c.y, c.z));
    if (peak < 0.76) {
        return c;
    }
    let d = 0.24;
    let np = 1.0 - d * d / (peak + d - 0.76);
    c = c * (np / peak);
    let g = 1.0 - 1.0 / (0.15 * (peak - np) + 1.0);
    return mix(c, vec3<f32>(np), g);
}

// AgX, as three.js writes it (three.js authors, MIT): Rec. 2020 inset, a log2
// range of -12.47 to 4.03, its contrast polynomial, the outset, a 2.2 power.
fn holo_agx(c0: vec3<f32>) -> vec3<f32> {
    let to2020 = mat3x3<f32>(
        vec3<f32>(0.6274, 0.0691, 0.0164),
        vec3<f32>(0.3293, 0.9195, 0.0880),
        vec3<f32>(0.0433, 0.0113, 0.8956));
    let from2020 = mat3x3<f32>(
        vec3<f32>(1.6605, -0.1246, -0.0182),
        vec3<f32>(-0.5876, 1.1329, -0.1006),
        vec3<f32>(-0.0728, -0.0083, 1.1187));
    let inset = mat3x3<f32>(
        vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
        vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
        vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859));
    let outset = mat3x3<f32>(
        vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
        vec3<f32>(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
        vec3<f32>(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405));
    var c = inset * (to2020 * c0);
    c = log2(max(c, vec3<f32>(1e-10)));
    c = clamp((c + 12.47393) / 16.49999, vec3<f32>(0.0), vec3<f32>(1.0));
    let x2 = c * c;
    let x4 = x2 * x2;
    c = 15.5 * x4 * x2 - 40.14 * x4 * c + 31.96 * x4 - 6.868 * x2 * c + 0.4298 * x2
        + 0.1191 * c - vec3<f32>(0.00232);
    c = outset * c;
    c = pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
    return clamp(from2020 * c, vec3<f32>(0.0), vec3<f32>(1.0));
}
