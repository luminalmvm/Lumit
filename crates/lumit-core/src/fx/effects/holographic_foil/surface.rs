//! Holographic foil's surface rows: the relief, the sheet's broad waves, the
//! varnish's ripple, embossed lines, bow, wear and the maps' settings.
//!
//! | Row | Holds |
//! |---|---|
//! | 192 | relief kind (7 none), slope per unit of height gradient, tile width in mm, on varnish |
//! | 193 | Height map repeats, wear share, seed (below 2 to the 24), Height map wrap |
//! | 194 | waves on the board, on the varnish, embossed lines, lines on varnish |
//! | 195 | bow in mm, bow axis, half sheet width and height in mm |
//! | 196 | direction map reading, 1 / pitch at white in 1/nm, environment amount, spin in radians |
//! | 197..203 | three smudges, two rows each: centre in mm and the inverse spread xx, xy; yy |
//! | 206..238 | up to 32 waves: frequency in cycles per mm, amplitude in mm, phase |
//! | 279 | bevel width in mm (0 off), height in mm, profile, softness share |
//!
//! The bevel's blur (0.45 of its reach, wider with softness) is held to a
//! quarter of the sheet's longer side, which the kernel's halved copies
//! always reach, so a wider wall is narrowed to it: the same share of the
//! sheet at every raster size.

use super::HolographicFoil;

const RELIEF: usize = 192;
const RELIEF_MORE: usize = 193;
const WAVE_HEAD: usize = 194;
const BOW: usize = 195;
const MAPS: usize = 196;
const SMUDGES: usize = 197;
const WAVES: usize = 206;
const WAVES_MAX: usize = 32;
const BEVEL: usize = 279;

/// The Relief choice that builds a bevel from the sheet's alpha.
pub(super) const BEVEL_SOURCE: u32 = 8;

/// The bevel profile whose cushion reaches twice the width.
const DOME: u32 = 2;

/// Holospot's height of a white relief, per millimetre of tile.
const RELIEF_HEIGHT: f64 = 0.0028;

/// A small seeded stream (splitmix64, Sebastiano Vigna, public domain), one
/// per use, so a row never shifts another's numbers.
struct Stream(u64);

