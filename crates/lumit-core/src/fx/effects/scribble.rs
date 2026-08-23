//! Scribble (docs/08 §3.78): a mask filled with pencil strokes — AE's Scribble.
//!
//! **In plain terms.** You draw a mask; this fills it in, the way somebody
//! shading a shape with a pencil fills it in — back and forth in parallel
//! strokes at whatever angle you choose, running a little past the edges the way
//! a hand does, and wavering as it goes.
//!
//! It is the first effect to read the **shape of a mask** rather than the hole
//! the mask cuts (K-408, docs/08 §1.2). A coverage buffer says which pixels are
//! inside; it cannot say where the boundary *runs*, and a hatch has to know
//! that to know where each stroke stops.
//!
//! Two things about how it works, because they are what make the controls mean
//! what they say. The strokes are one **continuous line** — the pen crosses the
//! shape, hops down the edge, and comes back — which is why Start and End trim
//! it the way a pen drawing it would, and why the pen has to lift where a shape
//! has a notch in it. And the waver is **not in the strokes**: the paper is
//! displaced by a smooth noise field instead, so every stroke wavers for the
//! price of one noise lookup a pixel rather than eight times the geometry
//! (§3.78's second decision).

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use crate::mask::MaskPolyline;
use lumit_fx_macros::Effect;

/// Scribble's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "scribble",
    label = "Scribble",
    version = 1,
    category = Generate,
    cost = Moderate,
    // The kernel reads its own pixel and nothing else: the drawing arrives as
    // geometry, not as a neighbourhood of the picture.
    roi = Exact,
    premultiplied = true,
    seeded = true,
    enabled_when = SCRIBBLE_ENABLED_WHEN,
    // K-428: the matte scales the amount, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales Opacity per pixel: white draws the scribble in full, grey          faintly, black nothing at all",
    ),
)]
pub struct Scribble {
    /// Which of the layer's masks to fill (K-408). Unset is **First mask**,
    /// because the effect is usually added before the mask is drawn.
    #[mask_path(label = "Mask")]
    pub path: bool,

    /// The pencil's colour. Scene-linear and open above 1 (§2.1).
    #[colour(default = [0.85, 0.16, 0.12, 1.0], max = 4.0)]
    pub colour: [f32; 4],

    /// Which way the strokes run. A dial, because it is an angle.
    #[dial(default = 30.0, step = 15.0)]
    pub angle: f32,

    /// How thick one pencil stroke is, px@comp (§2.3).
    #[slider(
        label = "Stroke width",
        min = 0.1,
        max = 20.0,
        default = 2.0,
        hard_min = 0.1,
        unit = Px
    )]
    pub stroke_width: f32,

    /// How far apart the strokes are laid, px@comp. Below the stroke width they
    /// merge into a solid fill; well above it the shape reads as hatching.
    #[slider(min = 1.0, max = 200.0, default = 8.0, hard_min = 0.5, unit = Px)]
    pub spacing: f32,

    /// How far each stroke runs past the mask's edge, px@comp — AE's Path
    /// Overlap. Negative keeps the strokes short of it, which is the neat,
    /// deliberate look.
    #[slider(
        label = "Path overlap",
        min = -50.0,
        max = 50.0,
        default = 4.0,
        unit = Px
    )]
    pub path_overlap: f32,

    /// Where the pen starts, per cent of the whole drawing's length. Keyframe
    /// End from 0 and the scribble draws itself on.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub start: f32,

    /// Where it stops; see [`start`](Self::start).
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub end: f32,

    /// How the waver moves. **Static** holds one arrangement; **Jagged** snaps
    /// to a new one [`wiggles_per_second`](Self::wiggles_per_second) times a
    /// second, which is the pencil-test look; **Wiggly** drifts between them
    /// continuously.
    #[choice(label = "Wiggle type", options = ["Static", "Jagged", "Wiggly"], default = 0)]
    pub wiggle_type: u32,

    /// How fast it moves. Greyed while the waver is Static, which is not moving
    /// at all.
    #[slider(
        label = "Wiggles per second",
        min = 0.0,
        max = 30.0,
        default = 8.0,
        hard_min = 0.0
    )]
    pub wiggles_per_second: f32,

    /// Which waver. Two Scribbles on one shape with different seeds are two
    /// different hands.
    #[seed]
    pub seed: u32,

    /// How strong the pencil is, per cent.
    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0
    )]
    pub opacity: f32,

    /// On, the layer that arrived stays under the scribble; off, the scribble is
    /// all there is — which is how a fill becomes its own element.
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

/// A rate is not a control while nothing is moving.
pub const SCRIBBLE_ENABLED_WHEN: &[crate::fx::EnabledWhen] = &[crate::fx::EnabledWhen {
    param: "wiggles_per_second",
    on: "wiggle_type",
    cond: crate::fx::EnabledCond::ChoiceIsNot(Scribble::WIGGLE_STATIC),
}];

/// The waver's amplitude, as a share of the spacing, and its wavelength, as a
/// multiple of it (docs/08 §3.78's second decision).
///
/// Tied to the spacing rather than shipped as two more controls, and tied
/// *there* rather than to the stroke width because what the waver must not do is
/// walk one stroke into its neighbour. A fifth of the gap, over four gaps, moves
/// the paper by about a third of a stroke width per stroke width travelled: a
/// hand's waver, and comfortably short of the point where a displacement folds
/// the picture back on itself.
const WIGGLE_AMPLITUDE: f32 = 0.2;
/// See [`WIGGLE_AMPLITUDE`].
const WIGGLE_WAVELENGTH: f32 = 4.0;

