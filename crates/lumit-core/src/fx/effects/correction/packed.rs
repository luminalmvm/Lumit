//! The Correction's rows turned into the numbers both render paths read.
//!
//! **In plain terms.** The rows say what the user asked for; this works out
//! the arithmetic once a frame: the gains, the exponents, the curve tables and
//! the window outlines. The CPU twin reads [`Packed`] directly, and the GPU
//! reads the same struct written out by [`Packed::words`] and [`Packed::data`],
//! so neither path redoes any of it.

use super::hdr::{Hdr, HDR_WORDS};
use super::Correction;
use crate::fx::cpu;
use crate::fx::params::CurvePoints;
use crate::fx::shape_points::{ShapePoints, SHAPE_MAX_POINTS};

/// Entries in one curve table.
pub const TABLE: usize = cpu::CURVE_TABLE;

/// Curve tables in [`Packed::data`]: Luma, Red, Green, Blue, then Hue v hue,
/// Hue v sat, Hue v lum, Lum v sat, Sat v sat and Sat v lum.
pub const TABLES: usize = 10;

/// Where the window outlines start in [`Packed::data`], as `x, y` pairs.
pub const POLY_AT: usize = TABLES * TABLE;

/// Lines a smooth polygon's span is cut into.
pub const SMOOTH_STEPS: usize = 4;

/// The most outline points one window can hold once cut into lines.
pub const POLY_MAX: usize = SHAPE_MAX_POINTS * SMOOTH_STEPS;

/// Midtone detail's blur, px at comp size.
pub const DETAIL_PX: f32 = 20.0;

/// Denoise's blur at 100, px at comp size.
pub const DENOISE_PX: f32 = 5.0;

/// The hue curves' rest: the flat line through the middle.
pub const HUE_FLAT: [[f32; 2]; 2] = [[0.0, 0.5], [1.0, 0.5]];

/// The qualifier, ready to key with.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Qualifier {
    /// 0 off, 1 HSL, 2 RGB, 3 Luma, 4 3D.
    pub mode: u32,
    /// The hue centre, in turns from red.
    pub hue: f32,
    /// The half widths below and above the centre, and the softness, turns.
    pub hue_lo: f32,
    pub hue_hi: f32,
    pub hue_soft: f32,
    pub use_hue: bool,
    pub use_sat: bool,
    pub use_lum: bool,
    /// Low, high and softness, 0..1.
    pub sat: [f32; 3],
    pub lum: [f32; 3],
    pub rgb_lo: [f32; 3],
    pub rgb_hi: [f32; 3],
    pub rgb_soft: f32,
    /// The key's new black and white, from Clean black and Clean white.
    pub clean: [f32; 2],
    /// Black clip and white clip, 0..1.
    pub clip: [f32; 2],
    pub invert: bool,
    /// The blurs and the grow, raster pixels.
    pub denoise_px: f32,
    pub blur_px: f32,
    pub grow_px: f32,
    /// The 3D key's samples as `Y, Cb, Cr`, the first `samples[0]` kept and
    /// the first `samples[1]` dropped.
    pub keep_at: [[f32; 3]; SHAPE_MAX_POINTS],
    pub drop_at: [[f32; 3]; SHAPE_MAX_POINTS],
    pub samples: [u32; 2],
    /// How far round a sample the 3D key reaches, and its softness, in
    /// `Y, Cb, Cr` distance with luma counted at half.
    pub reach: [f32; 2],
    /// Where the samples start in [`Packed::data`], kept then dropped.
    pub samples_at: u32,
}

/// One window, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Window {
    /// 0 off, 1 ellipse, 2 rectangle, 3 polygon, 4 gradient. A polygon of
    /// fewer than three points is 0.
    pub shape: u32,
    /// Raster pixels.
    pub centre: [f32; 2],
    pub half: [f32; 2],
    /// The angle's cosine and sine.
    pub rot: [f32; 2],
    /// The rectangle's corner radius, raster pixels.
    pub round: f32,
    pub soft_out: f32,
    pub soft_in: f32,
    /// 0..1.
    pub opacity: f32,
    pub invert: bool,
    pub subtract: bool,
    /// The outline's first point and how many, in the pairs after
    /// [`POLY_AT`].
    pub poly: [u32; 2],
    /// The outline's bounds in the box, raster pixels: min x, min y, max x,
    /// max y.
    pub bbox: [f32; 4],
}

