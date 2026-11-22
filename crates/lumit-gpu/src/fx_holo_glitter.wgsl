// Holographic foil's Glitter material: metallised embossed flakes (hex, square
// or round) in a dyed binder over a dark base ink, in one to three stacked
// layers, with sparse larger stars whose grooves run round. Each flake has its
// own tilt, groove direction and pitch, so each flashes alone. Not a kernel on
// its own: it is compiled with fx_holo_core.wgsl, fx_holo_relief.wgsl and
// fx_holographic_foil.wgsl, whose rows, Px, Acc and lights it uses. Its
// lattice, cover and candidate helpers serve fx_holo_sparkle.wgsl too.
//
// Sizes are sheet millimetres, so flakes keep their size at any raster. The
// flakes that cover enough of the pixel are lit one by one, front to back,
// each hiding what lies under it. The rest join a mean field: grooves turned
// every way give rainbow rings in q, and the tilts spread the mirror. As
// flakes grow many to a pixel, each leaves the lit set at its own fixed point,
// so the hand-over thins the sparkle instead of fading it.

// Flake size in mm, shape, layers, tangent of the flake tilt.
const R_GLITTER: u32 = 280u;
// Flake curve in radians, 1 / widest pitch, 1 / finest pitch, a flake's
// aperture variance in q.
const R_GLITTER_MORE: u32 = 281u;
// Star size in mm, share of star places filled.
const R_STARS: u32 = 282u;
// The base ink's colour.
const R_BASE_INK: u32 = 283u;

// Which table each spectrum reads (x to w for the first four, then the fifth
// in the next row's x), or -1 for none.
const R_TABLE_MAP: u32 = 288u;
// The band colour tables: per table, 10 widths of 96 wavelengths.
const R_TABLE: u32 = 290u;
const HG_TABLE_L: u32 = 96u;
const HG_TABLE_S: u32 = 10u;

// The colour of a band centred at lam nm, sig nm wide, under spectrum
// `spec`, read off its table: wavelengths from 280 nm every 7, widths from
// 0.5 nm doubling every 0.9 steps. Exact sums where no table was built.
fn holo_band_table(lam: f32, sig: f32, spec: u32) -> vec3<f32> {
    let maps = select(p.b[R_TABLE_MAP], p.b[R_TABLE_MAP + 1u], spec > 3u);
    let slot = maps[spec % 4u];
    if (slot < 0.0) {
        return band(lam, sig, spec);
    }
    let x = clamp((lam - 280.0) / 7.0, 0.0, f32(HG_TABLE_L - 1u));
    let y = clamp(0.9 * log2(max(sig, 0.5) / 0.5), 0.0, f32(HG_TABLE_S - 1u));
    let i = min(u32(x), HG_TABLE_L - 2u);
    let j = min(u32(y), HG_TABLE_S - 2u);
    let fx = x - f32(i);
    let fy = y - f32(j);
    let at = R_TABLE + u32(slot) * HG_TABLE_L * HG_TABLE_S + j * HG_TABLE_L + i;
    let lo = mix(p.b[at].xyz, p.b[at + 1u].xyz, fx);
    let hi = mix(p.b[at + HG_TABLE_L].xyz, p.b[at + HG_TABLE_L + 1u].xyz, fx);
    return mix(lo, hi, fy);
}

// Below this share of the pixel an element joins the mean field.
const HG_LEAST: f32 = 0.05;
// Below this share a second element joins it too.
const HG_SECOND: f32 = 0.12;

// Salts for a cell's second and third numbers.
const HG_SALT_B: u32 = 1597334677u;
const HG_SALT_E: u32 = 3812015801u;

// A lattice turned by an angle: unit axes, cell size in mm, offset in cells.
struct HgLattice {
    e0: vec2<f32>,
    e1: vec2<f32>,
    cell: f32,
    off: vec2<f32>,
};

fn hg_lattice(deg: f32, cell: f32, off: vec2<f32>) -> HgLattice {
    var l: HgLattice;
    let a = radians(deg);
    l.e0 = vec2<f32>(cos(a), sin(a));
    l.e1 = vec2<f32>(-sin(a), cos(a));
    l.cell = cell;
    l.off = off;
    return l;
}

// A sheet point, mm, in lattice cells.
fn hg_cells(l: HgLattice, p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(dot(p, l.e0), dot(p, l.e1)) / l.cell + l.off;
}

