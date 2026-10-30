//! Shake (docs/08 §3.4, FX-11): a seeded camera wobble, resampled once
//! through the Transform kernel — a transform-domain effect, never pixel noise.
//!
//! **In plain terms.** The wobble is a curve through seeded noise, read at
//! "layer time × frequency". Two things about it will not fit in a slider, and
//! here is where each of them went:
//!
//! - **The noise itself** is a function of the clock and of a 64-bit integer
//!   lattice the GPU has not got, so it is sampled here, at resolve time, and
//!   handed over as plain numbers ([`EffectDef::resolve_derived`]). It is
//!   pushed **unit-free** — the raw −1..1 wobble, with no amplitude in it —
//!   because a derived value carries no declared unit and so would never rescale
//!   when a stack is reused at another raster size. The amplitude stays a
//!   declared `Px` row, which does rescale, and the two are multiplied
//!   together at dispatch in [`Shake::packed`].
//! - **Its own motion blur** (T18) needs the same wobble at every
//!   sub-frame placement across the shutter, as many as the Samples row asks
//!   for. Each is four floats under one id ([`Value::Vec4`]), which is what
//!   that kind exists for: four flat `derived.*` entries per sub-frame would
//!   have drowned the bag.
//!
//! [`Shake::packed`] reassembles [`ShakeWobble::at`](crate::fx::maths::ShakeWobble::at)
//! step for step — same association, same cast points — so a shake that has not
//! been touched renders the bits it always did.

use crate::fx::{
    cpu, maths::shake_affine, maths::shake_jump_noise, maths::shake_mb_offsets, maths::shake_noise,
    maths::shake_twitch_gate, maths::transform_op, maths::ShakeSample, maths::NO_SKEW,
    maths::SHAKE_MB_DEFAULT_SAMPLES, maths::SHAKE_MB_SAMPLES, params::ParamId, params::Params,
    params::Value, registry::EffectDef, registry::EffectMetadata, registry::ResolveCx,
    schema::EdgesMode, schema::EffectSchema, schema::ParamGroup,
};
use crate::model::EffectValue;
use lumit_fx_macros::Effect;

/// `derived.mb_noise_<n>` for each listed n, as one const array.
macro_rules! mb_noise_ids {
    ($($n:literal),* $(,)?) => {
        [$(ParamId::new(concat!("derived.mb_noise_", $n))),*]
    };
}

