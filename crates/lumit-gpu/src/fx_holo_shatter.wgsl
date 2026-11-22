// Holographic foil's Shatter material: cracked-ice foil. A Voronoi mosaic is
// cut again by straight chords, and each shard is its own buried, slightly
// tipped linear grating from one of three pitch families. Part of the cracks
// carry an embossed crease. Not a kernel on its own: it is compiled with
// fx_holo_core.wgsl, fx_holo_relief.wgsl and fx_holographic_foil.wgsl, whose
// rows, Px, Acc and lights it uses.
//
// Sizes are sheet millimetres, so shards keep their size at any raster. Near a
// crack the shards on both sides share the pixel by the footprint's exact
// cover, with the crease's slope over the footprint as a mean and a spread,
// so cracks never alias; elsewhere one shard lights the pixel. The crack
// distance is Inigo Quilez's two-pass Voronoi edge distance (MIT,
// iquilezles.org/articles/voronoilines).

// Shard size in mm, irregularity, mean cuts per cell, groove directions.
const R_SHATTER: u32 = 243u;
// Tangent of the shard tilt, chirp share, crease height and half width in mm.
const R_SHATTER_MORE: u32 = 245u;
// Share of creased cracks, a shard's aperture variance in q.
const R_SHATTER_CREASE: u32 = 274u;

// Half a box window with the footprint's variance, in footprint sigmas.
const HS_WINDOW: f32 = 1.7320508;

// The metal's own curvature inside a shard, radians.
const HS_CURVE: f32 = 0.002;

// One shard: its Voronoi cell, which side of each of the cell's cuts it lies
// on, and the cell's feature point in mm.
struct HsShard {
    cell: vec2<i32>,
    bits: u32,
    centre: vec2<f32>,
};

// The cuts of a cell at a point: the side bits, and the nearest cut's
// distance, index and the unit normal pointing across it.
struct HsCuts {
    bits: u32,
    d: f32,
    k: u32,
    e: vec2<f32>,
};

// A cell's feature point in cell units.
fn hs_point(c: vec2<i32>, jit: f32, seed: u32) -> vec2<f32> {
    return vec2<f32>(c) + 0.5 + jit * (hr_rnd3(c, seed ^ 7368787u).xy - 0.5);
}

// Up to three straight cuts through a cell, near its feature point at any
// angle, read at p (mm).
fn hs_cuts(cell: vec2<i32>, centre: vec2<f32>, p_mm: vec2<f32>, size: f32, splits: f32,
           seed: u32) -> HsCuts {
    var out: HsCuts;
    out.bits = 0u;
    out.d = 1e9;
    out.k = 0u;
    out.e = vec2<f32>(1.0, 0.0);
    let salt = seed ^ 2147483647u;
    let n = min(u32(floor(splits + hr_rnd3(cell, salt).x)), 3u);
    for (var k = 0u; k < n; k = k + 1u) {
        let h = hr_rnd3(cell, salt + 101u * (k + 1u));
        let c = centre + 0.6 * size * (h.xy - 0.5);
        let a = HC_PI * h.z;
        let nk = vec2<f32>(-sin(a), cos(a));
        let side = dot(nk, p_mm - c);
        if (side > 0.0) {
            out.bits = out.bits | (1u << k);
        }
        if (abs(side) < out.d) {
            out.d = abs(side);
            out.k = k;
            out.e = select(nk, -nk, side > 0.0);
        }
    }
    return out;
}

// A number fixed for one shard, the same seen from either side of a crack.
fn hs_key(s: HsShard, seed: u32) -> u32 {
    return hr_pcg3(vec3<u32>(bitcast<u32>(s.cell.x), bitcast<u32>(s.cell.y),
                             seed ^ (s.bits * 7919u + 104729u))).x;
}

// The crease along the crack between a and b: its height in mm, 0 for none.
fn hs_crease(a: HsShard, b: HsShard, seed: u32) -> f32 {
    let ka = hs_key(a, seed);
    let kb = hs_key(b, seed);
    let h = vec2<f32>(hr_pcg3(vec3<u32>(ka ^ kb, ka + kb, seed + 31u)).xy >> vec2<u32>(8u))
        * (1.0 / 16777216.0);
    let share = p.b[R_SHATTER_CREASE].x;
    return select(0.0, p.b[R_SHATTER_MORE].z * h.y, h.x < share);
}

