// The holographic relief: procedural height fields with their slopes,
// filtered to the pixel. Not a kernel on its own; it is prepended after
// fx_holo_core.wgsl to the holographic kernels.
//
// Positions u are in tiles (the sheet's width over the relief's scale) and su
// is the pixel footprint's standard deviation in tiles. Heights run 0 to 1 for
// a white relief, so slopes are height per tile; the caller scales them to
// real slopes. A slope the pixel cannot resolve is never drawn as a pattern:
// it is handed back as a covariance that widens every lobe. Thin sparse marks
// (scratches) keep theirs apart, with the share of the pixel they cover, so
// they catch light the rest of the pixel does not.
//
// The hash is Mark Jarzynski and Marc Olano's pcg3d, as listed in "Hash
// Functions for GPU Rendering" (JCGT 2020, CC BY-ND 3.0).

struct HoloRelief {
    // The resolved mean slope.
    m: vec2<f32>,
    // The unresolved slope covariance over the whole pixel: xx, xy, yy.
    cv: vec3<f32>,
    // The share of the pixel under thin marks, and the covariance inside them.
    c: f32,
    cs: vec3<f32>,
};

fn hr_pcg3(v0: vec3<u32>) -> vec3<u32> {
    var v = v0 * 1664525u + vec3<u32>(1013904223u);
    v.x = v.x + v.y * v.z;
    v.y = v.y + v.z * v.x;
    v.z = v.z + v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x = v.x + v.y * v.z;
    v.y = v.y + v.z * v.x;
    v.z = v.z + v.x * v.y;
    return v;
}

// Three uniform numbers in 0..1 for cell c and salt.
fn hr_rnd3(c: vec2<i32>, salt: u32) -> vec3<f32> {
    let h = hr_pcg3(vec3<u32>(bitcast<u32>(c.x), bitcast<u32>(c.y), salt));
    return vec3<f32>(h >> vec3<u32>(8u)) * (1.0 / 16777216.0);
}

// Two standard normal numbers from two uniform ones (Box and Muller).
fn hr_gauss2(r: vec2<f32>) -> vec2<f32> {
    let a = sqrt(-2.0 * log(max(r.x, 1e-6)));
    let t = HC_TAU * r.y;
    return a * vec2<f32>(cos(t), sin(t));
}

fn hr_outer(v: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(v.x * v.x, v.x * v.y, v.y * v.y);
}

fn hr_grad(c: vec2<i32>, salt: u32) -> vec2<f32> {
    let a = HC_TAU * hr_rnd3(c, salt).x;
    return vec2<f32>(cos(a), sin(a));
}

// Gradient noise at p (value, d/dx, d/dy), with a quintic blend.
fn hr_noise(p: vec2<f32>, salt: u32) -> vec3<f32> {
    let i = floor(p);
    let f = p - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    let c = vec2<i32>(i);
    let ga = hr_grad(c, salt);
    let gb = hr_grad(c + vec2<i32>(1, 0), salt);
    let gc = hr_grad(c + vec2<i32>(0, 1), salt);
    let gd = hr_grad(c + vec2<i32>(1, 1), salt);
    let va = dot(ga, f);
    let vb = dot(gb, f - vec2<f32>(1.0, 0.0));
    let vc = dot(gc, f - vec2<f32>(0.0, 1.0));
    let vd = dot(gd, f - vec2<f32>(1.0, 1.0));
    let k = va - vb - vc + vd;
    let value = va + u.x * (vb - va) + u.y * (vc - va) + u.x * u.y * k;
    let d = ga + u.x * (gb - ga) + u.y * (gc - ga) + u.x * u.y * (ga - gb - gc + gd)
        + du * (u.yx * k + vec2<f32>(vb, vc) - vec2<f32>(va));
    return vec3<f32>(value, d);
}

// Variance of one axis of the noise's gradient at unit frequency.
const HR_NOISE_VAR: f32 = 0.55;