// A point in lattice cells on the sheet, mm.
fn hg_mm(l: HgLattice, u: vec2<f32>) -> vec2<f32> {
    let v = (u - l.off) * l.cell;
    return v.x * l.e0 + v.y * l.e1;
}

// The cells to look in around u: the first, and how many a side. Three by
// three (two by two on Draft), or five by five when `far` allows and the
// footprint (sc, in cells) is wider than Quality's threshold.
fn hg_window(u: vec2<f32>, sc: f32, far: bool) -> vec3<i32> {
    let q = p.b[R_QUALITY + 2u];
    let half = select(q.x, 2.5, far && sc > q.y);
    let start = vec2<i32>(floor(u - half + 0.5));
    return vec3<i32>(start, i32(2.0 * half));
}

// How much of the footprint the window's cells reach.
fn hg_reach(u: vec2<f32>, sc: f32, w: vec3<i32>) -> f32 {
    let s = max(sc, 1e-6);
    let lo = vec2<f32>(w.xy);
    let hi = lo + f32(w.z);
    return (holo_cdf((hi.x - u.x) / s) - holo_cdf((lo.x - u.x) / s))
        * (holo_cdf((hi.y - u.y) / s) - holo_cdf((lo.y - u.y) / s));
}

// How much of the pixel a shape covers. sd is the signed distance from the
// pixel's centre to its outline (negative inside), area its area, d2 the
// squared distance to its middle, size its width, s the footprint sigma, all
// in mm. A shape far smaller than the pixel counts as a point.
fn hg_cover(sd: f32, area: f32, d2: f32, size: f32, s: f32) -> f32 {
    let v = s * s + size * size / 16.0;
    let point = area * exp(-0.5 * d2 / v) / (HC_TAU * v);
    let big = holo_cdf(-sd / max(s, 1e-6));
    return clamp(mix(point, big, smoothstep(0.5, 2.0, 0.5 * size / max(s, 1e-6))), 0.0, 1.0);
}

// Whether an element whose own number is r stays lit on its own while
// `shown` of the explicit set is kept: 1 at shown 1, 0 at shown 0.
fn hg_keep(r: f32, shown: f32) -> f32 {
    let c = 0.1 + 0.8 * r;
    return smoothstep(c - 0.1, c + 0.1, shown);
}

// What the candidate loop has found: how much of the pixel is still seen past
// the elements in front, the two heaviest elements to light on their own (cell
// x, cell y, which lattice, weight; the heavier first), the weight left to the
// mean field, and the weight of elements drawn another way (sparkle's specks).
struct HgRun {
    seen: f32,
    n: u32,
    p0: vec4<f32>,
    p1: vec4<f32>,
    rest: f32,
    other: f32,
    // The flashes the mean field smooths away, from flakes that joined it.
    flick: vec3<f32>,
    // Each pick's own flashes, kept in case it joins the field later.
    f0: vec3<f32>,
    f1: vec3<f32>,
};

fn hg_run() -> HgRun {
    var r: HgRun;
    r.seen = 1.0;
    r.n = 0u;
    r.rest = 0.0;
    r.other = 0.0;
    r.flick = vec3<f32>(0.0);
    r.f0 = vec3<f32>(0.0);
    r.f1 = vec3<f32>(0.0);
    return r;
}

// Hands an element covering `cover` of the pixel to the run, with its own
// flashes `fl`. The two heaviest kept elements that cover enough are lit on
// their own; the rest of every weight joins the mean field.
fn hg_take(run: ptr<function, HgRun>, cover: f32, keep: f32, id: vec3<f32>, fl: vec3<f32>) {
    let w = cover * (*run).seen;
    let k = w * keep;
    var lit_w = 0.0;
    if (k >= HG_LEAST) {
        lit_w = k * smoothstep(HG_LEAST, 2.0 * HG_LEAST, k);
        let v = vec4<f32>(id, lit_w);
        if ((*run).n == 0u) {
            (*run).p0 = v;
            (*run).f0 = fl;
            (*run).n = 1u;
        } else if ((*run).n == 1u) {
            if (lit_w > (*run).p0.w) {
                (*run).p1 = (*run).p0;
                (*run).f1 = (*run).f0;
                (*run).p0 = v;
                (*run).f0 = fl;
            } else {
                (*run).p1 = v;
                (*run).f1 = fl;
            }
            (*run).n = 2u;
        } else if (lit_w > (*run).p1.w) {
            (*run).rest = (*run).rest + (*run).p1.w;
            if (lit_w > (*run).p0.w) {
                (*run).p1 = (*run).p0;
                (*run).f1 = (*run).f0;
                (*run).p0 = v;
                (*run).f0 = fl;
            } else {
                (*run).p1 = v;
                (*run).f1 = fl;
            }
        } else {
            lit_w = 0.0;
        }
    }
    (*run).rest = (*run).rest + w - lit_w;
}

