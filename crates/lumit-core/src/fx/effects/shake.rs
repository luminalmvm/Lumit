//! Shake (docs/08 §3.4, FX-11/K-146): a seeded camera wobble, resampled once
//! through the Transform kernel — a transform-domain effect, never pixel noise.
//!
//! **In plain terms.** The wobble is a curve through seeded noise, read at
//! "layer time × frequency". Two things about it will not fit in a slider, and
//! K-388 is where each of them went:
//!
//! - **The noise itself** is a function of the clock and of a 64-bit integer
//!   lattice the GPU has not got, so it is sampled here, at resolve time, and
//!   handed over as plain numbers ([`EffectDef::resolve_derived`], K-385). It is
//!   pushed **unit-free** — the raw −1..1 wobble, with no amplitude in it —
//!   because a derived value carries no declared unit and so would never rescale
//!   when a stack is reused at another raster size. The amplitude stays a
//!   declared `Px` row, which does rescale, and the two are multiplied
//!   together at dispatch in [`Shake::packed`].
//! - **Its own motion blur** (T18, K-165) needs the same wobble at nine
//!   sub-frame placements across the shutter. Each is four floats under one id
//!   ([`Value::Vec4`]), which is what that kind exists for: forty flat
//!   `derived.*` entries would have drowned the bag.
//!
//! [`Shake::packed`] reassembles [`ShakeWobble::at`](crate::fx::ShakeWobble::at)
//! step for step — same association, same cast points — so a shake that has not
//! been touched renders the bits it always did.

use crate::fx::{
    cpu, shake_affine, shake_mb_offsets, shake_noise, transform_op, EdgesMode, EffectDef,
    EffectMetadata, EffectSchema, ParamGroup, ParamId, Params, ResolveCx, ShakeSample, Value,
    SHAKE_MB_SAMPLES,
};
use crate::model::EffectValue;
use lumit_fx_macros::Effect;

