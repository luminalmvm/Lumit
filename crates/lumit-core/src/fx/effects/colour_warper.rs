//! Colour warper: a mesh over the colour wheel, its points dragged to move
//! whole families of colour at once.
//!
//! **In plain terms.** Picture the vectorscope with a web laid over it:
//! spokes at even hues and three rings out to full colour, plus the centre.
//! Drag a point and the colours under it go where it went, the ones round it
//! following smoothly, fading out by the next spoke and ring. Hue and
//! saturation move together. Luma stays put unless Keep luma is off, when the
//! brightest channel stays put instead. A mesh at rest changes nothing.
//!
//! # Rows
//!
//! `resolution` (6 or 8 hues, by 3 rings), `points` (Shape: each mesh
//! point's move, below), `keep_luma`, `strength` and `mix`.
//!
//! `points` holds one move a mesh point, in the chroma plane's own units
//! (`(Cb, Cr)` over 0.5, so the outer ring is 1 from the centre). Point 0 is
//! the centre; ring `r` (1..3) spoke `k` is point `1 + (r − 1)·n + k`, `n` the
//! hues. A point the row doesn't reach has not moved.
//!
//! # The formulas
//!
//! Worked in video gamma, `v = sgnpow(u, 1/2.4)`, as a Correction's Video
//! space. `c = YCbCr(v)`, `p = (Cb, Cr)/0.5`. In mesh units `θ = n·hue(Cb,
//! Cr)` (red at 0, the vectorscope's way round) and `ρ = 3·|p|`, held to 0..3.
//! - Round each ring the moves are a periodic Catmull-Rom spline through the
//!   spokes at `θ`, so hue 0 has no seam: `R_r(θ)`, `r = 1..3`, and
//!   `R_0 = centre`.
//! - Out from the centre a Catmull-Rom through `R_0..R_3` at `ρ`, with
//!   `R_−1 = 2R_0 − R_1` and `R_4 = 2R_3 − R_2` at the ends: `D`.
//! - `p' = p + strength·D`; `v' = RGB(Y, 0.5·p')`. Without Keep luma, `v'` is
//!   scaled so its largest channel is `v`'s.
//! - `w = v·(1 − mix) + v'·mix`, back to linear and premultiplied.

use crate::fx::effects::correction::maths;
use crate::fx::shape_points::ShapePoints;
use crate::fx::{
    params::Params, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// The match name.
pub const COLOUR_WARPER: &str = "colour_warper";

/// How many hues a mesh has, by Resolution.
pub const RESOLUTION_OPTIONS: [&str; 2] = ["Six hues", "Eight hues"];

/// The mesh's rings, the centre not counted.
pub const RINGS: usize = 3;

/// The most mesh points: eight hues by three rings, and the centre.
pub const MESH_MAX: usize = 1 + 8 * RINGS;

/// How many `vec4`s [`WarpPacked::words`] writes.
pub const WORDS: usize = 16;

/// Video gamma, the space the mesh is drawn in.
const SPACE: u32 = 0;

/// Colour warper's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "colour_warper",
    label = "Colour warper",
    version = 1,
    category = Colour,
    cost = Cheap,
    roi = Exact,
    // A grade shifts matte edges when worked premultiplied.
    premultiplied = false,
)]
pub struct ColourWarper {
    /// How many spokes the mesh has, round three rings.
    #[choice(options = RESOLUTION_OPTIONS, default = 0)]
    pub resolution: u32,
    /// Where each mesh point has been moved to, from where it sits.
    #[shape(label = "Points")]
    pub points: ShapePoints,
    /// Moves colour without moving brightness.
    #[toggle(label = "Keep luma", default = true)]
    pub keep_luma: bool,
    /// How far the colours follow the points.
    #[bounded(min = 0.0, max = 100.0, default = 100.0, unit = Percent)]
    pub strength: f32,
    /// The host-uniform Mix every effect ends with, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,
}

/// The mesh, ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WarpPacked {
    /// Hues: 6 or 8.
    pub hues: u32,
    /// Each point's move, strength applied, centre first.
    pub moves: [[f32; 2]; MESH_MAX],
    pub keep_luma: bool,
    pub mix: f32,
    /// The mesh at rest, or Mix 0.
    pub neutral: bool,
}

impl ColourWarper {
    /// What both render paths draw with.
    #[must_use]
    pub fn packed(&self) -> WarpPacked {
        let hues = if self.resolution == 1 { 8 } else { 6 };
        let strength = (self.strength / 100.0).clamp(0.0, 1.0);
        let mut moves = [[0.0f32; 2]; MESH_MAX];
        let used = 1 + hues as usize * RINGS;
        for (slot, p) in moves.iter_mut().zip(self.points.points()).take(used) {
            *slot = [p[0] * strength, p[1] * strength];
        }
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        WarpPacked {
            hues,
            moves,
            keep_luma: self.keep_luma,
            mix,
            neutral: mix == 0.0 || moves.iter().all(|m| *m == [0.0, 0.0]),
        }
    }
}

