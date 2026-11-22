// Holographic foil: the layer as a sheet of embossed rainbow foil under a
// rig of lights fixed to the viewer. Compiled after fx_holo_core.wgsl,
// fx_holo_relief.wgsl, fx_holo_shatter.wgsl, fx_holo_glitter.wgsl and
// fx_holo_sparkle.wgsl.
//
// Per pixel: where the point sits on the tipped sheet, which way the eye and
// each light lie from it, how the relief bends the foil and the varnish there,
// then for each light the grating's orders (up to three, the rainbow), the
// flat metal's mirror and the clear coat's gloss, and the same three for the
// soft studio around. The lights are the three spots (each seen twice where
// thin marks cross the pixel) and up to twelve cells of the panel: at most 18
// lamps. The layer's alpha is the sheet, its colour the print. Seen from
// behind, the sheet is plain satin metal. Output is scene-linear,
// premultiplied by the layer alpha.
//
// The uniform rows are laid out as the tables in
// lumit_core::fx::effects::holographic_foil (pack.rs, surface.rs) say.

struct Params {
    // x: which optional pictures are bound (the F_ bits).
    flags: vec4<u32>,
    b: array<vec4<f32>, 2210>,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> p: Params;
@group(0) @binding(2) var dst: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var matte: texture_2d<f32>;
@group(0) @binding(4) var height_map: texture_2d<f32>;
@group(0) @binding(5) var foil_mask: texture_2d<f32>;
@group(0) @binding(6) var direction_map: texture_2d<f32>;
@group(0) @binding(7) var pitch_map: texture_2d<f32>;
@group(0) @binding(8) var environment: texture_2d<f32>;
// Halved copies of the layer, each texel the mean of four below it, whose
// alpha a bevel reads blurred. Bound only when the Relief is a bevel.
@group(0) @binding(13) var bevel_mips: texture_2d<f32>;

// The Material this pipeline draws, fixed when it is built: one pipeline per
// material keeps each small enough for FXC to compile in reasonable time.
override MATERIAL: u32 = 0u;

// The whole kernel, or a lean one without wear, the Height, Direction,
// Pitch and Environment maps and the lone flashes of glitter flakes smaller
// than a pixel. Most frames need none of them, and the lean kernel compiles
// in about half the time.
override FULL: bool = true;

const F_MATTE: u32 = 1u;
const F_HEIGHT: u32 = 2u;
const F_MASK: u32 = 4u;
const F_DIRECTION: u32 = 8u;
const F_PITCH: u32 = 16u;
const F_ENVIRONMENT: u32 = 32u;
// The maps only the full kernel reads.
const F_FULL: u32 = 58u;

const R_SIZE: u32 = 0u;
const R_T: u32 = 1u;
const R_B: u32 = 2u;
const R_N: u32 = 3u;
const R_GRATING: u32 = 4u;
const R_CENTRE: u32 = 5u;
const R_SPREAD: u32 = 6u;
const R_TINT: u32 = 7u;
const R_OUTPUT: u32 = 8u;
const R_STUDIO_LEVELS: u32 = 9u;
const R_AMBIENT: u32 = 10u;
const R_LIGHTS: u32 = 11u;
const R_SPECTRA: u32 = 27u;
const R_BINS: u32 = 47u;
const R_STUDIO: u32 = 53u;
const R_TERMS: u32 = 65u;
const R_RELIEF: u32 = 192u;
const R_RELIEF_MORE: u32 = 193u;
const R_WAVE_HEAD: u32 = 194u;
const R_BOW: u32 = 195u;
const R_MAPS: u32 = 196u;
const R_SMUDGES: u32 = 197u;
const R_WAVES: u32 = 206u;
const R_PREV_T: u32 = 240u;
const R_PREV_B: u32 = 241u;
const R_PREV_KEY: u32 = 242u;
const R_VORTEX: u32 = 244u;
const R_ETA: u32 = 246u;
const R_ETA_MIRROR: u32 = 258u;
const R_ETA_ENV: u32 = 270u;
const R_FAMILIES: u32 = 273u;
const R_QUALITY: u32 = 275u;
const R_COUNTS: u32 = 278u;
const R_BEVEL: u32 = 279u;

// The studio panels' own edge softness, radians.
const EDGE: f32 = 0.026;
// About the brightest a glint shows after the highlights curve.
const GLINT_WHITE: f32 = 24.0;
// How much of the light on the back of the sheet (the rig's and the
// studio's) its plain satin metal sends back.
const BACK_SATIN: f32 = 0.35;
// Half the fan of a hairline scratch's walls, as a tangent (30 degrees).
const HAIR_FAN: f32 = 0.57735;

fn holo_row(i: u32) -> vec4<f32> {
    return p.b[R_TERMS + i];
}

// The three spots, as a count read from the uniform, so FXC keeps a loop over
// them rather than a copy of its body for each.
fn holo_spots() -> u32 {
    return u32(p.b[R_QUALITY + 1u].y);
}

// Loop counts read from the uniform (4, 9, 2 and 3): FXC copies the body of a
// loop with a constant count once per turn, which made the kernels slow to
// compile.
fn holo_four() -> u32 {
    return u32(p.b[R_COUNTS].x);
}

fn holo_nine() -> u32 {
    return u32(p.b[R_COUNTS].y);
}

fn holo_two() -> u32 {
    return u32(p.b[R_COUNTS].z);
}

fn holo_three() -> u32 {
    return u32(p.b[R_COUNTS].w);
}

fn has(bit: u32) -> bool {
    if (!FULL && (bit & F_FULL) != 0u) {
        return false;
    }
    return (p.flags.x & bit) != 0u;
}

// What every light needs of this pixel.
struct Px {
    x: vec3<f32>,
    v: vec3<f32>,
    // The eye's direction in the foil's own x and y.
    vt: vec2<f32>,
    co: f32,
    // The eye's cosine inside the lacquer.
    co_in: f32,
    // The foil's frame, bent by the relief.
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
    // The varnish's normal.
    nc: vec3<f32>,
    // The groove normal on the sheet, and how far it strays within the pixel.
    g: vec2<f32>,
    sig_th: f32,
    // 1 / pitch here, 1/nm, how far the pitch strays as a share, and where the
    // pitch sits among the three families the renormalisation knows (0 to 2).
    inv_d: f32,
    sig_f: f32,
    fam: f32,
    // Grating coverage.
    w: f32,
    // The pixel's footprint in q with the relief it cannot resolve: xx, xy, yy.
    spx: vec3<f32>,
    // The same for the varnish, which an element's own spread leaves alone.
    cspx: vec3<f32>,
    // Above 0, grooves turned every way (a mean field of flakes).
    ring: f32,
    // A small element among many (a flake): the panel's rainbow as one cell,
    // the studio's in three bins, band colours from the table.
    lite: bool,
    // The varnish's roughness squared, with what the relief adds.
    a2: f32,
    // Gloss left by smudges.
    gloss: f32,
    // Thin marks: their share of the pixel, their spread in q, and what they
    // add to the varnish's roughness squared.
    sc: f32,
    sq: vec3<f32>,
    sa2: f32,
    // How much of each spot's light a display would lose in glints too small
    // to see on their own: 0 keeps it all.
    dim: vec3<f32>,
};

struct Acc {
    // Diffracted orders, the mirror, the coat, light on a printed surface.
    lo: vec3<f32>,
    lz: vec3<f32>,
    lc: vec3<f32>,
    ed: vec3<f32>,
};

fn lacquer() -> f32 {
    return p.b[R_OUTPUT].w;
}

// The renormalisation at the four nodes (430, 510, 590, 670 nm) for this
// pixel's pitch family, from a block of rows laid out family by family with
// `per` rows each, of which `slot` is read.
fn eta_nodes(base: u32, per: u32, slot: u32, fam: f32) -> vec4<f32> {
    let f = clamp(fam, 0.0, 2.0);
    let j = min(u32(f), 1u);
    return mix(p.b[base + per * j + slot], p.b[base + per * (j + 1u) + slot], f - f32(j));
}

// The renormalisation at lam nm, read off the nodes, and its slope as a share
// per nm.
fn eta_at(e: vec4<f32>, lam: f32) -> vec2<f32> {
    let x = clamp((lam - 430.0) / 80.0, 0.0, 3.0);
    let k = min(u32(x), 2u);
    let v0 = e[k];
    let v1 = e[k + 1u];
    let v = mix(v0, v1, x - f32(k));
    let inside = lam > 430.0 && lam < 670.0;
    return vec2<f32>(v, select(0.0, (v1 - v0) / (80.0 * max(v, 1e-3)), inside));
}

// The colour the flat metal mirrors under spectrum `spec`, when the grating
// covers `w` of it and sends J0 squared of its light, renormalised by `eta`,
// to the mirror.
fn mirror_rgb(spec: u32, sn: f32, w: f32, eta: vec4<f32>) -> vec3<f32> {
    let depth = p.b[R_CENTRE].w;
    let ph = min(vec4<f32>(HC_PI * depth * sn) / vec4<f32>(430.0, 510.0, 590.0, 670.0),
                 vec4<f32>(5.5));
    let j0 = holo_bessel0(ph);
    let mw = vec4<f32>(1.0 - w) + w * j0 * j0 * eta;
    let at = R_SPECTRA + 4u * spec;
    return vec3<f32>(dot(p.b[at + 1u], mw), dot(p.b[at + 2u], mw), dot(p.b[at + 3u], mw));
}

// The band colour of spectrum `spec`.
fn band(lam: f32, sig: f32, spec: u32) -> vec3<f32> {
    let h = p.b[R_SPECTRA + 4u * spec];
    return holo_band(lam, sig, u32(h.x), u32(h.y), h.zw, p.b[R_TINT]);
}

// The grating's orders 1 to 3 for light `slot`, whose half-vector sum is q on
// the foil, spread by the covariance s (xx, xy, yy). sn is n (c_in + c_out)
// inside the lacquer. Each band is weighed by its efficiency and the
// renormalisation, and shifted toward where they grow across it.
fn orders(px: Px, q: vec2<f32>, s: vec3<f32>, sn: f32, spec: u32, slot: u32) -> vec3<f32> {
    let gx = px.g.x;
    let gy = px.g.y;
    let s00 = gx * gx * s.x + 2.0 * gx * gy * s.y + gy * gy * s.z;
    let s11 = gy * gy * s.x - 2.0 * gx * gy * s.y + gx * gx * s.z;
    let s01 = -gx * gy * s.x + (gx * gx - gy * gy) * s.y + gx * gy * s.z;
    let qg = dot(q, px.g);
    let qt = -gy * q.x + gx * q.y;
    let rho = length(q);
    let spread = p.b[R_SPREAD];
    let iso2 = (sn * spread.y) * (sn * spread.y);
    let sgg = max(s00 + iso2 + (rho * px.sig_f) * (rho * px.sig_f), 1e-10);
    let stt0 = s11 + iso2 + (rho * px.sig_th) * (rho * px.sig_th);
    let d = 1.0 / px.inv_d;
    let depth = p.b[R_CENTRE].w;
    let eta = eta_nodes(R_ETA, 4u, slot, px.fam);
    if (px.ring > 0.0) {
        return hg_rings(px, q, s, sn, spec, eta);
    }
    var acc = vec3<f32>(0.0);
    let top = u32(p.b[R_QUALITY].x);
    for (var m = 1u; m <= top; m = m + 1u) {
        let mf = f32(m);
        let stt = max(stt0 + mf * spread.x * spread.x, 1e-10);
        let qgc = qg - (s01 / stt) * qt;
        let sgc = sqrt(max(sgg - s01 * s01 / stt, 1e-10));
        let lam = d * abs(qgc) / mf;
        let sig_l = clamp(d * sgc / mf, 0.5, 512.0);
        if (lam + 3.0 * sig_l < 360.0 || lam - 3.0 * sig_l > 830.0) {
            continue;
        }
        let n1 = exp(-0.5 * qt * qt / stt) * inverseSqrt(HC_TAU * stt);
        if (n1 < 1e-5) {
            continue;
        }
        let raw_ph = HC_PI * depth * sn / max(lam, 1.0);
        let ph = min(raw_ph, 5.5);
        let jm = holo_bessel(m, ph);
        let e = eta_at(eta, lam);
        // How fast ln(efficiency) changes with wavelength, which moves the
        // band's centre by that slope times its variance.
        var slope = e.y;
        if (raw_ph < 5.5) {
            let jn = holo_bessel(m + 1u, ph);
            let ratio = jn / select(jm, 1e-6, abs(jm) < 1e-6);
            slope = slope - 2.0 * (mf - ph * ratio) / max(lam, 1.0);
        }
        let sh = clamp(slope * sig_l * sig_l, -0.5 * sig_l, 0.5 * sig_l);
        let k = exp(0.5 * (sh / sig_l) * (sh / sig_l));
        var c: vec3<f32>;
        if (px.lite) {
            c = holo_band_table(lam + sh, sig_l, spec);
        } else {
            c = band(lam + sh, sig_l, spec);
        }
        acc = acc + (n1 * (d / mf) * jm * jm * e.x * k) * c;
    }
    return acc;
}

// One light as the pixel sees it: its direction l, projected solid angle a,
// footprint s in q, radiance rad (0 draws nothing), colour, spectrum, slot and
// angular radius. A sphere's mirror is its own disc (`disc`: the radial axis
// in q and the disc's two radii); the panel's mirror is worked out over its
// whole outline instead. xs and xa widen the lobes in q and the varnish's
// roughness squared; wt weighs the whole result.
struct Lamp {
    l: vec3<f32>,
    a: f32,
    s: vec3<f32>,
    rad: f32,
    rgb: vec3<f32>,
    spec: u32,
    slot: u32,
    r_ang: f32,
    sphere: bool,
    disc: vec4<f32>,
    xs: vec3<f32>,
    xa: f32,
    wt: f32,
};

// One lamp on this pixel: the orders, the mirror of a sphere blurred by the
// metal and the pixel, and the gloss as a lobe.
fn shade(px: Px, lm: Lamp, acc: ptr<function, Acc>) {
    let l = lm.l;
    let a = lm.a;
    let rad = lm.rad;
    let wt = lm.wt;
    let disc = lm.disc;
    let xs = lm.xs;
    let nl = dot(px.n, l);
    if (nl <= 0.0 || a <= 0.0 || wt <= 0.0 || rad <= 0.0) {
        return;
    }
    let lt = vec2<f32>(dot(l, px.t), dot(l, px.b));
    let q = lt + px.vt;
    let nc = lacquer();
    let ci = sqrt(max(1.0 - dot(lt, lt) / (nc * nc), 0.0));
    let sn = nc * (ci + px.co_in);
    let tc = (1.0 - holo_fresnel(nl, 1.5)) * (1.0 - holo_fresnel(px.co, 1.5));
    let st = px.spx + lm.s + xs;
    let e = rad * a * wt;
    if (px.w > 0.0) {
        (*acc).lo = (*acc).lo + (e * tc * px.w) * orders(px, q, st, sn, lm.spec, lm.slot);
    }
    (*acc).ed = (*acc).ed + e * lm.rgb;
    if (!lm.sphere) {
        return;
    }
    {
        // In the disc's own units the lamp is the unit disc. A sharp blur
        // leaves its edge; a wide one leaves a Gaussian of the same energy.
        let soft = p.b[R_SPREAD].y * sn;
        let iso = soft * soft + 1e-10;
        let bl = px.spx + xs + vec3<f32>(iso, 0.0, iso);
        let e1 = disc.xy / disc.z;
        let e2 = vec2<f32>(-disc.y, disc.x) / disc.w;
        let su = e1.x * e1.x * bl.x + 2.0 * e1.x * e1.y * bl.y + e1.y * e1.y * bl.z;
        let sv = e2.x * e2.x * bl.x + 2.0 * e2.x * e2.y * bl.y + e2.y * e2.y * bl.z;
        let s2 = 0.5 * (su + sv);
        let r = length(vec2<f32>(dot(q, e1), dot(q, e2)));
        let sb = sqrt(s2);
        let vv = 0.25 + s2;
        let lobe = mix(holo_cdf((1.0 - r) / max(sb, 1e-3)), exp(-0.5 * r * r / vv) / (2.0 * vv),
                       smoothstep(0.15, 0.6, sb));
        if (lobe > 1e-7) {
            let eta = eta_nodes(R_ETA_MIRROR, 4u, lm.slot, px.fam);
            (*acc).lz = (*acc).lz + (rad * wt * tc * lobe) * mirror_rgb(lm.spec, sn, px.w, eta);
        }
    }
    let ncl = dot(px.nc, l);
    let ncv = max(dot(px.nc, px.v), 1e-4);
    if (ncl <= 0.0 || px.gloss <= 0.0) {
        return;
    }
    let a2 = px.a2 + lm.xa + 0.25 * lm.r_ang * lm.r_ang;
    let hv = normalize(l + px.v);
    let gloss = holo_ggx(max(dot(px.nc, hv), 0.0), a2) * holo_smith(ncl, ncv, a2)
        * holo_fresnel(dot(hv, l), 1.5);
    (*acc).lc = (*acc).lc + (e * gloss * px.gloss) * lm.rgb;
}

// How much of each spot's light a small element's glint, spread by `iso` in
// q, loses to the highlights curve when the element is smaller than a pixel:
// its peak is the spot's radiance, or less where the spread is wider than the
// spot's disc. The curve sees such glints only averaged, so they are rolled
// off here as a display would show them, and the look holds at any preview
// size. Highlights Off keeps all of the light, for Glow.
fn glint_dim(base: Px, iso: f32) -> vec3<f32> {
    var dim = vec3<f32>(0.0);
    if (u32(p.b[R_OUTPUT].y) == 0u) {
        return dim;
    }
    for (var i = 0u; i < holo_spots(); i = i + 1u) {
        let lp = p.b[R_LIGHTS + 4u * i];
        let dv = lp.xyz - base.x;
        let dist = length(dv);
        let sa = clamp(p.b[R_LIGHTS + 4u * i + 2u].x / dist, 1e-5, 0.999);
        let ar = sa * max(dot(dv / dist, base.n), 0.0);
        let peak = lp.w * min(1.0, 1.0 / (0.5 + 2.0 * iso / max(ar * sa, 1e-10)));
        let d = select(0.0, 1.0 - min(1.0, GLINT_WHITE / max(peak, 1e-6)), lp.w > 0.0);
        if (i == 0u) {
            dim.x = d;
        } else if (i == 1u) {
            dim.y = d;
        } else {
            dim.z = d;
        }
    }
    return dim;
}

// The arcs hairline scratches in the varnish draw round a sphere's
// highlight: their walls fan across the scratch, so one lights wherever it
// runs square to the way from the pixel's mirror point to the light.
fn hairline(h: vec4<f32>, q: vec2<f32>, sn: f32, st: vec3<f32>) -> f32 {
    if (h.z <= 0.0) {
        return 0.0;
    }
    let dir = h.xy;
    let nrm = vec2<f32>(-dir.y, dir.x);
    let sa = 0.02 * sn;
    let vt = sa * sa + dir.x * dir.x * st.x + 2.0 * dir.x * dir.y * st.y + dir.y * dir.y * st.z;
    let vn = nrm.x * nrm.x * st.x + 2.0 * nrm.x * nrm.y * st.y + nrm.y * nrm.y * st.z;
    let qa = dot(q, dir);
    let along = exp(-0.5 * qa * qa / vt) * inverseSqrt(HC_TAU * vt);
    let across = holo_box(dot(q, nrm), sn * HAIR_FAN, sqrt(vn)) / (2.0 * sn * HAIR_FAN);
    return h.z * along * across;
}

fn hairlines(px: Px, hair: ptr<function, HoloHairlines>, l: vec3<f32>, e: f32, s: vec3<f32>,
             rgb: vec3<f32>, acc: ptr<function, Acc>) {
    let hsum = l + px.v;
    let sn = max(dot(px.nc, hsum), 0.05);
    let tc = normalize(px.t - px.nc * dot(px.t, px.nc));
    let bc = cross(px.nc, tc);
    let q = vec2<f32>(dot(hsum, tc), dot(hsum, bc));
    // The walls cut through to the metal, which sends back most of the light.
    let f = 0.85 * (1.0 - holo_fresnel(dot(normalize(hsum), l), 1.5));
    let st = px.spx + s;
    // The four through one call.
    var sum = 0.0;
    for (var k = 0u; k < holo_four(); k = k + 1u) {
        let late = select((*hair).h3, (*hair).h2, k == 2u);
        let h = select(late, select((*hair).h1, (*hair).h0, k == 0u), k < 2u);
        sum = sum + hairline(h, q, sn, st);
    }
    (*acc).lc = (*acc).lc + (e * f * sum) * rgb;
}

// How much of a Gaussian lobe in q, centred on -q0 with covariance s (xx, xy,
// yy) plus iso, falls on the parallelogram whose inverse has columns i0, i1.
fn rect_cover(q0: vec2<f32>, i0: vec2<f32>, i1: vec2<f32>, s: vec3<f32>, iso: f32) -> f32 {
    let ab = -(i0 * q0.x + i1 * q0.y);
    let sxx = s.x + iso;
    let syy = s.z + iso;
    // Rows of the inverse are (i0.x, i1.x) and (i0.y, i1.y).
    let va = i0.x * i0.x * sxx + 2.0 * i0.x * i1.x * s.y + i1.x * i1.x * syy;
    let vb = i0.y * i0.y * sxx + 2.0 * i0.y * i1.y * s.y + i1.y * i1.y * syy;
    return holo_box(ab.x, 1.0, sqrt(va)) * holo_box(ab.y, 1.0, sqrt(vb));
}

// A direction's offset from the light at l, in q per metre of the light's
// own size, `dist` metres away.
fn across(v: vec3<f32>, l: vec3<f32>, dist: f32, px: Px) -> vec2<f32> {
    let o = (v - l * dot(l, v)) / dist;
    return vec2<f32>(dot(o, px.t), dot(o, px.b));
}

// Spot i as seen from px; rad 0 when it is off or behind the sheet.
fn spot_lamp(px: Px, i: u32) -> Lamp {
    var lm: Lamp;
    lm.slot = i;
    lm.sphere = true;
    lm.wt = 1.0;
    let at = R_LIGHTS + 4u * i;
    let lp = p.b[at];
    if (lp.w <= 0.0) {
        return lm;
    }
    let dv = lp.xyz - px.x;
    let dist = length(dv);
    let l = dv / dist;
    let sa = clamp(p.b[at + 2u].x / dist, 1e-5, 0.999);
    let cc = max(dot(l, px.n), 0.0);
    if (cc <= 0.0) {
        return lm;
    }
    let lc = vec2<f32>(dot(l, px.t), dot(l, px.b));
    let lcl = length(lc);
    let er = select(vec2<f32>(1.0, 0.0), lc / lcl, lcl > 1e-6);
    let ep = vec2<f32>(-er.y, er.x);
    let ar = max(sa * cc, 1e-5);
    let ap = sa;
    let col = p.b[at + 1u];
    lm.l = l;
    lm.a = HC_PI * ar * ap;
    lm.s = 0.5 * vec3<f32>(
        ar * ar * er.x * er.x + ap * ap * ep.x * ep.x,
        ar * ar * er.x * er.y + ap * ap * ep.x * ep.y,
        ar * ar * er.y * er.y + ap * ap * ep.y * ep.y);
    lm.rad = lp.w;
    lm.rgb = col.xyz;
    lm.spec = u32(col.w);
    lm.r_ang = asin(sa);
    lm.disc = vec4<f32>(er, ar, ap);
    return lm;
}

// Cell (i, j) of the panel's nu by nw cells, a soft source for its rainbow.
fn cell_lamp(px: Px, i: u32, j: u32, nu: u32, nw: u32) -> Lamp {
    var lm: Lamp;
    let at = R_LIGHTS + 12u;
    let lp = p.b[at];
    let col = p.b[at + 1u];
    let uh = p.b[at + 2u].xyz;
    let wh = p.b[at + 3u].xyz;
    let c = lp.xyz + uh * ((2.0 * f32(i) + 1.0) / f32(nu) - 1.0)
        + wh * ((2.0 * f32(j) + 1.0) / f32(nw) - 1.0);
    let cv = c - px.x;
    let cd = length(cv);
    let cl = cv / cd;
    let du = across(uh / f32(nu), cl, cd, px);
    let dw = across(wh / f32(nw), cl, cd, px);
    lm.l = cl;
    lm.a = 4.0 * abs(du.x * dw.y - du.y * dw.x);
    lm.s = (2.0 / 3.0) * vec3<f32>(
        du.x * du.x + dw.x * dw.x,
        du.x * du.y + dw.x * dw.y,
        du.y * du.y + dw.y * dw.y);
    lm.rad = lp.w;
    lm.rgb = col.xyz;
    lm.spec = u32(col.w);
    lm.slot = 3u;
    lm.disc = vec4<f32>(1.0);
    lm.xs = px.sc * px.sq;
    lm.wt = 1.0;
    return lm;
}

// The three spots and the panel's cells, in one loop, so the lighting is
// compiled once: FXC inlines every call, and a copy per light made the kernel
// take minutes to compile. Thin marks see each spot a second time, on their
// own, away from the rest of the pixel.
fn lamps(px: Px, wear: bool, hair: ptr<function, HoloHairlines>, acc: ptr<function, Acc>) {
    let parts = select(1u, 2u, px.sc > 0.002);
    let ns = holo_spots() * parts;
    var nu = 0u;
    var nw = 0u;
    if (p.b[R_LIGHTS + 12u].w > 0.0) {
        nu = select(max(u32(p.b[R_LIGHTS + 14u].w), 1u), 1u, px.lite);
        nw = select(max(u32(p.b[R_LIGHTS + 15u].w), 1u), 1u, px.lite);
    }
    for (var k = 0u; k < ns + nu * nw; k = k + 1u) {
        var lm: Lamp;
        if (k < ns) {
            let part = k % parts;
            lm = spot_lamp(px, k / parts);
            lm.wt = 1.0 - px.dim[min(k / parts, 2u)];
            if (parts == 2u) {
                lm.wt = lm.wt * select(1.0 - px.sc, px.sc, part == 1u);
                if (part == 1u) {
                    lm.xs = px.sq;
                    lm.xa = px.sa2;
                }
            }
            if (wear && part == 0u && lm.rad > 0.0) {
                hairlines(px, hair, lm.l, lm.rad * lm.a, lm.s, lm.rgb, acc);
            }
        } else {
            let c = k - ns;
            lm = cell_lamp(px, c / nw, c % nw, nu, nw);
        }
        shade(px, lm, acc);
    }
}

// The panel's mirror and gloss, each as one crisp rectangle. Its rainbow comes
// from the cells in lamps().
fn panel(px: Px, acc: ptr<function, Acc>) {
    let at = R_LIGHTS + 12u;
    let lp = p.b[at];
    if (lp.w <= 0.0) {
        return;
    }
    let col = p.b[at + 1u];
    let spec = u32(col.w);
    let uh = p.b[at + 2u];
    let wh = p.b[at + 3u];

    // The mirror and the gloss: each reflection lobe over the panel's outline.
    let dv = lp.xyz - px.x;
    let dist = length(dv);
    let l = dv / dist;
    let nl = dot(px.n, l);
    if (nl > 0.0) {
        let du = across(uh.xyz, l, dist, px);
        let dw = across(wh.xyz, l, dist, px);
        let det = du.x * dw.y - du.y * dw.x;
        if (abs(det) > 1e-9) {
            let lt = vec2<f32>(dot(l, px.t), dot(l, px.b));
            let q0 = lt + px.vt;
            let nc = lacquer();
            let ci = sqrt(max(1.0 - dot(lt, lt) / (nc * nc), 0.0));
            let sn = nc * (ci + px.co_in);
            let soft = p.b[R_SPREAD].y * sn;
            let iso = soft * soft + 1e-10;
            // The inverse of the matrix whose columns are du and dw.
            let i0 = vec2<f32>(dw.y, -du.y) / det;
            let i1 = vec2<f32>(-dw.x, du.x) / det;
            // The mirror, then the coat's lobe (about as wide in q as its
            // roughness), each also as thin marks see it on their own, as
            // they see a spot: four covers through one call.
            let thin = px.sc > 0.002;
            let snc = dot(px.nc, l + px.v);
            var covers = vec4<f32>(0.0);
            for (var k = 0u; k < holo_four(); k = k + 1u) {
                let marks = (k & 1u) == 1u;
                let coat = k >= 2u;
                if ((marks && !thin) || (coat && px.gloss <= 0.0)) {
                    continue;
                }
                var s = px.spx + select(vec3<f32>(0.0), px.sq, marks);
                var v = iso;
                if (coat) {
                    s = px.cspx;
                    v = 0.5 * select(px.a2, px.a2 + px.sa2, marks) * snc * snc;
                }
                let c = rect_cover(q0, i0, i1, s, v);
                if (k == 0u) {
                    covers.x = c;
                } else if (k == 1u) {
                    covers.y = c;
                } else if (k == 2u) {
                    covers.z = c;
                } else {
                    covers.w = c;
                }
            }
            var cover = covers.x;
            var gloss = covers.z;
            if (thin) {
                cover = mix(cover, covers.y, px.sc);
                gloss = mix(gloss, covers.w, px.sc);
            }
            if (cover > 1e-6) {
                let tc = (1.0 - holo_fresnel(nl, 1.5)) * (1.0 - holo_fresnel(px.co, 1.5));
                let eta = eta_nodes(R_ETA_MIRROR, 4u, 3u, px.fam);
                (*acc).lz = (*acc).lz + (lp.w * tc * cover) * mirror_rgb(spec, sn, px.w, eta);
            }
            if (px.gloss > 0.0) {
                (*acc).lc = (*acc).lc
                    + (lp.w * gloss * px.gloss * holo_fresnel(nl, 1.5)) * col.xyz;
            }
        }
    }
}

// The studio's grey radiance toward d, blurred by `blur` radians.
fn studio(d: vec3<f32>, blur: f32) -> f32 {
    let levels = p.b[R_STUDIO_LEVELS];
    var e = levels.w * smoothstep(-0.15, 0.15, d.z);
    let sg = sqrt(EDGE * EDGE + blur * blur);
    for (var i = 0u; i < holo_four(); i = i + 1u) {
        let c = p.b[R_STUDIO + 3u * i];
        if (c.w <= 0.0) {
            continue;
        }
        let dc = dot(d, c.xyz);
        if (dc <= 0.05) {
            continue;
        }
        let er = p.b[R_STUDIO + 3u * i + 1u];
        let et = p.b[R_STUDIO + 3u * i + 2u];
        let ax = atan(dot(d, er.xyz) / dc);
        let ay = atan(dot(d, et.xyz) / dc);
        e = e + c.w * holo_box(ax, er.w, sg) * holo_box(ay, et.w, sg);
    }
    return e;
}

// The Environment layer at texel position c, bilinear, held at its edges.
// Its size comes from the caller, as map_luma's does.
fn env_tap(c: vec2<f32>, dims: vec2<i32>) -> vec3<f32> {
    let q = c - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    let hi = dims - vec2<i32>(1);
    let a = textureLoad(environment, clamp(i, vec2<i32>(0), hi), 0).rgb;
    let b = textureLoad(environment, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), hi), 0).rgb;
    let cc = textureLoad(environment, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), hi), 0).rgb;
    let d = textureLoad(environment, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), hi), 0).rgb;
    return mix(mix(a, b, f.x), mix(cc, d, f.x), f.y);
}

