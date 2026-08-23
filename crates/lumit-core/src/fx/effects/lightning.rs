//! Lightning (docs/08 §3.74): a forked bolt between two points — AE's Advanced
//! Lightning.
//!
//! **In plain terms.** A jagged line of light from one point to another, with
//! smaller branches leaving it. Conductivity state is the bolt's shape: hold it
//! still and the bolt holds still, animate it and the bolt writhes, scrub back
//! and exactly the same bolt comes back.
//!
//! **The bolt is built here, not in the kernel** (docs/08 §3.74's first
//! decision). A bolt is a recursive displacement of a straight line, and
//! recursion is what a per-pixel kernel cannot afford to redo two million times
//! for a shape that is the same for every pixel. [`Lightning::packed`] walks it
//! once a frame into a list of straight segments; the kernel then only measures
//! distances to them, which also disposes of §1.6 for free — both paths are
//! handed the identical numbers, so there is no second generator to disagree
//! with the first.

use crate::fx::noise::{fractal, hash01, FractalField};
use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, Params};
use lumit_fx_macros::Effect;

/// Steps in a main bolt, and in one of the several bolts the multi-bolt types
/// draw. Chosen so the worst case — five Omni bolts plus a full complement of
/// forks — fits [`cpu::LIGHTNING_SEGMENTS`] with room to spare.
const STEPS: usize = 24;
const MULTI_STEPS: usize = 16;
const FORK_STEPS: usize = 6;
const MAX_FORKS: usize = 12;
/// The longest bolt this module walks, in points.
const MAX_POINTS: usize = STEPS + 1;

/// Lightning's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "lightning",
    label = "Lightning",
    version = 1,
    category = Generate,
    // Up to 192 capsule distances a pixel — the price of not rebuilding the bolt
    // per pixel, and much the cheaper end of that trade.
    cost = Moderate,
    roi = Exact,
    premultiplied = true,
    seeded = true,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the bolt's opacity per pixel: white draws the core and glow in          full, grey faintly, black nothing at all",
    ),
)]
pub struct Lightning {
    /// Where the bolt leaves from, px@comp (K-260: point parameters are PIXELS).
    #[slider(label = "Origin x", min = 0.0, max = 3840.0, default = 300.0, unit = Px)]
    pub origin_x: f32,

    /// px@comp; see [`origin_x`](Self::origin_x).
    #[slider(label = "Origin y", min = 0.0, max = 2160.0, default = 900.0, unit = Px)]
    pub origin_y: f32,

    /// Where the bolt is aimed, px@comp. Under Omni it is only the *radius* that
    /// is read from it, since those bolts go every way at once.
    #[slider(label = "Direction x", min = 0.0, max = 3840.0, default = 1620.0, unit = Px)]
    pub direction_x: f32,

    /// px@comp; see [`direction_x`](Self::direction_x).
    #[slider(label = "Direction y", min = 0.0, max = 2160.0, default = 200.0, unit = Px)]
    pub direction_y: f32,

    /// Four of AE's eight, chosen to be visibly different from one another
    /// (§3.74's third decision): the far end free, both ends pinned, five bolts
    /// radiating, or two bolts meeting in the middle.
    #[choice(
        label = "Type",
        options = ["Direction", "Strike", "Omni", "Two-way strike"],
        default = 0
    )]
    pub lightning_type: u32,

    /// The depth axis of the noise the bolt is displaced by — AE's control under
    /// AE's name. Animate it to make the bolt writhe; it is a coordinate, never a
    /// clock (§2.4).
    #[slider(label = "Conductivity state", min = 0.0, max = 100.0, default = 0.0)]
    pub conductivity: f32,

    /// Which bolt (§2.4).
    #[seed]
    pub seed: u32,

    /// How far the bolt wanders off the straight line, as a per cent of its own
    /// length.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 12.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub amplitude: f32,

    /// How many branches leave the bolt, per cent of the twelve there is room
    /// for.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 45.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub forking: f32,

    /// How much dimmer the bolt's far end is than its root, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 30.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub decay: f32,

    /// The bright filament's half-width, px@comp (§2.3).
    #[slider(
        label = "Core radius",
        min = 0.0,
        max = 40.0,
        default = 3.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub core_radius: f32,

    /// The filament's colour. Scene-linear and open above 1 (§2.1).
    #[colour(label = "Core colour", default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub core_colour: [f32; 4],

    /// How far the halo reaches, px@comp.
    #[slider(
        label = "Glow radius",
        min = 0.0,
        max = 200.0,
        default = 22.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub glow_radius: f32,

    /// The halo's colour.
    #[colour(label = "Glow colour", default = [0.25, 0.45, 1.0, 1.0], max = 4.0)]
    pub glow_colour: [f32; 4],

    /// How strong the halo is, per cent.
    #[slider(
        label = "Glow opacity",
        min = 0.0,
        max = 100.0,
        default = 70.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub glow_opacity: f32,

    /// On, the layer that arrived stays under the bolt; off, the bolt is all
    /// there is.
    #[toggle(label = "Composite on original", default = true)]
    pub composite_on_original: bool,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub mix: f32,
}

