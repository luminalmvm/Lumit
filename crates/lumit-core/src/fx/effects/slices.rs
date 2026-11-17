//! Slices: a cut made of strips. The outgoing picture is cut into bands that
//! slide out of the frame one after another, and the incoming picture's bands
//! slide in right behind them.
//!
//! One instance runs on each of the two pictures, and Side says which it is.
//! Each strip has its own Completion, staggered from the first strip to the
//! last or in a seeded order, so the outgoing strip and the incoming one meet
//! at the same edge and the two pictures always add up to a whole one.

use crate::fx::{
    effects::move_transition::{EASE_OPTIONS, SIDE_OPTIONS},
    noise::hash01,
    params::Params,
    presets::EffectPreset,
    presets::PresetValue::{Bool, Choice, Float},
    registry::EffectDef,
    registry::EffectMetadata,
    schema::EffectSchema,
};
use lumit_fx_macros::Effect;

/// Taps in the smear. Both kernels average exactly this many.
pub const SLICES_TAPS: usize = 16;

/// The most strips, which is Count's hard limit.
pub const SLICES_MAX: usize = 64;

/// Slices' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "slices",
    label = "Slices",
    version = 1,
    category = Transition,
    // At most 16 pairs of reads a pixel.
    cost = Cheap,
    // A strip travels the whole frame.
    roi = FullFrame,
    // Random order is the same on both pictures under fixed controls.
    seeded = true,
    premultiplied = true,
)]
pub struct Slices {
    /// How far through the cut, per cent.
    #[bounded(min = 0.0, max = 100.0, default = 50.0, unit = Percent)]
    pub completion: f32,

    /// Which picture this is.
    #[choice(options = *SIDE_OPTIONS, default = 0)]
    pub side: u32,

    /// How many strips.
    #[counter(min = 2, max = 64, default = 8, hard_min = 2, hard_max = 64, unit = Raw)]
    pub count: i32,

    /// Which way the strips lie. Horizontal strips travel sideways.
    #[choice(options = ["Horizontal", "Vertical"], default = 0)]
    pub across: u32,

    /// Every other strip travels the other way.
    #[toggle(default = true)]
    pub alternate: bool,

    /// How far apart the strips set off, per cent of the cut.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 40.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub stagger: f32,

    /// The strips set off in a seeded order rather than one after another.
    #[toggle(label = "Random order", default = false)]
    pub random: bool,

    /// How each strip gathers speed.
    #[choice(options = *EASE_OPTIONS, default = 1)]
    pub ease: u32,

    /// How much of each strip's move is smeared, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 3.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub smear: f32,

    /// The order Random order takes.
    #[seed]
    pub seed: u32,

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

/// One frame of Slices, as both render paths draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlicesParams {
    /// How far through the cut, 0..1.
    pub t: f32,
    /// Strips, `2..=SLICES_MAX`.
    pub count: u32,
    /// The strips are columns that travel up and down.
    pub vertical: bool,
    pub alternate: bool,
    pub random: bool,
    /// How far apart the strips set off, 0..0.95.
    pub stagger: f32,
    /// The Ease code: 0 Linear, 1 Smooth, 2 Sharp.
    pub ease: u32,
    /// This is the incoming picture.
    pub incoming: bool,
    /// Half the widest smear window, in a strip's own time, 0..0.5.
    pub smear: f32,
    pub seed: u32,
    /// 0..1, blended against the unprocessed input.
    pub mix: f32,
}

impl SlicesParams {
    /// Nothing moves: the outgoing picture before the cut, or the incoming one
    /// after it.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.mix == 0.0 || (!self.incoming && self.t <= 0.0) || (self.incoming && self.t >= 1.0)
    }
}

