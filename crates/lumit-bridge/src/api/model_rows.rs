//! A Model layer's Animation and parts' rows, and the picks a click makes
//! among its parts.

use flutter_rust_bridge::frb;
use lumit_core::model::{Document, EffectInstance, Layer, LayerKind};
use lumit_core::model_layer::{ModelSettings, Take};
use lumit_core::model_part::{part_id, PART_NAME};
use lumit_scene::ModelScene;
use uuid::Uuid;

use crate::api::composition::CompositionReference;
use crate::api::effect::{
    read_instance_info, BridgeEffectInstance, BridgeEffectInstanceInfo, BridgeRowOptions,
};
use crate::api::layer::LayerReference;
use crate::api::place::AUTO_KEY;
use crate::api::BridgeError;

/// One node of a 3D model's file, in the tree under its layer.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeModelPart {
    /// Its rows' id.
    pub id: Uuid,
    /// As the file names it, empty for a node it doesn't.
    pub name: String,
    /// How many parents it has.
    pub depth: u32,
    /// Its rows when it keeps any, `None` when they're [`BridgeModelRows::untouched`].
    pub rows: Option<BridgeEffectInstanceInfo>,
    /// Whether a skin places what it draws, so its own transform moves nothing.
    pub skinned: bool,
}

/// Where a part is at one frame, in its layer's own pixels.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeModelPartPlace {
    /// Its pivot, three numbers.
    pub pivot: Vec<f64>,
    /// The corners of the box round what it moves, three numbers each, by the
    /// bits of x, y and z. Empty on a part that moves nothing of its own.
    pub corners: Vec<f64>,
}

/// A Model layer's rows, for the fold to draw.
#[frb(non_opaque)]
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeModelRows {
    /// Animation, Loop and Offset, `None` when the file holds no takes.
    pub playback: Option<BridgeEffectInstanceInfo>,
    /// A part's rows as nobody has changed them, so a file of a thousand nodes
    /// doesn't send them a thousand times.
    pub untouched: Option<BridgeEffectInstanceInfo>,
    /// The file's tree, each node before its children.
    pub parts: Vec<BridgeModelPart>,
    /// Whether the model's shape changes with the playhead.
    pub moves: bool,
    /// Whether the file holds a camera or a light to make a layer from.
    pub holds_lights: bool,
    /// The take the layer names that the file no longer has.
    pub lost_take: Option<String>,
}

/// `layer`'s rows, `None` while its file is being read and on any other kind of
/// layer.
pub(crate) fn read_model_rows(doc: &Document, layer: &Layer) -> Option<BridgeModelRows> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return None;
    };
    let offset = layer.start_offset.0;
    let scene = lumit_render::model::scene_if_read(doc, *item)?;
    let Some(scene) = scene else {
        return Some(BridgeModelRows {
            playback: None,
            untouched: None,
            parts: Vec::new(),
            moves: false,
            holds_lights: false,
            lost_take: None,
        });
    };
    let list = lumit_render::model_motion::all_rows(layer, settings, &scene);
    let read = |rows: &EffectInstance| read_instance_info(rows, offset, true);
    let playback = list
        .first()
        .filter(|_| !scene.clips.is_empty())
        .map(|rows| {
            let mut info = read(rows);
            let names = scene.clips.iter().map(|c| c.name.clone());
            let options = std::iter::once("None".to_owned()).chain(names).collect();
            info.row_options.push(BridgeRowOptions {
                id: "take".to_owned(),
                options,
            });
            info
        });
    let untouched = lumit_core::fx::builtins::instantiate(PART_NAME)
        .map(|rows| read_instance_info(&rows, offset, true));
    let parts = tree(&scene)
        .into_iter()
        .map(|(k, depth)| {
            let kept = settings
                .parts
                .iter()
                .find(|r| r.custom_name.as_deref() == scene.nodes.get(k).map(|n| n.path.as_str()));
            BridgeModelPart {
                id: part_id(layer.id, k),
                name: scene
                    .nodes
                    .get(k)
                    .map_or_else(String::new, |n| n.name.clone()),
                depth,
                rows: kept.map(|rows| {
                    let mut rows = rows.clone();
                    rows.id = part_id(layer.id, k);
                    read(&rows)
                }),
                skinned: scene.nodes.get(k).is_some_and(|n| n.skin.is_some()),
            }
        })
        .collect();
    Some(BridgeModelRows {
        playback,
        untouched,
        parts,
        moves: lumit_render::model_motion::moves(settings, &scene),
        holds_lights: lumit_render::model_extract::holds_any(&scene),
        lost_take: match &settings.take {
            Take::Named(name) if !scene.clips.iter().any(|c| &c.name == name) => Some(name.clone()),
            _ => None,
        },
    })
}