// The run's pick i.
fn hg_pick(run: HgRun, i: u32) -> vec4<f32> {
    return select(run.p1, run.p0, i == 0u);
}

// How a buried slope bends light against one in air, for the brightest light
// (spec 2.2's kTilt), and that light's n (c_in + c_out) inside the lacquer.
fn hg_key(base: Px) -> vec2<f32> {
    let eye = vec3<f32>(0.0, 0.0, p.b[R_SIZE].w);
    let lk = normalize(key_light(eye) - base.x);
    let nc = lacquer();
    let ltk = vec2<f32>(dot(lk, base.t), dot(lk, base.b));
    let cik = sqrt(max(1.0 - dot(ltk, ltk) / (nc * nc), 0.0));
    let sn = nc * (cik + base.co_in);
    let snk = max(dot(base.n, lk + base.v), 0.25);
    return vec2<f32>(sn / snk, sn);
}

// An element of its own on the pixel `base` describes: tipped by a buried
// slope, with grooves g, 1 / pitch, coverage w and extra spread iso (q
// squared). The coat is left to the call that lights the base.
fn hg_element(base: Px, slope: vec2<f32>, kt: f32, g: vec2<f32>, inv_d: f32, w: f32,
              iso: f32) -> Px {
    let f = tipped(base.t, base.b, base.n, kt * slope);
    var px = base;
    px.t = f.t;
    px.b = f.b;
    px.n = f.n;
    px.vt = vec2<f32>(dot(px.v, f.t), dot(px.v, f.b));
    px.co = max(dot(px.v, f.n), 1e-4);
    let nc = lacquer();
    px.co_in = sqrt(max(1.0 - dot(px.vt, px.vt) / (nc * nc), 0.0));
    px.g = g;
    px.inv_d = inv_d;
    px.fam = family(inv_d);
    px.sig_f = 0.0;
    px.sig_th = 0.0;
    px.spx = px.spx + vec3<f32>(iso, 0.0, iso);
    px.w = w;
    px.gloss = 0.0;
    px.ring = 0.0;
    return px;
}

// The orders of a population of gratings turned every way: four groove
// directions a quarter turn apart, each spread over its quarter, so the sum
// holds for a light of any size and shape (a long panel too). s is the spread
// in q (the tilts, the light, the pixel), as orders() takes it.
fn hg_rings(px: Px, q: vec2<f32>, s: vec3<f32>, sn: f32, spec: u32, eta: vec4<f32>) -> vec3<f32> {
    let rho = length(q);
    let soft = p.b[R_SPREAD].y * sn;
    let iso2 = soft * soft;
    let d = 1.0 / px.inv_d;
    let depth = p.b[R_CENTRE].w;
    // A quarter of a half turn, as a spread that leaves the sum smooth.
    let quarter = 0.34;
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < holo_four(); k = k + 1u) {
        // The groove direction k eighths of a turn round, as constants.
        let gx = select(select(-0.70710677, -4.371139e-8, k == 2u),
                        select(0.70710677, 1.0, k == 0u), k < 2u);
        let gy = select(select(0.0, 0.70710677, k > 0u), 1.0, k == 2u);
        let s00 = gx * gx * s.x + 2.0 * gx * gy * s.y + gy * gy * s.z;
        let s11 = gy * gy * s.x - 2.0 * gx * gy * s.y + gx * gx * s.z;
        let s01 = -gx * gy * s.x + (gx * gx - gy * gy) * s.y + gx * gy * s.z;
        let qg = gx * q.x + gy * q.y;
        let qt = -gy * q.x + gx * q.y;
        let sgg = max(s00 + iso2 + (rho * px.sig_f) * (rho * px.sig_f), 1e-10);
        let stt = max(s11 + iso2 + (rho * quarter) * (rho * quarter), 1e-10);
        let qgc = qg - (s01 / stt) * qt;
        let sgc = sqrt(max(sgg - s01 * s01 / stt, 1e-10));
        let n1 = exp(-0.5 * qt * qt / stt) * inverseSqrt(HC_TAU * stt);
        if (n1 < 1e-5) {
            continue;
        }
        for (var m = 1u; m <= holo_three(); m = m + 1u) {
            let mf = f32(m);
            let lam = d * abs(qgc) / mf;
            let sig_l = clamp(d * sgc / mf, 0.5, 512.0);
            if (lam + 3.0 * sig_l < 360.0 || lam - 3.0 * sig_l > 830.0) {
                continue;
            }
            let jm = holo_bessel(m, min(HC_PI * depth * sn / max(lam, 1.0), 5.5));
            let e = eta_at(eta, lam).x;
            acc = acc + (0.25 * n1 * (d / mf) * jm * jm * e) * holo_band_table(lam, sig_l, spec);
        }
    }
    return acc;
}

