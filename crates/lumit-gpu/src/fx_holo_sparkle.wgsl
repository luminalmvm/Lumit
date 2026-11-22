// Holographic foil's Sparkle material: embossed sparkle film. Irregular lobed
// dots on two jittered lattices all share one grating, so the field lights as
// a band, dot by dot; a share of them are dyed satin specks; between them is
// sand-matte aluminium, under the lacquer. Not a kernel on its own: it is
// compiled with fx_holo_core.wgsl, fx_holo_relief.wgsl, fx_holo_glitter.wgsl
// (whose lattice, cover and candidate helpers it uses) and
// fx_holographic_foil.wgsl.
//
// Dots that cover enough of the pixel are lit one by one; the rest join a mean
// field, the dots as one untilted grating with their strays as spread. The
// sand is one mirror element: its facets' mean slope at the pixel, and their
// spread as a widening.

// Dot size in mm, dot cover, irregularity, tangent of the dot tilt.
const R_SPARKLE: u32 = 284u;
// Dome in radians, speck share, speck size share, dot lattice pitch in mm.
const R_SPARKLE_MORE: u32 = 285u;
// Sand pitch in mm, tangent of the sand tilt, the sand's own roughness.
const R_SAND: u32 = 286u;
// The specks' dye as a colour ratio.
const R_SPECK_TINT: u32 = 287u;

const HP_SALT_N: u32 = 2798796415u;
const HP_SALT_M: u32 = 1234567891u;

fn hp_lattice(layer: u32) -> HgLattice {
    let pitch = p.b[R_SPARKLE_MORE].w;
    return hg_lattice(select(0.0, 37.0, layer == 1u), pitch, f32(layer) * vec2<f32>(0.371, 0.619));
}

fn hp_salt(layer: u32) -> u32 {
    return u32(p.b[R_RELIEF_MORE].z) ^ ((layer + 5u) << 8u);
}

// A dot's shape from its cell: middle in mm, mean radius in mm, and whether it
// is a speck (z of the second number).
struct HpDot {
    centre: vec2<f32>,
    r: f32,
    speck: bool,
    // Lobe amounts and phases for cos 2 and cos 3.
    a2: f32,
    a3: f32,
    p2: f32,
    p3: f32,
    keep: f32,
};

fn hp_dot(lat: HgLattice, c: vec2<i32>, salt: u32) -> HpDot {
    let sp = p.b[R_SPARKLE];
    let more = p.b[R_SPARKLE_MORE];
    let a = hr_rnd3(c, salt);
    let t = hr_rnd3(c, salt ^ HG_SALT_B);
    let o = hr_rnd3(c, salt ^ HG_SALT_E);
    var d: HpDot;
    d.centre = hg_mm(lat, vec2<f32>(c) + 0.5 + 0.8 * (a.xy - 0.5));
    d.speck = t.z < more.y;
    let irr = sp.z;
    if (d.speck) {
        d.r = 0.5 * sp.x * more.z * (0.9 + 0.2 * a.z);
        d.a2 = 0.0;
        d.a3 = 0.0;
    } else {
        d.r = 0.5 * sp.x * (1.0 + 0.4 * irr * (2.0 * a.z - 1.0));
        d.a2 = 0.15 * irr * t.x;
        d.a3 = 0.15 * irr * t.y;
    }
    d.p2 = HC_TAU * o.x;
    d.p3 = HC_TAU * o.y;
    d.keep = o.z;
    return d;
}

// The signed distance from the pixel to a dot's lobed outline, along the line
// from its middle (dp is the pixel less the middle).
fn hp_sd(d: HpDot, dp: vec2<f32>) -> f32 {
    let len = length(dp);
    if (len < 1e-9) {
        return -d.r;
    }
    let u = dp / len;
    let c2 = u.x * u.x - u.y * u.y;
    let s2 = 2.0 * u.x * u.y;
    let c3 = u.x * (4.0 * u.x * u.x - 3.0);
    let s3 = u.y * (3.0 - 4.0 * u.y * u.y);
    let lobes = d.a2 * (c2 * cos(d.p2) + s2 * sin(d.p2)) + d.a3 * (c3 * cos(d.p3) + s3 * sin(d.p3));
    return len - d.r * (1.0 + lobes);
}