/// Each node reached from the roots, before its children, with how many parents
/// it has.
fn tree(scene: &ModelScene) -> Vec<(usize, u32)> {
    let mut out = Vec::with_capacity(scene.nodes.len());
    let mut stack: Vec<(usize, u32)> = scene.roots.iter().rev().map(|&n| (n, 0)).collect();
    while let Some((k, depth)) = stack.pop() {
        let Some(node) = scene.nodes.get(k) else {
            continue;
        };
        out.push((k, depth));
        let children = node.children.iter().rev();
        stack.extend(
            children
                .filter(|&&c| scene.nodes.get(c).is_some_and(|n| n.parent == Some(k)))
                .map(|&c| (c, depth + 1)),
        );
    }
    out
}

impl LayerReference {
    /// A Model layer's playback rows and every part's, as staged copies. Empty
    /// until its file has been read, and on any other kind of layer.
    #[frb(sync)]
    pub fn get_model_rows(&self) -> Result<Vec<BridgeEffectInstance>, BridgeError> {
        let layer = self.item()?;
        let Some((settings, scene)) = self.settings_and_scene(&layer)? else {
            return Ok(Vec::new());
        };
        let list = lumit_render::model_motion::all_rows(&layer, settings, &scene);
        Ok(list
            .into_iter()
            .map(|rows| BridgeEffectInstance::new(rows, layer.start_offset.0))
            .collect())
    }

    /// The id of the part of a Model layer's model first met along the ray
    /// from `origin` by `direction`, three numbers each in the layer's own
    /// pixels, as it is at comp frame `at_frame`. `None` where it misses. Asked
    /// once a click.
    #[frb(sync)]
    pub fn get_model_part_at(
        &self,
        origin: Vec<f64>,
        direction: Vec<f64>,
        at_frame: i64,
    ) -> Result<Option<Uuid>, BridgeError> {
        let layer = self.item()?;
        let LayerKind::Model { item, settings } = &layer.kind else {
            return Ok(None);
        };
        let (Ok(origin), Ok(direction)) = (
            <[f64; 3]>::try_from(origin),
            <[f64; 3]>::try_from(direction),
        ) else {
            return Ok(None);
        };
        let doc = self.snapshot()?;
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let pose = crate::api::model::pose_at_frame(&doc, comp, &layer, at_frame);
        let hit = lumit_render::model::hit(
            &doc,
            *item,
            settings,
            (comp.width as f32, comp.height as f32),
            pose.as_ref(),
            origin.map(|v| v as f32),
            direction.map(|v| v as f32),
        );
        Ok(hit.map(|(_, node)| part_id(layer.id, node)))
    }

    /// Where the part with rows `part` of a Model layer is at comp frame
    /// `at_frame`. `None` before its file has been read. Asked while a part is
    /// picked.
    #[frb(sync)]
    pub fn get_model_part_place(
        &self,
        part: Uuid,
        at_frame: i64,
    ) -> Result<Option<BridgeModelPartPlace>, BridgeError> {
        let (doc, layer) = (self.snapshot()?, self.item()?);
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let t = (comp.frame_rate.time_of_frame(at_frame.max(0)))
            .map_err(|_| BridgeError::InvalidTime)?
            .0
            .to_f64();
        let place = lumit_render::model_motion::part_place(&doc, comp, &layer, part, t);
        let numbers = |p: [f32; 3]| p.map(f64::from);
        Ok(place.map(|(pivot, corners)| BridgeModelPartPlace {
            pivot: numbers(pivot).to_vec(),
            corners: corners.iter().flatten().flat_map(|&c| numbers(c)).collect(),
        }))
    }

