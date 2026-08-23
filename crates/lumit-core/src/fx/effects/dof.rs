//! Depth of field (docs/08 §3.22, docs/impl/layer-input.md, K-288/K-313): a lens
//! blur whose per-pixel circle of confusion comes from a depth pass.
//!
//! **In plain terms.** The largest declaration in the catalogue, and almost all
//! of it is arithmetic: where focus sits, how wide the iris opens on each side of
//! it, what shape the iris is, how hard the highlights bloom, and how the depth
//! pass is read. The one thing that is *not* a number is the depth pass itself —
//! a whole picture, the referenced layer rendered alone at this raster — so it
//! arrives beside the resolved op (K-387). **It is this effect's Matte** (K-395):
//! the same row every effect has, under this effect's older stored id (`depth`,
//! K-065) and with the deeper meaning this effect declares — a depth, not a
//! strength. It therefore rides the one matte carriage rather than a list of its
//! own. An unset, missing or cyclic reference leaves the slot empty and the
//! effect is the labelled no-op every layer-input effect follows.
//!
//! Whether a depth pass arrived is itself part of the maths: with none, the rows
//! that describe how to *read* one have nothing to describe, so the focus point,
//! the edge-leak suppression and the diagnostic views are neutralised rather than
//! left to sample a stand-in texture. That is why [`Dof::packed`] takes the
//! binding as an argument — the render knows it, the bag cannot (a Layer row
//! never reaches the arena).
//!
//! There is no CPU reference through the single-buffer dispatcher, which carries
//! no second picture, so `apply_cpu` keeps its identity default — the passthrough
//! the old `Resolved::Dof` arm of `cpu::apply` was. The §1.6 oracle is
//! [`crate::fx::cpu::dof`], exercised directly from the lumit-gpu test, which can
//! upload a depth map.

use crate::fx::{
    aperture_blades, cpu, EffectDef, EffectMetadata, EffectSchema, EnabledCond, EnabledWhen,
    ParamGroup, ParamId, Params, ResolveCx, Value, CHANNEL_OPTIONS, MAX_BLADES,
};
use lumit_fx_macros::Effect;