// What the Environment layer shows toward d, blurred by `blur` radians with
// up to `taps` samples on a disc.
fn env_picture(d: vec3<f32>, blur: f32, taps: u32) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(environment));
    let c = (0.5 + 0.5 * vec2<f32>(d.x, -d.y)) * dims;
    let rad = min(0.75 * blur * dims.x, 0.25 * dims.x);
    let one = rad < 0.75 || taps <= 1u;
    // The middle first, then the disc, through one read.
    let count = select(taps, 1u, one);
    let n = f32(taps - 1u);
    var acc = vec3<f32>(0.0);
    for (var i = 0u; i < count; i = i + 1u) {
        var at = c;
        if (i > 0u) {
            let k = i - 1u;
            let a = 2.39996 * f32(k);
            let r = rad * sqrt((f32(k) + 0.5) / n);
            at = c + r * vec2<f32>(cos(a), sin(a));
        }
        acc = acc + env_tap(at, vec2<i32>(dims));
    }
    if (one) {
        return max(acc, vec3<f32>(0.0));
    }
    return max(acc / f32(taps), vec3<f32>(0.0));
}

// Everything around the sheet toward d: the studio, the Environment layer in
// its place as far as its Amount says.
fn surround(d: vec3<f32>, blur: f32, taps: u32) -> vec3<f32> {
    let st = vec3<f32>(studio(d, blur));
    if (!has(F_ENVIRONMENT)) {
        return st;
    }
    let amount = p.b[R_MAPS].z;
    return st * max(1.0 - amount, 0.0) + amount * env_picture(d, blur, taps);
}