impl Slices {
    /// The numbers both kernels read.
    #[must_use]
    pub fn packed(self) -> SlicesParams {
        SlicesParams {
            t: (self.completion / 100.0).clamp(0.0, 1.0),
            count: self.count.clamp(2, SLICES_MAX as i32) as u32,
            vertical: self.across == 1,
            alternate: self.alternate,
            random: self.random,
            stagger: (self.stagger / 100.0).clamp(0.0, 0.95),
            ease: self.ease.min(2),
            incoming: self.side == 1,
            smear: (self.smear / 200.0).clamp(0.0, 0.5),
            seed: self.seed,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// `t` through the Ease. Polynomial only, so the WGSL twin is exact.
fn eased(ease: u32, t: f32) -> f32 {
    match ease {
        0 => t,
        2 if t < 0.5 => 16.0 * t * t * t * t * t,
        2 => {
            let r = 1.0 - t;
            1.0 - 16.0 * r * r * r * r * r
        }
        _ => t * t * (3.0 - 2.0 * t),
    }
}

/// The shifts of strip `i` along a side `len` long, one a tap, and how many
/// taps there are. The WGSL twin is the same expression in the same order.
#[must_use]
pub fn slices_shifts(i: u32, len: f32, p: &SlicesParams) -> ([f32; SLICES_TAPS], usize) {
    let sign = if p.alternate && i % 2 == 1 { -1.0 } else { 1.0 };
    let order = if p.random {
        hash01(p.seed, 0, i as i32, 0, 0)
    } else {
        i as f32 / (p.count.max(2) - 1) as f32
    };
    let ti = ((p.t - order * p.stagger) / (1.0 - p.stagger)).clamp(0.0, 1.0);
    // The window narrows to nothing as the strip sets off and as it lands.
    let half = p.smear.min(ti).min(1.0 - ti);
    let count = if half > 0.0 { SLICES_TAPS } else { 1 };
    let mut shifts = [0.0; SLICES_TAPS];
    for (k, shift) in shifts.iter_mut().take(count).enumerate() {
        let at = if count == 1 {
            ti
        } else {
            ti - half + 2.0 * half * k as f32 / (SLICES_TAPS - 1) as f32
        };
        let e = eased(p.ease, at);
        *shift = if p.incoming {
            -sign * len * (1.0 - e)
        } else {
            sign * len * e
        };
    }
    (shifts, count)
}

/// The CPU oracle. Each strip is read `shift` back along its length, linear
/// along it, with nothing past the frame's edge, and the taps averaged in
/// order.
pub fn slices(rgba: &mut [f32], w: u32, h: u32, p: &SlicesParams) {
    let px = w as usize * h as usize;
    if p.is_identity() || px == 0 || rgba.len() < px * 4 {
        return;
    }
    let src = rgba.to_vec();
    let (len, across) = if p.vertical { (h, w) } else { (w, h) };
    let n = p.count.clamp(2, SLICES_MAX as u32);
    let mut strips = [([0.0; SLICES_TAPS], 1usize); SLICES_MAX];
    for (i, strip) in strips.iter_mut().take(n as usize).enumerate() {
        *strip = slices_shifts(i as u32, len as f32, p);
    }
    let read = |a: i64, b: u32| -> [f32; 4] {
        if a < 0 || a >= i64::from(len) {
            return [0.0; 4];
        }
        let (x, y) = if p.vertical {
            (b as usize, a as usize)
        } else {
            (a as usize, b as usize)
        };
        let o = (y * w as usize + x) * 4;
        src.get(o..o + 4)
            .and_then(|s| <[f32; 4]>::try_from(s).ok())
            .unwrap_or([0.0; 4])
    };
    for (o, out) in rgba.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((o % w as usize) as u32, (o / w as usize) as u32);
        let (a, b) = if p.vertical { (y, x) } else { (x, y) };
        let i = (u64::from(b) * u64::from(n) / u64::from(across)).min(u64::from(n - 1));
        let Some((shifts, count)) = strips.get(i as usize) else {
            continue;
        };
        let mut sum = [0.0f32; 4];
        for shift in shifts.iter().take(*count) {
            let f = a as f32 - shift;
            let i0 = f.floor();
            let frac = f - i0;
            let (lo, hi) = (read(i0 as i64, b), read(i0 as i64 + 1, b));
            for ((s, l), r) in sum.iter_mut().zip(lo).zip(hi) {
                *s += (1.0 - frac) * l + frac * r;
            }
        }
        let inv = 1.0 / *count as f32;
        for (v, s) in out.iter_mut().zip(sum) {
            *v = *v * (1.0 - p.mix) + s * inv * p.mix;
        }
    }
}

/// Slices' looks, which the Transition menu offers too.
pub const SLICES_LOOKS: &[EffectPreset] = &[
    EffectPreset {
        name: "Slices",
        values: &[],
    },
    EffectPreset {
        name: "Slice glitch",
        values: &[
            ("count", Float(24.0)),
            ("random", Bool(true)),
            ("stagger", Float(80.0)),
            ("smear", Float(30.0)),
            ("ease", Choice(2)),
        ],
    },
];

/// Slices' behaviour.
pub struct SlicesDef;

impl EffectDef for SlicesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Slices as EffectMetadata>::SCHEMA
    }

    fn presets(&self) -> &'static [EffectPreset] {
        SLICES_LOOKS
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        slices(rgba, w, h, &Slices::read(p).packed());
    }
}
