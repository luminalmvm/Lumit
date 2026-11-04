//! 3D models: telling a model item from footage, and a Model layer's settings.

use flutter_rust_bridge::frb;
use lumit_core::model::{LayerKind, ProjectItem};
use lumit_core::model_layer::{ModelSettings, UpAxis};

use crate::api::effect::BridgeEffectInstance;
use crate::api::layer::InstanceHome;
use crate::api::{
    composition::CompositionReference, footage::FootageReference, layer::LayerReference,
    BridgeError,
};
use lumit_core::model::{EffectInstance, EffectValue};
use uuid::Uuid;

/// One part of a 3D precomp's scene: the number a matte keeps, and the name to
/// show.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeScenePart {
    pub id: u32,
    pub name: String,
}

impl LayerReference {
    /// The parts of this Model layer or 3D precomp, `None` for any other layer.
    /// Not sync, as a model not read yet is read now.
    pub fn scene_parts(&self) -> Result<Option<Vec<BridgeScenePart>>, BridgeError> {
        let doc = self.document()?;
        let layer = self.item()?;
        Ok(
            lumit_render::passes::scene_parts(&doc, &layer).map(|parts| {
                parts
                    .into_iter()
                    .map(|(id, name)| BridgeScenePart { id, name })
                    .collect()
            }),
        )
    }
}

/// Which of a 3D model file's axes points up.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeUpAxis {
    Y,
    Z,
}

/// How a Viewer view shows its models: lit as they export, in a studio
/// light of the view's own, as lines, both, or as the way they face.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeModelDisplay {
    Final,
    Shaded,
    Wireframe,
    ShadedWire,
    Normals,
}

/// How a Viewer view shows its models, and whether it tints their faces by
/// which way they face.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeModelLook {
    pub display: BridgeModelDisplay,
    pub faces: bool,
}

impl BridgeModelLook {
    /// The engine's own.
    #[frb(ignore)]
    pub(crate) fn engine(self) -> lumit_gpu::mesh::display::ModelDisplay {
        use lumit_gpu::mesh::display::Shading;
        let shading = match self.display {
            BridgeModelDisplay::Final => Shading::Final,
            BridgeModelDisplay::Shaded => Shading::Shaded,
            BridgeModelDisplay::Wireframe => Shading::Wireframe,
            BridgeModelDisplay::ShadedWire => Shading::ShadedWire,
            BridgeModelDisplay::Normals => Shading::Normals,
        };
        lumit_gpu::mesh::display::ModelDisplay {
            shading,
            faces: self.faces,
        }
    }
}

impl CompositionReference {
    /// Whether this comp draws a model, its own or inside a precomp.
    #[frb(sync)]
    pub fn holds_models(&self) -> Result<bool, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        Ok(!model_items(&doc, self.id).is_empty())
    }

    /// The names of the 3D models this comp draws, precomps included, that
    /// can't be read right now. An export leaves them out, or draws the last
    /// version read. Reads any not read yet beside the caller, so never from a
    /// build.
    #[frb(sync)]
    pub fn unread_models(&self) -> Result<Vec<String>, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        let mut names = Vec::new();
        for item in model_items(&doc, self.id) {
            let Some(ProjectItem::Footage(f)) = doc.item(item) else {
                continue;
            };
            let state = lumit_scene::blender::state(std::path::Path::new(f.media.on_disk()));
            let trouble = !matches!(state, None | Some(lumit_scene::blender::State::Converting));
            let scene = lumit_render::model::scene_if_read(&doc, item);
            if (trouble || matches!(scene, Some(None))) && !names.contains(&f.name) {
                names.push(f.name.clone());
            }
        }
        Ok(names)
    }
}