// The studio in the mirror, in the coat, and split into colour by the
// grating, through one call of surround(): FXC inlines every call.
fn surroundings(px: Px, acc: ptr<function, Acc>) {
    let nc = lacquer();
    let spread = p.b[R_SPREAD];
    let tv = 1.0 - holo_fresnel(px.co, 1.5);
    let rm = 2.0 * px.co * px.n - px.v;
    let sn_m = 2.0 * nc * px.co_in;
    let soft = sn_m * spread.y;
    let wide = px.spx + px.sc * px.sq;
    let pix = 0.5 * (wide.x + wide.z);
    let blur_m = sqrt(soft * soft + pix) / max(px.co, 0.05);
    let eta_env = eta_nodes(R_ETA_ENV, 1u, 0u, px.fam);
    let mirror = mirror_rgb(0u, sn_m, px.w, eta_env);
    let inv_d = px.inv_d;
    let depth = p.b[R_CENTRE].w;
    // Every bin, or every other on Draft and for a small element, each seen
    // from both sides; none without a grating.
    let step = select(u32(p.b[R_QUALITY].y), 2u, px.lite);
    let bins = select(0u, holo_two() * (6u / step), px.w > 0.0);
    // The mirror, the coat, then the bins.
    for (var i = 0u; i < holo_two() + bins; i = i + 1u) {
        var d = rm;
        var blur = blur_m;
        var taps = 9u;
        var gain = 0.0;
        var wj = vec3<f32>(0.0);
        if (i == 1u) {
            if (px.gloss <= 0.0) {
                continue;
            }
            let ncv = max(dot(px.nc, px.v), 1e-4);
            let alpha = sqrt(px.a2 + px.sc * px.sa2);
            let rough = sqrt(alpha);
            // The coat's share of the studio: Brian Karis's fit of the
            // environment BRDF ("Physically Based Shading on Mobile", Epic
            // Games, 2014), its published constants.
            let r = rough * vec4<f32>(-1.0, -0.0275, -0.572, 0.022)
                + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
            let a004 = min(r.x * r.x, exp2(-9.28 * ncv)) * r.x + r.y;
            let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
            d = 2.0 * ncv * px.nc - px.v;
            blur = 1.414 * alpha;
            taps = 5u;
            gain = (0.04 * ab.x + ab.y) * px.gloss;
        } else if (i > 1u) {
            let m = i - 2u;
            let j = (m / 2u) * step;
            var lam = 425.0 + 50.0 * f32(j);
            wj = p.b[R_BINS + j].xyz;
            if (px.lite) {
                lam = lam + 25.0;
                wj = wj + p.b[R_BINS + j + 1u].xyz;
            }
            let rho = lam * inv_d;
            let sgn = select(-1.0, 1.0, m % 2u == 0u);
            let lt = sgn * rho * px.g - px.vt;
            let r2 = dot(lt, lt);
            if (r2 >= 0.995) {
                continue;
            }
            let ci = sqrt(1.0 - r2);
            d = lt.x * px.t + lt.y * px.b + ci * px.n;
            let sn = nc * (sqrt(max(1.0 - r2 / (nc * nc), 0.0)) + px.co_in);
            let iso = (sn * spread.y) * (sn * spread.y) + pix;
            let sg = sqrt(iso + (rho * px.sig_f) * (rho * px.sig_f));
            let st = sqrt(iso + spread.x * spread.x + (rho * px.sig_th) * (rho * px.sig_th));
            blur = max(sqrt(sg * st), 25.0 * inv_d) / sqrt(max(ci, 0.05));
            let j1 = holo_bessel(1u, min(HC_PI * depth * sn / lam, 5.5));
            let tc = (1.0 - holo_fresnel(ci, 1.5)) * tv;
            let e = eta_at(eta_env, lam).x;
            taps = 4u;
            gain = px.w * tc * j1 * j1 * e;
        }
        let seen = surround(d, blur, taps);
        if (i == 0u) {
            (*acc).lz = (*acc).lz + (tv * tv) * seen * mirror;
        } else if (i == 1u) {
            (*acc).lc = (*acc).lc + seen * gain;
        } else {
            (*acc).lo = (*acc).lo + (gain * dot(seen, HC_LUMA)) * wj;
        }
    }
}

