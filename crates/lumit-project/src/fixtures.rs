//! Deterministic fixture documents for performance and golden testing
//! (docs/13-PERFORMANCE-RULES.md §2.1, §7.3).
//!
//! In plain terms: the performance budgets need a *known, repeatable* heavy
//! project to measure against — the same document every run, on every machine —
//! so a regression shows up as a real slow-down, not as noise. This builder
//! constructs that document from plain counts, using fixed identifiers so two
//! builds are byte-for-byte identical.
//!
//! Not runtime code: it is called by tests and the (future) perf harness only.
//! Its constructors take compile-time-valid constants (a 60/1 frame rate, whole
//! keyframe times), so the `unwrap`s below cannot fire; the module opts out of
//! the workspace's no-unwrap lint on that basis, exactly as the test modules do.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use lumit_core::anim::{Animation, Keyframe, SideInterp};
use lumit_core::model::{
    AutoFolders, Composition, Document, FootageItem, Layer, LayerKind, LinearColour, MediaRef,
    ProjectItem, Switches, TransformGroup,
};
use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
use uuid::Uuid;

/// The counts that define a stress document (docs/13 §2.1).
#[derive(Debug, Clone, Copy)]
pub struct StressParams {
    /// Number of compositions.
    pub comps: usize,
    /// Total layers across all comps.
    pub layers_total: usize,
    /// Layers concentrated in the single biggest comp.
    pub biggest_comp_layers: usize,
    /// Total keyframes across all layers.
    pub keyframes_total: usize,
    /// Number of footage items in the Project panel.
    pub footage_items: usize,
}

impl StressParams {
    /// The canonical stress document (docs/13 §2.1): 200 comps, 5,000 layers
    /// (one comp holds 1,000), 250,000 keyframes, 2,000 footage items.
    pub const REFERENCE: StressParams = StressParams {
        comps: 200,
        layers_total: 5_000,
        biggest_comp_layers: 1_000,
        keyframes_total: 250_000,
        footage_items: 2_000,
    };

    /// A tiny variant for fast unit tests — same shape, trivial counts.
    pub const TINY: StressParams = StressParams {
        comps: 3,
        layers_total: 12,
        biggest_comp_layers: 5,
        keyframes_total: 40,
        footage_items: 6,
    };
}

/// A deterministic UUID from a `(kind, index)` pair — distinct kinds keep
/// footage, comps and layers from ever colliding.
fn uid(kind: u128, index: usize) -> Uuid {
    Uuid::from_u128((kind << 96) | index as u128)
}

/// Split `total` into `buckets` counts summing to exactly `total`, as evenly as
/// possible (the first `total % buckets` buckets get one extra).
fn spread(total: usize, buckets: usize) -> Vec<usize> {
    if buckets == 0 {
        return Vec::new();
    }
    let base = total / buckets;
    let extra = total % buckets;
    (0..buckets)
        .map(|i| base + usize::from(i < extra))
        .collect()
}

/// Layers per comp: the biggest comp holds `biggest`, the rest share the
/// remainder evenly (docs/13 §2.1: "one comp holding 1,000").
fn layer_distribution(comps: usize, layers_total: usize, biggest: usize) -> Vec<usize> {
    if comps == 0 {
        return Vec::new();
    }
    if comps == 1 {
        return vec![layers_total];
    }
    let biggest = biggest.min(layers_total);
    let mut out = vec![0usize; comps];
    out[0] = biggest;
    for (i, n) in spread(layers_total - biggest, comps - 1)
        .into_iter()
        .enumerate()
    {
        out[1 + i] = n;
    }
    out
}

/// `k` keyframes on a property, at successive frames with cycling values —
/// enough to exercise keyframe-heavy paths; the shape, not the motion, matters.
fn keyframes(k: usize) -> Vec<Keyframe> {
    (0..k)
        .map(|i| Keyframe {
            time: Rational::new(i as i64 + 1, 30).unwrap_or(Rational::ONE),
            value: (i % 100) as f64,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        })
        .collect()
}

