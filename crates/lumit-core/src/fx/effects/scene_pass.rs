//! Scene pass and Part matte: what a 3D precomp knows about its pixels, handed
//! to the layer's effects. Their Source row names the precomp as a Depth row
//! does, and the render makes what they ask for in its place. Any other layer
//! has no such thing to give, so the row does nothing there.

use crate::fx::{
    params::ShortText, registry::EffectDef, registry::EffectMetadata, schema::EffectSchema,
};
use crate::model::{EffectInstance, EffectValue};
use lumit_fx_macros::Effect;

/// What a 3D precomp hands over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassKind {
    /// Which way each pixel faces, as colour.
    Normal,
    /// Where each pixel is, metres.
    Position,
    /// Each part's own flat colour, which is its number.
    PartId,
    /// How far each pixel is from the camera, metres.
    Depth,
    /// White over the parts named, with soft edges.
    Matte,
}

/// What a row reads from a 3D precomp in place of its picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneRead {
    pub pass: PassKind,
    /// Normal and Position from the camera's point of view, not the comp's.
    pub from_camera: bool,
    /// The parts a matte shows.
    pub parts: Vec<u32>,
}

impl SceneRead {
    /// Whether it has nothing to read: a matte of no parts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pass == PassKind::Matte && self.parts.is_empty()
    }
}

/// A Part matte's parts as its row keeps them, numbers with spaces between.
#[must_use]
pub fn parts_of(text: &str) -> Vec<u32> {
    text.split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect()
}

fn choice(inst: &EffectInstance, id: &str) -> u32 {
    match inst.param(id) {
        Some(EffectValue::Choice(v)) => *v,
        _ => 0,
    }
}

/// Scene pass's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "scene_pass",
    label = "Scene pass",
    version = 1,
    category = Utility,
    cost = Moderate,
    roi = Exact,
    matte = false,
)]
pub struct ScenePass {
    #[choice(options = ["Normals", "Position", "Part id", "Depth"], default = 0)]
    pub pass: u32,

    /// Whose point of view Normal and Position are from.
    #[choice(options = ["Comp", "Camera"], default = 0)]
    pub space: u32,

    #[layer(label = "Source", self_default = true)]
    pub source: bool,
}

pub struct ScenePassDef;

impl EffectDef for ScenePassDef {
    fn schema(&self) -> &'static EffectSchema {
        &<ScenePass as EffectMetadata>::SCHEMA
    }

    fn scene_read(&self, inst: &EffectInstance) -> Option<SceneRead> {
        Some(SceneRead {
            pass: match choice(inst, "pass") {
                1 => PassKind::Position,
                2 => PassKind::PartId,
                3 => PassKind::Depth,
                _ => PassKind::Normal,
            },
            from_camera: choice(inst, "space") == 1,
            parts: Vec::new(),
        })
    }
}

/// Part matte's controls.
#[derive(Debug, Clone, Copy, PartialEq, Effect)]
#[effect(
    match_name = "part_matte",
    label = "Part matte",
    version = 1,
    category = Utility,
    cost = Moderate,
    roi = Exact,
    // Coverage alone changes, as for Set matte.
    premultiplied = false,
    matte = false,
)]
pub struct PartMatte {
    /// The parts it keeps, filled in by picking them in the Viewer.
    #[text(label = "Parts", default = "")]
    pub parts: ShortText,

    #[toggle(label = "Invert", default = false)]
    pub invert: bool,

    #[slider(
        min = 0.0,
        max = 100.0,
        default = 100.0,
        hard_min = 0.0,
        hard_max = 100.0,
        unit = Percent
    )]
    pub mix: f32,

    #[layer(label = "Source", self_default = true)]
    pub source: bool,
}

pub struct PartMatteDef;

impl EffectDef for PartMatteDef {
    fn schema(&self) -> &'static EffectSchema {
        &<PartMatte as EffectMetadata>::SCHEMA
    }

    fn scene_read(&self, inst: &EffectInstance) -> Option<SceneRead> {
        Some(SceneRead {
            pass: PassKind::Matte,
            from_camera: false,
            parts: parts_of(inst.text("parts").unwrap_or_default()),
        })
    }
}
