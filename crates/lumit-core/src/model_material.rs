//! A Model layer's materials: a group of rows for each material in its 3D
//! model's file, declared the way an effect is, so keyframes, expressions,
//! undo, presets and the panels all come with them.
//!
//! The rows live on the layer and never in the file. A material nobody has
//! changed isn't saved, and its rows show the file's own values.

use crate::fx::presets::{EffectPreset, PresetValue};
use crate::fx::registry::{Catalogue, EffectDef, EffectMetadata};
use crate::fx::schema::{EffectSchema, EnabledCond, EnabledWhen, ParamGroup};
use crate::model::{
    CameraPose, Composition, Document, EffectInstance, EffectValue, Layer, LayerKind,
};
use lumit_fx_macros::Effect;
use std::sync::Arc;
use uuid::Uuid;

/// What [`ModelMaterial`] is known by.
pub const MATCH_NAME: &str = "model_material";

/// The file extension a saved material preset uses, so the effect presets
/// never list one.
pub const PRESET_EXTENSION: &str = "lummat";

/// Whether a projection follows the playhead or stops at its Hold frame.
const PROJECTION_TIME: &[&str] = &["Follow", "Hold"];

/// Hold frame only shows while the projection holds.
const GROUPS: &[ParamGroup] = &[ParamGroup {
    label: "",
    params: &["hold_frame"],
    collapsed: false,
    visible_when: Some(("projection_time", &[1])),
    visible_when_lens_elements: None,
}];

/// With no camera there is nothing to hold.
const ENABLED_WHEN: &[EnabledWhen] = &[EnabledWhen {
    param: "projection_time",
    on: "camera",
    cond: EnabledCond::LayerSet,
}];

/// One material's rows.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "model_material",
    label = "Material",
    version = 1,
    category = Utility,
    cost = Trivial,
    roi = Exact,
    matte = false,
    groups = GROUPS,
    enabled_when = ENABLED_WHEN,
)]
pub struct ModelMaterial {
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub base_colour: [f32; 4],
    /// A layer's picture in place of the file's, on the model's own uvs or
    /// seen from the camera in Projected from.
    #[layer(self_default = false, label = "Base colour layer")]
    pub base_colour_layer: bool,
    /// The Camera layer the base colour layer is projected from. Left empty,
    /// it goes on through the model's uvs.
    #[layer(self_default = false, label = "Projected from")]
    pub camera: bool,
    #[choice(label = "Projection time", options = *PROJECTION_TIME, default = 0)]
    pub projection_time: u32,
    /// The comp frame the camera and its picture stop at.
    #[counter(min = 0, max = 300, default = 0, hard_min = 0, unit = Frames, label = "Hold frame")]
    pub hold_frame: i32,
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub metallic: f32,
    #[slider(min = 0.0, max = 100.0, default = 50.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub roughness: f32,
    #[colour(default = [0.0, 0.0, 0.0, 1.0], max = 4.0)]
    pub emission_colour: [f32; 4],
    #[slider(min = 0.0, max = 20.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub emission_strength: f32,
    #[layer(self_default = false, label = "Emission layer")]
    pub emission_layer: bool,
    #[slider(min = 0.0, max = 100.0, default = 100.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub opacity: f32,
    /// Its brightness is how much shows.
    #[layer(self_default = false, label = "Opacity layer")]
    pub opacity_layer: bool,
    #[slider(min = 0.0, max = 2.0, default = 1.0, hard_min = 0.0, unit = Raw)]
    pub normal_strength: f32,
    /// A layer's picture as the normal map.
    #[layer(self_default = false, label = "Normal layer")]
    pub normal_layer: bool,
    /// Whether the file's base colour and metal-rough pictures still show
    /// under the numbers above.
    #[toggle(default = true)]
    pub textures: bool,
    /// How much of what is behind shows through, bent by IOR.
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent)]
    pub transmission: f32,
    #[slider(min = 1.0, max = 2.5, default = 1.5, hard_min = 1.0, hard_max = 3.0, unit = Raw, label = "IOR")]
    pub ior: f32,
    /// A clear varnish over the rest, with a highlight of its own.
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent, label = "Coat")]
    pub clearcoat: f32,
    #[slider(min = 0.0, max = 100.0, default = 0.0, hard_min = 0.0, hard_max = 100.0, unit = Percent, label = "Coat roughness")]
    pub clearcoat_roughness: f32,
}