/// One bolt's points: a straight run from `a` to `b`, pushed sideways by the
/// shared noise core.
///
/// `pinned` is the difference between AE's Direction and its Strike — the
/// envelope is `t` for a far end that wanders free, and `sin(πt)` for a bolt
/// that lands exactly on its target. The noise is **turbulent** (the core's
/// `|n|` fold), which is what puts creases in the line rather than curves.
fn bolt_points(
    a: [f32; 2],
    b: [f32; 2],
    steps: usize,
    seed: u32,
    amp: f32,
    z: f32,
    pinned: bool,
) -> [[f32; 2]; MAX_POINTS] {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let len = (dx * dx + dy * dy).sqrt().max(1e-3);
    // The unit perpendicular the displacement rides on.
    let nx = -dy / len;
    let ny = dx / len;
    let field = FractalField {
        seed,
        octaves: 4,
        gain: 0.55,
        lacunarity: 2.0,
        perlin: false,
        turbulent: true,
        cycle: 0,
    };
    let amp_px = amp * len;
    let mut out = [[0.0f32; 2]; MAX_POINTS];
    let steps = steps.min(STEPS);
    for (i, p) in out.iter_mut().enumerate().take(steps + 1) {
        let t = i as f32 / steps as f32;
        let env = if pinned {
            (std::f32::consts::PI * t).sin()
        } else {
            t
        };
        let w = fractal(&field, t * 6.0, 0.0, z);
        let o = amp_px * w * env;
        *p = [a[0] + dx * t + nx * o, a[1] + dy * t + ny * o];
    }
    out
}

/// Everything the emitter needs to append one bolt's segments to the list.
struct Bolt<'a> {
    points: &'a [[f32; 2]; MAX_POINTS],
    steps: usize,
    /// The brightness at the bolt's root and at its far end.
    fade0: f32,
    fade1: f32,
}

/// Append one bolt's segments, stopping at [`cpu::LIGHTNING_SEGMENTS`] rather
/// than growing (docs/14: budgeted allocations, and this one is a fixed array).
fn emit(p: &mut cpu::LightningParams, count: &mut usize, b: &Bolt<'_>) {
    for i in 0..b.steps {
        if *count >= cpu::LIGHTNING_SEGMENTS {
            return;
        }
        let t = (i as f32 + 0.5) / b.steps as f32;
        p.segments[*count] = [
            b.points[i][0],
            b.points[i][1],
            b.points[i + 1][0],
            b.points[i + 1][1],
        ];
        p.fades[*count] = b.fade0 + (b.fade1 - b.fade0) * t;
        *count += 1;
    }
}