// Signed distance from local point q to a flake outline `size` wide: hex (flat
// to flat, Inigo Quilez's hexagon, MIT, iquilezles.org/articles/
// distfunctions2d), square or round.
fn hg_shape(q: vec2<f32>, size: f32, shape: u32) -> f32 {
    let r = 0.5 * size;
    if (shape == 0u) {
        let k = vec3<f32>(-0.8660254, 0.5, 0.57735027);
        var a = abs(q);
        a = a - 2.0 * min(dot(k.xy, a), 0.0) * k.xy;
        a = a - vec2<f32>(clamp(a.x, -k.z * r, k.z * r), r);
        return length(a) * sign(a.y);
    }
    if (shape == 1u) {
        let e = abs(q) - vec2<f32>(r);
        return length(max(e, vec2<f32>(0.0))) + min(max(e.x, e.y), 0.0);
    }
    return length(q) - r;
}

// A flake outline's area over its width squared.
fn hg_area(shape: u32) -> f32 {
    return select(select(0.7853982, 1.0, shape == 1u), 0.8660254, shape == 0u);
}

// Flake layer `layer` 0 to 2, or 3 for the stars.
fn hg_glitter_lattice(layer: u32) -> HgLattice {
    if (layer == 3u) {
        return hg_lattice(19.0, 2.0 * p.b[R_STARS].x, vec2<f32>(0.597, 0.333));
    }
    let deg = select(select(71.0, 37.0, layer == 1u), 0.0, layer == 0u);
    return hg_lattice(deg, 1.25 * p.b[R_GLITTER].x, f32(layer) * vec2<f32>(0.371, 0.619));
}

fn hg_salt(layer: u32) -> u32 {
    return u32(p.b[R_RELIEF_MORE].z) ^ ((layer + 1u) << 8u);
}

// A flake's or star's middle in mm and its outline's turn, from its cell.
fn hg_place(lat: HgLattice, c: vec2<i32>, salt: u32) -> vec4<f32> {
    let a = hr_rnd3(c, salt);
    let rot = HC_TAU * a.z;
    return vec4<f32>(hg_mm(lat, vec2<f32>(c) + 0.5 + 0.2 * (a.xy - 0.5)), cos(rot), sin(rot));
}

// A spot as the flakes below one pixel see it: q on the untilted foil, its
// footprint with the pixel's, the light it brings (radiance, solid angle and
// the coat's two passes), n (c_in + c_out) inside the lacquer and in air, its
// spectrum, the colour a flake's flat metal mirrors, and what the mean field
// gives one average flake of it. e is 0 for a spot that is off.
struct HgSpot {
    q: vec2<f32>,
    s: vec3<f32>,
    e: f32,
    sn: f32,
    snk: f32,
    spec: u32,
    slot: u32,
    mrgb: vec3<f32>,
    mean: vec3<f32>,
    // A flake's footprint (with its curve and aperture), its inverse and the
    // Gaussian's norm.
    sf: vec3<f32>,
    inv: vec3<f32>,
    norm: f32,
};

struct HgSpots {
    s: array<HgSpot, 3>,
};