// One octave of noise of amplitude a, f cycles per tile along x and g along
// y, added to r: what the pixel resolves as slope, the rest as covariance.
fn hr_octave(r: ptr<function, HoloRelief>, u: vec2<f32>, su: f32, a: f32, f: f32, g: f32,
             salt: u32) {
    let kx = exp(-19.74 * su * su * f * f);
    let ky = exp(-19.74 * su * su * g * g);
    var d = vec2<f32>(0.0);
    if (max(kx, ky) > 1e-3) {
        d = hr_noise(u * vec2<f32>(f, g), salt).yz * vec2<f32>(a * f, a * g);
    }
    (*r).m = (*r).m + d * vec2<f32>(kx, ky);
    (*r).cv = (*r).cv + HR_NOISE_VAR * vec3<f32>(
        (1.0 - kx * kx) * a * a * f * f, 0.0, (1.0 - ky * ky) * a * a * g * g);
}

// Fine scratches: thin grooves of Gaussian section, mostly near one
// direction, each faded at its ends. Analytic in the footprint: the blurred
// groove's slope and the slope it hides.
fn hr_scratches(u: vec2<f32>, su: f32, salt: u32) -> HoloRelief {
    var r: HoloRelief;
    let cell = 0.14;
    let base = vec2<i32>(floor(u / cell));
    let lead = hr_rnd3(vec2<i32>(7, 3), salt).xy * HC_PI;
    var cover = 0.0;
    var hidden = vec3<f32>(0.0);
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<i32>(dx, dy);
            for (var k = 0u; k < 3u; k = k + 1u) {
                let h = hr_rnd3(c, salt ^ (k * 2654435769u));
                if (h.z > 0.36) {
                    continue;
                }
                let h2 = hr_rnd3(c, salt ^ (k * 2654435769u + 2246822507u));
                let h3 = hr_rnd3(c, salt ^ (k * 2654435769u + 3266489909u));
                let centre = (vec2<f32>(c) + h.xy) * cell;
                // Two families of direction, and some strays.
                let fam = select(lead.x, lead.y, h3.z < 0.4);
                let ang = select(fam + (h2.x - 0.5) * 0.6, h2.x * HC_PI, h2.y < 0.3);
                let dir = vec2<f32>(cos(ang), sin(ang));
                let half = cell * (0.08 + 0.92 * pow(h2.z, 2.5));
                let d = u - centre;
                let along = dot(d, dir);
                // A gentle bend: the groove curves away by bend * along^2 / 2.
                let bend = (h3.z - 0.5) * 0.5 / half;
                let x = dot(d, vec2<f32>(-dir.y, dir.x)) - 0.5 * bend * along * along;
                let nrm = normalize(vec2<f32>(-dir.y, dir.x) - bend * along * dir);
                let w = 1.1e-4 * (0.6 + 1.4 * h3.x);
                let v = w * w + su * su;
                if (x * x > 25.0 * v || abs(along) > half + 3.0 * su) {
                    continue;
                }
                // Mostly faint, a few deep.
                let hy = h3.y * h3.y;
                let depth = w * (20.0 + 220.0 * hy * hy);
                let dd = depth * holo_box(along, half, su + 0.3 * half);
                let g = exp(-0.5 * x * x / v);
                let ms = dd * (w * inverseSqrt(v)) * x / v * g;
                let a2 = 0.5 * w * w;
                let va = a2 + su * su;
                let mu = x * a2 / va;
                let es = dd * dd / (w * w * w * w) * sqrt(a2 / va) * exp(-0.5 * x * x / va)
                    * (mu * mu + a2 * su * su / va);
                r.m = r.m + ms * nrm;
                // A scratch's floor is not perfectly smooth either.
                let un = max(es - ms * ms, 0.0);
                hidden = hidden + un * (hr_outer(nrm) + vec3<f32>(0.06, 0.0, 0.06));
                cover = cover + w * inverseSqrt(v) * g * holo_box(along, half, su);
            }
        }
    }
    r.c = min(cover, 1.0);
    r.cs = hidden / max(cover, 1e-4);
    return r;
}