/// The panel's three twirls. Each is a contiguous run of the declared rows
/// (K-145), collapsed by default: the effect must read as Focus + Aperture until
/// someone goes looking for the shaping.
pub const DOF_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Iris",
        params: &["blades", "roundness", "rotation", "aspect", "rim"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Highlights",
        params: &["threshold", "exposure"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        // How the depth pass is READ — which number in it is depth, which way
        // round it runs, and how hard the blur answers to it. Where focus *is*
        // lives above, beside the rows that set it.
        label: "Depth map",
        // `depth_invert` used to sit here. K-395 moved it up beside the picker,
        // where every effect's Invert now lives; the group is a contiguous run
        // of declared rows, so it leaves the list as well as the struct.
        params: &[
            "depth_channel",
            "gamma",
            "remove_edge_leak",
            "detect_edge_threshold",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// The greyed rows: which of two controls is in charge, said in the panel rather
/// than left for the owner to discover by dragging something inert.
pub const DOF_ENABLED_WHEN: &[EnabledWhen] = &[
    // Focus point takes over from Focus distance. While it is ticked, focus is
    // whatever depth sits under the point and the distance number decides
    // nothing — and while it is not, the point does not.
    EnabledWhen {
        param: "focus",
        on: "use_focus_point",
        cond: EnabledCond::BoolIs(false),
    },
    EnabledWhen {
        param: "focus_point_x",
        on: "use_focus_point",
        cond: EnabledCond::BoolIs(true),
    },
    EnabledWhen {
        param: "focus_point_y",
        on: "use_focus_point",
        cond: EnabledCond::BoolIs(true),
    },
    // Everything that reads the depth pass needs one to read. With no layer
    // picked the effect defocuses the frame uniformly, and these rows have
    // nothing to describe.
    EnabledWhen {
        param: "depth_channel",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "use_focus_point",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "remove_edge_leak",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "detect_edge_threshold",
        on: "depth",
        cond: EnabledCond::LayerSet,
    },
];

/// Depth of field's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "dof",
    label = "Depth of field",
    version = 1,
    category = BlurSharpen,
    cost = Moderate,
    // Aperture is px@comp and its hard maximum is open (typing above the 40 px
    // slider is allowed), so the padding is the slider maximum doubled — a safe
    // static bound for a runtime-sized gather (docs/impl/layer-input.md). The
    // aperture polygon is INSCRIBED in that circle at every Roundness and
    // Deform (see `aperture_blades`), so shaping it can only gather fewer taps.
    roi = PaddedPx(80.0),
    premultiplied = true, // the aperture gathers premultiplied colour (fx_dof.wgsl)
    groups = DOF_GROUPS,
    enabled_when = DOF_ENABLED_WHEN,
    // K-395: Depth of field claims the matte inside its own maths, under a
    // deeper meaning — the matte is a *depth* pass, and it decides focus rather
    // than strength. It keeps `depth` as its stored id (a save is a save,
    // K-065), so nothing is injected; only its row treatment and prose adopt the
    // uniform shape. Naming the id here is what puts it on the one matte
    // carriage every effect uses, instead of a private list of its own.
    matte = (
        "depth",
        "a depth pass, not a strength: its luma is how far away each pixel is, \
         and the blur widens with the distance from the focus depth — so a \
         mid-grey matte can be perfectly sharp",
    ),
    // K-425: `depth_channel` below is this effect's own channel pick.
    matte_channel = false,
)]
pub struct Dof {
    /// The layer whose depth channel is the depth pass (0 = near, 1 = far by
    /// convention; the effect is symmetric about Focus). Unset until the owner
    /// picks one (a labelled no-op): a depth pass is never the picture itself, so
    /// no `self_default` here (K-288) — though pointing it at this layer is still
    /// allowed, and reads the effect's own input.
    ///
    /// **Always `false` here, by design.** A Layer binding is decided by the
    /// caller — only the render knows which layer was actually rendered — so
    /// `resolve_into_arena` carries no `Value::Layer`, and the depth pass arrives
    /// at the GPU pass as its aux slot instead (K-387). The row exists because the
    /// panel needs it, and whether it is bound reaches [`Dof::packed`] as an
    /// argument.
    ///
    /// **Labelled "Matte", not "Depth layer"** (K-395): every effect's matte row
    /// is one row with one word on it, and an effect that already owned the idea
    /// adopts the shared label rather than keeping a private synonym. The stored
    /// id stays `depth` — a save is a save (K-065) — and the meaning stays the
    /// deeper one this effect declares: focus, not strength.
    #[layer(label = "Matte", self_default = false)]
    pub depth: bool,

    /// Invert the depth pass (d' = 1 − d) before the circle of confusion, swapping
    /// near and far — the owner's "tick to invert the depth" box (Frischluft /
    /// DOF PRO both offer it). Off (default) keeps the historical reading, so old
    /// projects are unchanged. Continuous, so the §1.6 ULP oracle still holds.
    ///
    /// **This IS the uniform pair's Invert** (K-395), so it is labelled "Invert"
    /// and sits beside the picker on the Matte row rather than down in the Depth
    /// map twirl where it used to live. Presentation and prose only: the stored
    /// id is still `depth_invert`.
    #[toggle(label = "Invert", default = false)]
    pub depth_invert: bool,

    // The depth Layer input's sampling mode (K-142) is not a schema parameter:
    // the inspector renders a source combobox beside the Layer picker (None /
    // Masks / Effects and masks) and stores it as a `depth_source` Choice on the
    // instance, read through `EffectInstance::layer_source("depth")`. A project
    // saved with K-125's `depth_after_effects` bool still loads — `layer_source`
    // falls back to it.
    /// The in-focus depth, 0..1. Mid-depth by default so a typical near-to-far
    /// pass has its middle sharp. Greys out while Use focus point is on, because
    /// then the point decides.
    #[slider(
        label = "Focus distance",
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        hard_max = 1.0
    )]
    pub focus: f32,

    /// Focus by clicking the thing you want sharp rather than by hunting for a
    /// number. Off by default: it changes what Focus distance means, and a saved
    /// project must keep meaning what it meant.
    #[toggle(default = false)]
    pub use_focus_point: bool,

    /// Where to read the focus depth, px@comp (K-260: point parameters are
    /// PIXELS, never % of frame). Pairs with `focus_point_y` into one point row
    /// with a crosshair pick (docs/07 §6.1) — the same row the Lens flare's Light
    /// uses, which is why this is a Float pair and not a schema kind of its own.
    /// The schema default is nominal 1080p centre; `instantiate_for_raster`
    /// centres a fresh instance on the actual comp.
    #[slider(min = 0.0, max = 3840.0, default = 960.0, unit = Px)]
    pub focus_point_x: f32,

    /// See [`focus_point_x`](Self::focus_point_x).
    #[slider(min = 0.0, max = 2160.0, default = 540.0, unit = Px)]
    pub focus_point_y: f32,

    /// Half-width of the sharp band around Focus, 0..1: depths within it stay
    /// crisp.
    #[slider(
        label = "Focus range",
        min = 0.0,
        max = 1.0,
        default = 0.1,
        hard_min = 0.0,
        hard_max = 1.0
    )]
    pub range: f32,

    /// The master maximum circle-of-confusion radius, reached at the
    /// farthest-from-focus depth. Scales both per-side radii about its default 8
    /// (unity: `aperture / 8`), so a project saved before Near/Far existed —
    /// which has only this param — renders identically. Clamped at zero below (a
    /// zero master is a passthrough), unbounded typing above the 40 px slider.
    ///
    /// **Declared `Raw`, not `Px`, and that is deliberate.** The number is
    /// authored as px@comp, but it enters the maths as the unitless ratio
    /// `aperture / 8` multiplying the per-side radii — which are the values that
    /// become actual pixels. Exactly one factor of the product may follow the
    /// raster or a half-resolution preview would blur by a quarter of the disc,
    /// and the factor that does is Near/Far below.
    #[slider(min = 0.0, max = 40.0, default = 8.0, hard_min = 0.0)]
    pub aperture: f32,

    /// Per-side circle of confusion for the near side — depths in front of focus
    /// (`d < focus`). px@comp, scaled by the Aperture master. Owner's "adjust
    /// close/far blur separately". Absent on pre-feature projects, where it reads
    /// its default 8 and so falls back to Aperture alone.
    #[slider(
        label = "Near blur",
        min = 0.0,
        max = 40.0,
        default = 8.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub near_aperture: f32,

    /// Per-side circle of confusion for the far side — depths behind focus
    /// (`d >= focus`). px@comp, scaled by the Aperture master. Absent on
    /// pre-feature projects, keeping the old symmetric behaviour.
    #[slider(
        label = "Far blur",
        min = 0.0,
        max = 40.0,
        default = 8.0,
        hard_min = 0.0,
        unit = Px
    )]
    pub far_aperture: f32,

    // ---- The Iris twirl ----
    /// The iris's blade count — the shape a defocused highlight is smeared into.
    /// Inert at Roundness 1 (a circle has no blades), which is why the schema
    /// needs no Circle entry beside it. The ceiling is [`MAX_BLADES`], shared with
    /// the kernel's uniform array; an Int rather than a Choice so a keyframe can
    /// sweep it, stepping 5 → 6 rather than growing half a blade.
    ///
    /// That step is a **floor**, and a floor is not what the arena's generic Int
    /// conversion does (it rounds), so the count the kernel reads is
    /// [`Dof::DERIVED_BLADES`] rather than this row — see
    /// [`DofDef::resolve_derived`].
    #[counter(
        min = 3,
        max = MAX_BLADES as i64,
        default = 6,
        hard_min = 3,
        hard_max = MAX_BLADES as i64
    )]
    pub blades: i32,

    /// Bows the blades. 0 is a straight-edged polygon, 1 is the circle, and
    /// **negative goes concave** — five blades at −1 is a star. 1 by default: that
    /// is the plain disc this effect has always gathered, so the shape controls
    /// cost an existing project nothing until it asks for them.
    #[slider(
        min = -1.0,
        max = 1.0,
        default = 1.0,
        hard_min = -1.0,
        hard_max = 1.0
    )]
    pub roundness: f32,

    /// Turns the iris. Degrees on a dial (docs/07 §6), unbounded, so it winds
    /// through full turns rather than stopping at 360. Inert at Roundness 1, like
    /// Blades.
    #[dial(default = 0.0, step = 15.0)]
    pub rotation: f32,

    /// The aperture's aspect: 0 is round, positive stretches the highlights wide
    /// and negative stretches them tall — the oval an anamorphic scope lens
    /// throws. Not a ratio in the 1.33-or-2.0 sense; it is a squeeze either side
    /// of round, which is why it runs −1…1 rather than upward from 1.
    #[slider(
        label = "Aspect ratio",
        min = -1.0,
        max = 1.0,
        default = 0.0,
        hard_min = -1.0,
        hard_max = 1.0
    )]
    pub aspect: f32,

    /// **Where the light sits inside each ball.** A real lens does not throw a
    /// flat disc: an under-corrected one rings the edge bright (the "soap bubble"
    /// bokeh), an over-corrected one pools the light in the middle (creamy,
    /// smooth). That is spherical aberration, and this is the dial for it —
    /// negative a soft centre, 0 the flat disc a plain gather produces, positive a
    /// bright rim. **Our reading of the curve, not measured against a reference
    /// plugin** — docs/08 §3.22 records that.
    #[slider(
        label = "Rim brightness",
        min = -1.0,
        max = 1.0,
        default = 0.0,
        hard_min = -1.0,
        hard_max = 1.0
    )]
    pub rim: f32,

    // ---- The Highlights twirl ----
    /// The linear level each tap is split at before the power mean: everything
    /// below it averages flat, everything above expands. 1.0 is scene white, so
    /// only genuinely over-range highlights bloom — and with Exposure at 0 this
    /// decides nothing at all, because the split never happens.
    #[slider(
        label = "Highlight threshold",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0
    )]
    pub threshold: f32,

    /// How hard the over-threshold part of each tap blooms, in stops. The
    /// gather's mean becomes a *power* mean, so a small bright area survives being
    /// averaged with its dark surroundings instead of vanishing into it — which is
    /// the whole difference between a blur and a bokeh.
    ///
    /// **0 by default, and 0 is the plain arithmetic mean**: the kernel branches
    /// around the split entirely, so an existing project's blur is untouched to
    /// the bit. Turning this up is what lights the balls.
    ///
    /// The stops-to-power constant is [`Dof::EXPOSURE_STOPS_PER_DOUBLING`], fitted
    /// rather than measured; docs/08 §3.22 records it as open.
    #[slider(
        label = "Highlight exposure",
        min = 0.0,
        max = 30.0,
        default = 0.0,
        hard_min = -30.0,
        hard_max = 30.0
    )]
    pub exposure: f32,

    // ---- The Depth map twirl ----
    /// Which channel of the depth layer carries depth. Red by default — the
    /// channel this effect has always read, and the one a depth pass
    /// conventionally arrives in — but a pass that comes as luminance or in the
    /// alpha is ordinary enough to deserve the pick.
    #[choice(options = *CHANNEL_OPTIONS, default = 0)]
    pub depth_channel: u32,

    /// **The gamma on the depth axis** — the depth distance rescaled before the
    /// ramp, which decides how hard the blur answers to a small change in depth,
    /// and is what stops focus being all-or-nothing on a real depth pass.
    ///
    /// **The range is wide on purpose, and ±1 was not enough.** A real depth pass
    /// rarely spreads its content over 0..1: a linear depth channel puts the sky
    /// or a distant ceiling at 1.0 and compresses an entire room into the bottom
    /// fifth, so the depth *differences* that matter are a tenth of the range or
    /// less. At ±1 this control could only compress the falloff fourfold, which
    /// left such a pass focusing all-or-nothing however it was set — verified on
    /// the owner's own footage through the Focus map view.
    ///
    /// The scale is **one doubling per unit** (`2^profile`), chosen so the whole
    /// slider stays useful: the setting that reads well on a linear depth pass off
    /// game footage lands around 6 (a 64× magnification), which is the middle
    /// rather than the end, and ±10 reaches 1024× for a pass squeezed harder
    /// still. 0 is the neutral multiplier of exactly 1.
    #[slider(
        min = -10.0,
        max = 10.0,
        default = 0.0,
        hard_min = -10.0,
        hard_max = 10.0
    )]
    pub gamma: f32,

    /// Sharp foreground colour bleeding into the defocused background is the
    /// standard artefact of gathering across a depth discontinuity; this pulls
    /// back taps that sit across one AND in front of this pixel. 0 is off, and off
    /// takes the unweighted gather — the arithmetic this effect has always done.
    /// **Our reading**, though the artefact and the family of fixes are well
    /// known.
    #[slider(min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 1.0)]
    pub remove_edge_leak: f32,

    /// How big a depth jump counts as an edge for the row above.
    #[slider(min = 0.0, max = 1.0, default = 0.10, hard_min = 0.0, hard_max = 1.0)]
    pub detect_edge_threshold: f32,

    // ---- Back out of the twirls ----
    /// On by default, which is what this effect has always done: a gather running
    /// off the frame holds the border pixel outward instead of pulling in
    /// transparency, so a bright edge does not darken. Off lets the frame edge
    /// fall away, which is what a flare element over black wants.
    ///
    /// Not the shared EdgesMode enum (P3, K-145) on purpose — that is a three-way
    /// choice, and this is a two-state switch.
    #[toggle(default = true)]
    pub repeat_edge_pixels: bool,

    /// Diagnostic views (the realistic subset the reference plugins ship).
    /// Rendered is the normal blurred output; Depth map shows the post-invert
    /// depth as greyscale — after the channel pick, so it is what the effect is
    /// actually reading; Focus map is the smooth in-focus mask (white where sharp,
    /// darkening out of focus). Every mode is continuous, so the §1.6 ULP oracle
    /// holds across them. Absent on pre-feature projects → Rendered. Forced to
    /// Rendered when no depth is bound: with nothing to show, the views would draw
    /// whatever texture stands in for the depth binding.
    #[choice(options = ["Rendered", "Depth map", "Focus map"], default = 0)]
    pub display: u32,

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

