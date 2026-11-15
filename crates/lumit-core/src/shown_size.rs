//! The largest size a footage layer is shown at, as a fraction of its own
//! size, so a preview can decode it no bigger (Settings → Performance →
//! Decode footage at shown size).
//!
//! # In plain terms
//!
//! A 4K clip scaled to 30% in a 1080p composition shows about 1150 pixels
//! across, yet it was decoded at 3840 and shrunk on the card every frame. The
//! decode, the upload and the cache all paid for pixels nobody saw. This
//! answers how small the clip can be decoded and still look the same.
//!
//! The idea follows Venturi's "Decode at the size media show at"
//! (github.com/VenturiVideo/Venturi pull 108, GPL-3.0-or-later): the largest
//! scale a clip reaches, keys included, rounded up in steps of √2 so a scale
//! being animated does not drop the cache at every frame. No code is taken from
//! it; Lumit already decodes to a width for its preview resolutions, so only
//! the width had to be worked out.
//!
//! **Only where nothing else reads the pixels at their own size.** A layer
//! with a parent, in 3D, with masks, paint, a puppet, effects, styles or a
//! driver graph, or inside a precomp whose transformations collapse into the
//! comp above, answers `None` and decodes as before. So does a scale an
//! expression drives, since nothing tells how large it gets.
//!
//! Both the decode plan and the frame's name in the cache ask this, with the
//! same document, so the width the pixels are decoded at is the width the name
//! says.

use crate::anim::{Animation, Property};
use crate::model::{Document, Layer, LayerKind, ProjectItem};

/// The smallest fraction a layer is decoded at, however small it is shown.
const SMALLEST: f64 = 1.0 / 16.0;

/// Headroom over the largest key, for an eased key that overshoots it.
const HEADROOM: f64 = 1.1;

/// The fraction of its own size `layer` can be decoded at, or `None` to decode
/// it at its own size.
#[must_use]
pub fn largest_scale(doc: &Document, layer: &Layer) -> Option<f32> {
    if layer.parent.is_some()
        || layer.is_3d()
        || layer.adjustment
        || !layer.masks.is_empty()
        || !layer.paint.is_empty()
        || layer.puppet.is_some()
        || !layer.styles.is_empty()
        || !layer.graph.is_empty()
        || (layer.switches.fx && !layer.effects.is_empty())
        || inside_collapse(doc, layer)
    {
        return None;
    }
    let x = most(&layer.transform.scale_x)?;
    let y = most(&layer.transform.scale_y)?;
    step(x.max(y) / 100.0 * HEADROOM)
}

/// The largest magnitude `p` takes, or `None` when an expression drives it.
fn most(p: &Property) -> Option<f64> {
    match &p.animation {
        Animation::Static(v) => Some(v.abs()),
        Animation::Keyframed(keys) => keys.iter().map(|k| k.value.abs()).reduce(f64::max),
        Animation::Expression(_) => None,
    }
}

/// `scale` rounded up to a power of √2, or `None` at its own size or more.
#[must_use]
pub fn step(scale: f64) -> Option<f32> {
    if !scale.is_finite() || scale >= 1.0 {
        return None;
    }
    let halves = (scale.max(SMALLEST).log2() * 2.0).ceil();
    let stepped = (halves / 2.0).exp2();
    (stepped < 1.0).then_some(stepped as f32)
}

/// Whether `layer` sits in a comp that a collapsed Precomp layer splices into
/// the comp above, where it is shown at that comp's transform rather than its
/// own comp's size.
fn inside_collapse(doc: &Document, layer: &Layer) -> bool {
    doc.items.iter().any(|item| {
        let ProjectItem::Composition(comp) = item else {
            return false;
        };
        comp.layers.iter().any(|l| {
            let LayerKind::Precomp { comp: nested } = l.kind else {
                return false;
            };
            l.switches.collapse
                && doc
                    .comp(nested)
                    .is_some_and(|n| n.layers.iter().any(|x| x.id == layer.id))
        })
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_scale_rounds_up_to_a_step_of_root_two() {
        assert_eq!(step(1.0), None, "its own size decodes as it is");
        assert_eq!(step(1.5), None, "larger than its own size too");
        assert_eq!(step(0.5), Some(0.5));
        let s = step(0.3).unwrap();
        assert!(
            (s - 0.353_553_4).abs() < 1e-4,
            "0.3 goes up to √2/4, got {s}"
        );
        assert_eq!(step(0.001), Some(0.0625), "never below a sixteenth");
        assert_eq!(step(f64::NAN), None);
    }

    #[test]
    fn a_still_or_keyed_scale_is_read_at_its_largest() {
        assert_eq!(most(&Property::fixed(-40.0)), Some(40.0));
        assert_eq!(
            most(&Property {
                animation: Animation::Expression("value".into()),
                extra: serde_json::Map::new(),
            }),
            None
        );
    }
}