/// The 3D models comp `comp` draws, precomps included.
fn model_items(doc: &lumit_core::model::Document, comp: Uuid) -> Vec<Uuid> {
    let mut seen = vec![comp];
    let mut items = Vec::new();
    let mut i = 0;
    // Each comp once, so a comp nested twice or in a loop is walked once.
    while let Some(&id) = seen.get(i) {
        i += 1;
        for layer in doc.comp(id).map_or(&[][..], |c| &c.layers) {
            match &layer.kind {
                LayerKind::Model { item, .. } if !items.contains(item) => items.push(*item),
                LayerKind::Precomp { comp } if !seen.contains(comp) => seen.push(*comp),
                _ => {}
            }
        }
    }
    items
}

/// How a 3D model sits in its Model layer, so its Scale stays at 100%.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeModelSettings {
    pub up_axis: BridgeUpAxis,
    /// Comp pixels for each metre of the model.
    pub units: f64,
    /// Sized to fill the comp and centred, in place of `units`.
    pub fit: bool,
}

impl FootageReference {
    /// Whether this item is a 3D model rather than footage that plays.
    #[frb(sync)]
    pub fn is_model(&self) -> Result<bool, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(match proj.store.snapshot().item(self.id()) {
            Some(ProjectItem::Footage(f)) => lumit_scene::is_model(f.media.on_disk()),
            _ => false,
        })
    }

    /// The pictures this 3D model names that are missing or won't read, as
    /// it names them. Reads the model if nothing has yet, so never from a
    /// build.
    pub fn get_model_missing(&self) -> Result<Vec<String>, BridgeError> {
        let doc = {
            let proj = self.project()?;
            let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
            proj.store.snapshot()
        };
        Ok(lumit_render::model::missing(&doc, self.id()))
    }
}

/// A layer's pose at comp frame `frame`, `None` when it has none.
#[frb(ignore)]
pub(crate) fn pose_at_frame(
    doc: &std::sync::Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    frame: i64,
) -> Option<lumit_scene::pose::Pose> {
    let t = comp.frame_rate.time_of_frame(frame.max(0)).ok()?.0.to_f64();
    lumit_render::model_motion::layer_pose(doc, comp, layer, t)
}

impl LayerReference {
    /// A Model layer's settings, or `None` on any other kind of layer.
    #[frb(sync)]
    pub fn get_model_settings(&self) -> Result<Option<BridgeModelSettings>, BridgeError> {
        Ok(match self.item()?.kind {
            LayerKind::Model { settings, .. } => Some(BridgeModelSettings {
                up_axis: match settings.up_axis {
                    UpAxis::Y => BridgeUpAxis::Y,
                    UpAxis::Z => BridgeUpAxis::Z,
                },
                units: settings.units,
                fit: settings.fit,
            }),
            _ => None,
        })
    }