    /// A Camera or Light layer for each camera and light a Model layer's file
    /// holds, keyed so they follow it, as one undo step. The Key light its first
    /// model brought goes if lights are made and nobody has touched it, as the
    /// file's lights take its place. `camera_name` and `light_name` name one the
    /// file doesn't. Answers how many were made.
    #[frb(sync)]
    pub fn create_layers_from_model(
        &self,
        camera_name: String,
        light_name: String,
    ) -> Result<u32, BridgeError> {
        let (doc, layer) = (self.snapshot()?, self.item()?);
        let comp = doc.comp(self.comp_id).ok_or(BridgeError::InvalidComp)?;
        let made = lumit_render::model_extract::layers_from_model(
            &doc,
            comp,
            &layer,
            (&camera_name, &light_name),
        );
        if made.is_empty() {
            return Ok(0);
        }
        let count = made.len() as u32;
        let lights = made
            .iter()
            .any(|l| matches!(l.kind, LayerKind::Light { .. }));
        let fresh = CompositionReference::key_light(comp);
        let spent = comp.layers.iter().filter(|l| {
            lights
                && l.extra.contains_key(AUTO_KEY)
                && l.kind == fresh.kind
                && l.transform == fresh.transform
                && l.switches == fresh.switches
                && l.parent.is_none()
                && l.effects.is_empty()
        });
        let removed = spent.map(|l| lumit_core::ops::Op::RemoveLayer {
            comp: self.comp_id,
            layer: l.id,
        });
        let added = made
            .into_iter()
            .rev()
            .map(|layer| lumit_core::ops::Op::AddLayer {
                comp: self.comp_id,
                index: 0,
                layer: Box::new(layer),
            });
        let ops = removed.chain(added).collect();
        self.commit(lumit_core::ops::Op::Batch { ops })?;
        Ok(count)
    }

    /// `layer`'s settings and its file's scene once read.
    #[frb(ignore)]
    fn settings_and_scene<'a>(
        &self,
        layer: &'a Layer,
    ) -> Result<Option<(&'a ModelSettings, std::sync::Arc<ModelScene>)>, BridgeError> {
        let LayerKind::Model { item, settings } = &layer.kind else {
            return Ok(None);
        };
        let doc = self.snapshot()?;
        let scene = lumit_render::model::scene_if_read(&doc, *item).flatten();
        Ok(scene.map(|scene| (&**settings, scene)))
    }

    /// The op that keeps a Model layer's rows once `edit` has changed them:
    /// only what differs from the file.
    #[frb(ignore)]
    pub(crate) fn model_rows_op(
        &self,
        edit: impl FnOnce(&mut Vec<EffectInstance>) -> Result<(), BridgeError>,
    ) -> Result<lumit_core::ops::Op, BridgeError> {
        let layer = self.item()?;
        let (settings, scene) = self
            .settings_and_scene(&layer)?
            .ok_or(BridgeError::InvalidEffect)?;
        let mut list = lumit_render::model_motion::all_rows(&layer, settings, &scene);
        edit(&mut list)?;
        let settings = lumit_render::model_motion::with_rows(settings, &scene, &list);
        let LayerKind::Model { item, .. } = layer.kind else {
            return Err(BridgeError::InvalidLayer);
        };
        Ok(lumit_core::ops::Op::SetLayerKind {
            comp: self.comp_id,
            layer: self.layer_id,
            kind: Box::new(LayerKind::Model {
                item,
                settings: Box::new(settings),
            }),
        })
    }
}