fn qk(x: vec3<f32>, t: vec3<f32>, b: vec3<f32>, eye: vec3<f32>, kp: vec3<f32>) -> vec2<f32> {
    let h = normalize(kp - x) + normalize(eye - x);
    return vec2<f32>(dot(h, t), dot(h, b));
}

// Where the brightest light is, or the eye when there is none.
fn key_light(eye: vec3<f32>) -> vec3<f32> {
    let key = i32(p.b[R_OUTPUT].z);
    if (key >= 0) {
        return p.b[R_LIGHTS + 4u * u32(key)].xyz;
    }
    return eye;
}

// The pixel footprint in q, from how fast the brightest light's half-vector
// sum moves across one pixel.
fn footprint(x: vec3<f32>, t: vec3<f32>, b: vec3<f32>, eye: vec3<f32>, kp: vec3<f32>,
             snk: f32, px_m: f32) -> vec3<f32> {
    let q0 = qk(x, t, b, eye, kp);
    let qx = qk(x + t * px_m, t, b, eye, kp);
    let qy = qk(x - b * px_m, t, b, eye, kp);
    let fw = abs(qx - q0) + abs(qy - q0);
    let cap = 0.09 * snk * snk;
    return vec3<f32>(min(fw.x * fw.x / HC_TAU, cap), 0.0, min(fw.y * fw.y / HC_TAU, cap));
}

