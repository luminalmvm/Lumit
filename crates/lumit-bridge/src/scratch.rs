//! The composition a footage view or a layer view is drawn through
//! (docs/impl/multi-viewer.md §3.6).
//!
//! # In plain terms
//!
//! A Viewer view can show a piece of footage on its own, or one layer's source
//! before its transform. Neither of those is a composition, and the engine has
//! exactly one road to a picture: composite a composition. So one is built for
//! the item, on a **clone** of the document, and sent down the ordinary render
//! path. Nothing is committed: no op, no journal entry, no undo step, and
//! nothing in the Project panel. It is the same trick every drag preview
//! already plays, which is what makes it cheap to be sure of.
//!
//! Two rules make it work with the cache rather than against it.
//!
//! **The scratch composition's id is derived from the item's**, so the same
//! footage always makes the same composition and its frames name themselves the
//! same way from one ask to the next. A random id would name a new frame every
//! time and the cache would never hit.
//!
//! **It is sized to the item**, at the item's own rate, so what is shown is the
//! source and not the source letterboxed into whatever composition happened to
//! be fronted.

use lumit_core::model::{Composition, Document, LayerKind, ProjectItem};
use lumit_core::time::{Duration, FrameRate, Rational};
use std::sync::Arc;
use uuid::Uuid;

/// The namespace every scratch composition id is minted inside, so one can
/// never collide with a composition the document actually has.
const SCRATCH_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6c, 0x75, 0x6d, 0x69, 0x74, 0x00, 0x50, 0x00, 0x80, 0x00, 0x73, 0x63, 0x72, 0x61, 0x74, 0x63,
]);

/// What a scratch composition was built for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ScratchOf {
    /// A project footage item, with its interpretation applied.
    Footage(Uuid),
    /// One layer's source before its transform, with its masks.
    Layer { comp: Uuid, layer: Uuid },
}

impl ScratchOf {
    /// The composition id this scratch always takes.
    ///
    /// Derived rather than minted: the same item asked for twice must name the
    /// same frames, or every ask is a miss and the picture is composited from
    /// nothing each time. Namespaced so it can never collide with a
    /// composition the document actually has.
    #[must_use]
    pub fn comp_id(self) -> Uuid {
        // A version 5 uuid is exactly this: a name inside a namespace, always
        // the same answer. No hash of our own, and no new dependency.
        let name = match self {
            ScratchOf::Footage(item) => format!("footage/{item}"),
            ScratchOf::Layer { comp, layer } => format!("layer/{comp}/{layer}"),
        };
        Uuid::new_v5(&SCRATCH_NAMESPACE, name.as_bytes())
    }
}

/// The document with the scratch composition added, or `None` when there is
/// nothing to show one of: an item that has gone, a layer that has gone, or a
/// piece of media with no picture in it.
///
/// `effects` says whether the layer view runs the layer's effect stack. After
/// Effects calls this the Render tick, and it is the difference between "what
/// the source is" and "what this layer makes"; a footage view never has one.
#[must_use]
pub fn document_with(
    document: &Arc<Document>,
    of: ScratchOf,
    effects: bool,
    size: (u32, u32),
    rate: FrameRate,
    duration: Duration,
) -> Option<Arc<Document>> {
    let (name, layer) = match of {
        ScratchOf::Footage(item) => {
            let Some(ProjectItem::Footage(f)) = document.item(item) else {
                return None;
            };
            let layer = crate::edits::base_layer(
                f.name.clone(),
                LayerKind::Footage { item },
                duration.0,
                // Identity: a footage view shows the source, not the source
                // placed somewhere.
                crate::edits::centred_transform(
                    f64::from(size.0),
                    f64::from(size.1),
                    size.0,
                    size.1,
                ),
            );
            (f.name.clone(), layer)
        }
        ScratchOf::Layer { comp, layer } => {
            let source = document.comp(comp)?.layers.iter().find(|l| l.id == layer)?;
            let mut copy = source.clone();
            // Before transform, which is what a layer view is for: the source
            // in its own frame, with its masks and its anchor point on it.
            copy.transform = crate::edits::centred_transform(
                f64::from(size.0),
                f64::from(size.1),
                size.0,
                size.1,
            );
            copy.parent = None;
            copy.matte = None;
            // The whole of it, from the start: a layer view shows the source,
            // not the slice of it this composition happens to use.
            copy.in_point = lumit_core::time::CompTime(Rational::ZERO);
            copy.out_point = lumit_core::time::CompTime(duration.0);
            if !effects {
                copy.effects.clear();
            }
            (copy.name.clone(), copy)
        }
    };

    let mut scratch = Composition {
        id: of.comp_id(),
        name,
        width: size.0.clamp(16, 16384),
        height: size.1.clamp(16, 16384),
        frame_rate: rate,
        duration,
        background: lumit_core::model::LinearColour([0.0, 0.0, 0.0, 0.0]),
        work_area: None,
        layers: vec![layer],
        groups: Vec::new(),
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        master_volume_db: 0.0,
        // A scratch composition is never a mix: it is one item on its own, and
        // the Audio timeline has nothing to show of it.
        sound_mix: false,
        beat_grid: None,
        extra: serde_json::Map::new(),
    };
    // A source is shown as it is: no motion blur, no work area, and nothing
    // behind it but the transparency the Viewer draws its board through.
    scratch.motion_blur.enabled = false;

    let mut doc = document.as_ref().clone();
    // Straight onto the clone, never through an op: a view is a way of
    // looking, and this composition must not reach the document, the journal,
    // the undo stack or the Project panel.
    doc.items.push(ProjectItem::Composition(scratch));
    Some(Arc::new(doc))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The same item always names the same composition, so the frames it makes
    /// can be found again. A minted id would miss the cache every time.
    #[test]
    fn a_scratch_composition_keeps_its_name() {
        let item = Uuid::now_v7();
        assert_eq!(
            ScratchOf::Footage(item).comp_id(),
            ScratchOf::Footage(item).comp_id()
        );
        assert_ne!(
            ScratchOf::Footage(item).comp_id(),
            ScratchOf::Footage(Uuid::now_v7()).comp_id()
        );
    }

    /// A footage view and a layer view of the same uuid are different
    /// questions and must not share a picture.
    #[test]
    fn the_two_kinds_never_collide() {
        let id = Uuid::now_v7();
        assert_ne!(
            ScratchOf::Footage(id).comp_id(),
            ScratchOf::Layer {
                comp: id,
                layer: id
            }
            .comp_id()
        );
    }
}
