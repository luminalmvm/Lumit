//! The built-in effect registry and CPU reference implementations
//! (docs/08-EFFECTS.md §1). The WGSL production path lives in `lumit-gpu`
//! (docs/05 crate table); this module is the engine-pure side: what each
//! effect *is* (schema, parameters, traits), how an instance is born with
//! tasteful defaults, how a stack resolves to plain evaluated numbers at a
//! frame, and the CPU maths that serve as the test oracle (§1.6) and the
//! degradation ladder's fallback rung (K-019).
//!
//! In plain terms: this file is the effects catalogue. Each entry declares
//! its parameters (names, defaults, slider ranges) and its cost/behaviour
//! traits; dropping one on a layer copies the declared defaults into the
//! project. At render time the animatable parameters are evaluated at the
//! frame's time into a flat list of numbers — the same list the GPU kernels
//! and these CPU functions both consume, which is what makes "the GPU must
//! agree with the CPU" a testable promise.

use crate::anim::{Animation, Property};
use crate::model::{EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue};

/// Cost class (docs/08 §1.3) — consumed by degradation ordering and budgets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostClass {
    Trivial,
    Cheap,
    Moderate,
    Heavy,
}

/// Region-of-interest support (docs/08 §1.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Roi {
    /// Output pixel needs only the same input pixel.
    Exact,
    /// Needs input dilated by a radius, in % of the comp diagonal (§2.3).
    PaddedPctDiag(f32),
    /// Needs the whole input.
    FullFrame,
}

/// Static trait declaration (docs/08 §1.3), read by the scheduler and caches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectTraits {
    pub cost: CostClass,
    pub roi: Roi,
    /// Source-relative frame offsets required; `&[0]` = current frame only.
    pub temporal: &'static [i32],
    /// True = operates on premultiplied alpha (the default working form).
    pub premultiplied: bool,
    pub seeded: bool,
    pub beat_input: bool,
}

/// One declared parameter (docs/08 §1.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamSchema {
    /// Stable snake_case identifier (expressions address this).
    pub id: &'static str,
    /// Sentence-case UI label.
    pub label: &'static str,
    pub kind: ParamKind,
}

/// Parameter type + defaults/ranges (docs/08 §1.2: sliders may be exceeded
/// by typing; hard ranges may not).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    Float {
        default: f64,
        slider: (f64, f64),
        /// Hard bounds; either side may be None (K-090: a threshold clamps
        /// at zero below and runs unbounded above).
        hard: (Option<f64>, Option<f64>),
    },
    Choice {
        options: &'static [&'static str],
        default: u32,
    },
    Bool {
        default: bool,
    },
    Colour {
        /// Scene-linear RGBA (docs/08 §1.1's colour type); channels animate
        /// independently in the model.
        default: [f64; 4],
        /// Per-channel edit range — linear values may exceed 1 (HDR tints)
        /// or dip below 0 (a lift), so each colour declares its own.
        range: (f64, f64),
    },
    /// An integer seed (docs/08 §1.1's seed type): selects which
    /// deterministic random-looking sequence a seeded effect follows
    /// (§2.4). No declared default — the default is per-instance (§3.4),
    /// drawn from the fresh instance id at instantiation, so two copies of
    /// a seeded effect never wobble in sync.
    Seed,
}

/// The Add-effect menu's grouping (K-090): every schema declares one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxCategory {
    BlurSharpen,
    Colour,
    Distortion,
    Stylise,
    Temporal,
    Utility,
}

impl FxCategory {
    /// Sentence-case menu label.
    pub const fn label(self) -> &'static str {
        match self {
            FxCategory::BlurSharpen => "Blur & sharpen",
            FxCategory::Colour => "Colour",
            FxCategory::Distortion => "Distortion",
            FxCategory::Stylise => "Stylise",
            FxCategory::Temporal => "Temporal",
            FxCategory::Utility => "Utility",
        }
    }

    /// Every category, in menu order.
    pub const ALL: [FxCategory; 6] = [
        FxCategory::BlurSharpen,
        FxCategory::Colour,
        FxCategory::Distortion,
        FxCategory::Stylise,
        FxCategory::Temporal,
        FxCategory::Utility,
    ];
}

/// One built-in effect's full declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSchema {
    /// Stable match name (participates in the cache key with `version`).
    pub match_name: &'static str,
    pub label: &'static str,
    pub version: u32,
    pub category: FxCategory,
    pub traits: EffectTraits,
    pub params: &'static [ParamSchema],
}

/// Edge policy shared by the blur family (docs/08 §3.8).
pub const EDGE_OPTIONS: &[&str] = &["Transparent", "Repeat", "Mirror"];

/// The host-uniform Mix parameter every effect ends with (docs/08 §1.5),
/// in per cent, blending processed over unprocessed input.
const MIX_PARAM: ParamSchema = ParamSchema {
    id: "mix",
    label: "Mix",
    kind: ParamKind::Float {
        default: 100.0,
        slider: (0.0, 100.0),
        hard: (Some(0.0), Some(100.0)),
    },
};

/// The catalogue. Grows one entry per landed effect; the schema is the single
/// source of truth the UI menu, instantiation and resolution all read.
pub const BUILTINS: &[EffectSchema] = &[
    // One blur, several modes (docs/08 §3.8): Gaussian (separable two-pass)
    // and Directional (line-integral streak along an angle); Radial follows.
    // Mode selects which extra parameters matter — Radius drives Gaussian,
    // Length/Angle drive Directional. Instances saved before the mode
    // existed resolve as Gaussian, and the gaussian maths are untouched by
    // the other modes (same kernel, same version).
    EffectSchema {
        match_name: "blur",
        label: "Blur",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            // The largest slider across modes (Directional length, 50).
            roi: Roi::PaddedPctDiag(50.0),
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "mode",
                label: "Mode",
                kind: ParamKind::Choice {
                    options: &["Gaussian", "Directional"],
                    default: 0,
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // % of the comp diagonal (§2.3), so half-res preview matches.
                // Default per §1.2's "drop it on and it already looks right".
                kind: ParamKind::Float {
                    default: 1.5,
                    slider: (0.0, 25.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "length",
                label: "Length",
                // Directional mode: the full streak length, % diag (§2.3).
                kind: ParamKind::Float {
                    default: 10.0,
                    slider: (0.0, 50.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "angle",
                label: "Angle",
                // Directional mode: streak direction, degrees (0° = +x).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-180.0, 180.0),
                    hard: (Some(-3600.0), Some(3600.0)),
                },
            },
            ParamSchema {
                id: "edge",
                label: "Edges",
                kind: ParamKind::Choice {
                    options: EDGE_OPTIONS,
                    default: 1, // Repeat: full-frame game footage never darkens
                },
            },
            MIX_PARAM,
        ],
    },
    // Unsharp mask in linear light (docs/08 §3.9), on unpremultiplied colour
    // (§2.2: sharpening premultiplied values haloes matte edges). The
    // unpremultiply → sharpen → re-premultiply wrap is fused into the kernel.
    EffectSchema {
        match_name: "sharpen",
        label: "Sharpen",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::PaddedPctDiag(4.0),
            temporal: &[0],
            premultiplied: false, // §2.2: operates on unpremultiplied colour
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // Per cent of the detail signal added back (§3.9: 0–300%).
                kind: ParamKind::Float {
                    default: 60.0,
                    slider: (0.0, 300.0),
                    hard: (Some(0.0), Some(300.0)),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // % of the comp diagonal (§2.3) — the width of the detail
                // the mask lifts; small values crispen, larger add clarity.
                kind: ParamKind::Float {
                    default: 0.4,
                    slider: (0.05, 2.0),
                    hard: (Some(0.0), Some(4.0)),
                },
            },
            ParamSchema {
                id: "threshold",
                label: "Threshold",
                // Linear-light contrast below which detail is left alone,
                // so compression noise is not amplified (§3.9).
                kind: ParamKind::Float {
                    default: 0.05,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "luminance_only",
                label: "Luminance only",
                // Sharpen the luma signal only — avoids chroma fringing on
                // compressed game capture (§3.9).
                kind: ParamKind::Bool { default: true },
            },
            MIX_PARAM,
        ],
    },
    // Chromatic aberration (docs/08 §3.6): R and B sample offset positions,
    // G stays put, alpha follows the green channel so mattes never fringe.
    // Operates premultiplied. The Wavelength Bool (K-090 quality tier)
    // swaps the three-channel split for a nine-sample spectral dispersion
    // sharing the same parameters. The §3.6 Centre/Falloff/channel-blur
    // extras land later; radial mode grows the offset from the frame
    // centre.
    EffectSchema {
        match_name: "rgb_split",
        label: "RGB split",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::PaddedPctDiag(25.0),
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // % of the comp diagonal (§2.3); the impact-frame staple is
                // a keyframed spike on this.
                kind: ParamKind::Float {
                    default: 0.4,
                    slider: (0.0, 10.0),
                    hard: (Some(0.0), Some(25.0)),
                },
            },
            ParamSchema {
                id: "angle",
                label: "Angle",
                // Degrees, linear mode: the direction R shifts (B mirrors).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-180.0, 180.0),
                    hard: (Some(-3600.0), Some(3600.0)),
                },
            },
            ParamSchema {
                id: "radial",
                label: "Radial",
                // Off: one shared shift. On: offsets grow from the centre,
                // like lens fringing.
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                id: "wavelength",
                label: "Wavelength",
                // K-090 quality tier: off = the classic three-channel
                // split (byte-identical to before this Bool existed); on =
                // wavelength-based dispersion — nine spectral samples along
                // the same offset, weighted by SPECTRAL_BASIS and
                // recombined in linear, for the higher-quality rainbow
                // fringe. All other parameters are shared between modes.
                kind: ParamKind::Bool { default: false },
            },
            MIX_PARAM,
        ],
    },
    // Beat-aware strobe (docs/08 §3.7), in its manual form for now: each
    // keyframe on Trigger is a hit (its value = how hard, 0..1) that decays
    // exponentially over Decay; a static Trigger holds a constant flash.
    // The §1.4 marker-trigger binding (trigger source, strobe mode, every
    // Nth beat) follows once marker plumbing exists — these parameters stay
    // stable when it does. Default is a no-op by design: §1.2 exempts
    // inherently trigger-driven effects.
    EffectSchema {
        match_name: "flash",
        label: "Flash",
        version: 1,
        category: FxCategory::Stylise,
        traits: EffectTraits {
            cost: CostClass::Trivial,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: true, // binds to beat markers per §1.4, later
        },
        params: &[
            ParamSchema {
                id: "trigger",
                label: "Trigger",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Per cent scale on the trigger envelope.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(400.0)),
                },
            },
            ParamSchema {
                id: "colour",
                label: "Colour",
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0), // linear light: HDR flashes are legal
                },
            },
            ParamSchema {
                id: "decay",
                label: "Decay",
                // Milliseconds for a hit to fall to 1/e.
                kind: ParamKind::Float {
                    default: 120.0,
                    slider: (10.0, 1000.0),
                    hard: (Some(0.0), Some(10000.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Colour balance (docs/08 §3.10 as amended by K-090: the v1 Grade split
    // into single-purpose colour effects): lift / gamma / gain per channel,
    // in linear, on unpremultiplied colour (§2.2). Defaults are neutral —
    // a grade's "tasteful default" is a preset choice, which is what the
    // §3.10 preset browser is for.
    EffectSchema {
        match_name: "colour_balance",
        label: "Colour balance",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: grading premult shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "lift",
                label: "Lift",
                // Added after gain: raises (or crushes, negative) the blacks.
                kind: ParamKind::Colour {
                    default: [0.0, 0.0, 0.0, 1.0],
                    range: (-1.0, 1.0),
                },
            },
            ParamSchema {
                id: "gamma",
                label: "Gamma",
                // Mid-tone curve per channel; 1 is neutral.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.1, 4.0),
                },
            },
            ParamSchema {
                id: "gain",
                label: "Gain",
                // Linear multiplier per channel; 1 is neutral.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            MIX_PARAM,
        ],
    },
    // Saturation (docs/08 §3.10 as amended by K-090): one job — scale
    // colourfulness about Rec. 709 luma, in linear, on unpremultiplied
    // colour (§2.2). Neutral default: like the balance above, its tasteful
    // setting is a preset choice.
    EffectSchema {
        match_name: "saturation",
        label: "Saturation",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: grading premult shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "saturation",
                label: "Saturation",
                // Per cent about Rec. 709 luma: 0 = greyscale, 200 = doubled.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), Some(200.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Transform (docs/08 §3.5, K-090): the layer transform group as a stack
    // entry — same parameter names, units and animatability. Its point is
    // adjustment layers: applied there, it transforms the composite of
    // everything below, which is the montage punch-in/whip gesture without
    // touching per-layer transforms. Identity parameters pass the input
    // through bit-exactly (pinned by test). The §3.5 Skew pair is post-v1.
    EffectSchema {
        match_name: "transform",
        label: "Transform",
        version: 1,
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Trivial,
            // §3.5: exact under pure translation, full-frame otherwise —
            // the static declaration carries the general case.
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "anchor_x",
                label: "Anchor x",
                // Pixels at full comp resolution (px@comp, §2.3), exactly
                // like the layer transform's Anchor; unbounded (K-090).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "anchor_y",
                label: "Anchor y",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "position_x",
                label: "Position x",
                // px@comp; the anchor point lands here. Defaults equal the
                // anchor's, so a fresh instance is the identity.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "position_y",
                label: "Position y",
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1000.0, 1000.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scale_x",
                label: "Scale x %",
                // Per cent, 100 = natural size; negative flips (like the
                // layer transform), so both hard sides stay open.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 400.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scale_y",
                label: "Scale y %",
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 400.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "rotation",
                label: "Rotation °",
                // Degrees, unbounded — whip transitions spin whole turns.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-180.0, 180.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "opacity",
                label: "Opacity %",
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Glow (docs/08 §3.3): exposure-aware bloom in scene-linear light —
    // bright-pass with a soft knee, a wide gaussian on the leftover light,
    // additive recombine. The v1 core ships Threshold/Knee/Radius/Intensity/
    // Tint; the §3.3 mip-chain items (Falloff, Chromatic aberration, the
    // Screen recombine) land with the progressive chain later and these
    // parameters stay stable when they do. The bright pass thresholds all
    // four premultiplied channels alike, so the halo carries alpha and glow
    // spreads over transparency like light.
    EffectSchema {
        match_name: "glow",
        label: "Glow",
        version: 1,
        category: FxCategory::Stylise,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            roi: Roi::PaddedPctDiag(50.0),
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "threshold",
                label: "Threshold",
                // Linear-light value above which pixels bloom. The K-090
                // one-sided hard range made concrete: clamped at zero below,
                // unbounded above — HDR values beyond the slider are legal
                // and glow harder (§2.1).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 4.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "knee",
                label: "Knee",
                // Soft-knee width: the threshold's onset is eased by a
                // smoothstep over ±knee around it (§3.3 step 1), so the
                // bloom fades in rather than snapping on.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // % of the comp diagonal (§2.3), the halo gaussian's
                // half-width — measured exactly like Blur's Radius.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 50.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // Gain on the added halo; 0 is the effect's neutral point
                // (bit-exact passthrough, pinned by test).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 10.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "tint",
                label: "Tint",
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0), // linear light: HDR tints are legal
                },
            },
            MIX_PARAM,
        ],
    },
    // Shake (docs/08 §3.4): seeded camera-shake — a transform-domain
    // wobble (translation, rotation, zoom pump) resampled once through the
    // Transform kernel, never pixel noise. The v1 continuous form ships
    // Amplitude/Frequency/Rotation amount/Zoom pump/Auto-scale/Seed; the
    // Style presets, Triggered mode (§1.4 markers), Motion blur shake and
    // the Repeat/Mirror edge options follow — these parameters stay stable
    // when they do. Seeded (§1.3): its pixels are a function of time under
    // constant parameters, which the frame key reads (lumit-eval).
    EffectSchema {
        match_name: "shake",
        label: "Shake",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: true,
            beat_input: false, // Triggered mode arrives with §1.4 plumbing
        },
        params: &[
            ParamSchema {
                id: "amplitude",
                label: "Amplitude",
                // % of the comp diagonal (§2.3): how far the wobble roams.
                kind: ParamKind::Float {
                    default: 1.5,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "frequency",
                label: "Frequency",
                // Hz — how fast the wobble wanders; the noise samples at
                // local time × frequency. Unbounded above (K-090): any
                // positive rate is meaningful, sampling handles it.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.1, 30.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "rotation",
                label: "Rotation amount",
                // Degrees of twist wobble either way.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 45.0),
                    hard: (Some(0.0), Some(360.0)),
                },
            },
            ParamSchema {
                id: "zoom_pump",
                label: "Zoom pump",
                // % of scale wobble about natural size (§3.4).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "auto_scale",
                label: "Auto-scale",
                // On (the montage default): scale up by the declared maxima
                // so the wobble never reveals an edge. Off: revealed area
                // is transparent. The §3.4 Repeat/Mirror options follow.
                kind: ParamKind::Bool { default: true },
            },
            ParamSchema {
                id: "seed",
                label: "Seed",
                kind: ParamKind::Seed,
            },
            MIX_PARAM,
        ],
    },
];