// What the shutter sweeps: how far the brightest light's half-vector sum at
// this point of the sheet moved since the pose at the shutter's start, as a
// uniform streak (its variance is a twelfth of its square).
fn shutter(p_mm: vec2<f32>, x: vec3<f32>, t: vec3<f32>, b: vec3<f32>, eye: vec3<f32>,
           kp: vec3<f32>) -> vec3<f32> {
    let pt = p.b[R_PREV_T];
    if (pt.w <= 0.0) {
        return vec3<f32>(0.0);
    }
    let pb = p.b[R_PREV_B].xyz;
    let pk = p.b[R_PREV_KEY];
    let xp = (pt.xyz * p_mm.x + pb * p_mm.y) * 1e-3;
    let kp0 = select(eye, pk.xyz, pk.w > 0.5);
    var dq = qk(x, t, b, eye, kp) - qk(xp, pt.xyz, pb, eye, kp0);
    let len = length(dq);
    if (len > 0.2) {
        dq = dq * (0.2 / len);
    }
    return hr_outer(dq) / 12.0;
}

fn luma_of(c: vec4<f32>) -> f32 {
    return clamp(dot(c.rgb, HC_LUMA), 0.0, 1.0);
}

// A frame tipped by slope s: normal n - s.x t - s.y b, x axis kept as near t
// as it can be.
struct Frame {
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
};

fn tipped(t: vec3<f32>, b: vec3<f32>, n: vec3<f32>, s: vec2<f32>) -> Frame {
    var f: Frame;
    f.n = normalize(n - s.x * t - s.y * b);
    f.t = normalize(t - f.n * dot(t, f.n));
    f.b = cross(f.n, f.t);
    return f;
}

// The relief at this pixel, the sheet's waves, the varnish's ripple, the
// embossed lines and the bow, as slopes and covariances.
struct Surf {
    // The board (foil and varnish together) and the varnish alone.
    sb: vec2<f32>,
    cb: vec3<f32>,
    sc: vec2<f32>,
    cc: vec3<f32>,
    // Thin marks: share of the pixel and covariance inside them.
    c: f32,
    cs: vec3<f32>,
    varnish: bool,
};

// A Height map texel past the picture's edge: repeated (0), mirrored (1) or
// held at the edge (2).
fn height_texel(i: vec2<i32>, dims: vec2<i32>, wrap: u32) -> f32 {
    var at = ((i % dims) + dims) % dims;
    if (wrap == 1u) {
        let m = ((i % (2 * dims)) + 2 * dims) % (2 * dims);
        at = select(m, 2 * dims - vec2<i32>(1) - m, m >= dims);
    } else if (wrap == 2u) {
        at = clamp(i, vec2<i32>(0), dims - vec2<i32>(1));
    }
    return luma_of(textureLoad(height_map, at, 0));
}

// The Height map's brightness at texel position t, bilinear, its four
// texels read in one loop.
fn height_at(t: vec2<f32>, dims: vec2<i32>, wrap: u32) -> f32 {
    let q = t - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    var h = 0.0;
    for (var k = 0u; k < holo_four(); k = k + 1u) {
        let o = vec2<u32>(k & 1u, k >> 1u);
        let w = select(1.0 - f, f, o == vec2<u32>(1u));
        h = h + w.x * w.y * height_texel(i + vec2<i32>(o), dims, wrap);
    }
    return h;
}

// Tap o's weights in the slope of the 2 by 2 quarter whose first tap is c,
// zero outside it.
fn height_quarter(o: vec2<f32>, c: vec2<f32>) -> vec2<f32> {
    let d = o - c;
    let inside = all(d >= vec2<f32>(0.0)) && all(d <= vec2<f32>(1.0));
    return select(vec2<f32>(0.0), vec2<f32>(2.0 * d.x - 1.0, 1.0 - 2.0 * d.y), inside);
}

// The Height map as relief: a Sobel slope over the pixel, repeated `reps`
// times across the sheet, and the spread of the four quarter slopes as what
// the pixel cannot resolve. Slopes are height per width of the map.
fn height_relief(xy: vec2<i32>, reps: f32, wrap: u32) -> HoloRelief {
    var r: HoloRelief;
    let di = vec2<i32>(textureDimensions(height_map));
    let dims = vec2<f32>(di);
    // Mirrored copies alternate, so the position runs on over the whole sheet;
    // held edges keep each copy to itself.
    let u = (vec2<f32>(xy) + 0.5) / dims * reps;
    let t0 = select(fract(u), u, wrap == 1u) * dims;
    let st = max(reps, 1.0);
    // The nine taps summed one by one into the Sobel slope and the slopes of
    // the four quarters, so the read is compiled once. The picture's y runs
    // down, the sheet's up.
    var sobel = vec2<f32>(0.0);
    var s0 = vec2<f32>(0.0);
    var s1 = vec2<f32>(0.0);
    var s2 = vec2<f32>(0.0);
    var s3 = vec2<f32>(0.0);
    for (var k = 0u; k < holo_nine(); k = k + 1u) {
        let o = vec2<f32>(f32(k % 3u) - 1.0, f32(k / 3u) - 1.0);
        let h = height_at(t0 + st * o, di, wrap);
        sobel = sobel + h * vec2<f32>(o.x * (2.0 - abs(o.y)), -o.y * (2.0 - abs(o.x)));
        s0 = s0 + h * height_quarter(o, vec2<f32>(-1.0, -1.0));
        s1 = s1 + h * height_quarter(o, vec2<f32>(0.0, -1.0));
        s2 = s2 + h * height_quarter(o, vec2<f32>(-1.0, 0.0));
        s3 = s3 + h * height_quarter(o, vec2<f32>(0.0, 0.0));
    }
    let mean = sobel / (8.0 * st);
    let k = 1.0 / (2.0 * st);
    let q0 = s0 * k;
    let q1 = s1 * k;
    let q2 = s2 * k;
    let q3 = s3 * k;
    let cv = hr_outer(q0 - mean) + hr_outer(q1 - mean) + hr_outer(q2 - mean)
        + hr_outer(q3 - mean);
    r.m = mean * dims.x;
    r.cv = 0.25 * cv * dims.x * dims.x;
    return r;
}