/// The material's behaviour: none of its own, the mesh pass reads it.
pub struct ModelMaterialDef;

impl EffectDef for ModelMaterialDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ModelMaterial as EffectMetadata>::SCHEMA
    }

    /// Projected from is read for where the camera is, never for a picture.
    fn shape_inputs(&self) -> &'static [&'static str] {
        &["camera"]
    }

    fn is_image_op(&self) -> bool {
        false
    }

    fn presets(&self) -> &'static [EffectPreset] {
        &PRESETS
    }
}

/// Kept apart from the effect catalogue, so no menu offers it as an effect.
/// [`crate::fx::builtins::def`] finds it here.
pub static MATERIAL_DEFS: Catalogue = Catalogue::new(&[&ModelMaterialDef]);

use PresetValue::{Bool, Colour, Float};

const PRESETS: [EffectPreset; 8] = [
    EffectPreset {
        name: "Chrome",
        values: &[
            ("textures", Bool(false)),
            ("base_colour", Colour([0.95, 0.95, 0.95, 1.0])),
            ("metallic", Float(100.0)),
            ("roughness", Float(5.0)),
        ],
    },
    EffectPreset {
        name: "Gold",
        values: &[
            ("textures", Bool(false)),
            ("base_colour", Colour([1.0, 0.71, 0.29, 1.0])),
            ("metallic", Float(100.0)),
            ("roughness", Float(20.0)),
        ],
    },
    EffectPreset {
        name: "Brushed metal",
        values: &[
            ("textures", Bool(false)),
            ("base_colour", Colour([0.8, 0.8, 0.82, 1.0])),
            ("metallic", Float(100.0)),
            ("roughness", Float(45.0)),
        ],
    },
    EffectPreset {
        name: "Glass",
        values: &[
            ("textures", Bool(false)),
            ("metallic", Float(0.0)),
            ("roughness", Float(0.0)),
            ("transmission", Float(100.0)),
        ],
    },
    EffectPreset {
        name: "Frosted glass",
        values: &[
            ("textures", Bool(false)),
            ("metallic", Float(0.0)),
            ("roughness", Float(40.0)),
            ("transmission", Float(100.0)),
        ],
    },
    EffectPreset {
        name: "Plastic",
        values: &[
            ("textures", Bool(false)),
            ("metallic", Float(0.0)),
            ("base_colour", Colour([0.6, 0.05, 0.05, 1.0])),
            ("roughness", Float(35.0)),
            ("clearcoat", Float(60.0)),
            ("clearcoat_roughness", Float(10.0)),
        ],
    },
    EffectPreset {
        name: "Rubber",
        values: &[
            ("textures", Bool(false)),
            ("metallic", Float(0.0)),
            ("base_colour", Colour([0.04, 0.04, 0.04, 1.0])),
            ("roughness", Float(90.0)),
        ],
    },
    EffectPreset {
        name: "Emissive",
        values: &[
            ("textures", Bool(false)),
            ("metallic", Float(0.0)),
            ("base_colour", Colour([0.05, 0.05, 0.05, 1.0])),
            ("emission_colour", Colour([1.0, 0.85, 0.6, 1.0])),
            ("emission_strength", Float(4.0)),
        ],
    },
];

/// The low half of every material's id is its number, the rest a fixed mark
/// over its layer's id, so the id says which material of which layer it is.
const MARK: u128 = 0x6d61_7465_7269_616c_7320_6f66_0000_0000;

/// The id of material `index`'s rows on Model layer `layer`.
#[must_use]
pub fn material_id(layer: Uuid, index: usize) -> Uuid {
    Uuid::from_u128(layer.as_u128() ^ (MARK | (index as u128 + 1)))
}