/// Shake's twirls (P4): the per-axis wobble (FX-11, K-146) — the master
/// Amplitude/Frequency drive x and y together while this group biases each axis
/// and adds the z (depth/scale) shake that replaced the old Zoom pump — and the
/// Motion blur group (T18, K-165), the shake's own inter-frame smear (toggle +
/// amount). Each group's ids are a contiguous run of the schema's `params`.
pub const SHAKE_GROUPS: &[ParamGroup] = &[
    ParamGroup {
        label: "Per-axis wobble",
        params: &["x_amp", "x_freq", "y_amp", "y_freq", "z_amp", "z_freq"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Motion blur",
        params: &["motion_blur", "mb_amount"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
];

/// Shake's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "shake",
    label = "Shake",
    version = 1,
    category = Distortion,
    cost = Cheap,
    roi = FullFrame,
    // §1.3: its pixels are a function of time under constant parameters, which
    // the frame key reads (lumit-eval).
    seeded = true,
    groups = SHAKE_GROUPS,
    // K-427: the matte scales the displacement, inside the kernel (the
    // owner's rule for mattes); the generic strength dissolve does not also run.
    matte = (
        "matte",
        "scales the shake's displacement per pixel, read where the pixel lands: \
         white moves it the full Amplitude and Rotation amount, grey less, \
         black leaves it still, so a soft matte turns the shove into a warp",
    ),
)]
pub struct Shake {
    /// px@comp (§2.3): how far the wobble roams. Declared `Px`, so the resolve
    /// step scales it to the raster in play and
    /// [`ResolvedStack::rescale_spatial`](crate::fx::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size.
    /// The old arm scaled the *resolved offsets* by hand; declaring the unit
    /// does it one multiply earlier (K-388).
    #[slider(
        min = 0.0,
        max = 400.0,
        default = 30.0,
        hard_min = 0.0,
        hard_max = 2000.0,
        unit = Px
    )]
    pub amplitude: f32,

    /// Hz — how fast the wobble wanders; the noise samples at local time ×
    /// frequency. Unbounded above (K-090): any positive rate is meaningful,
    /// sampling handles it. Read by [`ShakeDef::resolve_derived`] rather than by
    /// [`Shake::packed`], because what it produces is the noise sample itself.
    #[slider(min = 0.1, max = 30.0, default = 8.0, hard_min = 0.0, unit = Raw)]
    pub frequency: f32,

    /// Degrees of twist wobble either way.
    #[slider(
        label = "Rotation amount",
        min = 0.0,
        max = 45.0,
        default = 1.0,
        hard_min = 0.0,
        hard_max = 360.0,
        unit = Degrees
    )]
    pub rotation: f32,

    /// × the master Frequency, for the twist alone (K-541). The twist used to
    /// be the one axis with an amount but no rate of its own, so a slow drift
    /// with a fast shudder in it — the handheld look — could not be dialled:
    /// raising the master Frequency sped the twist up with everything else.
    /// Defaults to 1, which multiplies the noise base by exactly one and so
    /// leaves every shake made before this row bit-for-bit itself.
    #[slider(
        label = "Rotation frequency",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub rot_freq: f32,

    /// × the master Amplitude (0 stills this axis).
    #[slider(
        label = "X amount",
        min = 0.0,
        max = 2.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub x_amp: f32,

    /// × the master Frequency. See [`frequency`](Self::frequency) for where it
    /// is read.
    #[slider(
        label = "X frequency",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub x_freq: f32,

    /// See [`x_amp`](Self::x_amp).
    #[slider(
        label = "Y amount",
        min = 0.0,
        max = 2.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub y_amp: f32,

    /// See [`x_freq`](Self::x_freq).
    #[slider(
        label = "Y frequency",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub y_freq: f32,

    /// Depth/scale shake, % of scale wobble about natural size — the old Zoom
    /// pump's range and meaning (§3.4). What the kernel reads is
    /// [`Shake::DERIVED_Z_AMP`], not this row: an old project carries the value
    /// under `zoom_pump` instead, and that fold is a resolve-time job.
    #[slider(
        label = "Z amount",
        min = 0.0,
        max = 20.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub z_amp: f32,

    /// See [`x_freq`](Self::x_freq).
    #[slider(
        label = "Z frequency",
        min = 0.0,
        max = 4.0,
        default = 1.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub z_freq: f32,

    /// The shake's own motion blur (T18, K-165), smeared along the wobble's
    /// inter-frame movement and applied to this effect alone.
    #[toggle(label = "Motion blur", default = false)]
    pub motion_blur: bool,

    /// 0..1 shutter fraction: how far across the shutter window the wobble is
    /// sampled and averaged. 0 is the plain shake (no smear); motion blur off is
    /// the same bit-exact single resample.
    #[slider(
        label = "Shutter",
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub mb_amount: f32,

    /// How the resample treats the border the wobble reveals (P3, K-145).
    /// Default Mirror (owner, 2026-07-19; was Repeat): the reflected border
    /// reads more naturally under the shake's own motion blur than a smeared
    /// repeat edge. What the kernel reads is [`Shake::DERIVED_EDGE`], not this
    /// row: a project saved before FX-11 carries an `auto_scale` bool instead.
    #[choice(label = "Edges", options = *crate::fx::EDGE_OPTIONS, default = 2)]
    pub edge: u32,

    /// Per instance (§1.3), so two shakes on two layers do not wobble in step.
    #[seed]
    pub seed: u32,

    /// The host-uniform Mix every effect ends with (docs/08 §1.5), per cent.
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

/// What this frame's wobble is built from that no slider holds (K-388): the
/// unit-free noise, and the two rows whose stored form an old project spells
/// differently. All of it comes out of the bag through [`Shake::derived_of`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeDerived {
    /// The frame's noise sample, `(x, y, rotation, z)`, each −1..1.
    pub noise: [f32; 4],
    /// The same, at each motion-blur sub-frame across the shutter — `Some` only
    /// when the smear is on, which is the one place that decision is taken.
    pub mb_noise: Option<[[f32; 4]; SHAKE_MB_SAMPLES]>,
    /// Depth (z) scale-pump magnitude, 0..1: the Z amount row as a fraction,
    /// or an old project's `zoom_pump`.
    pub z_amp: f32,
    /// The [`EdgesMode`] wire code, or an old project's `auto_scale` migrated.
    pub edge: u32,
}