// A normalised Gaussian in q of covariance s (xx, xy, yy), read at q.
fn hg_n2(q: vec2<f32>, s: vec3<f32>) -> f32 {
    let det = max(s.x * s.z - s.y * s.y, 1e-12);
    let m = (s.z * q.x * q.x - 2.0 * s.y * q.x * q.y + s.x * q.y * q.y) / det;
    return exp(-0.5 * m) / (HC_TAU * sqrt(det));
}

fn hg_spot(base: Px, i: u32, field: Px, kt: f32, iso: f32) -> HgSpot {
    var o: HgSpot;
    o.e = 0.0;
    let at = R_LIGHTS + 4u * i;
    let lp = p.b[at];
    if (lp.w <= 0.0) {
        return o;
    }
    let dv = lp.xyz - base.x;
    let dist = length(dv);
    let l = dv / dist;
    let sa = clamp(p.b[at + 2u].x / dist, 1e-5, 0.999);
    let cc = dot(l, base.n);
    if (cc <= 0.0) {
        return o;
    }
    let lc = vec2<f32>(dot(l, base.t), dot(l, base.b));
    let lcl = length(lc);
    let er = select(vec2<f32>(1.0, 0.0), lc / lcl, lcl > 1e-6);
    let ep = vec2<f32>(-er.y, er.x);
    let ar = max(sa * cc, 1e-5);
    let ap = sa;
    o.q = lc + base.vt;
    o.s = base.spx + 0.5 * (ar * ar * hr_outer(er) + ap * ap * hr_outer(ep));
    let nc = lacquer();
    o.sn = nc * (sqrt(max(1.0 - dot(lc, lc) / (nc * nc), 0.0)) + base.co_in);
    o.snk = dot(base.n, l + base.v);
    let tc = (1.0 - holo_fresnel(cc, 1.5)) * (1.0 - holo_fresnel(base.co, 1.5));
    o.e = lp.w * HC_PI * ar * ap * tc;
    o.spec = u32(p.b[at + 1u].w);
    o.slot = i;
    o.mrgb = mirror_rgb(o.spec, o.sn, base.w, eta_nodes(R_ETA_MIRROR, 4u, i, 1.0));
    // The flakes' tilts move q by snk kt slope.
    let tq = o.snk * kt * p.b[R_GLITTER].w;
    let sm = o.s + vec3<f32>(tq * tq + iso, 0.0, tq * tq + iso);
    o.mean = o.e * o.mrgb * hg_n2(o.q, sm)
        + (o.e * base.w) * hg_rings(field, o.q, sm, o.sn, o.spec, eta_nodes(R_ETA, 4u, i, 1.0));
    o.sf = o.s + vec3<f32>(iso, 0.0, iso);
    let det = max(o.sf.x * o.sf.z - o.sf.y * o.sf.y, 1e-12);
    o.inv = vec3<f32>(o.sf.z, -o.sf.y, o.sf.x) / det;
    o.norm = 1.0 / (HC_TAU * sqrt(det));
    return o;
}

// One flake's flash from spot o, less what the mean field gives it: its
// mirror, and its first two orders from grooves g of 1 / pitch inv_d, as
// orders() works them for a flake.
fn hg_flick(o: HgSpot, g: vec2<f32>, inv_d: f32, fam: f32, w: f32, slope: vec2<f32>,
            kt: f32, dim: f32) -> vec3<f32> {
    if (o.e <= 0.0) {
        return vec3<f32>(0.0);
    }
    let q = o.q + (o.snk * kt) * slope;
    let s = o.sf;
    let mq = o.inv.x * q.x * q.x + 2.0 * o.inv.y * q.x * q.y + o.inv.z * q.y * q.y;
    // The mirror's glint, rolled off as the mean field's is.
    var f = o.mrgb * (o.norm * exp(-0.5 * mq));
    let s11 = g.y * g.y * s.x - 2.0 * g.x * g.y * s.y + g.x * g.x * s.z;
    let stt = max(s11, 1e-10);
    let qt = -g.y * q.x + g.x * q.y;
    // Most flakes lie far off the line that lights them.
    if (qt * qt < 16.0 * stt && w > 0.0) {
        let s00 = g.x * g.x * s.x + 2.0 * g.x * g.y * s.y + g.y * g.y * s.z;
        let s01 = -g.x * g.y * s.x + (g.x * g.x - g.y * g.y) * s.y + g.x * g.y * s.z;
        let qgc = dot(q, g) - (s01 / stt) * qt;
        let sgc = sqrt(max(s00 - s01 * s01 / stt, 1e-10));
        let n1 = exp(-0.5 * qt * qt / stt) * inverseSqrt(HC_TAU * stt);
        let d = 1.0 / inv_d;
        let depth = p.b[R_CENTRE].w;
        let eta = eta_nodes(R_ETA, 4u, o.slot, fam);
        // Orders 1 and 2, and 3 on High: the third is faint and the field
        // keeps its mean.
        for (var m = 1u; m <= u32(p.b[R_QUALITY + 1u].x); m = m + 1u) {
            let mf = f32(m);
            let lam = d * abs(qgc) / mf;
            let sig_l = clamp(d * sgc / mf, 0.5, 512.0);
            if (lam + 3.0 * sig_l < 360.0 || lam - 3.0 * sig_l > 830.0) {
                continue;
            }
            let jm = holo_bessel(m, min(HC_PI * depth * o.sn / max(lam, 1.0), 5.5));
            f = f + (w * n1 * (d / mf) * jm * jm * eta_at(eta, lam).x)
                * holo_band_table(lam, sig_l, o.spec);
        }
    }
    return (1.0 - dim) * (o.e * f - o.mean);
}