fn surface(p_mm: vec2<f32>, smm: f32, xy: vec2<i32>) -> Surf {
    var sf: Surf;
    let head = p.b[R_WAVE_HEAD];
    let nb = u32(head.x);
    let ncoat = u32(head.y);
    let n = min(nb + ncoat + u32(head.z), 32u);
    // Filtered a little wider than the pixel, as the relief is.
    let s2 = 2.25 * smm * smm;
    for (var k = 0u; k < n; k = k + 1u) {
        let w = p.b[R_WAVES + k];
        let f = w.xy;
        let arg = HC_TAU * (dot(f, p_mm) + w.w);
        let g = exp(-19.74 * dot(f, f) * s2);
        let amp = HC_TAU * w.z;
        let sl = (g * amp * cos(arg)) * f;
        let cv = 0.5 * (1.0 - g * g) * amp * amp * hr_outer(f);
        let coat = (k >= nb && k < nb + ncoat) || (k >= nb + ncoat && head.w > 0.5);
        if (coat) {
            sf.sc = sf.sc + sl;
            sf.cc = sf.cc + cv;
        } else {
            sf.sb = sf.sb + sl;
            sf.cb = sf.cb + cv;
        }
    }
    // The bow: z = sag (2x / W)^2.
    let bow = p.b[R_BOW];
    if (bow.x != 0.0) {
        if (bow.y < 0.5) {
            sf.sb.x = sf.sb.x + 2.0 * bow.x * p_mm.x / (bow.z * bow.z);
        } else {
            sf.sb.y = sf.sb.y + 2.0 * bow.x * p_mm.y / (bow.w * bow.w);
        }
    }
    let rel = p.b[R_RELIEF];
    let more = p.b[R_RELIEF_MORE];
    var r: HoloRelief;
    if (has(F_HEIGHT)) {
        r = height_relief(xy, more.x, u32(more.w));
    } else if (rel.x < 6.5 && rel.y != 0.0) {
        r = holo_relief(u32(rel.x), p_mm / rel.z, smm / rel.z, u32(more.z));
    }
    let k = rel.y;
    sf.varnish = rel.w > 0.5;
    if (sf.varnish) {
        sf.sc = sf.sc + k * r.m;
        sf.cc = sf.cc + k * k * r.cv;
    } else {
        sf.sb = sf.sb + k * r.m;
        sf.cb = sf.cb + k * k * r.cv;
    }
    sf.c = r.c;
    sf.cs = k * k * r.cs;
    return sf;
}

// The Direction or Pitch map's brightness at texel position c, bilinear,
// held at its edges.
fn map_tap(which: u32, at: vec2<i32>) -> f32 {
    if (which == 0u) {
        return luma_of(textureLoad(direction_map, at, 0));
    }
    return luma_of(textureLoad(pitch_map, at, 0));
}

fn map_dims(which: u32) -> vec2<i32> {
    if (which == 0u) {
        return vec2<i32>(textureDimensions(direction_map));
    }
    return vec2<i32>(textureDimensions(pitch_map));
}

// The map's size comes from the caller: FXC cannot keep a loop round a
// texture's size query inside another loop.
fn map_luma(which: u32, c: vec2<f32>, dims: vec2<i32>) -> f32 {
    let q = c - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    let hi = dims - vec2<i32>(1);
    let a = map_tap(which, clamp(i, vec2<i32>(0), hi));
    let b = map_tap(which, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), hi));
    let cc = map_tap(which, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), hi));
    let d = map_tap(which, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), hi));
    return mix(mix(a, b, f.x), mix(cc, d, f.x), f.y);
}

// The Direction map's brightness gradient at c by central differences h
// pixels apart, per pixel, with y up.
fn map_grad(c: vec2<f32>, h: f32, dims: vec2<i32>) -> vec2<f32> {
    // The four reads in one loop: right less left, then up less down.
    var g = vec2<f32>(0.0);
    for (var k = 0u; k < holo_four(); k = k + 1u) {
        let fore = k == 0u || k == 3u;
        let along = select(vec2<f32>(0.0, h), vec2<f32>(h, 0.0), k < 2u);
        let l = map_luma(0u, c + select(-along, along, fore), dims);
        let sgn = select(-1.0, 1.0, fore);
        if (k < 2u) {
            g.x = g.x + sgn * l;
        } else {
            g.y = g.y - sgn * l;
        }
    }
    return g / (2.0 * h);
}

// Grooves along the Direction map's edges: the groove normal follows the
// brightness gradient, and falls back to `fb` where the map is flat. The
// gradient is taken from the map blurred over a small window (most of a
// millimetre, never under one and a half pixels), at nine points across it,
// and the nine are summed as a structure tensor, so a thin stroke gives one
// calm direction across its whole width instead of flipping at its middle.
// Directions add as doubled angles, since a groove and its reverse are the
// same groove; where they disagree the grooves stray, which widens `sig`.
fn edge_grooves(xy: vec2<i32>, px_mm: f32, fb: vec2<f32>, sig: ptr<function, f32>) -> vec2<f32> {
    let c = vec2<f32>(xy) + 0.5;
    let rad = clamp(0.8 / px_mm, 1.5, 40.0);
    let h = 0.5 * rad;
    var j = vec3<f32>(0.0);
    var wsum = 0.0;
    let dims = map_dims(0u);
    for (var k = 0u; k < holo_nine(); k = k + 1u) {
        let o = vec2<f32>(f32(k % 3u) - 1.0, f32(k / 3u) - 1.0);
        let wt = exp(-0.5 * dot(o, o));
        let g = map_grad(c + 0.6 * rad * o, h, dims) / px_mm;
        j = j + wt * hr_outer(g);
        wsum = wsum + wt;
    }
    j = j / wsum;
    let tr = j.x + j.z;
    let dm2 = vec2<f32>(j.x - j.z, 2.0 * j.y);
    let len = length(dm2);
    // A soft hand-over to the fallback, so the edge of a stroke's reach is
    // not a hard step. Where the window's directions disagree (tight turns,
    // crossing strokes) the grooves lean back toward the fallback and stray
    // more, so the colour blurs there instead of hatching.
    let agree = len / max(tr, 1e-12);
    let edge = tr / (tr + 0.1) * smoothstep(0.25, 0.75, agree);
    let dm = select(vec2<f32>(1.0, 0.0), dm2 / max(len, 1e-12), len > 1e-12);
    let dfb = vec2<f32>(fb.x * fb.x - fb.y * fb.y, 2.0 * fb.x * fb.y);
    let d = edge * dm + (1.0 - edge) * dfb;
    let th = 0.5 * atan2(d.y, d.x);
    let stray = (tr / (tr + 0.1)) * (1.0 - agree) * 0.7;
    *sig = sqrt((*sig) * (*sig) + stray * stray);
    return vec2<f32>(cos(th), sin(th));
}

// Where 1 / pitch `inv_d` sits among the renormalisation's three families,
// whose 1 / pitch run downward: 0 at or above the first, 2 at or below the
// last, linear between.
fn family(inv_d: f32) -> f32 {
    let f = p.b[R_FAMILIES];
    if (inv_d >= f.x) {
        return 0.0;
    }
    if (inv_d >= f.y) {
        return (f.x - inv_d) / max(f.x - f.y, 1e-9);
    }
    return min(1.0 + (f.y - inv_d) / max(f.y - f.z, 1e-9), 2.0);
}

// Vortex: one spiral grating about the centre. Its grooves run out from the
// centre there, at 45 degrees at the Swirl radius and round it at the rim, so
// the pitch shrinks toward the middle; coverage bands follow the spiral.
// Writes the groove normal, pitch, strays and the bands' coverage.
fn vortex(p_mm: vec2<f32>, smm: f32, px: ptr<function, Px>, cover: ptr<function, f32>) {
    let v = p.b[R_VORTEX];
    let swirl = v.x;
    let hand = v.y;
    let spin = p.b[R_MAPS].w;
    let cs = vec2<f32>(cos(spin), sin(spin));
    let d0 = p_mm - p.b[R_CENTRE].xy;
    // Turn the point back by Spin, work in the spiral's own frame, then turn
    // the groove normal forward again.
    let dd = vec2<f32>(cs.x * d0.x + cs.y * d0.y, -cs.y * d0.x + cs.x * d0.y);
    let dl = length(dd);
    let r = max(dl, 0.02);
    let rh = select(vec2<f32>(1.0, 0.0), dd / dl, dl > 1e-6);
    let th = vec2<f32>(-rh.y, rh.x);
    let k = hand * swirl / r;
    let gl = normalize(rh + k * th);
    (*px).g = vec2<f32>(cs.x * gl.x - cs.y * gl.y, cs.y * gl.x + cs.x * gl.y);
    (*px).inv_d = (*px).inv_d * sqrt(1.0 + k * k);
    let r2 = r * r;
    let s2 = r2 + swirl * swirl;
    let turn = sqrt(1.0 / r2 + (swirl / s2) * (swirl / s2)) * smm;
    (*px).sig_th = sqrt(3.046e-6 + turn * turn);
    let stretch = swirl * swirl / (r * s2) * smm;
    (*px).sig_f = sqrt(4e-6 + stretch * stretch);
    let bands = v.z;
    if (bands <= 0.0) {
        return;
    }
    let rb = max(swirl, 1.0);
    let per = bands / HC_TAU;
    let xi = per * (atan2(dd.y, dd.x) + hand * r / rb);
    let h = 0.5 * per * sqrt(1.0 / r2 + 1.0 / (rb * rb)) * 2.50662827 * smm;
    var on = step(fract(xi), 0.5);
    if (h >= 1e-4) {
        on = (band_area(xi + h) - band_area(xi - h)) / (2.0 * h);
    }
    *cover = 1.0 - v.w * on;
}