/// Which dispatch a shake draws through, and the numbers that dispatch wants
/// (docs/impl/effect-registry.md §2.4).
///
/// **Why `packed` returns this rather than a tuple.** The shake's own motion
/// blur is not a dial but a different pass — the averaging kernel, fed one
/// affine per sub-frame — exactly as RGB split's Wavelength is (K-090). One enum
/// keeps the fork in one place, so the CPU reference and the GPU wrapper cannot
/// disagree about which one an instance is in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shaken {
    /// The plain single resample through the Transform kernel.
    Plain {
        /// This frame's wobble.
        wobble: ShakeSample,
        /// [`EdgesMode`] wire code for the revealed border.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    /// The shake's own motion blur (T18, K-165): the wobble at
    /// [`SHAKE_MB_SAMPLES`] sub-frame placements, resampled and averaged in
    /// premultiplied linear space. The centre sample is the frame itself.
    Blurred {
        /// One wobble per sub-frame, in shutter order.
        samples: [ShakeSample; SHAKE_MB_SAMPLES],
        /// [`EdgesMode`] wire code for the revealed border.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
}

impl Shake {
    /// This frame's unit-free noise sample, `(x, y, rotation, z)`. Never a panel
    /// row: it is what the seed, the frequencies and the clock produce.
    pub const DERIVED_NOISE: ParamId = ParamId::new("derived.noise");

    /// The same at each motion-blur sub-frame, present only when the smear is
    /// on. Fixed ids rather than a counted list, so the bag stays a flat
    /// key/value set.
    pub const DERIVED_MB_NOISE: [ParamId; SHAKE_MB_SAMPLES] = [
        ParamId::new("derived.mb_noise_0"),
        ParamId::new("derived.mb_noise_1"),
        ParamId::new("derived.mb_noise_2"),
        ParamId::new("derived.mb_noise_3"),
        ParamId::new("derived.mb_noise_4"),
        ParamId::new("derived.mb_noise_5"),
        ParamId::new("derived.mb_noise_6"),
        ParamId::new("derived.mb_noise_7"),
        ParamId::new("derived.mb_noise_8"),
    ];

    /// The depth (z) pump as a 0..1 magnitude, with an old project's
    /// `zoom_pump` folded in (K-146).
    pub const DERIVED_Z_AMP: ParamId = ParamId::new("derived.z_amp");

    /// The edge policy, with an old project's `auto_scale` bool folded in
    /// (K-145/K-146).
    pub const DERIVED_EDGE: ParamId = ParamId::new("derived.edge");

    /// Everything [`Shake::packed`] needs that is not a declared row, out of a
    /// resolved bag — so no caller has to know the ids.
    ///
    /// The motion-blur set is read by *presence*: [`ShakeDef::resolve_derived`]
    /// pushes it only when the toggle is on and the shutter is non-zero, which
    /// is the one place that decision is taken (the old arm took it in `f64`, on
    /// the stored value, and it still does).
    pub fn derived_of(p: Params<'_>) -> ShakeDerived {
        ShakeDerived {
            noise: p.vec4(Self::DERIVED_NOISE, [0.0; 4]),
            mb_noise: p
                .get(Self::DERIVED_MB_NOISE[0])
                .is_some()
                .then(|| Self::DERIVED_MB_NOISE.map(|id| p.vec4(id, [0.0; 4]))),
            z_amp: p.float(Self::DERIVED_Z_AMP, 0.0),
            edge: p.choice(Self::DERIVED_EDGE, EdgesMode::Repeat.code()),
        }
    }

    /// Which dispatch to run and what to hand it (docs/impl/effect-registry.md
    /// §2.4).
    ///
    /// The wobble is [`ShakeWobble::at`](crate::fx::ShakeWobble::at) reassembled
    /// from the pieces: `amp_px · axis amount · noise` for the offsets, `rotation
    /// amount · noise` for the twist, `1 + z · noise` for the zoom — the same
    /// association and the same `f64 → f32` cast points, so an untouched shake
    /// is bit-for-bit itself (K-388). `amplitude` arrives already converted from
    /// % diagonal by the resolve step, so this only floors it, as the old arm
    /// floored the same product. Both render paths read this one method, so the
    /// CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self, d: ShakeDerived) -> Shaken {
        let amp_px = self.amplitude.max(0.0);
        let x_amp = self.x_amp.max(0.0);
        let y_amp = self.y_amp.max(0.0);
        let rot_amount = self.rotation.max(0.0);
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        let at = |n: [f32; 4]| ShakeSample {
            offset_px: [amp_px * x_amp * n[0], amp_px * y_amp * n[1]],
            rotation_deg: rot_amount * n[2],
            zoom: 1.0 + d.z_amp * n[3],
        };
        match d.mb_noise {
            Some(noise) => Shaken::Blurred {
                samples: noise.map(at),
                edge: d.edge,
                mix,
            },
            None => Shaken::Plain {
                wobble: at(d.noise),
                edge: d.edge,
                mix,
            },
        }
    }
}

/// Shake's behaviour.
pub struct ShakeDef;