/// Everything one Correction draws with.
#[derive(Debug, Clone, PartialEq)]
pub struct Packed {
    /// 0 Video, 1 Log, 2 Linear.
    pub space: u32,
    /// Every grade row at rest, or Mix 0: the picture goes through untouched.
    pub neutral: bool,
    pub mix: f32,
    /// `2^exposure` times the Temperature gains, applied in scene linear.
    pub pre_gain: [f32; 3],
    pub offset: [f32; 3],
    pub lift: [f32; 3],
    /// The exponent `2^(−2·gamma)`.
    pub gamma: [f32; 3],
    pub gain: [f32; 3],
    pub shadow: [f32; 3],
    pub midtone: [f32; 3],
    pub highlight: [f32; 3],
    /// Low range and high range.
    pub range: [f32; 2],
    pub lum_mix: f32,
    pub contrast: f32,
    pub pivot: f32,
    pub sat: f32,
    /// Hue's cosine and sine.
    pub hue: [f32; 2],
    pub boost: f32,
    /// Shadows and highlights, as the lift each gives at full weight.
    pub tonal: [f32; 2],
    pub detail: f32,
    /// Midtone detail's blur, raster pixels.
    pub detail_px: f32,
    /// Rows are the outputs.
    pub mixer: [[f32; 3]; 3],
    pub mixer_on: bool,
    pub keep_luma: bool,
    pub curves_on: bool,
    /// The soft clip's low and high knees; 0 and 1 are off.
    pub knees: [f32; 2],
    pub hue_curves_on: bool,
    pub qual: Qualifier,
    pub windows: [Window; 4],
    pub hdr: Hdr,
    /// The [`TABLES`] curve tables, then the window outlines.
    pub data: Vec<f32>,
}

/// How many `vec4`s [`Packed::words`] writes: the WGSL `Params` struct.
pub const WORDS: usize = 24 + 5 * 4 + HDR_WORDS + 2;

impl Packed {
    /// Whether the qualifier keys anything.
    #[must_use]
    pub fn qualifier_on(&self) -> bool {
        self.qual.mode != 0
    }

    /// Whether any window is on.
    #[must_use]
    pub fn windows_on(&self) -> bool {
        self.windows.iter().any(|w| w.shape != 0)
    }

    /// Only what a colour does to itself, for a LUT. The qualifier stays, as
    /// it keys on the pixel's own colour, but its denoise, grow and blur look
    /// at the neighbours and go. Windows and Midtone detail go too, so the
    /// node grades as if every window keyed it whole.
    pub fn colour_only(&mut self) {
        for w in &mut self.windows {
            w.shape = 0;
        }
        self.qual.denoise_px = 0.0;
        self.qual.grow_px = 0.0;
        self.qual.blur_px = 0.0;
        self.detail = 0.0;
    }