// Flake c's flashes from every spot, less the mean field's share. pitch is
// where its pitch sits in the range, 0 to 1.
fn hg_flicks(spots: HgSpots, w: f32, c: vec2<i32>, salt: u32, pitch: f32, kt: f32,
             dim: vec3<f32>) -> vec3<f32> {
    let gl = p.b[R_GLITTER];
    let more = p.b[R_GLITTER_MORE];
    let b = hr_rnd3(c, salt ^ HG_SALT_B);
    let th = HC_PI * b.z;
    let g = vec2<f32>(cos(th), sin(th));
    let inv_d = mix(more.y, more.z, pitch);
    let fam = family(inv_d);
    let slope = gl.w * hr_gauss2(b.xy);
    var f = vec3<f32>(0.0);
    for (var i = 0u; i < holo_spots(); i = i + 1u) {
        f = f + hg_flick(spots.s[i], g, inv_d, fam, w, slope, kt, dim[i]);
    }
    return f;
}

// One lattice of flakes (layer 0 to 2) or of stars (3) seen through the
// pixel, front to back.
fn hg_lattice_pass(run: ptr<function, HgRun>, p_mm: vec2<f32>, smm: f32, layer: u32,
                   shown: f32, flick: bool, spots: HgSpots, base: Px, kt: f32) {
    let gl = p.b[R_GLITTER];
    let stars = p.b[R_STARS];
    let lat = hg_glitter_lattice(layer);
    let salt = hg_salt(layer);
    let star = layer == 3u;
    let shape = select(u32(gl.y), 2u, star);
    let size0 = select(gl.x, stars.x, star);
    // The share of the sheet these cover, on average.
    var mean = hg_area(shape) * size0 * size0 / (lat.cell * lat.cell);
    if (star) {
        mean = stars.y * 0.7853982 * 1.0533 * size0 * size0 / (lat.cell * lat.cell);
    }
    let u = hg_cells(lat, p_mm);
    let sc = smm / lat.cell;
    let win = hg_window(u, sc, false);
    let reach = 0.75 * size0 + 3.5 * smm;
    var sum = 0.0;
    for (var j = 0; j < win.z; j = j + 1) {
        for (var i = 0; i < win.z; i = i + 1) {
            let c = win.xy + vec2<i32>(i, j);
            let pl = hg_place(lat, c, salt);
            let dp = p_mm - pl.xy;
            let d2 = dot(dp, dp);
            if (d2 > reach * reach) {
                continue;
            }
            let e = hr_rnd3(c, salt ^ HG_SALT_E);
            var size = size0;
            var r = e.y;
            if (star) {
                if (e.y >= stars.y) {
                    continue;
                }
                size = size0 * (0.6 + 0.8 * e.z);
                r = e.y / max(stars.y, 1e-6);
            }
            let q = vec2<f32>(pl.z * dp.x + pl.w * dp.y, -pl.w * dp.x + pl.z * dp.y);
            let area = hg_area(shape) * size * size;
            let cover = hg_cover(hg_shape(q, size, shape), area, d2, size, smm);
            if (cover <= 1e-4) {
                continue;
            }
            sum = sum + cover;
            let kp = hg_keep(r, shown);
            // The share the thinning drops still flashes on its own; a pick's
            // flashes are kept with it, in case it joins the field later.
            let dropped = cover * (*run).seen * (1.0 - kp);
            var fl = vec3<f32>(0.0);
            if (flick && !star && (dropped > 4e-3 || cover * (*run).seen * kp >= HG_LEAST)) {
                fl = hg_flicks(spots, base.w, c, salt, e.x, kt, base.dim);
            }
            hg_take(run, cover, kp, vec3<f32>(vec2<f32>(c), f32(layer)), fl);
            if (flick && !star && dropped > 4e-3) {
                (*run).flick = (*run).flick + dropped * fl;
            }
        }
    }
    // What the window cannot reach joins the mean field, at the mean cover.
    let miss = mean * (1.0 - hg_reach(u, sc, win));
    (*run).rest = (*run).rest + (*run).seen * miss;
    (*run).seen = (*run).seen * max(1.0 - sum - miss, 0.0);
}

