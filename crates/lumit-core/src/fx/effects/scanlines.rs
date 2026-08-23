//! Scanlines (docs/08 §3.12): the pointwise periodic darken of a CRT line
//! pattern, rolling if asked.
//!
//! **In plain terms.** Two of the three numbers the kernel wants are not
//! controls. The roll offset is *where the pattern has scrolled to by now*, a
//! product of the roll speed, the clock and the line period — and the darkening
//! strength has a migration folded into it, because a project saved before
//! FX-13/K-147 carries a separate Darkness dial that no longer exists as a row.
//! Both are worked out at resolve time through the one hook that sees the clock
//! and the stored instance ([`EffectDef::resolve_derived`], K-385) and handed to
//! the kernel as plain numbers, exactly as the hand-written resolve arm handed
//! them over before.

use crate::fx::{cpu, EffectDef, EffectMetadata, EffectSchema, ParamId, Params, ResolveCx, Value};
use lumit_fx_macros::Effect;

/// Scanlines' controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "scanlines",
    label = "Scanlines",
    version = 2,
    category = Distortion,
    // No hash, no seed: a pointwise darken read straight from the input pixel,
    // never a neighbour, so the region of interest is exact.
    cost = Cheap,
    roi = Exact,
    // K-427: the matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "widens Line period per pixel: white keeps the set period, grey spreads \
         the lines apart, black spreads them too far to see",
    ),
)]
pub struct Scanlines {
    /// The single dial (FX-13, K-147): 0..1 is how dark the dark lines get — 0
    /// is the bit-exact passthrough (pinned by test), 1 takes them to black.
    /// Collapses the old Intensity × Darkness pair into one control; an old
    /// project's Darkness folds into it at resolve, which is why the number the
    /// kernel reads is [`Scanlines::DERIVED_INTENSITY`] rather than this row.
    #[slider(min = 0.0, max = 1.0, default = 0.35, hard_min = 0.0, hard_max = 1.0)]
    pub intensity: f32,

    /// px@comp: the deliberately pixel-scale scanline pitch. Declared `Px`, so
    /// the resolve step scales it by the §2.3 preview factor and
    /// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size —
    /// which the old `rescale_px` did *not* do for this effect, so a scanlined
    /// adjustment layer under a reduced-resolution preview kept comp-sized lines
    /// while every other spatial value shrank (the K-266 shape). The unit states
    /// it once and the generic pass cannot forget it.
    #[slider(
        label = "Line period",
        min = 1.0,
        max = 20.0,
        default = 3.0,
        hard_min = 1.0,
        unit = Px
    )]
    pub scanline_period: f32,

    /// Lines (periods) per second; either direction (K-090). What it *produces*
    /// — the pattern's pixel offset this frame — is
    /// [`Scanlines::DERIVED_ROLL_PX`].
    #[slider(label = "Roll speed", min = -30.0, max = 30.0, default = 0.0)]
    pub scanline_roll: f32,

    /// Alternates which half of each period darkens on odd periods: the classic
    /// interlaced-field look.
    #[toggle(label = "Interlace offset", default = false)]
    pub scanline_interlace: bool,

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

impl Scanlines {
    /// The darkening strength this instance actually renders with, 0..1 — the
    /// Intensity row with an old project's Darkness folded in (K-385). Never a
    /// panel row: it is what the stored parameters *produce*.
    pub const DERIVED_INTENSITY: ParamId = ParamId::new("derived.intensity");

    /// The pattern's pixel offset at this frame (roll speed × layer time × line
    /// period), so the kernel never sees raw time or does its own time maths
    /// (§2.4). Never a panel row.
    ///
    /// **Known ceiling** (docs/impl/effect-registry.md §2.4a, and the open item
    /// in docs/TODO.md): this is in raster pixels, and a derived id carries no
    /// declared unit, so [`ResolvedStack::rescale_spatial`](crate::fx::
    /// ResolvedStack::rescale_spatial) leaves it behind while it moves
    /// `scanline_period`. A stack resolved against one raster and reused at
    /// another therefore rolls to the wrong phase. Only reachable with a
    /// non-zero Roll speed on a precomp realised at a second size; the fix is a
    /// decision (teach the rescale pass about derived units, or derive the roll
    /// in periods rather than pixels), so it is not taken here.
    pub const DERIVED_ROLL_PX: ParamId = ParamId::new("derived.roll_px");

    /// The intensity and roll offset out of a resolved bag: [`Scanlines::
    /// packed`]'s two missing arguments, so no caller has to know the ids.
    pub fn derived_of(p: Params<'_>) -> (f32, f32) {
        (
            p.float(Self::DERIVED_INTENSITY, 0.0),
            p.float(Self::DERIVED_ROLL_PX, 0.0),
        )
    }

    /// What the kernel wants (docs/impl/effect-registry.md §2.4). `intensity`
    /// and `roll_px` come from the bag rather than from declared rows because
    /// they are functions of the clock and of a parameter that no longer exists
    /// — [`ScanlinesDef::resolve_derived`] computed them. The period floors at
    /// one pixel, as the old arm floored it, so a degenerate pitch can never
    /// divide the kernel by zero. Both render paths read this one method, so the
    /// CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self, intensity: f32, roll_px: f32) -> (f32, f32, f32, bool, f32) {
        (
            intensity,
            self.scanline_period.max(1.0),
            roll_px,
            self.scanline_interlace,
            (self.mix / 100.0).clamp(0.0, 1.0),
        )
    }
}

/// Scanlines' behaviour.
pub struct ScanlinesDef;

impl EffectDef for ScanlinesDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Scanlines as EffectMetadata>::SCHEMA
    }

    /// The folded intensity and the rolled offset — the whole of what the old
    /// resolve arm did beyond reading its rows, moved unchanged (K-385).
    ///
    /// The fold: an old project also carried a separate Darkness parameter
    /// (0..100), which is **not** a schema row and so cannot come out of the
    /// bag; read from the instance here, the loaded look is the old Intensity ×
    /// Darkness product exactly. A new project has no Darkness parameter, so the
    /// raw Intensity stands.
    ///
    /// The roll: `roll_speed × lt × period_px`, every step in `f64` and the
    /// period floored to a pixel *before* the product, exactly as the arm
    /// ordered it — f32 time would round differently near a tick boundary than
    /// f64 does, which is why the offset is precomputed rather than left to the
    /// kernel (§2.4).
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let (e, lt) = (cx.inst, cx.lt);
        let fl = |id: &str| e.float_at_with_context(id, lt, cx.context.clone());
        let raw = fl("intensity").unwrap_or(0.35);
        let folded = match fl("scanline_darkness") {
            Some(darkness_pct) => raw * (darkness_pct / 100.0),
            None => raw,
        };
        push(
            Scanlines::DERIVED_INTENSITY,
            Value::Float((folded as f32).clamp(0.0, 1.0)),
        );
        let period_px = (fl("scanline_period").unwrap_or(3.0) as f32 * cx.px_scale).max(1.0);
        let roll_speed = fl("scanline_roll").unwrap_or(0.0);
        push(
            Scanlines::DERIVED_ROLL_PX,
            Value::Float((roll_speed * lt * f64::from(period_px)) as f32),
        );
    }

    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        let (i, r) = Scanlines::derived_of(p);
        let (intensity, period_px, roll_px, interlace, mix) = Scanlines::read(p).packed(i, r);
        cpu::scanlines(rgba, w, h, intensity, period_px, roll_px, interlace, mix);
    }
}
