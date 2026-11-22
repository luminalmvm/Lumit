// Hologram: the layer as a volume hologram on a flat film. Compiled after
// fx_holo_core.wgsl, fx_holo_relief.wgsl, fx_holo_shatter.wgsl,
// fx_holo_glitter.wgsl, fx_holo_sparkle.wgsl and fx_holographic_foil.wgsl,
// whose sheet, lights, studio, surface and output it shares.
//
// Per pixel: the film's own surface as Holographic foil lights a plain
// mirror (with the film's line patterns as its grating), then for the
// brightest lights the replay: the direction the hologram sends each light,
// its colour band and weight, a trace of that ray through the subject's
// depth behind the film, and the subject and the backdrop read there,
// softened as the ray goes deeper. The layer is the subject; its alpha is the
// film. Seen from behind, the film is plain satin metal. Output is
// scene-linear, premultiplied by the layer alpha.
//
// The replay rows sit where the foil keeps its flakes' and dots', which a
// hologram never reads, as the table in
// lumit_core::fx::effects::hologram (pack.rs) says.

@group(0) @binding(9) var depth_map: texture_2d<f32>;
@group(0) @binding(10) var backdrop_layer: texture_2d<f32>;
// Halved copies of the layer and of the Backdrop layer, each texel the mean
// of four below it, for reading them blurred.
@group(0) @binding(11) var subject_mips: texture_2d<f32>;
@group(0) @binding(12) var backdrop_mips: texture_2d<f32>;

const F_DEPTH: u32 = 64u;
const F_BACKDROP: u32 = 128u;

const R_H_TYPE: u32 = 280u;
const R_H_REFERENCE: u32 = 281u;
const R_H_REPLAY: u32 = 282u;
const R_H_SLIT: u32 = 283u;
const R_H_DEPTHS: u32 = 284u;
const R_H_VIEWS: u32 = 285u;
const R_H_PATTERN: u32 = 286u;

// The replay's gain over holospot's: Lumit's rigs are a quarter as bright
// and it draws no display glare, so the image would read dim.
const HL_GAIN: f32 = 3.0;

// The film's index, for the Bragg condition inside it.
const HL_N2: f32 = 2.25;
// How far a laser speckle grain reaches on the film, mm, and how far the eye
// moves, in direction cosines, before it changes.
const HL_GRAIN: f32 = 0.05;
const HL_DECORRELATE: f32 = 0.011;

// One light's replay at this pixel.
struct Replay {
    // pi', the direction into the film along which the image is seen.
    dir: vec3<f32>,
    lam: f32,
    sig: f32,
    // rad A Tc w / pi: what the light gives the image before its colour.
    e: f32,
    spec: u32,
    // The tangent of the cone a point of the image is blurred over by the
    // light's size.
    cone: f32,
    // The replay band's colour, its peak at 1.
    band: vec3<f32>,
};

// A texel of the layer (level 0) or of its halved copies, nothing outside.
fn hl_texel(which: u32, level: u32, c: vec2<i32>, dims: vec2<i32>) -> vec4<f32> {
    if (any(c < vec2<i32>(0)) || any(c >= dims)) {
        return vec4<f32>(0.0);
    }
    if (which == 0u) {
        if (level == 0u) {
            return textureLoad(src, c, 0);
        }
        return textureLoad(subject_mips, c, i32(level) - 1);
    }
    if (level == 0u) {
        return textureLoad(backdrop_layer, c, 0);
    }
    return textureLoad(backdrop_mips, c, i32(level) - 1);
}

fn hl_dims(which: u32, level: u32) -> vec2<i32> {
    if (which == 0u) {
        if (level == 0u) {
            return vec2<i32>(textureDimensions(src));
        }
        return vec2<i32>(textureDimensions(subject_mips, i32(level) - 1));
    }
    if (level == 0u) {
        return vec2<i32>(textureDimensions(backdrop_layer));
    }
    return vec2<i32>(textureDimensions(backdrop_mips, i32(level) - 1));
}

// The cubic B-spline's four weights at t.
fn hl_spline(t: f32) -> vec4<f32> {
    let u = 1.0 - t;
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(u * u * u, 3.0 * t3 - 6.0 * t2 + 4.0, -3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0,
                     t3) / 6.0;
}

// Level `level` (`dims` texels) at c in level-0 texels (level 0 is d0
// texels): bilinear on the layer itself, a cubic B-spline on the halved
// copies, so a wide blur has no blocks. Sizes come from the caller: FXC
// cannot keep a loop round a texture's size query inside another loop.
fn hl_level(which: u32, level: u32, c0: vec2<f32>, d0: vec2<f32>, dims: vec2<i32>) -> vec4<f32> {
    let c = c0 * vec2<f32>(dims) / d0;
    let q = c - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    if (level == 0u) {
        let a = hl_texel(which, level, i, dims);
        let b = hl_texel(which, level, i + vec2<i32>(1, 0), dims);
        let cc = hl_texel(which, level, i + vec2<i32>(0, 1), dims);
        let d = hl_texel(which, level, i + vec2<i32>(1, 1), dims);
        return mix(mix(a, b, f.x), mix(cc, d, f.x), f.y);
    }
    let wx = hl_spline(f.x);
    let wy = hl_spline(f.y);
    var acc = vec4<f32>(0.0);
    for (var y = 0; y < 4; y = y + 1) {
        var row = vec4<f32>(0.0);
        for (var x = 0; x < 4; x = x + 1) {
            row = row + wx[x] * hl_texel(which, level, i + vec2<i32>(x - 1, y - 1), dims);
        }
        acc = acc + wy[y] * row;
    }
    return acc;
}