impl EffectDef for ShakeDef {
    fn schema(&self) -> &'static EffectSchema {
        &<Shake as EffectMetadata>::SCHEMA
    }

    /// The noise, and the two legacy folds — the whole of what the old resolve
    /// arm did beyond reading its rows, moved unchanged (K-385, K-388).
    ///
    /// Everything here that feeds the noise stays `f64` to the last step: layer
    /// time, the master frequency and the per-axis multipliers, exactly as the
    /// arm had them. Only the sample itself narrows to `f32`, at the point
    /// [`ShakeWobble::at`](crate::fx::ShakeWobble::at) narrowed it.
    fn resolve_derived(&self, cx: &ResolveCx<'_>, push: &mut dyn FnMut(ParamId, Value)) {
        let (e, lt) = (cx.inst, cx.lt);
        let fl = |id: &str| e.float_at_with_context(id, lt, cx.context.clone());
        let freq = fl("frequency").unwrap_or(8.0).max(0.0);
        let rot_freq = fl("rot_freq").unwrap_or(1.0).max(0.0);
        let x_freq = fl("x_freq").unwrap_or(1.0).max(0.0);
        let y_freq = fl("y_freq").unwrap_or(1.0).max(0.0);
        let z_freq = fl("z_freq").unwrap_or(1.0).max(0.0);
        let seed = match e.param("seed") {
            Some(EffectValue::Seed(s)) => *s,
            _ => 0,
        };
        // Independent noise channels sampled at local time × frequency (per
        // axis, §3.4) — deterministic, hop-free, identical on every machine
        // (§2.4). One sampler drives the frame-time wobble and the motion-blur
        // sub-frames, so they agree bit-for-bit.
        let base = lt * freq;
        let noise = |b: f64| {
            Value::Vec4([
                shake_noise(seed, 0, b * x_freq) as f32,
                shake_noise(seed, 1, b * y_freq) as f32,
                shake_noise(seed, 2, b * rot_freq) as f32,
                shake_noise(seed, 3, b * z_freq) as f32,
            ])
        };
        push(Shake::DERIVED_NOISE, noise(base));

        // z (depth/scale) amount: the new id, else the old `zoom_pump` (a
        // project saved before FX-11 keeps its pump), a scale-pump per cent
        // either way.
        let z_pct = fl("z_amp").or_else(|| fl("zoom_pump")).unwrap_or(0.0) as f32;
        push(
            Shake::DERIVED_Z_AMP,
            Value::Float((z_pct / 100.0).clamp(0.0, 1.0)),
        );

        // Edges (P3, K-145): the stored Choice, else the old Auto-scale bool
        // (on → Repeat hides the border as the cover once did; off →
        // Transparent), else Repeat.
        let edge = match e.param("edge") {
            Some(EffectValue::Choice(c)) => {
                EdgesMode::from_code((*c).min(2)).unwrap_or(EdgesMode::Repeat)
            }
            _ => match e.param("auto_scale") {
                Some(EffectValue::Bool(false)) => EdgesMode::Transparent,
                _ => EdgesMode::Repeat,
            },
        };
        push(Shake::DERIVED_EDGE, Value::Choice(edge.code()));

        // The shake's own motion blur (T18, K-165): when the toggle is on and
        // the amount is non-zero, sample the wobble across the shutter for the
        // dispatch to average; off is the plain single resample (the bit-exact
        // passthrough). The centre offset is 0, so the middle sample is the
        // frame-time wobble exactly.
        let motion_blur = e.bool_of("motion_blur").unwrap_or(false);
        let mb_amount = fl("mb_amount").unwrap_or(0.5);
        if motion_blur && mb_amount > 0.0 {
            for (id, db) in Shake::DERIVED_MB_NOISE
                .iter()
                .zip(shake_mb_offsets(mb_amount))
            {
                push(*id, noise(base + db));
            }
        }
    }

    /// Shake is a transform-domain effect (docs/08 §3.4): the wobble maps to the
    /// Transform reference through the shared [`shake_affine`], so the CPU and
    /// GPU paths consume bit-identical numbers. A neutral wobble maps to the
    /// identity affine — the bit-exact passthrough the Transform reference pins.
    fn apply_cpu(&self, rgba: &mut [f32], w: u32, h: u32, p: Params<'_>) {
        match Shake::read(p).packed(Shake::derived_of(p)) {
            Shaken::Plain { wobble, edge, mix } => {
                let (anchor, position, scale, rot) =
                    shake_affine(w, h, wobble.offset_px, wobble.rotation_deg, wobble.zoom);
                cpu::transform(rgba, w, h, anchor, position, scale, rot, edge, 1.0, mix);
            }
            Shaken::Blurred { samples, edge, mix } => {
                let mut ops = [([1.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0]); SHAKE_MB_SAMPLES];
                for (op, s) in ops.iter_mut().zip(samples.iter()) {
                    let (anchor, position, scale, rot) =
                        shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                    let (m, o, _opacity) = transform_op(anchor, position, scale, rot, 1.0);
                    *op = (m, o);
                }
                cpu::transform_average(rgba, w, h, &ops, edge, mix);
            }
        }
    }
}