impl Dof {
    /// The blade count the aperture is actually built from — the Blades row
    /// **floored** to an integer and clamped to 3..=[`MAX_BLADES`], which is not
    /// what the arena's generic Int conversion does (it rounds), so it is derived
    /// rather than read (K-385). Never a panel row.
    pub const DERIVED_BLADES: ParamId = ParamId::new("derived.blades");

    /// The stops-to-power constant for [`Dof::exposure`] — **fitted, not
    /// measured**. 6 (the stops-per-stop reading) puts the top of the slider at a
    /// power of 32, which is a maximum filter rather than a mean: it renders
    /// hard-edged flat polygons instead of bokeh discs and erases everything below
    /// the local peak. 12 puts the top at about 5.7 and the point where balls
    /// begin at about 2 — strong, but still an average. docs/08 §3.22 records it
    /// as open; turn it if the onset feels early or late.
    pub const EXPOSURE_STOPS_PER_DOUBLING: f32 = 12.0;

    /// The budget cap on an effective per-side radius, raster pixels (docs/13,
    /// docs/14). The disc gather is O(coc²) taps per pixel, and the Aperture
    /// master MULTIPLIES the per-side radii — so Aperture 150 × Near 55 becomes a
    /// ~1000 px circle of confusion, which submits quadrillions of taps and hangs
    /// the GPU, freezing the preview that renders on the UI thread. Ordinary
    /// apertures (≤ the 40 px slider) sit far below it.
    pub const MAX_APERTURE_PX: f32 = 128.0;