// The layer (which 0) or the Backdrop layer (1) at c, blurred over about
// `r` texels and smeared along `fan` (one sigma, texels), read between two of
// its halved copies. The smear is three reads, at -1, 0 and +1 times 0.987
// fan, each blurred by a share of it so they join.
fn hl_picture(which: u32, c: vec2<f32>, r0: f32, fan: vec2<f32>) -> vec4<f32> {
    var levels: u32;
    if (which == 0u) {
        levels = textureNumLevels(subject_mips);
    } else {
        levels = textureNumLevels(backdrop_mips);
    }
    let r = sqrt(r0 * r0 + 0.35 * dot(fan, fan));
    let lod = clamp(log2(max(2.0 * r, 1.0)), 0.0, f32(levels));
    let l0 = u32(floor(lod));
    let f = lod - f32(l0);
    let two = !(f < 0.02 || l0 >= levels);
    let d0 = vec2<f32>(hl_dims(which, 0u));
    let da = hl_dims(which, l0);
    let db = hl_dims(which, min(l0 + 1u, levels));
    // Both levels and every read through one call.
    let nl = select(1u, holo_two(), two);
    let taps = select(1u, holo_three(), dot(fan, fan) > 0.01);
    var a = vec4<f32>(0.0);
    var b = vec4<f32>(0.0);
    for (var k = 0u; k < nl * taps; k = k + 1u) {
        let tap = k / nl;
        let lev = k - tap * nl;
        let o = select((f32(tap) - 1.0) * 0.987, 0.0, taps == 1u) * fan;
        let v = hl_level(which, l0 + lev, c + o, d0, select(da, db, lev == 1u)) / f32(taps);
        if (lev == 0u) {
            a = a + v;
        } else {
            b = b + v;
        }
    }
    if (!two) {
        return a;
    }
    return mix(a, b, f);
}

// The Depth map (`dims` texels) at c, bilinear, held at its edges.
fn hl_depth_luma(c: vec2<f32>, dims: vec2<i32>) -> f32 {
    let q = c - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    let hi = dims - vec2<i32>(1);
    let a = luma_of(textureLoad(depth_map, clamp(i, vec2<i32>(0), hi), 0));
    let b = luma_of(textureLoad(depth_map, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), hi), 0));
    let cc = luma_of(textureLoad(depth_map, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), hi), 0));
    let d = luma_of(textureLoad(depth_map, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), hi), 0));
    return mix(mix(a, b, f.x), mix(cc, d, f.x), f.y);
}

// Sheet millimetres (y up, centred) to layer texels.
fn hl_texel_at(p_mm: vec2<f32>) -> vec2<f32> {
    let size = p.b[R_SIZE];
    return vec2<f32>(p_mm.x / size.z + 0.5 * size.x, 0.5 * size.y - p_mm.y / size.z);
}

// The subject's surface at p_mm, mm in front of the film (below 0 behind).
fn hl_height(p_mm: vec2<f32>, dims: vec2<i32>) -> f32 {
    let d = p.b[R_H_DEPTHS];
    return d.x + d.y * (hl_depth_luma(hl_texel_at(p_mm), dims) - 0.5);
}

// Where the ray from p_mm along `dir` meets the subject: the point to read,
// in sheet mm, and the ray's length to it, mm. The ray runs from the nearest
// the surface can be to the farthest in up to Quality's steps (with no early
// exit, which FXC cannot compile inside the trace loop), then three secant
// steps. Where the surface jumps toward the eye faster than the ray falls (a
// cliff in the depth map) the ray reads the far side's edge, so the cliff's
// wall is not smeared with the near side's colour. The ray always runs into
// the film: a direction along it is held just below, so nothing divides by 0.
fn hl_trace(p_mm: vec2<f32>, dir: vec3<f32>) -> vec3<f32> {
    let d = p.b[R_H_DEPTHS];
    let zd = min(dir.z, -1e-3);
    let k = dir.xy / zd;
    if (!has(F_DEPTH) || d.y == 0.0) {
        return vec3<f32>(p_mm + k * d.x, d.x / zd);
    }
    let dims = vec2<i32>(textureDimensions(depth_map));
    let hw = 0.5 * abs(d.y);
    let z_hi = d.x + hw;
    let z_lo = d.x - hw;
    let travel = length(k) * (z_hi - z_lo) / p.b[R_SIZE].z;
    // About one step every Quality's step of pixels of travel, at most its
    // step count.
    let n = u32(clamp(travel / p.b[R_QUALITY + 1u].w, 4.0, p.b[R_QUALITY].w));
    let dz = (z_hi - z_lo) / f32(n);
    var za = z_hi;
    var fa = za - hl_height(p_mm + k * za, dims);
    var zb = z_lo;
    var fb = -1e-4;
    var found = false;
    for (var i = 1u; i <= n; i = i + 1u) {
        if (found) {
            continue;
        }
        let z = z_hi - f32(i) * dz;
        let f = z - hl_height(p_mm + k * z, dims);
        if (f <= 0.0) {
            zb = z;
            fb = f;
            found = true;
        } else {
            za = z;
            fa = f;
        }
    }
    // A cliff: the surface rose by more than three times the ray's fall.
    let rise = (zb - fb) - (za - fa);
    if (rise > 3.0 * (za - zb)) {
        return vec3<f32>(p_mm + k * za, za / zd);
    }
    var lo = za;
    var flo = fa;
    var hi = zb;
    var fhi = fb;
    for (var j = 0u; j < 3u; j = j + 1u) {
        let zm = lo + (hi - lo) * flo / max(flo - fhi, 1e-6);
        let fm = zm - hl_height(p_mm + k * zm, dims);
        if (fm > 0.0) {
            lo = zm;
            flo = fm;
        } else {
            hi = zm;
            fhi = fm;
        }
    }
    let z = lo + (hi - lo) * flo / max(flo - fhi, 1e-6);
    return vec3<f32>(p_mm + k * z, z / zd);
}

