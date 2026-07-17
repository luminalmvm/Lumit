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
    // Operates premultiplied. The §3.6 Centre/Falloff/channel-blur extras
    // land later; radial mode grows the offset from the frame centre.
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
    // Grade (docs/08 §3.10), minimal v1: the lift/gamma/gain stage plus
    // saturation, per channel, in linear, on unpremultiplied colour (§2.2).
    // Exposure/white balance, vibrance, curves, the vignette and the preset
    // browser follow as the remaining §3.10 stages. Defaults are neutral —
    // a grade's "tasteful default" is a preset choice, which is what the
    // §3.10 browser is for.
    EffectSchema {
        match_name: "grade",
        label: "Grade",
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
];

/// Look a schema up by its match name.
pub fn schema(match_name: &str) -> Option<&'static EffectSchema> {
    BUILTINS.iter().find(|s| s.match_name == match_name)
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
    Flash {
        /// The evaluated envelope × intensity, 0..1 (0 = no flash).
        strength: f32,
        /// Scene-linear RGBA flash colour (alpha unused: the flash respects
        /// the layer's own footprint).
        colour: [f32; 4],
        /// 0..1.
        mix: f32,
    },
    Grade {
        /// Added per channel after gain (raises or crushes the blacks).
        lift: [f32; 3],
        /// Per-channel mid-tone exponent's base; 1 is neutral, > 0.
        gamma: [f32; 3],
        /// Per-channel linear multiplier; 1 is neutral.
        gain: [f32; 3],
        /// Factor about Rec. 709 luma: 0 = greyscale, 1 = neutral, 2 = max.
        saturation: f32,
        /// 0..1.
        mix: f32,
    },
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

/// The linear-mode channel offset vector for an RGB split: `amount_px`
/// along `angle_deg`. Shared by the CPU reference and the GPU op
/// construction so both paths carry the same host-computed sines (WGSL's
/// `cos`/`sin` are not correctly rounded, so the kernel never computes its
/// own).
pub fn rgb_split_offset(amount_px: f32, angle_deg: f32) -> (f32, f32) {
    let rad = angle_deg.to_radians();
    (amount_px * rad.cos(), amount_px * rad.sin())
}

