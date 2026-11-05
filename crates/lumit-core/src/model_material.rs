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

/// With no camera there is nothing to hold, and with no picture nothing to
/// project.
const ENABLED_WHEN: &[EnabledWhen] = &[
    EnabledWhen {
        param: "projection_time",
        on: "camera",
        cond: EnabledCond::LayerSet,
    },
    EnabledWhen {
        param: "camera",
        on: "base_colour_layer",
        cond: EnabledCond::LayerSet,
    },
];

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
pub static MATERIAL_DEFS: Catalogue = Catalogue::new(&[
    &ModelMaterialDef,
    &crate::extrude::GeometryOptionsDef,
    &crate::mesh_shape::CubeGeometryDef,
    &crate::mesh_shape::SphereGeometryDef,
    &crate::mesh_shape::CylinderGeometryDef,
    &crate::mesh_shape::ConeGeometryDef,
    &crate::mesh_shape::TorusGeometryDef,
    &crate::mesh_shape::PlaneGeometryDef,
]);

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
    let kept = rows_kept(layer).iter().flatten();
    kept.filter_map(|rows| projection(rows, comp, lt))
        .filter_map(|p| Some((p.picture, p.held?)))
        .collect()
}

/// `rows` as a frame's name reads them: a held row's picture isn't what its
/// layer shows now, and [`held_moments`]' frame is what is named instead.
#[must_use]
pub fn unheld(rows: &[EffectInstance], comp: &Composition, lt: f64) -> Vec<EffectInstance> {
    let mut rows = rows.to_vec();
    for r in &mut rows {
        if projection(r, comp, lt).is_some_and(|p| p.held.is_some()) {
            let row = r.params.iter_mut().find(|p| p.id == "base_colour_layer");
            if let Some(row) = row {
                row.value = EffectValue::Layer(None);
            }
        }
    }
    rows
}