// The recorded grid or stars at q (mm, turned with the film's patterns), over
// a blur of `r` mm.
fn hl_recorded(q: vec2<f32>, kind: u32, r: f32) -> f32 {
    let pat = p.b[R_H_PATTERN];
    if (kind == 1u) {
        // A 4 mm grid at the default size, lines 0.11 mm half wide.
        let pitch = pat.x * 7.0;
        let hw = 0.11 * pitch / 4.0;
        let s = sqrt(r * r + 0.09 * hw * hw);
        let m = q / pitch - floor(q / pitch + 0.5);
        let cx = holo_box(m.x * pitch, hw, s);
        let cy = holo_box(m.y * pitch, hw, s);
        return 0.2 * (1.0 - (1.0 - cx) * (1.0 - cy));
    }
    // Stars: one place per 2.5 mm cell at the default size, about a third
    // filled, each a small round glow.
    let cell = pat.x * 4.3;
    let u = q / cell;
    let base = vec2<i32>(floor(u));
    var e = 0.0;
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let c = base + vec2<i32>(i, j);
            let h = hr_rnd3(c, 7919u);
            if (h.z > 0.35) {
                continue;
            }
            let at = (vec2<f32>(c) + 0.15 + 0.7 * h.xy) * cell;
            let d2 = dot(q - at, q - at);
            let sz = (0.04 + 0.08 * fract(h.x * 7.31)) * cell / 2.5;
            let v = sz * sz + r * r;
            e = e + (0.3 + 2.0 * fract(h.y * 13.7)) * (sz * sz / v) * exp(-0.5 * d2 / v);
        }
    }
    return e;
}

// What was recorded behind the subject at q: the grid or stars and the
// Backdrop layer, premultiplied.
fn hl_backdrop(p_b: vec2<f32>, kind: u32, r_mm: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    if (kind == 1u || kind == 2u) {
        let sp = p.b[R_MAPS].w;
        let cs = vec2<f32>(cos(sp), sin(sp));
        let q = vec2<f32>(cs.x * p_b.x + cs.y * p_b.y, -cs.y * p_b.x + cs.x * p_b.y);
        c = vec3<f32>(hl_recorded(q, kind, r_mm));
    }
    if (has(F_BACKDROP)) {
        let px_mm = p.b[R_SIZE].z;
        c = c + hl_picture(1u, hl_texel_at(p_b), r_mm / px_mm, vec2<f32>(0.0)).rgb;
    }
    return c;
}

// Aluminium's reflectance, which the band colours carry and the replay
// divides back out.
fn hl_aluminium(lam: f32) -> f32 {
    return 0.889 - 1.25e-4 * (clamp(lam, 380.0, 780.0) - 450.0);
}