// How much of the bands' first halves lies below x: half a band per whole
// band, plus the part of this one.
fn band_area(x: f32) -> f32 {
    return 0.5 * floor(x) + min(fract(x), 0.5);
}

// The layer's alpha in halved copy `level` (1 is half size), nothing
// outside.
fn bevel_texel(level: u32, c: vec2<i32>, dims: vec2<i32>) -> f32 {
    if (any(c < vec2<i32>(0)) || any(c >= dims)) {
        return 0.0;
    }
    return textureLoad(bevel_mips, c, i32(level) - 1).a;
}

// The cubic B-spline's four weights at t, and their slopes.
fn bevel_spline(t: f32) -> vec4<f32> {
    let u = 1.0 - t;
    let t2 = t * t;
    return vec4<f32>(u * u * u, 3.0 * t2 * t - 6.0 * t2 + 4.0,
                     -3.0 * t2 * t + 3.0 * t2 + 3.0 * t + 1.0, t2 * t) / 6.0;
}

fn bevel_dspline(t: f32) -> vec4<f32> {
    let u = 1.0 - t;
    return vec4<f32>(-u * u, 3.0 * t * t - 4.0 * t, -3.0 * t * t + 2.0 * t + 1.0, t * t) / 2.0;
}

// The alpha at c (pixel units of the layer, which is d0 pixels) read from
// halved copy `level` (dims texels) through a cubic B-spline, so it is smooth
// with no blocks, and its slope per pixel along x and y (y down).
fn bevel_level(level: u32, c0: vec2<f32>, d0: vec2<f32>, dims: vec2<i32>) -> vec3<f32> {
    let k = vec2<f32>(dims) / d0;
    let q = c0 * k - 0.5;
    let i = vec2<i32>(floor(q));
    let f = q - floor(q);
    let wx = bevel_spline(f.x);
    let wy = bevel_spline(f.y);
    let dx = bevel_dspline(f.x);
    let dy = bevel_dspline(f.y);
    var acc = vec3<f32>(0.0);
    for (var y = 0u; y < holo_four(); y = y + 1u) {
        for (var x = 0u; x < holo_four(); x = x + 1u) {
            let a = bevel_texel(level, i + vec2<i32>(i32(x) - 1, i32(y) - 1), dims);
            acc = acc + a * vec3<f32>(wx[x] * wy[y], dx[x] * wy[y] * k.x, wx[x] * dy[y] * k.y);
        }
    }
    return acc;
}

// The inverse of the error function, to within a few parts in a thousand:
// Sergei Winitzki's closed form with a = 0.147 ("A handy approximation for
// the error function and its inverse", 2008).
fn bevel_erfinv(x: f32) -> f32 {
    let w = log(max(1.0 - x * x, 1e-7));
    let t = 4.3307 + 0.5 * w;
    return sign(x) * sqrt(sqrt(t * t - w / 0.147) - t);
}

// Bevel from alpha: the layer's alpha blurred over about half the bevel's
// width (wider with Softness), read between two halved copies, gives how far
// in from the outline each pixel lies and which way the outline runs. The
// wall's profile turns that into a slope, so the sheet is pressed into a
// raised or sunk logo whose shoulders catch the lights. Where a stroke is
// narrower than the wall, the blurred alpha flattens and so does the slope.
// The packer holds the wall to what the halved copies can blur (surface.rs),
// so it is the same share of the sheet at every raster size.
// Returns the slope and the variance of what the pixel cannot resolve.
fn bevel(xy: vec2<i32>) -> vec4<f32> {
    let bv = p.b[R_BEVEL];
    let px_mm = p.b[R_SIZE].z;
    let profile = u32(bv.z);
    let reach = select(1.0, 2.0, profile == 2u);
    let r_px = bv.x * reach / px_mm;
    let sigma = max(0.45 * r_px * (1.0 + bv.w), 0.8);
    let top = max(textureNumLevels(bevel_mips), 1u);
    let lf = clamp(log2(sigma / 0.645), 1.0, f32(top));
    let l0 = u32(floor(lf));
    let l1 = min(l0 + 1u, top);
    let d0 = vec2<f32>(textureDimensions(src));
    let dims0 = vec2<i32>(textureDimensions(bevel_mips, i32(l0) - 1));
    let dims1 = vec2<i32>(textureDimensions(bevel_mips, i32(l1) - 1));
    let c0 = vec2<f32>(xy) + 0.5;
    // The two copies through one call.
    var v = vec3<f32>(0.0);
    for (var k = 0u; k < holo_two(); k = k + 1u) {
        let hi = k == 1u;
        let wt = select(1.0 - fract(lf), fract(lf), hi);
        v = v + wt * bevel_level(select(l0, l1, hi), c0, d0, select(dims0, dims1, hi));
    }
    let u = clamp(2.0 * v.x - 1.0, 0.0, 0.9999);
    if (u <= 0.0) {
        return vec4<f32>(0.0);
    }
    // How far in, in blur widths, then as a share of the wall.
    let z = 1.41421356 * bevel_erfinv(u);
    let t = z * sigma / r_px;
    var wall = 0.0;
    if (t < 1.0) {
        if (profile == 1u) {
            // A flat slope with rounded ends a fifth of the wall long.
            wall = 1.25 * clamp(min(t, 1.0 - t) / 0.2, 0.0, 1.0);
        } else if (profile == 2u) {
            // A cushion: steepest at the outline, level at its top.
            wall = 2.0 * (1.0 - t);
        } else {
            wall = 1.5707963 * sin(HC_PI * t);
        }
    }
    if (profile == 3u) {
        wall = -wall;
    }
    // The blurred alpha's slope divided by a straight edge's, so the
    // direction is the outline's and a thin stroke's middle stays level.
    let edge = exp(-0.5 * z * z) * 0.39894228 / sigma;
    let dir = vec2<f32>(v.y, -v.z) / max(edge, 1e-4);
    let s = (bv.y / (bv.x * reach)) * wall * dir;
    let spread = 0.5 * length(s) / max(r_px, 1.0);
    return vec4<f32>(s, spread * spread, 0.0);
}

// Every light and the studio on the element px describes.
fn lit(px: Px, wear: bool, hair: ptr<function, HoloHairlines>) -> Acc {
    var acc: Acc;
    acc.lo = vec3<f32>(0.0);
    acc.lz = vec3<f32>(0.0);
    acc.lc = vec3<f32>(0.0);
    acc.ed = vec3<f32>(0.0);
    lamps(px, wear, hair, &acc);
    panel(px, &acc);
    surroundings(px, &acc);
    return acc;
}

// A point of the tipped sheet with everything its lights need, and what wear
// leaves there.
struct Sheet {
    px: Px,
    p_mm: vec2<f32>,
    smm: f32,
    // The sheet's own frame, before any relief.
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
    wear: f32,
    mottle: f32,
    dust: f32,
    hair: HoloHairlines,
};