// One lattice of dots seen through the pixel.
fn hp_pass(run: ptr<function, HgRun>, p_mm: vec2<f32>, smm: f32, layer: u32, shown: f32) {
    let sp = p.b[R_SPARKLE];
    let more = p.b[R_SPARKLE_MORE];
    let lat = hp_lattice(layer);
    let salt = hp_salt(layer);
    let u = hg_cells(lat, p_mm);
    let sc = smm / lat.cell;
    let win = hg_window(u, sc, true);
    let reach = 0.85 * sp.x + 3.5 * smm;
    var sum = 0.0;
    for (var j = 0; j < win.z; j = j + 1) {
        for (var i = 0; i < win.z; i = i + 1) {
            let c = win.xy + vec2<i32>(i, j);
            let d = hp_dot(lat, c, salt);
            let dp = p_mm - d.centre;
            let d2 = dot(dp, dp);
            if (d2 > reach * reach) {
                continue;
            }
            let area = HC_PI * d.r * d.r * (1.0 + 0.5 * (d.a2 * d.a2 + d.a3 * d.a3));
            let cover = hg_cover(hp_sd(d, dp), area, d2, 2.0 * d.r, smm);
            if (cover <= 1e-4) {
                continue;
            }
            sum = sum + cover;
            if (d.speck) {
                (*run).other = (*run).other + cover * (*run).seen;
            } else {
                hg_take(run, cover, hg_keep(d.keep, shown), vec3<f32>(vec2<f32>(c), f32(layer)),
                        vec3<f32>(0.0));
            }
        }
    }
    // What the window cannot reach joins the mean field at the mean cover,
    // dots and specks by their share of the area.
    let c1 = 1.0 - sqrt(max(1.0 - sp.y, 0.0));
    let miss = c1 * (1.0 - hg_reach(u, sc, win));
    let specks = more.y * more.z * more.z;
    (*run).rest = (*run).rest + (*run).seen * miss * (1.0 - specks);
    (*run).other = (*run).other + (*run).seen * miss * specks;
    (*run).seen = (*run).seen * max(1.0 - sum - miss, 0.0);
}

// The dot a pick names, as an element: the field's grating turned and spaced
// by its own strays, its own tilt, its dome and aperture as spread.
fn hp_element(base: Px, k: vec4<f32>, kt: f32, sn: f32) -> Px {
    let sp = p.b[R_SPARKLE];
    let more = p.b[R_SPARKLE_MORE];
    let spread = p.b[R_SPREAD];
    let layer = u32(k.z);
    let c = vec2<i32>(k.xy);
    let salt = hp_salt(layer);
    let d = hp_dot(hp_lattice(layer), c, salt);
    let n = hr_gauss2(hr_rnd3(c, salt ^ HP_SALT_N).xy);
    let tilt = sp.w * hr_gauss2(hr_rnd3(c, salt ^ HP_SALT_M).xy);
    let a = spread.z * n.x;
    let g = vec2<f32>(cos(a) * base.g.x - sin(a) * base.g.y, sin(a) * base.g.x + cos(a) * base.g.y);
    let inv_d = base.inv_d / max(1.0 + spread.w * n.y, 0.5);
    let ap = 0.242 / (2.0 * d.r * 1000.0);
    let iso = (sn * more.x) * (sn * more.x) + ap * ap;
    var px = hg_element(base, tilt, kt, g, inv_d, base.w, iso);
    px.lite = true;
    return px;
}

// The sand under the dots at p: Worley facets on a lattice at 71 degrees, the
// three nearest blended by how sure the pixel is of each, as one mirror with
// their mean slope and their spread. It also lights the coat.
fn hp_sand(base: Px, p_mm: vec2<f32>, smm: f32, kt: f32, sn: f32) -> Px {
    let sand = p.b[R_SAND];
    let lat = hg_lattice(71.0, sand.x, vec2<f32>(0.0));
    let u = hg_cells(lat, p_mm);
    let home = vec2<i32>(floor(u));
    let salt = u32(p.b[R_RELIEF_MORE].z) ^ 4093u;
    // The three nearest facet points: distance in cells and slope.
    var d0 = 1e9;
    var d1 = 1e9;
    var d2 = 1e9;
    var s0 = vec2<f32>(0.0);
    var s1 = vec2<f32>(0.0);
    var s2 = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j = j + 1) {
        for (var i = -1; i <= 1; i = i + 1) {
            let c = home + vec2<i32>(i, j);
            let a = hr_rnd3(c, salt);
            let d = length(vec2<f32>(c) + a.xy - u);
            if (d >= d2) {
                continue;
            }
            var g = hr_gauss2(hr_rnd3(c, salt ^ HG_SALT_B).xy);
            let gl = length(g);
            if (gl > 2.5) {
                g = g * (2.5 / gl);
            }
            let s = sand.y * 1.02272 * g;
            if (d < d0) {
                d2 = d1;
                s2 = s1;
                d1 = d0;
                s1 = s0;
                d0 = d;
                s0 = s;
            } else if (d < d1) {
                d2 = d1;
                s2 = s1;
                d1 = d;
                s1 = s;
            } else {
                d2 = d;
                s2 = s;
            }
        }
    }
    let sig = sqrt(smm * smm + (0.22 * sand.x) * (0.22 * sand.x));
    let w1 = exp(-1.702 * (d1 - d0) * sand.x / sig);
    let w2 = exp(-1.702 * (d2 - d0) * sand.x / sig);
    let ws = 1.0 + w1 + w2;
    var m = (s0 + w1 * s1 + w2 * s2) / ws;
    var cv = (hr_outer(s0 - m) + w1 * hr_outer(s1 - m) + w2 * hr_outer(s2 - m)) / ws;
    // Grains far smaller than the pixel average out: three are no longer a
    // fair sample, so hand over to their whole spread.
    let fine = smoothstep(0.3, 1.0, smm / sand.x);
    let all = sand.y * 1.02272;
    m = m * (1.0 - fine);
    cv = cv * (1.0 - fine) + vec3<f32>(fine * all * all, 0.0, fine * all * all);
    let own = sn * sand.z;
    var px = hg_element(base, m, kt, base.g, base.inv_d, 0.0, own * own);
    px.spx = px.spx + sn * sn * cv;
    px.gloss = base.gloss;
    // A plain mirror: the panel's cells would only add to its diffuse light.
    px.lite = true;
    return px;
}