/// Shake's twirls (P4): the per-axis wobble (FX-11) — the master
/// Amplitude/Frequency drive x and y together while this group biases each axis
/// and adds the z (depth/scale) shake that replaced the old Zoom pump — and the
/// Motion blur group (T18), the shake's own inter-frame smear (toggle,
/// shutter, samples). Each group's ids are a contiguous run of the schema's
/// `params`.
pub const SHAKE_GROUPS: &[ParamGroup] = &[
    // Each Style's own rows, shown only while that style is picked.
    ParamGroup {
        label: "",
        params: &["stillness", "twitch_freq"],
        collapsed: false,
        visible_when: Some(("style", &[1])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "",
        params: &["drift", "centre_bias"],
        collapsed: false,
        visible_when: Some(("style", &[2])),
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Per-axis wobble",
        params: &["x_amp", "x_freq", "y_amp", "y_freq", "z_amp", "z_freq"],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Wave",
        params: &[
            "x_wave",
            "x_wave_freq",
            "x_wave_phase",
            "y_wave",
            "y_wave_freq",
            "y_wave_phase",
            "z_wave",
            "z_wave_freq",
            "z_wave_phase",
            "rot_wave",
            "rot_wave_freq",
            "rot_wave_phase",
        ],
        collapsed: true,
        visible_when: None,
        visible_when_lens_elements: None,
    },
    ParamGroup {
        label: "Motion blur",
        params: &["motion_blur", "mb_amount", "mb_samples"],
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
    // The matte scales the displacement, inside the kernel (the owner's rule
    // for mattes); the generic strength dissolve does not also run.
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
    /// [`ResolvedStack::rescale_spatial`](crate::fx::params::ResolvedStack::
    /// rescale_spatial) moves it again if the stack is reused at another size.
    /// The old arm scaled the *resolved offsets* by hand; declaring the unit
    /// does it one multiply earlier.
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
    /// frequency. Unbounded above: any positive rate is meaningful,
    /// sampling handles it. Read by [`ShakeDef::resolve_derived`] rather than by
    /// [`Shake::packed`], because what it produces is the noise sample itself.
    #[slider(min = 0.1, max = 30.0, default = 8.0, hard_min = 0.0, unit = Raw)]
    pub frequency: f32,

    /// Seconds the whole shake is shifted in time, so two layers with the
    /// same seed can shake out of step.
    #[slider(min = -5.0, max = 5.0, default = 0.0, unit = Seconds)]
    pub phase: f32,

    /// How the wobble moves over time. Normal is the smooth wobble every shake
    /// had before this row. Twitchy holds still between bursts of it, and
    /// Jumpy jumps to a new place at the Frequency's rate.
    #[choice(options = ["Normal", "Twitchy", "Jumpy"], default = 0)]
    pub style: u32,

    /// Twitchy: about how much of the time the picture is held still.
    #[slider(min = 0.0, max = 1.0, default = 0.7, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub stillness: f32,

    /// Twitchy: Hz, how often a burst can start or stop.
    #[slider(
        label = "Twitch frequency",
        min = 0.0,
        max = 10.0,
        default = 2.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub twitch_freq: f32,

    /// Jumpy: how far the picture creeps towards its next place before it
    /// jumps there. 0 holds still between jumps, 1 drifts the whole way.
    #[slider(min = 0.0, max = 1.0, default = 0.3, hard_min = 0.0, hard_max = 1.0, unit = Raw)]
    pub drift: f32,

    /// Jumpy: the chance that a jump lands back at the centre rather than
    /// somewhere new. 0 never does, 1 always does.
    #[slider(
        label = "Centre bias",
        min = 0.0,
        max = 1.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 1.0,
        unit = Raw
    )]
    pub centre_bias: f32,

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

    /// × the master Frequency, for the twist alone. The twist used to
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

    /// A steady back and forth on top of the random wobble, × the master
    /// Amplitude like X amount.
    #[slider(
        label = "X wave",
        min = 0.0,
        max = 2.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub x_wave: f32,

    /// × the master Frequency, like X frequency: at the defaults the X wave
    /// swings four times a second.
    #[slider(
        label = "X wave frequency",
        min = 0.0,
        max = 4.0,
        default = 0.5,
        hard_min = 0.0,
        unit = Raw
    )]
    pub x_wave_freq: f32,

    /// Where in its swing the X wave starts. A quarter turn between X
    /// and Y sends the picture round in a circle.
    #[slider(label = "X wave phase", min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub x_wave_phase: f32,

    /// See [`x_wave`](Self::x_wave).
    #[slider(
        label = "Y wave",
        min = 0.0,
        max = 2.0,
        default = 0.0,
        hard_min = 0.0,
        unit = Raw
    )]
    pub y_wave: f32,

    /// See [`x_wave_freq`](Self::x_wave_freq).
    #[slider(
        label = "Y wave frequency",
        min = 0.0,
        max = 4.0,
        default = 0.5,
        hard_min = 0.0,
        unit = Raw
    )]
    pub y_wave_freq: f32,

    /// See [`x_wave_phase`](Self::x_wave_phase).
    #[slider(label = "Y wave phase", min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub y_wave_phase: f32,

    /// A steady pulse in scale, % about natural size like Z amount.
    #[slider(
        label = "Z wave",
        min = 0.0,
        max = 20.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub z_wave: f32,

    /// See [`x_wave_freq`](Self::x_wave_freq).
    #[slider(
        label = "Z wave frequency",
        min = 0.0,
        max = 4.0,
        default = 0.5,
        hard_min = 0.0,
        unit = Raw
    )]
    pub z_wave_freq: f32,

    /// See [`x_wave_phase`](Self::x_wave_phase).
    #[slider(label = "Z wave phase", min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub z_wave_phase: f32,

    /// Degrees of steady twist either way.
    #[slider(
        label = "Rotation wave",
        min = 0.0,
        max = 45.0,
        default = 0.0,
        hard_min = 0.0,
        hard_max = 360.0,
        unit = Degrees
    )]
    pub rot_wave: f32,

    /// See [`x_wave_freq`](Self::x_wave_freq).
    #[slider(
        label = "Rotation wave frequency",
        min = 0.0,
        max = 4.0,
        default = 0.5,
        hard_min = 0.0,
        unit = Raw
    )]
    pub rot_wave_freq: f32,

    /// See [`x_wave_phase`](Self::x_wave_phase).
    #[slider(label = "Rotation wave phase", min = 0.0, max = 360.0, default = 0.0, unit = Degrees)]
    pub rot_wave_phase: f32,

    /// The shake's own motion blur (T18), smeared along the wobble's
    /// inter-frame movement and applied to this effect alone.
    #[toggle(label = "Motion blur", default = false)]
    pub motion_blur: bool,

    /// Shutter fraction: how far across the shutter window the wobble is
    /// sampled and averaged. 0 is the plain shake (no smear); motion blur off is
    /// the same bit-exact single resample. Typed past 1 for a longer smear,
    /// like S_Shake's Mo Blur Length.
    #[slider(
        label = "Shutter",
        min = 0.0,
        max = 1.0,
        default = 0.5,
        hard_min = 0.0,
        hard_max = 4.0,
        unit = Raw
    )]
    pub mb_amount: f32,

    /// How many sub-frames the smear averages. A big amplitude at nine samples
    /// shows the taps as ghosts; more samples fill the same shutter window more
    /// densely. Defaults to what the smear always used, so an old shake
    /// renders the bits it always did. Read by [`ShakeDef::resolve_derived`],
    /// which is where the sub-frames are sampled.
    #[slider(
        label = "Samples",
        min = 2.0,
        max = 64.0,
        default = 9.0,
        hard_min = 2.0,
        hard_max = 64.0,
        unit = Raw
    )]
    pub mb_samples: f32,

    /// A steady zoom on top of the wobble, so the picture can be pushed in
    /// far enough that the edges it reveals never show. 100 is natural size.
    #[slider(
        min = 100.0,
        max = 200.0,
        default = 100.0,
        hard_min = 1.0,
        hard_max = 1000.0,
        unit = Percent
    )]
    pub scale: f32,

    /// How the resample treats the border the wobble reveals (P3).
    /// Default Mirror (owner, 2026-07-19; was Repeat): the reflected border
    /// reads more naturally under the shake's own motion blur than a smeared
    /// repeat edge. What the kernel reads is [`Shake::DERIVED_EDGE`], not this
    /// row: a project saved before FX-11 carries an `auto_scale` bool instead.
    #[choice(label = "Edges", options = *crate::fx::builtins::EDGE_OPTIONS, default = 2)]
    pub edge: u32,

    /// The top and bottom border on its own, like S_Shake's Wrap Y. Same
    /// follows Edges.
    #[choice(
        label = "Y edges",
        options = ["Same", "Transparent", "Repeat", "Mirror"],
        default = 0
    )]
    pub y_edge: u32,

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