/// Look a schema up by its match name.
pub fn schema(match_name: &str) -> Option<&'static EffectSchema> {
    BUILTINS.iter().find(|s| s.match_name == match_name)
}

/// A fresh random seed value — the per-instance Seed default (docs/08
/// §3.4) and the Effect Controls "reseed" button (§2.4) both draw from
/// here. Taken from a new UUID's random tail, so it needs no extra
/// dependency; the value becomes stored project data the moment it is
/// chosen, so evaluation determinism (§2.4) is untouched.
pub fn fresh_seed() -> u32 {
    let b = uuid::Uuid::now_v7().into_bytes();
    u32::from_le_bytes([b[12], b[13], b[14], b[15]])
}

/// A new instance of a built-in, carrying the declared defaults.
pub fn instantiate(match_name: &str) -> Option<EffectInstance> {
    let s = schema(match_name)?;
    Some(EffectInstance {
        id: uuid::Uuid::now_v7(),
        effect: EffectKey {
            namespace: EffectNamespace::Builtin,
            match_name: s.match_name.to_owned(),
            version: s.version,
            extra: serde_json::Map::new(),
        },
        enabled: true,
        params: s
            .params
            .iter()
            .map(|p| EffectParam {
                id: p.id.to_owned(),
                value: match p.kind {
                    ParamKind::Float { default, .. } => {
                        EffectValue::Float(Property::fixed(default))
                    }
                    ParamKind::Choice { default, .. } => EffectValue::Choice(default),
                    ParamKind::Bool { default } => EffectValue::Bool(default),
                    ParamKind::Colour { default, .. } => {
                        EffectValue::Colour(default.map(Property::fixed))
                    }
                    ParamKind::Seed => EffectValue::Seed(fresh_seed()),
                },
                extra: serde_json::Map::new(),
            })
            .collect(),
        extra: serde_json::Map::new(),
    })
}

/// One effect, resolved to plain numbers at a frame — the flat form both the
/// WGSL kernels (lumit-gpu) and the CPU references below consume.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resolved {
    Blur {
        /// Kernel half-width in *pixels of the target raster* (the caller
        /// converts from % diagonal using the raster it renders at, §2.3).
        radius_px: f32,
        /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    DirBlur {
        /// Full streak length in raster pixels.
        length_px: f32,
        /// Streak direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// 0 = Transparent, 1 = Repeat, 2 = Mirror.
        edge: u32,
        /// 0..1.
        mix: f32,
    },
    Sharpen {
        /// Fraction of the detail signal added back (0..3 = 0–300%).
        amount: f32,
        /// The internal gaussian's half-width, in raster pixels.
        radius_px: f32,
        /// Linear-light detail magnitude below which nothing is added.
        threshold: f32,
        /// True: sharpen the Rec. 709 luma only (no chroma fringing).
        luma_only: bool,
        /// 0..1.
        mix: f32,
    },
    RgbSplit {
        /// Peak channel offset in raster pixels.
        amount_px: f32,
        /// Linear-mode shift direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// True: offsets grow from the frame centre instead.
        radial: bool,
        /// 0..1.
        mix: f32,
    },
    /// The RGB split's Wavelength mode (docs/08 §3.6, K-090): its own
    /// variant, exactly as Blur's Directional mode is — so the classic
    /// mode's path stays byte-identical.
    SpectralSplit {
        /// Peak spectral offset in raster pixels.
        amount_px: f32,
        /// Linear-mode shift direction, degrees (0° = +x, y-down raster).
        angle_deg: f32,
        /// True: offsets grow from the frame centre instead.
        radial: bool,
        /// 0..1.
        mix: f32,
    },
    Flash {
        /// The evaluated envelope × intensity, 0..1 (0 = no flash).
        strength: f32,
        /// Scene-linear RGBA flash colour (alpha unused: the flash respects
        /// the layer's own footprint).
        colour: [f32; 4],
        /// 0..1.
        mix: f32,
    },
    ColourBalance {
        /// Added per channel after gain (raises or crushes the blacks).
        lift: [f32; 3],
        /// Per-channel mid-tone exponent's base; 1 is neutral, > 0.
        gamma: [f32; 3],
        /// Per-channel linear multiplier; 1 is neutral.
        gain: [f32; 3],
        /// 0..1.
        mix: f32,
    },
    Saturation {
        /// Factor about Rec. 709 luma: 0 = greyscale, 1 = neutral, 2 = max.
        saturation: f32,
        /// 0..1.
        mix: f32,
    },
    Transform {
        /// Anchor point, raster pixels (converted from px@comp, §2.3).
        anchor: [f32; 2],
        /// Where the anchor lands, raster pixels.
        position: [f32; 2],
        /// Per-axis factor; 1 is natural size, negative flips.
        scale: [f32; 2],
        /// Degrees about the anchor (0° = none; y-down raster, so positive
        /// turns clockwise on screen, matching the layer transform).
        rotation_deg: f32,
        /// 0..1, multiplied into the premultiplied output.
        opacity: f32,
        /// 0..1.
        mix: f32,
    },
    Glow {
        /// The halo gaussian's half-width in raster pixels.
        radius_px: f32,
        /// Linear-light bright threshold, ≥ 0 (unbounded above, K-090).
        threshold: f32,
        /// Soft-knee width around the threshold, 0..1.
        knee: f32,
        /// Gain on the added halo; 0 is the neutral point.
        intensity: f32,
        /// Scene-linear RGBA halo tint (alpha unused: the halo's own alpha
        /// is untinted coverage).
        tint: [f32; 4],
        /// 0..1.
        mix: f32,
    },
    /// A shake, already sampled at this frame (the noise runs at resolve
    /// time, host-side): the current wobble plus the declared maxima the
    /// Auto-scale cover is computed from. Dispatches through the Transform
    /// kernel via [`shake_affine`] — no kernel of its own.
    Shake {
        /// This frame's wobble offset, raster pixels.
        offset_px: [f32; 2],
        /// This frame's rotation wobble, degrees.
        rotation_deg: f32,
        /// This frame's zoom factor; 1 = no pump.
        zoom: f32,
        /// The amplitude ceiling in raster pixels (Auto-scale input).
        amp_px: f32,
        /// The rotation ceiling in degrees (Auto-scale input).
        rotation_max_deg: f32,
        /// The zoom floor, 1 − pump (Auto-scale input).
        zoom_min: f32,
        /// True: scale up so the wobble never reveals an edge (§3.4).
        auto_scale: bool,
        /// 0..1.
        mix: f32,
    },
}

/// The inverse affine of a Transform effect (docs/08 §3.5): the forward map
/// is `p_out = position + R(rotation) · S(scale) · (p_in − anchor)` — the
/// layer transform's own shape — so each output pixel centre `p` samples the
/// input at `q = m·p + o` with `m = S⁻¹·R⁻¹` (row-major 2×2) and
/// `o = anchor − m·position`. Host-computed so the WGSL kernel never runs
/// its own trigonometry (its `cos`/`sin` are not correctly rounded) and the
/// CPU reference consumes bit-identical numbers. `None` when a scale axis is
/// degenerate (|s| < 1e-6): the image has collapsed to nothing and renders
/// fully transparent — never a division blow-up (docs/14 no-panic rule).
pub fn transform_inverse(
    anchor: [f32; 2],
    position: [f32; 2],
    scale: [f32; 2],
    rotation_deg: f32,
) -> Option<([f32; 4], [f32; 2])> {
    if scale[0].abs() < 1e-6 || scale[1].abs() < 1e-6 {
        return None;
    }
    let rad = (rotation_deg as f64).to_radians();
    let (sin, cos) = (rad.sin() as f32, rad.cos() as f32);
    let m = [
        cos / scale[0],
        sin / scale[0],
        -sin / scale[1],
        cos / scale[1],
    ];
    let o = [
        anchor[0] - (m[0] * position[0] + m[1] * position[1]),
        anchor[1] - (m[2] * position[0] + m[3] * position[1]),
    ];
    Some((m, o))
}

/// [`transform_inverse`] folded with the degenerate case, as the GPU op
/// ingredients `(m, offset, effective opacity)`: a zero-scale transform
/// maps to an identity matrix with opacity 0 — fully transparent. The CPU
/// reference and both render paths all build from this one function, so
/// every path consumes bit-identical numbers.
pub fn transform_op(
    anchor: [f32; 2],
    position: [f32; 2],
    scale: [f32; 2],
    rotation_deg: f32,
    opacity: f32,
) -> ([f32; 4], [f32; 2], f32) {
    match transform_inverse(anchor, position, scale, rotation_deg) {
        Some((m, o)) => (m, o, opacity),
        None => ([1.0, 0.0, 0.0, 1.0], [0.0, 0.0], 0.0),
    }
}

/// The Flash trigger envelope (docs/08 §3.7, manual form). A static Trigger
/// is a constant flash. A keyframed Trigger reads each keyframe as a hit:
/// the key's value (0..1) is the hit strength, decaying exponentially to 1/e
/// over `decay_s`; overlapping hits take the loudest. The curve between keys
/// is deliberately not interpolated — one keyframe per beat is the authoring
/// unit, exactly what the §1.4 marker binding will automate. Pure function
/// of the property and time, so determinism (§2.4) holds.
pub fn flash_envelope(trigger: &Property, t: f64, decay_s: f64) -> f64 {
    match &trigger.animation {
        Animation::Static(v) => v.clamp(0.0, 1.0),
        Animation::Keyframed(keys) => {
            let mut env: f64 = 0.0;
            for k in keys {
                let kt = k.time.to_f64();
                if kt > t {
                    break; // keys are sorted; later hits cannot contribute
                }
                let fall = if decay_s > 0.0 {
                    (-(t - kt) / decay_s).exp()
                } else if t == kt {
                    1.0
                } else {
                    0.0
                };
                env = env.max(k.value.clamp(0.0, 1.0) * fall);
            }
            env
        }
    }
}