    /// The corners of a Model layer's box in its own pixels at comp frame
    /// `at_frame`, three numbers each, once its file has been read. Empty
    /// before that and on any other kind of layer.
    #[frb(sync)]
    pub fn get_model_box(&self, at_frame: i64) -> Result<Vec<f64>, BridgeError> {
        let layer = self.item()?;
        let LayerKind::Model { item, settings } = &layer.kind else {
            return Ok(Vec::new());
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let size = (comp.width as f32, comp.height as f32);
        let pose = pose_at_frame(&doc, comp, &layer, at_frame);
        let corners = lumit_render::model::layer_box(&doc, *item, settings, size, pose.as_ref());
        Ok(corners
            .iter()
            .flatten()
            .flatten()
            .map(|v| f64::from(*v))
            .collect())
    }

    /// The comp pixels a metre Fit to comp gives a Model layer's model with
    /// `up_axis` up, once its file has been read. `None` before that and on
    /// any other kind of layer.
    #[frb(sync)]
    pub fn get_model_fitted_units(
        &self,
        up_axis: BridgeUpAxis,
    ) -> Result<Option<f64>, BridgeError> {
        let LayerKind::Model { item, .. } = self.item()?.kind else {
            return Ok(None);
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let up = match up_axis {
            BridgeUpAxis::Y => UpAxis::Y,
            BridgeUpAxis::Z => UpAxis::Z,
        };
        let size = (comp.width as f32, comp.height as f32);
        Ok(lumit_render::model::fitted_units(&doc, item, up, size).map(f64::from))
    }

    /// How far along the ray from `origin` by `direction`, three numbers
    /// each in the layer's own pixels, it first meets a Model layer's model,
    /// in parts of `direction`. `None` where it misses, before the file has
    /// been read and on any other kind of layer. Asked once a click, and when
    /// the pointer rests for a moment, as the model is at comp frame `at_frame`.
    #[frb(sync)]
    pub fn get_model_hit(
        &self,
        origin: Vec<f64>,
        direction: Vec<f64>,
        at_frame: i64,
    ) -> Result<Option<f64>, BridgeError> {
        let layer = self.item()?;
        let LayerKind::Model { item, settings } = &layer.kind else {
            return Ok(None);
        };
        let (Ok([ox, oy, oz]), Ok([dx, dy, dz])) = (
            <[f64; 3]>::try_from(origin),
            <[f64; 3]>::try_from(direction),
        ) else {
            return Ok(None);
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let pose = pose_at_frame(&doc, comp, &layer, at_frame);
        Ok(lumit_render::model::hit(
            &doc,
            *item,
            settings,
            (comp.width as f32, comp.height as f32),
            pose.as_ref(),
            [ox, oy, oz].map(|v| v as f32),
            [dx, dy, dz].map(|v| v as f32),
        )
        .map(|(along, _)| f64::from(along)))
    }

    /// The document as it is now.
    #[frb(ignore)]
    pub(crate) fn snapshot(
        &self,
    ) -> Result<std::sync::Arc<lumit_core::model::Document>, BridgeError> {
        let proj = self.project()?;
        let proj = proj.read().map_err(|_| BridgeError::ReadFailed)?;
        Ok(proj.store.snapshot())
    }

    /// Replace a Model layer's settings, as one undo step. Refused on any
    /// other kind of layer.
    #[frb(sync)]
    pub fn set_model_settings(&self, settings: BridgeModelSettings) -> Result<(), BridgeError> {
        let LayerKind::Model {
            item,
            settings: kept,
        } = self.item()?.kind
        else {
            return Err(BridgeError::InvalidLayer);
        };
        let settings = ModelSettings {
            take: kept.take,
            playback: kept.playback,
            parts: kept.parts,
            up_axis: match settings.up_axis {
                BridgeUpAxis::Y => UpAxis::Y,
                BridgeUpAxis::Z => UpAxis::Z,
            },
            // Nought would shrink the model to nothing and no further.
            units: settings.units.max(f64::EPSILON),
            fit: settings.fit,
            materials: kept.materials,
        };
        self.commit(lumit_core::ops::Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(LayerKind::Model {
                item,
                settings: Box::new(settings),
            }),
        })
    }

    /// A Model layer's material rows as staged copies, one for each material
    /// in its file, in the file's order. Empty until the file has been read,
    /// and on any other kind of layer.
    #[frb(sync)]
    pub fn get_materials(&self) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let (doc, layer) = (self.snapshot()?, self.item()?);
        let rows = lumit_render::model_material::rows(&doc, &layer);
        Ok(rows
            .unwrap_or_default()
            .into_iter()
            .map(|rows| BridgeEffectInstance::new(rows, layer.start_offset.0))
            .collect())
    }

    /// Set `material`'s rows to the first material in preset `text`, as one
    /// undo step, any keyframes starting at comp frame `at_frame`, the
    /// playhead. The layers its rows read are kept.
    #[frb(sync)]
    pub fn apply_material_preset(
        &self,
        material: &BridgeEffectInstance,
        text: String,
        at_frame: i64,
    ) -> Result<(), BridgeError> {
        let mut found = lumit_core::preset::from_json(&text)
            .ok()
            .and_then(|preset| {
                (preset.effects.into_iter())
                    .find(|e| e.effect.match_name == lumit_core::model_material::MATCH_NAME)
            })
            .ok_or(BridgeError::InvalidPreset)?;
        let (comp, layer) = (self.composition()?, self.item()?);
        let at = (comp.frame_rate.time_of_frame(at_frame.max(0)))
            .and_then(|t| t.delta(layer.start_offset))
            .map_err(|_| BridgeError::InvalidTime)?;
        let found_keys = std::slice::from_mut(&mut found);
        if let Some(first) = lumit_core::preset::first_key_time(found_keys) {
            let delta =
                at.0.checked_sub(first)
                    .map_err(|_| BridgeError::InvalidTime)?;
            lumit_core::preset::shift_keys(found_keys, delta);
        }
        let id = material.id();
        self.with_instances(InstanceHome::Materials, move |list| {
            let rows = list
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(BridgeError::InvalidEffect)?;
            for p in &mut rows.params {
                let theirs = found.params.iter().find(|q| q.id == p.id);
                if let Some(q) = theirs.filter(|q| !matches!(q.value, EffectValue::Layer(_))) {
                    p.value = q.value.clone();
                }
            }
            Ok(())
        })
    }

    /// Put row `param` of `material` back to its file's value, as one undo
    /// step.
    #[frb(sync)]
    pub fn reset_material_row(
        &self,
        material: &BridgeEffectInstance,
        param: String,
    ) -> Result<(), BridgeError> {
        let (doc, layer, id) = (self.snapshot()?, self.item()?, material.id());
        let file = lumit_render::model_material::file_rows(&doc, &layer, id)
            .ok_or(BridgeError::InvalidEffect)?;
        let value = (file.params.into_iter())
            .find(|p| p.id == param)
            .ok_or(BridgeError::InvalidEffect)?
            .value;
        self.with_instances(InstanceHome::Materials, move |list| {
            let rows = list.iter_mut().find(|e| e.id == id);
            let row = rows.and_then(|r| r.params.iter_mut().find(|p| p.id == param));
            row.ok_or(BridgeError::InvalidEffect)?.value = value;
            Ok(())
        })
    }

    /// The op that keeps a Model layer's material rows once `edit` has
    /// changed them: only the materials that differ from the file.
    #[frb(ignore)]
    pub(crate) fn materials_op(
        &self,
        edit: impl FnOnce(&mut Vec<EffectInstance>) -> Result<(), BridgeError>,
    ) -> Result<lumit_core::ops::Op, BridgeError> {
        let (doc, layer) = (self.snapshot()?, self.item()?);
        let mut list =
            lumit_render::model_material::rows(&doc, &layer).ok_or(BridgeError::InvalidEffect)?;
        edit(&mut list)?;
        let materials = lumit_render::model_material::kept(&doc, &layer, &list)
            .ok_or(BridgeError::StaleEffectStack)?;
        let LayerKind::Model { item, settings } = layer.kind else {
            return Err(BridgeError::InvalidLayer);
        };
        let settings = Box::new(ModelSettings {
            materials,
            ..*settings
        });
        Ok(lumit_core::ops::Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(LayerKind::Model { item, settings }),
        })
    }
}

/// The user's saved material presets, sorted by name.
#[frb(sync)]
pub fn list_material_presets() -> Vec<crate::api::effect::BridgePresetInfo> {
    lumit_project::presets_dir()
        .map(|dir| {
            crate::api::effect::presets_in(&dir, lumit_core::model_material::PRESET_EXTENSION)
        })
        .unwrap_or_default()
}

/// A number that moves when a 3D model's file starts or finishes being read,
/// which is when a Model layer's material rows come.
#[frb(sync)]
pub fn model_reads() -> u64 {
    lumit_render::model::reads()
}

/// `material`'s rows as the text of a preset named `name`.
#[frb(sync)]
pub fn material_preset(
    material: &BridgeEffectInstance,
    name: String,
) -> Result<String, BridgeError> {
    lumit_core::preset::to_json(&name, &[material.get_effects()])
        .map_err(|_| BridgeError::InvalidPreset)
}