/// Catmull-Rom between `b` and `c` at `t`, `a` and `d` beyond them.
#[must_use]
pub fn spline(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2], t: f32) -> [f32; 2] {
    let (t2, t3) = (t * t, t * t * t);
    [0, 1].map(|i| {
        0.5 * (2.0 * b[i]
            + (c[i] - a[i]) * t
            + (2.0 * a[i] - 5.0 * b[i] + 4.0 * c[i] - d[i]) * t2
            + (3.0 * b[i] - a[i] - 3.0 * c[i] + d[i]) * t3)
    })
}

impl WarpPacked {
    /// The uniform `fx_grade.wgsl`'s `colour_warper` reads: the header, then
    /// two moves a `vec4`.
    #[must_use]
    pub fn words(&self) -> [[f32; 4]; WORDS] {
        let mut out = [[0.0f32; 4]; WORDS];
        out[0] = [self.hues as f32, f32::from(self.keep_luma), self.mix, 0.0];
        for (i, m) in self.moves.iter().enumerate() {
            let w = &mut out[1 + i / 2];
            w[2 * (i % 2)] = m[0];
            w[2 * (i % 2) + 1] = m[1];
        }
        out
    }

    /// Ring `r`'s move at spoke position `theta`: the centre's for ring 0.
    fn ring(&self, r: usize, theta: f32) -> [f32; 2] {
        if r == 0 {
            return self.moves[0];
        }
        let n = self.hues as usize;
        let k0 = theta.floor();
        let t = theta - k0;
        let k = k0 as isize;
        let at = |o: isize| {
            let spoke = (k + o).rem_euclid(n as isize) as usize;
            self.moves
                .get(1 + (r - 1) * n + spoke)
                .copied()
                .unwrap_or([0.0, 0.0])
        };
        spline(at(-1), at(0), at(1), at(2), t)
    }

    /// The mesh's move at mesh position `theta` (spokes) and `rho` (rings).
    #[must_use]
    pub fn move_at(&self, theta: f32, rho: f32) -> [f32; 2] {
        let f = rho.clamp(0.0, RINGS as f32);
        let r0 = (f.floor() as usize).min(RINGS - 1);
        let t = f - r0 as f32;
        let ring = |r: isize| -> [f32; 2] {
            let lerp2 = |a: [f32; 2], b: [f32; 2]| [2.0 * a[0] - b[0], 2.0 * a[1] - b[1]];
            if r < 0 {
                lerp2(self.ring(0, theta), self.ring(1, theta))
            } else if r as usize > RINGS {
                lerp2(self.ring(RINGS, theta), self.ring(RINGS - 1, theta))
            } else {
                self.ring(r as usize, theta)
            }
        };
        let r = r0 as isize;
        spline(ring(r - 1), ring(r), ring(r + 1), ring(r + 2), t)
    }

    /// One premultiplied pixel through the mesh.
    #[must_use]
    pub fn pixel(&self, o: [f32; 4]) -> [f32; 4] {
        let v = maths::v_in(o, SPACE);
        let c = maths::ycc(v);
        let p = [c[1] / 0.5, c[2] / 0.5];
        let theta = self.hues as f32 * maths::hue_of(c[1], c[2]);
        let rho = RINGS as f32 * (p[0] * p[0] + p[1] * p[1]).sqrt();
        let d = self.move_at(theta, rho);
        let mut w = maths::rgb_of([c[0], 0.5 * (p[0] + d[0]), 0.5 * (p[1] + d[1])]);
        if !self.keep_luma {
            let top = v[0].max(v[1]).max(v[2]);
            let now = w[0].max(w[1]).max(w[2]);
            if top > 0.0 && now > 1e-6 {
                let k = top / now;
                w = w.map(|x| x * k);
            }
        }
        let a = o[3];
        let out = [0, 1, 2].map(|i| {
            let m = v[i] * (1.0 - self.mix) + w[i] * self.mix;
            maths::decode(m, SPACE).clamp(-maths::BOUND, maths::BOUND) * a
        });
        [out[0], out[1], out[2], a]
    }
}

/// Colour warper's behaviour.
pub struct ColourWarperDef;

impl EffectDef for ColourWarperDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ColourWarper as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], _w: u32, _h: u32, p: Params<'_>) {
        let pk = ColourWarper::read(p).packed();
        if pk.neutral {
            return;
        }
        for px in rgba.chunks_exact_mut(4) {
            let out = pk.pixel([px[0], px[1], px[2], px[3]]);
            px.copy_from_slice(&out);
        }
    }
}