// Light `l` (radiance rad, projected solid angle a, angular radius r_ang,
// spectrum spec) replayed at a point whose film frame is t, b, n, seen from
// v, at sheet height y_mm. tc is the coat's two-way transmission; wk scales
// the efficiency.
fn hl_replay(t: vec3<f32>, b: vec3<f32>, n: vec3<f32>, v: vec3<f32>, y_mm: f32, l: vec3<f32>,
             rad: f32, a: f32, r_ang: f32, spec: u32, wk: f32, colour: bool) -> Replay {
    var r: Replay;
    r.e = 0.0;
    r.spec = spec;
    let lz = dot(l, n);
    let vz = dot(v, n);
    if (lz <= 0.0 || vz <= 0.0 || rad <= 0.0 || a <= 0.0) {
        return r;
    }
    let lt = vec2<f32>(dot(l, t), dot(l, b));
    let vt = vec2<f32>(dot(v, t), dot(v, b));
    let s = lt + vt;
    let rf = p.b[R_H_REFERENCE];
    let rho = rf.xy;
    let lr = rf.w;
    let rp = p.b[R_H_REPLAY];
    let eff = rp.x * wk;
    var pt: vec2<f32>;
    var w: f32;
    if (p.b[R_H_TYPE].x < 0.5) {
        // Reflection: the Bragg colour, found by three fixed-point steps,
        // shifts bluer as the light and the eye grow steeper.
        let cv = sqrt(max(1.0 - dot(vt, vt) / HL_N2, 0.0));
        let cl = sqrt(max(1.0 - dot(lt, lt) / HL_N2, 0.0));
        let cr = sqrt(max(1.0 - dot(rho, rho) / HL_N2, 0.0));
        var lam = lr;
        for (var i = 0u; i < 3u; i = i + 1u) {
            let q = rho - (lr / lam) * s;
            let cp = sqrt(max(1.0 - dot(q, q) / HL_N2, 0.0));
            lam = lr * (cv + cl) / max(cr + cp, 1e-3);
        }
        pt = rho - (lr / lam) * s;
        let th = asin(min(length(pt), 1.0));
        let sw = rp.w;
        w = eff * exp(-0.5 * (th / sw) * (th / sw)) / (2.0 * sw * sw * 0.4);
        r.lam = lam;
        r.sig = max(rp.z * lam / lr, 0.5);
        r.cone = tan(r_ang);
    } else {
        // Rainbow: the slit's image fixes the vertical direction, so the
        // colour follows the eye up and down.
        let sl = p.b[R_H_SLIT];
        let dy = y_mm - sl.z;
        let py0 = dy * inverseSqrt(dy * dy + sl.y * sl.y);
        let den = rho.y - py0;
        let lam = lr * s.y / select(den, 1e-4, abs(den) < 1e-4);
        if (lam <= 0.0) {
            return r;
        }
        pt = vec2<f32>(rho.x - (lr / lam) * s.x, py0);
        let ax = asin(clamp(pt.x, -1.0, 1.0)) / 0.6;
        w = eff * 0.3 * exp(-0.5 * ax * ax) / (2.0 * sl.x * 0.6 * 0.4);
        let sig_slit = lam * sl.x / max(abs(den), 1e-3);
        let sig_src = lam * 0.5 * r_ang / max(abs(s.y), 1e-3);
        r.lam = lam;
        r.sig = clamp(sqrt(sig_slit * sig_slit + sig_src * sig_src), 0.5, 200.0);
        r.cone = tan(r_ang);
    }
    let p2 = dot(pt, pt);
    if (p2 >= 0.9999) {
        return r;
    }
    // Fades toward the evanescent edge, at grazing light or eye, and at the
    // ends of the visible.
    w = w * (1.0 - smoothstep(0.85, 0.9999, p2)) * smoothstep(0.0, 0.1, lz)
        * smoothstep(0.0, 0.1, vz) * smoothstep(360.0, 390.0, r.lam)
        * (1.0 - smoothstep(790.0, 830.0, r.lam));
    r.dir = vec3<f32>(pt, -sqrt(1.0 - p2));
    let tc = (1.0 - holo_fresnel(lz, 1.5)) * (1.0 - holo_fresnel(vz, 1.5));
    r.e = rad * a * tc * w / HC_PI;
    if (colour) {
        let amp = 2.50662827 * r.sig / hl_aluminium(r.lam);
        r.band = amp * band(r.lam, r.sig, spec);
    }
    return r;
}

// The image along `dir` from p_mm: the subject where the ray meets it,
// smeared along the band's fan, and what lies behind where the subject is
// empty, blurred by the cone and the fan, as the Subject colour choice reads
// it, times the replay band.
fn hl_image(p_mm: vec2<f32>, dir: vec3<f32>, r: Replay) -> vec3<f32> {
    let px_mm = p.b[R_SIZE].z;
    let pix = 0.4 * px_mm;
    let hit = hl_trace(p_mm, dir);
    // The band's spread of wavelengths fans the direction along rho - pi'
    // (one sigma, sig / lr of it); a rainbow's slit holds the vertical.
    let rf = p.b[R_H_REFERENCE];
    let up = select(1.0, 0.0, p.b[R_H_TYPE].x > 0.5);
    let disp = (rf.xy - r.dir.xy) * vec2<f32>(1.0, up) * (r.sig / rf.w);
    let fan = length(disp);
    let rs = sqrt(pix * pix + (hit.z * r.cone) * (hit.z * r.cone));
    // Texel rows run down the film.
    let smear = vec2<f32>(1.0, -1.0) * disp * (hit.z / px_mm);
    let sub = hl_picture(0u, hl_texel_at(hit.xy), rs / px_mm, smear);
    // The subject hides what is behind it as far as it is bright: black is
    // empty, and a soft edge lets the backdrop through as it fades, with no
    // dark rim.
    let occ = smoothstep(0.0, 0.25, max(sub.r, max(sub.g, sub.b)));
    var rec = sub.rgb;
    let kind = u32(p.b[R_H_TYPE].w);
    if ((kind == 1u || kind == 2u) || has(F_BACKDROP)) {
        let zb = p.b[R_H_DEPTHS].z;
        let dz = min(dir.z, -1e-3);
        let tb = zb / dz;
        // The backdrop is read once, the fan folded into its blur.
        let cb = r.cone * r.cone + fan * fan;
        let rb = sqrt(pix * pix + tb * tb * cb);
        let bd = hl_backdrop(p_mm + dir.xy * tb, kind, rb);
        if (zb <= hit.z * dz) {
            rec = rec + (1.0 - occ) * bd;
        } else {
            // A backdrop in front of the subject covers it where it is bright.
            let cover = smoothstep(0.0, 0.03, max(bd.r, max(bd.g, bd.b)));
            rec = rec * (1.0 - cover) + bd;
        }
    }
    let mode = u32(p.b[R_H_TYPE].y);
    if (mode == 0u) {
        return dot(rec, HC_LUMA) * r.band;
    }
    if (mode == 1u) {
        return rec * r.band;
    }
    return rec * dot(r.band, HC_LUMA);
}