// Shard s as an element: its own tilt (and the crease slope `crease`) under
// the lacquer, a groove direction, a pitch family with its chirp, and its own
// coverage. `kt` turns a buried slope into the air slope that bends the light
// as far.
fn hs_element(base: Px, s: HsShard, p_mm: vec2<f32>, crease: vec2<f32>, kt: f32, wk: f32,
              extra: vec3<f32>, seed: u32) -> Px {
    let sh = p.b[R_SHATTER];
    let more = p.b[R_SHATTER_MORE];
    let salt = seed ^ (s.bits << 4u);
    let a = hr_rnd3(s.cell, salt ^ 40503u);
    let b = hr_rnd3(s.cell, salt ^ 92821u);
    var th = a.x * HC_PI;
    if (sh.w > 0.5) {
        th = (floor(a.x * sh.w) + 0.5) * HC_PI / sh.w;
    }
    th = th + p.b[R_MAPS].w;
    let gc = vec2<f32>(cos(th), sin(th));
    let slope = more.x * hr_gauss2(a.yz) + crease;
    let f = tipped(base.t, base.b, base.n, kt * slope);
    var px = base;
    px.t = f.t;
    px.b = f.b;
    px.n = f.n;
    px.vt = vec2<f32>(dot(px.v, f.t), dot(px.v, f.b));
    px.co = max(dot(px.v, f.n), 1e-4);
    let nc = lacquer();
    px.co_in = sqrt(max(1.0 - dot(px.vt, px.vt) / (nc * nc), 0.0));
    px.g = gc;
    let fam = min(u32(b.x * 3.0), 2u);
    let chirp = 1.0 + more.y * (b.y - 0.5) * dot(p_mm - s.centre, gc) / sh.x;
    px.inv_d = p.b[R_FAMILIES][fam] * max(chirp, 0.1);
    px.fam = f32(fam);
    px.sig_f = 0.0;
    px.sig_th = 0.0;
    px.spx = px.spx + extra;
    let cover = clamp(p.b[R_GRATING].w + 0.2 * (b.z - 0.5), 0.0, 1.0);
    px.w = clamp(cover * wk, 0.0, 1.0);
    return px;
}

fn hs_add(acc: ptr<function, Acc>, a: Acc, w: f32) {
    (*acc).lo = (*acc).lo + w * a.lo;
    (*acc).lz = (*acc).lz + w * a.lz;
    (*acc).lc = (*acc).lc + w * a.lc;
    (*acc).ed = (*acc).ed + w * a.ed;
}

// The crease's height at signed distance x from its crack:
// h = H (1 - (x / W)^2)^2 inside W.
fn hs_h(x: f32, hgt: f32, wid: f32) -> f32 {
    let r = min(abs(x) / wid, 1.0);
    return hgt * (1.0 - r * r) * (1.0 - r * r);
}

// The integral of the crease's slope squared from 0 to x.
fn hs_s2(x: f32, hgt: f32, wid: f32) -> f32 {
    let u = clamp(x, -wid, wid) / wid;
    let u3 = u * u * u;
    return 16.0 * hgt * hgt / wid * (u3 / 3.0 - 0.4 * u3 * u * u + u3 * u3 * u / 7.0);
}

// The crease's mean slope and its variance over the stretch a..b of signed
// distance, as a slope vector across the crack (e points across it) and a
// covariance. A crease thinner than the stretch spreads into the variance
// instead of being sampled, so it never breaks into dashes.
struct HsSlope {
    m: vec2<f32>,
    cv: vec3<f32>,
};

fn hs_stretch(a: f32, b: f32, hgt: f32, wid: f32, e: vec2<f32>) -> HsSlope {
    var out: HsSlope;
    let len = max(b - a, 1e-6);
    // Distance falls toward the crack, so the slope along e is -dh/dx.
    let m = -(hs_h(b, hgt, wid) - hs_h(a, hgt, wid)) / len;
    let m2 = (hs_s2(b, hgt, wid) - hs_s2(a, hgt, wid)) / len;
    out.m = m * e;
    out.cv = max(m2 - m * m, 0.0) * hr_outer(e);
    return out;
}

