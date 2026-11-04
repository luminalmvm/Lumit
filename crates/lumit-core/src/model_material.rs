//! A Model layer's materials: a group of rows for each material in its 3D
//! model's file, declared the way an effect is, so keyframes, expressions,
//! undo, presets and the panels all come with them.
//!
//! The rows live on the layer and never in the file. A material nobody has
//! changed isn't saved, and its rows show the file's own values.

use crate::fx::presets::{EffectPreset, PresetValue};
use crate::fx::registry::{Catalogue, EffectDef, EffectMetadata};
use crate::fx::schema::EffectSchema;
use crate::model::{EffectInstance, Layer, LayerKind};
use lumit_fx_macros::Effect;
use uuid::Uuid;

/// What [`ModelMaterial`] is known by.
pub const MATCH_NAME: &str = "model_material";

/// The file extension a saved material preset uses, so the effect presets
/// never list one.
pub const PRESET_EXTENSION: &str = "lummat";

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
)]
pub struct ModelMaterial {
    #[colour(default = [1.0, 1.0, 1.0, 1.0], max = 4.0)]
    pub base_colour: [f32; 4],
    /// A layer's picture in place of the file's, on the model's own uvs.
    #[layer(self_default = false, label = "Base colour layer")]
    pub base_colour_layer: bool,
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

    fn is_image_op(&self) -> bool {
        false
    }

    fn presets(&self) -> &'static [EffectPreset] {
        &PRESETS
    }
}

/// Kept apart from the effect catalogue, so no menu offers it as an effect.
/// [`crate::fx::builtins::def`] finds it here.
pub static MATERIAL_DEFS: Catalogue =
    Catalogue::new(&[&ModelMaterialDef, &crate::extrude::GeometryOptionsDef]);

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
    layer
        .effects
        .iter()
        .chain(rows_kept(layer).iter().flatten())
}

/// The rows `layer` keeps: a Model layer's materials, or a Text or Shape
/// layer's Geometry options and faces.
fn rows_kept(layer: &Layer) -> &[Option<EffectInstance>] {
    match &layer.kind {
        LayerKind::Model { settings, .. } => settings.materials.as_slice(),
        _ => crate::extrude::kept(layer),
    }
}

/// The instance `id` names on layer `layer`: one of its effects, or one of
/// its kept material rows by the material it names, whichever layer it was
/// first kept on.
#[must_use]
pub fn instance(layer: &Layer, id: Uuid) -> Option<&EffectInstance> {
    let kept = rows_kept(layer);
    let by_index = index_of(layer.id, id).and_then(|i| kept.get(i)?.as_ref());
    and_effects(layer).find(|e| e.id == id).or(by_index)
}

/// The material rows Model layer `layer` keeps, in file order. Empty on any
/// other layer.
#[must_use]
pub fn stored(layer: &Layer) -> Vec<EffectInstance> {
    rows_kept(layer).iter().flatten().cloned().collect()
}
