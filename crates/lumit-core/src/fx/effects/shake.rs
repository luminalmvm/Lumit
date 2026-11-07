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

use crate::anim::{Animation, Property};
use crate::fx::builtins::scale_property;
use crate::fx::{
    cpu, maths::shake_affine, maths::transform_op, maths::ShakeSample, maths::NO_SKEW,
    maths::SHAKE_MB_SAMPLES, params::ParamId, params::Params, params::Value, registry::EffectDef,
    registry::EffectMetadata, registry::ResolveCx, schema::EdgesMode, schema::EffectSchema,
    schema::ParamGroup,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

mod resolve;

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
        label: "Channels",
        params: &[
            "red_amount",
            "red_phase",
            "green_amount",
            "green_phase",
            "blue_amount",
            "blue_phase",
            "rgb_randomness",
            "rgb_freq",
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
    ParamGroup {
        label: "Crop",
        params: &["crop_left", "crop_right", "crop_top", "crop_bottom"],
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
    // 2: Phase holds the shake back rather than bringing it forward.
    // 3: Amplitude scales the twist and the zoom too. Both are carried over
    // on load ([`migrate`]).
    version = 3,
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
    /// px@comp (§2.3): how far the wobble roams. The twist and the zoom go
    /// with it, × Amplitude / 30, so 0 leaves the picture where it is.
    /// Declared `Px`, so the resolve step scales it to the raster in play and
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

    /// Seconds the whole shake runs behind, as S_Shake's Phase does, so two
    /// layers with the same seed can shake out of step.
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

    /// Degrees of twist wobble either way at the default Amplitude.
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

    /// × the whole shake for the red channel alone. Typed below 0 it goes
    /// the other way.
    #[slider(label = "Red amount", min = 0.0, max = 2.0, default = 1.0, hard_min = -4.0, hard_max = 4.0, unit = Raw)]
    pub red_amount: f32,

    /// Seconds the red channel's shake runs behind the rest, as S_Shake's
    /// Red Phase does. Typed below 0 it runs ahead.
    #[slider(label = "Red phase", min = -1.0, max = 1.0, default = 0.0, unit = Seconds)]
    pub red_phase: f32,

    /// See [`red_amount`](Self::red_amount).
    #[slider(label = "Green amount", min = 0.0, max = 2.0, default = 1.0, hard_min = -4.0, hard_max = 4.0, unit = Raw)]
    pub green_amount: f32,

    /// See [`red_phase`](Self::red_phase).
    #[slider(label = "Green phase", min = -1.0, max = 1.0, default = 0.0, unit = Seconds)]
    pub green_phase: f32,

    /// See [`red_amount`](Self::red_amount).
    #[slider(label = "Blue amount", min = 0.0, max = 2.0, default = 1.0, hard_min = -4.0, hard_max = 4.0, unit = Raw)]
    pub blue_amount: f32,

    /// See [`red_phase`](Self::red_phase).
    #[slider(label = "Blue phase", min = -1.0, max = 1.0, default = 0.0, unit = Seconds)]
    pub blue_phase: f32,

    /// A wobble of each channel's own on top, × the shake's size. 0 keeps
    /// the channels together.
    #[slider(label = "Randomness", min = 0.0, max = 1.0, default = 0.0, hard_min = 0.0, hard_max = 4.0, unit = Raw)]
    pub rgb_randomness: f32,

    /// × the master Frequency, for the channels' own wobble.
    #[slider(label = "Random rate", min = 0.0, max = 4.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub rgb_freq: f32,

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

    /// % of the width cut from the left before the shake. The cut lines
    /// stand in for the frame's edges, so Edges fills past them.
    #[slider(label = "Crop left", min = 0.0, max = 50.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub crop_left: f32,

    /// See [`crop_left`](Self::crop_left).
    #[slider(label = "Crop right", min = 0.0, max = 50.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub crop_right: f32,

    /// See [`crop_left`](Self::crop_left).
    #[slider(label = "Crop top", min = 0.0, max = 50.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub crop_top: f32,

    /// See [`crop_left`](Self::crop_left).
    #[slider(label = "Crop bottom", min = 0.0, max = 50.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub crop_bottom: f32,

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
    /// The channels shake apart: the motion-blur set holds red's sub-frames,
    /// then green's, then blue's, the same count each.
    pub split: bool,
    /// How much of Scale, Crop and Blend is left, 0..1: 0 at Amplitude 0, and
    /// exactly 1 above its last few pixels.
    pub settle: f32,
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
    /// The plain single resample through the Transform kernel: no smear,
    /// no Channels and no Crop.
    Plain {
        /// This frame's wobble.
        wobble: ShakeSample,
        /// [`EdgesMode`] wire code for the revealed border.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    /// Everything else through the averaging kernel: the wobble at `count`
    /// placements, each resampled and averaged in premultiplied linear space.
    /// Its own motion blur (T18) puts them across the shutter, an odd count's
    /// centre sample being the frame itself. One placement is a plain shake
    /// that is only cropped.
    Taps {
        /// One wobble per placement, in shutter order; only the first `count`
        /// are live.
        samples: [ShakeSample; SHAKE_MB_SAMPLES],
        /// Live placements, `1..=SHAKE_MB_SAMPLES`.
        count: usize,
        /// The first third of the placements feed red, the next green, the
        /// last blue. Alpha averages all of them.
        split: bool,
        /// The Crop rows as 0..1 of the frame: left, top, right, bottom.
        crop: [f32; 4],
        /// [`EdgesMode`] wire code for the revealed border.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
}

impl Shaken {
    /// The channels placement `i` of `count` feeds, as bits: 1 red, 2 green,
    /// 4 blue. Alpha takes every placement.
    pub fn tap_channels(i: usize, count: usize, split: bool) -> u32 {
        match split {
            true => 1 << (i * 3 / count.max(1)).min(2),
            false => 7,
        }
    }

    /// A Crop as whole pixels of a `w`×`h` frame: the first column and row
    /// kept and one past the last. Never empty.
    pub fn crop_px(crop: [f32; 4], w: u32, h: u32) -> [u32; 4] {
        let cut = |v: f32, n: u32| ((v * n as f32).round() as u32).min(n.saturating_sub(1));
        let (x0, y0) = (cut(crop[0], w), cut(crop[1], h));
        [
            x0,
            y0,
            w.saturating_sub(cut(crop[2], w)).max(x0 + 1),
            h.saturating_sub(cut(crop[3], h)).max(y0 + 1),
        ]
    }
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

    /// Whether the channels shake apart (see [`ShakeDerived::split`]).
    pub const DERIVED_SPLIT: ParamId = ParamId::new("derived.split");

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
            split: p.bool(Self::DERIVED_SPLIT, false),
            settle: p.float(crate::fx::schema::BLEND_FADE, 1.0),
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
        // 100% divides to exactly 1, so an old shake keeps its bits, and a
        // settle of exactly 1 adds nothing.
        let scale = (self.scale / 100.0).max(0.01);
        let scale = scale + (1.0 - scale) * (1.0 - d.settle);
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
        let crop = [
            self.crop_left,
            self.crop_top,
            self.crop_right,
            self.crop_bottom,
        ]
        .map(|v| (v / 100.0).clamp(0.0, 1.0));
        let mut samples = d.mb_noise.map(at);
        let count = match d.split || d.mb_count >= 2 {
            true => d.mb_count,
            false => {
                samples[0] = at(d.noise);
                1
            }
        };
        match count == 1 && crop == [0.0; 4] {
            true => Shaken::Plain {
                wobble: samples[0],
                edge: d.edge,
                mix,
            },
            false => Shaken::Taps {
                samples,
                count,
                split: d.split,
                crop,
                edge: d.edge,
                // A Crop has no wobble to shrink, so what it fills dissolves.
                mix: match crop == [0.0; 4] {
                    true => mix,
                    false => mix * d.settle,
                },
            },
        }
    }
}

/// Carry a saved shake over to this version, so it renders as it did.
/// `wired` says whether a row has a wire into it.
pub fn migrate(e: &mut EffectInstance, wired: &dyn Fn(&str) -> bool) {
    let rows = |e: &mut EffectInstance, ids: &[&str], k: f64| {
        for p in &mut e.params {
            if let (true, EffectValue::Float(prop)) = (ids.contains(&p.id.as_str()), &mut p.value) {
                scale_property(prop, k);
            }
        }
    };
    // v1 → v2: Phase brought the shake forward and now holds it back, so an
    // old value is turned round. One an expression drives is left on version
    // 1, which the resolver reads the old way, since a script cannot be turned
    // round safely.
    let scripted = matches!(e.param("phase"), Some(EffectValue::Float(p))
        if matches!(p.animation, Animation::Expression(_)));
    if e.effect.version < 2 && !scripted {
        rows(e, &["phase"], -1.0);
        e.effect.version = 2;
    }
    // v2 → v3: the twist and zoom rows are divided by Amplitude / 30. That
    // only works for a still Amplitude above 0, and while the rows stay in
    // range. Any other shake stays on version 2 and reads the old way, apart
    // from settling when its keys take Amplitude to 0 (`resolve::derived`).
    if e.effect.version != 2 {
        return;
    }
    // Only for this update: a still Amplitude of 0 only ever twisted and
    // zoomed, so it comes in as 30 with no X or Y and moves as it did.
    let across = ["x_amp", "y_amp", "x_wave", "y_wave"];
    if matches!(e.param("amplitude"), Some(EffectValue::Float(p))
        if matches!(p.animation, Animation::Static(a) if a <= 0.0))
        && !wired("amplitude")
        && !across.iter().any(|id| wired(id))
    {
        for p in &mut e.params {
            let v = match p.id.as_str() {
                "amplitude" => 30.0,
                id if across.contains(&id) => 0.0,
                _ => continue,
            };
            p.value = EffectValue::Float(Property::fixed(v));
        }
        e.effect.version = 3;
        return;
    }
    let k = match e.param("amplitude") {
        Some(EffectValue::Float(p)) if !wired("amplitude") => match p.animation {
            Animation::Static(a) if a > 0.0 => 30.0 / a,
            _ => return,
        },
        _ => return,
    };
    let twist = [
        ("rotation", 360.0),
        ("rot_wave", 360.0),
        ("z_amp", 100.0),
        ("zoom_pump", 100.0),
        ("z_wave", 100.0),
    ];
    let fits = |(id, max): (&str, f64)| match e.param(id) {
        None => true,
        Some(EffectValue::Float(p)) if !wired(id) => match &p.animation {
            Animation::Static(v) => v * k <= max,
            Animation::Keyframed(keys) => keys.iter().all(|key| key.value * k <= max),
            Animation::Expression(_) => false,
        },
        _ => false,
    };
    if twist.into_iter().all(fits) {
        rows(e, &twist.map(|(id, _)| id), k);
        e.effect.version = 3;
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
        resolve::derived(cx, push);
    }

    /// Amplitude 0 has faded out everything the rows can do.
    fn leaves_picture(&self, p: Params<'_>) -> bool {
        Shake::derived_of(p).settle == 0.0
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
            Shaken::Taps {
                samples,
                count,
                split,
                crop,
                edge,
                mix,
            } => {
                let mut ops = [([1.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0], 7); SHAKE_MB_SAMPLES];
                for (i, (op, s)) in ops.iter_mut().zip(samples.iter()).enumerate() {
                    let (anchor, position, scale, rot) =
                        shake_affine(w, h, s.offset_px, s.rotation_deg, s.zoom);
                    let (m, o, _opacity) = transform_op(anchor, position, scale, rot, NO_SKEW, 1.0);
                    *op = (m, o, Shaken::tap_channels(i, count, split));
                }
                let crop = Shaken::crop_px(crop, w, h);
                cpu::transform_average(rgba, w, h, &ops[..count], edge, crop, mix);
            }
        }
    }
}