// The flake or star a pick names, as an element.
fn hg_glitter_element(base: Px, k: vec4<f32>, p_mm: vec2<f32>, smm: f32, kt: f32,
                      sn: f32) -> Px {
    let gl = p.b[R_GLITTER];
    let more = p.b[R_GLITTER_MORE];
    let layer = u32(k.z);
    let c = vec2<i32>(k.xy);
    let lat = hg_glitter_lattice(layer);
    let salt = hg_salt(layer);
    let b = hr_rnd3(c, salt ^ HG_SALT_B);
    let e = hr_rnd3(c, salt ^ HG_SALT_E);
    let curve = (sn * more.x) * (sn * more.x);
    if (layer == 3u) {
        // A star: round grooves about its middle, half a flake's tilt, domed
        // by as much again so its glint fills it, and its grooves pressed
        // with a fan, so a light draws a bow tie of colour across it rather
        // than a thin line.
        let size = p.b[R_STARS].x * (0.6 + 0.8 * e.z);
        let d = p_mm - hg_place(lat, c, salt).xy;
        let r = length(d);
        let g = select(vec2<f32>(1.0, 0.0), d / r, r > 1e-6);
        let ap = 0.242 / (size * 1000.0);
        let dome = 0.5 * sn * gl.w;
        var px = hg_element(base, 0.5 * gl.w * hr_gauss2(b.xy), kt, g,
                            0.5 * (more.y + more.z), base.w, curve + ap * ap + dome * dome);
        px.sig_th = max(smm / max(r, 0.02), 0.12);
        px.lite = true;
        return px;
    }
    let th = HC_PI * b.z;
    var px = hg_element(base, gl.w * hr_gauss2(b.xy), kt, vec2<f32>(cos(th), sin(th)),
                        mix(more.y, more.z, e.x), base.w, curve + more.w);
    px.lite = true;
    return px;
}

