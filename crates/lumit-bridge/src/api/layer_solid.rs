//! Layer settings on a solid layer: one layer, or every layer using the solid.

use flutter_rust_bridge::frb;
use lumit_core::model::{Document, LayerKind, ProjectItem};
use lumit_core::ops::Op;
use lumit_core::sequence::ClipSource;
use uuid::Uuid;

use crate::api::{
    assets::{linear_of, BridgeColourRgba, BridgeSolidDef},
    composition::CompositionReference,
    layer::LayerReference,
    worker_thread::{RenderCompRequestWithPreview, WorkerRequest},
    BridgeError,
};

/// How many layers, in every composition, draw the solid `def`. A Sequence
/// layer counts once for each clip of it.
fn users(doc: &Document, def: Uuid) -> u32 {
    let mut n = 0;
    for item in &doc.items {
        let ProjectItem::Composition(comp) = item else {
            continue;
        };
        for layer in &comp.layers {
            n += match &layer.kind {
                LayerKind::Solid { def: d } => u32::from(*d == def),
                LayerKind::Sequence { clips } => clips
                    .iter()
                    .flat_map(|c| c.sources())
                    .filter(|s| matches!(s, ClipSource::Solid(d) if *d == def))
                    .count() as u32,
                _ => 0,
            };
        }
    }
    n
}

impl LayerReference {
    /// How many layers draw this layer's solid, this one included. Zero for a
    /// layer that is not a solid. A duplicated layer shares its original's
    /// solid, as a solid dragged in from the Project panel does.
    #[frb(sync)]
    pub fn solid_layer_count(&self) -> Result<u32, BridgeError> {
        let LayerKind::Solid { def } = self.item()?.kind else {
            return Ok(0);
        };
        Ok(users(&*self.document()?, def))
    }

    /// Change this solid layer's colour and size. With `all_layers`, or when
    /// no other layer uses the solid, the solid itself is edited. Otherwise
    /// the layer gets a solid of its own with the new settings and the others
    /// keep theirs. On a shared solid the name stays the layer's own and never
    /// renames it. One undo step.
    #[frb(sync)]
    pub fn set_solid_settings(
        &self,
        definition: BridgeSolidDef,
        all_layers: bool,
    ) -> Result<(), BridgeError> {
        let LayerKind::Solid { def } = self.item()?.kind else {
            return Err(BridgeError::NotConvertible);
        };
        let doc = self.document()?;
        let Some(ProjectItem::Solid(old)) = doc.item(def) else {
            return Err(BridgeError::InvalidItem);
        };
        let name = definition.name.trim().to_owned();
        if name.is_empty() {
            return Err(BridgeError::EmptyName);
        }
        let colour = linear_of(definition.colour);
        let (width, height) = (definition.width.max(1), definition.height.max(1));

        let shared = users(&doc, def) > 1;
        if all_layers || !shared {
            // The name in this window is the layer's, so it never renames a
            // shared solid.
            let name = if shared { old.name.clone() } else { name };
            if (&name, colour, width, height) == (&old.name, old.colour, old.width, old.height) {
                return Ok(());
            }
            return self.commit(Op::SetSolidDef {
                def,
                name,
                colour,
                width,
                height,
            });
        }
        if (colour, width, height) == (old.colour, old.width, old.height) {
            return Ok(());
        }
        let (folder, mut ops) = lumit_core::edits::ensure_auto_folder_ops(
            &doc,
            lumit_core::ops::AutoFolderKind::Solids,
        );
        let added = ops
            .iter()
            .filter(|o| matches!(o, Op::AddItem { .. }))
            .count();
        let mut own = old.clone();
        own.id = Uuid::now_v7();
        own.name = name;
        own.colour = colour;
        own.width = width;
        own.height = height;
        let id = own.id;
        ops.push(Op::AddItem {
            index: doc.items.len() + added,
            item: Box::new(ProjectItem::Solid(own)),
        });
        ops.push(lumit_core::edits::file_into_folder_op(&doc, folder, id));
        ops.push(Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(LayerKind::Solid { def: id }),
        });
        self.commit(Op::Batch { ops })
    }
}

impl CompositionReference {
    #[frb(sync)]
    pub fn render_frame_with_solid_preview(
        &self,
        frame: u64,
        scale: f32,
        layer: LayerReference,
        colour: BridgeColourRgba,
        all_layers: bool,
    ) -> Result<(), BridgeError> {
        self.dispatch(WorkerRequest::RenderCompWithPreview(
            RenderCompRequestWithPreview {
                comp: self.clone(),
                frame,
                scale,
                layer: Some(layer),
                graph_instances: None,
                effects: None,
                drivers: None,
                transform: None,
                text: None,
                paint: None,
                contents: None,
                masks: None,
                clip_retime: None,
                clip_span: None,
                solid: Some((colour, all_layers)),
                retime: None,
            },
        ))
    }
}

#[frb(ignore)]
pub(crate) fn stage_solid(
    doc: &mut Document,
    layer: &LayerReference,
    colour: BridgeColourRgba,
    all_layers: bool,
) {
    let def = match doc
        .comp(layer.comp_id)
        .and_then(|c| c.layers.iter().find(|l| l.id == layer.layer_id))
    {
        Some(lumit_core::model::Layer {
            kind: LayerKind::Solid { def },
            ..
        }) => *def,
        _ => return,
    };
    let shared = users(doc, def) > 1;
    let Some(ProjectItem::Solid(solid)) = doc.item_mut(def) else {
        return;
    };
    if all_layers || !shared {
        solid.colour = linear_of(colour);
        return;
    }
    let mut own = solid.clone();
    own.id = Uuid::now_v7();
    own.colour = linear_of(colour);
    let id = own.id;
    doc.items.push(ProjectItem::Solid(own));
    if let Some(l) = doc
        .comp_mut(layer.comp_id)
        .and_then(|c| c.layers.iter_mut().find(|l| l.id == layer.layer_id))
    {
        l.kind = LayerKind::Solid { def: id };
    }
}
