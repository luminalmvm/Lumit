use super::*;
use crate::anim::Property;
use crate::model::{
    EffectInstance, EffectKey, EffectNamespace, EffectParam, EffectValue, FileParam,
};

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
            // Seed sits second-last, immediately before Mix (owner convention
            // for seeded effects: the "which roll of the dice" dial lives at the
            // bottom of the stack of look controls).
            ParamSchema {
                id: "seed",
                label: "Seed",
                kind: ParamKind::Seed,
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
    // Posterize time (docs/08 §3.25, docs/impl/temporal-rerender.md): a
    // temporal resample that holds its input on a coarser frame-rate grid, for
    // the choppy stop-motion look. NOT a per-pixel op — it changes *what time*
    // the layers it covers render at, so it is detected and executed at the
    // frame-orchestration layer (the adjustment re-render seam in `draws`/`gpu`
    // and export's `render_comp_linear`), never in `run_ops`; `resolve_stack`
    // deliberately has no arm for it, so it resolves to nothing. Category
    // Temporal, cheap (one render at the held time — often the SAME held time
    // across many frames). Scope chooses adjustment behaviour (Everything below,
    // the owner's global pass) or a per-layer time hold (This layer's effects).
    EffectSchema {
        match_name: "posterize_time",
        label: "Posterize time",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Cheap,
            // It re-renders the composite below at a held time; no per-pixel ROI
            // applies. Full-frame is the safe static declaration.
            roi: Roi::FullFrame,
            // The held frame is the frame the decode already produced (footage
            // is held, docs/impl/temporal-rerender.md §2), so no neighbour
            // window is requested — the decode planner is never re-entered.
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "rate",
                label: "Frame rate",
                // The posterised grid in fps: the animation updates only this
                // many times a second. Default 12 (the classic on-twos look).
                kind: ParamKind::Float {
                    default: 12.0,
                    slider: (1.0, 60.0),
                    hard: (Some(0.01), None),
                },
            },
            ParamSchema {
                id: "phase",
                label: "Phase",
                // Comp seconds: shifts where the steps land, so the hold can be
                // aligned to a beat. 0 snaps to the comp's own zero.
                kind: ParamKind::Float {
                    default: 0.0,
                    slider: (-1.0, 1.0),
                    hard: (None, None),
                },
            },
            ParamSchema {
                id: "scope",
                label: "Scope",
                // Everything below = adjustment behaviour (the whole composite
                // beneath holds); This layer's effects = only the layer's own
                // source and stack hold. Default adjustment, the owner's global
                // pass.
                kind: ParamKind::Choice {
                    options: &["Everything below", "This layer's effects"],
                    default: 0,
                },
            },
        ],
    },
    // Accumulation motion blur (docs/08 §3.26, docs/impl/temporal-rerender.md):
    // the expensive, correct motion blur — it renders the WHOLE scene below it
    // several times at in-between moments and averages the finished frames, so
    // footage motion, animated effects, depth passes and everything else are all
    // correct per sample (no blurred-depth artefact). NOT a per-pixel op: like
    // Posterize time it changes *what time the layers below it render at*, so it
    // is detected and executed at the frame-orchestration layer (the adjustment
    // re-render seam in `draws`/`gpu` and export's `render_comp_linear`), never in
    // `run_ops`; `resolve_stack` deliberately has no arm for it, so it resolves to
    // nothing. An **adjustment** effect (docs/08 §1.5): it processes everything
    // below, so "apply to all layers" is just the effect on a full-frame
    // adjustment layer. Category Temporal, cost Heavy (≈ N× a full comp render).
    // The sub-frame sample times reuse `MotionBlur::sample_offsets` (the same
    // centred shutter maths per-layer motion blur uses), so `τ_k = t + off_k·dt`;
    // the N finished below-composites are averaged by the hardware
    // additive-at-1/N pass (`Compositor::accumulate`). Mix blends the averaged
    // result against the frame-time composite. Boundaries as Posterize (K-125):
    // temporal effects inside the sampled below-stack hold to stills.
    EffectSchema {
        match_name: "accumulation_mb",
        // The user-facing motion blur (docs/08 §3.26): the accumulation kind is
        // the correct, whole-scene one, so it takes the plain name. The
        // optical-flow effect (match_name "motion_blur") is "Fast motion blur".
        label: "Motion blur",
        version: 1,
        category: FxCategory::Temporal,
        traits: EffectTraits {
            cost: CostClass::Heavy,
            roi: Roi::FullFrame,
            // The below-stack is re-rendered at each sub-frame time from the SAME
            // held decode (footage is held, docs/impl/temporal-rerender.md §2), so
            // no neighbour window is requested — the decode planner is never
            // re-entered.
            temporal: &[0],
            premultiplied: true,
            seeded: false,
            beat_input: false,
        },
        params: &[
            ParamSchema {
                id: "samples",
                label: "Samples",
                // Sub-frame renders of the scene below across the open shutter
                // (≥ 2 to blur). The schema has no integer kind, so this is a
                // Float row (as Echo's Echoes and flow Motion blur's Samples are);
                // the detector rounds and clamps. Heavy — each sample is a full
                // comp re-render — so a tasteful default of 8.
                kind: ParamKind::Float {
                    default: 8.0,
                    slider: (2.0, 32.0),
                    hard: (Some(2.0), Some(64.0)),
                },
            },
            ParamSchema {
                id: "shutter_angle",
                label: "Shutter angle",
                // Degrees: the fraction of the frame interval the shutter is open
                // is shutter ÷ 360, so the samples span that much of the motion.
                // 180° (half a frame) is the film-standard look.
                kind: ParamKind::Float {
                    default: 180.0,
                    slider: (0.0, 720.0),
                    hard: (Some(0.0), Some(720.0)),
                },
            },
            ParamSchema {
                id: "shutter_phase",
                label: "Shutter phase",
                // Degrees: where the open interval sits relative to the frame
                // time. -90 centres the samples on the frame (pairing with a 180
                // angle to open a quarter-frame either side), the AE default.
                kind: ParamKind::Float {
                    default: -90.0,
                    slider: (-360.0, 360.0),
                    hard: (Some(-720.0), Some(720.0)),
                },
            },
            ParamSchema {
                id: "force_all",
                label: "Force on all layers",
                // Force per-layer motion blur (K-120) on every layer during the
                // sub-frame sample renders — the shutter above stands in for the
                // comp master and each layer's own switch, without mutating the
                // comp. So one effect blurs every moving layer without toggling
                // each one; each accumulation sample is itself transform-smeared,
                // smoothing the result at lower sample counts. Off by default.
                kind: ParamKind::Bool { default: false },
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
        // The optical-flow, footage-internal blur (docs/08 §3.2): "Fast" because
        // it is a single-pass per-pixel smear, distinct from the whole-scene,
        // re-rendering "Motion blur" (accumulation, §3.26).
        label: "Fast motion blur",
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
            ParamSchema {
                id: "view",
                label: "View",
                // Diagnostic outputs (FX-19): the blurred picture, the flow
                // vectors colour-coded (red +x, green +y), or the confidence as
                // greyscale (white = trusted, black = suspect — where the streak
                // fades out). Rendered by default.
                kind: ParamKind::Choice {
                    options: &["Rendered", "Motion vectors", "Confidence"],
                    default: 0,
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