// Sparkle at p (mm), footprint smm (mm), on the pixel `base` describes.
fn holo_sparkle(p_mm: vec2<f32>, smm: f32, base: Px, wear: bool,
                hair: ptr<function, HoloHairlines>) -> Acc {
    let sp = p.b[R_SPARKLE];
    let more = p.b[R_SPARKLE_MORE];
    let key = hg_key(base);
    let kt = key.x;
    let sn = key.y;
    let cell = more.w;
    let nfp = 2.0 * HC_TAU * smm * smm / (cell * cell);
    let shown = 1.0 - max(smoothstep(1.5, 6.0, nfp), smoothstep(0.35, 0.5, smm / cell));
    var run = hg_run();
    if (shown > 0.0) {
        hp_pass(&run, p_mm, smm, 0u, shown);
        hp_pass(&run, p_mm, smm, 1u, shown);
    } else {
        let specks = more.y * more.z * more.z;
        run.seen = 1.0 - sp.y;
        run.rest = sp.y * (1.0 - specks);
        run.other = sp.y * specks;
    }

    // The dots' mean field: the one grating, untilted, the dots' tilt, dome
    // and strays as spread.
    var field = base;
    let spread = sn * sqrt(more.x * more.x + sp.w * sp.w);
    let ap = 0.242 / (sp.x * 1000.0);
    let iso = spread * spread + ap * ap;
    field.spx = base.spx + vec3<f32>(iso, 0.0, iso);
    field.gloss = 0.0;
    field.lite = true;
    // Dots smaller than the pixel have their glints rolled off as a display
    // would show them.
    let dim = glint_dim(base, iso);
    field.dim = dim;
    if (run.n > 1u && (run.p1.w < HG_SECOND || u32(p.b[R_QUALITY].z) < 2u)) {
        run.rest = run.rest + run.p1.w;
        run.n = 1u;
    }
    let sand = hp_sand(base, p_mm, smm, kt, sn);

    var acc: Acc;
    acc.lo = vec3<f32>(0.0);
    acc.lz = vec3<f32>(0.0);
    acc.lc = vec3<f32>(0.0);
    acc.ed = vec3<f32>(0.0);
    // The picks, the dots' mean field when it holds enough, then the sand,
    // through one call of the lighting, as glitter does.
    let with_field = run.rest >= 0.02;
    let n = run.n + select(1u, 2u, with_field);
    for (var k = 0u; k < n; k = k + 1u) {
        let last = k + 1u == n;
        var el = sand;
        var w = vec3<f32>(run.rest);
        if (k < run.n) {
            let pick = select(run.p1, run.p0, k == 0u);
            el = hp_element(base, pick, kt, sn);
            w = vec3<f32>(pick.w);
            el.dim = dim * (1.0 - min(pick.w, 1.0));
        } else if (!last) {
            el = field;
        }
        let a = lit(el, wear && last, hair);
        if (last) {
            // The sand shows between the dots, and through the specks' dye.
            let wt = vec3<f32>(run.seen) + run.other * p.b[R_SPECK_TINT].xyz;
            acc.lz = acc.lz + wt * a.lz;
            acc.lc = a.lc;
            acc.ed = a.ed;
        } else {
            acc.lo = acc.lo + w * a.lo;
            acc.lz = acc.lz + w * a.lz;
        }
    }
    return acc;
}