// Shatter at p (mm), footprint smm (mm), on the pixel `base` describes. wk is
// what scales the coverage here (mottling, matte).
fn holo_shatter(p_mm: vec2<f32>, smm: f32, base: Px, wk: f32, wear: bool,
                hair: ptr<function, HoloHairlines>) -> Acc {
    let sh = p.b[R_SHATTER];
    let size = sh.x;
    let seed = u32(p.b[R_RELIEF_MORE].z);
    let s2 = 2.0 * size;
    let u = p_mm / s2;

    // Pass one: the nearest feature point is this pixel's cell.
    let home = vec2<i32>(floor(u));
    var best = 1e9;
    var id = home;
    var mr = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let c = home + vec2<i32>(i, j);
            let r = hs_point(c, sh.y, seed) - u;
            let d = dot(r, r);
            if (d < best) {
                best = d;
                id = c;
                mr = r;
            }
        }
    }
    // Pass two: the exact distance to the nearest bisector.
    var de = 1e9;
    var en = vec2<f32>(1.0, 0.0);
    var nid = id;
    for (var j = -2; j <= 2; j = j + 1) {
        for (var i = -2; i <= 2; i = i + 1) {
            let c = id + vec2<i32>(i, j);
            if (i == 0 && j == 0) {
                continue;
            }
            let r = hs_point(c, sh.y, seed) - u;
            let dr = r - mr;
            let l2 = dot(dr, dr);
            if (l2 < 1e-10) {
                continue;
            }
            let dn = dr * inverseSqrt(l2);
            let d = dot(0.5 * (mr + r), dn);
            if (d < de) {
                de = d;
                en = dn;
                nid = c;
            }
        }
    }

    var own: HsShard;
    own.cell = id;
    own.centre = (u + mr) * s2;
    let cuts = hs_cuts(id, own.centre, p_mm, size, sh.z, seed);
    own.bits = cuts.bits;

    // The nearest crack: a cut inside the cell, or the cell's own edge.
    var other = own;
    var dist = de * s2;
    var e = en;
    if (cuts.d < dist) {
        dist = cuts.d;
        e = cuts.e;
        other.bits = own.bits ^ (1u << cuts.k);
    } else {
        other.cell = nid;
        other.centre = hs_point(nid, sh.y, seed) * s2;
        let foot = p_mm + (dist + min(smm, 0.1 * size) + 1e-4) * e;
        other.bits = hs_cuts(nid, other.centre, foot, size, sh.z, seed).bits;
    }

    // How far a buried slope bends the light compared with one in air, for
    // the brightest light (spec 2.2's kTilt).
    let eye = vec3<f32>(0.0, 0.0, p.b[R_SIZE].w);
    let lk = normalize(key_light(eye) - base.x);
    let nc = lacquer();
    let ltk = vec2<f32>(dot(lk, base.t), dot(lk, base.b));
    let cik = sqrt(max(1.0 - dot(ltk, ltk) / (nc * nc), 0.0));
    let sn = nc * (cik + base.co_in);
    let snk = max(dot(base.n, lk + base.v), 0.25);
    let kt = sn / snk;
    // The metal's curvature and the shard's aperture widen every lobe. Shards
    // far smaller than the pixel blend into a spread of tilts and grooves.
    let fine = smoothstep(0.12, 0.4, smm / size);
    let tilt_q = kt * snk * p.b[R_SHATTER_MORE].x;
    let iso = (sn * HS_CURVE) * (sn * HS_CURVE) + p.b[R_SHATTER_CREASE].y
        + fine * tilt_q * tilt_q;
    let extra = vec3<f32>(iso, 0.0, iso);

    // The crack is filtered over the pixel: the shard on this side covers
    // the normal CDF of the distance, exactly, and the other shard the rest.
    // Each side takes the crease's mean slope and spread over its part of a
    // window as wide as the footprint, so a crease the pixel cannot resolve
    // widens the lobes instead of breaking into dashes.
    let hgt = hs_crease(own, other, seed);
    let wid = p.b[R_SHATTER_MORE].w;
    let reach = select(0.0, wid, hgt != 0.0) + 3.0 * smm;
    var c_own = 1.0;
    var mine: HsSlope;
    var theirs: HsSlope;
    if (dist < reach) {
        c_own = select(holo_cdf(dist / max(smm, 1e-6)), 1.0, dist > 2.5 * smm);
        if (hgt != 0.0) {
            let half = HS_WINDOW * smm;
            mine = hs_stretch(max(dist - half, 0.0), dist + half, hgt, wid, e);
            theirs = hs_stretch(min(dist - half, -1e-5), 0.0, hgt, wid, e);
        }
    }

    var acc: Acc;
    acc.lo = vec3<f32>(0.0);
    acc.lz = vec3<f32>(0.0);
    acc.lc = vec3<f32>(0.0);
    acc.ed = vec3<f32>(0.0);
    let q_tilt = (kt * snk) * (kt * snk);
    // This side's shard, then the other's where the crack reaches, through
    // one call of the lighting so FXC compiles it once.
    let n = select(1u, 2u, c_own < 1.0);
    for (var k = 0u; k < n; k = k + 1u) {
        let first = k == 0u;
        var shard = own;
        var side = mine;
        if (!first) {
            shard = other;
            side = theirs;
        }
        var el = hs_element(base, shard, p_mm, side.m, kt, wk, extra + q_tilt * side.cv, seed);
        el.sig_th = fine * 1.5;
        hs_add(&acc, lit(el, wear, hair), select(1.0 - c_own, c_own, first));
    }
    return acc;
}