    /// The uniform `fx_correction.wgsl` reads, in its `Params` order. `matte`
    /// says whether a matte is bound, which only the caller knows.
    #[must_use]
    pub fn words(&self, matte: bool) -> [[f32; 4]; WORDS] {
        let b = f32::from;
        let v3 = |v: [f32; 3], w: f32| [v[0], v[1], v[2], w];
        let q = &self.qual;
        let key_on = self.qualifier_on() || self.windows_on() || matte;
        let denoised = self.qualifier_on() && q.denoise_px > 0.0;
        let mut out = [[0.0f32; 4]; WORDS];
        let head = [
            [self.space as f32, self.mix, self.lum_mix, b(key_on)],
            v3(self.pre_gain, b(self.detail != 0.0)),
            v3(self.offset, 0.0),
            v3(self.lift, 0.0),
            v3(self.gamma, 0.0),
            v3(self.gain, 0.0),
            v3(self.shadow, 0.0),
            v3(self.midtone, 0.0),
            v3(self.highlight, 0.0),
            [self.range[0], self.range[1], self.contrast, self.pivot],
            [self.sat, self.hue[0], self.hue[1], self.boost],
            [self.tonal[0], self.tonal[1], self.detail, self.knees[0]],
            [
                self.knees[1],
                b(self.curves_on),
                b(self.hue_curves_on),
                b(self.mixer_on),
            ],
            v3(self.mixer[0], b(self.keep_luma)),
            v3(self.mixer[1], 0.0),
            v3(self.mixer[2], 0.0),
            [q.mode as f32, q.hue, q.hue_lo, q.hue_hi],
            [q.hue_soft, b(q.use_hue), b(q.use_sat), b(q.use_lum)],
            v3(q.sat, b(denoised)),
            v3(q.lum, b(q.invert)),
            v3(q.rgb_lo, q.rgb_soft),
            v3(q.rgb_hi, 0.0),
            [q.clean[0], q.clean[1], q.clip[0], q.clip[1]],
            [b(self.qualifier_on()), b(matte), b(self.windows_on()), 0.0],
        ];
        out[58] = [
            q.samples_at as f32,
            q.samples[0] as f32,
            q.samples[1] as f32,
            0.0,
        ];
        out[59] = [q.reach[0], q.reach[1], 0.0, 0.0];
        out[..24].copy_from_slice(&head);
        for (i, w) in self.windows.iter().enumerate() {
            let flags = b(w.invert) + 2.0 * b(w.subtract);
            let at = 24 + 5 * i;
            out[at] = [w.shape as f32, w.centre[0], w.centre[1], w.opacity];
            out[at + 1] = [w.half[0], w.half[1], w.rot[0], w.rot[1]];
            out[at + 2] = [w.round, w.soft_out, w.soft_in, flags];
            out[at + 3] = w.bbox;
            out[at + 4] = [w.poly[0] as f32, w.poly[1] as f32, 0.0, 0.0];
        }
        out[44..58].copy_from_slice(&self.hdr.words());
        out
    }
}

/// How far apart two 3D samples are at Size 100.
pub const REACH: f32 = 0.25;

/// A 3D key's samples from its two Shape rows: Cb and Cr doubled in the
/// first, luma first in the second.
fn samples(chroma: &ShapePoints, luma: &ShapePoints) -> [[f32; 3]; SHAPE_MAX_POINTS] {
    let mut out = [[0.0; 3]; SHAPE_MAX_POINTS];
    for (i, (slot, c)) in out.iter_mut().zip(chroma.points()).enumerate() {
        let y = luma.points().get(i).map_or(0.0, |p| p[0]);
        *slot = [y, 0.5 * c[0], 0.5 * c[1]];
    }
    out
}

/// Hue curves bake round the circle: the point at hue 1 is the point at hue
/// 0, and the line through the last point carries on into the first.
fn periodic_table(points: &CurvePoints) -> [f32; TABLE] {
    let p = points.points();
    let mut xs: Vec<(f64, f64)> = Vec::with_capacity(p.len() + 1);
    for xy in p {
        let x = f64::from(xy[0]);
        let y = f64::from(xy[1]);
        if x < 1.0 {
            xs.push((x, y));
        } else if !xs.iter().any(|(x0, _)| *x0 == 0.0) && !p.iter().any(|q| q[0] == 0.0) {
            xs.insert(0, (0.0, y));
        }
    }
    let n = xs.len();
    if n < 2 {
        let y = xs.first().map_or(0.5, |p| p.1) as f32;
        return [y.clamp(0.0, 1.0); TABLE];
    }
    // Two points either side, a turn away, so every span has neighbours.
    let at = |k: isize| -> (f64, f64) {
        let len = n as isize;
        let wraps = k.div_euclid(len);
        let (x, y) = xs[k.rem_euclid(len) as usize];
        (x + wraps as f64, y)
    };
    let slope = |k: isize| {
        let (x0, y0) = at(k - 1);
        let (x1, y1) = at(k + 1);
        if x1 > x0 {
            (y1 - y0) / (x1 - x0)
        } else {
            0.0
        }
    };
    let mut out = [0.0f32; TABLE];
    let mut k: isize = -1;
    for (i, slot) in out.iter_mut().enumerate().take(TABLE - 1) {
        let x = i as f64 / (TABLE - 1) as f64;
        while at(k + 1).0 <= x {
            k += 1;
        }
        let (x0, y0) = at(k);
        let (x1, y1) = at(k + 1);
        let span = x1 - x0;
        let t = if span > 0.0 { (x - x0) / span } else { 0.0 };
        let (t2, t3) = (t * t, t * t * t);
        let y = y0 * (2.0 * t3 - 3.0 * t2 + 1.0)
            + slope(k) * span * (t3 - 2.0 * t2 + t)
            + y1 * (-2.0 * t3 + 3.0 * t2)
            + slope(k + 1) * span * (t3 - t2);
        #[allow(clippy::cast_possible_truncation)]
        {
            *slot = y.clamp(0.0, 1.0) as f32;
        }
    }
    out[TABLE - 1] = out[0];
    out
}

