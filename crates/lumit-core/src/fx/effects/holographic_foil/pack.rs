//! Holographic foil's uniform: every number the kernel reads, as rows of four
//! floats. All the trigonometry and the spectral sums happen here, once a
//! frame, so the kernel only does per-pixel work.
//!
//! The rows, mirrored by the constants at the top of
//! `fx_holographic_foil.wgsl`:
//!
//! | Row | Holds |
//! |---|---|
//! | 0 | raster width, height, mm per pixel, view distance in m |
//! | 1 | sheet x axis (T), coat roughness |
//! | 2 | sheet y axis (B), exposure gain |
//! | 3 | sheet normal (N), mix |
//! | 4 | groove normal x, y on the sheet, pattern, coverage |
//! | 5 | pattern centre x, y in mm, 1 / pitch in 1/nm, depth in nm |
//! | 6 | fan, softness, orientation jitter (all radians), pitch jitter share |
//! | 7 | tint kind, edge nm, edge width nm, floor |
//! | 8 | print, highlights, key light slot (-1 none), lacquer index |
//! | 9 | studio rims, fill, scrim, room |
//! | 10 | diffuse light from the studio on the sheet, material, the studio's and the lights' on its back |
//! | 11..27 | four lights (spot A, B, C, panel), four rows each: position in m and radiance (0 off); light colour and spectrum; spot radius in m, or the panel's half width as a vector and its cells across; the panel's half length as a vector and its cells along |
//! | 27..47 | five spectra, four rows each: first term, term count, Planck temperature, Planck factor; then the node projection by rows |
//! | 47..53 | the environment's six wavelength bins, linear sRGB |
//! | 53..65 | the studio's four panels, three rows each: centre and level; the axis out from the view axis and half width in radians; the axis around it and half length |
//! | 65..191 | colour terms, two rows each: amplitude rgb and centre; width, cut, side |
//! | 192..240 | the surface, as `surface.rs` says |
//! | 240, 241 | the sheet's x and y axes at the shutter's start, motion blur on |
//! | 242 | the brightest light at the shutter's start, 1 when there is one |
//! | 243 | shard size in mm, irregularity, mean cuts per cell, groove directions (0 any) |
//! | 244 | vortex swirl in mm, turn (+1 anticlockwise, -1 clockwise), bands, band contrast |
//! | 245 | tangent of the shard tilt, chirp share, crease height and half width in mm |
//! | 246..258 | the orders' renormalisation at the four nodes, by pitch family then light |
//! | 258..270 | the mirror's, laid out the same |
//! | 270..273 | the studio's, by pitch family |
//! | 273 | 1 / pitch of the three families: falling, or shatter's A, B, C |
//! | 274 | share of creased cracks, a shard's aperture variance in q |
//! | 275 | Quality: highest order, environment bin step, picks lit on their own, trace steps |
//! | 276 | Quality: a flake's orders, spots (3), 0, trace pixels per step |
//! | 277 | Quality: candidate window half width in cells, footprint in cells for the wide window |
//! | 278 | loop counts: 4, 9, 2, 3 |
//! | 279 | the bevel, as `surface.rs` says |
//! | 280 | flake size in mm, shape, layers, tangent of the flake tilt |
//! | 281 | flake curve in radians, 1 / widest pitch, 1 / finest pitch, a flake's aperture variance |
//! | 282 | star size in mm, share of star places filled |
//! | 283 | base colour |
//! | 284 | dot size in mm, dot cover, irregularity, tangent of the dot tilt |
//! | 285 | dome in radians, speck share, speck size share, dot lattice pitch in mm |
//! | 286 | sand pitch in mm, tangent of the sand tilt, the sand's own roughness in radians |
//! | 287 | the specks' dye as a colour ratio |
//! | 288.. | band colour tables, as `table.rs` says |