    /// The floored blade count out of a resolved bag: [`Dof::packed`]'s other
    /// missing argument, so no caller has to know the id.
    pub fn blades_of(p: Params<'_>) -> u32 {
        p.int(Self::DERIVED_BLADES, 6).clamp(3, MAX_BLADES as i32) as u32
    }

    /// What the kernel wants (docs/impl/effect-registry.md §2.4), derived exactly
    /// as the old resolve arm derived it. Both render paths read this one method,
    /// so the CPU reference and the WGSL kernel cannot drift apart.
    ///
    /// `depth_bound` is whether the render actually has a depth pass for this op —
    /// the aux slot, which the bag cannot carry. With none, the rows that describe
    /// how to read one are neutralised here rather than in the kernel: the focus
    /// point cannot be picked off a depth that is not there, edge-leak suppression
    /// is dead weight without a second depth per tap (and its neutral is what
    /// keeps the gather bit-identical to the historical one), and the diagnostic
    /// views would otherwise draw whatever texture stands in for the binding.
    ///
    /// `blade_count` is [`Dof::blades_of`]; the two host-side `exp2`s (the tonal
    /// power and the depth profile) are taken here for the reason §1.6 gives —
    /// neither render path then evaluates its own — and both are exactly 1 at
    /// their neutral, which is the branch the kernel takes to stay bit-identical.
    pub fn packed(self, depth_bound: bool, blade_count: u32) -> cpu::DofParams {
        // The master is the unitless ratio; Near/Far carry the length, already
        // through the §2.3 preview factor.
        let master = self.aperture / 8.0;
        let (blade_normals, apothem2) = aperture_blades(blade_count, self.rotation);

        // Deform squeezes one axis and leaves the other alone, so the aperture
        // only ever shrinks inside the circle and the kernel's scan box stays a
        // correct bound. The reciprocal is taken here, not per tap (K-137's
        // host-side single division), and the magnitude is held below 1 so it
        // cannot divide by zero at the range's ends.
        let deform = self.aspect.clamp(-1.0, 1.0);
        let squeeze = 1.0 / (1.0 - deform.abs().min(0.95));
        let aspect_scale = if deform > 0.0 {
            [1.0, squeeze] // a wide oval: pull y in
        } else if deform < 0.0 {
            [squeeze, 1.0]
        } else {
            [1.0, 1.0]
        };

        cpu::DofParams {
            focus: self.focus.clamp(0.0, 1.0),
            range: self.range.clamp(0.0, 1.0),
            near_aperture: (self.near_aperture * master).clamp(0.0, Self::MAX_APERTURE_PX),
            far_aperture: (self.far_aperture * master).clamp(0.0, Self::MAX_APERTURE_PX),
            blade_normals,
            blade_count,
            apothem2,
            // 1 is the circle, and the circle is what this effect has always
            // gathered — so it is the default and the kernel's fast path.
            roundness: self.roundness.clamp(-1.0, 1.0),
            rim: self.rim.clamp(-1.0, 1.0),
            aspect_scale,
            threshold: self.threshold.max(0.0),
            bokeh_power: (self.exposure.clamp(-30.0, 30.0) / Self::EXPOSURE_STOPS_PER_DOUBLING)
                .exp2(),
            repeat_edge: self.repeat_edge_pixels,
            depth_channel: self.depth_channel.min(CHANNEL_OPTIONS.len() as u32 - 1),
            depth_invert: self.depth_invert,
            use_focus_point: depth_bound && self.use_focus_point,
            focus_point: [self.focus_point_x, self.focus_point_y],
            gamma: self.gamma.clamp(-10.0, 10.0).exp2(),
            remove_edge_leak: if depth_bound {
                self.remove_edge_leak.clamp(0.0, 1.0)
            } else {
                0.0
            },
            detect_edge_threshold: self.detect_edge_threshold.clamp(0.0, 1.0),
            display: if depth_bound { self.display.min(2) } else { 0 },
            mix: (self.mix / 100.0).clamp(0.0, 1.0),
        }
    }
}

/// Depth of field's behaviour: no CPU reference through the single-image
/// dispatcher (the depth pass is a second picture), so `apply_cpu` keeps its
/// identity default — the passthrough the old `Resolved::Dof` arm was.
pub struct DofDef;

impl EffectDef for DofDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Dof as EffectMetadata>::SCHEMA
    }

    /// The blade count, **floored** — the one number the arena's generic Int
    /// conversion would get differently, since it rounds (K-385).
    ///
    /// A pentagon does not interpolate into a hexagon, so a keyframe sweeping
    /// 5 → 6 has to step; the floor is where that truth was enforced, and moving
    /// the step from 6.0 to 5.5 would change what an existing project renders
    /// mid-sweep. So it is derived here rather than read off the row.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let blades = cx
            .inst
            .float_at_with_context("blades", cx.lt, cx.context.clone())
            .unwrap_or(6.0) as f32;
        push(
            Dof::DERIVED_BLADES,
            Value::Int(blades.floor().clamp(3.0, MAX_BLADES as f32) as i32),
        );
    }
}