/// Which material of Model layer `layer` the rows `id` are, or `None` for
/// rows of anything else.
#[must_use]
pub fn index_of(layer: Uuid, id: Uuid) -> Option<usize> {
    let x = layer.as_u128() ^ id.as_u128();
    let n = usize::try_from(x & 0xffff_ffff).ok()?;
    (x & !0xffff_ffff == MARK).then(|| n.checked_sub(1))?
}

/// Layer `layer`'s effects, then the material rows it keeps when it's a Model
/// layer: what an expression can name.
pub fn and_effects(layer: &Layer) -> impl Iterator<Item = &EffectInstance> {
    let kept = match &layer.kind {
        LayerKind::Model { settings, .. } => settings.materials.as_slice(),
        _ => &[],
    };
    layer.effects.iter().chain(kept.iter().flatten())
}

/// The instance `id` names on layer `layer`: one of its effects, or one of
/// its kept material rows by the material it names, whichever layer it was
/// first kept on.
#[must_use]
pub fn instance(layer: &Layer, id: Uuid) -> Option<&EffectInstance> {
    let kept = match &layer.kind {
        LayerKind::Model { settings, .. } => settings.materials.as_slice(),
        _ => &[],
    };
    let by_index = index_of(layer.id, id).and_then(|i| kept.get(i)?.as_ref());
    and_effects(layer).find(|e| e.id == id).or(by_index)
}

/// The material rows Model layer `layer` keeps, in file order. Empty on any
/// other layer.
#[must_use]
pub fn stored(layer: &Layer) -> Vec<EffectInstance> {
    match &layer.kind {
        LayerKind::Model { settings, .. } => settings.materials.iter().flatten().cloned().collect(),
        _ => Vec::new(),
    }
}

/// Where a material's base colour layer is projected from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projection {
    /// The Camera layer.
    pub camera: Uuid,
    /// The layer it projects.
    pub picture: Uuid,
    /// The comp time it holds at, `None` when it follows the playhead.
    pub held: Option<f64>,
}

impl Projection {
    /// The comp time the camera and its picture are read at, for a frame at `t`.
    #[must_use]
    pub fn time_at(&self, t: f64) -> f64 {
        self.held.unwrap_or(t)
    }
}

/// What rows `rows` of a material in `comp` project at layer time `lt`. `None`
/// for rows that read through the model's uvs, and for a row naming anything
/// but one of `comp`'s Camera layers.
#[must_use]
pub fn projection(rows: &EffectInstance, comp: &Composition, lt: f64) -> Option<Projection> {
    if !rows.enabled {
        return None;
    }
    let camera = rows.layer_ref("camera")?;
    let picture = rows.layer_ref("base_colour_layer")?;
    let is_camera = |l: &Layer| l.id == camera && matches!(l.kind, LayerKind::Camera { .. });
    comp.layers.iter().find(|l| is_camera(l))?;
    let holds = matches!(rows.param("projection_time"), Some(EffectValue::Choice(1)));
    let frame = match rows.param("hold_frame") {
        Some(EffectValue::Float(f)) if holds => f.value_at(lt).round().max(0.0) as i64,
        _ => 0,
    };
    let held = holds
        .then(|| comp.frame_rate.time_of_frame(frame).ok())
        .flatten()
        .map(|t| t.0.to_f64());
    Some(Projection {
        camera,
        picture,
        held,
    })
}

/// Each layer Model layer `layer` holds a projection of and the comp time it
/// holds at, so that frame of it is fetched. Empty on any other layer.
#[must_use]
pub fn held_moments(layer: &Layer, comp: &Composition, lt: f64) -> Vec<(Uuid, f64)> {
    let LayerKind::Model { settings, .. } = &layer.kind else {
        return Vec::new();
    };
    let kept = settings.materials.iter().flatten();
    kept.filter_map(|rows| projection(rows, comp, lt))
        .filter_map(|p| Some((p.picture, p.held?)))
        .collect()
}

/// Camera layer `layer`'s placement at comp time `t` as a picture is drawn
/// with it, its solve and parents followed.
#[must_use]
pub fn camera_at(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &Layer,
    t: f64,
) -> Option<CameraPose> {
    let store = crate::track::solve_store().unwrap_or(&crate::track::Unsolved);
    crate::track::camera_pose_of(doc, comp, layer, t, store).map(|linked| linked.pose)
}