// Brushed metal (fine streaks along x, forty times longer than wide) as
// three octaves, or sandblast (fine even grain, mostly too small to see, so
// mostly satin) as two, through one octave call.
fn hr_octaves(u: vec2<f32>, su: f32, salt: u32, brushed: bool, count: u32) -> HoloRelief {
    var r: HoloRelief;
    for (var k = 0u; k < count; k = k + 1u) {
        var a: f32;
        var f: f32;
        var g: f32;
        if (brushed) {
            g = select(select(320.0, 140.0, k == 1u), 60.0, k == 0u);
            f = select(select(8.0, 3.5, k == 1u), 1.5, k == 0u);
            a = 8.0 / g;
        } else {
            f = select(600.0, 250.0, k == 0u);
            g = f;
            a = 9.0 / f;
        }
        hr_octave(&r, u, su, a, f, g, salt + k);
    }
    return r;
}

// Hammered: overlapping round dents, each a smooth dip that ends within a
// cell, so the 3 by 3 cells around always hold every dent that reaches. The
// footprint widens a dent and makes it shallower in step.
fn hr_hammered(u: vec2<f32>, su: f32, salt: u32) -> HoloRelief {
    var r: HoloRelief;
    let cell = 0.08;
    let base = vec2<i32>(floor(u / cell));
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<i32>(dx, dy);
            let h = hr_rnd3(c, salt);
            let h2 = hr_rnd3(c, salt ^ 1597334677u);
            let centre = (vec2<f32>(c) + 0.5 + 0.6 * (h.xy - 0.5)) * cell;
            let rad = cell * (0.55 + 0.25 * h.z);
            let depth = rad * 5.0 * (0.6 + 0.8 * h2.x);
            let re2 = rad * rad + 4.0 * su * su;
            let d = u - centre;
            let t = 1.0 - dot(d, d) / re2;
            if (t > 0.0) {
                // h = -depth (rad^2 / re2) t^3, so its slope is 6 depth rad^2 t^2 d / re2^2.
                r.m = r.m + (6.0 * depth * rad * rad / (re2 * re2)) * t * t * d;
            }
        }
    }
    return r;
}

// Crinkled: flat facets each tipped its own way, blended softly at their
// creases over the footprint. What the blend hides is covariance.
fn hr_facet(c: vec2<i32>, u: vec2<f32>, cell: f32, salt: u32) -> vec3<f32> {
    let h = hr_rnd3(c, salt);
    let pt = (vec2<f32>(c) + 0.5 + 0.9 * (h.xy - 0.5)) * cell;
    let slope = 9.0 * hr_gauss2(hr_rnd3(c, salt ^ 3812015801u).xy);
    return vec3<f32>(length(u - pt), slope);
}

fn hr_crinkled(u: vec2<f32>, su: f32, salt: u32) -> HoloRelief {
    var r: HoloRelief;
    let cell = 0.09;
    let base = vec2<i32>(floor(u / cell));
    let soft = max(1.2 * su, 0.03 * cell);
    var d1 = 1e9;
    for (var i = 0; i < 9; i = i + 1) {
        let c = base + vec2<i32>(i % 3 - 1, i / 3 - 1);
        let h = hr_rnd3(c, salt);
        d1 = min(d1, length(u - (vec2<f32>(c) + 0.5 + 0.9 * (h.xy - 0.5)) * cell));
    }
    var ws = 0.0;
    var m = vec2<f32>(0.0);
    var m2 = vec3<f32>(0.0);
    for (var i = 0; i < 9; i = i + 1) {
        let f = hr_facet(base + vec2<i32>(i % 3 - 1, i / 3 - 1), u, cell, salt);
        let w = exp(-(f.x - d1) / soft);
        ws = ws + w;
        m = m + w * f.yz;
        m2 = m2 + w * hr_outer(f.yz);
    }
    r.m = m / ws;
    r.cv = max(m2 / ws - hr_outer(r.m), vec3<f32>(0.0, -1e9, 0.0));
    return r;
}