/// The glow bright pass on one channel (docs/08 §3.3 step 1):
/// `max(0, x − threshold)` with a soft knee — the hinge's onset is weighted
/// by a smoothstep over `threshold ± knee`, so the bloom fades in over the
/// knee width instead of snapping on at the threshold. Knee 0 is the hard
/// subtract. Written with the exact arithmetic order the WGSL kernel uses
/// (§1.6: both paths must agree), and shared with the CPU reference below.
pub fn glow_bright(x: f32, threshold: f32, knee: f32) -> f32 {
    let d = x - threshold;
    if d <= 0.0 {
        return 0.0;
    }
    if knee > 0.0 {
        let t = ((x - (threshold - knee)) / (2.0 * knee)).clamp(0.0, 1.0);
        let w = t * t * (3.0 - 2.0 * t);
        return d * w;
    }
    d
}

/// The SplitMix64 finaliser — the integer mixer behind the shake noise
/// lattice. Chosen for its published avalanche quality and its five-line
/// portability: any future twin (a WGSL noise, an expression binding)
/// can reproduce it exactly.
fn splitmix64(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

/// The lattice value for one noise channel at integer coordinate `i`:
/// `hash(seed, channel, i)` mapped to [−1, 1] — the §2.4 seeded-stateless
/// shape, a pure function of its inputs with no history.
fn noise_lattice(seed: u32, channel: u32, i: i64) -> f64 {
    let mixed = splitmix64(splitmix64(splitmix64(u64::from(seed)) ^ u64::from(channel)) ^ i as u64);
    // Top 53 bits → [0, 1) exactly representable in f64, then to [−1, 1].
    (mixed >> 11) as f64 * (2.0 / 9_007_199_254_740_992.0) - 1.0
}

/// One octave of seeded 1D value noise: the lattice values at the two
/// surrounding integers, smoothstep-interpolated. C¹-continuous, so the
/// wobble it drives is hop-free; deterministic per §2.4 (same inputs, same
/// output, on every machine and every run).
pub fn value_noise_1d(seed: u32, channel: u32, x: f64) -> f64 {
    let x0 = x.floor();
    let i0 = x0 as i64; // saturating cast: astronomically distant times clamp
    let f = x - x0;
    let t = f * f * (3.0 - 2.0 * f);
    let a = noise_lattice(seed, channel, i0);
    let b = noise_lattice(seed, channel, i0.wrapping_add(1));
    a + (b - a) * t
}

/// The Shake generator (docs/08 §3.4): two octaves of value noise (the
/// sketch's "Normal" fBm — lacunarity 2, gain 0.5, octaves decorrelated by
/// channel offset), normalised so the result stays within [−1, 1]. One
/// independent channel each for x, y, rotation and zoom.
pub fn shake_noise(seed: u32, channel: u32, x: f64) -> f64 {
    (value_noise_1d(seed, channel, x) + 0.5 * value_noise_1d(seed, channel + 4, x * 2.0)) / 1.5
}

/// The §3.4 Auto-scale cover factor: the smallest uniform scale that keeps
/// the frame fully covered under every wobble the current parameters allow
/// (offset up to `amp_px` in any direction, rotation up to
/// `rot_max_deg` either way, zoom down to `zoom_min`), so no edge is ever
/// revealed — the montage default. Derived from the inverse map: an output
/// corner displaced by the offset and rotated back must still land inside
/// the source frame, per axis. Pure host maths in f64, shared by the CPU
/// reference and the GPU dispatch so both consume the identical scale.
pub fn shake_cover_scale(w: u32, h: u32, amp_px: f32, rot_max_deg: f32, zoom_min: f32) -> f32 {
    let hw = f64::from(w) * 0.5;
    let hh = f64::from(h) * 0.5;
    if hw <= 0.0 || hh <= 0.0 {
        return 1.0;
    }
    let ex = hw + f64::from(amp_px.max(0.0));
    let ey = hh + f64::from(amp_px.max(0.0));
    let rot = f64::from(rot_max_deg.abs()).to_radians();
    // max over θ ∈ [0, rot] of a·cos θ + b·sin θ: at the interior optimum
    // atan2(b, a) when rot reaches it, else at rot itself.
    let reach = |a: f64, b: f64| -> f64 {
        if rot >= b.atan2(a) {
            a.hypot(b)
        } else {
            a * rot.cos() + b * rot.sin()
        }
    };
    // A full zoom-out pump (zoom_min → 0) cannot be covered by any finite
    // scale; clamp so the cover stays large but sane rather than infinite.
    let z = f64::from(zoom_min).max(1e-3);
    ((reach(ex, ey) / hw).max(reach(ey, ex) / hh) / z).max(1.0) as f32
}

/// A resolved Shake as the transform-effect ingredients it dispatches as
/// (docs/08 §3.4: a transform-domain effect — perturb a virtual camera,
/// resample once): `(anchor, position, scale, rotation)` for
/// [`transform_op`] / [`cpu::transform`], wobbling about the frame centre.
/// Both the CPU reference and the GPU path build from this one function,
/// so every path consumes bit-identical numbers.
#[allow(clippy::too_many_arguments)]
pub fn shake_affine(
    w: u32,
    h: u32,
    offset_px: [f32; 2],
    rotation_deg: f32,
    zoom: f32,
    amp_px: f32,
    rotation_max_deg: f32,
    zoom_min: f32,
    auto_scale: bool,
) -> ([f32; 2], [f32; 2], [f32; 2], f32) {
    let centre = [w as f32 * 0.5, h as f32 * 0.5];
    let cover = if auto_scale {
        shake_cover_scale(w, h, amp_px, rotation_max_deg, zoom_min)
    } else {
        1.0
    };
    let s = zoom * cover;
    (
        centre,
        [centre[0] + offset_px[0], centre[1] + offset_px[1]],
        [s, s],
        rotation_deg,
    )
}

/// The linear-mode channel offset vector for an RGB split: `amount_px`
/// along `angle_deg`. Shared by the CPU reference and the GPU op
/// construction so both paths carry the same host-computed sines (WGSL's
/// `cos`/`sin` are not correctly rounded, so the kernel never computes its
/// own).
pub fn rgb_split_offset(amount_px: f32, angle_deg: f32) -> (f32, f32) {
    let rad = angle_deg.to_radians();
    (amount_px * rad.cos(), amount_px * rad.sin())
}

/// The wavelength → linear-sRGB basis behind the RGB split's Wavelength
/// mode (docs/08 §3.6, K-090): nine taps across the visible spectrum. Tap
/// `i` sits at spectral fraction `t = i/4 − 1`, sampling `position +
/// t·offset` — so the red end (t = −1, 650 nm) lands where the classic
/// mode's R samples and the blue end (t = +1, 450 nm) where its B does,
/// and the two modes disperse in the same direction. Derived offline: CIE
/// 1931 x̄ȳz̄ via the Wyman et al. (2013) multi-lobe Gaussian fits at
/// 650–450 nm in 25 nm steps, through the sRGB D65 matrix, negatives
/// clipped, then each channel's column normalised to sum 1 (within one
/// f32 ULP) so a uniform image passes through unchanged. The CPU reference
/// reads this table directly and the WGSL kernel receives it in its
/// uniform, so both paths consume bit-identical numbers.
pub const SPECTRAL_BASIS: [[f32; 3]; 9] = [
    [0.112_422_91, 0.0, 0.0],           // 650 nm
    [0.294_590_23, 0.0, 0.0],           // 625 nm
    [0.365_333_56, 0.036_021_75, 0.0],  // 600 nm
    [0.201_592_3, 0.192_775_3, 0.0],    // 575 nm
    [0.0, 0.311_754_2, 0.0],            // 550 nm
    [0.0, 0.300_619_63, 0.0],           // 525 nm
    [0.0, 0.134_424_22, 0.068_714_05],  // 500 nm
    [0.0, 0.024_404_911, 0.339_951_04], // 475 nm
    [0.026_061_023, 0.0, 0.591_334_94], // 450 nm — the violet re-red bump
];

/// [`SPECTRAL_BASIS`] as vec4 rows (w zero) for the GPU uniform — the
/// kernel reads the very same numbers the CPU reference does.
pub fn spectral_basis_vec4() -> [[f32; 4]; 9] {
    let mut out = [[0.0; 4]; 9];
    for (dst, src) in out.iter_mut().zip(SPECTRAL_BASIS.iter()) {
        dst[..3].copy_from_slice(src);
    }
    out
}

/// Resolve a layer's live stack at layer time `lt` for a raster whose
/// diagonal is `diag_px` pixels; `px_scale` is raster pixels per comp pixel
/// (the §2.3 preview-resolution factor — 1.0 at full resolution), which
/// converts px@comp parameters exactly as `diag_px` converts % diag ones.
/// Placeholders, unknown names and bypassed effects resolve to nothing
/// (they render as identity, docs/03 §8).
pub fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
) -> Vec<Resolved> {
    effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
        .filter_map(|e| match e.effect.match_name.as_str() {
            "blur" => {
                let edge = match e.param("edge") {
                    Some(EffectValue::Choice(c)) => (*c).min(2),
                    _ => 1,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                // Instances saved before the mode existed carry no "mode"
                // parameter and resolve as Gaussian.
                let mode = match e.param("mode") {
                    Some(EffectValue::Choice(c)) => *c,
                    _ => 0,
                };
                if mode == 1 {
                    let length_pct = e.float_at("length", lt).unwrap_or(0.0) as f32;
                    let angle_deg = e.float_at("angle", lt).unwrap_or(0.0) as f32;
                    Some(Resolved::DirBlur {
                        length_px: (length_pct / 100.0 * diag_px).max(0.0),
                        angle_deg,
                        edge,
                        mix,
                    })
                } else {
                    let radius_pct = e.float_at("radius", lt)? as f32;
                    Some(Resolved::Blur {
                        radius_px: (radius_pct / 100.0 * diag_px).max(0.0),
                        edge,
                        mix,
                    })
                }
            }
            "sharpen" => {
                let amount = (e.float_at("amount", lt)? as f32 / 100.0).clamp(0.0, 3.0);
                let radius_pct = e.float_at("radius", lt)? as f32;
                let threshold =
                    (e.float_at("threshold", lt).unwrap_or(0.05) as f32).clamp(0.0, 1.0);
                let luma_only = match e.param("luminance_only") {
                    Some(EffectValue::Bool(b)) => *b,
                    _ => true,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Sharpen {
                    amount,
                    radius_px: (radius_pct / 100.0 * diag_px).max(0.0),
                    threshold,
                    luma_only,
                    mix,
                })
            }
            "rgb_split" => {
                let amount_pct = e.float_at("amount", lt)? as f32;
                let angle_deg = e.float_at("angle", lt).unwrap_or(0.0) as f32;
                let radial = match e.param("radial") {
                    Some(EffectValue::Bool(b)) => *b,
                    _ => false,
                };
                // Instances saved before the Wavelength mode existed carry
                // no such parameter and resolve as the classic split.
                let wavelength = match e.param("wavelength") {
                    Some(EffectValue::Bool(b)) => *b,
                    _ => false,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                let amount_px = (amount_pct / 100.0 * diag_px).max(0.0);
                Some(if wavelength {
                    Resolved::SpectralSplit {
                        amount_px,
                        angle_deg,
                        radial,
                        mix,
                    }
                } else {
                    Resolved::RgbSplit {
                        amount_px,
                        angle_deg,
                        radial,
                        mix,
                    }
                })
            }
            "flash" => {
                let decay_s = (e.float_at("decay", lt).unwrap_or(120.0) / 1000.0).max(0.0);
                let envelope = match e.param("trigger") {
                    Some(EffectValue::Float(p)) => flash_envelope(p, lt, decay_s),
                    _ => 0.0,
                };
                let intensity = e.float_at("intensity", lt).unwrap_or(100.0).max(0.0) / 100.0;
                let colour = e.colour_at("colour", lt).unwrap_or([1.0; 4]);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Flash {
                    strength: (envelope * intensity).clamp(0.0, 1.0) as f32,
                    colour: colour.map(|c| c as f32),
                    mix,
                })
            }
            "colour_balance" => {
                let rgb = |id: &str, neutral: f64| -> [f32; 3] {
                    let c = e.colour_at(id, lt).unwrap_or([neutral; 4]);
                    [c[0] as f32, c[1] as f32, c[2] as f32]
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::ColourBalance {
                    lift: rgb("lift", 0.0),
                    gamma: rgb("gamma", 1.0).map(|g| g.max(0.01)),
                    gain: rgb("gain", 1.0),
                    mix,
                })
            }
            "saturation" => {
                let saturation =
                    (e.float_at("saturation", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 2.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Saturation { saturation, mix })
            }
            "glow" => {
                let radius_pct = e.float_at("radius", lt).unwrap_or(8.0) as f32;
                let threshold = (e.float_at("threshold", lt).unwrap_or(1.0) as f32).max(0.0);
                let knee = (e.float_at("knee", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
                let intensity = (e.float_at("intensity", lt).unwrap_or(1.0) as f32).max(0.0);
                let tint = e.colour_at("tint", lt).unwrap_or([1.0; 4]);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Glow {
                    radius_px: (radius_pct / 100.0 * diag_px).max(0.0),
                    threshold,
                    knee,
                    intensity,
                    tint: tint.map(|c| c as f32),
                    mix,
                })
            }
            "shake" => {
                let amp_pct = (e.float_at("amplitude", lt).unwrap_or(1.5) as f32).max(0.0);
                let freq = e.float_at("frequency", lt).unwrap_or(8.0).max(0.0);
                let rot_amount = (e.float_at("rotation", lt).unwrap_or(1.0) as f32).max(0.0);
                let pump =
                    (e.float_at("zoom_pump", lt).unwrap_or(0.0) as f32 / 100.0).clamp(0.0, 1.0);
                let auto_scale = match e.param("auto_scale") {
                    Some(EffectValue::Bool(b)) => *b,
                    _ => true,
                };
                let seed = match e.param("seed") {
                    Some(EffectValue::Seed(s)) => *s,
                    _ => 0,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                // The wobble: four independent noise channels sampled at
                // local time × frequency (§3.4) — deterministic, hop-free,
                // identical on every machine (§2.4).
                let x = lt * freq;
                let amp_px = (amp_pct / 100.0 * diag_px).max(0.0);
                Some(Resolved::Shake {
                    offset_px: [
                        amp_px * shake_noise(seed, 0, x) as f32,
                        amp_px * shake_noise(seed, 1, x) as f32,
                    ],
                    rotation_deg: rot_amount * shake_noise(seed, 2, x) as f32,
                    zoom: 1.0 + pump * shake_noise(seed, 3, x) as f32,
                    amp_px,
                    rotation_max_deg: rot_amount,
                    zoom_min: 1.0 - pump,
                    auto_scale,
                    mix,
                })
            }
            "transform" => {
                // px@comp parameters scale by the preview factor (§2.3) so
                // Half preview frames exactly like Full, only softer.
                let px = |id: &str| e.float_at(id, lt).unwrap_or(0.0) as f32 * px_scale;
                let pct = |id: &str| e.float_at(id, lt).unwrap_or(100.0) as f32 / 100.0;
                let opacity =
                    (e.float_at("opacity", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Transform {
                    anchor: [px("anchor_x"), px("anchor_y")],
                    position: [px("position_x"), px("position_y")],
                    scale: [pct("scale_x"), pct("scale_y")],
                    rotation_deg: e.float_at("rotation", lt).unwrap_or(0.0) as f32,
                    opacity,
                    mix,
                })
            }
            _ => None,
        })
        .collect()
}

/// CPU reference implementations (docs/08 §1.6): identical semantics to the
/// WGSL kernels, plain and readable — the oracle the GPU must agree with.
pub mod cpu {
    use super::Resolved;

    /// Apply one resolved effect to an RGBA f32 image (premultiplied,
    /// linear light), in place.
    pub fn apply(rgba: &mut [f32], w: u32, h: u32, fx: &Resolved) {
        match fx {
            Resolved::Blur {
                radius_px,
                edge,
                mix,
            } => blur_gaussian(rgba, w, h, *radius_px, *edge, *mix),
            Resolved::DirBlur {
                length_px,
                angle_deg,
                edge,
                mix,
            } => blur_directional(rgba, w, h, *length_px, *angle_deg, *edge, *mix),
            Resolved::Sharpen {
                amount,
                radius_px,
                threshold,
                luma_only,
                mix,
            } => sharpen(
                rgba, w, h, *amount, *radius_px, *threshold, *luma_only, *mix,
            ),
            Resolved::RgbSplit {
                amount_px,
                angle_deg,
                radial,
                mix,
            } => rgb_split(rgba, w, h, *amount_px, *angle_deg, *radial, *mix),
            Resolved::SpectralSplit {
                amount_px,
                angle_deg,
                radial,
                mix,
            } => spectral_split(rgba, w, h, *amount_px, *angle_deg, *radial, *mix),
            Resolved::Flash {
                strength,
                colour,
                mix,
            } => flash(rgba, *strength, *colour, *mix),
            Resolved::ColourBalance {
                lift,
                gamma,
                gain,
                mix,
            } => colour_balance(rgba, *lift, *gamma, *gain, *mix),
            Resolved::Saturation { saturation, mix } => saturate(rgba, *saturation, *mix),
            Resolved::Transform {
                anchor,
                position,
                scale,
                rotation_deg,
                opacity,
                mix,
            } => transform(
                rgba,
                w,
                h,
                *anchor,
                *position,
                *scale,
                *rotation_deg,
                *opacity,
                *mix,
            ),
            Resolved::Glow {
                radius_px,
                threshold,
                knee,
                intensity,
                tint,
                mix,
            } => glow(
                rgba, w, h, *radius_px, *threshold, *knee, *intensity, *tint, *mix,
            ),
            // Shake is a transform-domain effect (docs/08 §3.4): the
            // resolved wobble maps to the Transform reference through the
            // same shared affine the GPU dispatch uses, so both paths
            // consume bit-identical numbers. A neutral shake (zero
            // amplitude, rotation and pump) maps to the identity affine —
            // the bit-exact passthrough the Transform reference pins.
            Resolved::Shake {
                offset_px,
                rotation_deg,
                zoom,
                amp_px,
                rotation_max_deg,
                zoom_min,
                auto_scale,
                mix,
            } => {
                let (anchor, position, scale, rot) = super::shake_affine(
                    w,
                    h,
                    *offset_px,
                    *rotation_deg,
                    *zoom,
                    *amp_px,
                    *rotation_max_deg,
                    *zoom_min,
                    *auto_scale,
                );
                transform(rgba, w, h, anchor, position, scale, rot, 1.0, *mix);
            }
        }
    }

    /// Glow (docs/08 §3.3, v1 core): bright-pass every premultiplied channel
    /// through [`super::glow_bright`] — alpha included, so the halo carries
    /// coverage and glow spreads over transparency like light — blur the
    /// leftover light with the shared gaussian (Repeat edges, fixed: the
    /// halo holds its strength along frame borders instead of dimming), then
    /// recombine additively in linear: `out = input + intensity · tint ·
    /// halo`, output alpha saturating at 1 (full coverage). Highlights are
    /// never clipped (§2.1). Intensity 0 is the effect's neutral point and
    /// short-circuits to the bit-exact identity (the WGSL twin matches).
    #[allow(clippy::too_many_arguments)]
    pub fn glow(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        radius_px: f32,
        threshold: f32,
        knee: f32,
        intensity: f32,
        tint: [f32; 4],
        mix: f32,
    ) {
        if intensity == 0.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        let original = rgba.to_vec();
        let mut halo = vec![0.0f32; rgba.len()];
        for (dst, src) in halo.iter_mut().zip(original.iter()) {
            *dst = super::glow_bright(*src, threshold, knee);
        }
        blur_gaussian(&mut halo, w, h, radius_px, 1, 1.0);
        for i in (0..rgba.len()).step_by(4) {
            let o = &original[i..i + 4];
            let hl = &halo[i..i + 4];
            for c in 0..3 {
                let glowed = o[c] + intensity * (hl[c] * tint[c]);
                rgba[i + c] = o[c] * (1.0 - mix) + glowed * mix;
            }
            let a = (o[3] + intensity * hl[3]).min(1.0);
            rgba[i + 3] = o[3] * (1.0 - mix) + a * mix;
        }
    }

    /// Transform (docs/08 §3.5, K-090): resample the input through the
    /// inverse of `position + R·S·(p − anchor)` — one bilinear tap per
    /// output pixel, transparent outside the frame, premultiplied
    /// throughout, with opacity multiplied into all four channels.
    /// Identity parameters reproduce the input bit-exactly: the inverse
    /// affine is exactly `q = p`, a bilinear tap at a pixel centre is
    /// exactly that pixel, and opacity/mix 1 multiply by exact 1.0 — the
    /// WGSL twin follows the identical arithmetic. A degenerate scale
    /// (|s| < 1e-6) renders fully transparent, never a division blow-up.
    #[allow(clippy::too_many_arguments)]
    pub fn transform(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        anchor: [f32; 2],
        position: [f32; 2],
        scale: [f32; 2],
        rotation_deg: f32,
        opacity: f32,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        // A collapsed (zero-scale) image is invisible: opacity 0, and the
        // sample point no longer matters (super::transform_op's rule).
        let (m, o, opacity) = super::transform_op(anchor, position, scale, rotation_deg, opacity);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let qx = m[0] * px + m[1] * py + o[0];
                let qy = m[2] * px + m[3] * py + o[1];
                let s = bilinear_edge(&original, w, h, qx, qy, 0);
                for c in 0..4 {
                    let v = s[c] * opacity;
                    rgba[i + c] = original[i + c] * (1.0 - mix) + v * mix;
                }
            }
        }
    }

    /// Colour balance (docs/08 §3.10 as amended by K-090): per-channel
    /// gain → lift → gamma in linear light on unpremultiplied colour (§2.2),
    /// re-premultiplied on the way out. Fully neutral parameters
    /// short-circuit the whole effect, so a Colour balance at defaults is
    /// the bit-exact identity rather than a round trip through `powf` and
    /// the unpremultiply divide. Negative light clamps at zero (that is
    /// what a crushing lift means); highlights are never clipped (§2.1).
    pub fn colour_balance(
        rgba: &mut [f32],
        lift: [f32; 3],
        gamma: [f32; 3],
        gain: [f32; 3],
        mix: f32,
    ) {
        if lift == [0.0; 3] && gamma == [1.0; 3] && gain == [1.0; 3] {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            let mut v = [0.0f32; 3];
            for c in 0..3 {
                let mut x = (u[c] * gain[c] + lift[c]).max(0.0);
                if gamma[c] != 1.0 {
                    x = x.powf(1.0 / gamma[c]);
                }
                v[c] = x;
            }
            for c in 0..3 {
                let graded = v[c] * a;
                px[c] = px[c] * (1.0 - mix) + graded * mix;
            }
        }
    }

    /// Saturation (docs/08 §3.10 as amended by K-090): scale colourfulness
    /// about Rec. 709 luma, in linear light on unpremultiplied colour
    /// (§2.2), re-premultiplied on the way out. Saturation 1 short-circuits
    /// the whole effect (bit-exact identity); 0 collapses to true greyscale.
    /// Named `saturate` so the parameter can keep the plain name.
    pub fn saturate(rgba: &mut [f32], saturation: f32, mix: f32) {
        if saturation == 1.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            let luma = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
            for c in 0..3 {
                let v = (luma + (u[c] - luma) * saturation).max(0.0);
                let s = v * a;
                px[c] = px[c] * (1.0 - mix) + s * mix;
            }
        }
    }

    /// Flash (docs/08 §3.7, manual form): blend each pixel toward the flash
    /// colour by the evaluated strength. The colour is scaled by the pixel's
    /// own alpha so the flash respects the layer's footprint (a transparent
    /// region never lights up); alpha itself is untouched.
    pub fn flash(rgba: &mut [f32], strength: f32, colour: [f32; 4], mix: f32) {
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            for c in 0..3 {
                let lit = px[c] * (1.0 - strength) + colour[c] * a * strength;
                px[c] = px[c] * (1.0 - mix) + lit * mix;
            }
        }
    }

    /// Rec. 709 luma weights, applied in linear light.
    pub const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

    /// The unpremultiplied colour of one premultiplied RGBA pixel. A fully
    /// transparent pixel's colour is undefined, so it reads as black — the
    /// WGSL kernels use the identical rule.
    fn unpremult(px: &[f32]) -> [f32; 3] {
        if px[3] > 0.0 {
            [px[0] / px[3], px[1] / px[3], px[2] / px[3]]
        } else {
            [0.0; 3]
        }
    }

    /// Soft threshold: detail within ±t collapses to zero, detail beyond it
    /// is shrunk by t — no hard step, so no contouring at the gate (§3.9's
    /// noise suppression). Written as explicit branches so the WGSL twin
    /// matches bit-for-bit.
    fn soft_gate(d: f32, t: f32) -> f32 {
        if d > t {
            d - t
        } else if d < -t {
            d + t
        } else {
            0.0
        }
    }

    /// Clamp-addressed bilinear sample at continuous pixel-centre
    /// coordinates (the texel at index x covers [x, x+1), centre x+0.5).
    /// Written with the exact arithmetic order the WGSL kernels use.
    fn bilinear(rgba: &[f32], w: u32, h: u32, sx: f32, sy: f32) -> [f32; 4] {
        let fx = sx - 0.5;
        let fy = sy - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (wi, hi) = (w as i64, h as i64);
        let at = |x: i64, y: i64| {
            let s = ((y.clamp(0, hi - 1) * wi + x.clamp(0, wi - 1)) * 4) as usize;
            [rgba[s], rgba[s + 1], rgba[s + 2], rgba[s + 3]]
        };
        let (x0, y0) = (x0 as i64, y0 as i64);
        let c00 = at(x0, y0);
        let c10 = at(x0 + 1, y0);
        let c01 = at(x0, y0 + 1);
        let c11 = at(x0 + 1, y0 + 1);
        let mut out = [0.0f32; 4];
        for c in 0..4 {
            let top = c00[c] * (1.0 - tx) + c10[c] * tx;
            let bottom = c01[c] * (1.0 - tx) + c11[c] * tx;
            out[c] = top * (1.0 - ty) + bottom * ty;
        }
        out
    }

    /// Chromatic aberration (docs/08 §3.6): R samples behind the offset, B
    /// ahead of it, G and alpha stay put (alpha follows the green channel so
    /// mattes never fringe). Linear mode shifts every pixel by the same
    /// vector; radial mode scales the pixel's own offset from the frame
    /// centre so aberration grows toward the corners (`amount_px` is reached
    /// at the corner distance). Premultiplied throughout; samples outside
    /// the frame clamp to the edge.
    pub fn rgb_split(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        amount_px: f32,
        angle_deg: f32,
        radial: bool,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        let (dx, dy) = super::rgb_split_offset(amount_px, angle_deg);
        let (fw, fh) = (w as f32, h as f32);
        let diag = (fw * fw + fh * fh).sqrt();
        let k = amount_px / (0.5 * diag);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);
                let (ox, oy) = if radial {
                    ((pos.0 - fw * 0.5) * k, (pos.1 - fh * 0.5) * k)
                } else {
                    (dx, dy)
                };
                let r = bilinear(&original, w, h, pos.0 - ox, pos.1 - oy)[0];
                let b = bilinear(&original, w, h, pos.0 + ox, pos.1 + oy)[2];
                let split = [r, original[i + 1], b, original[i + 3]];
                for c in 0..4 {
                    rgba[i + c] = original[i + c] * (1.0 - mix) + split[c] * mix;
                }
            }
        }
    }

    /// The RGB split's Wavelength mode (docs/08 §3.6, K-090): instead of
    /// three channels at three offsets, nine spectral samples spread across
    /// `±offset` (tap i at fraction i/4 − 1), each weighted by its
    /// wavelength's linear-RGB basis colour ([`super::SPECTRAL_BASIS`]) and
    /// summed — real dispersion's rainbow fringe rather than the classic
    /// hard R/G/B rim. The basis columns are normalised, so a uniform image
    /// passes through unchanged. Offsets (linear or radial) and edge
    /// handling match the classic mode exactly; alpha still follows the
    /// green channel's rule and stays put, so mattes never fringe.
    pub fn spectral_split(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        amount_px: f32,
        angle_deg: f32,
        radial: bool,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        let (dx, dy) = super::rgb_split_offset(amount_px, angle_deg);
        let (fw, fh) = (w as f32, h as f32);
        let diag = (fw * fw + fh * fh).sqrt();
        let k = amount_px / (0.5 * diag);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);
                let (ox, oy) = if radial {
                    ((pos.0 - fw * 0.5) * k, (pos.1 - fh * 0.5) * k)
                } else {
                    (dx, dy)
                };
                let mut acc = [0.0f32; 3];
                for (tap, weight) in super::SPECTRAL_BASIS.iter().enumerate() {
                    let t = tap as f32 * 0.25 - 1.0;
                    let s = bilinear(&original, w, h, pos.0 + t * ox, pos.1 + t * oy);
                    for c in 0..3 {
                        acc[c] += weight[c] * s[c];
                    }
                }
                let split = [acc[0], acc[1], acc[2], original[i + 3]];
                for c in 0..4 {
                    rgba[i + c] = original[i + c] * (1.0 - mix) + split[c] * mix;
                }
            }
        }
    }

    /// Unsharp mask (docs/08 §3.9) in linear light on unpremultiplied colour
    /// (§2.2): detail = input − gaussian(input, radius), gated by the soft
    /// threshold, scaled by amount and added back. The internal gaussian
    /// always uses Repeat edges (blurring unpremultiplied colour against
    /// transparent borders would invent dark detail). Undershoot clamps at
    /// zero — negative light is not a thing — and alpha passes through.
    #[allow(clippy::too_many_arguments)]
    pub fn sharpen(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        amount: f32,
        radius_px: f32,
        threshold: f32,
        luma_only: bool,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        // Unpremultiplied colour buffer, alpha carried along for the ride.
        let mut blurred = vec![0.0f32; rgba.len()];
        for (dst, src) in blurred.chunks_exact_mut(4).zip(original.chunks_exact(4)) {
            dst[..3].copy_from_slice(&unpremult(src));
            dst[3] = src[3];
        }
        blur_gaussian(&mut blurred, w, h, radius_px, 1, 1.0);
        for i in (0..rgba.len()).step_by(4) {
            let o = &original[i..i + 4];
            let u = unpremult(o);
            let b = &blurred[i..i + 3];
            let mut v = [0.0f32; 3];
            if luma_only {
                let d = soft_gate(
                    (u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2])
                        - (b[0] * LUMA[0] + b[1] * LUMA[1] + b[2] * LUMA[2]),
                    threshold,
                );
                for c in 0..3 {
                    v[c] = u[c] + amount * d;
                }
            } else {
                for c in 0..3 {
                    v[c] = u[c] + amount * soft_gate(u[c] - b[c], threshold);
                }
            }
            for c in 0..3 {
                let s = v[c].max(0.0) * o[3];
                rgba[i + c] = o[c] * (1.0 - mix) + s * mix;
            }
            rgba[i + 3] = o[3];
        }
    }

    /// Gaussian tap weights for a half-width `r` (σ = r/2, the visible
    /// extent reading), normalised. r = 0 → identity single tap.
    pub fn gaussian_weights(radius_px: f32) -> Vec<f32> {
        let r = radius_px.ceil().max(0.0) as i32;
        if r == 0 {
            return vec![1.0];
        }
        let sigma = (radius_px * 0.5).max(1e-3);
        let mut w: Vec<f32> = (-r..=r)
            .map(|i| (-0.5 * (i as f32 / sigma).powi(2)).exp())
            .collect();
        let sum: f32 = w.iter().sum();
        for v in &mut w {
            *v /= sum;
        }
        w
    }

    /// Resolve a sample index under an edge policy; None = transparent.
    fn edge_index(i: i64, len: i64, edge: u32) -> Option<i64> {
        if (0..len).contains(&i) {
            return Some(i);
        }
        match edge {
            1 => Some(i.clamp(0, len - 1)), // repeat edge pixel
            2 => {
                // mirror: reflect without repeating the edge sample
                let m = if i < 0 { -i } else { 2 * (len - 1) - i };
                Some(m.clamp(0, len - 1))
            }
            _ => None, // transparent
        }
    }

    /// The directional blur's tap count for a streak length in pixels —
    /// shared with the GPU op construction so both paths dispatch the same
    /// kernel size (§1.6).
    pub fn dir_blur_taps(length_px: f32) -> i32 {
        (length_px.ceil() as i32).clamp(1, 511)
    }

    /// Bilinear sample under a blur edge policy: out-of-frame taps repeat or
    /// mirror per axis, or read as transparent (contributing nothing while
    /// keeping full weight, exactly like the gaussian's normalisation).
    fn bilinear_edge(rgba: &[f32], w: u32, h: u32, sx: f32, sy: f32, edge: u32) -> [f32; 4] {
        let fx = sx - 0.5;
        let fy = sy - 0.5;
        let x0 = fx.floor();
        let y0 = fy.floor();
        let tx = fx - x0;
        let ty = fy - y0;
        let (wi, hi) = (w as i64, h as i64);
        let at = |x: i64, y: i64| match (edge_index(x, wi, edge), edge_index(y, hi, edge)) {
            (Some(x), Some(y)) => {
                let s = ((y * wi + x) * 4) as usize;
                [rgba[s], rgba[s + 1], rgba[s + 2], rgba[s + 3]]
            }
            _ => [0.0; 4],
        };
        let (x0, y0) = (x0 as i64, y0 as i64);
        let c00 = at(x0, y0);
        let c10 = at(x0 + 1, y0);
        let c01 = at(x0, y0 + 1);
        let c11 = at(x0 + 1, y0 + 1);
        let mut out = [0.0f32; 4];
        for c in 0..4 {
            let top = c00[c] * (1.0 - tx) + c10[c] * tx;
            let bottom = c01[c] * (1.0 - tx) + c11[c] * tx;
            out[c] = top * (1.0 - ty) + bottom * ty;
        }
        out
    }

    /// Directional blur (docs/08 §3.8): a line integral along the angle —
    /// evenly spaced bilinear taps across a segment `length_px` long centred
    /// on the pixel, box weighted, normalised over the full kernel whatever
    /// the edge policy (matching the gaussian's rule). Fixed tap order for
    /// determinism (§2.4).
    pub fn blur_directional(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        length_px: f32,
        angle_deg: f32,
        edge: u32,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        let (dx, dy) = super::rgb_split_offset(1.0, angle_deg); // unit vector
        let n = dir_blur_taps(length_px);
        let nf = n as f32;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);
                let mut acc = [0.0f32; 4];
                for k in 0..n {
                    let t = ((k as f32 + 0.5) / nf - 0.5) * length_px;
                    let s = bilinear_edge(&original, w, h, pos.0 + t * dx, pos.1 + t * dy, edge);
                    for c in 0..4 {
                        acc[c] += s[c];
                    }
                }
                for c in 0..4 {
                    let v = acc[c] / nf;
                    rgba[i + c] = original[i + c] * (1.0 - mix) + v * mix;
                }
            }
        }
    }

    /// Separable two-pass gaussian on premultiplied RGBA (docs/08 §3.8),
    /// fixed tap order for determinism (§2.4).
    pub fn blur_gaussian(rgba: &mut [f32], w: u32, h: u32, radius_px: f32, edge: u32, mix: f32) {
        let (w, h) = (w as i64, h as i64);
        let weights = gaussian_weights(radius_px);
        let r = (weights.len() / 2) as i64;
        if r == 0 && (mix - 1.0).abs() < f32::EPSILON {
            return;
        }
        let original = rgba.to_vec();
        let mut pass = vec![0.0f32; rgba.len()];
        // Horizontal.
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 4];
                for (k, wt) in weights.iter().enumerate() {
                    if let Some(sx) = edge_index(x + k as i64 - r, w, edge) {
                        let s = ((y * w + sx) * 4) as usize;
                        for c in 0..4 {
                            acc[c] += rgba[s + c] * wt;
                        }
                    }
                }
                let d = ((y * w + x) * 4) as usize;
                pass[d..d + 4].copy_from_slice(&acc);
            }
        }
        // Vertical, blending the host Mix against the untouched input.
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 4];
                for (k, wt) in weights.iter().enumerate() {
                    if let Some(sy) = edge_index(y + k as i64 - r, h, edge) {
                        let s = ((sy * w + x) * 4) as usize;
                        for c in 0..4 {
                            acc[c] += pass[s + c] * wt;
                        }
                    }
                }
                let d = ((y * w + x) * 4) as usize;
                for c in 0..4 {
                    rgba[d + c] = original[d + c] * (1.0 - mix) + acc[c] * mix;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn instantiate_carries_declared_defaults() {
        let e = instantiate("blur").unwrap();
        assert_eq!(e.effect.match_name, "blur");
        assert_eq!(e.effect.version, 1);
        assert!(e.enabled);
        assert_eq!(e.float_at("radius", 0.0), Some(1.5));
        assert_eq!(e.float_at("mix", 0.0), Some(100.0));
        assert!(matches!(e.param("edge"), Some(EffectValue::Choice(1))));
        assert!(instantiate("nonsense").is_none());
    }

    #[test]
    fn resolve_stack_evaluates_converts_and_skips_dead_effects() {
        let mut e = instantiate("blur").unwrap();
        // 1.5% of a 1000px diagonal = 15px.
        let r = resolve_stack(&[e.clone()], 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Blur {
                radius_px: 15.0,
                edge: 1,
                mix: 1.0
            }]
        );
        e.enabled = false;
        assert!(resolve_stack(&[e.clone()], 0.0, 1000.0, 1.0).is_empty());
        e.enabled = true;
        e.effect.namespace = EffectNamespace::Placeholder;
        assert!(
            resolve_stack(&[e], 0.0, 1000.0, 1.0).is_empty(),
            "placeholders render as identity"
        );
    }

    #[test]
    fn cpu_blur_identity_energy_and_mix() {
        // A 9x9 with one bright premultiplied pixel in the middle.
        let (w, h) = (9u32, 9u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        let mid = ((4 * w + 4) * 4) as usize;
        img[mid..mid + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]); // HDR > 1

        // Radius 0 is the identity.
        let mut id = img.clone();
        cpu::blur_gaussian(&mut id, w, h, 0.0, 1, 1.0);
        assert_eq!(id, img);

        // A blur spreads but conserves energy away from edges (repeat policy,
        // small radius, bright pixel far from borders).
        let mut blurred = img.clone();
        cpu::blur_gaussian(&mut blurred, w, h, 2.0, 1, 1.0);
        assert!(blurred[mid] < img[mid], "peak flattens");
        let sum = |v: &[f32]| v.iter().step_by(4).sum::<f32>(); // red plane
        assert!((sum(&blurred) - sum(&img)).abs() < 1e-3, "energy conserved");

        // Mix 0 returns the input exactly, whatever the radius.
        let mut mixed = img.clone();
        cpu::blur_gaussian(&mut mixed, w, h, 5.0, 1, 0.0);
        assert_eq!(mixed, img);

        // Transparent edges lose energy when the kernel hangs off the border.
        let mut corner = vec![0.0f32; (w * h * 4) as usize];
        corner[0..4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        let mut t = corner.clone();
        cpu::blur_gaussian(&mut t, w, h, 3.0, 0, 1.0);
        let mut rep = corner;
        cpu::blur_gaussian(&mut rep, w, h, 3.0, 1, 1.0);
        assert!(sum(&t) < sum(&rep), "transparent edge sheds energy");
    }

    #[test]
    fn sharpen_instantiates_and_resolves() {
        let e = instantiate("sharpen").unwrap();
        assert_eq!(e.float_at("amount", 0.0), Some(60.0));
        assert_eq!(e.float_at("radius", 0.0), Some(0.4));
        assert_eq!(e.float_at("threshold", 0.0), Some(0.05));
        assert!(matches!(
            e.param("luminance_only"),
            Some(EffectValue::Bool(true))
        ));
        // 0.4% of a 1000px diagonal = 4px; amount 60% = 0.6.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Sharpen {
                amount: 0.6,
                radius_px: 4.0,
                threshold: 0.05,
                luma_only: true,
                mix: 1.0
            }]
        );
    }

    /// A step edge for sharpen tests: left half dark, right half bright,
    /// fully opaque, with an HDR right side.
    fn step_image(w: u32, h: u32) -> Vec<f32> {
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let v = if x < w / 2 { 0.2 } else { 2.0 };
                img[i..i + 4].copy_from_slice(&[v, v * 0.5, v * 0.25, 1.0]);
            }
        }
        img
    }

    #[test]
    fn cpu_sharpen_identity_edge_overshoot_and_threshold() {
        let (w, h) = (16u32, 8u32);
        let img = step_image(w, h);

        // Mix 0 is the exact identity.
        let mut m0 = img.clone();
        cpu::sharpen(&mut m0, w, h, 1.0, 3.0, 0.0, true, 0.0);
        assert_eq!(m0, img);

        // Amount 0 changes nothing (opaque pixels, so unpremultiply is exact).
        let mut a0 = img.clone();
        cpu::sharpen(&mut a0, w, h, 0.0, 3.0, 0.0, true, 1.0);
        for (a, b) in a0.iter().zip(&img) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }

        // A flat region is untouched; the step edge overshoots both ways.
        let mut s = img.clone();
        cpu::sharpen(&mut s, w, h, 1.0, 2.0, 0.0, true, 1.0);
        let px = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let far = px(1, 4);
        assert!((s[far] - img[far]).abs() < 1e-4, "flat area stays put");
        let dark_side = px(w / 2 - 1, 4);
        let bright_side = px(w / 2, 4);
        assert!(s[dark_side] < img[dark_side], "dark side of edge dips");
        assert!(s[bright_side] > img[bright_side], "bright side lifts");

        // A threshold above the edge contrast suppresses the sharpening.
        let mut t = img.clone();
        cpu::sharpen(&mut t, w, h, 1.0, 2.0, 1.0, true, 1.0);
        for (a, b) in t.iter().zip(&img) {
            assert!((a - b).abs() < 1e-5, "threshold 1.0 gates the edge detail");
        }

        // Fully transparent input stays fully transparent (no invented light).
        let mut clear = vec![0.0f32; (w * h * 4) as usize];
        cpu::sharpen(&mut clear, w, h, 3.0, 2.0, 0.0, false, 1.0);
        assert!(clear.iter().all(|v| *v == 0.0));

        // Per-channel mode fringes where luma-only does not: on a pure
        // chroma edge (constant luma), luma-only is inert.
        let mut chroma = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                // Two colours with identical Rec. 709 luma.
                let (r, g, b) = if x < w / 2 {
                    (0.5, 0.25, 0.0)
                } else {
                    let r = 0.1f32;
                    let b = 0.4f32;
                    let g = (0.5 * cpu::LUMA[0] + 0.25 * cpu::LUMA[1] - r * cpu::LUMA[0]
                        + 0.0 * cpu::LUMA[2]
                        - b * cpu::LUMA[2])
                        / cpu::LUMA[1];
                    (r, g, b)
                };
                chroma[i..i + 4].copy_from_slice(&[r, g, b, 1.0]);
            }
        }
        let mut luma_pass = chroma.clone();
        cpu::sharpen(&mut luma_pass, w, h, 2.0, 2.0, 0.0, true, 1.0);
        let mut chan_pass = chroma.clone();
        cpu::sharpen(&mut chan_pass, w, h, 2.0, 2.0, 0.0, false, 1.0);
        let dev = |out: &[f32]| {
            out.iter()
                .zip(&chroma)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(dev(&luma_pass) < 1e-4, "luma-only ignores chroma edges");
        assert!(dev(&chan_pass) > 0.05, "per-channel mode sharpens them");
    }

    #[test]
    fn rgb_split_instantiates_and_resolves() {
        let e = instantiate("rgb_split").unwrap();
        assert_eq!(e.float_at("amount", 0.0), Some(0.4));
        assert_eq!(e.float_at("angle", 0.0), Some(0.0));
        assert!(matches!(e.param("radial"), Some(EffectValue::Bool(false))));
        // 0.4% of a 1000px diagonal = 4px.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::RgbSplit {
                amount_px: 4.0,
                angle_deg: 0.0,
                radial: false,
                mix: 1.0
            }]
        );
    }

    #[test]
    fn cpu_rgb_split_shifts_channels_and_keeps_alpha() {
        // A white impulse in the middle of a black opaque frame.
        let (w, h) = (17u32, 9u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for px in img.chunks_exact_mut(4) {
            px[3] = 1.0;
        }
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let mid = at(8, 4);
        img[mid..mid + 3].copy_from_slice(&[1.0, 1.0, 1.0]);

        // Amount 0 and mix 0 are both the exact identity.
        let mut a0 = img.clone();
        cpu::rgb_split(&mut a0, w, h, 0.0, 0.0, false, 1.0);
        assert_eq!(a0, img);
        let mut m0 = img.clone();
        cpu::rgb_split(&mut m0, w, h, 3.0, 45.0, false, 0.0);
        assert_eq!(m0, img);

        // Angle 0°, 2px: red lands 2px right of the impulse, blue 2px left,
        // green and alpha exactly where they were.
        let mut s = img.clone();
        cpu::rgb_split(&mut s, w, h, 2.0, 0.0, false, 1.0);
        assert_eq!(s[at(10, 4)], 1.0, "red shifted +x");
        assert_eq!(s[at(8, 4)], 0.0, "red left the impulse");
        assert_eq!(s[at(6, 4) + 2], 1.0, "blue shifted -x");
        assert_eq!(s[at(8, 4) + 1], 1.0, "green stays");
        assert!(
            s.iter().skip(3).step_by(4).all(|a| *a == 1.0),
            "alpha follows green: untouched"
        );

        // Radial: the exact centre pixel is unmoved even at a huge amount.
        let mut c = img.clone();
        // Centre the impulse for the radial test (odd dimensions: the middle
        // pixel's centre is the frame centre).
        cpu::rgb_split(&mut c, w, h, 20.0, 0.0, true, 1.0);
        assert_eq!(c[mid], 1.0, "frame-centre red is unmoved");
        assert_eq!(c[mid + 2], 1.0, "frame-centre blue is unmoved");
    }

    #[test]
    fn rgb_split_wavelength_bool_selects_the_variant() {
        // A fresh instance defaults to the classic split — and resolves to
        // the exact same Resolved value it did before the Bool existed.
        let mut e = instantiate("rgb_split").unwrap();
        assert!(matches!(
            e.param("wavelength"),
            Some(EffectValue::Bool(false))
        ));
        let classic = Resolved::RgbSplit {
            amount_px: 4.0,
            angle_deg: 0.0,
            radial: false,
            mix: 1.0,
        };
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(r, vec![classic]);

        // Wavelength on: the same numbers arrive as SpectralSplit.
        for p in &mut e.params {
            if p.id == "wavelength" {
                p.value = EffectValue::Bool(true);
            }
        }
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::SpectralSplit {
                amount_px: 4.0,
                angle_deg: 0.0,
                radial: false,
                mix: 1.0
            }]
        );

        // A legacy instance (saved before the Bool existed) has no
        // wavelength parameter and still resolves as the classic split.
        e.params.retain(|p| p.id != "wavelength");
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(r, vec![classic]);
    }

    #[test]
    fn spectral_basis_columns_sum_to_one() {
        // The normalisation that makes a uniform image pass through
        // unchanged: each channel's nine weights sum to 1 (within f32
        // rounding of the summation itself).
        for c in 0..3 {
            let sum: f32 = SPECTRAL_BASIS.iter().map(|w| w[c]).sum();
            assert!((sum - 1.0).abs() < 1e-6, "channel {c} sums to {sum}");
        }
    }

    #[test]
    fn cpu_spectral_split_disperses_and_preserves_uniform() {
        let (w, h) = (17u32, 9u32);
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;

        // A uniform image is unchanged (the basis is normalised, and clamp
        // addressing keeps edges uniform too).
        let mut uniform = vec![0.0f32; (w * h * 4) as usize];
        for px in uniform.chunks_exact_mut(4) {
            px.copy_from_slice(&[0.5, 0.25, 0.125, 1.0]);
        }
        let before = uniform.clone();
        cpu::spectral_split(&mut uniform, w, h, 3.0, 25.0, false, 1.0);
        for (i, (a, b)) in uniform.iter().zip(&before).enumerate() {
            assert!((a - b).abs() < 1e-6, "texel {i}: {a} vs {b}");
        }

        // A white impulse on an opaque black frame disperses: red mass
        // lands ahead of the impulse (the classic mode's R direction), blue
        // behind, green astride it — and alpha never moves.
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for px in img.chunks_exact_mut(4) {
            px[3] = 1.0;
        }
        let mid = at(8, 4);
        img[mid..mid + 3].copy_from_slice(&[1.0, 1.0, 1.0]);

        // Mix 0 is the exact identity.
        let mut m0 = img.clone();
        cpu::spectral_split(&mut m0, w, h, 3.0, 45.0, false, 0.0);
        assert_eq!(m0, img);

        let mut s = img.clone();
        cpu::spectral_split(&mut s, w, h, 2.0, 0.0, false, 1.0);
        assert!(s[at(10, 4)] > 0.1, "red end lands +2x of the impulse");
        assert!(s[at(6, 4) + 2] > 0.3, "blue end lands -2x of the impulse");
        assert!(s[mid + 1] > 0.3, "green stays astride the impulse");
        assert!(s[at(10, 4) + 2] < 1e-6, "no blue leaks toward the red end");
        assert!(
            s.iter().skip(3).step_by(4).all(|a| *a == 1.0),
            "alpha stays put: mattes never fringe"
        );
    }

    #[test]
    fn flash_envelope_decays_hits_and_holds_statics() {
        use crate::anim::{Keyframe, SideInterp};
        use crate::time::Rational;
        // A static trigger is a constant flash.
        assert_eq!(flash_envelope(&Property::fixed(0.5), 7.0, 0.12), 0.5);
        assert_eq!(flash_envelope(&Property::fixed(2.0), 0.0, 0.12), 1.0);

        // Keyframed: hits at t=1 (full) and t=2 (0.6), decay 0.5s.
        let key = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let trig = Property {
            animation: Animation::Keyframed(vec![key(1, 1.0), key(2, 0.6)]),
            extra: serde_json::Map::new(),
        };
        assert_eq!(flash_envelope(&trig, 0.5, 0.5), 0.0, "before the first hit");
        assert_eq!(
            flash_envelope(&trig, 1.0, 0.5),
            1.0,
            "full on the hit frame"
        );
        let half_later = flash_envelope(&trig, 1.5, 0.5);
        assert!(
            (half_later - (-1.0f64).exp()).abs() < 1e-12,
            "1/e after one decay constant"
        );
        assert_eq!(
            flash_envelope(&trig, 2.0, 0.5),
            0.6,
            "second hit wins over the tail"
        );
        // Overlap takes the loudest: right after t=2 the first hit's tail
        // (1.0·e^-2) is quieter than the fresh 0.6 hit.
        let after = flash_envelope(&trig, 2.1, 0.5);
        assert!((after - 0.6 * (-0.2f64).exp()).abs() < 1e-12);

        // Decay 0 flashes only on the exact hit time.
        assert_eq!(flash_envelope(&trig, 1.0, 0.0), 1.0);
        assert_eq!(flash_envelope(&trig, 1.01, 0.0), 0.0);
    }

    #[test]
    fn flash_instantiates_resolves_and_lights_within_the_footprint() {
        let e = instantiate("flash").unwrap();
        assert_eq!(e.float_at("trigger", 0.0), Some(0.0));
        assert_eq!(e.float_at("intensity", 0.0), Some(100.0));
        assert_eq!(e.float_at("decay", 0.0), Some(120.0));
        assert_eq!(e.colour_at("colour", 0.0), Some([1.0, 1.0, 1.0, 1.0]));
        // Trigger 0: resolves to a zero-strength (identity) flash — the
        // §1.2 trigger-driven exemption.
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Flash {
                strength: 0.0,
                colour: [1.0; 4],
                mix: 1.0
            }]
        );

        // CPU semantics: strength 1 paints the footprint the flash colour.
        let mut img = vec![
            0.5, 0.25, 0.1, 1.0, // opaque pixel
            0.2, 0.1, 0.05, 0.5, // half-transparent pixel
            0.0, 0.0, 0.0, 0.0, // empty pixel
        ];
        let before = img.clone();
        cpu::flash(&mut img, 1.0, [2.0, 1.0, 0.5, 1.0], 1.0);
        assert_eq!(&img[0..4], &[2.0, 1.0, 0.5, 1.0], "opaque: flash colour");
        assert_eq!(
            &img[4..8],
            &[1.0, 0.5, 0.25, 0.5],
            "half alpha: premultiplied flash"
        );
        assert_eq!(&img[8..12], &[0.0; 4], "empty pixels never light up");

        // Strength 0 and mix 0 are both the exact identity.
        let mut s0 = before.clone();
        cpu::flash(&mut s0, 0.0, [1.0; 4], 1.0);
        assert_eq!(s0, before);
        let mut m0 = before.clone();
        cpu::flash(&mut m0, 1.0, [1.0; 4], 0.0);
        assert_eq!(m0, before);
    }

    #[test]
    fn colour_balance_instantiates_and_resolves_neutral() {
        let e = instantiate("colour_balance").unwrap();
        assert_eq!(e.colour_at("lift", 0.0), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(e.colour_at("gamma", 0.0), Some([1.0; 4]));
        assert_eq!(e.colour_at("gain", 0.0), Some([1.0; 4]));
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::ColourBalance {
                lift: [0.0; 3],
                gamma: [1.0; 3],
                gain: [1.0; 3],
                mix: 1.0
            }]
        );
    }

    #[test]
    fn saturation_instantiates_and_resolves_neutral() {
        let e = instantiate("saturation").unwrap();
        assert_eq!(e.float_at("saturation", 0.0), Some(100.0));
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Saturation {
                saturation: 1.0,
                mix: 1.0
            }]
        );
    }

    /// One opaque mid-grey-ish pixel, one half-alpha, one HDR, one empty —
    /// the colour-effect test quartet.
    fn colour_quartet() -> Vec<f32> {
        vec![
            0.25, 0.5, 0.1, 1.0, //
            0.1, 0.2, 0.05, 0.5, //
            4.0, 2.0, 1.0, 1.0, //
            0.0, 0.0, 0.0, 0.0,
        ]
    }

    #[test]
    fn cpu_colour_balance_stages_behave() {
        let img = colour_quartet();

        // A neutral balance is the bit-exact identity (K-090 split: the
        // whole effect short-circuits, no unpremultiply round trip).
        let mut n = img.clone();
        cpu::colour_balance(&mut n, [0.0; 3], [1.0; 3], [1.0; 3], 1.0);
        assert_eq!(n, img);

        // Mix 0 is the exact identity whatever the balance.
        let mut m0 = img.clone();
        cpu::colour_balance(&mut m0, [0.5; 3], [2.0; 3], [3.0; 3], 0.0);
        assert_eq!(m0, img);

        // Gain doubles linear values; HDR stays unclipped (§2.1).
        let mut g = img.clone();
        cpu::colour_balance(&mut g, [0.0; 3], [1.0; 3], [2.0; 3], 1.0);
        assert_eq!(g[0], 0.5);
        assert_eq!(g[8], 8.0, "highlights never clip");

        // Lift raises blacks (empty alpha stays empty: premultiplied zero).
        let mut l = img.clone();
        cpu::colour_balance(&mut l, [0.1; 3], [1.0; 3], [1.0; 3], 1.0);
        assert!((l[2] - 0.2).abs() < 1e-6, "0.1 blue lifted by 0.1");
        assert_eq!(&l[12..16], &[0.0; 4], "empty pixels stay empty");

        // Gamma 2 is a square root in linear: 0.25 → 0.5.
        let mut ga = img.clone();
        cpu::colour_balance(&mut ga, [0.0; 3], [2.0; 3], [1.0; 3], 1.0);
        assert!((ga[0] - 0.5).abs() < 1e-6);

        // Alpha is untouched by any of it.
        for v in [&n, &m0, &g, &l, &ga] {
            assert_eq!(v[3], 1.0);
            assert_eq!(v[7], 0.5);
        }
    }

    #[test]
    fn cpu_saturation_behaves() {
        let img = colour_quartet();

        // Saturation 1 is the bit-exact identity (whole-effect
        // short-circuit, K-090 split).
        let mut n = img.clone();
        cpu::saturate(&mut n, 1.0, 1.0);
        assert_eq!(n, img);

        // Mix 0 is the exact identity whatever the saturation.
        let mut m0 = img.clone();
        cpu::saturate(&mut m0, 0.0, 0.0);
        assert_eq!(m0, img);

        // Saturation 0 collapses to Rec. 709 luma (true greyscale).
        let mut s = img.clone();
        cpu::saturate(&mut s, 0.0, 1.0);
        let luma = 0.25 * cpu::LUMA[0] + 0.5 * cpu::LUMA[1] + 0.1 * cpu::LUMA[2];
        for (c, v) in s.iter().take(3).enumerate() {
            assert!((v - luma).abs() < 1e-6, "channel {c} at luma");
        }
        // The half-alpha pixel desaturates in unpremultiplied space: its
        // premultiplied channels all land on (unpremult luma) × alpha.
        let luma_half = (0.2 * cpu::LUMA[0] + 0.4 * cpu::LUMA[1] + 0.1 * cpu::LUMA[2]) * 0.5;
        for c in 0..3 {
            assert!((s[4 + c] - luma_half).abs() < 1e-6, "channel {c}");
        }
        assert_eq!(&s[12..16], &[0.0; 4], "empty pixels stay empty");

        // Oversaturation spreads channels apart and clamps at zero, never
        // clipping highlights (§2.1).
        let mut o = img.clone();
        cpu::saturate(&mut o, 2.0, 1.0);
        assert!(o[1] > 0.5, "dominant green pushes up");
        assert!(o[2] >= 0.0, "recessive blue clamps at zero, not negative");
        assert!(o[8] > 4.0, "HDR red keeps its headroom");

        // Alpha is untouched by any of it.
        for v in [&n, &m0, &s, &o] {
            assert_eq!(v[3], 1.0);
            assert_eq!(v[7], 0.5);
        }
    }

    #[test]
    fn blur_mode_resolves_gaussian_directional_and_legacy() {
        // A fresh instance defaults to Gaussian and resolves exactly as the
        // pre-mode blur did.
        let mut e = instantiate("blur").unwrap();
        assert!(matches!(e.param("mode"), Some(EffectValue::Choice(0))));
        assert_eq!(e.float_at("length", 0.0), Some(10.0));
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Blur {
                radius_px: 15.0,
                edge: 1,
                mix: 1.0
            }]
        );

        // Directional mode reads Length/Angle instead (10% of 1000 = 100px).
        for p in &mut e.params {
            if p.id == "mode" {
                p.value = EffectValue::Choice(1);
            }
        }
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::DirBlur {
                length_px: 100.0,
                angle_deg: 0.0,
                edge: 1,
                mix: 1.0
            }]
        );

        // A legacy instance (saved before the mode existed) has no mode
        // parameter and still resolves as Gaussian.
        e.params
            .retain(|p| !matches!(p.id.as_str(), "mode" | "length" | "angle"));
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert!(matches!(r[..], [Resolved::Blur { .. }]));
    }

    #[test]
    fn cpu_directional_blur_streaks_along_the_angle() {
        // A white impulse in the middle of a transparent frame.
        let (w, h) = (17u32, 9u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let mid = at(8, 4);
        img[mid..mid + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);

        // Length 0 and mix 0 are both the exact identity.
        let mut l0 = img.clone();
        cpu::blur_directional(&mut l0, w, h, 0.0, 0.0, 1, 1.0);
        assert_eq!(l0, img);
        let mut m0 = img.clone();
        cpu::blur_directional(&mut m0, w, h, 6.0, 45.0, 1, 0.0);
        assert_eq!(m0, img);

        // Angle 0, length 5: the impulse smears along x only — energy
        // appears beside it on its own row, none above or below.
        let mut s = img.clone();
        cpu::blur_directional(&mut s, w, h, 5.0, 0.0, 1, 1.0);
        assert!(s[mid] < 1.0, "peak flattens");
        assert!(
            s[at(7, 4)] > 0.0 && s[at(9, 4)] > 0.0,
            "streak spreads in x"
        );
        assert_eq!(s[at(8, 3)], 0.0, "no bleed upward");
        assert_eq!(s[at(8, 5)], 0.0, "no bleed downward");
        // Box weights conserve energy away from edges (5 interior taps).
        let sum = |v: &[f32]| v.iter().step_by(4).sum::<f32>();
        assert!((sum(&s) - sum(&img)).abs() < 1e-4, "energy conserved");

        // Angle 90 streaks along y instead.
        let mut v = img.clone();
        cpu::blur_directional(&mut v, w, h, 5.0, 90.0, 1, 1.0);
        assert!(
            v[at(8, 3)] > 0.0 && v[at(8, 5)] > 0.0,
            "streak spreads in y"
        );
        assert!(v[at(7, 4)] < 1e-6, "x row stays clean");
    }

    #[test]
    fn transform_instantiates_and_resolves_with_the_preview_factor() {
        let e = instantiate("transform").unwrap();
        assert_eq!(e.float_at("anchor_x", 0.0), Some(0.0));
        assert_eq!(e.float_at("position_x", 0.0), Some(0.0));
        assert_eq!(e.float_at("scale_x", 0.0), Some(100.0));
        assert_eq!(e.float_at("rotation", 0.0), Some(0.0));
        assert_eq!(e.float_at("opacity", 0.0), Some(100.0));
        // Defaults resolve to the exact identity op.
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Transform {
                anchor: [0.0; 2],
                position: [0.0; 2],
                scale: [1.0; 2],
                rotation_deg: 0.0,
                opacity: 1.0,
                mix: 1.0
            }]
        );

        // px@comp parameters scale by the §2.3 preview factor; percentages
        // and degrees do not.
        let mut e = e;
        for p in &mut e.params {
            match p.id.as_str() {
                "anchor_x" => p.value = EffectValue::Float(Property::fixed(40.0)),
                "position_x" => p.value = EffectValue::Float(Property::fixed(100.0)),
                "scale_x" => p.value = EffectValue::Float(Property::fixed(200.0)),
                "rotation" => p.value = EffectValue::Float(Property::fixed(90.0)),
                _ => {}
            }
        }
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 500.0, 0.5);
        assert_eq!(
            r,
            vec![Resolved::Transform {
                anchor: [20.0, 0.0],
                position: [50.0, 0.0],
                scale: [2.0, 1.0],
                rotation_deg: 90.0,
                opacity: 1.0,
                mix: 1.0
            }]
        );
    }

    #[test]
    fn glow_instantiates_resolves_and_pins_the_one_sided_threshold() {
        // The K-090 poster child: the Threshold hard range is clamped at
        // zero below and unbounded above — HDR values glow harder.
        let s = schema("glow").unwrap();
        let threshold = s.params.iter().find(|p| p.id == "threshold").unwrap();
        assert!(matches!(
            threshold.kind,
            ParamKind::Float {
                hard: (Some(0.0), None),
                ..
            }
        ));

        let e = instantiate("glow").unwrap();
        assert_eq!(e.float_at("threshold", 0.0), Some(1.0));
        assert_eq!(e.float_at("knee", 0.0), Some(0.5));
        assert_eq!(e.float_at("radius", 0.0), Some(8.0));
        assert_eq!(e.float_at("intensity", 0.0), Some(1.0));
        assert_eq!(e.colour_at("tint", 0.0), Some([1.0; 4]));
        // 8% of a 1000px diagonal = 80px.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0);
        assert_eq!(
            r,
            vec![Resolved::Glow {
                radius_px: 80.0,
                threshold: 1.0,
                knee: 0.5,
                intensity: 1.0,
                tint: [1.0; 4],
                mix: 1.0
            }]
        );
    }

    #[test]
    fn glow_bright_gates_eases_and_passes_hdr() {
        // Below the threshold: nothing, knee or not.
        assert_eq!(glow_bright(0.5, 1.0, 0.0), 0.0);
        assert_eq!(glow_bright(0.5, 1.0, 0.5), 0.0);
        assert_eq!(glow_bright(1.0, 1.0, 0.5), 0.0);
        // Knee 0 is the hard subtract.
        assert_eq!(glow_bright(3.0, 1.0, 0.0), 2.0);
        // Inside the knee the onset is eased below the hard hinge.
        let eased = glow_bright(1.25, 1.0, 0.5);
        assert!(eased > 0.0 && eased < 0.25, "eased onset: {eased}");
        // Beyond threshold + knee the smoothstep saturates: hard subtract.
        assert_eq!(glow_bright(3.0, 1.0, 0.5), 2.0);
        // Monotone across the knee (no dips as the smoothstep engages).
        let mut prev = 0.0;
        for i in 0..=40 {
            let x = 0.4 + i as f32 * 0.05;
            let b = glow_bright(x, 1.0, 0.5);
            assert!(b >= prev, "monotone at x={x}");
            prev = b;
        }
    }

    #[test]
    fn cpu_glow_blooms_spreads_alpha_and_keeps_neutral_exact() {
        // An HDR spike on an opaque dark frame, plus a transparent border.
        let (w, h) = (17u32, 9u32);
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 2..w - 2 {
                let i = at(x, y);
                img[i..i + 4].copy_from_slice(&[0.1, 0.1, 0.1, 1.0]);
            }
        }
        let mid = at(8, 4);
        img[mid..mid + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);

        // Intensity 0 is the bit-exact identity (the neutral pin).
        let mut n = img.clone();
        cpu::glow(&mut n, w, h, 4.0, 1.0, 0.5, 0.0, [1.0; 4], 1.0);
        assert_eq!(n, img);

        // Mix 0 is the exact identity whatever the parameters.
        let mut m0 = img.clone();
        cpu::glow(&mut m0, w, h, 4.0, 0.2, 0.1, 2.0, [1.0; 4], 0.0);
        assert_eq!(m0, img);

        // A frame entirely below the threshold gains nothing: the halo is
        // zero everywhere and the add is exact.
        let dim = {
            let mut d = img.clone();
            d[mid..mid + 4].copy_from_slice(&[0.1, 0.1, 0.1, 1.0]);
            d
        };
        let mut quiet = dim.clone();
        cpu::glow(&mut quiet, w, h, 4.0, 1.0, 0.5, 1.0, [1.0; 4], 1.0);
        assert_eq!(quiet, dim);

        // The spike blooms: neighbours gain light, the spike itself gains
        // its own halo back (additive, §2.1: nothing clips).
        let mut g = img.clone();
        cpu::glow(&mut g, w, h, 3.0, 1.0, 0.5, 1.0, [1.0; 4], 1.0);
        assert!(g[at(10, 4)] > img[at(10, 4)], "neighbour catches the halo");
        assert!(g[mid] > img[mid], "the spike gains its own bloom");

        // The halo carries alpha over transparency: with a threshold low
        // enough that opaque coverage passes it, the transparent border
        // next to the footprint gains coverage — glow reads as light there.
        let mut a = img.clone();
        cpu::glow(&mut a, w, h, 3.0, 0.05, 0.0, 1.0, [1.0; 4], 1.0);
        assert!(a[at(1, 4) + 3] > 0.0, "coverage bloomed past the edge");
        assert!(a[at(8, 4) + 3] <= 1.0, "alpha saturates at full coverage");

        // Tint colours the halo, not the underlying image: with a red tint,
        // the transparent border gains red light only.
        let mut t = img.clone();
        cpu::glow(&mut t, w, h, 3.0, 0.05, 0.0, 1.0, [1.0, 0.0, 0.0, 1.0], 1.0);
        assert!(t[at(1, 4)] > 0.0, "red halo over the border");
        assert_eq!(t[at(1, 4) + 1], 0.0, "no green in a red-tinted halo");
    }

    #[test]
    fn shake_noise_is_deterministic_seeded_and_hop_free() {
        // Same inputs → same outputs, exactly (§2.4 determinism).
        for i in 0..50 {
            let x = i as f64 * 0.173;
            assert_eq!(shake_noise(7, 0, x), shake_noise(7, 0, x));
        }
        // Different seeds → different sequences; different channels too.
        assert_ne!(shake_noise(1, 0, 0.37), shake_noise(2, 0, 0.37));
        assert_ne!(shake_noise(1, 0, 0.37), shake_noise(1, 1, 0.37));
        // Bounded to [−1, 1] and actually moving.
        let mut spread = (f64::MAX, f64::MIN);
        for i in 0..500 {
            let v = shake_noise(11, 2, i as f64 * 0.31);
            assert!(v.abs() <= 1.0, "bounded at x={i}: {v}");
            spread = (spread.0.min(v), spread.1.max(v));
        }
        assert!(spread.1 - spread.0 > 0.5, "the wobble wanders: {spread:?}");
        // Hop-free: tiny steps in time give tiny steps in value, across
        // lattice boundaries included (the smoothstep is C¹ there).
        for i in 0..400 {
            let x = i as f64 * 0.01;
            let dv = (shake_noise(3, 1, x + 1e-4) - shake_noise(3, 1, x)).abs();
            assert!(dv < 1e-2, "no hop at x={x}: step {dv}");
        }
    }

    #[test]
    fn shake_cover_scale_keeps_every_worst_case_corner_inside() {
        // For a sweep of parameter sets, the inverse map of every frame
        // corner under every extreme wobble must stay inside the source
        // frame when the cover scale is applied.
        for (w, h, amp, rot, zmin) in [
            (1920u32, 1080u32, 33.0f32, 1.0f32, 1.0f32),
            (1920, 1080, 440.0, 45.0, 0.8),
            (640, 480, 0.0, 0.0, 1.0),
            (100, 100, 10.0, 30.0, 0.9),
            (1280, 720, 100.0, 5.0, 1.0),
        ] {
            let cover = shake_cover_scale(w, h, amp, rot, zmin);
            assert!(cover >= 1.0, "cover never shrinks the frame");
            let (cx, cy) = (f64::from(w) * 0.5, f64::from(h) * 0.5);
            // The tolerance absorbs the cover's f64 → f32 rounding: a
            // thousandth of a pixel, far below anything visible.
            let tol = 1e-3;
            for (ox, oy) in [(amp, amp), (-amp, amp), (amp, -amp), (-amp, -amp)] {
                // Sweep the rotation range densely: the worst angle sits
                // strictly inside (−rot, rot) for wide frames.
                for k in 0..=8 {
                    let theta = f64::from(rot) * (f64::from(k) / 4.0 - 1.0);
                    for zoom in [zmin, 1.0] {
                        let s = f64::from(cover) * f64::from(zoom);
                        let rad = theta.to_radians();
                        for (px, py) in [
                            (0.0, 0.0),
                            (f64::from(w), 0.0),
                            (0.0, f64::from(h)),
                            (f64::from(w), f64::from(h)),
                        ] {
                            // Inverse map: q = centre + R(−θ)·(p − centre − off)/s.
                            let ux = px - cx - f64::from(ox);
                            let uy = py - cy - f64::from(oy);
                            let qx = cx + (rad.cos() * ux + rad.sin() * uy) / s;
                            let qy = cy + (-rad.sin() * ux + rad.cos() * uy) / s;
                            assert!(
                                qx >= -tol
                                    && qx <= f64::from(w) + tol
                                    && qy >= -tol
                                    && qy <= f64::from(h) + tol,
                                "{w}x{h} amp {amp} rot {rot} zmin {zmin} theta {theta}: \
                                 corner ({px},{py}) maps outside to ({qx},{qy})"
                            );
                        }
                    }
                }
            }
        }
        // Zero maxima: the cover is exactly 1 — auto-scale on a neutral
        // shake stays the bit-exact identity.
        assert_eq!(shake_cover_scale(1920, 1080, 0.0, 0.0, 1.0), 1.0);
    }

    #[test]
    fn shake_instantiates_with_a_per_instance_seed_and_resolves() {
        let e = instantiate("shake").unwrap();
        assert_eq!(e.float_at("amplitude", 0.0), Some(1.5));
        assert_eq!(e.float_at("frequency", 0.0), Some(8.0));
        assert_eq!(e.float_at("rotation", 0.0), Some(1.0));
        assert_eq!(e.float_at("zoom_pump", 0.0), Some(0.0));
        assert!(matches!(
            e.param("auto_scale"),
            Some(EffectValue::Bool(true))
        ));
        assert!(matches!(e.param("seed"), Some(EffectValue::Seed(_))));

        // Resolving is deterministic: the same instance at the same time
        // yields the identical wobble, twice.
        let a = resolve_stack(std::slice::from_ref(&e), 0.4, 1000.0, 1.0);
        let b = resolve_stack(std::slice::from_ref(&e), 0.4, 1000.0, 1.0);
        assert_eq!(a, b);
        let Resolved::Shake {
            offset_px,
            amp_px,
            zoom,
            zoom_min,
            auto_scale,
            mix,
            ..
        } = a[0]
        else {
            panic!("expected a Shake");
        };
        // 1.5% of a 1000px diagonal = 15px ceiling; the wobble stays
        // within it, and pump 0 leaves zoom at exactly 1.
        assert_eq!(amp_px, 15.0);
        assert!(offset_px[0].abs() <= 15.0 && offset_px[1].abs() <= 15.0);
        assert_eq!(zoom, 1.0);
        assert_eq!(zoom_min, 1.0);
        assert!(auto_scale);
        assert_eq!(mix, 1.0);

        // Different frames wobble differently; different seeds too.
        let later = resolve_stack(std::slice::from_ref(&e), 0.9, 1000.0, 1.0);
        assert_ne!(a, later, "the wobble moves between frames");
        let mut reseeded = e.clone();
        for p in &mut reseeded.params {
            if p.id == "seed" {
                let old = match p.value {
                    EffectValue::Seed(s) => s,
                    _ => 0,
                };
                p.value = EffectValue::Seed(old.wrapping_add(1));
            }
        }
        let other = resolve_stack(std::slice::from_ref(&reseeded), 0.4, 1000.0, 1.0);
        assert_ne!(a, other, "a different seed wobbles differently");
    }

    #[test]
    fn cpu_shake_is_identity_at_zero_and_wobbles_through_the_affine() {
        let (w, h) = (17u32, 9u32);
        let img = transform_card(w, h);

        // A neutral shake (zero amplitude, rotation, pump) is the
        // bit-exact identity even with auto-scale on: the cover is
        // exactly 1 and the affine is the identity.
        let neutral = Resolved::Shake {
            offset_px: [0.0, 0.0],
            rotation_deg: 0.0,
            zoom: 1.0,
            amp_px: 0.0,
            rotation_max_deg: 0.0,
            zoom_min: 1.0,
            auto_scale: true,
            mix: 1.0,
        };
        let mut n = img.clone();
        cpu::apply(&mut n, w, h, &neutral);
        assert_eq!(n, img);

        // A pure offset without auto-scale matches the Transform reference
        // fed the same shared affine — the oracle path is one path.
        let shaken = Resolved::Shake {
            offset_px: [2.0, -1.0],
            rotation_deg: 0.0,
            zoom: 1.0,
            amp_px: 2.0,
            rotation_max_deg: 0.0,
            zoom_min: 1.0,
            auto_scale: false,
            mix: 1.0,
        };
        let mut s = img.clone();
        cpu::apply(&mut s, w, h, &shaken);
        let (anchor, position, scale, rot) =
            shake_affine(w, h, [2.0, -1.0], 0.0, 1.0, 2.0, 0.0, 1.0, false);
        let mut t = img.clone();
        cpu::transform(&mut t, w, h, anchor, position, scale, rot, 1.0, 1.0);
        assert_eq!(s, t);
        assert_ne!(s, img, "the wobble actually moves pixels");

        // Auto-scale zooms in: with a rotation ceiling the cover exceeds 1,
        // so the revealed corners stay covered (no transparent corners).
        let covered = Resolved::Shake {
            offset_px: [1.0, 0.0],
            rotation_deg: 5.0,
            zoom: 1.0,
            amp_px: 1.0,
            rotation_max_deg: 5.0,
            zoom_min: 1.0,
            auto_scale: true,
            mix: 1.0,
        };
        let mut c = img.clone();
        cpu::apply(&mut c, w, h, &covered);
        let corner_alpha = |v: &[f32]| {
            let at = |x: u32, y: u32| ((y * w + x) * 4 + 3) as usize;
            [
                v[at(0, 0)],
                v[at(w - 1, 0)],
                v[at(0, h - 1)],
                v[at(w - 1, h - 1)],
            ]
        };
        assert!(
            corner_alpha(&c).iter().all(|a| *a > 0.0),
            "auto-scale keeps every corner covered: {:?}",
            corner_alpha(&c)
        );
    }

    #[test]
    fn transform_inverse_is_exact_at_identity_and_none_at_zero_scale() {
        let (m, o) = transform_inverse([0.0; 2], [0.0; 2], [1.0; 2], 0.0).unwrap();
        assert_eq!(m, [1.0, 0.0, -0.0, 1.0]);
        assert_eq!(o, [0.0, 0.0]);
        assert!(transform_inverse([0.0; 2], [0.0; 2], [0.0, 1.0], 0.0).is_none());
        assert!(transform_inverse([0.0; 2], [0.0; 2], [1.0, 0.0], 0.0).is_none());
    }

    /// A varied premultiplied test card for the transform: gradient, an HDR
    /// spike, a half-alpha region and an opaque border pixel.
    fn transform_card(w: u32, h: u32) -> Vec<f32> {
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let g = (x + y) as f32 / (w + h) as f32;
                let a = if y < h / 2 { 1.0 } else { 0.5 };
                img[i] = g * a;
                img[i + 1] = (1.0 - g) * a;
                img[i + 2] = 0.25 * a;
                img[i + 3] = a;
            }
        }
        let spike = ((3 * w + 4) * 4) as usize;
        img[spike..spike + 4].copy_from_slice(&[6.0, 3.0, 1.5, 1.0]);
        img
    }

    #[test]
    fn cpu_transform_identity_is_bit_exact() {
        let (w, h) = (13u32, 9u32);
        let img = transform_card(w, h);
        // Identity parameters: the docs/08 §3.5 bit-exact passthrough pin.
        let mut id = img.clone();
        cpu::transform(&mut id, w, h, [0.0; 2], [0.0; 2], [1.0; 2], 0.0, 1.0, 1.0);
        assert_eq!(id, img);
        // Mix 0 is the exact identity whatever the parameters.
        let mut m0 = img.clone();
        cpu::transform(
            &mut m0,
            w,
            h,
            [3.0; 2],
            [9.0, 1.0],
            [2.0, 0.5],
            33.0,
            0.4,
            0.0,
        );
        assert_eq!(m0, img);
    }

    #[test]
    fn cpu_transform_moves_scales_rotates_and_fades() {
        // A white impulse on a transparent frame.
        let (w, h) = (17u32, 9u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let mid = at(8, 4);
        img[mid..mid + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);

        // Position +2 in x (anchor 0): the impulse lands two pixels right,
        // exactly (integer offsets keep bilinear taps on pixel centres).
        let mut t = img.clone();
        cpu::transform(&mut t, w, h, [0.0; 2], [2.0, 0.0], [1.0; 2], 0.0, 1.0, 1.0);
        assert_eq!(t[at(10, 4)], 1.0, "impulse moved +2x");
        assert_eq!(t[mid], 0.0, "and left its old home");

        // The area revealed beyond the source edge is transparent, not a
        // smeared border: shifting +2 leaves columns 0-1 fully empty.
        for y in 0..h {
            for x in 0..2 {
                assert_eq!(t[at(x, y) + 3], 0.0, "({x},{y}) revealed as clear");
            }
        }

        // Rotation 90° about the frame centre: y-down raster, so the pixel
        // two to the right of centre lands two below it (clockwise).
        let centre = [8.5, 4.5];
        let mut r = img.clone();
        img[at(10, 4)..at(10, 4) + 4].copy_from_slice(&[0.0, 1.0, 0.0, 1.0]);
        r.copy_from_slice(&img);
        cpu::transform(&mut r, w, h, centre, centre, [1.0; 2], 90.0, 1.0, 1.0);
        assert_eq!(r[mid], 1.0, "the centre pixel stays put");
        assert!(r[at(8, 6) + 1] > 0.999, "+2x lands at +2y");

        // Scale 0 is degenerate: the image collapses to nothing and renders
        // fully transparent — never a division fault (docs/14).
        let mut z = img.clone();
        cpu::transform(&mut z, w, h, centre, centre, [0.0, 0.0], 0.0, 1.0, 1.0);
        assert!(z.iter().all(|v| *v == 0.0), "zero scale collapses to clear");

        // Opacity halves all four channels (premultiplied).
        let mut o = img.clone();
        cpu::transform(&mut o, w, h, [0.0; 2], [0.0; 2], [1.0; 2], 0.0, 0.5, 1.0);
        for c in 0..4 {
            assert_eq!(o[mid + c], 0.5, "channel {c} at half");
        }
    }
}