// Up to two directions to trace for one light, with their weights.
struct Views {
    d0: vec3<f32>,
    w0: f32,
    d1: vec3<f32>,
    w1: f32,
};

// The stereogram's views: the nearest recorded view, handed over softly to
// the next across the View blend (or at once when `blend_on` is false), and
// gone beyond the zone. An optical recording traces pi' itself.
fn hl_views(r: Replay, stereo: bool, blend_on: bool) -> Views {
    var o: Views;
    o.d0 = r.dir;
    o.w0 = 1.0;
    o.d1 = r.dir;
    o.w1 = 0.0;
    if (!stereo) {
        return o;
    }
    let vw = p.b[R_H_VIEWS];
    let n = vw.x;
    let zone = vw.z;
    let pv = sqrt(max(1.0 - r.dir.y * r.dir.y, 1e-6));
    let th = atan2(r.dir.x, -r.dir.z);
    let gap = zone / max(n - 1.0, 1.0);
    let f = (th + 0.5 * zone) / gap;
    let fade = smoothstep(-0.5, 0.0, f) * (1.0 - smoothstep(n - 1.0, n - 0.5, f));
    let fc = clamp(f, 0.0, n - 1.0);
    let lo = floor(fc);
    let hi = min(lo + 1.0, n - 1.0);
    let hw = 0.5 * vw.y;
    var up = select(0.0, 1.0, fc - lo >= 0.5);
    if (hw > 0.0) {
        up = smoothstep(0.5 - hw, 0.5 + hw, fc - lo);
    }
    if (!blend_on) {
        up = select(0.0, 1.0, up >= 0.5);
    }
    let a0 = -0.5 * zone + lo * gap;
    let a1 = -0.5 * zone + hi * gap;
    o.d0 = vec3<f32>(sin(a0) * pv, r.dir.y, -cos(a0) * pv);
    o.d1 = vec3<f32>(sin(a1) * pv, r.dir.y, -cos(a1) * pv);
    o.w0 = fade * (1.0 - up);
    o.w1 = fade * up;
    return o;
}

// Laser speckle: grains on the film that change as the eye moves, their
// contrast falling where a pixel covers several.
fn hl_speckle(p_mm: vec2<f32>, dir: vec3<f32>, px_mm: f32) -> f32 {
    let k = p.b[R_H_SLIT].w;
    if (k <= 0.0) {
        return 1.0;
    }
    let cell = max(HL_GRAIN, 0.8 * px_mm);
    let c = vec2<i32>(floor(p_mm / cell));
    let s = dot(dir.xy, vec2<f32>(0.8, 0.6)) / HL_DECORRELATE;
    let i = i32(floor(s));
    let f = s - floor(s);
    let seed = u32(p.b[R_RELIEF_MORE].z);
    let u0 = hr_rnd3(c + vec2<i32>(i * 7919, 0), seed + 31u).x;
    let u1 = hr_rnd3(c + vec2<i32>((i + 1) * 7919, 0), seed + 31u).x;
    let g = mix(-log(max(u0, 1e-4)), -log(max(u1, 1e-4)), smoothstep(0.0, 1.0, f));
    return max(1.0 + k * (HL_GRAIN / cell) * (g - 1.0), 0.0);
}

// A pattern's lines: how much of the pixel they cover, and the line normal.
struct Lines {
    cover: f32,
    g: vec2<f32>,
};

// How much of the lines at whole values of u, each `w` of the spacing wide,
// falls in a box of half width du about u.
fn hl_area(x: f32, w: f32) -> f32 {
    return floor(x) * w + min(fract(x), w);
}

fn hl_band(u: f32, du: f32, w: f32) -> f32 {
    let x = u + 0.5 * w;
    if (du < 1e-4) {
        return select(0.0, 1.0, fract(x) < w);
    }
    return (hl_area(x + du, w) - hl_area(x - du, w)) / (2.0 * du);
}

// One family of lines u, with gradient gu (per mm), seen over a footprint of
// smm, inside a region of weight m.
fn hl_family(u: f32, gu: vec2<f32>, smm: f32, w: f32, m: f32) -> Lines {
    var l: Lines;
    let gl = length(gu);
    l.cover = m * hl_band(u, 1.2 * gl * smm, w);
    l.g = select(vec2<f32>(1.0, 0.0), gu / gl, gl > 1e-9);
    return l;
}