/// A curve that leaves everything alone: every point on the middle line.
fn flat(points: &CurvePoints) -> bool {
    points.points().iter().all(|p| p[1] == 0.5)
}

/// The outline of a polygon window in the box, raster pixels: the points
/// themselves, or a closed Catmull-Rom curve through them cut into lines.
fn outline(points: &ShapePoints, half: [f32; 2], smooth: bool) -> Vec<[f32; 2]> {
    let p: Vec<[f64; 2]> = points
        .points()
        .iter()
        .map(|q| {
            [
                f64::from(q[0]) * f64::from(half[0]),
                f64::from(q[1]) * f64::from(half[1]),
            ]
        })
        .collect();
    let n = p.len();
    #[allow(clippy::cast_possible_truncation)]
    let f = |v: [f64; 2]| [v[0] as f32, v[1] as f32];
    if !smooth {
        return p.into_iter().map(f).collect();
    }
    let mut out = Vec::with_capacity(n * SMOOTH_STEPS);
    for i in 0..n {
        let [p0, p1, p2, p3] = [n - 1, 0, 1, 2].map(|o| p[(i + o) % n]);
        for s in 0..SMOOTH_STEPS {
            let t = s as f64 / SMOOTH_STEPS as f64;
            let (t2, t3) = (t * t, t * t * t);
            let c = |a: usize| {
                0.5 * (2.0 * p1[a]
                    + (p2[a] - p0[a]) * t
                    + (2.0 * p0[a] - 5.0 * p1[a] + 4.0 * p2[a] - p3[a]) * t2
                    + (3.0 * p1[a] - p0[a] - 3.0 * p2[a] + p3[a]) * t3)
            };
            out.push(f([c(0), c(1)]));
        }
    }
    out
}