/// `times`, the comp time each layer of `comp` is fetched at for a frame at
/// `t`, with a hidden layer that only held projections read moved to the frame
/// they hold, so the one at the playhead isn't decoded for nothing.
#[must_use]
pub fn with_held_frames(comp: &Composition, mut times: Vec<f64>, t: f64) -> Vec<f64> {
    let held = |l: &Layer| held_moments(l, comp, crate::time::layer_time(t, l.start_offset.0));
    let all: Vec<(Uuid, f64)> = comp.layers.iter().flat_map(held).collect();
    if all.is_empty() {
        return times;
    }
    let reads = |id: Uuid, l: &Layer| {
        let matte = usize::from(l.matte.as_ref().is_some_and(|m| m.layer == id));
        let rows = and_effects(l).flat_map(|e| &e.params);
        matte
            + rows
                .filter(|p| p.value == EffectValue::Layer(Some(id)))
                .count()
    };
    for (slot, l) in times.iter_mut().zip(&comp.layers) {
        let mut these = all.iter().filter(|(id, _)| *id == l.id).map(|(_, at)| *at);
        let Some(at) = these.next().filter(|_| !l.switches.visible) else {
            continue;
        };
        let count = 1 + these.clone().count();
        let alone = comp.layers.iter().map(|o| reads(l.id, o)).sum::<usize>() == count;
        if alone && these.all(|other| other.to_bits() == at.to_bits()) {
            *slot = at;
        }
    }
    times
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

/// What Project from camera uses for Model layer `model` of `comp` at comp time
/// `t`, as (camera, picture). The camera is a selected one, else the comp's
/// own, and `None` when it has none to use. The picture is the shot that camera
/// was solved from, else a selected picture, else the nearest below the model.
#[must_use]
pub fn projection_source(
    comp: &Composition,
    model: Uuid,
    selected: &[Uuid],
    t: f64,
) -> Option<(Option<Uuid>, Uuid)> {
    let picked = || {
        let find = |id: &Uuid| comp.layers.iter().find(|l| l.id == *id);
        selected.iter().filter_map(find)
    };
    let is_picture = |l: &&Layer| {
        matches!(
            l.kind,
            LayerKind::Footage { .. } | LayerKind::Precomp { .. }
        )
    };
    let camera = picked()
        .find(|l| matches!(l.kind, LayerKind::Camera { .. }))
        .or_else(|| comp.active_camera(t));
    let solved = camera.and_then(|c| match c.kind {
        LayerKind::Camera { solve_link, .. } => solve_link,
        _ => None,
    });
    let below = comp.layers.iter().skip_while(|l| l.id != model).skip(1);
    let picture = solved
        .or_else(|| picked().find(is_picture).map(|l| l.id))
        .or_else(|| below.clone().find(is_picture).map(|l| l.id))?;
    Some((camera.map(|c| c.id), picture))
}

/// Set material rows `rows` to project `picture` from `camera`, held at comp
/// frame `hold` or following the playhead.
pub fn set_projection(rows: &mut EffectInstance, picture: Uuid, camera: Uuid, hold: Option<i64>) {
    for p in &mut rows.params {
        match p.id.as_str() {
            "base_colour_layer" => p.value = EffectValue::Layer(Some(picture)),
            "camera" => p.value = EffectValue::Layer(Some(camera)),
            "projection_time" => p.value = EffectValue::Choice(u32::from(hold.is_some())),
            "hold_frame" => {
                if let Some(frame) = hold {
                    p.value = EffectValue::Float(crate::anim::Property::fixed(frame as f64));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::edits::{base_layer, new_comp_ops, CompSettings};
    use crate::model::Document;
    use crate::ops::apply;

    /// A comp holding a Model layer holding rows, over two pictures, with a
    /// camera above them, each named as the test wants it.
    fn set(hold: bool) -> (Composition, [Uuid; 4]) {
        let mut doc = Document::new();
        let (id, ops) = new_comp_ops(&doc, CompSettings::default(), None);
        for op in &ops {
            apply(&mut doc, op).unwrap();
        }
        let mut comp = doc.comp(id).unwrap().clone();
        let layer = |kind| base_layer("x".into(), kind, comp.duration.0, Default::default());
        let mut near = layer(LayerKind::Footage {
            item: Uuid::now_v7(),
        });
        near.switches.visible = false;
        let far = layer(LayerKind::Footage {
            item: Uuid::now_v7(),
        });
        let camera = crate::edits::fresh_camera(&comp, "Camera".into(), None);
        let mut rows = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        set_projection(&mut rows, near.id, camera.id, hold.then_some(6));
        let mut model = layer(LayerKind::Model {
            item: Uuid::now_v7(),
            settings: Box::new(crate::model_layer::ModelSettings {
                materials: vec![Some(rows)],
                ..Default::default()
            }),
        });
        model.switches.three_d = true;
        let ids = [camera.id, model.id, near.id, far.id];
        comp.layers = vec![camera, model, near, far];
        (comp, ids)
    }

    /// Project from camera takes a selected camera and picture over the comp's
    /// own, and a solve's shot over either.
    #[test]
    fn project_from_camera_picks_what_it_should() {
        let (mut comp, [camera, model, near, far]) = set(false);
        let pick =
            |comp: &Composition, selected: &[Uuid]| projection_source(comp, model, selected, 0.0);
        assert_eq!(pick(&comp, &[]), Some((Some(camera), near)));
        assert_eq!(pick(&comp, &[far]), Some((Some(camera), far)));
        if let LayerKind::Camera { solve_link, .. } = &mut comp.layers[0].kind {
            *solve_link = Some(far);
        }
        assert_eq!(pick(&comp, &[near]), Some((Some(camera), far)));
        comp.layers.remove(0);
        assert_eq!(pick(&comp, &[near]), Some((None, near)), "one is made");
        comp.layers.retain(|l| l.id == model);
        assert_eq!(pick(&comp, &[]), None, "no picture, nothing to project");
    }

    /// A hidden picture only a held projection reads is fetched at the frame
    /// it holds, and named by that frame alone.
    #[test]
    fn a_held_picture_is_read_at_the_held_frame() {
        let (comp, [_, _, near, far]) = set(true);
        let rate = comp.frame_rate;
        let held = rate.time_of_frame(6).unwrap().0.to_f64();
        let rows = stored(&comp.layers[1]);
        let p = projection(&rows[0], &comp, 1.0).unwrap();
        assert_eq!(
            (p.picture, p.held, p.time_at(1.0)),
            (near, Some(held), held)
        );
        assert_eq!(
            held_moments(&comp.layers[1], &comp, 1.0),
            vec![(near, held)]
        );
        let times = with_held_frames(&comp, vec![1.0; 4], 1.0);
        assert_eq!(times, vec![1.0, 1.0, held, 1.0]);
        let named = unheld(&rows, &comp, 1.0);
        assert_eq!(named[0].layer_ref("base_colour_layer"), None);
        assert_eq!(named[0].layer_ref("camera"), rows[0].layer_ref("camera"));
        assert_ne!(far, near);

        // Read by anything else too, it's fetched at the playhead after all.
        let mut read_twice = comp.clone();
        let mut other = crate::fx::builtins::instantiate(MATCH_NAME).unwrap();
        set_projection(&mut other, near, near, None);
        read_twice.layers[3].effects.push(other);
        assert_eq!(
            with_held_frames(&read_twice, vec![1.0; 4], 1.0),
            vec![1.0; 4]
        );
    }
}