use super::eta::{nodes, nodes_mean, Grating};
use super::lights::{pad_to_dir, radial_axes, rig_of, Light, Rig, STUDIO};
use super::memo::{self, Memo};
use super::motion::{column, frame, Motion};
use super::spectra::{
    bins, equal_terms, fluorescent_terms, led_terms, planck_factor, projection, speck_tint, Term,
    Tint, FLUORESCENT, LED_SPECTRUM, SPECTRA,
};
use super::{surface, table, HolographicFoil, BLOCKS, GLITTER, SHATTER, SPARKLE, VORTEX};

const SIZE: usize = 0;
const T_AXIS: usize = 1;
const B_AXIS: usize = 2;
const N_AXIS: usize = 3;
const GRATING: usize = 4;
const CENTRE: usize = 5;
const SPREAD: usize = 6;
const TINT: usize = 7;
const OUTPUT: usize = 8;
const STUDIO_LEVELS: usize = 9;
const AMBIENT: usize = 10;
const LIGHTS: usize = 11;
const SPECTRA_AT: usize = 27;
const BINS_AT: usize = 47;
const STUDIO_AT: usize = 53;
const TERMS_AT: usize = 65;
/// Where the surface rows begin.
const SURFACE_AT: usize = 192;
/// Room for the longest lists: 14 lobes, 28 LED terms and 21 fluorescent.
const TERMS_MAX: usize = (SURFACE_AT - TERMS_AT) / 2;
const PREV_T: usize = 240;
const PREV_B: usize = 241;
const PREV_KEY: usize = 242;
const SHATTER_ROW: usize = 243;
const VORTEX_ROW: usize = 244;
const SHATTER_MORE: usize = 245;
const ETA: usize = 246;
const ETA_MIRROR: usize = 258;
const ETA_ENV: usize = 270;
const FAMILIES: usize = 273;
const SHATTER_CREASE: usize = 274;
const QUALITY: usize = 275;
const GLITTER_ROW: usize = 280;
const GLITTER_MORE: usize = 281;
const STARS: usize = 282;
const BASE_INK: usize = 283;
const SPARKLE_ROW: usize = 284;
const SPARKLE_MORE: usize = 285;
const SAND: usize = 286;
const SPECK_TINT: usize = 287;
/// Where the band tables begin, as `table.rs` says.
const TABLES_AT: usize = 288;

/// The slow parts' recent results: the spectra and their terms by the tint,
/// the band tables by the spectra used and the tint, the renormalisation by
/// everything it reads.
static SPECTRAL: Memo = memo::memo();
static TABLES: Memo = memo::memo();
static RENORMALISED: Memo = memo::memo();

type Blocks = [[f32; 4]; BLOCKS];

fn put(b: &mut [[f32; 4]], row: usize, v: [f64; 4]) {
    if let Some(r) = b.get_mut(row) {
        *r = v.map(|x| x as f32);
    }
}