// Two families woven: their union, the stronger's line normal.
fn hl_weave(a: Lines, b: Lines) -> Lines {
    var l: Lines;
    l.cover = 1.0 - (1.0 - a.cover) * (1.0 - b.cover);
    l.g = select(b.g, a.g, a.cover >= b.cover);
    return l;
}

// A soft annulus from r0 to r1 mm, edges `e` wide.
fn hl_ring(r: f32, r0: f32, r1: f32, e: f32) -> f32 {
    return smoothstep(r0 - e, r0 + e, r) * (1.0 - smoothstep(r1 - e, r1 + e, r));
}

// The film's line patterns at q (mm about the centre, already turned):
// seal, rosette, spiral, lattice, sunburst, or a dot matrix of small
// gratings. Lines are `sp` mm apart and `w` of that wide; the round
// patterns end at radius `rad`.
fn hl_pattern(q: vec2<f32>, kind: u32, smm: f32, inv_d: ptr<function, f32>) -> Lines {
    let pat = p.b[R_H_PATTERN];
    let sp = pat.x;
    let w = pat.y;
    let rad = pat.w;
    let r = max(length(q), 1e-3);
    let rh = q / r;
    let th = vec2<f32>(-rh.y, rh.x);
    let phi = atan2(q.y, q.x);
    let e = max(0.5 * sp, smm);
    var none: Lines;
    none.cover = 0.0;
    none.g = vec2<f32>(1.0, 0.0);
    if (kind == 3u) {
        // Seal: a rim of radial dashes round a woven ring.
        let nr = round(HC_TAU * 0.88 * rad / sp);
        let rim = hl_family(nr * phi / HC_TAU, th * nr / (HC_TAU * r), smm, w,
                            hl_ring(r, 0.80 * rad, 0.96 * rad, e));
        let r1 = 0.22 * rad;
        let r2 = 0.70 * rad;
        let m = round(HC_TAU * 0.46 * rad / (2.0 * sp));
        let x = HC_PI * (r - r1) / (r2 - r1);
        let amp = 1.2;
        let dr = amp * HC_PI / (r2 - r1) * cos(x);
        let mr = hl_ring(r, r1, r2, e);
        let a = hl_family(m * (phi + amp * sin(x)) / HC_TAU,
                          m / HC_TAU * (th / r + dr * rh), smm, w, mr);
        let b = hl_family(m * (phi - amp * sin(x)) / HC_TAU,
                          m / HC_TAU * (th / r - dr * rh), smm, w, mr);
        let ring = hl_family(r / sp, rh / sp, smm, w, hl_ring(r, 0.745 * rad, 0.755 * rad, 0.5 * sp));
        return hl_weave(hl_weave(rim, ring), hl_weave(a, b));
    }
    if (kind == 4u) {
        // Rosette: two families of petalled rings, half a petal apart.
        let k = 8.0;
        let amp = 0.12;
        let mr = 1.0 - smoothstep(rad - e, rad + e, r);
        let ca = cos(k * phi);
        let sa = sin(k * phi);
        let a = hl_family(r * (1.0 + amp * ca) / sp,
                          ((1.0 + amp * ca) * rh - amp * k * sa * th) / sp, smm, w, mr);
        let b = hl_family(r * (1.0 - amp * ca) / sp,
                          ((1.0 - amp * ca) * rh + amp * k * sa * th) / sp, smm, w, mr);
        return hl_weave(a, b);
    }
    if (kind == 5u) {
        // Spiral: six arms winding out.
        let arms = 6.0;
        let mr = 1.0 - smoothstep(rad - e, rad + e, r);
        return hl_family(r / sp + arms * phi / HC_TAU, rh / sp + th * arms / (HC_TAU * r), smm,
                         w, mr);
    }
    if (kind == 6u) {
        // Lattice: two families of waves crossing over the whole film.
        let lw = 6.0 * sp;
        let amp = 1.2 * sp;
        let x = HC_TAU * q.x / lw;
        let dx = amp * HC_TAU / lw * cos(x);
        let a = hl_family((q.y + amp * sin(x)) / sp, vec2<f32>(dx, 1.0) / sp, smm, w, 1.0);
        let b = hl_family((q.y - amp * sin(x)) / sp, vec2<f32>(-dx, 1.0) / sp, smm, w, 1.0);
        return hl_weave(a, b);
    }
    if (kind == 7u) {
        // Sunburst: lines out from the centre over the whole film.
        let nr = round(HC_PI * rad / sp);
        return hl_family(nr * phi / HC_TAU, th * nr / (HC_TAU * r), smm, w, 1.0);
    }
    if (kind == 8u) {
        // Sparkle: a dot matrix, each dot a small grating of its own
        // direction and a slightly different pitch.
        let cell = 1.6 * sp;
        let u = q / cell;
        let c = vec2<i32>(floor(u));
        let h = hr_rnd3(c, 4513u);
        let at = (vec2<f32>(c) + 0.5 + 0.3 * (h.xy - 0.5)) * cell;
        let rd = (0.28 + 0.12 * h.z) * cell;
        var l: Lines;
        l.cover = holo_box(length(q - at), rd, max(smm, 0.05 * rd));
        let a = HC_PI * fract(h.x * 5.17 + h.y * 3.11);
        l.g = vec2<f32>(cos(a), sin(a));
        *inv_d = *inv_d * (1.0 + 0.05 * (fract(h.z * 9.73) - 0.5));
        return l;
    }
    return none;
}