// Linen: a plain weave, threads along x and along y passing over and under.
fn hr_linen(u: vec2<f32>, su: f32) -> HoloRelief {
    var r: HoloRelief;
    let pitch = 0.018;
    let k = HC_TAU / pitch;
    let iy = floor(u.y / pitch + 0.5);
    let ix = floor(u.x / pitch + 0.5);
    // Threads along x: section across y, over and under along x.
    let sy = 0.5 + 0.5 * cos(k * u.y);
    let ox = 0.5 + 0.5 * cos(0.5 * k * u.x + HC_PI * iy);
    let sx = 0.5 + 0.5 * cos(k * u.x);
    let oy = 0.5 + 0.5 * cos(0.5 * k * u.y + HC_PI * (ix + 1.0));
    let a = 0.07;
    let d = a * vec2<f32>(
        sy * (-0.25 * k * sin(0.5 * k * u.x + HC_PI * iy)) + (-0.5 * k * sin(k * u.x)) * oy,
        (-0.5 * k * sin(k * u.y)) * ox + sx * (-0.25 * k * sin(0.5 * k * u.y + HC_PI * (ix + 1.0))));
    let g = exp(-19.74 * su * su / (pitch * pitch));
    r.m = g * d;
    let var1 = (1.0 - g * g) * a * a * k * k * 0.1;
    r.cv = vec3<f32>(var1, 0.0, var1);
    return r;
}

// Fingerprints: patches of oval ridges, a few to a tile.
fn hr_fingerprint(u: vec2<f32>, su: f32, salt: u32) -> HoloRelief {
    var r: HoloRelief;
    let cell = 0.45;
    let lam = 0.018;
    let base = vec2<i32>(floor(u / cell));
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<i32>(dx, dy);
            let h = hr_rnd3(c, salt);
            if (h.z > 0.6) {
                continue;
            }
            let h2 = hr_rnd3(c, salt ^ 2747636419u);
            let centre = (vec2<f32>(c) + 0.5 + 0.5 * (h.xy - 0.5)) * cell;
            let ang = HC_PI * h2.x;
            let ca = cos(ang);
            let sa = sin(ang);
            let d = u - centre;
            let e = vec2<f32>(ca * d.x + sa * d.y, -sa * d.x + ca * d.y);
            let ax = vec2<f32>(0.11, 0.15) * (0.8 + 0.4 * h2.y);
            let n = e / ax;
            let rn = length(n);
            if (rn > 1.6) {
                continue;
            }
            // Radius in tiles and its gradient in the sheet, with a whorl of
            // none, one or two turns: the angle adds whole ridges, so the
            // ridges stay unbroken where it wraps.
            let re = rn * ax.y;
            let turns = floor(3.0 * h.x);
            let ge = select(vec2<f32>(0.0), (n / max(rn, 1e-5)) / ax * ax.y, rn > 1e-5);
            let ga = vec2<f32>(-n.y / ax.x, n.x / ax.y) / max(rn * rn, 1e-4);
            let gp = ge + (lam * turns / HC_TAU) * ga;
            let gr = vec2<f32>(ca * gp.x - sa * gp.y, sa * gp.x + ca * gp.y);
            let warp = 0.01 * sin(9.0 * e.x + 6.0 * h2.z) * sin(7.0 * e.y);
            let phase = re + lam * turns * atan2(n.y, n.x) / HC_TAU + warp;
            let mask = exp(-pow(rn, 4.0));
            let amp = 0.05 * mask;
            let f = length(gr) / lam;
            let kf = exp(-19.74 * su * su * f * f);
            let s = -amp * HC_TAU / lam * sin(HC_TAU * phase / lam);
            r.m = r.m + kf * s * gr;
            let nr = gr / max(length(gr), 1e-5);
            r.cv = r.cv + (1.0 - kf * kf) * 0.5 * (amp * HC_TAU * f) * (amp * HC_TAU * f)
                * hr_outer(nr);
        }
    }
    return r;
}