impl Correction {
    /// The numbers both render paths draw with. `px_scale` is raster pixels
    /// per comp pixel, for the blurs whose size is fixed at comp size.
    #[must_use]
    pub fn packed(&self, px_scale: f32) -> Packed {
        let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
        let (gain_r, gain_b) = cpu::temperature_gains((self.temperature / 100.0).clamp(-2.0, 2.0));
        let gain_g = cpu::tint_gain((self.tint / 100.0).clamp(-2.0, 2.0));
        let expo = self.exposure.clamp(-20.0, 20.0).exp2();
        let pre_gain = [gain_r * expo, gain_g * expo, gain_b * expo];
        let hue = self.hue.to_radians();
        let mixer = [
            rgb(self.mixer_red),
            rgb(self.mixer_green),
            rgb(self.mixer_blue),
        ];
        let mixer_on = mixer != [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let soft = |s: f32| 0.5 * (s / 100.0).clamp(0.0, 1.0);

        let tone = [self.curve_y, self.curve_r, self.curve_g, self.curve_b];
        let hues = [self.hue_hue, self.hue_sat, self.hue_lum];
        let others = [self.lum_sat, self.sat_sat, self.sat_lum];
        let curves_on = tone.iter().any(|c| *c != CurvePoints::IDENTITY);
        let hue_curves_on = !hues.iter().chain(&others).all(flat);
        let mut data = vec![0.0f32; POLY_AT];
        let mut put = |t: usize, table: &[f32; TABLE]| {
            if let Some(slot) = data.get_mut(t * TABLE..(t + 1) * TABLE) {
                slot.copy_from_slice(table);
            }
        };
        for (t, c) in tone.iter().enumerate() {
            put(t, &cpu::curve_table(c));
        }
        for (t, c) in hues.iter().enumerate() {
            put(
                4 + t,
                &if flat(c) {
                    [0.5; TABLE]
                } else {
                    periodic_table(c)
                },
            );
        }
        for (t, c) in others.iter().enumerate() {
            put(
                7 + t,
                &if flat(c) {
                    [0.5; TABLE]
                } else {
                    cpu::curve_table(c)
                },
            );
        }

        let slots = [
            (
                self.win1_shape,
                self.win1_x,
                self.win1_y,
                self.win1_w,
                self.win1_h,
            ),
            (
                self.win2_shape,
                self.win2_x,
                self.win2_y,
                self.win2_w,
                self.win2_h,
            ),
            (
                self.win3_shape,
                self.win3_x,
                self.win3_y,
                self.win3_w,
                self.win3_h,
            ),
            (
                self.win4_shape,
                self.win4_x,
                self.win4_y,
                self.win4_w,
                self.win4_h,
            ),
        ];
        let rest = [
            (
                self.win1_angle,
                self.win1_round,
                self.win1_soft,
                self.win1_soft_in,
            ),
            (
                self.win2_angle,
                self.win2_round,
                self.win2_soft,
                self.win2_soft_in,
            ),
            (
                self.win3_angle,
                self.win3_round,
                self.win3_soft,
                self.win3_soft_in,
            ),
            (
                self.win4_angle,
                self.win4_round,
                self.win4_soft,
                self.win4_soft_in,
            ),
        ];
        let looks = [
            (self.win1_opacity, self.win1_invert, self.win1_subtract),
            (self.win2_opacity, self.win2_invert, self.win2_subtract),
            (self.win3_opacity, self.win3_invert, self.win3_subtract),
            (self.win4_opacity, self.win4_invert, self.win4_subtract),
        ];
        let outlines = [
            (self.win1_points, self.win1_smooth),
            (self.win2_points, self.win2_smooth),
            (self.win3_points, self.win3_smooth),
            (self.win4_points, self.win4_smooth),
        ];
        let mut windows = [Window::default(); 4];
        for (i, w) in windows.iter_mut().enumerate() {
            let (shape, x, y, ww, wh) = slots[i];
            let (angle, round, soft_out, soft_in) = rest[i];
            let (opacity, invert, subtract) = looks[i];
            let half = [ww.max(0.0) * 0.5, wh.max(0.0) * 0.5];
            let a = angle.to_radians();
            *w = Window {
                shape: if shape <= 4 { shape } else { 0 },
                centre: [x, y],
                half,
                rot: [a.cos(), a.sin()],
                round: (round / 100.0).clamp(0.0, 1.0) * half[0].min(half[1]),
                soft_out: soft_out.max(0.0),
                soft_in: soft_in.max(0.0),
                opacity: (opacity / 100.0).clamp(0.0, 1.0),
                invert,
                subtract,
                poly: [0, 0],
                bbox: [0.0; 4],
            };
            if w.shape == 3 {
                let (points, smooth) = outlines[i];
                if points.len() < 3 {
                    w.shape = 0;
                    continue;
                }
                let line = outline(&points, half, smooth);
                let start = (data.len() - POLY_AT) / 2;
                let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for v in &line {
                    data.extend_from_slice(v);
                    bbox = [
                        bbox[0].min(v[0]),
                        bbox[1].min(v[1]),
                        bbox[2].max(v[0]),
                        bbox[3].max(v[1]),
                    ];
                }
                w.poly = [start as u32, line.len() as u32];
                w.bbox = bbox;
            }
        }

        let q = Qualifier {
            mode: if self.qual_mode <= 4 {
                self.qual_mode
            } else {
                0
            },
            hue: {
                let t = self.qual_hue / 360.0;
                t - t.floor()
            },
            hue_lo: (self.qual_hue_width / 720.0).max(0.0) * (1.0 - self.qual_hue_sym / 100.0),
            hue_hi: (self.qual_hue_width / 720.0).max(0.0) * (1.0 + self.qual_hue_sym / 100.0),
            hue_soft: (self.qual_hue_soft / 360.0).max(0.0),
            use_hue: self.qual_use_hue,
            use_sat: self.qual_use_sat,
            use_lum: self.qual_use_lum,
            sat: [self.qual_sat_low, self.qual_sat_high, self.qual_sat_soft].map(|v| v / 100.0),
            lum: [self.qual_lum_low, self.qual_lum_high, self.qual_lum_soft].map(|v| v / 100.0),
            rgb_lo: [self.qual_red_low, self.qual_green_low, self.qual_blue_low].map(|v| v / 100.0),
            rgb_hi: [
                self.qual_red_high,
                self.qual_green_high,
                self.qual_blue_high,
            ]
            .map(|v| v / 100.0),
            rgb_soft: self.qual_rgb_soft / 100.0,
            clean: [
                0.49 * (self.qual_clean_black / 100.0).clamp(0.0, 1.0),
                1.0 - 0.49 * (self.qual_clean_white / 100.0).clamp(0.0, 1.0),
            ],
            clip: [self.qual_black_clip / 100.0, self.qual_white_clip / 100.0],
            invert: self.qual_invert,
            denoise_px: (self.qual_denoise / 100.0).clamp(0.0, 1.0) * DENOISE_PX * px_scale,
            blur_px: self.qual_blur.max(0.0),
            grow_px: self.qual_grow,
            keep_at: samples(&self.qual_3d_keep, &self.qual_3d_keep_luma),
            drop_at: samples(&self.qual_3d_drop, &self.qual_3d_drop_luma),
            samples: [self.qual_3d_keep.len(), self.qual_3d_drop.len()].map(|n| n as u32),
            reach: [self.qual_3d_size, self.qual_3d_soft].map(|v| REACH * v.max(0.0) / 100.0),
            samples_at: data.len() as u32,
        };
        for (at, n) in [(&q.keep_at, q.samples[0]), (&q.drop_at, q.samples[1])] {
            for s in at.iter().take(n as usize) {
                data.extend_from_slice(s);
            }
        }

        let mut pk = Packed {
            space: if self.space <= 2 { self.space } else { 0 },
            neutral: false,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
            pre_gain,
            offset: rgb(self.offset),
            lift: rgb(self.lift),
            gamma: rgb(self.gamma).map(|g| (-2.0 * g.clamp(-4.0, 4.0)).exp2()),
            gain: rgb(self.gain),
            shadow: rgb(self.shadow),
            midtone: rgb(self.midtone),
            highlight: rgb(self.highlight),
            range: [self.low_range, self.high_range],
            lum_mix: (self.lum_mix / 100.0).clamp(0.0, 1.0),
            contrast: self.contrast.max(0.0) / 100.0,
            pivot: self.pivot,
            sat: self.saturation.max(0.0) / 100.0,
            hue: [hue.cos(), hue.sin()],
            boost: (self.colour_boost / 100.0).clamp(-1.0, 1.0),
            tonal: [self.shadows, self.highlights].map(|v| 0.25 * v / 100.0),
            detail: self.midtone_detail / 100.0,
            detail_px: DETAIL_PX * px_scale,
            mixer,
            mixer_on,
            keep_luma: self.mixer_keep_luma,
            curves_on,
            knees: [soft(self.soft_low), 1.0 - soft(self.soft_high)],
            hue_curves_on,
            qual: q,
            windows,
            hdr: Hdr::of(self),
            data,
        };
        pk.neutral = pk.mix == 0.0 || pk.at_rest();
        pk
    }
}

impl Packed {
    /// Whether every grade row is where it leaves the picture alone. The key
    /// does not count: a key over no grade grades nothing.
    fn at_rest(&self) -> bool {
        self.pre_gain == [1.0; 3]
            && self.offset == [0.0; 3]
            && self.lift == [0.0; 3]
            && self.gamma == [1.0; 3]
            && self.gain == [1.0; 3]
            && self.shadow == [0.0; 3]
            && self.midtone == [0.0; 3]
            && self.highlight == [0.0; 3]
            && self.contrast == 1.0
            && self.sat == 1.0
            && self.hue == [1.0, 0.0]
            && self.boost == 0.0
            && self.tonal == [0.0; 2]
            && self.detail == 0.0
            && !self.mixer_on
            && !self.curves_on
            && self.knees == [0.0, 1.0]
            && !self.hue_curves_on
            && !self.hdr.on
    }
}