// Every light's replay seen from x (metres on the sheet) toward v: the three
// spots, the panel as one broad light, and the studio as one more around the
// reference direction. `colour` adds each band's colour.
struct Candidates {
    r: array<Replay, 5>,
};

fn hl_candidates(sh: Sheet, x: vec3<f32>, v: vec3<f32>, y_mm: f32, wk: f32,
                 colour: bool) -> Candidates {
    var c: Candidates;
    let t = sh.t;
    let b = sh.b;
    let n = sh.n;
    // One call of the replay in a loop, so FXC compiles it once.
    let spots = holo_spots();
    for (var i = 0u; i < spots + 2u; i = i + 1u) {
        var l: vec3<f32>;
        var rad: f32;
        var a: f32;
        var r_ang: f32;
        var spec = 0u;
        if (i <= spots) {
            let at = R_LIGHTS + 4u * i;
            let lp = p.b[at];
            let dv = lp.xyz - x;
            let dist = length(dv);
            l = dv / max(dist, 1e-6);
            rad = lp.w;
            spec = u32(p.b[at + 1u].w);
            if (i < spots) {
                let sa = clamp(p.b[at + 2u].x / max(dist, 1e-6), 1e-5, 0.999);
                a = HC_PI * sa * sa * max(dot(l, n), 0.0);
                r_ang = asin(sa);
            } else {
                // The panel, as one broad light.
                let du = across(p.b[at + 2u].xyz, l, dist, sh.px);
                let dw = across(p.b[at + 3u].xyz, l, dist, sh.px);
                a = 4.0 * abs(du.x * dw.y - du.y * dw.x);
                r_ang = sqrt(a / (HC_PI * max(dot(l, n), 0.05)));
            }
        } else {
            // The studio, as one more light round the reference.
            let rf = p.b[R_H_REFERENCE];
            l = normalize(rf.x * t + rf.y * b + rf.z * n);
            r_ang = 0.2;
            rad = dot(surround(l, r_ang, 1u), HC_LUMA);
            a = HC_PI * r_ang * r_ang;
        }
        // Written by constant index: FXC cannot keep a loop that writes an
        // array at a counted one.
        let r = hl_replay(t, b, n, v, y_mm, l, rad, a, r_ang, spec, wk, colour);
        if (i == 0u) {
            c.r[0] = r;
        } else if (i == 1u) {
            c.r[1] = r;
        } else if (i == 2u) {
            c.r[2] = r;
        } else if (i == 3u) {
            c.r[3] = r;
        } else {
            c.r[4] = r;
        }
    }
    return c;
}