// The relief of `kind` (0 scratches, 1 brushed, 2 hammered, 3 crinkled,
// 4 linen, 5 fingerprint, 6 sandblast, otherwise none).
fn holo_relief(kind: u32, u: vec2<f32>, su: f32, seed: u32) -> HoloRelief {
    let salt = hr_pcg3(vec3<u32>(seed, kind, 40503u)).x;
    // Filtered a little wider than the pixel, so detail near the pixel's own
    // size goes to covariance rather than to a pattern that crawls.
    let sf = 1.5 * su;
    var r: HoloRelief;
    switch kind {
        case 0u: { r = hr_scratches(u, sf, salt); }
        case 1u, 6u: {
            r = hr_octaves(u, sf, salt, kind == 1u, select(holo_two(), holo_three(), kind == 1u));
        }
        case 2u: { r = hr_hammered(u, sf, salt); }
        case 3u: { r = hr_crinkled(u, sf, salt); }
        case 4u: { r = hr_linen(u, sf); }
        case 5u: { r = hr_fingerprint(u, sf, salt); }
        default: {}
    }
    return r;
}

// Up to four hairline scratches in the varnish near p (mm), footprint smm:
// each its direction and the share of the pixel it covers. Per 1 mm cell two
// may lie, each with chance `share`.
struct HoloHairlines {
    h0: vec4<f32>,
    h1: vec4<f32>,
    h2: vec4<f32>,
    h3: vec4<f32>,
};

// Puts `h` in the place of the slot that covers least, if it covers more.
fn hr_keep(out: ptr<function, HoloHairlines>, h: vec4<f32>) {
    let lo = min(min((*out).h0.z, (*out).h1.z), min((*out).h2.z, (*out).h3.z));
    if (h.z <= lo) {
        return;
    }
    if ((*out).h0.z == lo) {
        (*out).h0 = h;
    } else if ((*out).h1.z == lo) {
        (*out).h1 = h;
    } else if ((*out).h2.z == lo) {
        (*out).h2 = h;
    } else {
        (*out).h3 = h;
    }
}

fn holo_hairlines(p: vec2<f32>, smm: f32, share: f32, seed: u32) -> HoloHairlines {
    var out: HoloHairlines;
    let cell = 1.0;
    let base = vec2<i32>(floor(p / cell));
    let salt = hr_pcg3(vec3<u32>(seed, 77u, 1013u)).x;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<i32>(dx, dy);
            for (var k = 0u; k < 2u; k = k + 1u) {
                let h = hr_rnd3(c, salt ^ (k * 2654435769u));
                if (h.z >= share) {
                    continue;
                }
                let h2 = hr_rnd3(c, salt ^ (k * 2654435769u + 668265263u));
                let centre = (vec2<f32>(c) + h.xy) * cell;
                let ang = HC_PI * h2.x;
                let dir = vec2<f32>(cos(ang), sin(ang));
                let half = cell * (0.3 + 0.7 * h2.y);
                let d = p - centre;
                let along = dot(d, dir);
                let x = dot(d, vec2<f32>(-dir.y, dir.x));
                let hw = 0.0015 + 0.0025 * h2.z;
                if (abs(x) > hw + 4.0 * smm || abs(along) > half + 3.0 * smm) {
                    continue;
                }
                let cov = holo_box(x, hw, smm) * holo_box(along, half, smm);
                hr_keep(&out, vec4<f32>(dir, cov, 0.0));
            }
        }
    }
    return out;
}

// How much of the pixel at p (mm), footprint smm, dust covers: specks 15 to
// 50 um across on 5 mm cells, each cell holding one with chance `share`.
fn holo_dust(p: vec2<f32>, smm: f32, share: f32, seed: u32) -> f32 {
    let cell = 5.0;
    let base = vec2<i32>(floor(p / cell));
    let salt = hr_pcg3(vec3<u32>(seed, 91u, 2029u)).x;
    var cov = 0.0;
    for (var dy = -1; dy <= 1; dy = dy + 1) {
        for (var dx = -1; dx <= 1; dx = dx + 1) {
            let c = base + vec2<i32>(dx, dy);
            let h = hr_rnd3(c, salt);
            if (h.z >= share) {
                continue;
            }
            let rad = 0.0075 + 0.0175 * hr_rnd3(c, salt ^ 1u).x;
            let d = p - (vec2<f32>(c) + h.xy) * cell;
            let v = rad * rad + 2.0 * smm * smm;
            cov = cov + rad * rad / v * exp(-dot(d, d) / v);
        }
    }
    return min(cov, 1.0);
}