/// What this frame's wobble is built from that no slider holds: the
/// unit-free noise, and the two rows whose stored form an old project spells
/// differently. All of it comes out of the bag through [`Shake::derived_of`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShakeDerived {
    /// The frame's noise sample, `(x, y, rotation, z)`, each −1..1.
    pub noise: [f32; 4],
    /// The same, at each motion-blur sub-frame across the shutter. The first
    /// `mb_count` entries are live; the rest are zeros.
    pub mb_noise: [[f32; 4]; SHAKE_MB_SAMPLES],
    /// How many sub-frames the smear averages. 0 is the smear off, which is the
    /// one place that decision is taken.
    pub mb_count: usize,
    /// Depth (z) scale-pump magnitude, 0..1: the Z amount row as a fraction,
    /// or an old project's `zoom_pump`.
    pub z_amp: f32,
    /// The [`EdgesMode`] wire code, or an old project's `auto_scale` migrated.
    pub edge: u32,
    /// The noise already carries the per-axis amounts and the waves, so
    /// [`Shake::packed`] only adds the master Amplitude. Set only while a wave
    /// is on, so a shake without one keeps the bits it always had.
    pub prescaled: bool,
}

/// Which dispatch a shake draws through, and the numbers that dispatch wants
/// (docs/impl/effect-registry.md §2.4).
///
/// **Why `packed` returns this rather than a tuple.** The shake's own motion
/// blur is not a dial but a different pass — the averaging kernel, fed one
/// affine per sub-frame — exactly as RGB split's Wavelength is. One enum
/// keeps the fork in one place, so the CPU reference and the GPU wrapper cannot
/// disagree about which one an instance is in.
///
/// The smeared variant carries its sub-frames inline, so the enum is as wide as
/// the widest smear (a kilobyte). It is built once per dispatch, never per
/// pixel, and boxing it would cost the `Copy` both render paths read it through.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::large_enum_variant)]
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
    /// The shake's own motion blur (T18): the wobble at `count` sub-frame
    /// placements, resampled and averaged in premultiplied linear space. An
    /// odd count's centre sample is the frame itself.
    Blurred {
        /// One wobble per sub-frame, in shutter order; only the first `count`
        /// are live.
        samples: [ShakeSample; SHAKE_MB_SAMPLES],
        /// Live sub-frames, `2..=SHAKE_MB_SAMPLES`.
        count: usize,
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

    /// The same at each motion-blur sub-frame, the first Samples of them
    /// present only when the smear is on. Fixed ids rather than a counted list,
    /// so the bag stays a flat key/value set.
    pub const DERIVED_MB_NOISE: [ParamId; SHAKE_MB_SAMPLES] = mb_noise_ids!(
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47,
        48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63
    );

    /// The depth (z) pump as a 0..1 magnitude, with an old project's
    /// `zoom_pump` folded in.
    pub const DERIVED_Z_AMP: ParamId = ParamId::new("derived.z_amp");

    /// The edge policy, with an old project's `auto_scale` bool folded in.
    pub const DERIVED_EDGE: ParamId = ParamId::new("derived.edge");

    /// Whether the noise is prescaled (see [`ShakeDerived::prescaled`]).
    pub const DERIVED_PRESCALED: ParamId = ParamId::new("derived.prescaled");

    /// Everything [`Shake::packed`] needs that is not a declared row, out of a
    /// resolved bag — so no caller has to know the ids.
    ///
    /// The motion-blur set is read by *presence*: [`ShakeDef::resolve_derived`]
    /// pushes the first Samples of it only when the toggle is on and the
    /// shutter is non-zero, which is the one place that decision is taken (the
    /// old arm took it in `f64`, on the stored value, and it still does).
    /// The set runs from the front and stops at the first id the resolver did
    /// not push, so a plain shake reads one absent id and no more.
    pub fn derived_of(p: Params<'_>) -> ShakeDerived {
        let mut mb_noise = [[0.0f32; 4]; SHAKE_MB_SAMPLES];
        let mut mb_count = 0;
        for (slot, id) in mb_noise.iter_mut().zip(Self::DERIVED_MB_NOISE) {
            match p.get(id) {
                Some(_) => *slot = p.vec4(id, [0.0; 4]),
                None => break,
            }
            mb_count += 1;
        }
        ShakeDerived {
            noise: p.vec4(Self::DERIVED_NOISE, [0.0; 4]),
            mb_noise,
            mb_count,
            z_amp: p.float(Self::DERIVED_Z_AMP, 0.0),
            edge: p.choice(Self::DERIVED_EDGE, EdgesMode::Repeat.code()),
            prescaled: p.bool(Self::DERIVED_PRESCALED, false),
        }
    }

    /// Which dispatch to run and what to hand it (docs/impl/effect-registry.md
    /// §2.4).
    ///
    /// The wobble is [`ShakeWobble::at`](crate::fx::maths::ShakeWobble::at) reassembled
    /// from the pieces: `amp_px · axis amount · noise` for the offsets, `rotation
    /// amount · noise` for the twist, `1 + z · noise` for the zoom — the same
    /// association and the same `f64 → f32` cast points, so an untouched shake
    /// is bit-for-bit itself. `amplitude` arrives already converted from
    /// % diagonal by the resolve step, so this only floors it, as the old arm
    /// floored the same product. Both render paths read this one method, so the
    /// CPU reference and the WGSL kernel cannot drift apart.
    pub fn packed(self, d: ShakeDerived) -> Shaken {
        let amp_px = self.amplitude.max(0.0);
        let x_amp = self.x_amp.max(0.0);
        let y_amp = self.y_amp.max(0.0);
        let rot_amount = self.rotation.max(0.0);
        let mix = (self.mix / 100.0).clamp(0.0, 1.0);
        // 100% divides to exactly 1, so an old shake keeps its bits.
        let scale = (self.scale / 100.0).max(0.01);
        let at = |n: [f32; 4]| match d.prescaled {
            true => ShakeSample {
                offset_px: [amp_px * n[0], amp_px * n[1]],
                rotation_deg: n[2],
                zoom: (1.0 + n[3]) * scale,
            },
            false => ShakeSample {
                offset_px: [amp_px * x_amp * n[0], amp_px * y_amp * n[1]],
                rotation_deg: rot_amount * n[2],
                zoom: (1.0 + d.z_amp * n[3]) * scale,
            },
        };
        match d.mb_count {
            count if count >= 2 => Shaken::Blurred {
                samples: d.mb_noise.map(at),
                count,
                edge: d.edge,
                mix,
            },
            _ => Shaken::Plain {
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
    /// arm did beyond reading its rows, moved unchanged.
    ///
    /// Everything here that feeds the noise stays `f64` to the last step: layer
    /// time, the master frequency and the per-axis multipliers, exactly as the
    /// arm had them. Only the sample itself narrows to `f32`, at the point
    /// [`ShakeWobble::at`](crate::fx::maths::ShakeWobble::at) narrowed it.
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
        // sub-frames, so they agree bit-for-bit. Phase shifts the whole shake
        // in time; at 0 the base is exactly what it always was.
        let phase = fl("phase").unwrap_or(0.0);
        let base = (lt + phase) * freq;
        // Style: 0 Normal, 1 Twitchy, 2 Jumpy. Normal takes the same path as
        // before, and its gate is exactly 1, so an old shake keeps its bits.
        let style = match e.param("style") {
            Some(EffectValue::Choice(c)) => *c,
            _ => 0,
        };
        let drift = fl("drift").unwrap_or(0.3).clamp(0.0, 1.0);
        let centre = fl("centre_bias").unwrap_or(0.0);
        let stillness = fl("stillness").unwrap_or(0.7);
        let twitch_freq = fl("twitch_freq").unwrap_or(2.0).max(0.0);
        let sample = |channel: u32, x: f64| match style {
            2 => shake_jump_noise(seed, channel, x, drift, centre),
            _ => shake_noise(seed, channel, x),
        };

        // z (depth/scale) amount: the new id, else the old `zoom_pump` (a
        // project saved before FX-11 keeps its pump), a scale-pump per cent
        // either way.
        let z_pct = fl("z_amp").or_else(|| fl("zoom_pump")).unwrap_or(0.0) as f32;
        let z_amp = (z_pct / 100.0).clamp(0.0, 1.0);
        push(Shake::DERIVED_Z_AMP, Value::Float(z_amp));

        // The waves, as (amount, × Frequency, phase in radians) per axis in
        // the noise's order. Z's amount is a per cent like Z amount's.
        let wave = |[amount, hz, ph]: [&str; 3], per: f64| {
            (
                fl(amount).unwrap_or(0.0).max(0.0) / per,
                fl(hz).unwrap_or(0.5).max(0.0),
                fl(ph).unwrap_or(0.0).to_radians(),
            )
        };
        let waves = [
            wave(["x_wave", "x_wave_freq", "x_wave_phase"], 1.0),
            wave(["y_wave", "y_wave_freq", "y_wave_phase"], 1.0),
            wave(["rot_wave", "rot_wave_freq", "rot_wave_phase"], 1.0),
            wave(["z_wave", "z_wave_freq", "z_wave_phase"], 100.0),
        ];
        let waving = waves.iter().any(|w| w.0 > 0.0);
        // With a wave on, the random part takes its per-axis amount here, so
        // the wave can be added on top in the same units.
        let amounts = [
            fl("x_amp").unwrap_or(1.0).max(0.0),
            fl("y_amp").unwrap_or(1.0).max(0.0),
            fl("rotation").unwrap_or(1.0).max(0.0),
            f64::from(z_amp),
        ];
        if waving {
            push(Shake::DERIVED_PRESCALED, Value::Bool(true));
        }

        // The twitch gate runs on time, not on the noise base, so a
        // motion-blur sub-frame gets its time back from its base.
        let noise = |b: f64| {
            let gate = match style {
                1 => {
                    let t = if freq > 0.0 { b / freq } else { lt + phase };
                    shake_twitch_gate(seed, t * twitch_freq, stillness)
                }
                _ => 1.0,
            };
            let n = [
                sample(0, b * x_freq) * gate,
                sample(1, b * y_freq) * gate,
                sample(2, b * rot_freq) * gate,
                sample(3, b * z_freq) * gate,
            ];
            if !waving {
                return Value::Vec4(n.map(|v| v as f32));
            }
            let mut out = [0.0f32; 4];
            for (o, ((v, k), (a, f, p))) in out.iter_mut().zip(n.iter().zip(amounts).zip(waves)) {
                *o = (v * k + a * (std::f64::consts::TAU * f * b + p).sin()) as f32;
            }
            Value::Vec4(out)
        };
        push(Shake::DERIVED_NOISE, noise(base));

        // Edges (P3): the stored Choice, else the old Auto-scale bool
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
        // Y edges ride in the bits above, plus one, so Same leaves the code
        // as it always was (cpu::bilinear_edge reads them).
        let y_edge = match e.param("y_edge") {
            Some(EffectValue::Choice(c)) => (*c).min(3),
            _ => 0,
        };
        push(
            Shake::DERIVED_EDGE,
            Value::Choice(edge.code() | (y_edge << 2)),
        );

        // The shake's own motion blur (T18): when the toggle is on and
        // the amount is non-zero, sample the wobble across the shutter for the
        // dispatch to average; off is the plain single resample (the bit-exact
        // passthrough). The centre offset is 0, so the middle sample is the
        // frame-time wobble exactly.
        let motion_blur = e.bool_of("motion_blur").unwrap_or(false);
        let mb_amount = fl("mb_amount").unwrap_or(0.5);
        // A shake saved before the Samples row keeps the nine it was made with.
        let mb_samples =
            fl("mb_samples").map_or(SHAKE_MB_DEFAULT_SAMPLES, |s| s.round().max(0.0) as usize);
        if motion_blur && mb_amount > 0.0 {
            for (id, db) in Shake::DERIVED_MB_NOISE
                .iter()
                .zip(shake_mb_offsets(mb_amount, mb_samples))
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
                // A wobble is an offset, a turn and a zoom; the Shake has no
                // Skew control, so it passes none.
                cpu::transform(
                    rgba, w, h, anchor, position, scale, rot, NO_SKEW, edge, 1.0, mix,
                );
            }
            Shaken::Blurred {
                samples,
                count,
                edge,
                mix,
            } => {
                let mut ops = [([1.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0]); SHAKE_MB_SAMPLES];
                for (op, s) in ops.iter_mut().zip(samples.iter()) {
                    let (anchor, position, scale, rot) =
                        shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                    let (m, o, _opacity) = transform_op(anchor, position, scale, rot, NO_SKEW, 1.0);
                    *op = (m, o);
                }
                cpu::transform_average(rgba, w, h, &ops[..count], edge, mix);
            }
        }
    }
}