// The sheet at pixel xy: where it sits, the eye, the footprint, the relief's
// frames, the varnish and the wear. The grating is left for the caller. bev
// adds a slope and its unresolved variance to the board (a bevel).
fn sheet_at(xy: vec2<i32>, bev: vec4<f32>) -> Sheet {
    var sh: Sheet;
    let size = p.b[R_SIZE];
    // Where this point sits on the tipped sheet, metres.
    let px_mm = size.z;
    let smm = 0.39894228 * px_mm;
    let p_mm = vec2<f32>((f32(xy.x) + 0.5 - 0.5 * size.x) * px_mm,
                         (0.5 * size.y - f32(xy.y) - 0.5) * px_mm);
    let t = p.b[R_T].xyz;
    let b = p.b[R_B].xyz;
    let n = p.b[R_N].xyz;
    let x = (t * p_mm.x + b * p_mm.y) * 1e-3;
    let eye = vec3<f32>(0.0, 0.0, size.w);
    let v = normalize(eye - x);
    let kp = key_light(eye);
    var snk = 2.0;
    if (i32(p.b[R_OUTPUT].z) >= 0) {
        snk = max(dot(n, normalize(kp - x) + v), 0.25);
    }

    // The relief bends the whole board, or only the varnish, whose slope
    // the foil sees turned back by refraction.
    var sf = surface(p_mm, smm, xy);
    sf.sb = sf.sb + bev.xy;
    sf.cb = sf.cb + vec3<f32>(bev.z, 0.0, bev.z);
    let foil = tipped(t, b, n, sf.sb - sf.sc / snk);
    let coat = tipped(t, b, n, sf.sb + sf.sc);

    var px: Px;
    px.x = x;
    px.v = v;
    px.t = foil.t;
    px.b = foil.b;
    px.n = foil.n;
    px.nc = coat.n;
    px.vt = vec2<f32>(dot(v, px.t), dot(v, px.b));
    px.co = max(dot(v, px.n), 1e-4);
    let nc = lacquer();
    px.co_in = sqrt(max(1.0 - dot(px.vt, px.vt) / (nc * nc), 0.0));
    px.spx = footprint(x, t, b, eye, kp, snk, px_mm * 1e-3) + shutter(p_mm, x, t, b, eye, kp)
        + snk * snk * sf.cb + sf.cc;
    px.sc = sf.c;
    px.sq = select(snk * snk, 1.0, sf.varnish) * sf.cs;
    px.sa2 = sf.cs.x + sf.cs.z;

    // Wear: smudges dull the gloss, hairlines and dust mark the varnish, the
    // grating's efficiency drifts in slow patches.
    let more = p.b[R_RELIEF_MORE];
    let wear = select(0.0, more.y, FULL);
    let seed = u32(more.z);
    var alpha = p.b[R_T].w;
    px.gloss = 1.0;
    sh.mottle = 1.0;
    sh.dust = 0.0;
    if (wear > 0.0) {
        var smudge = 0.0;
        for (var k = 0u; k < 3u; k = k + 1u) {
            let a = p.b[R_SMUDGES + 2u * k];
            let c = p.b[R_SMUDGES + 2u * k + 1u];
            let d = p_mm - a.xy;
            let qd = a.z * d.x * d.x + 2.0 * a.w * d.x * d.y + c.x * d.y * d.y;
            smudge = max(smudge, exp(-0.5 * qd));
        }
        alpha = alpha + 0.3 * wear * smudge;
        px.gloss = 1.0 - 0.35 * wear * smudge;
        // The oil film scatters what the foil sends back, too.
        let haze = 0.05 * wear * smudge;
        px.spx = px.spx + vec3<f32>(haze * haze, 0.0, haze * haze);
        // Patchy foil: slow noise over 5 to 20 mm.
        let salt = hr_pcg3(vec3<u32>(seed, 13u, 577u)).x;
        var m = 0.0;
        for (var k = 0u; k < holo_two(); k = k + 1u) {
            let n = hr_noise(p_mm / select(6.0, 14.0, k == 0u), salt + k).x;
            m = m + select(0.5, 1.0, k == 0u) * n;
        }
        sh.mottle = 1.0 + 0.5 * wear * clamp(1.6 * m, -1.0, 1.0);
        sh.hair = holo_hairlines(p_mm, smm, 0.6 * wear, seed);
        sh.dust = holo_dust(p_mm, smm, 0.3 * wear, seed);
    }
    px.a2 = alpha * alpha + sf.cb.x + sf.cb.z + sf.cc.x + sf.cc.z;
    sh.px = px;
    sh.p_mm = p_mm;
    sh.smm = smm;
    sh.t = t;
    sh.b = b;
    sh.n = n;
    sh.wear = wear;
    return sh;
}

// The light a printed or dusty surface facing n sends back: the lights' and
// the studio's, or the Environment layer's in its place. On the sheet's back
// (`back`) the lights' and the studio's light from that side, as packed.
fn diffuse_light(acc: Acc, n: vec3<f32>, back: bool) -> vec3<f32> {
    var ambient = vec3<f32>(select(p.b[R_AMBIENT].x, p.b[R_AMBIENT].z, back));
    if (has(F_ENVIRONMENT)) {
        let amount = min(p.b[R_MAPS].z, 1.0);
        ambient = ambient * (1.0 - amount) + amount * HC_PI * env_picture(n, 1.2, 9u);
    }
    let lights = select(acc.ed, vec3<f32>(p.b[R_AMBIENT].w), back);
    return (lights + ambient) / HC_PI;
}

// Exposure, back into gamut, the highlights, then premultiplied by the
// layer's alpha and mixed with the input s.
fn finish(c0: vec3<f32>, s: vec4<f32>) -> vec4<f32> {
    var c = holo_gamut(c0 * p.b[R_B].w);
    let highlights = u32(p.b[R_OUTPUT].y);
    if (highlights == 1u) {
        c = holo_neutral(c);
    } else if (highlights == 2u) {
        c = holo_agx(c);
    }
    c = clamp(select(c, vec3<f32>(0.0), c != c), vec3<f32>(0.0), vec3<f32>(60000.0));
    let result = vec4<f32>(c * s.a, s.a);
    return mix(s, result, p.b[R_N].w);
}

@compute @workgroup_size(8, 8)
fn holographic_foil(@builtin(global_invocation_id) gid: vec3<u32>) {
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
    let straight = s.rgb / s.a;
    // The bevel; the pass leaves its row empty when a Height map takes the
    // relief's place.
    var bev = vec4<f32>(0.0);
    if (FULL && p.b[R_BEVEL].x > 0.0) {
        bev = bevel(xy);
    }
    var sh = sheet_at(xy, bev);
    var px = sh.px;
    let p_mm = sh.p_mm;
    let smm = sh.smm;
    let px_mm = p.b[R_SIZE].z;
    let wear = sh.wear;

    // The grating at this point.
    let grating = p.b[R_GRATING];
    let maps = p.b[R_MAPS];
    var g = grating.xy;
    var sig_th = p.b[R_SPREAD].z;
    px.sig_f = p.b[R_SPREAD].w;
    px.inv_d = p.b[R_CENTRE].z;
    if (has(F_PITCH)) {
        let lp = map_luma(1u, vec2<f32>(xy) + 0.5, map_dims(1u));
        px.inv_d = 1.0 / mix(1.0 / px.inv_d, 1.0 / maps.y, lp);
    }
    var cover = 1.0;
    let pattern = u32(grating.z);
    let material = MATERIAL;
    if (material == 4u) {
        vortex(p_mm, smm, &px, &cover);
        g = px.g;
        sig_th = px.sig_th;
    } else if (pattern != 0u) {
        let r = p_mm - p.b[R_CENTRE].xy;
        let rl = length(r);
        let u = select(vec2<f32>(1.0, 0.0), r / rl, rl >= 0.05);
        g = select(vec2<f32>(-u.y, u.x), u, pattern != 2u || rl < 0.05);
        let s_px = px_mm * 0.39894228 / max(rl, 0.5);
        sig_th = sqrt(sig_th * sig_th + s_px * s_px);
        if (pattern == 3u && has(F_DIRECTION)) {
            if (maps.x < 0.5) {
                let a = map_luma(0u, vec2<f32>(xy) + 0.5, map_dims(0u)) * HC_PI + maps.w;
                g = vec2<f32>(sin(a), -cos(a));
            } else {
                g = edge_grooves(xy, px_mm, g, &sig_th);
            }
        }
    }
    px.g = g;
    px.sig_th = sig_th;
    px.fam = family(px.inv_d);
    var wk = sh.mottle * cover;
    if (has(F_MATTE)) {
        wk = wk * luma_of(textureLoad(matte, xy, 0));
    }
    px.w = clamp(grating.w * wk, 0.0, 1.0);

    px.cspx = px.spx;
    var acc: Acc;
    if (material == 1u) {
        acc = holo_shatter(p_mm, smm, px, wk, wear > 0.0, &sh.hair);
    } else if (material == 2u) {
        acc = holo_glitter(p_mm, smm, px, wear > 0.0, &sh.hair);
    } else if (material == 3u) {
        acc = holo_sparkle(p_mm, smm, px, wear > 0.0, &sh.hair);
    } else {
        acc = lit(px, wear > 0.0, &sh.hair);
    }

    // The print over the foil: white underprint W, ink T2 over bare foil, and
    // the colour printed on white.
    let mode = u32(p.b[R_OUTPUT].x);
    let ink = clamp(straight, vec3<f32>(0.0), vec3<f32>(1.0));
    var bare = 1.0;
    if (has(F_MASK)) {
        bare = luma_of(textureLoad(foil_mask, xy, 0));
    }
    var t2 = vec3<f32>(1.0);
    var on_white = vec3<f32>(0.85);
    if (mode == 0u) {
        t2 = ink;
        on_white = 0.85 * ink;
    } else if (mode == 2u) {
        on_white = ink;
        if (!has(F_MASK)) {
            bare = dot(ink, HC_LUMA);
        }
    }
    let white = 1.0 - bare;
    // The eye on the sheet's back sees plain satin metal, no grating.
    let back = dot(sh.n, px.v) <= 0.0;
    let diffuse = diffuse_light(acc, select(px.n, -sh.n, back), back);
    var c = acc.lc + white * on_white * diffuse + (1.0 - white) * t2 * (acc.lz + acc.lo);
    if (sh.dust > 0.0) {
        let grazing = 1.0 - px.co;
        c = mix(c, 0.4 * diffuse * (1.0 + 3.0 * grazing * grazing), sh.dust);
    }
    // The back: plain satin metal under the rig and the studio, with no
    // grating, so a sheet turned past edge-on never goes black.
    if (back) {
        c = BACK_SATIN * diffuse;
    }
    textureStore(dst, xy, finish(c, s));
}