// Glitter at p (mm), footprint smm (mm), on the pixel `base` describes.
fn holo_glitter(p_mm: vec2<f32>, smm: f32, base: Px, wear: bool,
                hair: ptr<function, HoloHairlines>) -> Acc {
    let gl = p.b[R_GLITTER];
    let more = p.b[R_GLITTER_MORE];
    let stars = p.b[R_STARS];
    let layers = u32(gl.z);
    let key = hg_key(base);
    let kt = key.x;
    let sn = key.y;

    // The mean field: untilted, grooves every way, the tilts and the flakes'
    // curve and aperture spread into the footprint. It also lights the coat,
    // so it is drawn even when no flake is left to it.
    let tq = sn * gl.w;
    let iso_f = (sn * more.x) * (sn * more.x) + more.w;
    var field = base;
    field.spx = base.spx + vec3<f32>(tq * tq + iso_f, 0.0, tq * tq + iso_f);
    field.ring = 1.0;
    field.sig_th = 1.0;
    field.inv_d = 0.5 * (more.y + more.z);
    field.sig_f = (more.z - more.y) / (3.4641 * field.inv_d);
    field.fam = 1.0;
    // The tilts blur the panel far more than its cells are wide.
    field.lite = true;

    // With many flakes to a pixel, fewer stay lit on their own. Those that
    // join the mean field still add their own flashes from the spots, less
    // its average, until there are too many to tell apart (spec 2.3's regime
    // B, by other means).
    let cell = 1.25 * gl.x;
    let nfp = f32(layers) * HC_TAU * smm * smm / (cell * cell);
    let shown = 1.0 - smoothstep(1.0, 3.0, nfp);
    let flick = FULL && nfp > 1.0;
    var spots: HgSpots;
    if (flick) {
        // Written by constant index: FXC cannot keep a loop that writes an
        // array at a counted one.
        for (var i = 0u; i < holo_spots(); i = i + 1u) {
            let o = hg_spot(base, i, field, kt, iso_f);
            if (i == 0u) {
                spots.s[0] = o;
            } else if (i == 1u) {
                spots.s[1] = o;
            } else {
                spots.s[2] = o;
            }
        }
    }
    // Flakes smaller than the pixel have their glints rolled off as a
    // display would show them.
    let dim = glint_dim(base, iso_f);
    field.dim = dim;
    var base_d = base;
    base_d.dim = dim;
    var run = hg_run();
    if (nfp < 128.0) {
        // The stars first when there are any, then each layer of flakes, in
        // one loop so FXC compiles the pass once.
        let first = select(0u, 1u, stars.y > 0.0);
        for (var k = 0u; k < layers + first; k = k + 1u) {
            let star = k < first;
            if (!star && run.seen < 0.02) {
                continue;
            }
            hg_lattice_pass(&run, p_mm, smm, select(k - first, 3u, star), shown, flick && !star,
                            spots, base_d, kt);
        }
        // They fade as they grow too many to tell apart.
        run.flick = run.flick * (1.0 - smoothstep(64.0, 128.0, nfp));
    } else {
        // All in the mean field: only the mean cover matters.
        let a = hg_area(u32(gl.y)) / 1.5625;
        let sc = 2.0 * stars.x;
        let a_star = stars.y * 0.7853982 * 1.0533 * stars.x * stars.x / (sc * sc);
        run.seen = (1.0 - a_star) * pow(1.0 - a, f32(layers));
        run.rest = 1.0 - run.seen;
    }
    // A thin second pick, at the edge of a heavier one, joins the field; so
    // does any second pick once flakes are smaller than the pixel, keeping
    // its own flash, or when Quality allows one.
    let picks = u32(p.b[R_QUALITY].z);
    if (run.n > 1u && (run.p1.w < HG_SECOND || flick || picks < 2u)) {
        run.rest = run.rest + run.p1.w;
        if (flick && u32(run.p1.z) < 3u) {
            run.flick = run.flick + run.p1.w * run.f1;
        }
        run.n = 1u;
    }
    if (run.rest < 0.02) {
        field.w = 0.0;
    }

    var acc: Acc;
    acc.lo = vec3<f32>(0.0);
    acc.lz = vec3<f32>(0.0);
    acc.lc = vec3<f32>(0.0);
    acc.ed = vec3<f32>(0.0);
    // The picks lit on their own, then the mean field, through one call of
    // the lighting: FXC inlines every call, and a copy for each took minutes
    // to compile.
    for (var k = 0u; k <= run.n; k = k + 1u) {
        let last = k == run.n;
        var el = field;
        var w = run.rest;
        if (!last) {
            let pick = select(run.p1, run.p0, k == 0u);
            el = hg_glitter_element(base, pick, p_mm, smm, kt, sn);
            w = pick.w;
            // A pick that covers only part of the pixel is rolled off for
            // the rest.
            el.dim = dim * (1.0 - min(w, 1.0));
        }
        let a = lit(el, wear && last, hair);
        acc.lo = acc.lo + w * a.lo;
        acc.lz = acc.lz + w * a.lz;
        if (last) {
            acc.lc = a.lc;
            acc.ed = a.ed;
        }
    }
    acc.lo = acc.lo + run.flick;
    // The base ink shows through the gaps, lit as a matte print.
    let tv = 1.0 - holo_fresnel(base.co, 1.5);
    let light = acc.ed + vec3<f32>(p.b[R_AMBIENT].x);
    acc.lz = acc.lz + (run.seen * tv * tv / HC_PI) * p.b[R_BASE_INK].xyz * light;
    return acc;
}