impl Scribble {
    /// The [`wiggle_type`](Self::wiggle_type) option that does not move. Named
    /// because two places pin the tick on it, and a bare 0 in either would be
    /// the one that went stale.
    pub const WIGGLE_STATIC: u32 = 0;

    /// Raster pixels per comp pixel (§2.3), pushed at resolve because the seam
    /// hands its vertices over in px@comp and the drawing happens in the raster
    /// (K-408). Never a panel row.
    pub const DERIVED_PX_SCALE: ParamId = ParamId::new("derived.px_scale");

    /// Where in the waver's evolution this frame sits — already a number rather
    /// than a clock, for §2.4's reason. Never a panel row.
    pub const DERIVED_TICK: ParamId = ParamId::new("derived.tick");

    /// This instance's two derived values, read back out of a resolved bag, so
    /// no caller has to know the ids.
    #[must_use]
    pub fn derived_of(p: Params<'_>) -> (f32, f32) {
        (
            p.float(Self::DERIVED_PX_SCALE, 1.0),
            p.float(Self::DERIVED_TICK, 0.0),
        )
    }

    /// The bundle both kernels consume (docs/impl/effect-registry.md §2.4): the
    /// hatch, already laid out and already trimmed.
    ///
    /// An empty or absent mask leaves the count at zero, which the kernel draws
    /// as nothing — the documented no-op both paths take (docs/08 §1.2).
    #[must_use]
    pub fn packed(self, poly: &MaskPolyline, px_scale: f32, tick: f32) -> cpu::PathDrawParams {
        let mut p = cpu::PathDrawParams::blank();
        let spacing = self.spacing.max(0.5);
        let pts = cpu::path_points(poly, px_scale.max(1e-6));
        let chain = cpu::scribble_chain(&pts, self.angle, spacing, self.path_overlap);
        cpu::path_chain(&chain, self.start, self.end, &mut p);
        p.half_width = self.stroke_width.max(0.0) * 0.5;
        // A pencil line has a crisp edge; the band is one pixel of anti-aliasing
        // and nothing more, which is the floor Vegas' soft stroke also lands on.
        p.band = 0.5;
        p.wiggle_amp = spacing * WIGGLE_AMPLITUDE;
        p.wiggle_freq = 1.0 / (spacing * WIGGLE_WAVELENGTH);
        // Static is pinned **here as well as at resolve**, Add grain's Animate
        // toggle exactly: the effect's own numbers must say a still scribble is
        // still, rather than trusting a derived value that a bag could carry
        // over from another wiggle type (K-258).
        p.wiggle_tick = if self.wiggle_type == Self::WIGGLE_STATIC {
            0.0
        } else {
            tick
        };
        p.seed = self.seed;
        p.colour = [self.colour[0], self.colour[1], self.colour[2]];
        p.opacity = (self.opacity / 100.0).clamp(0.0, 1.0);
        p.style = if self.composite_on_original {
            cpu::PAINT_ON_ORIGINAL
        } else {
            cpu::PAINT_ON_TRANSPARENT
        };
        p.mix = (self.mix / 100.0).clamp(0.0, 1.0);
        p
    }
}

/// Scribble's behaviour: no CPU reference through the trait, because the mask's
/// geometry arrives beside the op rather than in the bag — the same shape Set
/// matte and the LUT have, and for the same reason. `apply_cpu` keeps its
/// identity default, which is exactly what an unset mask row renders anyway; the
/// §1.6 oracle is [`crate::fx::cpu::path_draw`], exercised directly from the
/// lumit-gpu test, which can build a polyline.
pub struct ScribbleDef;

impl EffectDef for ScribbleDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Scribble as EffectMetadata>::SCHEMA
    }

    /// The raster factor, and the frame's place in the waver.
    ///
    /// Every step of the tick is taken in `f64` and only then narrowed, as
    /// §3.36's is: an `f32` layer time would round differently either side of a
    /// tick boundary, and the frame a jagged waver snaps on has to be decided
    /// once and identically on every machine (§2.4).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        push(Scribble::DERIVED_PX_SCALE, Value::Float(cx.px_scale));
        let e = cx.inst;
        // A choice is not a float, and reading it as one is silently Static
        // for ever — the row is `EffectValue::Choice`, so it comes off the
        // instance directly rather than through the animated readers.
        let kind = match e.param("wiggle_type") {
            Some(crate::model::EffectValue::Choice(v)) => *v,
            _ => Scribble::WIGGLE_STATIC,
        };
        let rate = e
            .float_at_with_context("wiggles_per_second", cx.lt, cx.context.clone())
            .unwrap_or(8.0);
        let turns = cx.lt * rate;
        let tick = match kind {
            // Jagged: a new arrangement per wiggle, which is a floor and not a
            // round — a round would move the snap half a wiggle earlier and put
            // the first one before the layer starts. Wiggly: a continuous drift.
            // Static holds, and `packed` pins that a second time.
            1 => turns.floor(),
            2 => turns,
            _ => 0.0,
        };
        push(Scribble::DERIVED_TICK, Value::Float(tick as f32));
    }
}