@compute @workgroup_size(8, 8)
fn hologram(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }
    let xy = vec2<i32>(gid.xy);
    let s = textureLoad(src, xy, 0);
    let mix_amt = p.b[R_N].w;
    if (s.a <= 0.0) {
        textureStore(dst, xy, s * (1.0 - mix_amt));
        return;
    }
    var sh = sheet_at(xy, vec4<f32>(0.0));
    var px = sh.px;
    let p_mm = sh.p_mm;
    let px_mm = p.b[R_SIZE].z;

    // The film's line patterns, seen through the film at their depth: the
    // grating the film's own lighting reads.
    let kind = u32(p.b[R_H_TYPE].w);
    px.sig_f = p.b[R_SPREAD].w;
    px.inv_d = p.b[R_CENTRE].z;
    px.g = vec2<f32>(1.0, 0.0);
    px.sig_th = p.b[R_SPREAD].z;
    px.w = 0.0;
    if (kind >= 3u) {
        let vt = vec2<f32>(dot(px.v, sh.t), dot(px.v, sh.b));
        let din = vec3<f32>(-vt / 1.5, -sqrt(max(1.0 - dot(vt, vt) / HL_N2, 0.0)));
        let pf = p_mm + din.xy * (p.b[R_H_DEPTHS].w / din.z);
        let sp = p.b[R_MAPS].w;
        let cs = vec2<f32>(cos(sp), sin(sp));
        let q = vec2<f32>(cs.x * pf.x + cs.y * pf.y, -cs.y * pf.x + cs.x * pf.y);
        var inv_d = px.inv_d;
        let lines = hl_pattern(q, kind, sh.smm, &inv_d);
        // Back to the film's own axes.
        px.g = vec2<f32>(cs.x * lines.g.x - cs.y * lines.g.y, cs.y * lines.g.x + cs.x * lines.g.y);
        px.inv_d = inv_d;
        let s_px = sh.smm / max(length(q), 0.5);
        px.sig_th = sqrt(px.sig_th * px.sig_th + s_px * s_px);
        px.w = clamp(lines.cover * p.b[R_H_PATTERN].z * sh.mottle, 0.0, 1.0);
        // Thin lines: the panel's rainbow as one cell and the studio's in
        // three bins are enough.
        px.lite = true;
    }
    px.fam = family(px.inv_d);
    px.cspx = px.spx;
    // Seen from behind there is no light on the face, and no light replays
    // the image (each replay is dark where the eye is behind the film).
    let back = dot(sh.n, px.v) <= 0.0;
    var acc: Acc;
    if (!back) {
        acc = lit(px, sh.wear > 0.0, &sh.hair);
    }
    var c = acc.lc + p.b[R_H_REPLAY].y * acc.lz + acc.lo;

    // The replay, for every light, the studio counted as one more around the
    // reference direction. The two brightest are traced; the others reuse
    // the nearer one's image in their own colour.
    var wk = sh.mottle;
    if (has(F_MATTE)) {
        wk = wk * luma_of(textureLoad(matte, xy, 0));
    }
    // The two brightest lights at the film's centre are traced, in the same
    // order everywhere, so no seam runs where two lights trade places; the
    // others reuse the traced image nearer their own direction, in their own
    // colour. A light that gives no image here (behind the film, past the
    // film's edge of replay) has no direction to trace, so it is passed over
    // and the next light that does give one takes its place.
    // Here, then at the centre, through one call.
    var reps: array<Replay, 5>;
    var mid: array<Replay, 5>;
    for (var k = 0u; k < holo_two(); k = k + 1u) {
        let here = k == 0u;
        let c = hl_candidates(sh, select(vec3<f32>(0.0), px.x, here),
                              select(vec3<f32>(0.0, 0.0, 1.0), px.v, here), select(0.0, p_mm.y, here),
                              select(1.0, wk, here), here);
        if (here) {
            reps = c.r;
        } else {
            mid = c.r;
        }
    }
    var first = 0u;
    var second = 0u;
    var best = -1.0;
    var next = -1.0;
    for (var i = 0u; i < 5u; i = i + 1u) {
        if (reps[i].e <= 0.0) {
            continue;
        }
        // Ranked at the centre; lights dark there are ranked by this pixel,
        // after every light that lights the centre.
        let e = mid[i].e + 1e-6 * reps[i].e;
        if (e > best) {
            next = best;
            second = first;
            best = e;
            first = i;
        } else if (e > next) {
            next = e;
            second = i;
        }
    }
    if (best >= 0.0) {
        let stereo = p.b[R_H_TYPE].z > 0.5;
        // Draft traces the brightest light alone; the second then reuses its
        // image, as the others do.
        let has_b = next >= 0.0 && u32(p.b[R_QUALITY].z) > 1u;
        let ra = reps[first];
        let rb = reps[second];
        // Every trace in one loop: the brightest light's one or two views,
        // then the second's.
        let va = hl_views(ra, stereo, true);
        let vb = hl_views(rb, stereo, false);
        let wb = select(0.0, 1.0, has_b);
        var img_a = vec3<f32>(0.0);
        var img_b = vec3<f32>(0.0);
        for (var j = 0u; j < select(2u, 4u, has_b); j = j + 1u) {
            let second_light = j >= 2u;
            let odd = (j & 1u) == 1u;
            var dir = select(va.d0, va.d1, odd);
            var wt = select(va.w0, va.w1, odd);
            var r = ra;
            if (second_light) {
                dir = select(vb.d0, vb.d1, odd);
                wt = wb * select(vb.w0, vb.w1, odd);
                r = rb;
            }
            if (wt <= 0.0) {
                continue;
            }
            let img = wt * hl_image(p_mm, dir, r);
            if (second_light) {
                img_b = img_b + img;
            } else {
                img_a = img_a + img;
            }
        }
        let band_a = max(dot(ra.band, HC_LUMA), 1e-6);
        let band_b = max(dot(rb.band, HC_LUMA), 1e-6);
        var rep = ra.e * img_a + rb.e * img_b;
        for (var i = 0u; i < 5u; i = i + 1u) {
            let ri = reps[i];
            if (i == first || (has_b && i == second) || ri.e <= 0.0) {
                continue;
            }
            // The nearer traced image, recoloured: its brightness in this
            // light's band. Nearer at the centre, or here where one of the
            // three gives no image at the centre.
            let at_mid = mid[i].e > 0.0 && mid[first].e > 0.0 && mid[second].e > 0.0;
            let di = select(ri.dir, mid[i].dir, at_mid);
            let da = select(ra.dir, mid[first].dir, at_mid);
            let db = select(rb.dir, mid[second].dir, at_mid);
            let near_a = dot(di, da) >= dot(di, db) || !has_b;
            var img = img_b / band_b;
            if (near_a) {
                img = img_a / band_a;
            }
            rep = rep + ri.e * dot(img, HC_LUMA) * ri.band;
        }
        c = c + HL_GAIN * rep * hl_speckle(p_mm, ra.dir, px_mm);
    }
    let diffuse = diffuse_light(acc, select(px.n, -sh.n, back), back);
    if (sh.dust > 0.0) {
        let grazing = 1.0 - px.co;
        c = mix(c, 0.4 * diffuse * (1.0 + 3.0 * grazing * grazing), sh.dust);
    }
    // The back of the film: plain satin metal in the light around it.
    if (back) {
        c = BACK_SATIN * diffuse;
    }
    textureStore(dst, xy, finish(c, s));
}