/// Build the deterministic stress document for `p` (docs/13 §2.1). The same `p`
/// always yields a byte-identical document — fixed ids, no clock, no randomness.
#[must_use]
pub fn stress_document(p: &StressParams) -> Document {
    let mut items = Vec::with_capacity(p.footage_items + p.comps);

    let footage_ids: Vec<Uuid> = (0..p.footage_items)
        .map(|i| {
            let id = uid(1, i);
            items.push(ProjectItem::Footage(FootageItem {
                id,
                name: format!("asset {i}"),
                media: MediaRef {
                    relative_path: format!("footage/asset{i}.mov"),
                    absolute_path: format!("footage/asset{i}.mov"),
                    fingerprint: None,
                    extra: serde_json::Map::new(),
                },
                extra: serde_json::Map::new(),
            }));
            id
        })
        .collect();

    let per_comp = layer_distribution(p.comps, p.layers_total, p.biggest_comp_layers);
    let kf_per_layer = spread(p.keyframes_total, p.layers_total);
    let fps = FrameRate::new(60, 1).unwrap();
    let duration = Duration(Rational::new(10, 1).unwrap_or(Rational::ONE));
    let out_point = CompTime(Rational::new(10, 1).unwrap_or(Rational::ONE));

    let mut layer_index = 0usize;
    for (ci, &n_layers) in per_comp.iter().enumerate() {
        let mut layers = Vec::with_capacity(n_layers);
        for _ in 0..n_layers {
            let mut transform = TransformGroup::default();
            let k = kf_per_layer.get(layer_index).copied().unwrap_or(0);
            if k > 0 {
                transform.position_x.animation = Animation::Keyframed(keyframes(k));
            }
            let item = if footage_ids.is_empty() {
                uid(1, 0)
            } else {
                footage_ids[layer_index % footage_ids.len()]
            };
            layers.push(Layer {
                id: uid(3, layer_index),
                name: format!("layer {layer_index}"),
                kind: LayerKind::Footage { item, retime: None },
                in_point: CompTime(Rational::ZERO),
                out_point,
                start_offset: CompTime(Rational::ZERO),
                transform,
                matte: None,
                parent: None,
                label: 0,
                volume_db: lumit_core::anim::Property::zero(),
                blend: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                switches: Switches::default(),
                extra: serde_json::Map::new(),
            });
            layer_index += 1;
        }
        items.push(ProjectItem::Composition(Composition {
            id: uid(2, ci),
            name: format!("Comp {ci}"),
            width: 3840,
            height: 2160,
            frame_rate: fps,
            duration,
            background: LinearColour::BLACK,
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }));
    }

    Document {
        id: uid(0, 0),
        items,
        auto_folders: AutoFolders::default(),
        extra: serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Count (footage items, comps, total layers, biggest comp, total keyframes).
    fn shape(doc: &Document) -> (usize, usize, usize, usize, usize) {
        let mut footage = 0;
        let mut comps = 0;
        let mut layers = 0;
        let mut biggest = 0;
        let mut keys = 0;
        for item in &doc.items {
            match item {
                ProjectItem::Footage(_) => footage += 1,
                ProjectItem::Composition(c) => {
                    comps += 1;
                    layers += c.layers.len();
                    biggest = biggest.max(c.layers.len());
                    for l in &c.layers {
                        if let Animation::Keyframed(k) = &l.transform.position_x.animation {
                            keys += k.len();
                        }
                    }
                }
                _ => {}
            }
        }
        (footage, comps, layers, biggest, keys)
    }

    /// The same params always build a byte-identical document (fixed ids, no
    /// clock) — the property the perf harness and golden tests rely on.
    #[test]
    fn stress_document_is_deterministic() {
        let a = stress_document(&StressParams::TINY);
        let b = stress_document(&StressParams::TINY);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    /// The tiny document has exactly the requested shape.
    #[test]
    fn stress_document_matches_its_params() {
        let p = StressParams::TINY;
        let (footage, comps, layers, biggest, keys) = shape(&stress_document(&p));
        assert_eq!(footage, p.footage_items);
        assert_eq!(comps, p.comps);
        assert_eq!(layers, p.layers_total);
        assert_eq!(biggest, p.biggest_comp_layers);
        assert_eq!(keys, p.keyframes_total);
    }

    /// The full reference document (docs/13 §2.1) builds with the exact spec
    /// counts — 200 comps, 5,000 layers (one comp of 1,000), 250,000 keyframes,
    /// 2,000 footage items.
    #[test]
    fn reference_stress_document_matches_the_spec() {
        let p = StressParams::REFERENCE;
        assert_eq!(
            shape(&stress_document(&p)),
            (2_000, 200, 5_000, 1_000, 250_000)
        );
    }

    /// The fixture survives a `.lum` save/open round-trip unchanged (the path
    /// the S4/S5 open/save budgets exercise).
    #[test]
    fn stress_document_saves_and_opens() {
        let doc = stress_document(&StressParams::TINY);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stress.lum");
        crate::save(&doc, &path).unwrap();
        let (loaded, _) = crate::open(&path).unwrap();
        assert_eq!(loaded, doc);
    }
}
