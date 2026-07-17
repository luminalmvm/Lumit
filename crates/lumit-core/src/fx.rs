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

use crate::anim::Property;
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
        hard: (f64, f64),
    },
    Choice {
        options: &'static [&'static str],
        default: u32,
    },
    Bool {
        default: bool,
    },
}

/// One built-in effect's full declaration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectSchema {
    /// Stable match name (participates in the cache key with `version`).
    pub match_name: &'static str,
    pub label: &'static str,
    pub version: u32,
    pub traits: EffectTraits,
    pub params: &'static [ParamSchema],
}

/// Edge policy shared by the blur family (docs/08 §3.8).
pub const EDGE_OPTIONS: &[&str] = &["Transparent", "Repeat", "Mirror"];

/// The catalogue. Grows one entry per landed effect; the schema is the single
/// source of truth the UI menu, instantiation and resolution all read.
pub const BUILTINS: &[EffectSchema] = &[EffectSchema {
    match_name: "blur",
    label: "Blur",
    version: 1,
    traits: EffectTraits {
        cost: CostClass::Moderate,
        roi: Roi::PaddedPctDiag(25.0),
        temporal: &[0],
        premultiplied: true,
        seeded: false,
        beat_input: false,
    },
    params: &[
        ParamSchema {
            id: "radius",
            label: "Radius",
            // % of the comp diagonal (§2.3), so half-res preview matches.
            // Default per §1.2's "drop it on and it already looks right".
            kind: ParamKind::Float {
                default: 1.5,
                slider: (0.0, 25.0),
                hard: (0.0, 100.0),
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
        ParamSchema {
            id: "mix",
            label: "Mix",
            // The host-uniform Mix (docs/08 §1.5), in per cent.
            kind: ParamKind::Float {
                default: 100.0,
                slider: (0.0, 100.0),
                hard: (0.0, 100.0),
            },
        },
    ],
}];

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
                let radius_pct = e.float_at("radius", lt)? as f32;
                let edge = match e.param("edge") {
                    Some(EffectValue::Choice(c)) => (*c).min(2),
                    _ => 1,
                };
                let mix = (e.float_at("mix", lt).unwrap_or(100.0) as f32 / 100.0).clamp(0.0, 1.0);
                Some(Resolved::Blur {
                    radius_px: (radius_pct / 100.0 * diag_px).max(0.0),
                    edge,
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
}
