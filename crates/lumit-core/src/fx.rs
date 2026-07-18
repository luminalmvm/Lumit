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
use crate::model::{
    Composition, EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, FileParam,
    Layer,
};

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
    /// A file path chosen from a dialog (K-111), e.g. a `.cube` LUT. The
    /// `filter` extensions (lower-case, no dot) and `filter_name` drive the
    /// open dialog. The value carries a [`FileParam`]; it animates only by
    /// stepping (hold keys), since two paths cannot be blended.
    File {
        filter: &'static [&'static str],
        filter_name: &'static str,
    },
    /// A reference to another layer in the composition (docs/impl/
    /// layer-input.md), sampled as an auxiliary picture — the depth pass a
    /// depth-of-field effect reads. The value carries an
    /// [`EffectValue::Layer`] (an optional layer id); the caller renders that
    /// layer alone at comp size and threads its texture beside the resolved
    /// op, exactly as a matte layer is rendered alone. Unset (or a dangling
    /// reference) is a labelled no-op, never a fault.
    Layer {},
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
    // One blur, three modes (docs/08 §3.8): Gaussian (separable two-pass),
    // Directional (line-integral streak along an angle) and Radial (arcs or
    // rays about a centre). Mode selects which extra parameters matter —
    // Radius drives Gaussian, Length/Angle drive Directional, Centre/
    // Amount/Type drive Radial. Instances saved before a mode existed
    // resolve as Gaussian, and each mode's maths are untouched by the
    // others (same kernel per mode, same version).
    //
    // Status (Radial, shipped): the spec text (§3.8) names Centre, Amount
    // and Type without giving ranges — pinned here. Centre is Centre X /
    // Centre Y, two Float params in % of comp width/height (50/50 default):
    // the schema has no Point-shaped ParamKind (checked — Transform's own
    // Anchor/Position use the identical anchor_x/anchor_y split, so this
    // follows established precedent rather than adding a new kind). Amount
    // is % diag (default 8, slider 0–20, hard 0–100), matching the Radius/
    // Length unit family so all three modes read in the same currency —
    // it is the peak per-pixel tap spread, reached at the frame's farthest
    // corner (half the comp diagonal from Centre). Type is Spin / Zoom.
    // Both modes reduce to a pure linear scale of the pixel's own
    // (position − centre) vector — Zoom along that vector (an exact ray
    // sample), Spin along its perpendicular (the first-order/tangent
    // approximation to the true arc about Centre) — so neither needs a
    // division or a runtime trig call: no host trig table was needed
    // either, since the only scale factor (amount ÷ half diagonal) is a
    // plain division done once, not per pixel. The approximation is exact
    // for Zoom and holds closely for Spin across the shipped Amount range
    // (worst-case sweep well under a radian); it also means every tap
    // vanishes to zero exactly at Centre with no epsilon guard. The shared
    // Edge parameter (Transparent/Repeat/Mirror) applies unchanged — taps
    // run through the same bilinear_edge every mode already uses, so
    // Radial clamps/mirrors/clears at the frame border exactly like
    // Gaussian and Directional.
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
                    options: &["Gaussian", "Directional", "Radial"],
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
                id: "centre_x",
                label: "Centre X",
                // Radial mode: % of comp width. resolve_stack only carries
                // diag_px (no separate width/height), so this resolves to a
                // *fraction* of the raster and the CPU/GPU function scales
                // it by its own w — exactly how RGB split's radial mode
                // already derives the frame centre from w/h it already has.
                kind: ParamKind::Float {
                    default: 50.0,
                    slider: (0.0, 100.0),
                    hard: (None, None), // off-frame centres are legal
                },
            },
            ParamSchema {
                id: "centre_y",
                label: "Centre Y",
                // Radial mode: % of comp height (see centre_x).
                kind: ParamKind::Float {
                    default: 50.0,
                    slider: (0.0, 100.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "amount",
                label: "Amount",
                // Radial mode: peak tap spread, % diag (§2.3), reached at
                // the farthest corner from Centre — the same currency as
                // Radius/Length above.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "radial_type",
                label: "Type",
                kind: ParamKind::Choice {
                    options: &["Spin", "Zoom"],
                    default: 0,
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
    // Chromatic aberration (docs/08 §3.15): a dedicated, always-radial
    // sibling of RGB split's own Radial mode (§3.6) — R pulled outward, B
    // pulled inward, G and alpha unshifted, growing from the frame centre.
    // Where RGB split's Amount is % diag (so linear and radial modes share
    // one currency across both angle-driven and centre-driven offsets),
    // this effect has only the radial shape and one purpose, so Amount is
    // authored in raw px@comp (§2.3) instead — scaled by the preview factor
    // exactly like Glitch's Block size — because "how many pixels of
    // fringe" is the honest unit for a single-purpose corner effect with no
    // angle to share a currency with.
    EffectSchema {
        match_name: "chromatic_aberration",
        label: "Chromatic aberration",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            // Amount is raw px@comp, not % diag, so a tight %-diag padding
            // cannot be declared statically across every comp resolution;
            // full-frame is the safe static bound (mirroring Glitch's own
            // px@comp parameters, which take the same route).
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // px@comp (§2.3): peak channel offset, reached at the
                // corner distance from the frame centre.
                kind: ParamKind::Float {
                    default: 4.0,
                    slider: (0.0, 20.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Beat-aware strobe (docs/08 §3.7). Manual mode is the original manual
    // form: each keyframe on Trigger is a hit (its value = how hard, 0..1)
    // that decays exponentially over Decay; a static Trigger holds a
    // constant flash. Trigger mode fires the §1.4 envelope from the comp's
    // beat markers; Strobe fires every Nth beat only. Instances saved
    // before the marker modes existed carry no "mode" parameter and
    // resolve as Manual, byte-identically. Default is a no-op by design:
    // §1.2 exempts inherently trigger-driven effects.
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
            beat_input: true, // binds to comp beat markers per §1.4
        },
        params: &[
            ParamSchema {
                id: "mode",
                label: "Mode",
                // Manual = keyframed hits on Trigger (the original form);
                // Trigger = the §1.4 beat envelope; Strobe = every Nth
                // beat only.
                kind: ParamKind::Choice {
                    options: &["Manual", "Trigger", "Strobe"],
                    default: 0,
                },
            },
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
                id: "duration",
                label: "Duration",
                // Frames (comp-rate, §2.3) a marker-driven flash lasts.
                // Hard floor 0, unbounded above (the K-090 one-sided
                // clamp); 0 is honestly a flash zero frames long — never
                // shown.
                kind: ParamKind::Float {
                    default: 2.0,
                    slider: (0.0, 12.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                id: "shape",
                label: "Shape",
                // Hard holds full strength for Duration then cuts; Fade
                // decays linearly to zero across it.
                kind: ParamKind::Choice {
                    options: &["Hard", "Fade"],
                    default: 0,
                },
            },
            ParamSchema {
                id: "every_nth",
                label: "Every Nth beat",
                // Strobe mode: fire beats 0, N, 2N, … of the comp's beat
                // list. The spec's integer ≥ 1, carried as a Float row —
                // the resolver rounds and clamps at 1.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (1.0, 8.0),
                    hard: (Some(1.0), None),
                },
            },
            ParamSchema {
                id: "phase",
                label: "Phase offset",
                // Frames a marker-driven flash trails (> 0) or leads (< 0)
                // its beat.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-8.0, 8.0),
                    hard: (None, None),
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
    // Vignette (docs/08 §3.14, listed as a planned colour effect in §3.10):
    // darkens toward black away from the frame centre, in premultiplied
    // colour (a coverage-like darken, not a lift/gamma/gain grade, so no
    // unpremultiply round trip). Category Colour, alongside Colour balance
    // and Saturation — its closest siblings and where §3.10's own text
    // already lists it.
    EffectSchema {
        match_name: "vignette",
        label: "Vignette",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "amount",
                label: "Amount",
                // 0..1: the darkening strength; 0 is the neutral point
                // (bit-exact passthrough, pinned by test).
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "radius",
                label: "Radius",
                // 0..1: how far from centre the clear area reaches, in the
                // Roundness-blended distance metric below (1.0 = that
                // metric's own reference edge).
                kind: ParamKind::Float {
                    default: 0.75,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "softness",
                label: "Softness",
                // 0..1: feather width beyond Radius, in the same metric.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "roundness",
                label: "Roundness",
                // 1 = circular (both axes read equal pixel distances as
                // equal); 0 = follows the frame's own aspect ratio (an
                // ellipse exactly reaching every edge at Radius 1).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Exposure (docs/08 §3.16): a single scene-linear gain on RGB (2^stops) —
    // the montage grade's brightness lever. Premultiplied: a scalar scales
    // premultiplied colour consistently, so no unpremultiply round trip and
    // alpha is untouched. 0 stops is the neutral point (bit-exact passthrough,
    // pinned by test). Category Colour, beside its grade siblings.
    EffectSchema {
        match_name: "exposure",
        label: "Exposure",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "stops",
                label: "Stops",
                // Photographic stops; each +1 doubles the light. 0 is neutral.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-5.0, 5.0),
                    hard: (None, None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Hue shift (docs/08 §3.17): rotate every colour's hue by an angle, a
    // constant-luminance rotation (Rec.709 luma stays put). A linear 3×3
    // colour matrix, computed host-side so the CPU reference and the kernel
    // multiply by identical coefficients; premultiplied (a linear matrix
    // scales through alpha), alpha untouched. 0° is the bit-exact neutral
    // point. Category Colour, beside its grade siblings.
    EffectSchema {
        match_name: "hue_shift",
        label: "Hue shift",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "angle",
                label: "Angle",
                // Degrees; wraps every 360. 0 is neutral.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-180.0, 180.0),
                    hard: (None, None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Contrast (docs/08 §3.18): expand or compress RGB about a fixed mid-grey
    // pivot (0.5) — the montage grade's punch lever. An affine grade
    // (out = (in − pivot) × k + pivot), and because of the − pivot offset it
    // does NOT commute with premultiplied alpha, so premultiplied: false: the
    // host unpremultiplies, grades, and re-premultiplies, exactly like
    // Saturation and Colour balance. 100 % (k = 1) is the neutral point
    // (bit-exact passthrough, pinned by test). Continuous everywhere (no
    // round/clamp/quantize), so the §1.6 oracle holds. Category Colour, beside
    // its grade siblings.
    EffectSchema {
        match_name: "contrast",
        label: "Contrast",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: an affine grade shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "contrast",
                label: "Contrast",
                // Per cent about mid-grey: 0 = flat grey, 100 = neutral,
                // 200 = doubled. Hard min 0 (no inversion); unbounded above.
                kind: ParamKind::Float {
                    default: 100.0,
                    slider: (0.0, 200.0),
                    hard: (Some(0.0), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Gamma (docs/08 §3.19): a per-channel power curve in the effect's
    // scene-linear working space — out = pow(max(in, 0), 1/gamma) per RGB
    // channel, alpha untouched. The input is clamped to ≥ 0 before the pow
    // (scene-linear can dip slightly negative, and the clamp must be
    // byte-identical on CPU and GPU so the §1.6 oracle holds). pow is
    // non-linear, so — like Contrast and Saturation — it does NOT commute with
    // premultiplied alpha: premultiplied: false, and the host unpremultiplies,
    // curves, and re-premultiplies. Gamma 1.0 is the neutral point (a bit-exact
    // passthrough short-circuit, not a reliance on pow(x, 1) == x). Continuous
    // everywhere for input ≥ 0, so the §1.6 oracle holds. Category Colour,
    // beside its grade siblings.
    EffectSchema {
        match_name: "gamma",
        label: "Gamma",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: a non-linear curve shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "gamma",
                label: "Gamma",
                // The power curve raises to 1/gamma. 1 is neutral; hard floor
                // 0.01 keeps 1/gamma finite, no hard ceiling above.
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.1, 4.0),
                    hard: (Some(0.01), None),
                },
            },
            MIX_PARAM,
        ],
    },
    // Temperature (docs/08 §3.20): a warm/cool white-balance shift as a
    // per-channel gain in scene-linear light — the montage grade's warmth
    // lever. `k = Temperature ÷ 100`; `gain_r = 1 + 0.5·k` boosts red as it
    // warms, `gain_b = 1 − 0.5·k` cuts blue, green untouched. Premultiplied: a
    // per-channel scalar scales premultiplied colour consistently (straight ×
    // gain, then × the unchanged alpha), so no unpremultiply round trip and
    // alpha is untouched — exactly like Exposure's pure multiply, and unlike
    // the affine Contrast/Saturation grades (their − pivot offset breaks that
    // commutation, §2.2). The two gains are computed host-side (in the resolve
    // step) so the CPU reference and the kernel multiply by byte-identical
    // factors. Temperature 0 is the neutral point (gains 1.0, bit-exact
    // passthrough, pinned by test). Category Colour, beside its grade siblings.
    EffectSchema {
        match_name: "temperature",
        label: "Temperature",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "temperature",
                label: "Temperature",
                // A plain number: negative cools (blue up, red down), positive
                // warms (red up, blue down). 0 is neutral. Hard ±100.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-100.0, 100.0),
                    hard: (Some(-100.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // LUT (docs/08 §3.11, docs/impl/lut.md, K-114): a 3D colour look-up from a
    // `.cube` file — a colourist's baked grade dropped onto a layer. A File
    // parameter picks the cube (animatable only by stepping between paths, since
    // two files cannot be blended) and Mix blends the graded result over the
    // input. The heavy lifting lives elsewhere: `lumit_core::lut` parses the
    // cube, `lumit_gpu::fx` samples it as a 3D texture. The resolve step carries
    // only Mix — a path is not `Copy`, so (like Echo's neighbour frames and
    // Motion blur's flow field) the loaded cube travels beside the resolved op,
    // supplied by the caller's LUT cache. Unpremultiplied (§2.2: a LUT is an
    // arbitrary colour map, so it must not see premultiplied values); an unset,
    // missing, 1D or unreadable file is a labelled no-op, never a fault (§3.11
    // never-crash rule). Moderate cost (a per-pixel 3D lookup), Exact ROI.
    EffectSchema {
        match_name: "lut",
        label: "LUT",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: an arbitrary colour map must see straight colour
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "file",
                label: "File",
                // A `.cube` LUT chosen from a dialog (K-111); the value steps
                // between paths with hold keys only.
                kind: ParamKind::File {
                    filter: &["cube"],
                    filter_name: "Cube LUT",
                },
            },
            MIX_PARAM,
        ],
    },
    // Depth of field (docs/08 §3.22, docs/impl/layer-input.md): a lens blur
    // driven by a depth pass. A `Layer` parameter names another layer as the
    // depth input (its red channel read as 0..1 depth, docs/impl/layer-input.md
    // §3), Focus/Range set the sharp band and Aperture the maximum blur disc.
    // The heavy lifting is the existing `lumit_gpu::fx::dof` kernel; resolution
    // carries only the scalars (Focus/Range/Aperture/Mix) — the depth layer is
    // not `Copy`, so (like the LUT's cube and Motion blur's flow field) the
    // referenced layer's rendered texture travels beside the resolved op,
    // rendered alone at comp size exactly as a matte layer is. An unset (or
    // dangling) depth reference is a labelled no-op, never a fault (the same
    // sanctioned exception the File parameter takes to the "no no-op default"
    // rule). Premultiplied (the disc gathers the working premultiplied colour,
    // per `fx_dof.wgsl`), Moderate cost, `{0}` temporal. ROI is a padded
    // gather: the static declaration covers the Aperture slider's 40 px@comp
    // maximum across typical rasters (docs/08 §2.3 % diag ≈ 40 px at ≥ 1080p).
    EffectSchema {
        match_name: "dof",
        label: "Depth of field",
        version: 1,
        category: FxCategory::BlurSharpen,
        traits: EffectTraits {
            cost: CostClass::Moderate,
            // Aperture is px@comp (up to 40); 3 % of the comp diagonal covers
            // that on a 1080p+ raster and over-covers smaller ones — a safe
            // static bound for a runtime-sized gather (docs/impl/layer-input.md).
            roi: Roi::PaddedPctDiag(3.0),
            temporal: &[0],
            premultiplied: true, // the disc gathers premultiplied colour (fx_dof.wgsl)
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "depth",
                label: "Depth layer",
                // The layer whose red channel is the depth pass (0 = near,
                // 1 = far by convention; the effect is symmetric about Focus).
                // Unset until the owner picks one (a labelled no-op).
                kind: ParamKind::Layer {},
            },
            ParamSchema {
                // The "after effects" companion for the depth Layer input
                // (K-125): off (default) reads the depth layer's source pixels,
                // on runs the depth layer's own effect stack into the depth pass
                // first (a graded or blurred depth map). Named `<layer>_after_
                // effects` by convention so the host can find the flag for any
                // layer-input parameter. Off keeps the historical source-only
                // behaviour, so old projects are unchanged.
                id: "depth_after_effects",
                label: "Depth after effects",
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                // Invert the depth pass (d' = 1 - d) before the circle-of-
                // confusion, swapping near and far — the owner's "tick to
                // invert the depth" box (Frischluft / DOF PRO both offer it).
                // Off (default) keeps the historical reading, so old projects
                // are unchanged. Continuous, so the §1.6 ULP oracle still holds.
                id: "depth_invert",
                label: "Depth invert",
                kind: ParamKind::Bool { default: false },
            },
            ParamSchema {
                id: "focus",
                label: "Focus distance",
                // The in-focus depth, 0..1. Mid-depth by default so a typical
                // near-to-far pass has its middle sharp.
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "range",
                label: "Focus range",
                // Half-width of the sharp band around Focus, 0..1: depths
                // within it stay crisp.
                kind: ParamKind::Float {
                    default: 0.1,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "aperture",
                label: "Aperture",
                // The master maximum circle-of-confusion radius in px@comp
                // (§2.3), reached at the farthest-from-focus depth. Scales both
                // per-side radii about its default 8 (unity: `aperture / 8`), so
                // a project saved before Near/Far existed — which has only this
                // param — renders identically (Near/Far fall back to 8, and
                // 8·aperture/8 = aperture on both sides). Clamped at zero below
                // (a zero master is a passthrough), unbounded typing above the
                // 40 px slider.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 40.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // Per-side circle-of-confusion for the near side — depths in
                // front of focus (`d < focus`). px@comp, scaled by the Aperture
                // master. Owner's "adjust close/far blur separately". Absent on
                // pre-feature projects, where it falls back to Aperture.
                id: "near_aperture",
                label: "Near blur",
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 40.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // Per-side circle-of-confusion for the far side — depths behind
                // focus (`d >= focus`). px@comp, scaled by the Aperture master.
                // Absent on pre-feature projects, where it falls back to
                // Aperture, keeping the old symmetric behaviour.
                id: "far_aperture",
                label: "Far blur",
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (0.0, 40.0),
                    hard: (Some(0.0), None),
                },
            },
            ParamSchema {
                // Diagnostic views (the realistic subset the reference plugins
                // ship). Rendered is the normal blurred output; Depth map shows
                // the post-invert depth as greyscale; Focus map is the smooth
                // in-focus mask (white where sharp, darkening out of focus).
                // Every mode is continuous, so the §1.6 ULP oracle holds across
                // them. Absent on pre-feature projects → Rendered (default 0).
                id: "display",
                label: "Display",
                kind: ParamKind::Choice {
                    options: &["Rendered", "Depth map", "Focus map"],
                    default: 0,
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
    // Block glitch (docs/08 §3.12, split out of the old combined Glitch
    // effect by K-107 — one of three now-standalone one-thing effects,
    // alongside Scanlines and Datamosh below). Seeded — category Distortion
    // to match Shake and RGB split, its closest siblings (positional
    // wobble, channel split), not the additive-light Stylise pair (Glow,
    // Flash). Stacking Block glitch → Scanlines, each at Mix 100%,
    // reproduces the old combined Glitch's look bit-for-bit at Intensity 1
    // (each section ran unconditionally there too).
    //
    // Status (shipped): the spec text names most of these without ranges;
    // pinned here, carried over unchanged from the combined effect.
    // Intensity (0–1, the master dial) scales *everything* glitched — grid
    // jitter, displacement, channel offset and slice-repeat odds alike — so
    // it is a genuine single "how glitched" knob and 0 is the bit-exact
    // passthrough. "Rows/columns jitter" is one Block jitter % (of Block
    // size), not separate row/column controls, applied as a per-nominal-
    // block hashed offset to where that block's content is read from — a
    // cheap stand-in for actually moving grid lines (which would need a
    // boundary search a single pointwise pass cannot do), pinned as a
    // deliberate simplification. "Channel-offset toggle or amount" ships as
    // a Float (Channel offset, % diag) — continuous like every other
    // amount-shaped parameter in the catalogue, following RGB split's
    // R/B-offset-from-G shape but with a per-block hashed offset instead of
    // one global vector. Slice repetition ships as a Float 0–100%: the
    // odds (scaled by Intensity) that a given block folds its own content
    // to repeat a short hashed strip instead of a plain positional read.
    // Per-block hashing runs inside the GPU kernel (the block index is a
    // per-pixel quantity, so the hash cannot be a host-precomputed table):
    // WGSL has no 64-bit integer type, so it cannot host Shake's actual
    // splitmix64 lattice. `splitmix32` is a matching-spirit 32-bit sibling
    // added alongside it for exactly this (both CPU and GPU use it, so they
    // agree on the integer hash bit-for-bit; only the fp16 sampling that
    // follows carries the usual small tolerance) — Shake's own
    // splitmix64/value_noise_1d are untouched. "Time-derived tick" (the
    // spec's phrase for per-frame block variation) steps at a fixed,
    // unexposed 8 Hz — chosen so blocks visibly pop rather than blur into
    // continuous noise; no rate parameter is listed in the spec text, so
    // this is pinned as an internal constant.
    EffectSchema {
        match_name: "block_glitch",
        label: "Block glitch",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::FullFrame,
            temporal: &[0],
            premultiplied: true,
            seeded: true,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // The master dial (§1.2): scales every hashed quantity.
                // 0 is the bit-exact passthrough (pinned by test).
                kind: ParamKind::Float {
                    default: 0.35,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "seed",
                label: "Seed",
                kind: ParamKind::Seed,
            },
            ParamSchema {
                id: "block_size",
                label: "Block size",
                // px@comp (§2.3): a deliberately pixel-scale look.
                kind: ParamKind::Float {
                    default: 24.0,
                    slider: (4.0, 128.0),
                    hard: (Some(2.0), None), // ≥ 2px: never a degenerate grid
                },
            },
            ParamSchema {
                id: "block_jitter",
                label: "Rows/columns jitter",
                // % of Block size (status note above): a hashed offset to
                // where each nominal block's content is read from.
                kind: ParamKind::Float {
                    default: 25.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "block_amount",
                label: "Displacement",
                // % diag (§2.3), the same currency as Blur's Radius/Length.
                kind: ParamKind::Float {
                    default: 3.0,
                    slider: (0.0, 15.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "channel_offset",
                label: "Channel offset",
                // % diag: a per-block hashed RGB split (status note above).
                kind: ParamKind::Float {
                    default: 1.0,
                    slider: (0.0, 10.0),
                    hard: (Some(0.0), Some(50.0)),
                },
            },
            ParamSchema {
                id: "slice_repeat",
                label: "Slice repeat",
                // % odds (× Intensity) a block folds to a repeating strip.
                kind: ParamKind::Float {
                    default: 20.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Scanlines (docs/08 §3.12, split out of the old combined Glitch effect
    // by K-107). No hash, no seed — a pointwise periodic darken read
    // straight from the input pixel, never a neighbour, so its ROI is
    // `exact` (tighter than Block glitch's full-frame). Category Distortion,
    // alongside Block glitch and Datamosh. Roll speed's sign is open (either
    // direction); Interlace alternates which half of each scanline period
    // darkens on odd periods, the classic interlaced-field look.
    EffectSchema {
        match_name: "scanlines",
        label: "Scanlines",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // The master dial (§1.2): scales the darken strength. 0 is
                // the bit-exact passthrough (pinned by test).
                kind: ParamKind::Float {
                    default: 0.35,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "scanline_period",
                label: "Line period",
                // px@comp: the deliberately pixel-scale scanline pitch.
                kind: ParamKind::Float {
                    default: 3.0,
                    slider: (1.0, 20.0),
                    hard: (Some(1.0), None),
                },
            },
            ParamSchema {
                id: "scanline_darkness",
                label: "Darkness",
                kind: ParamKind::Float {
                    default: 40.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "scanline_roll",
                label: "Roll speed",
                // Lines (periods) per second; either direction (K-090).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-30.0, 30.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scanline_interlace",
                label: "Interlace offset",
                kind: ParamKind::Bool { default: false },
            },
            MIX_PARAM,
        ],
    },
    // Datamosh (docs/08 §3.12, K-104, split out on its own by K-107):
    // re-warps the -1 source neighbour along the flow measured from this
    // frame to it — "reused an old frame's motion" — blended by Intensity.
    // Reuses Motion blur's flow machinery and its own already-shipped GPU
    // pass/CPU oracle unchanged (`FxEngine::datamosh`, `cpu::datamosh`):
    // only the schema, the `Resolved` variant and the stack wiring are new.
    // Previously a toggle inside the combined Glitch effect with a dynamic
    // per-instance temporal reach (the one place `stack_temporal_window`/
    // `stack_flow_neighbour` read a param value instead of the schema's own
    // static `temporal`); as its own effect that toggle is gone and the
    // reach is simply the schema's `{0, -1}`, exactly the static shape
    // Motion blur's own `{0, 1}` already has. Footage-only: with no -1
    // neighbour or flow field (a non-footage layer, or a dropped decode) it
    // degrades to a no-op, never a fault. Category Distortion, matching
    // Shake and RGB split (its closest siblings: a seeded positional
    // wobble, a channel split) — but Datamosh itself reads no hash or seed,
    // so `seeded: false`, unlike them.
    EffectSchema {
        match_name: "datamosh",
        label: "Datamosh",
        version: 1,
        category: FxCategory::Distortion,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            // One bilinear tap, but the flow can point anywhere in the
            // frame — the same unbounded-read reasoning Motion blur's own
            // full-frame ROI already carries for its flow-directed taps.
            roi: Roi::FullFrame,
            temporal: &[0, -1],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "intensity",
                label: "Intensity",
                // 0..1: blends between the ordinary frame and the moshed
                // one. 0 is the bit-exact passthrough (pinned by test).
                kind: ParamKind::Float {
                    default: 0.5,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Echo / trails (docs/08 §3.13): the montage speed-line staple — the
    // first temporal effect (its window reaches back to previous frames, so
    // the render decodes the layer's source at those offsets). v1 status,
    // pinned here: echoes are spaced one comp frame apart (a Spacing control
    // is a later refinement), so the window reaches back Echoes frames, up to
    // 8 (the static trait cap). Each echo k is at offset -k with intensity
    // Decay^k, a geometric trail. Mode chooses how the echoes combine with
    // the leading frame — Add sums light (bright trails), Behind lays them
    // under it (ghosting), Max lightens per channel. Cheap and full-frame
    // (it reads whole neighbour frames). Operates on the layer's *source*
    // frames, not the upstream stack's output at those times (full temporal
    // stacking is later) — so it echoes the footage, placed by the layer's
    // own transform like any effect output.
    EffectSchema {
        match_name: "echo",
        label: "Echo",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::FullFrame,
            temporal: &[0, -1, -2, -3, -4, -5, -6, -7, -8],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "echoes",
                label: "Echoes",
                // Count of trailing frames; each is one comp frame further
                // back (v1 fixed spacing). Capped at the 8-frame window.
                kind: ParamKind::Float {
                    default: 4.0,
                    slider: (1.0, 8.0),
                    hard: (Some(1.0), Some(8.0)),
                },
            },
            ParamSchema {
                id: "decay",
                label: "Decay",
                // Per-echo intensity falloff: echo k has intensity decay^k.
                kind: ParamKind::Float {
                    default: 0.6,
                    slider: (0.0, 1.0),
                    hard: (Some(0.0), Some(1.0)),
                },
            },
            ParamSchema {
                id: "mode",
                label: "Mode",
                kind: ParamKind::Choice {
                    options: &["Add", "Behind", "Max"],
                    default: 1,
                },
            },
            MIX_PARAM,
        ],
    },
    // Motion blur (flow) / RSMB-class (docs/08 §3.2): synthesised motion blur
    // from real optical flow. Game capture has no natural blur; this estimates
    // the per-pixel motion between the current source frame and the next
    // (§3.1's flow engine, run in the decode worker where both frames live),
    // then smears each pixel along its own motion vector so fast-moving areas
    // streak along their actual motion. The second temporal effect: its window
    // is {0, 1} (current + one frame ahead), and the render fetches the +1
    // neighbour through the same machinery Echo added — but where Echo reads
    // the neighbour *pixels*, motion blur reads a *flow field* the decode
    // worker computes from them and hands the kernel as a texture.
    //
    // Status (v1, pinned here): the §3.2 parameter set is trimmed to
    // Shutter angle, Samples and the host Mix. Blur length in pixels =
    // motion vector × (shutter ÷ 360), integrated as a centred box streak of
    // Samples evenly spaced bilinear taps (the same line-integral shape as
    // Directional blur, but per-pixel-directed by the flow). Vector source is
    // Flow only (Auto's transform-derivative path and the engine-motion-blur
    // interaction guard follow); Amount (post-shutter vector scale) and the
    // Quality/adaptive-tap-count control are deferred — Samples is a fixed
    // per-frame tap count so the CPU and GPU integrate identically. Zero
    // motion or a zero shutter is a bit-exact passthrough (pinned by test).
    // Edges clamp (the flow sampler's own rule), so a full-frame smear never
    // darkens the border. Cost heavy, full-frame ROI; footage layers only,
    // exactly like Echo (adjustment-layer temporal effects follow).
    EffectSchema {
        match_name: "motion_blur",
        label: "Motion blur",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            // Current frame + one ahead: the flow engine brackets the motion
            // between them. The +1 neighbour is fetched by the same decode
            // planner Echo's negative offsets use.
            temporal: &[0, 1],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "shutter_angle",
                label: "Shutter angle",
                // Degrees (§3.2: 0–720, default 180): the fraction of the
                // frame interval the shutter is open, so the streak length is
                // shutter ÷ 360 of the inter-frame motion. 180° = half the
                // motion, the film-standard look.
                kind: ParamKind::Float {
                    default: 180.0,
                    slider: (0.0, 720.0),
                    hard: (Some(0.0), Some(720.0)),
                },
            },
            ParamSchema {
                id: "samples",
                label: "Samples",
                // Taps along the streak (§3.2). The spec's integer, carried
                // as a Float row (the schema has no integer kind — Echo's
                // Echoes does the same); the resolver rounds and clamps. More
                // taps smooth a long streak; fewer are cheaper.
                kind: ParamKind::Float {
                    default: 16.0,
                    slider: (8.0, 32.0),
                    hard: (Some(2.0), Some(64.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Matte key (docs/08 §3.21): a soft chroma key — a greenscreen remover.
    // Alpha is driven down where a pixel's chroma sits close to the key
    // colour's chroma, on straight (unpremultiplied) colour (§2.2, the wrap
    // fused into the kernel like Saturation's). The distance is Euclidean in
    // the chroma plane (RGB minus Rec. 709 luma), so greens of varying
    // brightness key alike; a smoothstep between Tolerance and
    // Tolerance + Softness makes the key continuous everywhere (no hard step,
    // so the §1.6 ULP oracle holds — cost class `cheap`). Spill suppression
    // pulls the residual key-hue projection out of the kept pixels' colour
    // (desaturating toward luma along the key hue) so green fringes fade.
    // Category Utility, beside Transform. The default green + Tolerance visibly
    // keys a typical green screen ("drop it on and it works", §1.2); there is
    // no neutral no-op default (Mix 0 is the identity). A viewer eyedropper to
    // pick the key colour off the image is a nice follow-up, out of scope here.
    EffectSchema {
        match_name: "matte_key",
        label: "Matte key",
        version: 1,
        category: FxCategory::Utility,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: keying/despill works on straight colour
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "key",
                label: "Key colour",
                // Scene-linear RGBA; alpha ignored. Default a saturated green,
                // the greenscreen the effect exists to remove.
                kind: ParamKind::Colour {
                    default: [0.0, 0.6, 0.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "tolerance",
                label: "Tolerance",
                // Per cent, the chroma-distance threshold below which a pixel
                // is fully keyed (alpha ·= 0). Resolves to a plain 0..1
                // fraction read against the Euclidean chroma metric.
                kind: ParamKind::Float {
                    default: 20.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "softness",
                label: "Softness",
                // Per cent, the soft-edge width above Tolerance across which
                // the keep-factor smoothsteps from 0 to 1.
                kind: ParamKind::Float {
                    default: 10.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            ParamSchema {
                id: "spill",
                label: "Spill suppression",
                // Per cent of the residual key-hue chroma pulled out of the
                // kept colour (0 = off, the default: keying without despill).
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (0.0, 100.0),
                    hard: (Some(0.0), Some(100.0)),
                },
            },
            MIX_PARAM,
        ],
    },
    // Invert (docs/08 §3.23, K-126): a simple colour inverse — out.rgb = 1 − in.rgb
    // per channel, alpha kept. Because 1 − c is affine (not a pure scale) it does
    // NOT commute with premultiplied alpha, so premultiplied: false: the host wraps
    // unpremultiply → invert → re-premultiply (fused into the kernel and the CPU
    // reference), exactly like Contrast and Gamma, so matte edges do not fringe.
    // The inverse is taken in the compositor's scene-linear fp16 working space (the
    // owner's "simple inverse"), so HDR values above 1 invert to honest negatives,
    // never clipped (§2.1). Continuous everywhere, so the §1.6 oracle holds. There
    // is no neutral no-op default — invert always inverts (§1.2) — so only Mix 0 is
    // the identity. Category Colour, beside its grade siblings.
    EffectSchema {
        match_name: "invert",
        label: "Invert",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: 1 − c is affine, so it shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[MIX_PARAM],
    },
    // Tint (docs/08 §3.24, K-127): a luminance duotone / gradient map. Two colour
    // params — "Map black to" (default black) and "Map white to" (default white) —
    // and out.rgb = black.rgb + (white.rgb − black.rgb) · luma(in.rgb) with Rec.709
    // luma on the unpremultiplied linear colour, alpha kept. A luma-driven colour
    // remap does not commute with premultiplied alpha, so premultiplied: false: the
    // host wraps unpremultiply → map → re-premultiply (fused into the kernel and the
    // CPU reference), exactly like Contrast and Gamma, so matte edges do not fringe.
    // The default black→black / white→white maps every pixel to its own luma — a
    // greyscale, a visible tasteful default (§1.2), not a no-op — so only Mix 0 is
    // the identity. Continuous everywhere, so the §1.6 oracle holds. Category Colour,
    // beside its grade siblings.
    EffectSchema {
        match_name: "tint",
        label: "Tint",
        version: 1,
        category: FxCategory::Colour,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            roi: Roi::Exact,
            temporal: &[0],
            premultiplied: false, // §2.2: a colour remap shifts matte edges
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "black",
                label: "Map black to",
                // Scene-linear RGBA (alpha ignored): the colour dark input maps to.
                kind: ParamKind::Colour {
                    default: [0.0, 0.0, 0.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            ParamSchema {
                id: "white",
                label: "Map white to",
                // Scene-linear RGBA (alpha ignored): the colour bright input maps to.
                kind: ParamKind::Colour {
                    default: [1.0, 1.0, 1.0, 1.0],
                    range: (0.0, 4.0),
                },
            },
            MIX_PARAM,
        ],
    },
];

/// Look a schema up by its match name.
pub fn schema(match_name: &str) -> Option<&'static EffectSchema> {
    BUILTINS.iter().find(|s| s.match_name == match_name)
}

/// The union of source-relative frame offsets a layer's live effect stack
/// needs (docs/08 §1.3 `temporal`), always sorted and always containing 0
/// (the current frame). `&[0]` when the stack is bypassed, empty, or every
/// effect is a plain single-frame one — so a layer with no temporal effect
/// pays nothing. The render pipeline decodes the layer's source at each of
/// these offsets so a temporal effect (echo, flow motion blur, datamosh)
/// can read its neighbours.
pub fn stack_temporal_window(effects: &[EffectInstance], fx_on: bool) -> Vec<i32> {
    let mut offsets = vec![0i32];
    if fx_on {
        for e in effects.iter().filter(|e| e.enabled) {
            if e.effect.namespace != EffectNamespace::Builtin {
                continue;
            }
            if let Some(s) = schema(&e.effect.match_name) {
                offsets.extend_from_slice(s.traits.temporal);
            }
        }
    }
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

/// True when any live effect in the stack reads frames other than the
/// current one — the cheap gate the render/cache paths check before doing
/// any neighbour-frame work.
pub fn stack_is_temporal(effects: &[EffectInstance], fx_on: bool) -> bool {
    fx_on
        && effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .any(|e| {
                schema(&e.effect.match_name)
                    .is_some_and(|s| s.traits.temporal.iter().any(|&o| o != 0))
            })
}

/// The neighbour offset a live effect wants a dense **flow field** measured
/// against (per-pixel motion vectors between the current source frame and
/// that neighbour), computed in the decode worker and handed to the kernel
/// as a texture — the gate mirroring [`stack_is_temporal`] that the render/
/// decode paths check before doing any flow work. Flow motion blur (docs/08
/// §3.2) wants `1` (the +1 neighbour); Datamosh (§3.12, K-107) wants `-1` —
/// both purely static reads of the schema's own match name now (K-107
/// dropped the dynamic per-instance check a combined Glitch effect used to
/// need). Both effects are also temporal (their windows reach that same
/// offset), so the neighbour machinery already fetches the source frame the
/// flow is measured against.
///
/// A layer can carry only one flow field per frame in v1
/// ([`crate`]-external callers store it in a single `Option` slot) — if a
/// stack somehow has both a live Motion blur and a live Datamosh, the first
/// one encountered in stack order wins and the other's flow-dependent
/// behaviour degrades to its own missing-field passthrough (never a fault;
/// pinned by test, K-104).
pub fn stack_flow_neighbour(effects: &[EffectInstance], fx_on: bool) -> Option<i32> {
    if !fx_on {
        return None;
    }
    for e in effects
        .iter()
        .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
    {
        if e.effect.match_name == "motion_blur" {
            return Some(1);
        }
        if e.effect.match_name == "datamosh" {
            return Some(-1);
        }
    }
    None
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

/// The constant-luminance hue-rotation matrix for `deg` degrees, row-major
/// (docs/08 §3.17). Rec.709 luma weights keep perceived brightness fixed as
/// the hue turns. Computed host-side (f64 then cast) so the CPU reference and
/// the WGSL kernel multiply by the identical `f32` coefficients. Exactly the
/// identity at 0°, so the effect's neutral point is bit-exact.
pub fn hue_matrix(deg: f64) -> [f32; 9] {
    if deg % 360.0 == 0.0 {
        return [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    let (s, c) = deg.to_radians().sin_cos();
    // Rec.709 luma; the standard SVG feColorMatrix hue-rotate coefficients.
    let (lr, lg, lb) = (0.2126_f64, 0.7152, 0.0722);
    [
        (lr + c * (1.0 - lr) - s * lr) as f32,
        (lg - c * lg - s * lg) as f32,
        (lb - c * lb + s * (1.0 - lb)) as f32,
        (lr - c * lr + s * 0.143) as f32,
        (lg + c * (1.0 - lg) + s * 0.140) as f32,
        (lb - c * lb - s * 0.283) as f32,
        (lr - c * lr - s * (1.0 - lr)) as f32,
        (lg - c * lg + s * lg) as f32,
        (lb + c * (1.0 - lb) + s * lb) as f32,
    ]
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
                    ParamKind::File { .. } => EffectValue::File(FileParam::empty()),
                    // A fresh layer reference is unset (docs/impl/
                    // layer-input.md): the effect is a labelled no-op until the
                    // owner picks a layer, the same sanctioned exception the
                    // File parameter takes to the "no no-op default" rule.
                    ParamKind::Layer {} => EffectValue::Layer(None),
                },
                extra: serde_json::Map::new(),
            })
            .collect(),
        sample_temporally: true,
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
    /// Blur's Radial mode (docs/08 §3.8): rays from, or a tangent to the
    /// arc about, a centre — see the schema's status note for why both
    /// reduce to a pure linear scale of (position − centre) with no
    /// division or runtime trig.
    RadialBlur {
        /// Centre as a *fraction* of the raster (not raster pixels):
        /// resolve_stack carries only diag_px, not separate width/height,
        /// so the CPU/GPU function scales this by its own w/h — exactly
        /// how RGB split's radial mode already derives the frame centre.
        centre_frac: [f32; 2],
        /// Peak tap spread in raster pixels, reached at the frame's
        /// farthest corner from Centre (half the raster diagonal away).
        amount_px: f32,
        /// True = Spin (tangent direction), false = Zoom (radial direction).
        spin: bool,
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
    /// Chromatic aberration (docs/08 §3.15): a dedicated, always-radial
    /// sibling of RGB split's own Radial mode — always centred on the
    /// frame, no angle or linear mode of its own.
    ChromaticAberration {
        /// Peak channel offset in raster pixels, reached at the corner
        /// distance from the frame centre.
        amount_px: f32,
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
    /// Matte key (docs/08 §3.21): a soft chroma key. `key` is the scene-linear
    /// RGBA key colour (resolved at frame time like Vignette's tint, alpha
    /// ignored); the CPU/GPU maths derive its chroma and hue direction from it
    /// identically. `tol`/`soft`/`spill` are plain 0..1 fractions. The keep
    /// factor smoothsteps from 0 (fully keyed, alpha ·= 0) at chroma distance
    /// `tol` to 1 (fully kept) at `tol + soft`, so it is continuous
    /// everywhere. There is no neutral no-op default; Mix 0 is the identity.
    MatteKey {
        /// Scene-linear RGBA key colour (alpha ignored).
        key: [f32; 4],
        /// Chroma-distance threshold, 0..1: at/below it the pixel is fully keyed.
        tol: f32,
        /// Soft-edge width above `tol`, 0..1: the smoothstep transition span.
        soft: f32,
        /// Key-hue spill removal, 0..1: fraction of residual key chroma pulled out.
        spill: f32,
        /// 0..1.
        mix: f32,
    },
    /// Vignette (docs/08 §3.14): darkens toward black away from the frame
    /// centre. `radius`/`softness` are read against the Roundness-blended
    /// distance metric [`cpu::vignette`] computes from `w`/`h` — no raster
    /// conversion happens here, unlike the %-diag family, because the
    /// metric is already resolution-relative by construction.
    Vignette {
        /// 0..1: darkening strength; 0 is the neutral point.
        amount: f32,
        /// 0..1: the clear centre's reach.
        radius: f32,
        /// 0..1: feather width beyond radius.
        softness: f32,
        /// 0..1: 1 = circular, 0 = follows the frame's aspect.
        roundness: f32,
        /// 0..1.
        mix: f32,
    },
    /// Exposure (docs/08 §3.16): RGB × `factor` (= 2^stops), alpha untouched.
    /// `factor` 1.0 is the neutral point.
    Exposure {
        /// Linear gain, 2^stops.
        factor: f32,
        /// 0..1.
        mix: f32,
    },
    /// Hue shift (docs/08 §3.17): a row-major linear 3×3 colour matrix (the
    /// constant-luminance hue rotation), computed host-side. Identity is the
    /// neutral point.
    HueShift {
        /// Row-major 3×3: `[m00,m01,m02, m10,m11,m12, m20,m21,m22]`.
        m: [f32; 9],
        /// 0..1.
        mix: f32,
    },
    /// Contrast (docs/08 §3.18): the affine grade `(in − 0.5) × k + 0.5` per
    /// RGB channel on unpremultiplied colour, alpha untouched. `k` 1.0
    /// (Contrast 100 %) is the neutral point.
    Contrast {
        /// Contrast factor, `contrast_percent / 100`; 1.0 is neutral.
        k: f32,
        /// 0..1.
        mix: f32,
    },
    /// Gamma (docs/08 §3.19): the per-channel power curve
    /// `out = pow(max(in, 0), 1/gamma)` on unpremultiplied colour, alpha
    /// untouched. `gamma` 1.0 is the neutral point.
    Gamma {
        /// Gamma value; the curve raises to `1/gamma`. 1.0 is neutral,
        /// clamped ≥ 0.01 so the reciprocal stays finite.
        gamma: f32,
        /// 0..1.
        mix: f32,
    },
    /// Temperature (docs/08 §3.20): a warm/cool white balance as a per-channel
    /// R/B gain in scene-linear light, computed host-side, alpha untouched.
    /// Gains `(1.0, 1.0)` (Temperature 0) are the neutral point.
    Temperature {
        /// Scene-linear red gain, `1 + 0.5·(temperature/100)`.
        gain_r: f32,
        /// Scene-linear blue gain, `1 − 0.5·(temperature/100)`.
        gain_b: f32,
        /// 0..1.
        mix: f32,
    },
    /// Invert (docs/08 §3.23): the colour inverse `out.rgb = 1 − in.rgb` per RGB
    /// channel on unpremultiplied colour, alpha untouched. No neutral value —
    /// invert always inverts — so only Mix 0 is the identity.
    Invert {
        /// 0..1.
        mix: f32,
    },
    /// Tint (docs/08 §3.24): a luminance duotone. `out.rgb = black + (white −
    /// black)·luma(in)` with Rec.709 luma on the unpremultiplied colour, alpha
    /// untouched. The two mapped colours resolve to scene-linear RGB at frame
    /// time; Mix 0 is the identity.
    Tint {
        /// Scene-linear RGB the darkest input maps to.
        black: [f32; 3],
        /// Scene-linear RGB the brightest input maps to.
        white: [f32; 3],
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
    /// Block glitch (docs/08 §3.12, split out by K-107). `tick` is the
    /// local time already discretised at [`GLITCH_TICK_HZ`] (host-side, so
    /// the kernel never sees raw time or does its own time maths).
    /// Intensity 0 is the bit-exact passthrough (pinned by test) — see the
    /// schema's status note for why every hashed quantity here is scaled by
    /// it.
    BlockGlitch {
        /// The master 0..1 dial; scales every hashed quantity.
        intensity: f32,
        seed: u32,
        /// Local time discretised at [`GLITCH_TICK_HZ`] (§3.12 status
        /// note): per-block hashing reads this, not raw time.
        tick: i32,
        /// Raster pixels (px@comp × the §2.3 preview factor).
        block_size_px: f32,
        /// 0..1, fraction of block_size_px (the "Rows/columns jitter").
        jitter_frac: f32,
        /// Peak per-block displacement, raster pixels (% diag).
        amount_px: f32,
        /// Peak per-block R/B split, raster pixels (% diag).
        chan_px: f32,
        /// 0..1: odds (before the Intensity scale) a block slice-repeats.
        slice_frac: f32,
        /// 0..1.
        mix: f32,
    },
    /// Scanlines (docs/08 §3.12, split out by K-107). `roll_px` is the
    /// scanline pattern's already-computed pixel offset (roll speed × local
    /// time × period), host-computed so the kernel never sees raw time.
    /// Intensity 0 is the bit-exact passthrough (pinned by test).
    Scanlines {
        /// The master 0..1 dial; scales the darken strength.
        intensity: f32,
        /// Raster pixels (px@comp × the §2.3 preview factor).
        period_px: f32,
        /// 0..1.
        darkness: f32,
        /// The scanline pattern's pixel offset at this frame (roll speed ×
        /// local time × period_px, host-computed).
        roll_px: f32,
        interlace: bool,
        /// 0..1.
        mix: f32,
    },
    /// Datamosh (docs/08 §3.12, K-104, its own effect since K-107): re-warp
    /// the -1 source neighbour along the flow measured from this frame to
    /// it. The neighbour frame and its flow field are not carried here —
    /// like Echo's neighbour frames and Motion blur's flow field, they
    /// travel beside the resolved op, supplied only when the layer is
    /// footage and the decode fetched them; a missing pair degrades this to
    /// a no-op, never a fault.
    Datamosh {
        /// 0..1: blended against the current frame.
        intensity: f32,
        /// 0..1, the host Mix. Composes with `intensity` by multiplication
        /// before reaching the kernel (mixing the same two inputs twice
        /// collapses to one mix by the product), so the existing GPU/CPU
        /// maths need not carry a second blend knob.
        mix: f32,
    },
    /// Echo / trails (docs/08 §3.13). `weights[i]` is the intensity of the
    /// echo at frame offset `-(i+1)` (0 = no echo there); the render supplies
    /// the neighbour frame at each live offset. `mode`: 0 = Add, 1 = Behind,
    /// 2 = Max.
    Echo {
        weights: [f32; 8],
        mode: u32,
        /// 0..1.
        mix: f32,
    },
    /// Flow motion blur (docs/08 §3.2). The per-pixel motion vectors are not
    /// carried here — they are a whole flow field, computed in the decode
    /// worker and passed to the kernel as a separate texture (the same way
    /// Echo's neighbour *frames* travel beside the resolved op, not inside
    /// it). This variant carries only the scalars the kernel needs to turn a
    /// vector into a streak.
    MotionBlur {
        /// Shutter ÷ 360: the streak length as a fraction of the inter-frame
        /// motion (0 = no blur; 0.5 at the 180° default).
        shutter_frac: f32,
        /// Evenly spaced bilinear taps along the streak (already rounded and
        /// clamped from the Samples parameter).
        samples: i32,
        /// 0..1.
        mix: f32,
    },
    /// LUT (docs/08 §3.11, docs/impl/lut.md, K-114): a 3D `.cube` colour
    /// lookup. Only the host Mix is `Copy`-carried here; the parsed-and-
    /// uploaded cube is a whole 3D texture, so — like Echo's neighbour frames
    /// and Motion blur's flow field — it travels beside the resolved op (the
    /// caller's LUT cache fills a parallel `luts` slot), not inside it. An
    /// unset/1D/unreadable file leaves that slot empty and the op is a
    /// passthrough. `mix == 0` is the bit-exact input.
    Lut {
        /// 0..1.
        mix: f32,
    },
    /// Depth of field (docs/08 §3.22, docs/impl/layer-input.md): a lens blur
    /// whose per-pixel circle-of-confusion comes from a depth pass. Only the
    /// scalars are `Copy`-carried here; the depth is a whole texture — the
    /// referenced layer rendered alone at comp size — so (like the LUT's cube
    /// and Motion blur's flow field) it travels beside the resolved op (the
    /// caller fills a parallel `layer_inputs` slot), not inside it. An unset,
    /// missing or cyclic depth reference leaves that slot empty and the op is
    /// a passthrough. `aperture == 0`, an all-in-band depth, or `mix == 0` are
    /// bit-exact passthroughs.
    Dof {
        /// The in-focus depth, 0..1.
        focus: f32,
        /// Half-width of the sharp band around `focus`, 0..1.
        range: f32,
        /// Maximum circle-of-confusion radius for the **near** side (depths in
        /// front of focus, `d < focus`), raster pixels — the per-side Near blur
        /// already scaled by the Aperture master and the §2.3 preview factor.
        near_aperture: f32,
        /// Maximum circle-of-confusion radius for the **far** side (depths
        /// behind focus, `d >= focus`), raster pixels — the Far blur already
        /// scaled by the master and the preview factor.
        far_aperture: f32,
        /// When set, the per-pixel depth is inverted (`d' = 1 - d`) before the
        /// circle-of-confusion, swapping near and far. A `Copy` scalar, so the
        /// enum stays `Copy` and threads beside the depth texture unchanged.
        depth_invert: bool,
        /// Diagnostic view: 0 = Rendered (the blurred output), 1 = Depth map
        /// (post-invert greyscale), 2 = Focus map (the smooth in-focus mask).
        /// Modes 1/2 ignore the blur and Mix and write the view directly.
        display: u32,
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

/// The trigger times a marker-driven Flash reads (docs/08 §3.7): every
/// `nth`-th beat of the ordered §1.4 context — indices 0, n, 2n, … of the
/// beat list, the comp's first beat being index 0 — shifted by
/// `phase_frames` comp frames. Yields layer-local seconds, ascending. One
/// iterator shared by the envelope and the frame-key window
/// ([`marker_window`]) so cache invalidation can never drift from what
/// resolution computes.
fn flash_trigger_times<'a>(
    markers: &'a MarkerContext,
    nth: u32,
    phase_frames: f64,
) -> impl Iterator<Item = f64> + 'a {
    let dt = if markers.fps > 0.0 {
        phase_frames / markers.fps
    } else {
        0.0
    };
    markers
        .beats
        .iter()
        .step_by(nth.max(1) as usize)
        .map(move |b| b + dt)
}

/// The Flash beat envelope (docs/08 §3.7 Trigger and Strobe modes), pinned
/// once for resolution, its unit tests and the frame key alike. From the
/// nearest trigger at/before the frame ([`flash_trigger_times`]), with
/// `elapsed = (lt − trigger) · fps` in comp frames: Hard holds 1 while
/// `0 ≤ elapsed < duration_frames`, Fade ramps `1 − elapsed/duration_frames`
/// over the same span; past it — and before the first trigger — the
/// envelope is 0. No markers, a non-positive frame rate (the [`MarkerContext::NONE`]
/// caller) or a non-positive duration all yield 0: the §1.4 graceful
/// fallback. Pure function of its inputs, so determinism (§2.4) holds.
pub fn flash_beat_envelope(
    markers: &MarkerContext,
    lt: f64,
    duration_frames: f64,
    fade: bool,
    nth: u32,
    phase_frames: f64,
) -> f64 {
    if markers.fps <= 0.0 || duration_frames <= 0.0 {
        return 0.0;
    }
    let mut env = 0.0;
    for tt in flash_trigger_times(markers, nth, phase_frames) {
        let elapsed = (lt - tt) * markers.fps;
        if elapsed < 0.0 {
            break; // ascending: every later trigger is in the future too
        }
        env = if elapsed < duration_frames {
            if fade {
                1.0 - elapsed / duration_frames
            } else {
                1.0
            }
        } else {
            0.0 // the nearest trigger at/before wins, even once spent
        };
    }
    env
}

/// Strobe's Every Nth beat parameter, read as the spec's integer ≥ 1
/// (docs/08 §3.7): rounded to the nearest whole beat count, clamped at 1,
/// non-finite values degrading to 1.
fn flash_nth(e: &EffectInstance, lt: f64) -> u32 {
    let n = e.float_at("every_nth", lt).unwrap_or(1.0);
    if n.is_finite() && n >= 1.0 {
        n.round() as u32
    } else {
        1
    }
}

/// What one marker-driven effect instance sees of the §1.4 context at a
/// frame — the nearest trigger either side of it, exactly as its envelope
/// consumes them (Nth-filtered and phase-shifted for a Strobe flash), plus
/// the comp frame rate its frame-authored parameters convert through. Fed
/// into the frame key (lumit-eval) so a cached frame is retired exactly
/// when a marker edit can change what this instance computes, and left
/// alone otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MarkerWindow {
    /// Comp frames per second.
    pub fps: f64,
    /// The nearest trigger at/before the frame, layer-local seconds.
    pub before: Option<f64>,
    /// The nearest trigger strictly after, layer-local seconds.
    pub after: Option<f64>,
}

/// The §1.4 window `e` consumes at layer time `lt` — None when the
/// instance is not marker-driven right now (an effect without marker
/// input, or a Flash in Manual mode), which is what keeps such instances'
/// frame keys time-free. v1: Flash is the only marker consumer; a new
/// marker-driven effect adds its arm here, so the frame key learns it in
/// the same place resolution does.
pub fn marker_window(e: &EffectInstance, lt: f64, markers: &MarkerContext) -> Option<MarkerWindow> {
    if e.effect.namespace != EffectNamespace::Builtin || e.effect.match_name != "flash" {
        return None;
    }
    let mode = match e.param("mode") {
        Some(EffectValue::Choice(c)) => *c,
        _ => 0,
    };
    if mode != 1 && mode != 2 {
        return None; // Manual: no marker input, no time in the key
    }
    let nth = if mode == 2 { flash_nth(e, lt) } else { 1 };
    let phase = e.float_at("phase", lt).unwrap_or(0.0);
    let mut w = MarkerWindow {
        fps: markers.fps,
        before: None,
        after: None,
    };
    for tt in flash_trigger_times(markers, nth, phase) {
        if tt <= lt {
            w.before = Some(tt);
        } else {
            w.after = Some(tt);
            break;
        }
    }
    Some(w)
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

/// A 32-bit avalanche mixer, in the same five-line-portability spirit as
/// [`splitmix64`] above (public-domain "splitmix32" shape: golden-ratio
/// increment, xorshift/multiply/xorshift finalisation) — Glitch's per-block
/// hash (docs/08 §3.12 status note) needs this narrower sibling because the
/// block index is a *per-pixel* quantity the WGSL kernel must hash itself
/// (there are too many blocks to precompute a host-side table into the
/// uniform), and WGSL has no 64-bit integer type to host the real
/// splitmix64 lattice. Both the CPU reference and the kernel run this exact
/// sequence of wrapping u32 ops, so they agree on the integer hash
/// bit-for-bit; Shake's splitmix64/[`value_noise_1d`] are untouched.
fn splitmix32(mut x: u32) -> u32 {
    x = x.wrapping_add(0x9e37_79b9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x21f0_aaad);
    x ^= x >> 15;
    x = x.wrapping_mul(0x735a_2d97);
    x ^= x >> 15;
    x
}

/// One Glitch per-block hash channel (docs/08 §3.12 status note): folds
/// `(seed, channel, block x, block y, tick)` through [`splitmix32`] and
/// maps the top 24 bits to `[0, 1)` — exactly representable in f32/f64, so
/// CPU and GPU read the identical value. Discrete and unfiltered on
/// purpose (unlike [`value_noise_1d`]'s smooth interpolation): adjacent
/// blocks are meant to be independent draws, and "tick" is the tick-rate
/// discretisation of local time that gives block glitching its per-frame
/// pop rather than a continuous wobble.
pub fn block_hash01(seed: u32, channel: u32, bx: i32, by: i32, tick: i32) -> f32 {
    let mut h = seed;
    h = splitmix32(h ^ channel);
    h = splitmix32(h ^ (bx as u32));
    h = splitmix32(h ^ (by as u32));
    h = splitmix32(h ^ (tick as u32));
    (h >> 8) as f32 / 16_777_216.0 // top 24 bits, /2^24 → exact in f32
}

/// Glitch's fixed, unexposed block-glitch update rate (docs/08 §3.12
/// status note): the spec's "time-derived tick" without a listed rate
/// parameter, pinned as an internal constant — fast enough that block
/// glitching reads as chaotic, slow enough that individual pops stay
/// visible instead of blurring into continuous noise.
pub const GLITCH_TICK_HZ: f64 = 8.0;

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

/// The §1.4 marker resolve context: what marker-driven effects see at
/// resolution time. It carries the comp's beat-marker times **translated
/// into the layer's local time** — comp marker time minus the layer's start
/// offset, the same one f64 subtraction that produces the `lt` handed to
/// [`resolve_stack`], so a beat and a frame at the same comp moment compare
/// exactly equal and the envelope maths lives in a single time base — plus
/// the comp frame rate, because duration-class parameters are authored in
/// comp frames (§2.3). Built by [`MarkerContext::for_layer`], the one
/// constructor preview and export both call (K-031), so the two can never
/// drift. A caller with no comp to hand passes [`MarkerContext::NONE`];
/// marker-driven effects MUST fall back gracefully on it (§1.4).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MarkerContext {
    /// Beat-marker times in the layer's local time base, seconds, sorted
    /// ascending.
    pub beats: Vec<f64>,
    /// Comp frames per second; 0 in the no-comp default (guarded wherever
    /// frames convert to seconds).
    pub fps: f64,
}

impl MarkerContext {
    /// The obvious empty context — no beats, no frame rate — for callers
    /// without markers. Every marker-driven effect resolves to its
    /// graceful fallback on it (§1.4).
    pub const NONE: MarkerContext = MarkerContext {
        beats: Vec::new(),
        fps: 0.0,
    };

    /// The context for one layer of `comp`: the comp's beat markers only
    /// (the v1 §1.4 scope — named-layer binding and label filters follow),
    /// each translated into the layer's local time, sorted ascending.
    pub fn for_layer(comp: &Composition, layer: &Layer) -> Self {
        let off = layer.start_offset.0.to_f64();
        let mut beats: Vec<f64> = comp
            .markers
            .iter()
            .filter(|m| m.is_beat())
            .map(|m| m.time.0.to_f64() - off)
            .collect();
        beats.sort_by(f64::total_cmp);
        Self {
            beats,
            fps: comp.frame_rate.fps(),
        }
    }

    /// The ordered beat times within `[from_s, to_s]` local seconds — the
    /// §1.4 "inside the effect's temporal window" view.
    pub fn window(&self, from_s: f64, to_s: f64) -> &[f64] {
        let a = self.beats.partition_point(|b| *b < from_s);
        let z = self.beats.partition_point(|b| *b <= to_s).max(a);
        &self.beats[a..z]
    }

    /// The nearest beat at/before `lt` and the nearest strictly after —
    /// the §1.4 "either side of the current frame" pair.
    pub fn nearest(&self, lt: f64) -> (Option<f64>, Option<f64>) {
        let i = self.beats.partition_point(|b| *b <= lt);
        (
            i.checked_sub(1).map(|j| self.beats[j]),
            self.beats.get(i).copied(),
        )
    }
}

/// Resolve a layer's live stack at layer time `lt` for a raster whose
/// diagonal is `diag_px` pixels; `px_scale` is raster pixels per comp pixel
/// (the §2.3 preview-resolution factor — 1.0 at full resolution), which
/// converts px@comp parameters exactly as `diag_px` converts % diag ones.
/// `markers` is the layer's §1.4 marker context ([`MarkerContext::for_layer`],
/// or [`MarkerContext::NONE`] where no comp is in play), consumed by the
/// marker-driven modes (Flash's Trigger and Strobe, §3.7). Placeholders,
/// unknown names and bypassed effects resolve to nothing (they render as
/// identity, docs/03 §8).
pub fn resolve_stack(
    effects: &[EffectInstance],
    lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &MarkerContext,
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
                // Instances saved before a mode existed carry no "mode"
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
                } else if mode == 2 {
                    let cx = (e.float_at("centre_x", lt).unwrap_or(50.0) / 100.0) as f32;
                    let cy = (e.float_at("centre_y", lt).unwrap_or(50.0) / 100.0) as f32;
                    let amount_pct = e.float_at("amount", lt).unwrap_or(0.0) as f32;
                    let spin = !matches!(e.param("radial_type"), Some(EffectValue::Choice(1)));
                    Some(Resolved::RadialBlur {
                        centre_frac: [cx, cy],
                        amount_px: (amount_pct / 100.0 * diag_px).max(0.0),
                        spin,
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
            "chromatic_aberration" => {
                let amount_px =
                    (e.float_at("amount", lt).unwrap_or(4.0) as f32 * px_scale).max(0.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::ChromaticAberration { amount_px, mix })
            }
            "flash" => {
                // Instances saved before the marker modes existed carry no
                // "mode" parameter and resolve as Manual — byte-identically.
                let mode = match e.param("mode") {
                    Some(EffectValue::Choice(c)) => *c,
                    _ => 0,
                };
                let envelope = match mode {
                    // Trigger (1) and Strobe (2): the §3.7 beat envelope
                    // from the §1.4 context; Strobe thins the beat list to
                    // every Nth.
                    1 | 2 => {
                        let duration = e.float_at("duration", lt).unwrap_or(2.0).max(0.0);
                        let fade = matches!(e.param("shape"), Some(EffectValue::Choice(1)));
                        let nth = if mode == 2 { flash_nth(e, lt) } else { 1 };
                        let phase = e.float_at("phase", lt).unwrap_or(0.0);
                        flash_beat_envelope(markers, lt, duration, fade, nth, phase)
                    }
                    // Manual: keyframed hits on Trigger, decaying over
                    // Decay — the original form, untouched.
                    _ => {
                        let decay_s = (e.float_at("decay", lt).unwrap_or(120.0) / 1000.0).max(0.0);
                        match e.param("trigger") {
                            Some(EffectValue::Float(p)) => flash_envelope(p, lt, decay_s),
                            _ => 0.0,
                        }
                    }
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
            "matte_key" => {
                // The colour is resolved to a scene-linear array at frame time,
                // like Vignette's tint would be; the CPU reference and the WGSL
                // kernel derive its chroma/hue direction from it identically.
                // Tolerance/Softness/Spill are per cent → plain 0..1 fractions.
                let key = e.colour_at("key", lt).unwrap_or([0.0, 0.6, 0.0, 1.0]);
                let tol = (e.float_at("tolerance", lt).unwrap_or(20.0) as f32 / 100.0).max(0.0);
                let soft = (e.float_at("softness", lt).unwrap_or(10.0) as f32 / 100.0).max(0.0);
                let spill = (e.float_at("spill", lt).unwrap_or(0.0) as f32 / 100.0).clamp(0.0, 1.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::MatteKey {
                    key: key.map(|c| c as f32),
                    tol,
                    soft,
                    spill,
                    mix,
                })
            }
            "vignette" => {
                let amount = (e.float_at("amount", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
                let radius = (e.float_at("radius", lt).unwrap_or(0.75) as f32).clamp(0.0, 1.0);
                let softness = (e.float_at("softness", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
                let roundness = (e.float_at("roundness", lt).unwrap_or(1.0) as f32).clamp(0.0, 1.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Vignette {
                    amount,
                    radius,
                    softness,
                    roundness,
                    mix,
                })
            }
            "exposure" => {
                let stops = e.float_at("stops", lt).unwrap_or(0.0);
                let factor = 2f64.powf(stops) as f32;
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Exposure { factor, mix })
            }
            "hue_shift" => {
                let angle = e.float_at("angle", lt).unwrap_or(0.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::HueShift {
                    m: hue_matrix(angle),
                    mix,
                })
            }
            "contrast" => {
                // k = contrast_percent / 100; hard min 0 (no inversion),
                // unbounded above — the schema's own honest shape.
                let k = (e.float_at("contrast", lt).unwrap_or(100.0) as f32 / 100.0).max(0.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Contrast { k, mix })
            }
            "gamma" => {
                // Hard floor 0.01 keeps 1/gamma finite; no ceiling — the
                // schema's own honest shape.
                let gamma = (e.float_at("gamma", lt).unwrap_or(1.0) as f32).max(0.01);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Gamma { gamma, mix })
            }
            "temperature" => {
                // k = Temperature / 100, clamped to the ±100 hard range. The
                // two gains are computed here so the CPU reference and the WGSL
                // kernel multiply by byte-identical f32 factors (§1.6);
                // Temperature 0 → k 0 → gains exactly (1.0, 1.0), the neutral
                // point.
                let k =
                    (e.float_at("temperature", lt).unwrap_or(0.0) as f32 / 100.0).clamp(-1.0, 1.0);
                let gain_r = 1.0 + 0.5 * k;
                let gain_b = 1.0 - 0.5 * k;
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Temperature {
                    gain_r,
                    gain_b,
                    mix,
                })
            }
            "invert" => {
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Invert { mix })
            }
            "tint" => {
                // The two mapped colours resolve to scene-linear RGB at frame
                // time (alpha ignored); the CPU reference and the WGSL kernel
                // read the identical numbers.
                let rgb = |id: &str, default: [f64; 4]| -> [f32; 3] {
                    let c = e.colour_at(id, lt).unwrap_or(default);
                    [c[0] as f32, c[1] as f32, c[2] as f32]
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Tint {
                    black: rgb("black", [0.0, 0.0, 0.0, 1.0]),
                    white: rgb("white", [1.0, 1.0, 1.0, 1.0]),
                    mix,
                })
            }
            "lut" => {
                // Only Mix is Copy-carried; the `.cube` file's parsed cube is a
                // 3D texture threaded beside the resolved op (the caller's LUT
                // cache), exactly as the flow field is for Motion blur. A `lut`
                // effect always resolves to exactly one Resolved::Lut, so the
                // ordered enabled-builtin-`lut` list stays 1:1 and in order with
                // the Resolved::Lut ops — the whole threading contract.
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Lut { mix })
            }
            "dof" => {
                // Scalars only; the depth pass (the referenced layer's rendered
                // texture) is threaded beside the op by the caller, exactly as
                // the LUT cube is. A `dof` effect always resolves to exactly one
                // Resolved::Dof, so the ordered enabled-builtin-`dof` list stays
                // 1:1 and in order with the Dof ops — the threading contract.
                let focus = (e.float_at("focus", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
                let range = (e.float_at("range", lt).unwrap_or(0.1) as f32).clamp(0.0, 1.0);
                // Aperture is the px@comp master; Near/Far are the per-side
                // radii it scales about its default 8 (unity). A pre-feature
                // project has only `aperture` and lacks Near/Far, which then
                // read their default 8, so each side resolves to
                // 8·(aperture/8)·px_scale = aperture·px_scale — identical to the
                // old single-aperture behaviour. px@comp is scaled by the §2.3
                // preview factor so a Half preview blurs the same disc as Full.
                let master = e.float_at("aperture", lt).unwrap_or(8.0) as f32 / 8.0;
                let near = e.float_at("near_aperture", lt).unwrap_or(8.0) as f32;
                let far = e.float_at("far_aperture", lt).unwrap_or(8.0) as f32;
                let near_aperture = (near * master * px_scale).max(0.0);
                let far_aperture = (far * master * px_scale).max(0.0);
                // Depth invert (a plain Bool; absent on pre-feature projects,
                // where it reads false — the historical, unchanged behaviour).
                let depth_invert = matches!(e.param("depth_invert"), Some(EffectValue::Bool(true)));
                // Diagnostic view (clamped to the shipped modes; absent on
                // pre-feature projects → 0 Rendered, the normal output).
                let display = match e.param("display") {
                    Some(EffectValue::Choice(c)) => (*c).min(2),
                    _ => 0,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Dof {
                    focus,
                    range,
                    near_aperture,
                    far_aperture,
                    depth_invert,
                    display,
                    mix,
                })
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
            "block_glitch" => {
                let intensity =
                    (e.float_at("intensity", lt).unwrap_or(0.35) as f32).clamp(0.0, 1.0);
                let seed = match e.param("seed") {
                    Some(EffectValue::Seed(s)) => *s,
                    _ => 0,
                };
                // Local time discretised at the fixed tick rate (§3.12
                // status note): block hashing reads this, never raw time.
                let tick = (lt * GLITCH_TICK_HZ).floor() as i32;
                let block_size_px =
                    (e.float_at("block_size", lt).unwrap_or(24.0) as f32 * px_scale).max(1.0);
                let jitter_frac =
                    (e.float_at("block_jitter", lt).unwrap_or(25.0) as f32 / 100.0).clamp(0.0, 1.0);
                let amount_pct = e.float_at("block_amount", lt).unwrap_or(3.0) as f32;
                let chan_pct = e.float_at("channel_offset", lt).unwrap_or(1.0) as f32;
                let slice_frac =
                    (e.float_at("slice_repeat", lt).unwrap_or(20.0) as f32 / 100.0).clamp(0.0, 1.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::BlockGlitch {
                    intensity,
                    seed,
                    tick,
                    block_size_px,
                    jitter_frac,
                    amount_px: (amount_pct / 100.0 * diag_px).max(0.0),
                    chan_px: (chan_pct / 100.0 * diag_px).max(0.0),
                    slice_frac,
                    mix,
                })
            }
            "scanlines" => {
                let intensity =
                    (e.float_at("intensity", lt).unwrap_or(0.35) as f32).clamp(0.0, 1.0);
                let period_px =
                    (e.float_at("scanline_period", lt).unwrap_or(3.0) as f32 * px_scale).max(1.0);
                let darkness = (e.float_at("scanline_darkness", lt).unwrap_or(40.0) as f32 / 100.0)
                    .clamp(0.0, 1.0);
                let roll_speed = e.float_at("scanline_roll", lt).unwrap_or(0.0);
                // The scanline pattern's pixel offset at this frame (roll
                // speed × local time × period), so the kernel never sees
                // raw time or does its own time maths (§2.4: the CPU/GPU
                // must agree, and f32 time would round differently near a
                // tick boundary than f64 does — precomputing sidesteps it).
                let roll_px = (roll_speed * lt * f64::from(period_px)) as f32;
                let interlace = match e.param("scanline_interlace") {
                    Some(EffectValue::Bool(b)) => *b,
                    _ => false,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Scanlines {
                    intensity,
                    period_px,
                    darkness,
                    roll_px,
                    interlace,
                    mix,
                })
            }
            "datamosh" => {
                let intensity = (e.float_at("intensity", lt).unwrap_or(0.5) as f32).clamp(0.0, 1.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Datamosh { intensity, mix })
            }
            "echo" => {
                // Echoes k = 1..count sit at offset -k with intensity
                // decay^k (v1 fixed one-frame spacing); the render supplies
                // the neighbour frame at each offset. weights[i] is the echo
                // at offset -(i+1).
                let count = (e.float_at("echoes", lt).unwrap_or(4.0).round() as i32).clamp(1, 8);
                let decay = (e.float_at("decay", lt).unwrap_or(0.6) as f32).clamp(0.0, 1.0);
                let mode = match e.param("mode") {
                    Some(EffectValue::Choice(c)) => (*c).min(2),
                    _ => 1,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                let mut weights = [0.0f32; 8];
                for (i, w) in weights.iter_mut().enumerate() {
                    if (i as i32) < count {
                        *w = decay.powi(i as i32 + 1);
                    }
                }
                Some(Resolved::Echo { weights, mode, mix })
            }
            "motion_blur" => {
                // Streak length = motion × (shutter ÷ 360); the flow field
                // (the motion itself) is threaded to the kernel separately.
                // Samples is the spec's integer carried as a Float row —
                // rounded and clamped to the same 2..64 the kernel loops.
                let shutter_frac =
                    (e.float_at("shutter_angle", lt).unwrap_or(180.0) as f32 / 360.0).max(0.0);
                let samples =
                    (e.float_at("samples", lt).unwrap_or(16.0).round() as i32).clamp(2, 64);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::MotionBlur {
                    shutter_frac,
                    samples,
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
            Resolved::RadialBlur {
                centre_frac,
                amount_px,
                spin,
                edge,
                mix,
            } => blur_radial(rgba, w, h, *centre_frac, *amount_px, *spin, *edge, *mix),
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
            Resolved::ChromaticAberration { amount_px, mix } => {
                chromatic_aberration(rgba, w, h, *amount_px, *mix)
            }
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
            Resolved::MatteKey {
                key,
                tol,
                soft,
                spill,
                mix,
            } => matte_key(rgba, *key, *tol, *soft, *spill, *mix),
            Resolved::Vignette {
                amount,
                radius,
                softness,
                roundness,
                mix,
            } => vignette(rgba, w, h, *amount, *radius, *softness, *roundness, *mix),
            Resolved::Exposure { factor, mix } => exposure(rgba, *factor, *mix),
            Resolved::HueShift { m, mix } => hue_shift(rgba, *m, *mix),
            Resolved::Contrast { k, mix } => contrast(rgba, *k, *mix),
            Resolved::Gamma { gamma: g, mix } => gamma(rgba, *g, *mix),
            Resolved::Temperature {
                gain_r,
                gain_b,
                mix,
            } => temperature(rgba, *gain_r, *gain_b, *mix),
            Resolved::Invert { mix } => invert(rgba, *mix),
            Resolved::Tint { black, white, mix } => tint(rgba, *black, *white, *mix),
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
            Resolved::BlockGlitch {
                intensity,
                seed,
                tick,
                block_size_px,
                jitter_frac,
                amount_px,
                chan_px,
                slice_frac,
                mix,
            } => block_glitch(
                rgba,
                w,
                h,
                *intensity,
                *seed,
                *tick,
                *block_size_px,
                *jitter_frac,
                *amount_px,
                *chan_px,
                *slice_frac,
                *mix,
            ),
            Resolved::Scanlines {
                intensity,
                period_px,
                darkness,
                roll_px,
                interlace,
                mix,
            } => scanlines(
                rgba, w, h, *intensity, *period_px, *darkness, *roll_px, *interlace, *mix,
            ),
            // Echo is temporal: it needs the layer's neighbour frames, which
            // this single-buffer in-place dispatcher does not carry. The real
            // path is [`echo`] (with neighbours) on the GPU; here it is a
            // pass-through (the CPU-fallback render can't echo).
            Resolved::Echo { .. } => {}
            // Motion blur needs the layer's flow field, which this
            // single-buffer dispatcher does not carry either. The real path is
            // [`motion_blur`] (with the flow field) on the GPU; here it is a
            // pass-through, exactly like Echo.
            Resolved::MotionBlur { .. } => {}
            // Datamosh needs the layer's -1 neighbour and its flow field,
            // which this single-buffer dispatcher does not carry either. The
            // real path is `FxEngine::datamosh` (with neighbour + flow) on
            // the GPU; here it is a pass-through, exactly like Echo and
            // Motion blur.
            Resolved::Datamosh { .. } => {}
            // A LUT is a GPU colour map: the parsed cube never reaches this
            // Resolved-based CPU dispatcher (the file path is threaded
            // separately), so the CPU-degradation rung renders it as identity.
            // The §1.6 oracle reference is `lut::Lut3d::sample`, exercised
            // directly in the lumit-gpu test, not through cpu::apply.
            Resolved::Lut { .. } => {}
            // Depth of field reads a depth texture (the referenced layer
            // rendered alone) that never reaches this single-buffer dispatcher,
            // so — like Echo, Motion blur and LUT — the CPU-degradation rung
            // renders it as identity. The §1.6 oracle reference is
            // `dof_reference` in the lumit-gpu test (the depth is a texture, not
            // a number), not through cpu::apply.
            Resolved::Dof { .. } => {}
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

    /// Matte key (docs/08 §3.21): a soft chroma key (greenscreen removal), on
    /// straight (unpremultiplied) colour (§2.2) — unpremultiply → key + despill
    /// → re-premultiply, exactly Saturation's premultiply handling. The metric
    /// is Euclidean distance in the chroma plane: each colour's chroma is
    /// `rgb − luma` (Rec. 709 luma, [`LUMA`]), a pure-chroma vector, so greens
    /// of any brightness sit at the same point and key alike. A **smoothstep**
    /// keep-factor is 0 (fully keyed, alpha ·= 0) at chroma distance ≤ `tol`,
    /// 1 (fully kept) at ≥ `tol + soft`, and smooth between — no hard step, so
    /// the effect is continuous everywhere and safe under the §1.6 fp16 ULP
    /// oracle (the WGSL twin matches op-for-op). Spill suppression pulls a
    /// `spill` fraction of the pixel's key-hue projection out of its chroma
    /// (desaturating toward luma along the key hue), fading green fringes on
    /// kept pixels. The key's chroma/hue direction are derived here once and
    /// per-invocation in the kernel from the identical `key`, so both paths use
    /// the same numbers; a grey key (no hue) makes spill a no-op. Mix 0 is the
    /// bit-exact identity (the `× (1 − mix) + · × mix` blend collapses to the
    /// input). `soft`'s transition width floors at a small epsilon so `soft` 0
    /// reads as a steep edge rather than a division by zero.
    pub fn matte_key(rgba: &mut [f32], key: [f32; 4], tol: f32, soft: f32, spill: f32, mix: f32) {
        // Key chroma (a pure-chroma vector: its own luma is zero) and unit hue
        // direction; a grey key has no hue, so its direction is zero and spill
        // does nothing.
        let kl = key[0] * LUMA[0] + key[1] * LUMA[1] + key[2] * LUMA[2];
        let kc = [key[0] - kl, key[1] - kl, key[2] - kl];
        let klen = (kc[0] * kc[0] + kc[1] * kc[1] + kc[2] * kc[2]).sqrt();
        let kdir = if klen > 1e-6 {
            [kc[0] / klen, kc[1] / klen, kc[2] / klen]
        } else {
            [0.0; 3]
        };
        let e1 = tol + soft.max(1e-6);
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            let pl = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
            let pc = [u[0] - pl, u[1] - pl, u[2] - pl];
            // Distance from the key's chroma → smoothstep keep-factor.
            let dc = [pc[0] - kc[0], pc[1] - kc[1], pc[2] - kc[2]];
            let d = (dc[0] * dc[0] + dc[1] * dc[1] + dc[2] * dc[2]).sqrt();
            let t = ((d - tol) / (e1 - tol)).clamp(0.0, 1.0);
            let keep = t * t * (3.0 - 2.0 * t);
            // Spill: remove the key-hue projection from the kept colour.
            let proj = (pc[0] * kdir[0] + pc[1] * kdir[1] + pc[2] * kdir[2]).max(0.0) * spill;
            let despilled = [
                u[0] - proj * kdir[0],
                u[1] - proj * kdir[1],
                u[2] - proj * kdir[2],
            ];
            let out_a = a * keep;
            for c in 0..3 {
                let proc = despilled[c] * out_a;
                px[c] = px[c] * (1.0 - mix) + proc * mix;
            }
            px[3] = a * (1.0 - mix) + out_a * mix;
        }
    }

    /// Exposure (docs/08 §3.16): a scene-linear gain on RGB. Premultiplied
    /// colour scales consistently under a scalar, so there is no unpremultiply
    /// round trip and alpha is untouched. `factor` (= 2^stops) 1.0 is the
    /// bit-exact neutral point (the WGSL twin matches its early return); Mix 0
    /// is likewise the identity.
    pub fn exposure(rgba: &mut [f32], factor: f32, mix: f32) {
        if factor == 1.0 {
            return;
        }
        for px in rgba.chunks_exact_mut(4) {
            for ch in &mut px[..3] {
                let scaled = *ch * factor;
                *ch = *ch * (1.0 - mix) + scaled * mix;
            }
        }
    }

    /// Hue shift (docs/08 §3.17): a row-major linear 3×3 colour matrix `m`
    /// (from [`super::hue_matrix`]) applied to RGB, alpha untouched. Works on
    /// premultiplied colour directly — a linear matrix scales through alpha —
    /// so no unpremultiply round trip. The identity matrix is the bit-exact
    /// neutral point (the WGSL twin matches); Mix 0 is likewise the identity.
    pub fn hue_shift(rgba: &mut [f32], m: [f32; 9], mix: f32) {
        if m == [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            return;
        }
        for px in rgba.chunks_exact_mut(4) {
            let (r, g, b) = (px[0], px[1], px[2]);
            let nr = m[0] * r + m[1] * g + m[2] * b;
            let ng = m[3] * r + m[4] * g + m[5] * b;
            let nb = m[6] * r + m[7] * g + m[8] * b;
            px[0] = r * (1.0 - mix) + nr * mix;
            px[1] = g * (1.0 - mix) + ng * mix;
            px[2] = b * (1.0 - mix) + nb * mix;
        }
    }

    /// The mid-grey pivot contrast expands or compresses about (docs/08 §3.18).
    pub const CONTRAST_PIVOT: f32 = 0.5;

    /// Contrast (docs/08 §3.18): the affine grade `(u − pivot) × k + pivot` per
    /// RGB channel about the fixed mid-grey pivot (0.5), in linear light on
    /// unpremultiplied colour (§2.2), re-premultiplied on the way out —
    /// exactly Saturation's premultiply handling. The `− pivot` offset is why
    /// this cannot run through premultiplied alpha: it is an affine grade, not
    /// a pure scale, so it does not commute with the alpha multiply. `k` 1.0
    /// (Contrast 100 %) short-circuits the whole effect (bit-exact identity;
    /// the WGSL twin matches). Purely continuous — no round/clamp/quantize — so
    /// it is safe under the §1.6 fp16 ULP oracle. Highlights are never clipped
    /// (§2.1) and values may go negative between grade and re-premultiply; that
    /// is the honest affine result, matched op-for-op by the kernel.
    pub fn contrast(rgba: &mut [f32], k: f32, mix: f32) {
        if k == 1.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            for c in 0..3 {
                let v = (u[c] - CONTRAST_PIVOT) * k + CONTRAST_PIVOT;
                let graded = v * a;
                px[c] = px[c] * (1.0 - mix) + graded * mix;
            }
        }
    }

    /// Gamma (docs/08 §3.19): a per-channel power curve
    /// `out = pow(max(u, 0), 1/gamma)` in the compositor's scene-linear working
    /// space, on unpremultiplied colour (§2.2), re-premultiplied on the way out
    /// — exactly Contrast's and Saturation's premultiply handling. pow is
    /// non-linear, so it does not commute with the alpha multiply: the pixel is
    /// unpremultiplied, curved, then re-premultiplied. The input is clamped to
    /// ≥ 0 before the pow (scene-linear colour can dip slightly negative, and
    /// pow of a negative base is undefined); the clamp is byte-identical in the
    /// WGSL twin so the §1.6 oracle holds. `gamma` 1.0 short-circuits the whole
    /// effect (bit-exact identity — a short-circuit, not a reliance on
    /// `pow(x, 1)` being exactly `x`; the WGSL twin matches). Continuous for
    /// input ≥ 0, so it is safe under the §1.6 fp16 ULP oracle. `gamma` is
    /// clamped ≥ 0.01 at resolve so `1/gamma` stays finite; alpha is untouched.
    pub fn gamma(rgba: &mut [f32], gamma: f32, mix: f32) {
        if gamma == 1.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        let inv = 1.0 / gamma;
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            for c in 0..3 {
                let curved = u[c].max(0.0).powf(inv);
                let graded = curved * a;
                px[c] = px[c] * (1.0 - mix) + graded * mix;
            }
        }
    }

    /// Temperature (docs/08 §3.20): a warm/cool white-balance shift as a
    /// per-channel gain in scene-linear light — red by `gain_r`, blue by
    /// `gain_b`, green and alpha untouched. Like Exposure, a per-channel scalar
    /// scales premultiplied colour consistently (straight × gain, then × the
    /// unchanged alpha), so there is no unpremultiply round trip — unlike the
    /// affine Contrast/Saturation grades, whose − pivot offset breaks that
    /// commutation. The gains are computed host-side (in the resolve step) so
    /// the CPU reference and the WGSL kernel multiply by the identical numbers
    /// (§1.6). Gains `(1.0, 1.0)` (Temperature 0) short-circuit the whole
    /// effect — the bit-exact neutral point (the WGSL twin matches); Mix 0 is
    /// likewise the identity. Purely continuous (a linear per-channel scale),
    /// so it is safe under the §1.6 fp16 ULP oracle; highlights are never
    /// clipped (§2.1).
    pub fn temperature(rgba: &mut [f32], gain_r: f32, gain_b: f32, mix: f32) {
        if gain_r == 1.0 && gain_b == 1.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        for px in rgba.chunks_exact_mut(4) {
            let sr = px[0] * gain_r;
            let sb = px[2] * gain_b;
            px[0] = px[0] * (1.0 - mix) + sr * mix;
            px[2] = px[2] * (1.0 - mix) + sb * mix;
        }
    }

    /// Invert (docs/08 §3.23): the colour inverse `out.rgb = 1 − u` per RGB
    /// channel in the compositor's scene-linear working space, on
    /// unpremultiplied colour (§2.2), re-premultiplied on the way out — exactly
    /// Contrast's and Gamma's premultiply handling. `1 − c` is affine, so it does
    /// not commute with premultiplied alpha: the pixel is unpremultiplied,
    /// inverted, then re-premultiplied, so matte edges do not fringe. The inverse
    /// is a plain `1 − c` in scene-linear light — the owner's "simple inverse" —
    /// so HDR values above 1 invert to honest negatives, never clipped (§2.1).
    /// There is no neutral value (invert always inverts); Mix 0 is the bit-exact
    /// identity (the `× (1 − mix) + · × mix` blend collapses to the input), and
    /// the WGSL twin matches. Purely continuous, so it is safe under the §1.6
    /// fp16 ULP oracle. Alpha is untouched.
    pub fn invert(rgba: &mut [f32], mix: f32) {
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            for c in 0..3 {
                let inverted = (1.0 - u[c]) * a;
                px[c] = px[c] * (1.0 - mix) + inverted * mix;
            }
        }
    }

    /// Tint (docs/08 §3.24): a luminance duotone / gradient map
    /// `out.rgb = black + (white − black)·luma(u)` per RGB channel, with Rec.709
    /// `luma` on the unpremultiplied colour `u` (§2.2), re-premultiplied on the
    /// way out — exactly Contrast's and Gamma's premultiply handling. A
    /// luma-driven colour remap does not commute with premultiplied alpha, so the
    /// pixel is unpremultiplied, mapped, then re-premultiplied, and matte edges do
    /// not fringe. The lerp is written `black + (white − black)·luma` (not the
    /// `black·(1 − luma) + white·luma` form) so the CPU reference and the WGSL
    /// kernel reduce in the same order and the §1.6 oracle holds. The default
    /// black→black / white→white maps every pixel to its own luma (a greyscale) —
    /// a visible tasteful default, not a no-op; Mix 0 is the bit-exact identity
    /// (the WGSL twin matches). Purely continuous, so it is safe under the §1.6
    /// fp16 ULP oracle. Alpha is untouched.
    pub fn tint(rgba: &mut [f32], black: [f32; 3], white: [f32; 3], mix: f32) {
        for px in rgba.chunks_exact_mut(4) {
            let a = px[3];
            let u = unpremult(px);
            let luma = u[0] * LUMA[0] + u[1] * LUMA[1] + u[2] * LUMA[2];
            for c in 0..3 {
                let mapped = black[c] + (white[c] - black[c]) * luma;
                let graded = mapped * a;
                px[c] = px[c] * (1.0 - mix) + graded * mix;
            }
        }
    }

    /// Vignette (docs/08 §3.14): darkens toward black away from the frame
    /// centre, on premultiplied colour — a coverage-like darkening, not a
    /// colour grade, so no unpremultiply round trip (alpha is untouched).
    /// Roundness blends the distance metric between a true circle (1: both
    /// axes normalised by the shorter side, so equal pixel distances read
    /// as equal) and an ellipse that exactly reaches the frame's own edges
    /// (0: each axis normalised by its own half-extent) — the schema's own
    /// description of the knob. Radius is the clear centre's reach in that
    /// normalised metric (1.0 = the metric's own reference edge) and
    /// Softness the feather beyond it; the feather width floors at a small
    /// epsilon so Softness 0 reads as a hard edge rather than a division by
    /// zero. Amount 0 is the neutral point (bit-exact passthrough, pinned
    /// by test — the WGSL twin matches).
    #[allow(clippy::too_many_arguments)]
    pub fn vignette(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        amount: f32,
        radius: f32,
        softness: f32,
        roundness: f32,
        mix: f32,
    ) {
        if amount == 0.0 {
            return;
        }
        let (fw, fh) = (w as f32, h as f32);
        if fw <= 0.0 || fh <= 0.0 {
            return;
        }
        let half = fw.min(fh) * 0.5;
        let rx = (fw * 0.5) * (1.0 - roundness) + half * roundness;
        let ry = (fh * 0.5) * (1.0 - roundness) + half * roundness;
        let (cx, cy) = (fw * 0.5, fh * 0.5);
        let edge0 = radius;
        let edge1 = radius + softness.max(1e-6);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let nx = (x as f32 + 0.5 - cx) / rx;
                let ny = (y as f32 + 0.5 - cy) / ry;
                let dist = (nx * nx + ny * ny).sqrt();
                let t = ((dist - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
                let s = t * t * (3.0 - 2.0 * t);
                let vig = (s * amount).clamp(0.0, 1.0);
                let keep = 1.0 - vig;
                for c in 0..3 {
                    let darkened = rgba[i + c] * keep;
                    rgba[i + c] = rgba[i + c] * (1.0 - mix) + darkened * mix;
                }
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

    /// The §1.6 oracle for Echo (docs/08 §3.13): the CPU twin of `fx_echo.wgsl`,
    /// op-for-op. `current` is the leading (this-frame) linear premultiplied
    /// RGBA; `neighbours` are the layer's decoded source frames keyed by their
    /// frame offset (all the same length as `current`). `weights[i]` is the
    /// tap intensity for the echo at offset `-(i+1)`; a zero weight or a
    /// missing neighbour is skipped. `mode` is 0 = Add, 1 = Behind (the
    /// accumulator over the echo), 2 = Max. Finally the trail is blended
    /// toward `current` by `mix`. Working colour is premultiplied, so a tap
    /// scales all four channels together — the correct premultiplied fade.
    pub fn echo(
        current: &[f32],
        neighbours: &[(i32, &[f32])],
        weights: [f32; 8],
        mode: u32,
        mix: f32,
    ) -> Vec<f32> {
        let mut out = current.to_vec();
        for (px_idx, o) in out.chunks_exact_mut(4).enumerate() {
            let mut acc = [
                current[px_idx * 4],
                current[px_idx * 4 + 1],
                current[px_idx * 4 + 2],
                current[px_idx * 4 + 3],
            ];
            for (i, &weight) in weights.iter().enumerate() {
                if weight <= 0.0 {
                    continue;
                }
                let offset = -(i as i32 + 1);
                let Some((_, buf)) = neighbours.iter().find(|(oo, _)| *oo == offset) else {
                    continue;
                };
                let base = px_idx * 4;
                let n = [
                    buf[base] * weight,
                    buf[base + 1] * weight,
                    buf[base + 2] * weight,
                    buf[base + 3] * weight,
                ];
                acc = match mode {
                    0 => [acc[0] + n[0], acc[1] + n[1], acc[2] + n[2], acc[3] + n[3]],
                    1 => {
                        let k = 1.0 - acc[3];
                        [
                            acc[0] + n[0] * k,
                            acc[1] + n[1] * k,
                            acc[2] + n[2] * k,
                            acc[3] + n[3] * k,
                        ]
                    }
                    _ => [
                        acc[0].max(n[0]),
                        acc[1].max(n[1]),
                        acc[2].max(n[2]),
                        acc[3].max(n[3]),
                    ],
                };
            }
            for c in 0..4 {
                o[c] = current[px_idx * 4 + c] * (1.0 - mix) + acc[c] * mix;
            }
        }
        out
    }

    /// The §1.6 oracle for Flow motion blur (docs/08 §3.2): the CPU twin of
    /// `fx_motionblur.wgsl`, op-for-op. `rgba` is linear premultiplied RGBA,
    /// mutated in place; `u`/`v` are the per-pixel forward flow (pixels of
    /// this raster, one entry per pixel) the decode worker measured between
    /// the current source frame and the next. Each pixel's streak vector is
    /// its own motion scaled by `shutter_frac` (shutter ÷ 360); the streak is
    /// a centred box integral of `samples` evenly spaced bilinear taps — the
    /// same line-integral shape as [`blur_directional`], but per-pixel
    /// directed by the flow rather than one global angle. Fixed tap order and
    /// count for determinism (§2.4). Edges clamp (the shared [`bilinear`]
    /// rule), so a full-frame smear never darkens the border. A zero streak —
    /// `shutter_frac == 0.0` or a still pixel — collapses every tap onto the
    /// pixel itself, so with `mix == 1.0` the result is the bit-exact input.
    #[allow(clippy::too_many_arguments)]
    pub fn motion_blur(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        u: &[f32],
        v: &[f32],
        shutter_frac: f32,
        samples: i32,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        let n = samples.max(1);
        let nf = n as f32;
        for y in 0..h {
            for x in 0..w {
                let idx = (y * w + x) as usize;
                let i = idx * 4;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);
                // The full streak vector: this pixel's inter-frame motion,
                // shortened by the shutter fraction. Taps span it centred.
                let sv = (u[idx] * shutter_frac, v[idx] * shutter_frac);
                let mut acc = [0.0f32; 4];
                for k in 0..n {
                    let t = (k as f32 + 0.5) / nf - 0.5;
                    let s = bilinear(&original, w, h, pos.0 + t * sv.0, pos.1 + t * sv.1);
                    for c in 0..4 {
                        acc[c] += s[c];
                    }
                }
                for c in 0..4 {
                    let vv = acc[c] / nf;
                    rgba[i + c] = original[i + c] * (1.0 - mix) + vv * mix;
                }
            }
        }
    }

    /// The §1.6 oracle for Datamosh (docs/08 §3.12, K-104): the CPU twin of
    /// `fx_datamosh.wgsl`, op-for-op. `current` is the already block/scanline'd
    /// frame (linear premultiplied RGBA) this section blends against; `prev`
    /// is the raw -1 source neighbour; `u`/`v` are the dense flow the decode
    /// worker measured from the current frame to it (this raster's pixel
    /// grid, one entry per pixel — the same current→neighbour convention
    /// [`motion_blur`] uses for its own +1 neighbour, just pointed at -1). A
    /// single bilinear tap per pixel, not a streak integral: this looks up
    /// one displaced source pixel (motion-compensated prediction), not a
    /// line integral of motion. `intensity == 0.0` collapses every warped tap
    /// weight to zero, so the result is the bit-exact `current` input.
    pub fn datamosh(
        current: &[f32],
        prev: &[f32],
        w: u32,
        h: u32,
        u: &[f32],
        v: &[f32],
        intensity: f32,
    ) -> Vec<f32> {
        let mut out = current.to_vec();
        for y in 0..h {
            for x in 0..w {
                let idx = (y * w + x) as usize;
                let i = idx * 4;
                let pos = (x as f32 + 0.5 + u[idx], y as f32 + 0.5 + v[idx]);
                let warped = bilinear(prev, w, h, pos.0, pos.1);
                for c in 0..4 {
                    out[i + c] = current[i + c] * (1.0 - intensity) + warped[c] * intensity;
                }
            }
        }
        out
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

    /// Chromatic aberration (docs/08 §3.15): a dedicated, always-radial
    /// sibling of [`rgb_split`]'s own Radial mode — same R-behind/B-ahead
    /// shape, always centred on the frame, no angle or linear mode of its
    /// own. R samples pulled toward centre, B pulled away, so R reads
    /// outward and B inward in the rendered image; G and alpha stay put.
    /// Premultiplied throughout; samples outside the frame clamp to the
    /// edge. Amount 0 is already the bit-exact passthrough through the
    /// general formula (`k` is an exact `0.0`, so every tap lands on its
    /// own pixel) — no separate short-circuit is needed, mirroring
    /// `rgb_split`'s own un-guarded style.
    pub fn chromatic_aberration(rgba: &mut [f32], w: u32, h: u32, amount_px: f32, mix: f32) {
        let original = rgba.to_vec();
        let (fw, fh) = (w as f32, h as f32);
        let diag = (fw * fw + fh * fh).sqrt();
        let k = amount_px / (0.5 * diag);
        let (cx, cy) = (fw * 0.5, fh * 0.5);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);
                let (ox, oy) = ((pos.0 - cx) * k, (pos.1 - cy) * k);
                let r = bilinear(&original, w, h, pos.0 - ox, pos.1 - oy)[0];
                let b = bilinear(&original, w, h, pos.0 + ox, pos.1 + oy)[2];
                let split = [r, original[i + 1], b, original[i + 3]];
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

    /// The radial blur's tap count for a peak per-pixel spread in pixels
    /// (docs/08 §3.8): the same rule as [`dir_blur_taps`], sized from the
    /// worst case — the spread reached at the frame's farthest corner —
    /// so CPU and GPU dispatch the same kernel size everywhere in the
    /// image (nearer Centre simply over-samples a shorter true spread,
    /// which costs taps but is never wrong).
    pub fn radial_blur_taps(amount_px: f32) -> i32 {
        dir_blur_taps(amount_px)
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

    /// Radial blur (docs/08 §3.8, schema status note): Spin samples along
    /// an arc about Centre, Zoom along a ray through it — box-weighted,
    /// evenly spaced taps across `[-0.5, 0.5]` exactly like
    /// [`blur_directional`]'s line integral, fixed tap order for
    /// determinism (§2.4). Both reduce to one linear scale of `d = pos −
    /// centre`: Zoom's ray is `pos + t·k·d` (an exact sample along the ray,
    /// since scaling `d` moves along the straight line through Centre and
    /// `pos`); Spin's arc is `pos + t·k·rot90(d)` (the first-order/tangent
    /// approximation to true rotation about Centre — accurate for the
    /// small sweep angles `k` reaches across the shipped Amount range).
    /// `k = amount_px / (half the raster diagonal)` is the same radial
    /// scale [`rgb_split`]'s radial mode uses. Neither branch divides by
    /// `|d|`, so every tap collapses to exactly `pos` at Centre — no
    /// epsilon guard, no NaN risk. `amount_px == 0.0` gives `k == 0.0`,
    /// [`radial_blur_taps`] floors at one tap (mirroring
    /// [`dir_blur_taps`]'s floor), and that single tap sits at exactly
    /// `pos`: with `mix == 1.0` the result is the bit-exact input (pinned
    /// by test, matching the directional blur's own zero-length case).
    #[allow(clippy::too_many_arguments)]
    pub fn blur_radial(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        centre_frac: [f32; 2],
        amount_px: f32,
        spin: bool,
        edge: u32,
        mix: f32,
    ) {
        let original = rgba.to_vec();
        let (fw, fh) = (w as f32, h as f32);
        let centre = (centre_frac[0] * fw, centre_frac[1] * fh);
        let diag = (fw * fw + fh * fh).sqrt();
        let k = if diag > 0.0 {
            amount_px / (0.5 * diag)
        } else {
            0.0
        };
        let n = radial_blur_taps(amount_px);
        let nf = n as f32;
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);
                let d = (pos.0 - centre.0, pos.1 - centre.1);
                // Zoom steps along d itself (a ray through Centre); Spin
                // steps along its perpendicular (the tangent to the arc).
                let step = if spin { (-d.1, d.0) } else { d };
                let mut acc = [0.0f32; 4];
                for t in 0..n {
                    let tt = (t as f32 + 0.5) / nf - 0.5;
                    let s = bilinear_edge(
                        &original,
                        w,
                        h,
                        pos.0 + tt * k * step.0,
                        pos.1 + tt * k * step.1,
                        edge,
                    );
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

    /// Block glitch (docs/08 §3.12, split out by K-107): standalone block
    /// displacement, the block section of the old combined Glitch effect.
    ///
    /// Partitions the raster into a `block_size_px` grid; each *nominal*
    /// block hashes a small jitter offset (`jitter_frac` of `block_size_px`,
    /// scaled by Intensity) that decides which block's content a pixel
    /// actually reads from — a cheap stand-in for moving grid lines
    /// themselves. That block then hashes its own displacement (±
    /// `amount_px` per axis), R/B channel split (± `chan_px`, alpha follows
    /// green exactly like [`rgb_split`]), and slice-repeat odds
    /// (`slice_frac` × Intensity: folds the block's own local Y to a short
    /// hashed repeat height instead of a plain read). Every hashed quantity
    /// is scaled by Intensity, so Intensity 0 collapses every read back to
    /// the pixel's own position — pinned as the bit-exact passthrough by
    /// the early return below (matching [`glow`]'s neutral short-circuit,
    /// not the tap-sum coincidence the blur family relies on, because
    /// Mix should not be able to perturb a fully neutral instance either).
    ///
    /// Clamp-addressed bilinear sampling throughout (like [`rgb_split`]);
    /// fixed evaluation order for determinism (§2.4).
    #[allow(clippy::too_many_arguments)]
    pub fn block_glitch(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        intensity: f32,
        seed: u32,
        tick: i32,
        block_size_px: f32,
        jitter_frac: f32,
        amount_px: f32,
        chan_px: f32,
        slice_frac: f32,
        mix: f32,
    ) {
        if intensity == 0.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        let original = rgba.to_vec();
        let bw = block_size_px.max(1.0);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos = (x as f32 + 0.5, y as f32 + 0.5);

                let bx0 = (pos.0 / bw).floor();
                let by0 = (pos.1 / bw).floor();
                let h01 = |ch: u32, bxx: f32, byy: f32| {
                    super::block_hash01(seed, ch, bxx as i32, byy as i32, tick)
                };
                // Grid jitter (status note): a hashed offset of the
                // *nominal* block, scaled by Intensity, decides which
                // block a pixel actually reads from.
                let jx = (h01(0, bx0, by0) - 0.5) * 2.0 * jitter_frac * bw * intensity;
                let jy = (h01(1, bx0, by0) - 0.5) * 2.0 * jitter_frac * bw * intensity;
                let jpos = (pos.0 + jx, pos.1 + jy);
                let bx = (jpos.0 / bw).floor();
                let by = (jpos.1 / bw).floor();

                let dx = (h01(2, bx, by) - 0.5) * 2.0 * amount_px * intensity;
                let dy = (h01(3, bx, by) - 0.5) * 2.0 * amount_px * intensity;
                let chan = (h01(4, bx, by) - 0.5) * 2.0 * chan_px * intensity;
                let slice_u = h01(5, bx, by);
                let slice_h_u = h01(6, bx, by);

                // Slice repeat: fold the block's own local Y to a short
                // hashed repeat height instead of a plain read.
                let mut eff_y = jpos.1;
                if slice_u < slice_frac * intensity {
                    let local_y = jpos.1 - by * bw;
                    let repeat_h = (slice_h_u * bw * 0.25).max(1.0);
                    let folded = local_y - (local_y / repeat_h).floor() * repeat_h;
                    eff_y = by * bw + folded;
                }
                let (sx, sy) = (jpos.0 + dx, eff_y + dy);

                // R/B split from the block hash (alpha follows green, like
                // rgb_split).
                let r = bilinear(&original, w, h, sx - chan, sy)[0];
                let g = bilinear(&original, w, h, sx, sy);
                let b = bilinear(&original, w, h, sx + chan, sy)[2];
                let c = [r, g[1], b, g[3]];

                for ch in 0..4 {
                    rgba[i + ch] = original[i + ch] * (1.0 - mix) + c[ch] * mix;
                }
            }
        }
    }

    /// Scanlines (docs/08 §3.12, split out by K-107): standalone periodic
    /// darken, the scanline section of the old combined Glitch effect. No
    /// hash, no block resample — reads the input pixel directly (pointwise,
    /// [`Roi::Exact`](super::Roi::Exact)), darkens by a periodic band in
    /// raster Y (plus the precomputed roll offset), alternating which half
    /// of the period darkens on odd periods when Interlace is on. Intensity
    /// 0 is the bit-exact passthrough, pinned by the early return below —
    /// the same neutral shape [`block_glitch`] uses.
    #[allow(clippy::too_many_arguments)]
    pub fn scanlines(
        rgba: &mut [f32],
        w: u32,
        h: u32,
        intensity: f32,
        period_px: f32,
        darkness: f32,
        roll_px: f32,
        interlace: bool,
        mix: f32,
    ) {
        if intensity == 0.0 {
            return; // neutral: bit-exact identity (the WGSL twin matches)
        }
        let original = rgba.to_vec();
        let period = period_px.max(1.0);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let pos_y = y as f32 + 0.5;
                let mut c = [
                    original[i],
                    original[i + 1],
                    original[i + 2],
                    original[i + 3],
                ];

                let yp = pos_y + roll_px;
                let cell = yp / period;
                let cell_floor = cell.floor();
                let t = cell - cell_floor;
                let odd = (cell_floor as i64).rem_euclid(2) != 0;
                let bright = (t < 0.5) != (interlace && odd);
                let band = if bright { 1.0 } else { 1.0 - darkness };
                let eff_mult = 1.0 - intensity * (1.0 - band);
                c[0] *= eff_mult;
                c[1] *= eff_mult;
                c[2] *= eff_mult;

                for ch in 0..4 {
                    rgba[i + ch] = original[i + ch] * (1.0 - mix) + c[ch] * mix;
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
        let r = resolve_stack(&[e.clone()], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::Blur {
                radius_px: 15.0,
                edge: 1,
                mix: 1.0
            }]
        );
        e.enabled = false;
        assert!(resolve_stack(&[e.clone()], 0.0, 1000.0, 1.0, &MarkerContext::NONE).is_empty());
        e.enabled = true;
        e.effect.namespace = EffectNamespace::Placeholder;
        assert!(
            resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE).is_empty(),
            "placeholders render as identity"
        );
    }

    #[test]
    fn dof_instantiates_unset_and_resolves_its_floats() {
        let e = instantiate("dof").unwrap();
        assert_eq!(e.effect.match_name, "dof");
        assert_eq!(e.effect.version, 1);
        // A fresh depth reference is unset — the effect is a labelled no-op
        // until a layer is picked (its run_ops depth slot is None, a
        // passthrough), the sanctioned exception the File parameter also takes.
        assert!(matches!(e.param("depth"), Some(EffectValue::Layer(None))));
        assert_eq!(e.layer_ref("depth"), None);
        assert_eq!(e.float_at("focus", 0.0), Some(0.5));
        assert_eq!(e.float_at("range", 0.0), Some(0.1));
        assert_eq!(e.float_at("aperture", 0.0), Some(8.0));
        assert_eq!(e.float_at("near_aperture", 0.0), Some(8.0));
        assert_eq!(e.float_at("far_aperture", 0.0), Some(8.0));
        assert_eq!(e.float_at("mix", 0.0), Some(100.0));
        // Depth invert is off by default (the historical reading).
        assert!(matches!(
            e.param("depth_invert"),
            Some(EffectValue::Bool(false))
        ));
        // Display defaults to Rendered (the normal blurred output).
        assert!(matches!(e.param("display"), Some(EffectValue::Choice(0))));

        // resolve_stack carries only the scalars; the depth is threaded beside
        // the op. The default Aperture master (8) is unity, so each side
        // resolves to its Near/Far radius (8) scaled by the §2.3 preview factor
        // (here 0.5 → 4 raster px). A `dof` always resolves to exactly one
        // Resolved::Dof, so it stays 1:1 and in order with the depth-input list
        // even when the depth reference is unset.
        let r = resolve_stack(&[e], 0.0, 1000.0, 0.5, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::Dof {
                focus: 0.5,
                range: 0.1,
                near_aperture: 4.0,
                far_aperture: 4.0,
                depth_invert: false,
                display: 0,
                mix: 1.0,
            }]
        );
    }

    #[test]
    fn dof_near_far_override_and_fall_back_to_the_aperture_master() {
        // Near/Far override the per-side radii; the Aperture master scales both
        // about its default 8. Set Aperture 16 (master 2×), Near 10, Far 4.
        let mut e = instantiate("dof").unwrap();
        for p in e.params.iter_mut() {
            match p.id.as_str() {
                "aperture" => p.value = EffectValue::Float(Property::fixed(16.0)),
                "near_aperture" => p.value = EffectValue::Float(Property::fixed(10.0)),
                "far_aperture" => p.value = EffectValue::Float(Property::fixed(4.0)),
                _ => {}
            }
        }
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::Dof {
                focus: 0.5,
                range: 0.1,
                near_aperture: 20.0, // 10 · (16/8)
                far_aperture: 8.0,   // 4 · (16/8)
                depth_invert: false,
                display: 0,
                mix: 1.0,
            }]
        );

        // A legacy instance saved before the Near/Far pair existed has only
        // `aperture`; both sides then fall back to it, reproducing the old
        // symmetric single-aperture behaviour exactly.
        let mut legacy = instantiate("dof").unwrap();
        for p in legacy.params.iter_mut() {
            if p.id == "aperture" {
                p.value = EffectValue::Float(Property::fixed(12.0));
            }
        }
        legacy
            .params
            .retain(|p| p.id != "near_aperture" && p.id != "far_aperture");
        let r = resolve_stack(
            std::slice::from_ref(&legacy),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::Dof {
                focus: 0.5,
                range: 0.1,
                near_aperture: 12.0, // 8 (default) · (12/8)
                far_aperture: 12.0,
                depth_invert: false,
                display: 0,
                mix: 1.0,
            }]
        );
    }

    #[test]
    fn layer_param_round_trips_through_serde() {
        // A Layer parameter survives a JSON round-trip set and unset, and
        // `layer_ref` reads back the id the caller renders as the depth pass.
        let id = uuid::Uuid::now_v7();
        let mut e = instantiate("dof").unwrap();
        if let Some(p) = e.params.iter_mut().find(|p| p.id == "depth") {
            p.value = EffectValue::Layer(Some(id));
        }
        let json = serde_json::to_string(&e).unwrap();
        let back: EffectInstance = serde_json::from_str(&json).unwrap();
        assert_eq!(back.layer_ref("depth"), Some(id));

        // The unset reference round-trips as such (a passthrough, never lost).
        let unset = EffectValue::Layer(None);
        let j = serde_json::to_string(&unset).unwrap();
        assert_eq!(serde_json::from_str::<EffectValue>(&j).unwrap(), unset);
    }

    #[test]
    fn temporal_window_is_zero_until_a_temporal_effect_joins() {
        // Every current built-in is single-frame (temporal &[0]), so any
        // stack of them needs only the current frame.
        let blur = instantiate("blur").unwrap();
        let glow = instantiate("glow").unwrap();
        assert_eq!(
            stack_temporal_window(&[blur.clone(), glow.clone()], true),
            vec![0]
        );
        assert!(!stack_is_temporal(&[blur.clone(), glow.clone()], true));
        // Bypassed stack, empty stack, and a disabled effect all reduce to
        // the current frame only.
        assert_eq!(stack_temporal_window(&[blur.clone(), glow], false), vec![0]);
        assert_eq!(stack_temporal_window(&[], true), vec![0]);
        let mut off = blur.clone();
        off.enabled = false;
        assert_eq!(stack_temporal_window(&[off], true), vec![0]);
        // The window always contains 0 and is sorted/deduped — pinned so a
        // temporal effect's offsets union cleanly with the current frame.
        assert!(stack_temporal_window(&[blur], true).contains(&0));
    }

    #[test]
    fn motion_blur_window_reaches_the_next_frame_and_wants_flow() {
        // Motion blur's window is {0, 1}: the current frame and one ahead,
        // the pair the flow engine measures motion between.
        let mb = instantiate("motion_blur").unwrap();
        let one = std::slice::from_ref(&mb);
        assert_eq!(stack_temporal_window(one, true), vec![0, 1]);
        assert!(stack_is_temporal(one, true));
        // The flow-field gate is set by motion blur and nothing else current.
        assert_eq!(stack_flow_neighbour(one, true), Some(1));
        let blur = instantiate("blur").unwrap();
        let echo = instantiate("echo").unwrap();
        assert_eq!(stack_flow_neighbour(&[blur.clone(), echo], true), None);
        // Bypassed by the layer fx switch, or disabled, it wants nothing.
        assert_eq!(stack_flow_neighbour(one, false), None);
        let mut off = mb.clone();
        off.enabled = false;
        assert_eq!(stack_flow_neighbour(std::slice::from_ref(&off), true), None);
    }

    #[test]
    fn datamosh_window_reaches_the_prior_frame_and_wants_flow() {
        // Datamosh's window is {-1, 0}: the current frame and one behind,
        // read statically off the schema (K-107 — no per-instance toggle,
        // unlike the old combined Glitch's dynamic special case).
        let dm = instantiate("datamosh").unwrap();
        let one = std::slice::from_ref(&dm);
        assert_eq!(stack_temporal_window(one, true), vec![-1, 0]);
        assert!(stack_is_temporal(one, true));
        assert_eq!(stack_flow_neighbour(one, true), Some(-1));

        // A plain Block glitch stays single-frame.
        let plain = instantiate("block_glitch").unwrap();
        let plain_one = std::slice::from_ref(&plain);
        assert_eq!(stack_temporal_window(plain_one, true), vec![0]);
        assert!(!stack_is_temporal(plain_one, true));
        assert_eq!(stack_flow_neighbour(plain_one, true), None);

        // Disabled, or the layer fx switch off, Datamosh wants nothing.
        let mut off = dm.clone();
        off.enabled = false;
        assert_eq!(
            stack_temporal_window(std::slice::from_ref(&off), true),
            vec![0]
        );
        assert_eq!(stack_flow_neighbour(std::slice::from_ref(&off), true), None);
        assert_eq!(stack_flow_neighbour(one, false), None);
    }

    #[test]
    fn motion_blur_and_datamosh_together_the_first_in_stack_order_wins() {
        // K-104: a layer can carry only one flow field per frame in v1: if
        // both a live Motion blur and a live Datamosh are in the same
        // stack, whichever comes first wins the single slot.
        let mb = instantiate("motion_blur").unwrap();
        let dm = instantiate("datamosh").unwrap();
        assert_eq!(
            stack_flow_neighbour(&[mb.clone(), dm.clone()], true),
            Some(1)
        );
        assert_eq!(stack_flow_neighbour(&[dm, mb], true), Some(-1));
    }

    #[test]
    fn datamosh_instantiates_and_resolves() {
        let e = instantiate("datamosh").unwrap();
        assert_eq!(e.float_at("intensity", 0.0), Some(0.5));
        assert_eq!(e.float_at("mix", 0.0), Some(100.0));

        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::Datamosh {
                intensity: 0.5,
                mix: 1.0,
            }]
        );

        // Intensity 0 and Mix 0 both resolve cleanly (the bit-exact
        // passthrough is enforced where the op actually runs, in lumit-gpu
        // and lumit-ui — this pins the resolve step carries both zeros
        // through untouched).
        let mut zero_intensity = e.clone();
        for p in &mut zero_intensity.params {
            if p.id == "intensity" {
                p.value = EffectValue::Float(Property::fixed(0.0));
            }
        }
        let r = resolve_stack(
            std::slice::from_ref(&zero_intensity),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::Datamosh {
                intensity: 0.0,
                mix: 1.0,
            }]
        );
    }

    #[test]
    fn cpu_apply_datamosh_is_a_passthrough() {
        // The single-buffer CPU dispatcher cannot carry a neighbour frame or
        // a flow field, so Resolved::Datamosh degrades to a no-op here,
        // exactly like Echo and Motion blur.
        let (w, h) = (5u32, 5u32);
        let img = transform_card(w, h);
        let mut out = img.clone();
        cpu::apply(
            &mut out,
            w,
            h,
            &Resolved::Datamosh {
                intensity: 1.0,
                mix: 1.0,
            },
        );
        assert_eq!(out, img);
    }

    #[test]
    fn resolve_motion_blur_converts_shutter_and_rounds_samples() {
        let e = instantiate("motion_blur").unwrap();
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        // Defaults: 180° → shutter_frac 0.5, 16 samples, full mix.
        assert_eq!(
            r,
            vec![Resolved::MotionBlur {
                shutter_frac: 0.5,
                samples: 16,
                mix: 1.0,
            }]
        );
        // A custom stack: 90° halves the streak; Samples rounds and clamps.
        let mut e = instantiate("motion_blur").unwrap();
        for p in e.params.iter_mut() {
            match p.id.as_str() {
                "shutter_angle" => p.value = EffectValue::Float(Property::fixed(90.0)),
                "samples" => p.value = EffectValue::Float(Property::fixed(8.4)),
                "mix" => p.value = EffectValue::Float(Property::fixed(50.0)),
                _ => {}
            }
        }
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::MotionBlur {
                shutter_frac: 0.25,
                samples: 8,
                mix: 0.5,
            }]
        );
    }

    #[test]
    fn cpu_motion_blur_still_and_zero_shutter_are_passthrough() {
        // A 9x9 with one bright premultiplied pixel in the middle.
        let (w, h) = (9u32, 9u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        let mid = ((4 * w + 4) * 4) as usize;
        img[mid..mid + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]);
        let n = (w * h) as usize;

        // Zero flow everywhere: every tap lands on the pixel itself, so with
        // Mix 1 the output is the bit-exact input whatever the shutter.
        let (zu, zv) = (vec![0.0f32; n], vec![0.0f32; n]);
        let mut still = img.clone();
        cpu::motion_blur(&mut still, w, h, &zu, &zv, 0.5, 16, 1.0);
        assert_eq!(still, img, "still pixels do not blur");

        // A real motion but a closed shutter (frac 0) is also identity.
        let (mu, mv) = (vec![3.0f32; n], vec![0.0f32; n]);
        let mut shut = img.clone();
        cpu::motion_blur(&mut shut, w, h, &mu, &mv, 0.0, 16, 1.0);
        assert_eq!(shut, img, "a closed shutter does not blur");

        // Mix 0 returns the input exactly, whatever the motion.
        let mut mixed = img.clone();
        cpu::motion_blur(&mut mixed, w, h, &mu, &mv, 0.5, 16, 0.0);
        assert_eq!(mixed, img, "mix 0 is a passthrough");
    }

    #[test]
    fn cpu_motion_blur_smears_along_the_flow() {
        // A vertical edge (left half bright, right half dark) smeared by a
        // constant horizontal flow should soften the edge along x while
        // leaving a pixel deep inside a flat region unchanged (a box streak
        // over constant colour is that colour) — the defining behaviour.
        let (w, h) = (16u32, 4u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let v = if x < w / 2 { 1.0 } else { 0.0 };
                img[i..i + 4].copy_from_slice(&[v, v, v, 1.0]);
            }
        }
        let n = (w * h) as usize;
        let (u, vv) = (vec![8.0f32; n], vec![0.0f32; n]); // 8px horizontal
        let mut out = img.clone();
        cpu::motion_blur(&mut out, w, h, &u, &vv, 0.5, 16, 1.0); // streak 4px

        // Indices on row 0 (a closure keeps clippy's erasing-op lint happy and
        // reads clearly as column, row).
        let idx = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        // A pixel far inside the bright flat region is untouched (1.0).
        let flat = idx(2, 0);
        assert!((out[flat] - 1.0).abs() < 1e-4, "flat interior is preserved");
        // A pixel far inside the dark flat region stays dark.
        let dark = idx(13, 0);
        assert!(out[dark].abs() < 1e-4, "dark interior stays dark");
        // The pixel just right of the edge picks up light from the bright
        // side it was smeared across — a genuine, directional softening.
        let edge = idx(8, 0);
        assert!(
            out[edge] > 0.05 && out[edge] < 0.95,
            "the edge softens along the flow: {}",
            out[edge]
        );
    }

    #[test]
    fn cpu_datamosh_zero_intensity_is_the_bit_exact_current_frame() {
        let (w, h) = (6u32, 4u32);
        let n = (w * h) as usize;
        let current: Vec<f32> = (0..n * 4).map(|i| (i % 7) as f32 * 0.1).collect();
        let prev: Vec<f32> = (0..n * 4).map(|i| (i % 5) as f32 * 0.2).collect();
        let (u, v) = (vec![3.0f32; n], vec![-2.0f32; n]);
        let out = cpu::datamosh(&current, &prev, w, h, &u, &v, 0.0);
        assert_eq!(out, current, "intensity 0 is a bit-exact passthrough");
    }

    #[test]
    fn cpu_datamosh_full_intensity_reads_the_shifted_previous_frame() {
        // A single bright premultiplied pixel in `prev`; datamosh at full
        // intensity with a flow that points straight at it should recover
        // that pixel's colour at the sampling position, not `current`'s.
        let (w, h) = (9u32, 9u32);
        let n = (w * h) as usize;
        let current = vec![0.0f32; n * 4]; // all black
        let mut prev = vec![0.0f32; n * 4];
        let bright = ((4 * w + 6) * 4) as usize; // (x=6, y=4)
        prev[bright..bright + 4].copy_from_slice(&[4.0, 2.0, 1.0, 1.0]);
        // Sample position for output pixel (4, 4) is (4 + u, 4 + v); pointing
        // it at (6, 4) means u = 2, v = 0.
        let mut u = vec![0.0f32; n];
        let v = vec![0.0f32; n];
        u[(4 * w + 4) as usize] = 2.0;
        let out = cpu::datamosh(&current, &prev, w, h, &u, &v, 1.0);
        let i = ((4 * w + 4) * 4) as usize;
        assert_eq!(&out[i..i + 4], &[4.0, 2.0, 1.0, 1.0]);
        // A pixel whose flow is zero and whose `prev` neighbourhood is dark
        // stays dark (current is also dark there) — no bleed from elsewhere.
        assert_eq!(&out[0..4], &[0.0, 0.0, 0.0, 0.0]);
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
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
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
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(r, vec![classic]);

        // Wavelength on: the same numbers arrive as SpectralSplit.
        for p in &mut e.params {
            if p.id == "wavelength" {
                p.value = EffectValue::Bool(true);
            }
        }
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(r, vec![classic]);
    }

    #[test]
    fn chromatic_aberration_instantiates_and_resolves() {
        let e = instantiate("chromatic_aberration").unwrap();
        assert_eq!(e.float_at("amount", 0.0), Some(4.0));
        // px@comp, not % diag: diag_px does not enter the conversion, unlike
        // rgb_split's own Amount — only the preview-resolution px_scale does.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::ChromaticAberration {
                amount_px: 4.0,
                mix: 1.0
            }]
        );
    }

    #[test]
    fn chromatic_aberration_amount_scales_with_the_preview_factor() {
        let e = instantiate("chromatic_aberration").unwrap();
        // Half preview (px_scale 0.5): px@comp parameters scale down with
        // it, exactly like Glitch's Block size (§2.3).
        let r = resolve_stack(&[e], 0.0, 1000.0, 0.5, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::ChromaticAberration {
                amount_px: 2.0,
                mix: 1.0
            }]
        );
    }

    #[test]
    fn cpu_chromatic_aberration_shifts_channels_radially_and_keeps_alpha() {
        // A white impulse in the middle of a black opaque frame — the same
        // corpus rgb_split's own radial-mode test uses.
        let (w, h) = (17u32, 9u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for px in img.chunks_exact_mut(4) {
            px[3] = 1.0;
        }
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let mid = at(8, 4);
        img[mid..mid + 3].copy_from_slice(&[1.0, 1.0, 1.0]);

        // Amount 0 and mix 0 are both the exact identity (the general
        // formula's own passthrough, mirroring rgb_split's un-guarded style).
        let mut a0 = img.clone();
        cpu::chromatic_aberration(&mut a0, w, h, 0.0, 1.0);
        assert_eq!(a0, img);
        let mut m0 = img.clone();
        cpu::chromatic_aberration(&mut m0, w, h, 5.0, 0.0);
        assert_eq!(m0, img);

        // The exact centre pixel is unmoved even at a huge amount: its own
        // (position − centre) vector is zero, so every tap collapses onto it.
        let mut c = img.clone();
        cpu::chromatic_aberration(&mut c, w, h, 20.0, 1.0);
        assert_eq!(c[mid], 1.0, "frame-centre red is unmoved");
        assert_eq!(c[mid + 2], 1.0, "frame-centre blue is unmoved");
        assert_eq!(c[mid + 1], 1.0, "green untouched everywhere");

        // At Amount = half the frame diagonal, k is exactly 1: every
        // pixel's R sample point algebraically collapses onto the frame
        // centre (`pos − (pos − centre)·1 = centre`) — and because every
        // coordinate here is an integer or half-integer well inside f32's
        // exact range, that cancellation is bit-exact, not approximate. So
        // red reads the centre's own red value (the impulse, 1.0)
        // everywhere: a clean, exact witness that the offset visibly moves
        // colour off-centre, which a single arbitrary amount cannot give
        // (a lone one-texel impulse can fall clean outside a shifted tap's
        // bilinear footprint, missing it entirely).
        let (fw, fh) = (w as f32, h as f32);
        let diag = (fw * fw + fh * fh).sqrt();
        let mut half_diag = img.clone();
        cpu::chromatic_aberration(&mut half_diag, w, h, 0.5 * diag, 1.0);
        assert!(
            half_diag.iter().step_by(4).all(|&r| r == 1.0),
            "every pixel's red reads the centre's red at Amount = half diagonal"
        );
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::Saturation {
                saturation: 1.0,
                mix: 1.0
            }]
        );
    }

    #[test]
    fn matte_key_instantiates_and_resolves_defaults() {
        let e = instantiate("matte_key").unwrap();
        // The defaults visibly key a green screen (a green key + a 20 %
        // tolerance); Spill is off so a fresh instance keys without despill.
        assert_eq!(e.colour_at("key", 0.0), Some([0.0, 0.6, 0.0, 1.0]));
        assert_eq!(e.float_at("tolerance", 0.0), Some(20.0));
        assert_eq!(e.float_at("softness", 0.0), Some(10.0));
        assert_eq!(e.float_at("spill", 0.0), Some(0.0));
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::MatteKey {
                key: [0.0, 0.6, 0.0, 1.0],
                tol: 0.2,
                soft: 0.1,
                spill: 0.0,
                mix: 1.0,
            }]
        );
    }

    #[test]
    fn exposure_instantiates_resolves_and_gains_light() {
        let e = instantiate("exposure").unwrap();
        assert_eq!(e.float_at("stops", 0.0), Some(0.0));
        // 0 stops resolves to a neutral factor of 1.0.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::Exposure {
                factor: 1.0,
                mix: 1.0
            }]
        );
        // The CPU reference: 0 stops is identity; +1 stop (factor 2) doubles
        // RGB and leaves alpha alone; Mix 0 is the identity at any factor.
        let mut neutral = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::exposure(&mut neutral, 1.0, 1.0);
        assert_eq!(neutral, vec![0.4, 0.5, 0.6, 1.0]);
        let mut bright = vec![0.2_f32, 0.3, 0.1, 0.8];
        cpu::exposure(&mut bright, 2.0, 1.0);
        assert_eq!(bright, vec![0.4, 0.6, 0.2, 0.8]);
        let mut mixed = vec![0.2_f32, 0.3, 0.1, 1.0];
        cpu::exposure(&mut mixed, 3.0, 0.0);
        assert_eq!(mixed, vec![0.2, 0.3, 0.1, 1.0]);
    }

    #[test]
    fn temperature_instantiates_resolves_and_warms_and_cools() {
        let e = instantiate("temperature").unwrap();
        assert_eq!(e.float_at("temperature", 0.0), Some(0.0));
        // Temperature 0 resolves to neutral gains of exactly 1.0 each.
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::Temperature {
                gain_r: 1.0,
                gain_b: 1.0,
                mix: 1.0
            }]
        );
        // +100 resolves to gains (1.5, 0.5): red boosted, blue cut. −100 is the
        // mirror (0.5, 1.5). The resolve step owns the gain formula.
        let mut warm = e.clone();
        for p in &mut warm.params {
            if p.id == "temperature" {
                p.value = EffectValue::Float(Property::fixed(100.0));
            }
        }
        assert_eq!(
            resolve_stack(&[warm], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
            vec![Resolved::Temperature {
                gain_r: 1.5,
                gain_b: 0.5,
                mix: 1.0
            }]
        );
        let mut cool = e;
        for p in &mut cool.params {
            if p.id == "temperature" {
                p.value = EffectValue::Float(Property::fixed(-100.0));
            }
        }
        assert_eq!(
            resolve_stack(&[cool], 0.0, 1000.0, 1.0, &MarkerContext::NONE),
            vec![Resolved::Temperature {
                gain_r: 0.5,
                gain_b: 1.5,
                mix: 1.0
            }]
        );
        // The CPU reference: neutral gains are the bit-exact identity; a warm
        // shift (gains 1.5 / 0.5) boosts red and cuts blue, green and alpha
        // untouched; Mix 0 is the identity at any gains.
        let mut neutral = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::temperature(&mut neutral, 1.0, 1.0, 1.0);
        assert_eq!(neutral, vec![0.4, 0.5, 0.6, 1.0]);
        let mut hot = vec![0.5_f32, 0.5, 0.5, 0.8];
        cpu::temperature(&mut hot, 1.5, 0.5, 1.0);
        assert_eq!(hot, vec![0.75, 0.5, 0.25, 0.8]);
        let mut mixed = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::temperature(&mut mixed, 1.5, 0.5, 0.0);
        assert_eq!(mixed, vec![0.4, 0.5, 0.6, 1.0]);
    }

    #[test]
    fn invert_instantiates_resolves_and_inverts() {
        let e = instantiate("invert").unwrap();
        // The only parameter is Mix, defaulting to 100 %.
        assert_eq!(e.float_at("mix", 0.0), Some(100.0));
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(r, vec![Resolved::Invert { mix: 1.0 }]);

        // The CPU reference: an opaque pixel inverts as 1 − c, alpha untouched.
        let mut opaque = vec![0.2_f32, 0.5, 0.9, 1.0];
        cpu::invert(&mut opaque, 1.0);
        for (v, want) in opaque.iter().zip([0.8_f32, 0.5, 0.1, 1.0]) {
            assert!((v - want).abs() < 1e-6, "opaque invert: {v} vs {want}");
        }
        // Mix 0 is the identity at any input.
        let mut m0 = vec![0.2_f32, 0.5, 0.9, 1.0];
        cpu::invert(&mut m0, 0.0);
        assert_eq!(m0, vec![0.2, 0.5, 0.9, 1.0]);

        // Half-alpha pixel: invert runs on the unpremultiplied colour and is
        // re-premultiplied — the round trip a naive invert of premultiplied
        // colour gets wrong. Straight (0.4,0.6,0.8) at alpha 0.5 is stored
        // premultiplied as (0.2,0.3,0.4); inverting the straight colour gives
        // (0.6,0.4,0.2), re-premultiplied to (0.3,0.2,0.1); alpha untouched.
        let mut half = vec![0.2_f32, 0.3, 0.4, 0.5];
        cpu::invert(&mut half, 1.0);
        for (v, want) in half.iter().zip([0.3_f32, 0.2, 0.1, 0.5]) {
            assert!((v - want).abs() < 1e-6, "half-alpha invert: {v} vs {want}");
        }

        // Scene-linear HDR values above 1 invert to honest negatives (§2.1).
        let mut hdr = vec![2.0_f32, 3.0, 0.5, 1.0];
        cpu::invert(&mut hdr, 1.0);
        for (v, want) in hdr.iter().zip([-1.0_f32, -2.0, 0.5, 1.0]) {
            assert!((v - want).abs() < 1e-6, "hdr invert: {v} vs {want}");
        }
    }

    #[test]
    fn tint_instantiates_resolves_and_maps_luma() {
        let e = instantiate("tint").unwrap();
        assert_eq!(e.colour_at("black", 0.0), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(e.colour_at("white", 0.0), Some([1.0, 1.0, 1.0, 1.0]));
        // Defaults resolve to black→black, white→white (a greyscale mapping).
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::Tint {
                black: [0.0, 0.0, 0.0],
                white: [1.0, 1.0, 1.0],
                mix: 1.0
            }]
        );

        // The CPU reference: default black→black / white→white maps every pixel
        // to its own Rec.709 luma in all three channels (a greyscale).
        let rgb = [0.8_f32, 0.2, 0.5];
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        let mut grey = vec![rgb[0], rgb[1], rgb[2], 1.0];
        cpu::tint(&mut grey, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0);
        for v in grey.iter().take(3) {
            assert!((v - luma).abs() < 1e-6, "greyscale luma: {v} vs {luma}");
        }
        assert_eq!(grey[3], 1.0, "alpha untouched");

        // A duotone: black→(0.1,0,0.2), white→(0.9,0.8,1.0). Each channel lerps
        // by the pixel's luma. Mix 0 is the identity at any colours.
        let black = [0.1_f32, 0.0, 0.2];
        let white = [0.9_f32, 0.8, 1.0];
        let mut duo = vec![rgb[0], rgb[1], rgb[2], 1.0];
        cpu::tint(&mut duo, black, white, 1.0);
        for c in 0..3 {
            let want = black[c] + (white[c] - black[c]) * luma;
            assert!(
                (duo[c] - want).abs() < 1e-6,
                "duotone ch{c}: {} vs {want}",
                duo[c]
            );
        }
        let mut m0 = vec![rgb[0], rgb[1], rgb[2], 1.0];
        cpu::tint(&mut m0, black, white, 0.0);
        assert_eq!(m0, vec![rgb[0], rgb[1], rgb[2], 1.0]);

        // Half-alpha pixel: the map runs on the unpremultiplied colour and is
        // re-premultiplied. Straight (0.8,0.2,0.5) at alpha 0.5 is stored
        // premultiplied as (0.4,0.1,0.25); with defaults it maps to the straight
        // luma in each channel, re-premultiplied to luma·0.5; alpha untouched.
        let mut half = vec![0.4_f32, 0.1, 0.25, 0.5];
        cpu::tint(&mut half, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], 1.0);
        for v in half.iter().take(3) {
            assert!((v - luma * 0.5).abs() < 1e-6, "half-alpha map: {v}");
        }
        assert_eq!(half[3], 0.5, "alpha untouched");
    }

    #[test]
    fn hue_shift_is_neutral_at_zero_and_preserves_grey_and_luma() {
        let e = instantiate("hue_shift").unwrap();
        assert_eq!(e.float_at("angle", 0.0), Some(0.0));
        // 0° resolves to the identity matrix.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::HueShift {
                m: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                mix: 1.0
            }]
        );
        // Identity is bit-exact identity.
        let mut a = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::hue_shift(&mut a, hue_matrix(0.0), 1.0);
        assert_eq!(a, vec![0.4, 0.5, 0.6, 1.0]);
        // Rotating a neutral grey leaves it grey (rows each ~sum to 1), and any
        // rotation preserves Rec.709 luma to within rounding.
        let m = hue_matrix(90.0);
        let grey = [0.5_f32, 0.5, 0.5];
        let out = [
            m[0] * grey[0] + m[1] * grey[1] + m[2] * grey[2],
            m[3] * grey[0] + m[4] * grey[1] + m[5] * grey[2],
            m[6] * grey[0] + m[7] * grey[1] + m[8] * grey[2],
        ];
        for c in out {
            assert!((c - 0.5).abs() < 1e-3, "grey stays grey: {c}");
        }
        let lin = [0.8_f32, 0.2, 0.5];
        let luma_in = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
        let ro = [
            m[0] * lin[0] + m[1] * lin[1] + m[2] * lin[2],
            m[3] * lin[0] + m[4] * lin[1] + m[5] * lin[2],
            m[6] * lin[0] + m[7] * lin[1] + m[8] * lin[2],
        ];
        let luma_out = 0.2126 * ro[0] + 0.7152 * ro[1] + 0.0722 * ro[2];
        assert!((luma_in - luma_out).abs() < 1e-3, "luma preserved");
    }

    #[test]
    fn contrast_is_neutral_at_100_and_pivots_about_mid_grey() {
        let e = instantiate("contrast").unwrap();
        assert_eq!(e.float_at("contrast", 0.0), Some(100.0));
        // 100 % resolves to a neutral factor of 1.0.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(r, vec![Resolved::Contrast { k: 1.0, mix: 1.0 }]);

        // Neutral (k 1.0) is the bit-exact identity; Mix 0 is too at any k.
        let mut n = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::contrast(&mut n, 1.0, 1.0);
        assert_eq!(n, vec![0.4, 0.5, 0.6, 1.0]);
        let mut m0 = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::contrast(&mut m0, 2.5, 0.0);
        assert_eq!(m0, vec![0.4, 0.5, 0.6, 1.0]);

        // Mid-grey (0.5) is the fixed point of the pivot at any k.
        let mut grey = vec![0.5_f32, 0.5, 0.5, 1.0];
        cpu::contrast(&mut grey, 2.0, 1.0);
        for v in grey.iter().take(3) {
            assert!((v - 0.5).abs() < 1e-6, "mid-grey stays put");
        }

        // Opaque pixel, k 2.0: each channel moves twice as far from 0.5.
        let mut op = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::contrast(&mut op, 2.0, 1.0);
        for (v, want) in op.iter().zip([0.3_f32, 0.5, 0.7, 1.0]) {
            assert!((v - want).abs() < 1e-6, "opaque grade: {v} vs {want}");
        }

        // Half-alpha pixel: the grade runs on the unpremultiplied colour and
        // is re-premultiplied — the premult round trip that a naive grade on
        // premultiplied colour would get wrong. Straight (0.4,0.6,0.5) at
        // alpha 0.5 is stored premultiplied as (0.2,0.3,0.25); k 2.0 grades
        // the straight colour to (0.3,0.7,0.5), re-premultiplied to
        // (0.15,0.35,0.25); alpha is untouched.
        let mut half = vec![0.2_f32, 0.3, 0.25, 0.5];
        cpu::contrast(&mut half, 2.0, 1.0);
        for (v, want) in half.iter().zip([0.15_f32, 0.35, 0.25, 0.5]) {
            assert!((v - want).abs() < 1e-6, "half-alpha grade: {v} vs {want}");
        }

        // Empty pixels stay empty (unpremult reads black, re-premult is zero).
        let mut empty = vec![0.0_f32, 0.0, 0.0, 0.0];
        cpu::contrast(&mut empty, 2.0, 1.0);
        assert_eq!(empty, vec![0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn gamma_is_neutral_at_one_and_curves_per_channel() {
        let e = instantiate("gamma").unwrap();
        assert_eq!(e.float_at("gamma", 0.0), Some(1.0));
        // Default 1.0 resolves to a neutral gamma.
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::Gamma {
                gamma: 1.0,
                mix: 1.0
            }]
        );

        // Neutral (gamma 1.0) is the bit-exact identity; Mix 0 is too at any
        // gamma (a short-circuit, not a reliance on pow(x, 1) == x).
        let mut n = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::gamma(&mut n, 1.0, 1.0);
        assert_eq!(n, vec![0.4, 0.5, 0.6, 1.0]);
        let mut m0 = vec![0.4_f32, 0.5, 0.6, 1.0];
        cpu::gamma(&mut m0, 2.2, 0.0);
        assert_eq!(m0, vec![0.4, 0.5, 0.6, 1.0]);

        // Opaque pixel, gamma 2.0: each channel becomes pow(u, 1/2).
        let mut op = vec![0.25_f32, 0.5, 0.81, 1.0];
        cpu::gamma(&mut op, 2.0, 1.0);
        for (v, want) in op.iter().zip([0.5_f32, 0.5_f32.powf(0.5), 0.9, 1.0]) {
            assert!((v - want).abs() < 1e-6, "opaque curve: {v} vs {want}");
        }

        // 0 and 1 are fixed points of any gamma (pow(0) = 0, pow(1) = 1).
        let mut ends = vec![0.0_f32, 1.0, 0.0, 1.0];
        cpu::gamma(&mut ends, 0.45, 1.0);
        assert!((ends[0] - 0.0).abs() < 1e-6 && (ends[1] - 1.0).abs() < 1e-6);

        // Half-alpha pixel: the curve runs on the unpremultiplied colour and is
        // re-premultiplied — the premult round trip a naive curve on
        // premultiplied colour would get wrong. Straight (0.25,0.81,0.49) at
        // alpha 0.5 is stored premultiplied as (0.125,0.405,0.245); gamma 2.0
        // curves the straight colour to (0.5,0.9,0.7), re-premultiplied to
        // (0.25,0.45,0.35); alpha is untouched.
        let mut half = vec![0.125_f32, 0.405, 0.245, 0.5];
        cpu::gamma(&mut half, 2.0, 1.0);
        for (v, want) in half.iter().zip([0.25_f32, 0.45, 0.35, 0.5]) {
            assert!((v - want).abs() < 1e-6, "half-alpha curve: {v} vs {want}");
        }

        // Negative scene-linear input is clamped to 0 before the pow (pow of a
        // negative base is undefined), so it curves to 0 rather than NaN.
        let mut neg = vec![-0.2_f32, 0.0, 0.0, 1.0];
        cpu::gamma(&mut neg, 2.0, 1.0);
        assert!(
            neg[0].is_finite() && neg[0].abs() < 1e-6,
            "clamped, not NaN: {}",
            neg[0]
        );

        // Empty pixels stay empty (unpremult reads black, re-premult is zero).
        let mut empty = vec![0.0_f32, 0.0, 0.0, 0.0];
        cpu::gamma(&mut empty, 2.0, 1.0);
        assert_eq!(empty, vec![0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn vignette_instantiates_and_resolves() {
        let e = instantiate("vignette").unwrap();
        assert_eq!(e.float_at("amount", 0.0), Some(0.5));
        assert_eq!(e.float_at("radius", 0.0), Some(0.75));
        assert_eq!(e.float_at("softness", 0.0), Some(0.5));
        assert_eq!(e.float_at("roundness", 0.0), Some(1.0));
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
        assert_eq!(
            r,
            vec![Resolved::Vignette {
                amount: 0.5,
                radius: 0.75,
                softness: 0.5,
                roundness: 1.0,
                mix: 1.0,
            }]
        );
    }

    #[test]
    fn cpu_vignette_darkens_the_corners_and_is_neutral_at_zero_amount() {
        let (w, h) = (20u32, 20u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for px in img.chunks_exact_mut(4) {
            px.copy_from_slice(&[1.0, 1.0, 1.0, 1.0]); // opaque white
        }
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;

        // Amount 0 and mix 0 are both the exact identity (the early return
        // and the general blend formula's own 1·x + 0·y identity).
        let mut a0 = img.clone();
        cpu::vignette(&mut a0, w, h, 0.0, 0.75, 0.5, 1.0, 1.0);
        assert_eq!(a0, img);
        let mut m0 = img.clone();
        cpu::vignette(&mut m0, w, h, 0.8, 0.2, 0.1, 1.0, 0.0);
        assert_eq!(m0, img);

        // A tight, hard-edged, fully-strength vignette: the centre stays
        // lit, the corner goes dark, alpha is never touched.
        let mut v = img.clone();
        cpu::vignette(&mut v, w, h, 1.0, 0.2, 0.05, 1.0, 1.0);
        let centre = at(10, 10);
        let corner = at(0, 0);
        assert!(v[centre] > 0.95, "centre stays lit: {}", v[centre]);
        assert!(v[corner] < 0.05, "corner goes dark: {}", v[corner]);
        assert_eq!(v[corner + 3], 1.0, "alpha is never touched");
    }

    #[test]
    fn cpu_vignette_roundness_changes_the_shape_on_a_non_square_frame() {
        let (w, h) = (40u32, 20u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for px in img.chunks_exact_mut(4) {
            px.copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        }
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        // The long edge's midpoint: circular roundness (normalised by the
        // short side, h here) reads its x distance as almost twice as far
        // as elliptical roundness (normalised by w itself) does, so a
        // Radius/Softness pair that fully darkens only one metric's reach
        // tells the two apart.
        let edge_right_mid = at(w - 1, h / 2);

        let mut circular = img.clone();
        cpu::vignette(&mut circular, w, h, 1.0, 0.9, 0.2, 1.0, 1.0);
        let mut elliptical = img.clone();
        cpu::vignette(&mut elliptical, w, h, 1.0, 0.9, 0.2, 0.0, 1.0);

        assert!(
            circular[edge_right_mid] < 1e-5,
            "circular is fully dark this far out: {}",
            circular[edge_right_mid]
        );
        assert!(
            elliptical[edge_right_mid] > 0.0,
            "elliptical has not fully darkened here: {}",
            elliptical[edge_right_mid]
        );
        assert!(
            circular[edge_right_mid] < elliptical[edge_right_mid],
            "circular darkens the long edge harder than elliptical: \
             circular {} elliptical {}",
            circular[edge_right_mid],
            elliptical[edge_right_mid]
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
    fn cpu_matte_key_behaves() {
        let key = [0.0_f32, 0.6, 0.0, 1.0];

        // A pixel exactly the key colour keys out fully (alpha → 0), and its
        // premultiplied colour collapses with it.
        let mut on_key = vec![0.0_f32, 0.6, 0.0, 1.0];
        cpu::matte_key(&mut on_key, key, 0.2, 0.1, 0.0, 1.0);
        assert_eq!(
            on_key,
            vec![0.0, 0.0, 0.0, 0.0],
            "the key colour is removed"
        );

        // A half-alpha key pixel (premultiplied [0,0.3,0,0.5] = straight
        // [0,0.6,0]) keys to nothing too — the metric works on straight colour.
        let mut half = vec![0.0_f32, 0.3, 0.0, 0.5];
        cpu::matte_key(&mut half, key, 0.2, 0.1, 0.0, 1.0);
        assert_eq!(half, vec![0.0, 0.0, 0.0, 0.0], "partial-alpha key removed");

        // A far-from-key colour (red) is kept exactly, with Spill off.
        let red = vec![0.8_f32, 0.0, 0.0, 1.0];
        let mut r = red.clone();
        cpu::matte_key(&mut r, key, 0.2, 0.1, 0.0, 1.0);
        assert_eq!(r, red, "far-from-key pixels are kept exactly");

        // Mix 0 is the exact identity whatever the settings.
        let mut m0 = red.clone();
        cpu::matte_key(&mut m0, key, 0.2, 0.1, 1.0, 0.0);
        assert_eq!(m0, red, "Mix 0 is the identity");

        // Spill: a kept pixel that is grey plus a pure key-hue tint desaturates
        // to that grey at full Spill — its chroma is entirely along the key
        // hue, so removing it lands every channel on the pixel's own luma.
        let mut spill = vec![0.4_f32, 0.6, 0.4, 1.0];
        cpu::matte_key(&mut spill, key, 0.2, 0.1, 1.0, 1.0);
        let luma = 0.4 * cpu::LUMA[0] + 0.6 * cpu::LUMA[1] + 0.4 * cpu::LUMA[2];
        for (c, v) in spill.iter().take(3).enumerate() {
            assert!((v - luma).abs() < 1e-4, "channel {c} despilled to luma");
        }
        assert_eq!(spill[3], 1.0, "a kept pixel keeps its alpha");

        // The soft edge is continuous: a pixel whose chroma distance lands
        // between tol and tol+soft keeps a partial alpha, never a hard 0 or 1
        // — the smoothstep that keeps the effect oracle-safe (§1.6). This
        // pixel is grey + 0.6·(key chroma), so its distance is 0.4·|key chroma|.
        let mut edge = vec![0.2425_f32, 0.6025, 0.2425, 1.0];
        cpu::matte_key(&mut edge, key, 0.2, 0.1, 0.0, 1.0);
        assert!(
            edge[3] > 0.0 && edge[3] < 1.0,
            "soft edge keeps a partial alpha: {}",
            edge[3]
        );
    }

    #[test]
    fn blur_mode_resolves_gaussian_directional_and_legacy() {
        // A fresh instance defaults to Gaussian and resolves exactly as the
        // pre-mode blur did.
        let mut e = instantiate("blur").unwrap();
        assert!(matches!(e.param("mode"), Some(EffectValue::Choice(0))));
        assert_eq!(e.float_at("length", 0.0), Some(10.0));
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::DirBlur {
                length_px: 100.0,
                angle_deg: 0.0,
                edge: 1,
                mix: 1.0
            }]
        );

        // Radial mode reads Centre/Amount/Type instead: Centre resolves to
        // a *fraction* (30/70%, unconverted — resolve_stack has no width/
        // height to scale it by), Amount 8% of 1000 = 80px, Type defaults
        // to Spin.
        for p in &mut e.params {
            match p.id.as_str() {
                "mode" => p.value = EffectValue::Choice(2),
                "centre_x" => p.value = EffectValue::Float(Property::fixed(30.0)),
                "centre_y" => p.value = EffectValue::Float(Property::fixed(70.0)),
                "amount" => p.value = EffectValue::Float(Property::fixed(8.0)),
                _ => {}
            }
        }
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(
            r,
            vec![Resolved::RadialBlur {
                centre_frac: [0.3, 0.7],
                amount_px: 80.0,
                spin: true,
                edge: 1,
                mix: 1.0
            }]
        );

        // The Type choice flips Spin/Zoom.
        for p in &mut e.params {
            if p.id == "radial_type" {
                p.value = EffectValue::Choice(1);
            }
        }
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert!(matches!(r[..], [Resolved::RadialBlur { spin: false, .. }]));

        // A legacy instance (saved before the mode existed) has no mode
        // parameter and still resolves as Gaussian.
        e.params
            .retain(|p| !matches!(p.id.as_str(), "mode" | "length" | "angle"));
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
    fn cpu_radial_blur_spins_and_zooms_from_centre() {
        // A white impulse 4px right of centre in a transparent square frame
        // (odd dimensions: pixel 8's centre is the exact frame centre, as
        // the RGB split radial test already relies on).
        let (w, h) = (17u32, 17u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        let at = |x: u32, y: u32| ((y * w + x) * 4) as usize;
        let imp = at(12, 8);
        img[imp..imp + 4].copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        let centre = [0.5f32, 0.5f32];

        // Amount 0 and mix 0 are both the exact identity, either type (the
        // same zero-tap-offset reasoning as blur_directional's length 0).
        let mut a0 = img.clone();
        cpu::blur_radial(&mut a0, w, h, centre, 0.0, true, 1, 1.0);
        assert_eq!(a0, img);
        let mut a0z = img.clone();
        cpu::blur_radial(&mut a0z, w, h, centre, 0.0, false, 1, 1.0);
        assert_eq!(a0z, img);
        let mut m0 = img.clone();
        cpu::blur_radial(&mut m0, w, h, centre, 30.0, true, 1, 0.0);
        assert_eq!(m0, img);

        // The exact centre pixel is unmoved even at a huge amount, either
        // type — d = 0 there, so every tap collapses to that pixel itself.
        let mut cs = img.clone();
        cpu::blur_radial(&mut cs, w, h, centre, 60.0, true, 1, 1.0);
        assert_eq!(cs[at(8, 8)], 0.0, "centre picks up no energy (spin)");
        let mut cz = img.clone();
        cpu::blur_radial(&mut cz, w, h, centre, 60.0, false, 1, 1.0);
        assert_eq!(cz[at(8, 8)], 0.0, "centre picks up no energy (zoom)");

        // Zoom steps along the ray through the impulse — here, exactly the
        // row — so energy spreads left/right of it on that same row. Row 8
        // is where the exact proof lives: any output pixel there has a
        // purely horizontal d (centre is also on row 8), so its zoom taps
        // never leave the row. Off-row neighbours (12,7)/(12,9) are not
        // proved zero — bilinear's one-pixel blend radius legitimately
        // bleeds a little across a row boundary near the impulse — so the
        // contrast is asserted as "far less", not "none".
        let mut z = img.clone();
        cpu::blur_radial(&mut z, w, h, centre, 20.0, false, 1, 1.0);
        assert!(z[imp] < 1.0, "peak flattens");
        assert!(
            z[at(11, 8)] > 0.0 && z[at(13, 8)] > 0.0,
            "zoom streak spreads along the ray"
        );
        assert!(
            z[at(12, 7)] < z[at(11, 8)] && z[at(12, 9)] < z[at(11, 8)],
            "zoom bleeds far less off the ray than along it"
        );

        // Spin steps along the perpendicular instead — energy spreads
        // above/below the impulse. The exact proof mirrors the zoom one:
        // row 8's own points have a purely *vertical* spin step there, so
        // they never reach column 12 — no bleed along the ray at all.
        let mut s = img.clone();
        cpu::blur_radial(&mut s, w, h, centre, 20.0, true, 1, 1.0);
        assert!(s[imp] < 1.0, "peak flattens");
        assert!(
            s[at(12, 7)] > 0.0 && s[at(12, 9)] > 0.0,
            "spin streak spreads tangentially"
        );
        assert_eq!(s[at(11, 8)], 0.0, "spin: no bleed along the ray");
        assert_eq!(s[at(13, 8)], 0.0, "spin: no bleed along the ray");
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let r = resolve_stack(
            std::slice::from_ref(&e),
            0.0,
            500.0,
            0.5,
            &MarkerContext::NONE,
        );
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
        let r = resolve_stack(&[e], 0.0, 1000.0, 1.0, &MarkerContext::NONE);
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
        let a = resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        let b = resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let later = resolve_stack(
            std::slice::from_ref(&e),
            0.9,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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
        let other = resolve_stack(
            std::slice::from_ref(&reseeded),
            0.4,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
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

    /// A minimal comp + layer pair for marker-context tests: a comp at the
    /// given frame rate carrying `markers`, and an adjustment layer whose
    /// start offset is `offset_s` seconds.
    fn marker_rig(
        fps: (u32, u32),
        markers: Vec<crate::markers::Marker>,
        offset_s: (i64, i64),
    ) -> (Composition, Layer) {
        use crate::model::{LayerKind, LinearColour, Switches, TransformGroup};
        use crate::time::{CompTime, Duration, FrameRate, Rational};
        let secs = |n, d| CompTime(Rational::new(n, d).unwrap());
        let comp = Composition {
            id: uuid::Uuid::now_v7(),
            name: "c".into(),
            width: 1920,
            height: 1080,
            frame_rate: FrameRate::new(fps.0, fps.1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers: Vec::new(),
            markers,
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let layer = Layer {
            id: uuid::Uuid::now_v7(),
            name: "l".into(),
            kind: LayerKind::Adjustment,
            in_point: secs(0, 1),
            out_point: secs(10, 1),
            start_offset: secs(offset_s.0, offset_s.1),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        };
        (comp, layer)
    }

    #[test]
    fn marker_context_builds_layer_local_ordered_beats() {
        use crate::markers::{Marker, MarkerKind};
        use crate::time::{CompTime, Rational};
        let rat = |n, d| Rational::new(n, d).unwrap();
        // Beats out of order, plus a user and a chapter marker to ignore.
        let user = Marker::user(uuid::Uuid::now_v7(), rat(1, 2));
        let chapter = Marker {
            kind: MarkerKind::Chapter,
            time: CompTime(rat(3, 1)),
            ..Marker::user(uuid::Uuid::now_v7(), rat(3, 1))
        };
        let late = Marker::beat(uuid::Uuid::now_v7(), rat(2, 1), 0.9);
        let early = Marker::beat(uuid::Uuid::now_v7(), rat(1, 1), 0.5);
        let (comp, layer) = marker_rig((30, 1), vec![user, late, chapter, early], (1, 4));
        let ctx = MarkerContext::for_layer(&comp, &layer);
        // Beat kind only, layer-local (comp time − start offset), sorted.
        assert_eq!(ctx.beats, vec![0.75, 1.75]);
        assert_eq!(ctx.fps, 30.0);
        // The local translation matches the resolver's own lt subtraction
        // exactly: a beat at comp second 1 and a frame evaluated there land
        // on the identical f64.
        let lt = 1.0 - layer.start_offset.0.to_f64();
        assert_eq!(ctx.beats[0], lt);
        // The obvious no-marker default (§1.4 graceful fallback).
        assert_eq!(MarkerContext::NONE.beats, Vec::<f64>::new());
        assert_eq!(MarkerContext::NONE.fps, 0.0);
        assert_eq!(MarkerContext::default(), MarkerContext::NONE);
    }

    #[test]
    fn marker_context_window_and_nearest() {
        let ctx = MarkerContext {
            beats: vec![1.0, 2.0, 4.0],
            fps: 30.0,
        };
        // The §1.4 temporal-window view: inclusive both ends.
        assert_eq!(ctx.window(1.0, 2.0), &[1.0, 2.0]);
        assert_eq!(ctx.window(1.5, 3.9), &[2.0]);
        assert_eq!(ctx.window(2.5, 3.5), &[] as &[f64]);
        assert_eq!(
            ctx.window(3.0, 1.0),
            &[] as &[f64],
            "inverted span is empty"
        );
        // The nearest-either-side pair: "before" is at/before the frame.
        assert_eq!(ctx.nearest(2.0), (Some(2.0), Some(4.0)));
        assert_eq!(ctx.nearest(2.5), (Some(2.0), Some(4.0)));
        assert_eq!(ctx.nearest(0.5), (None, Some(1.0)));
        assert_eq!(ctx.nearest(9.0), (Some(4.0), None));
        assert_eq!(MarkerContext::NONE.nearest(1.0), (None, None));
    }

    /// A context whose beats and rate use exactly representable values, so
    /// envelope boundary assertions are exact rather than tolerance games.
    fn beat_ctx(beats: &[f64], fps: f64) -> MarkerContext {
        MarkerContext {
            beats: beats.to_vec(),
            fps,
        }
    }

    #[test]
    fn flash_beat_envelope_hard_and_fade_shapes() {
        let ctx = beat_ctx(&[1.0], 4.0);
        // On the beat: full strength, whichever the shape.
        assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, false, 1, 0.0), 1.0);
        assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, true, 1, 0.0), 1.0);
        // One frame in (0.25 s at 4 fps): Hard still full, Fade at the
        // midpoint of a 2-frame duration.
        assert_eq!(flash_beat_envelope(&ctx, 1.25, 2.0, false, 1, 0.0), 1.0);
        assert_eq!(flash_beat_envelope(&ctx, 1.25, 2.0, true, 1, 0.0), 0.5);
        // The span is [0, duration): at exactly two frames both shapes are
        // spent, and well past the duration they stay zero.
        assert_eq!(flash_beat_envelope(&ctx, 1.5, 2.0, false, 1, 0.0), 0.0);
        assert_eq!(flash_beat_envelope(&ctx, 1.5, 2.0, true, 1, 0.0), 0.0);
        assert_eq!(flash_beat_envelope(&ctx, 3.0, 2.0, false, 1, 0.0), 0.0);
        // Before the first trigger there is nothing to decay from.
        assert_eq!(flash_beat_envelope(&ctx, 0.75, 2.0, false, 1, 0.0), 0.0);
        // A fresh beat wins over a spent one (nearest at/before rule).
        let two = beat_ctx(&[1.0, 2.0], 4.0);
        assert_eq!(flash_beat_envelope(&two, 2.0, 2.0, true, 1, 0.0), 1.0);
    }

    #[test]
    fn flash_beat_envelope_phase_shifts_the_triggers() {
        let ctx = beat_ctx(&[1.0], 4.0);
        // Phase +2 frames at 4 fps = +0.5 s: the beat itself no longer
        // fires; the shifted moment does, at full strength.
        assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, false, 1, 2.0), 0.0);
        assert_eq!(flash_beat_envelope(&ctx, 1.5, 2.0, false, 1, 2.0), 1.0);
        // Negative phase leads the beat.
        assert_eq!(flash_beat_envelope(&ctx, 0.5, 2.0, false, 1, -2.0), 1.0);
        assert_eq!(
            flash_beat_envelope(&ctx, 0.75, 2.0, true, 1, -2.0),
            0.5,
            "fade measures from the shifted trigger"
        );
    }

    #[test]
    fn flash_beat_envelope_strobe_skips_to_every_nth() {
        // Beats each second; every 2nd fires indices 0 and 2 (the comp's
        // first beat is index 0).
        let ctx = beat_ctx(&[1.0, 2.0, 3.0, 4.0], 4.0);
        assert_eq!(flash_beat_envelope(&ctx, 1.0, 2.0, false, 2, 0.0), 1.0);
        assert_eq!(
            flash_beat_envelope(&ctx, 2.0, 2.0, false, 2, 0.0),
            0.0,
            "the skipped beat does not fire"
        );
        assert_eq!(flash_beat_envelope(&ctx, 3.0, 2.0, false, 2, 0.0), 1.0);
        // Nth 1 fires them all; a degenerate 0 clamps to 1.
        assert_eq!(flash_beat_envelope(&ctx, 2.0, 2.0, false, 1, 0.0), 1.0);
        assert_eq!(flash_beat_envelope(&ctx, 2.0, 2.0, false, 0, 0.0), 1.0);
    }

    #[test]
    fn flash_beat_envelope_falls_back_gracefully() {
        // No markers, the NONE context, a zero duration and a zero frame
        // rate all yield exactly nothing (§1.4: MUST work with no markers).
        assert_eq!(
            flash_beat_envelope(&MarkerContext::NONE, 1.0, 2.0, false, 1, 0.0),
            0.0
        );
        assert_eq!(
            flash_beat_envelope(&beat_ctx(&[], 30.0), 1.0, 2.0, true, 1, 0.0),
            0.0
        );
        let ctx = beat_ctx(&[1.0], 4.0);
        assert_eq!(flash_beat_envelope(&ctx, 1.0, 0.0, false, 1, 0.0), 0.0);
        assert_eq!(
            flash_beat_envelope(&beat_ctx(&[1.0], 0.0), 1.0, 2.0, false, 1, 0.0),
            0.0
        );
    }

    #[test]
    fn flash_mode_resolves_manual_trigger_strobe_and_legacy() {
        let ctx = beat_ctx(&[1.0, 2.0, 3.0], 4.0);
        // A fresh instance defaults to Manual and resolves exactly as the
        // pre-mode flash did, markers or none.
        let mut e = instantiate("flash").unwrap();
        assert!(matches!(e.param("mode"), Some(EffectValue::Choice(0))));
        assert_eq!(e.float_at("duration", 0.0), Some(2.0));
        assert!(matches!(e.param("shape"), Some(EffectValue::Choice(0))));
        assert_eq!(e.float_at("every_nth", 0.0), Some(1.0));
        assert_eq!(e.float_at("phase", 0.0), Some(0.0));
        let dark = Resolved::Flash {
            strength: 0.0,
            colour: [1.0; 4],
            mix: 1.0,
        };
        let r = resolve_stack(std::slice::from_ref(&e), 1.0, 1000.0, 1.0, &ctx);
        assert_eq!(r, vec![dark], "Manual ignores markers entirely");

        // Trigger mode lights on the beat and is spent past Duration.
        for p in &mut e.params {
            if p.id == "mode" {
                p.value = EffectValue::Choice(1);
            }
        }
        let lit = Resolved::Flash {
            strength: 1.0,
            colour: [1.0; 4],
            mix: 1.0,
        };
        let r = resolve_stack(std::slice::from_ref(&e), 1.0, 1000.0, 1.0, &ctx);
        assert_eq!(r, vec![lit]);
        let r = resolve_stack(std::slice::from_ref(&e), 1.75, 1000.0, 1.0, &ctx);
        assert_eq!(r, vec![dark], "3 frames past a 2-frame flash");
        // And with no markers at all it resolves dark — never an error
        // (§1.4 graceful fallback).
        let r = resolve_stack(
            std::slice::from_ref(&e),
            1.0,
            1000.0,
            1.0,
            &MarkerContext::NONE,
        );
        assert_eq!(r, vec![dark]);

        // Strobe every 2nd beat: beat index 1 (2 s) does not fire, index 2
        // (3 s) does.
        for p in &mut e.params {
            match p.id.as_str() {
                "mode" => p.value = EffectValue::Choice(2),
                "every_nth" => p.value = EffectValue::Float(Property::fixed(2.0)),
                _ => {}
            }
        }
        let r = resolve_stack(std::slice::from_ref(&e), 2.0, 1000.0, 1.0, &ctx);
        assert_eq!(r, vec![dark]);
        let r = resolve_stack(std::slice::from_ref(&e), 3.0, 1000.0, 1.0, &ctx);
        assert_eq!(r, vec![lit]);

        // A legacy instance (saved before the marker modes existed) has no
        // mode parameter and still resolves Manual: a static Trigger of
        // 0.4 holds a 0.4 flash whatever the markers say.
        let mut legacy = instantiate("flash").unwrap();
        legacy.params.retain(|p| {
            !matches!(
                p.id.as_str(),
                "mode" | "duration" | "shape" | "every_nth" | "phase"
            )
        });
        for p in &mut legacy.params {
            if p.id == "trigger" {
                p.value = EffectValue::Float(Property::fixed(0.4));
            }
        }
        let r = resolve_stack(std::slice::from_ref(&legacy), 1.0, 1000.0, 1.0, &ctx);
        assert_eq!(
            r,
            vec![Resolved::Flash {
                strength: 0.4,
                colour: [1.0; 4],
                mix: 1.0
            }]
        );
    }

    #[test]
    fn marker_window_reports_what_the_envelope_reads() {
        let ctx = beat_ctx(&[1.0, 2.0, 3.0], 4.0);
        // Manual mode — and any effect without marker input — has no
        // window, which is what keeps its frame keys time-free.
        let mut e = instantiate("flash").unwrap();
        assert_eq!(marker_window(&e, 1.5, &ctx), None);
        let blur = instantiate("blur").unwrap();
        assert_eq!(marker_window(&blur, 1.5, &ctx), None);

        // Trigger mode: the nearest trigger either side of the frame.
        for p in &mut e.params {
            if p.id == "mode" {
                p.value = EffectValue::Choice(1);
            }
        }
        assert_eq!(
            marker_window(&e, 1.5, &ctx),
            Some(MarkerWindow {
                fps: 4.0,
                before: Some(1.0),
                after: Some(2.0),
            })
        );
        assert_eq!(
            marker_window(&e, 0.5, &ctx),
            Some(MarkerWindow {
                fps: 4.0,
                before: None,
                after: Some(1.0),
            })
        );

        // Strobe filters first: with every 2nd beat, the frame after beat
        // index 1 still sees indices 0 and 2 as its neighbours — the
        // window is the triggers the envelope actually consumes.
        for p in &mut e.params {
            match p.id.as_str() {
                "mode" => p.value = EffectValue::Choice(2),
                "every_nth" => p.value = EffectValue::Float(Property::fixed(2.0)),
                _ => {}
            }
        }
        assert_eq!(
            marker_window(&e, 2.5, &ctx),
            Some(MarkerWindow {
                fps: 4.0,
                before: Some(1.0),
                after: Some(3.0),
            })
        );
    }

    #[test]
    fn block_hash01_is_deterministic_bounded_and_varies() {
        let a = block_hash01(7, 0, 3, 5, 2);
        let b = block_hash01(7, 0, 3, 5, 2);
        assert_eq!(a, b, "same inputs, same hash");
        assert!((0.0..1.0).contains(&a), "hash lands in [0, 1)");

        // Changing any one input moves the hash (checked, not proved
        // statistically — a collision is possible in principle but
        // vanishingly unlikely for a well-mixed hash, and none of these
        // particular inputs happen to collide).
        assert_ne!(a, block_hash01(8, 0, 3, 5, 2), "seed matters");
        assert_ne!(a, block_hash01(7, 1, 3, 5, 2), "channel matters");
        assert_ne!(a, block_hash01(7, 0, 4, 5, 2), "block x matters");
        assert_ne!(a, block_hash01(7, 0, 3, 6, 2), "block y matters");
        assert_ne!(a, block_hash01(7, 0, 3, 5, 3), "tick matters");
    }

    #[test]
    fn block_glitch_instantiates_and_resolves() {
        let e = instantiate("block_glitch").unwrap();
        assert_eq!(e.float_at("intensity", 0.0), Some(0.35));
        assert!(matches!(e.param("seed"), Some(EffectValue::Seed(_))));
        assert_eq!(e.float_at("block_size", 0.0), Some(24.0));
        assert_eq!(e.float_at("block_jitter", 0.0), Some(25.0));
        assert_eq!(e.float_at("block_amount", 0.0), Some(3.0));
        assert_eq!(e.float_at("channel_offset", 0.0), Some(1.0));
        assert_eq!(e.float_at("slice_repeat", 0.0), Some(20.0));

        // Resolving is deterministic: the same instance at the same time
        // yields the identical result, twice — and the px_scale factor
        // (0.5 here) reaches the px@comp parameters exactly like Transform
        // and Shake's do.
        let a = resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            1000.0,
            0.5,
            &MarkerContext::NONE,
        );
        let b = resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            1000.0,
            0.5,
            &MarkerContext::NONE,
        );
        assert_eq!(a, b);
        let Resolved::BlockGlitch {
            intensity,
            tick,
            block_size_px,
            jitter_frac,
            amount_px,
            chan_px,
            slice_frac,
            mix,
            ..
        } = a[0]
        else {
            panic!("expected a BlockGlitch");
        };
        assert_eq!(intensity, 0.35);
        assert_eq!(tick, 3); // floor(0.4 * GLITCH_TICK_HZ 8) = 3
        assert_eq!(block_size_px, 12.0); // 24 px@comp * px_scale 0.5
        assert_eq!(jitter_frac, 0.25);
        assert_eq!(amount_px, 30.0); // 3% of a 1000px diagonal
        assert_eq!(chan_px, 10.0); // 1% of a 1000px diagonal
        assert_eq!(slice_frac, 0.20);
        assert_eq!(mix, 1.0);

        // A different frame ticks differently (the per-block hash itself
        // only runs inside cpu::block_glitch/the kernel, not here).
        let later = resolve_stack(
            std::slice::from_ref(&e),
            0.9,
            1000.0,
            0.5,
            &MarkerContext::NONE,
        );
        assert_ne!(a, later, "the tick moves between frames");
    }

    #[test]
    fn scanlines_instantiates_and_resolves() {
        let e = instantiate("scanlines").unwrap();
        assert_eq!(e.float_at("intensity", 0.0), Some(0.35));
        assert_eq!(e.float_at("scanline_period", 0.0), Some(3.0));
        assert_eq!(e.float_at("scanline_darkness", 0.0), Some(40.0));
        assert_eq!(e.float_at("scanline_roll", 0.0), Some(0.0));
        assert!(matches!(
            e.param("scanline_interlace"),
            Some(EffectValue::Bool(false))
        ));

        let a = resolve_stack(
            std::slice::from_ref(&e),
            0.4,
            1000.0,
            0.5,
            &MarkerContext::NONE,
        );
        assert_eq!(
            a,
            vec![Resolved::Scanlines {
                intensity: 0.35,
                period_px: 1.5, // 3 px@comp * px_scale 0.5
                darkness: 0.40,
                roll_px: 0.0, // roll speed 0
                interlace: false,
                mix: 1.0,
            }]
        );
    }

    #[test]
    fn cpu_block_glitch_is_identity_at_zero_intensity() {
        let (w, h) = (17u32, 9u32);
        let img = transform_card(w, h);

        // Intensity 0: every hashed quantity collapses — the early return
        // skips the blend entirely, so this holds for any Mix, unlike the
        // blur family's tap-sum coincidence.
        let mut a = img.clone();
        cpu::block_glitch(&mut a, w, h, 0.0, 7, 3, 6.0, 0.5, 5.0, 2.0, 0.5, 0.4);
        assert_eq!(a, img, "intensity 0 is the exact identity");
    }

    #[test]
    fn cpu_scanlines_is_identity_at_zero_intensity() {
        let (w, h) = (17u32, 9u32);
        let img = transform_card(w, h);
        let mut a = img.clone();
        cpu::scanlines(&mut a, w, h, 0.0, 3.0, 0.6, 1.0, true, 0.4);
        assert_eq!(a, img, "intensity 0 is the exact identity");
    }

    #[test]
    fn cpu_block_glitch_params_each_move_the_result() {
        // Every hashed quantity at zero is still an exact identity even
        // though block displacement runs (not the early return) — the
        // "scale by zero" branches must themselves be exact.
        let (w, h) = (40u32, 40u32);
        let img = transform_card(w, h);
        let (seed, tick) = (42u32, 5i32);
        let run = |amount: f32, jitter: f32, chan: f32, slice: f32| {
            let mut out = img.clone();
            cpu::block_glitch(
                &mut out, w, h, 1.0, seed, tick, 8.0, jitter, amount, chan, slice, 1.0,
            );
            out
        };
        let zero = run(0.0, 0.0, 0.0, 0.0);
        assert_eq!(
            zero, img,
            "every hashed quantity at zero is the identity too"
        );
        assert_ne!(
            run(6.0, 0.0, 0.0, 0.0),
            zero,
            "displacement amount moves pixels"
        );
        assert_ne!(run(0.0, 0.5, 0.0, 0.0), zero, "grid jitter moves pixels");
        assert_ne!(
            run(0.0, 0.0, 4.0, 0.0),
            zero,
            "channel offset splits colour"
        );
        assert_ne!(run(0.0, 0.0, 0.0, 1.0), zero, "slice repeat folds rows");
    }

    #[test]
    fn cpu_scanlines_darken_a_periodic_band() {
        let (w, h) = (4u32, 12u32);
        let mut img = vec![0.0f32; (w * h * 4) as usize];
        for px in img.chunks_exact_mut(4) {
            px.copy_from_slice(&[1.0, 1.0, 1.0, 1.0]);
        }
        let red_at = |img: &[f32], y: u32| img[(y * w * 4) as usize];

        // Period 4px, no roll, no interlace: rows 0-1 of every period are
        // bright, rows 2-3 dark — the same shape every period.
        let mut out = img.clone();
        cpu::scanlines(&mut out, w, h, 1.0, 4.0, 0.5, 0.0, false, 1.0);
        for y in 0..h {
            let expect = if (y % 4) < 2 { 1.0 } else { 0.5 };
            assert_eq!(red_at(&out, y), expect, "row {y}");
        }

        // Interlace flips which half darkens on odd periods only: period 1
        // (rows 4-7) is dark-then-bright instead of bright-then-dark;
        // period 0 and period 2 (even) are unaffected.
        let mut inter = img.clone();
        cpu::scanlines(&mut inter, w, h, 1.0, 4.0, 0.5, 0.0, true, 1.0);
        assert_eq!(red_at(&inter, 0), 1.0, "period 0 unaffected");
        assert_eq!(red_at(&inter, 2), 0.5, "period 0 unaffected");
        assert_eq!(red_at(&inter, 4), 0.5, "period 1 flips: dark first");
        assert_eq!(red_at(&inter, 6), 1.0, "period 1 flips: bright second");
        assert_eq!(red_at(&inter, 8), 1.0, "period 2 (even) unflipped again");
        assert_eq!(red_at(&inter, 10), 0.5, "period 2 (even) unflipped again");
    }
}