impl Stream {
    fn new(seed: u32, use_: u64) -> Self {
        Self(u64::from(seed) ^ use_.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One sine wave: frequency in cycles per mm, amplitude in mm, phase in
/// cycles.
type Wave = [f64; 4];

/// `n` waves with wavelengths spread evenly in log between `lo` and `hi` mm,
/// directions spread over a half turn, amplitude growing as wavelength to the
/// power 1.5, scaled so `rms` holds: RMS slope in radians for `slope`, or RMS
/// height in mm otherwise.
fn waves(n: usize, lo: f64, hi: f64, rms: f64, slope: bool, s: &mut Stream) -> Vec<Wave> {
    let turn = s.next();
    let raw: Vec<(f64, f64, f64)> = (0..n)
        .map(|k| {
            let lam = lo * (hi / lo).powf((k as f64 + s.next()) / n as f64);
            let dir = std::f64::consts::PI * ((k as f64 * 0.618_034 + turn) % 1.0);
            (lam, dir, s.next())
        })
        .collect();
    let c = if slope {
        let sum: f64 = raw.iter().map(|w| w.0).sum();
        rms / (2.0 * std::f64::consts::PI.powi(2) * sum).sqrt()
    } else {
        let sum: f64 = raw.iter().map(|w| w.0.powi(3)).sum();
        rms * (2.0 / sum).sqrt()
    };
    raw.into_iter()
        .map(|(lam, dir, phase)| {
            let (sd, cd) = dir.sin_cos();
            [cd / lam, sd / lam, c * lam.powf(1.5), phase]
        })
        .collect()
}

/// Writes the surface rows of `f` on a sheet `wmm` by `hmm` millimetres.
pub(super) fn put(f: &HolographicFoil, wmm: f64, hmm: f64, put: &mut dyn FnMut(usize, [f64; 4])) {
    let strength = f64::from(f.relief.max(0.0));
    let sign = if f.relief_invert { -1.0 } else { 1.0 };
    let tile = f64::from(f.sheet_width.max(1.0)) / f64::from(f.relief_scale.max(0.1));
    put(
        RELIEF,
        [
            f64::from(f.relief_source.min(7)),
            sign * RELIEF_HEIGHT * strength,
            tile,
            f64::from(f.relief_on.min(1)),
        ],
    );
    if f.relief_source == BEVEL_SOURCE {
        let profile = f.bevel_profile.min(3);
        let softness = f64::from(f.bevel_softness / 100.0).clamp(0.0, 1.0);
        let reach = if profile == DOME { 2.0 } else { 1.0 };
        let widest = 0.25 * wmm.max(hmm) / (0.45 * (1.0 + softness) * reach);
        put(
            BEVEL,
            [
                f64::from(f.bevel_width.max(0.02)).min(widest),
                f64::from(f.bevel_height.max(0.0)) * 1e-3,
                f64::from(profile),
                softness,
            ],
        );
    }
    let wear = f64::from(f.wear / 100.0).clamp(0.0, 1.0);
    put(
        RELIEF_MORE,
        [
            f64::from(f.height_map_scale.max(1.0)),
            wear,
            f64::from(f.seed & 0x00ff_ffff),
            f64::from(f.height_map_wrap.min(2)),
        ],
    );
    put(
        BOW,
        [
            f64::from(f.bow),
            f64::from(f.bow_axis.min(1)),
            0.5 * wmm,
            0.5 * hmm,
        ],
    );
    put(
        MAPS,
        [
            f64::from(f.direction_mode.min(1)),
            1.0 / f64::from(f.pitch_white.max(100.0)),
            f64::from(f.environment_amount.max(0.0)) / 100.0,
            f64::from(f.spin).to_radians(),
        ],
    );

    let mut all: Vec<Wave> = Vec::with_capacity(WAVES_MAX);
    let tilt = f64::from(f.waviness.max(0.0)).to_radians();
    let board = if tilt > 0.0 {
        waves(8, 8.0, 40.0, tilt, true, &mut Stream::new(f.seed, 1))
    } else {
        Vec::new()
    };
    let peel_mm = f64::from(f.orange_peel.max(0.0)) * 1e-3;
    let coat = if peel_mm > 0.0 {
        waves(16, 0.5, 5.0, peel_mm, false, &mut Stream::new(f.seed, 2))
    } else {
        Vec::new()
    };
    let pitch = f64::from(f.line_pitch.max(0.01));
    let amp = 0.5 * f64::from(f.line_height.max(0.0)) * 1e-3;
    let lines: Vec<Wave> = match f.lines {
        _ if amp <= 0.0 => Vec::new(),
        1 => vec![[0.0, 1.0 / pitch, amp, 0.0]],
        2 => {
            let k = std::f64::consts::FRAC_1_SQRT_2 / pitch;
            vec![[k, k, amp, 0.0], [-k, k, amp, 0.25]]
        }
        _ => Vec::new(),
    };
    let counts = [board.len(), coat.len(), lines.len()];
    all.extend(board);
    all.extend(coat);
    all.extend(lines);
    put(
        WAVE_HEAD,
        [
            counts[0] as f64,
            counts[1] as f64,
            counts[2] as f64,
            f64::from(f.relief_on.min(1)),
        ],
    );
    for (k, w) in all.iter().take(WAVES_MAX).enumerate() {
        put(WAVES + k, *w);
    }

    // Wear: three smudges.
    let mut s = Stream::new(f.seed, 11);
    for k in 0..3 {
        let cx = (s.next() - 0.5) * 0.8 * wmm;
        let cy = (s.next() - 0.5) * 0.8 * hmm;
        let major = 3.0 + 4.0 * s.next();
        let minor = major * (0.45 + 0.4 * s.next());
        let (sa, ca) = (std::f64::consts::PI * s.next()).sin_cos();
        let (a, b) = (1.0 / (major * major), 1.0 / (minor * minor));
        let xx = a * ca * ca + b * sa * sa;
        let xy = (a - b) * ca * sa;
        let yy = a * sa * sa + b * ca * ca;
        put(SMUDGES + 2 * k, [cx, cy, xx, xy]);
        put(SMUDGES + 2 * k + 1, [yy, 0.0, 0.0, 0.0]);
    }
}