/// Resolve a layer's live stack at layer time `lt` for a raster whose
/// diagonal is `diag_px` pixels. Placeholders, unknown names and bypassed
/// effects resolve to nothing (they render as identity, docs/03 §8).
pub fn resolve_stack(effects: &[EffectInstance], lt: f64, diag_px: f32) -> Vec<Resolved> {
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
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::RgbSplit {
                    amount_px: (amount_pct / 100.0 * diag_px).max(0.0),
                    angle_deg,
                    radial,
                    mix,
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
            "grade" => {
                let rgb = |id: &str, neutral: f64| -> [f32; 3] {
                    let c = e.colour_at(id, lt).unwrap_or([neutral; 4]);
                    [c[0] as f32, c[1] as f32, c[2] as f32]
                };
                let saturation =
                    (e.float_at("saturation", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 2.0);
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Grade {
                    lift: rgb("lift", 0.0),
                    gamma: rgb("gamma", 1.0).map(|g| g.max(0.01)),
                    gain: rgb("gain", 1.0),
                    saturation,
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
            Resolved::Flash {
                strength,
                colour,
                mix,
            } => flash(rgba, *strength, *colour, *mix),
            Resolved::Grade {
                lift,
                gamma,
                gain,
                saturation,
                mix,
            } => grade(rgba, *lift, *gamma, *gain, *saturation, *mix),
        }
    }

    /// Grade (docs/08 §3.10, minimal v1): per-channel gain → lift → gamma,
    /// then saturation about Rec. 709 luma, in linear light on
    /// unpremultiplied colour (§2.2), re-premultiplied on the way out.
    /// Neutral gamma and saturation short-circuit so a neutral grade is the
    /// identity rather than a round trip through `powf`. Negative light
    /// clamps at zero (that is what a crushing lift means); highlights are
    /// never clipped (§2.1).
    pub fn grade(
        rgba: &mut [f32],
        lift: [f32; 3],
        gamma: [f32; 3],
        gain: [f32; 3],
        saturation: f32,
        mix: f32,
    ) {
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
            if saturation != 1.0 {
                let luma = v[0] * LUMA[0] + v[1] * LUMA[1] + v[2] * LUMA[2];
                for x in &mut v {
                    *x = (luma + (*x - luma) * saturation).max(0.0);
                }
            }
            for c in 0..3 {
                let graded = v[c] * a;
                px[c] = px[c] * (1.0 - mix) + graded * mix;
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
        let r = resolve_stack(&[e.clone()], 0.0, 1000.0);
        assert_eq!(
            r,
            vec![Resolved::Blur {
                radius_px: 15.0,
                edge: 1,
                mix: 1.0
            }]
        );
        e.enabled = false;
        assert!(resolve_stack(&[e.clone()], 0.0, 1000.0).is_empty());
        e.enabled = true;
        e.effect.namespace = EffectNamespace::Placeholder;
        assert!(
            resolve_stack(&[e], 0.0, 1000.0).is_empty(),
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
        let r = resolve_stack(&[e], 0.0, 1000.0);
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
        let r = resolve_stack(&[e], 0.0, 1000.0);
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
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0);
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
    fn grade_instantiates_and_resolves_neutral() {
        let e = instantiate("grade").unwrap();
        assert_eq!(e.colour_at("lift", 0.0), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(e.colour_at("gamma", 0.0), Some([1.0; 4]));
        assert_eq!(e.colour_at("gain", 0.0), Some([1.0; 4]));
        assert_eq!(e.float_at("saturation", 0.0), Some(100.0));
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0);
        assert_eq!(
            r,
            vec![Resolved::Grade {
                lift: [0.0; 3],
                gamma: [1.0; 3],
                gain: [1.0; 3],
                saturation: 1.0,
                mix: 1.0
            }]
        );
    }

    #[test]
    fn cpu_grade_stages_behave() {
        let neutral = ([0.0f32; 3], [1.0f32; 3], [1.0f32; 3]);
        // One opaque mid-grey-ish pixel, one half-alpha, one HDR, one empty.
        let img = vec![
            0.25, 0.5, 0.1, 1.0, //
            0.1, 0.2, 0.05, 0.5, //
            4.0, 2.0, 1.0, 1.0, //
            0.0, 0.0, 0.0, 0.0,
        ];

        // A neutral grade is the identity on opaque pixels and within one
        // rounding step elsewhere (unpremultiply round-trips).
        let mut n = img.clone();
        cpu::grade(&mut n, neutral.0, neutral.1, neutral.2, 1.0, 1.0);
        for (a, b) in n.iter().zip(&img) {
            assert!((a - b).abs() < 1e-6, "{a} vs {b}");
        }

        // Mix 0 is the exact identity whatever the grade.
        let mut m0 = img.clone();
        cpu::grade(&mut m0, [0.5; 3], [2.0; 3], [3.0; 3], 0.0, 0.0);
        assert_eq!(m0, img);

        // Gain doubles linear values; HDR stays unclipped (§2.1).
        let mut g = img.clone();
        cpu::grade(&mut g, [0.0; 3], [1.0; 3], [2.0; 3], 1.0, 1.0);
        assert_eq!(g[0], 0.5);
        assert_eq!(g[8], 8.0, "highlights never clip");

        // Lift raises blacks (empty alpha stays empty: premultiplied zero).
        let mut l = img.clone();
        cpu::grade(&mut l, [0.1; 3], [1.0; 3], [1.0; 3], 1.0, 1.0);
        assert!((l[2] - 0.2).abs() < 1e-6, "0.1 blue lifted by 0.1");
        assert_eq!(&l[12..16], &[0.0; 4], "empty pixels stay empty");

        // Gamma 2 is a square root in linear: 0.25 → 0.5.
        let mut ga = img.clone();
        cpu::grade(&mut ga, [0.0; 3], [2.0; 3], [1.0; 3], 1.0, 1.0);
        assert!((ga[0] - 0.5).abs() < 1e-6);

        // Saturation 0 collapses to Rec. 709 luma (greyscale).
        let mut s = img.clone();
        cpu::grade(&mut s, neutral.0, neutral.1, neutral.2, 0.0, 1.0);
        let luma = 0.25 * cpu::LUMA[0] + 0.5 * cpu::LUMA[1] + 0.1 * cpu::LUMA[2];
        for (c, v) in s.iter().take(3).enumerate() {
            assert!((v - luma).abs() < 1e-6, "channel {c} at luma");
        }
        // Alpha is untouched by any of it.
        for v in [&n, &m0, &g, &l, &ga, &s] {
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
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0);
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
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0);
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
        let r = resolve_stack(std::slice::from_ref(&e), 0.0, 1000.0);
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
}