fn copy_row(b: &mut [[f32; 4]], from: usize, to: usize) {
    if let (Some(&v), true) = (b.get(from), to < b.len()) {
        put(b, to, v.map(f64::from));
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn scale(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|x| x * k)
}

/// The rows for `f` on a `w` by `h` raster, posed as `m` says.
pub(crate) fn blocks(f: &HolographicFoil, w: u32, h: u32, m: &Motion) -> Blocks {
    let mut b: Blocks = [[0.0; 4]; BLOCKS];
    let w = w.max(1);
    let px_mm = f64::from(f.sheet_width.max(1.0)) / f64::from(w);
    let view = f64::from(f.view_distance.max(0.05));
    put(&mut b, SIZE, [f64::from(w), f64::from(h), px_mm, view]);

    // The sheet tips about x, then turns about y, inside the layer's own turn.
    let pose = frame(f, m.time, m.turn);
    let t_axis = column(&pose, 0);
    let b_axis = column(&pose, 1);
    let n_axis = column(&pose, 2);
    let [t0, t1, t2] = t_axis;
    let [b0, b1, b2] = b_axis;
    let [n0, n1, n2] = n_axis;
    put(
        &mut b,
        T_AXIS,
        [t0, t1, t2, f64::from(f.coat_roughness.max(0.005))],
    );
    put(&mut b, B_AXIS, [b0, b1, b2, f64::from(f.exposure).exp2()]);
    put(
        &mut b,
        N_AXIS,
        [n0, n1, n2, f64::from(f.mix / 100.0).clamp(0.0, 1.0)],
    );

    let (ga_s, ga_c) = f64::from(f.groove_angle + f.spin).to_radians().sin_cos();
    let coverage = f64::from(f.coverage / 100.0).clamp(0.0, 1.0);
    put(
        &mut b,
        GRATING,
        [ga_s, -ga_c, f64::from(f.pattern), coverage],
    );
    let wmm = f64::from(w) * px_mm;
    let hmm = f64::from(h) * px_mm;
    put(
        &mut b,
        CENTRE,
        [
            (f64::from(f.centre_x) / 100.0 - 0.5) * wmm,
            (0.5 - f64::from(f.centre_y) / 100.0) * hmm,
            1.0 / f64::from(f.pitch.max(100.0)),
            f64::from(f.depth.max(0.0)),
        ],
    );
    // Flakes have no fan, metal roughness or strays of their own.
    let spread = if f.material == GLITTER {
        [0.0; 4]
    } else {
        [
            f64::from(f.fan.max(0.0)).to_radians(),
            f64::from(f.softness.max(0.0)).to_radians(),
            f64::from(f.orientation_jitter.max(0.0)).to_radians(),
            f64::from(f.pitch_jitter.max(0.0)) / 100.0,
        ]
    };
    put(&mut b, SPREAD, spread);
    let tint = tint_of(f);
    put(
        &mut b,
        TINT,
        [f64::from(tint.kind), tint.edge, tint.width, tint.floor],
    );

    // The rig, turned about the view axis.
    let rig = rig_of(f);
    let rot = f64::from(f.rig_rotation).to_radians();
    let lights = [rig.spots[0], rig.spots[1], rig.spots[2], rig.panel];
    let mut key = (-1.0, 0.0);
    // The lights' light on the sheet's back, which faces the rig once the
    // sheet is turned past edge-on.
    let mut behind = 0.0;
    for (slot, light) in lights.iter().enumerate() {
        let [e, back] = put_light(&mut b, slot, light, rot, n_axis, &tint, f.quality);
        behind += back;
        if e > key.1 {
            key = (slot as f64, e);
        }
    }
    let nc = f64::from(f.lacquer.clamp(1.0, 3.0));
    put(
        &mut b,
        OUTPUT,
        [f64::from(f.print), f64::from(f.highlights), key.0, nc],
    );
    put_quality(&mut b, f.quality);
    put(&mut b, STUDIO_LEVELS, rig.studio.map(f64::from));
    let ambient = put_studio(&mut b, rig.studio, rot, n_axis);
    let around = studio_light(rig.studio, rot, n_axis.map(|v| -v));
    put(
        &mut b,
        AMBIENT,
        [ambient, f64::from(f.material), around, behind],
    );

    // The spectra, with their terms, and the environment's bins.
    let tint_bits = [f64::from(tint.kind), tint.edge, tint.width, tint.floor];
    memo::rows(
        &SPECTRAL,
        memo::key(&tint_bits),
        &mut b,
        &[(SPECTRA_AT, STUDIO_AT), (TERMS_AT, SURFACE_AT)],
        |b| put_spectra(b, tint),
    );
    surface::put(f, wmm, hmm, &mut |row, v| put(&mut b, row, v));

    put(
        &mut b,
        VORTEX_ROW,
        [
            f64::from(f.swirl.max(0.0)),
            if f.handedness == 1 { -1.0 } else { 1.0 },
            f64::from(f.bands.max(0)),
            f64::from(f.band_contrast / 100.0).clamp(0.0, 1.0),
        ],
    );
    put_shatter(&mut b, f);
    put_glitter(&mut b, f);
    put_sparkle(&mut b, f, &tint);
    // Tables for the spectra the lights use, the brightest light's first,
    // only where small elements read them.
    let mut used: Vec<usize> = Vec::with_capacity(5);
    if f.material == GLITTER || f.material == SPARKLE {
        let order = [key.0 as usize, 0, 1, 2, 3];
        for slot in order.into_iter().filter(|&s| s < 4) {
            let l = lights[slot];
            if l.on && l.intensity > 0.0 {
                used.push((l.spectrum as usize).min(SPECTRA - 1));
            }
        }
    }
    let mut wanted: Vec<f64> = used.iter().map(|&s| s as f64).collect();
    wanted.extend(tint_bits);
    memo::rows(
        &TABLES,
        memo::key(&wanted),
        &mut b,
        &[(TABLES_AT, BLOCKS)],
        |b| table::put(&used, tint, &mut |row, v| put(b, row, v)),
    );
    let mut read: Vec<f64> = [
        f.pitch,
        f.pitch_a,
        f.pitch_b,
        f.pitch_c,
        f.pitch_min,
        f.pitch_max,
        f.depth,
        f.lacquer,
        f.groove_angle,
        f.spin,
    ]
    .map(f64::from)
    .to_vec();
    read.extend([f64::from(f.material), f64::from(f.pattern), rot]);
    read.extend(tint_bits);
    read.extend(t_axis.into_iter().chain(b_axis));
    read.extend(lights.iter().flat_map(|l| [f64::from(l.x), f64::from(l.y)]));
    memo::rows(
        &RENORMALISED,
        memo::key(&read),
        &mut b,
        &[(ETA, FAMILIES + 1)],
        |b| put_eta(b, f, &rig, rot, &tint, [t_axis, b_axis]),
    );
    if let Some(prev) = &m.prev {
        let then = frame(&prev.f, prev.time, prev.turn);
        let [pt, pb] = [column(&then, 0), column(&then, 1)];
        put(&mut b, PREV_T, [pt[0], pt[1], pt[2], 1.0]);
        put(&mut b, PREV_B, [pb[0], pb[1], pb[2], 0.0]);
        // The same light, where it was.
        let was = rig_of(&prev.f);
        let lights = [was.spots[0], was.spots[1], was.spots[2], was.panel];
        if let Some(l) = usize::try_from(key.0 as i64)
            .ok()
            .and_then(|k| lights.get(k))
        {
            let rot = f64::from(prev.f.rig_rotation).to_radians();
            let dir = pad_to_dir(f64::from(l.x), f64::from(l.y), rot);
            let pos = scale(dir, f64::from(l.distance.max(0.05)));
            put(&mut b, PREV_KEY, [pos[0], pos[1], pos[2], 1.0]);
        }
    }
    b
}

/// Writes the spectra's rows and their colour terms, and the environment's
/// bins, for `tint`.
fn put_spectra(b: &mut [[f32; 4]], tint: Tint) {
    let mut next = 0usize;
    let mut write_terms = |b: &mut [[f32; 4]], terms: &mut dyn Iterator<Item = Term>| {
        let first = next;
        for t in terms {
            if next >= TERMS_MAX {
                break;
            }
            let row = TERMS_AT + 2 * next;
            put(b, row, [t.amp[0], t.amp[1], t.amp[2], t.c]);
            put(b, row + 1, [t.s, t.cut, t.side, 0.0]);
            next += 1;
        }
        (first, next - first)
    };
    let lobes = write_terms(b, &mut equal_terms());
    let led = write_terms(b, &mut led_terms());
    let tube = write_terms(b, &mut fluorescent_terms(tint));
    for s in 0..SPECTRA {
        let (first, count) = match s {
            LED_SPECTRUM => led,
            FLUORESCENT => tube,
            _ => lobes,
        };
        let [temp, k] = planck_factor(s);
        let row = SPECTRA_AT + 4 * s;
        put(b, row, [first as f64, count as f64, temp, k]);
        let p = projection(s, tint);
        for (c, r) in p.project.iter().enumerate() {
            put(b, row + 1 + c, *r);
        }
    }
    for (j, bin) in bins(tint).iter().enumerate() {
        put(b, BINS_AT + j, [bin[0], bin[1], bin[2], 0.0]);
    }
}

/// Writes the renormalisation rows: for each pitch family and light, at the
/// sheet's centre. Straight grooves use their real direction; grooves that
/// turn use the direction each light's orders need, and the mean over
/// directions for the mirror and the studio.
fn put_eta(
    b: &mut [[f32; 4]],
    f: &HolographicFoil,
    rig: &Rig,
    rot: f64,
    tint: &Tint,
    axes: [[f64; 3]; 2],
) {
    let d0 = f64::from(f.pitch.max(100.0));
    let pitches = match f.material {
        VORTEX => [300.0, 0.7 * d0, d0],
        SHATTER => [f.pitch_a, f.pitch_b, f.pitch_c].map(|d| f64::from(d.max(100.0))),
        GLITTER => {
            let [lo, hi] = pitch_range(f);
            [lo, 2.0 / (1.0 / lo + 1.0 / hi), hi]
        }
        _ => [d0; 3],
    };
    put(
        b,
        FAMILIES,
        [1.0 / pitches[0], 1.0 / pitches[1], 1.0 / pitches[2], 0.0],
    );
    let straight = f.pattern == 0 && ![VORTEX, SHATTER, GLITTER].contains(&f.material);
    let (ga_s, ga_c) = f64::from(f.groove_angle + f.spin).to_radians().sin_cos();
    let g_line = [ga_s, -ga_c];
    let lights = [rig.spots[0], rig.spots[1], rig.spots[2], rig.panel];
    let on_sheet = |v: [f64; 3]| [dot(v, axes[0]), dot(v, axes[1])];
    // The eye looks straight down the view axis at the centre.
    let vt = on_sheet([0.0, 0.0, 1.0]);
    for (fam, d) in pitches.iter().enumerate() {
        if fam > 0 && pitches[fam] == pitches[0] {
            // One pitch: the families are the same rows.
            for slot in 0..4 {
                copy_row(b, ETA + slot, ETA + 4 * fam + slot);
                copy_row(b, ETA_MIRROR + slot, ETA_MIRROR + 4 * fam + slot);
            }
            copy_row(b, ETA_ENV, ETA_ENV + fam);
            continue;
        }
        let gr = Grating {
            inv_d: 1.0 / d,
            depth: f64::from(f.depth.max(0.0)),
            nc: f64::from(f.lacquer.clamp(1.0, 3.0)),
        };
        for (slot, l) in lights.iter().enumerate() {
            let lt = on_sheet(pad_to_dir(f64::from(l.x), f64::from(l.y), rot));
            let q = [lt[0] + vt[0], lt[1] + vt[1]];
            let ql = q[0].hypot(q[1]);
            let orders = if straight {
                nodes(gr, lt, g_line, tint)
            } else if ql > 1e-6 {
                nodes(gr, lt, [q[0] / ql, q[1] / ql], tint)
            } else {
                nodes_mean(gr, lt, tint)
            };
            let mirror = if straight {
                nodes(gr, lt, g_line, tint)
            } else {
                nodes_mean(gr, lt, tint)
            };
            put(b, ETA + 4 * fam + slot, orders);
            put(b, ETA_MIRROR + 4 * fam + slot, mirror);
        }
        // What the studio sends in mirrors the eye.
        let le = [-vt[0], -vt[1]];
        let env = if straight {
            nodes(gr, le, g_line, tint)
        } else {
            nodes_mean(gr, le, tint)
        };
        put(b, ETA_ENV + fam, env);
    }
}

/// Writes Quality's rows: Draft trims the bounded work in the kernel, High adds
/// to it. The counts the kernel loops over are read from here, so FXC keeps
/// loops rather than unrolling them.
fn put_quality(b: &mut Blocks, quality: u32) {
    let (orders, bins, picks, steps, flake, step_px, half, wide) = match quality {
        0 => (2.0, 2.0, 1.0, 12.0, 2.0, 3.0, 1.0, 1e9),
        2 => (3.0, 1.0, 2.0, 32.0, 3.0, 1.0, 1.5, 0.15),
        _ => (3.0, 1.0, 2.0, 24.0, 2.0, 1.5, 1.5, 0.25),
    };
    put(b, QUALITY, [orders, bins, picks, steps]);
    put(b, QUALITY + 1, [flake, 3.0, 0.0, step_px]);
    put(b, QUALITY + 2, [half, wide, 0.0, 0.0]);
    // Loop counts, read from here so FXC keeps the kernel's loops.
    put(b, QUALITY + 3, [4.0, 9.0, 2.0, 3.0]);
}

/// Writes the shards' rows. Sizes go in sheet millimetres, so shards keep
/// their size at any raster.
fn put_shatter(b: &mut Blocks, f: &HolographicFoil) {
    let size = f64::from(f.shard_size.max(0.05));
    put(
        b,
        SHATTER_ROW,
        [
            size,
            f64::from(f.irregularity).clamp(0.0, 1.0),
            f64::from(f.splits).clamp(0.0, 3.0),
            f64::from(f.directions.max(0)),
        ],
    );
    put(
        b,
        SHATTER_MORE,
        [
            f64::from(f.shard_tilt.clamp(0.0, 80.0)).to_radians().tan(),
            f64::from(f.chirp.max(0.0)) / 100.0,
            f64::from(f.ridge_height) * 1e-3,
            f64::from(f.ridge_width.max(1.0)) * 1e-3,
        ],
    );
    // A shard is a coherent patch: light leaving it spreads as from an
    // aperture its own size.
    let aperture = 0.44 * 0.55 / (size * 1000.0);
    put(
        b,
        SHATTER_CREASE,
        [
            f64::from(f.creased / 100.0).clamp(0.0, 1.0),
            aperture * aperture,
            0.0,
            0.0,
        ],
    );
}

/// The flakes' pitch range in nm, finest first, whichever way round the rows
/// are set.
fn pitch_range(f: &HolographicFoil) -> [f64; 2] {
    let a = f64::from(f.pitch_min.max(100.0));
    let b = f64::from(f.pitch_max.max(100.0));
    [a.min(b), a.max(b)]
}

/// Writes the flakes' and stars' rows, sizes in sheet millimetres.
fn put_glitter(b: &mut Blocks, f: &HolographicFoil) {
    let size = f64::from(f.flake_size.max(1.0)) * 1e-3;
    put(
        b,
        GLITTER_ROW,
        [
            size,
            f64::from(f.flake_shape.min(2)),
            f64::from(f.flake_layers.clamp(1, 3)),
            f64::from(f.flake_tilt.clamp(0.0, 80.0)).to_radians().tan(),
        ],
    );
    let [lo, hi] = pitch_range(f);
    // A flake is a coherent patch: light leaving it spreads as from an
    // aperture its own size.
    let aperture = 0.242 / (size * 1000.0);
    put(
        b,
        GLITTER_MORE,
        [
            f64::from(f.flake_curve.max(0.0)).to_radians(),
            1.0 / hi,
            1.0 / lo,
            aperture * aperture,
        ],
    );
    put(
        b,
        STARS,
        [
            f64::from(f.star_size.max(1.0)) * 1e-3,
            f64::from(f.star_density / 100.0).clamp(0.0, 1.0),
            0.0,
            0.0,
        ],
    );
    let c = f.base_colour.map(|x| f64::from(x.max(0.0)));
    put(b, BASE_INK, [c[0], c[1], c[2], 0.0]);
}

/// Writes the sparkle dots', specks' and sand's rows. The two dot lattices
/// together cover Dot cover of the sheet.
fn put_sparkle(b: &mut Blocks, f: &HolographicFoil, tint: &Tint) {
    let size = f64::from(f.dot_size.max(1.0)) * 1e-3;
    let irr = f64::from(f.dot_irregularity).clamp(0.0, 1.0);
    let cover = f64::from(f.dot_cover / 100.0).clamp(0.01, 0.95);
    let c1 = 1.0 - (1.0 - cover).sqrt();
    let es2 = 1.0 + (0.8 * irr) * (0.8 * irr) / 12.0;
    let elobe = 1.0 + (0.15 * irr) * (0.15 * irr) / 3.0;
    let pitch = size * (std::f64::consts::FRAC_PI_4 * es2 * elobe / c1).sqrt();
    let tan = |deg: f32| f64::from(deg.clamp(0.0, 80.0)).to_radians().tan();
    put(b, SPARKLE_ROW, [size, cover, irr, tan(f.dot_tilt)]);
    put(
        b,
        SPARKLE_MORE,
        [
            f64::from(f.dot_dome.max(0.0)).to_radians(),
            f64::from(f.speck_share / 100.0).clamp(0.0, 1.0),
            0.9,
            pitch,
        ],
    );
    let matte = f64::from(f.base_matte.max(0.0));
    let tilt = f64::from(f.sand_tilt.max(0.0)).min(matte);
    let own = (matte * matte - tilt * tilt).sqrt().max(0.25).to_radians();
    put(
        b,
        SAND,
        [
            f64::from(f.sand_size.max(0.01)) * size,
            tilt.to_radians().tan(),
            own,
            0.0,
        ],
    );
    let dye = speck_tint(
        *tint,
        f64::from(f.speck_peak),
        f64::from(f.speck_width.max(1.0)),
        f64::from(f.speck_floor / 100.0).clamp(0.0, 1.0),
    );
    put(b, SPECK_TINT, [dye[0], dye[1], dye[2], 0.0]);
}

/// The Tint choice that picks the dye for the Material.
const AUTO_TINT: u32 = 4;

/// The dye the Tint choice names. Auto is clear on Prism, gold on Shatter,
/// blue on Glitter, sky blue on Sparkle and a paler gold on Vortex.
fn tint_of(f: &HolographicFoil) -> Tint {
    let dye = |kind, edge, width, floor| Tint {
        kind,
        edge,
        width,
        floor,
    };
    let choice = if f.tint == AUTO_TINT {
        match f.material {
            SHATTER => 1,
            GLITTER => 2,
            SPARKLE => return dye(2, 600.0, 90.0, 0.5),
            VORTEX => return dye(1, 500.0, 50.0, 0.3),
            _ => 0,
        }
    } else {
        f.tint
    };
    match choice {
        1 => dye(1, 500.0, 70.0, 0.25),
        2 => dye(2, 525.0, 30.0, 0.15),
        3 => Tint {
            kind: 1 + f.tint_pass.min(1),
            edge: f64::from(f.tint_edge),
            width: f64::from(f.tint_width.max(1.0)),
            floor: f64::from(f.tint_floor / 100.0).clamp(0.0, 1.0),
        },
        _ => Tint {
            kind: 0,
            edge: 550.0,
            width: 50.0,
            floor: 1.0,
        },
    }
}

/// Writes light `slot` and returns its irradiance at the sheet's centre, so
/// the brightest can steer the pixel footprint, and on the sheet's back.
fn put_light(
    b: &mut Blocks,
    slot: usize,
    l: &Light,
    rot: f64,
    n: [f64; 3],
    tint: &Tint,
    quality: u32,
) -> [f64; 2] {
    let is_panel = slot == 3;
    let row = LIGHTS + 4 * slot;
    let rad = if l.on {
        f64::from(l.intensity.max(0.0))
    } else {
        0.0
    };
    let dist = f64::from(l.distance.max(0.05));
    let dir = pad_to_dir(f64::from(l.x), f64::from(l.y), rot);
    let pos = scale(dir, dist);
    let spectrum = (l.spectrum as usize).min(SPECTRA - 1);
    let colour = projection(spectrum, *tint).light;
    put(b, row, [pos[0], pos[1], pos[2], rad]);
    put(
        b,
        row + 1,
        [colour[0], colour[1], colour[2], spectrum as f64],
    );
    let cos = [dot(dir, n).max(0.0), (-dot(dir, n)).max(0.0)];
    if is_panel {
        let (er, et) = radial_axes(dir);
        let width = f64::from(l.size.clamp(0.1, 170.0)).to_radians();
        let length = f64::from(l.length.clamp(0.1, 170.0)).to_radians();
        let hu = dist * (width / 2.0).tan();
        let hw = dist * (length / 2.0).tan();
        let u = scale(er, hu);
        let v = scale(et, hw);
        // Cells about 8 degrees long and 12 across, so the rainbow keeps the
        // panel's edges: at most nine in all, six on Draft, twelve on High.
        let (most, along_min, along_max, cells) = match quality {
            0 => (2.0, 2.0, 3.0, 6.0),
            2 => (4.0, 3.0, 8.0, 12.0),
            _ => (3.0, 3.0, 6.0, 9.0),
        };
        let across = (f64::from(l.size) / 12.0).round().clamp(1.0, most);
        let along = (f64::from(l.length) / 8.0)
            .round()
            .clamp(along_min, along_max)
            .min((cells / across).floor());
        put(b, row + 2, [u[0], u[1], u[2], across]);
        put(b, row + 3, [v[0], v[1], v[2], along]);
        cos.map(|c| rad * 4.0 * hu * hw / (dist * dist) * c)
    } else {
        let half = f64::from(l.size.clamp(0.01, 90.0)).to_radians() / 2.0;
        let radius = dist * half.tan();
        put(b, row + 2, [radius, 0.0, 0.0, 0.0]);
        let sa = half.tan().min(0.999);
        cos.map(|c| rad * std::f64::consts::PI * sa * sa * c)
    }
}

/// The studio's panels turned by `rot`: centre, level and half sizes in
/// radians.
fn studio_panels(levels: [f32; 4], rot: f64) -> impl Iterator<Item = ([f64; 3], f64, f64, f64)> {
    STUDIO.iter().map(move |&(theta, phi, wr, wt, level)| {
        let (st, ct) = theta.to_radians().sin_cos();
        let (sp, cp) = (phi.to_radians() + rot).sin_cos();
        let lv = f64::from(levels.get(level).copied().unwrap_or(0.0).max(0.0));
        (
            [st * cp, st * sp, ct],
            lv,
            wr.to_radians() / 2.0,
            wt.to_radians() / 2.0,
        )
    })
}

/// The light the studio sheds on a surface facing `n`.
fn studio_light(levels: [f32; 4], rot: f64, n: [f64; 3]) -> f64 {
    let room = f64::from(levels[3].max(0.0));
    // The room fills the half of the sky in front of the viewer.
    let e = room * std::f64::consts::PI * 0.5 * (1.0 + n[2]);
    e + studio_panels(levels, rot)
        .map(|(c, lv, hr, ht)| lv * 4.0 * hr * ht * dot(c, n).max(0.0))
        .sum::<f64>()
}

/// Writes the studio's panels and returns the light they shed on the sheet,
/// for a printed surface.
fn put_studio(b: &mut Blocks, levels: [f32; 4], rot: f64, n: [f64; 3]) -> f64 {
    for (i, (c, lv, hr, ht)) in studio_panels(levels, rot).enumerate() {
        let (er, et) = radial_axes(c);
        let row = STUDIO_AT + 3 * i;
        put(b, row, [c[0], c[1], c[2], lv]);
        put(b, row + 1, [er[0], er[1], er[2], hr]);
        put(b, row + 2, [et[0], et[1], et[2], ht]);
    }
    studio_light(levels, rot, n)
}

#[cfg(test)]
mod tests;
