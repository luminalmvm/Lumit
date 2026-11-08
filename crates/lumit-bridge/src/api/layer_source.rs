//! Pointing a layer at a different source, keeping everything else on it.

use flutter_rust_bridge::frb;
use lumit_core::anim::{Animation, Property};
use lumit_core::model::{Composition, Document, LayerKind, TransformProp};
use lumit_core::ops::Op;
use uuid::Uuid;

use crate::api::{
    composition::CompositionReference, layer::LayerReference, project_item::ItemReference,
    BridgeError,
};

/// Whether `inner` shows `outer` somewhere inside it, however deep.
fn nests(doc: &Document, inner: Uuid, outer: Uuid, seen: &mut Vec<Uuid>) -> bool {
    if inner == outer {
        return true;
    }
    if seen.contains(&inner) {
        return false;
    }
    seen.push(inner);
    doc.comp(inner).is_some_and(|comp| {
        comp.layers.iter().any(|l| match l.kind {
            LayerKind::Precomp { comp } => nests(doc, comp, outer, seen),
            _ => false,
        })
    })
}

/// `p` with every number times `k`. An expression is left to itself.
fn scaled(p: &Property, k: f64) -> Option<Animation> {
    match &p.animation {
        Animation::Static(v) => Some(Animation::Static(v * k)),
        Animation::Keyframed(keys) => {
            let mut keys = keys.clone();
            for key in &mut keys {
                key.value *= k;
            }
            Some(Animation::Keyframed(keys))
        }
        Animation::Expression(_) => None,
    }
}

impl LayerReference {
    /// Whether this layer can take another source: footage, a solid or a
    /// precomp.
    #[frb(sync)]
    pub fn can_replace_source(&self) -> Result<bool, BridgeError> {
        Ok(matches!(
            self.item()?.kind,
            LayerKind::Footage { .. } | LayerKind::Solid { .. } | LayerKind::Precomp { .. }
        ))
    }

    /// Point this layer at `item`, keeping its transform, keys, masks, effects
    /// and markers. An anchor keeps its place on the picture as the size
    /// changes, a name that was the old source's follows the new one, and a
    /// Retime goes when the layer becomes a solid, which has no time to remap.
    /// Returns whether it did. One undo step.
    #[frb(sync)]
    pub fn set_source_item(&self, item: &ItemReference) -> Result<bool, BridgeError> {
        let layer = self.item()?;
        let comp = self.composition()?;
        let doc = self.document()?;
        let kind = match item {
            ItemReference::Footage(f) => LayerKind::Footage { item: f.id },
            ItemReference::Solid(s) => LayerKind::Solid { def: s.id },
            ItemReference::Composition(c) => {
                if nests(&doc, c.id, comp.id, &mut Vec::new()) {
                    return Err(BridgeError::InvalidComp);
                }
                LayerKind::Precomp { comp: c.id }
            }
            ItemReference::Folder(_) => return Err(BridgeError::InvalidItem),
        };
        if !self.can_replace_source()? {
            return Err(BridgeError::NotConvertible);
        }
        let new_id = source_of(&kind);
        if let ItemReference::Footage(f) = item {
            let Some(lumit_core::model::ProjectItem::Footage(footage)) = doc.item(f.id) else {
                return Err(BridgeError::InvalidItem);
            };
            // A model is drawn by a kind of its own, and a file with no picture
            // would leave a picture layer with nothing on it.
            if lumit_scene::is_model(footage.media.on_disk()) {
                return Err(BridgeError::NotConvertible);
            }
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            if !layer.audio_only && !CompositionReference::has_picture(&state, footage) {
                return Err(BridgeError::NotConvertible);
            }
        }
        let mut ops = vec![Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(kind.clone()),
        }];
        let was = source_of(&layer.kind);
        if was.and_then(|id| doc.item(id)).map(|i| i.name()) == Some(layer.name.as_str()) {
            if let Some(name) = new_id.and_then(|id| doc.item(id)).map(|i| i.name()) {
                ops.push(Op::RenameLayer {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    name: name.to_string(),
                });
            }
        }
        let sizes = {
            let proj = self.project()?;
            let state = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            (
                size_of(&state, &doc, &comp, &layer.kind),
                size_of(&state, &doc, &comp, &kind),
            )
        };
        if let (Some((from_w, from_h)), Some((to_w, to_h))) = sizes {
            let tr = &layer.transform;
            for (prop, property, k) in [
                (TransformProp::AnchorX, &tr.anchor_x, to_w / from_w),
                (TransformProp::AnchorY, &tr.anchor_y, to_h / from_h),
            ] {
                if (k - 1.0).abs() < 1e-9 || !k.is_finite() {
                    continue;
                }
                if let Some(animation) = scaled(property, k) {
                    ops.push(Op::SetTransformProperty {
                        comp: self.comp_id,
                        layer: self.layer_id,
                        prop,
                        animation,
                    });
                }
            }
        }
        if matches!(kind, LayerKind::Solid { .. }) && layer.retime.is_some() {
            ops.push(self.unretime_op(
                &layer,
                Op::SetRetimeProperty {
                    comp: self.comp_id,
                    layer: self.layer_id,
                    retime: None,
                },
            ));
        }
        self.commit(Op::Batch { ops })?;
        Ok(true)
    }
}

/// The project item a layer of this kind shows.
fn source_of(kind: &LayerKind) -> Option<Uuid> {
    match kind {
        LayerKind::Footage { item } => Some(*item),
        LayerKind::Solid { def } => Some(*def),
        LayerKind::Precomp { comp } => Some(*comp),
        _ => None,
    }
}

/// How big a picture this kind of layer has, in its own pixels.
fn size_of(
    state: &crate::api::state::LumitBridgeState,
    doc: &Document,
    comp: &Composition,
    kind: &LayerKind,
) -> Option<(f64, f64)> {
    let (w, h) = match kind {
        LayerKind::Footage { item } => {
            let Some(lumit_core::model::ProjectItem::Footage(f)) = doc.item(*item) else {
                return None;
            };
            let (_, w, h) = CompositionReference::footage_span_and_size(state, f, comp);
            (w, h)
        }
        LayerKind::Solid { def } => {
            let s = doc.solid(*def)?;
            (f64::from(s.width), f64::from(s.height))
        }
        LayerKind::Precomp { comp } => {
            let c = doc.comp(*comp)?;
            (f64::from(c.width), f64::from(c.height))
        }
        _ => return None,
    };
    (w > 0.0 && h > 0.0).then_some((w, h))
}