impl Lightning {
    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4) —
    /// including the whole bolt, already built (§3.74's first decision).
    #[must_use]
    pub fn packed(self) -> cpu::LightningParams {
        let mut p = cpu::LightningParams {
            segments: [[0.0; 4]; cpu::LIGHTNING_SEGMENTS],
            fades: [0.0; cpu::LIGHTNING_SEGMENTS],
            count: 0,
            core_radius: self.core_radius.max(0.0),
            glow_radius: self.glow_radius.max(0.0),
            glow_opacity: (self.glow_opacity / 100.0).clamp(0.0, 1.0),
            core_colour: [
                self.core_colour[0],
                self.core_colour[1],
                self.core_colour[2],
            ],
            glow_colour: [
                self.glow_colour[0],
                self.glow_colour[1],
                self.glow_colour[2],
            ],
            composite: self.composite_on_original,
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        };

        let a = [self.origin_x, self.origin_y];
        let b = [self.direction_x, self.direction_y];
        let amp = (self.amplitude / 100.0).clamp(0.0, 1.0);
        let decay = (self.decay / 100.0).clamp(0.0, 1.0);
        // The depth coordinate. A hundred per cent of Conductivity state walks
        // four cells of the field, which is several complete reshapings.
        let z = self.conductivity * 0.04;
        let seed = self.seed;
        let mut count = 0usize;

        // The bolt the forks hang off, whatever the type draws beside it.
        let main;
        let main_steps;
        match self.lightning_type {
            1 => {
                main_steps = STEPS;
                main = bolt_points(a, b, main_steps, seed, amp, z, true);
                emit(
                    &mut p,
                    &mut count,
                    &Bolt {
                        points: &main,
                        steps: main_steps,
                        fade0: 1.0,
                        fade1: 1.0 - decay,
                    },
                );
            }
            2 => {
                // Omni: five bolts out to the Direction point's radius, every
                // way at once. Only the first carries forks.
                let dx = b[0] - a[0];
                let dy = b[1] - a[1];
                let radius = (dx * dx + dy * dy).sqrt().max(1.0);
                let base = dy.atan2(dx);
                main_steps = MULTI_STEPS;
                let mut first = [[0.0f32; 2]; MAX_POINTS];
                for k in 0..5 {
                    let ang = base + k as f32 * std::f32::consts::TAU / 5.0;
                    let end = [a[0] + radius * ang.cos(), a[1] + radius * ang.sin()];
                    let pts = bolt_points(
                        a,
                        end,
                        main_steps,
                        seed.wrapping_add((k as u32).wrapping_mul(0x9e37_79b9)),
                        amp,
                        z,
                        false,
                    );
                    emit(
                        &mut p,
                        &mut count,
                        &Bolt {
                            points: &pts,
                            steps: main_steps,
                            fade0: 1.0,
                            fade1: 1.0 - decay,
                        },
                    );
                    if k == 0 {
                        first = pts;
                    }
                }
                main = first;
            }
            3 => {
                // Two-way strike: one bolt from each end, meeting in the middle.
                let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
                main_steps = MULTI_STEPS;
                main = bolt_points(a, mid, main_steps, seed, amp, z, true);
                emit(
                    &mut p,
                    &mut count,
                    &Bolt {
                        points: &main,
                        steps: main_steps,
                        fade0: 1.0,
                        fade1: 1.0 - decay,
                    },
                );
                let back = bolt_points(
                    b,
                    mid,
                    main_steps,
                    seed.wrapping_add(0x85eb_ca6b),
                    amp,
                    z,
                    true,
                );
                emit(
                    &mut p,
                    &mut count,
                    &Bolt {
                        points: &back,
                        steps: main_steps,
                        fade0: 1.0,
                        fade1: 1.0 - decay,
                    },
                );
            }
            _ => {
                main_steps = STEPS;
                main = bolt_points(a, b, main_steps, seed, amp, z, false);
                emit(
                    &mut p,
                    &mut count,
                    &Bolt {
                        points: &main,
                        steps: main_steps,
                        fade0: 1.0,
                        fade1: 1.0 - decay,
                    },
                );
            }
        }

        // The forks. Each leaves a hashed step of the main bolt at a hashed
        // angle, runs a fraction of its length, and is dimmer than what it left.
        let forks = ((self.forking / 100.0).clamp(0.0, 1.0) * MAX_FORKS as f32).round() as usize;
        // The fork layout re-rolls as Conductivity state walks, so a moving bolt
        // grows and sheds branches rather than waving a fixed set about.
        let zc = (z * 4.0).floor() as i32;
        let bolt_len = {
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            (dx * dx + dy * dy).sqrt().max(1.0)
        };
        for k in 0..forks {
            let ki = k as i32;
            let r0 = hash01(seed, 40, ki, 0, zc);
            let r1 = hash01(seed, 41, ki, 0, zc);
            let r2 = hash01(seed, 42, ki, 0, zc);
            let idx = 1 + (r0 * (main_steps as f32 - 2.0)) as usize;
            let idx = idx.min(main_steps - 1);
            let from = main[idx];
            let tx = main[idx + 1][0] - from[0];
            let ty = main[idx + 1][1] - from[1];
            let tl = (tx * tx + ty * ty).sqrt().max(1e-3);
            // ±50° off the parent's own direction, which is where a branch goes.
            let ang = (r1 - 0.5) * 1.75;
            let (s, c) = ang.sin_cos();
            let dirx = (tx * c - ty * s) / tl;
            let diry = (tx * s + ty * c) / tl;
            let flen = bolt_len * (0.10 + 0.22 * r2);
            let end = [from[0] + dirx * flen, from[1] + diry * flen];
            let pts = bolt_points(
                from,
                end,
                FORK_STEPS,
                seed.wrapping_add(0xc2b2_ae35).wrapping_add(k as u32),
                amp * 1.5,
                z,
                false,
            );
            let at = 1.0 - decay * (idx as f32 / main_steps as f32);
            emit(
                &mut p,
                &mut count,
                &Bolt {
                    points: &pts,
                    steps: FORK_STEPS,
                    fade0: at * 0.65,
                    fade1: at * 0.15,
                },
            );
        }

        p.count = count as u32;
        p
    }
}

/// Lightning's behaviour.
pub struct LightningDef;

impl EffectDef for LightningDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Lightning as EffectMetadata>::SCHEMA
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        cpu::lightning(rgba, w, h, &Lightning::read(p).packed());
    }
}
