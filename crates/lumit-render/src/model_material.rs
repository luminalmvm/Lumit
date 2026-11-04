//! A Model layer's material rows ([`lumit_core::model_material`]) beside its
//! file: what each starts at, what the layer keeps of them, and what they
//! come to at a moment for the mesh pass.

use std::borrow::Cow;
use std::sync::Arc;

use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Document, EffectInstance, EffectValue, Layer, LayerKind};
use lumit_core::model_layer::ModelSettings;
use lumit_core::model_material::{index_of, material_id, MATCH_NAME};
use lumit_scene::{AlphaMode, Material, ModelScene};
use uuid::Uuid;

use crate::draw::{DofInputDraw, LayerInputDraw};

/// The rows that take a layer's picture, in the order the mesh pass takes
/// them: base colour, emission, opacity and normal.
const PICTURES: [&str; 4] = [
    "base_colour_layer",
    "emission_layer",
    "opacity_layer",
    "normal_layer",
];

/// A scene's materials, or for one with none the default the mesh pass
/// draws it with, so it still has rows to colour.
pub(crate) fn materials_of(scene: &ModelScene) -> Cow<'_, [Material]> {
    match scene.materials.is_empty() {
        true => Cow::Owned(vec![Material::default()]),
        false => Cow::Borrowed(&scene.materials),
    }
}

/// The rows the layer keeps for file material `i`: by the material's name
/// when it's the only one so named, else by its place, so a file whose
/// materials move keeps each one's edits.
pub(crate) fn kept_for<'a>(
    settings: &'a ModelSettings,
    file: &[Material],
    i: usize,
) -> Option<&'a EffectInstance> {
    let name = file
        .get(i)
        .map(|m| m.name.as_str())
        .filter(|n| !n.is_empty());
    let unique = |n: &str| file.iter().filter(|m| m.name == n).count() == 1;
    if let Some(n) = name.filter(|n| unique(n)) {
        let mut kept = settings.materials.iter().flatten();
        if let Some(rows) = kept.find(|k| k.custom_name.as_deref() == Some(n)) {
            return Some(rows);
        }
    }
    // Kept by place only for a material with no name of its own, so a named
    // one the file has dropped doesn't land on whatever sits there now.
    let at = settings.materials.get(i)?.as_ref()?;
    (at.custom_name.is_none() || at.custom_name.as_deref() == name).then_some(at)
}

/// Material `index`'s rows on Model layer `layer` as `m` in its file has
/// them.
fn offered(layer: Uuid, index: usize, m: &Material) -> Option<EffectInstance> {
    let mut rows = lumit_core::fx::builtins::instantiate(MATCH_NAME)?;
    rows.id = material_id(layer, index);
    rows.custom_name = (!m.name.is_empty()).then(|| m.name.clone());
    let colour = |c: [f32; 3]| EffectValue::Colour([c[0], c[1], c[2], 1.0].map(fixed));
    let [r, g, b, a] = m.base_colour;
    for (id, value) in [
        ("base_colour", colour([r, g, b])),
        ("metallic", EffectValue::Float(fixed(m.metallic * 100.0))),
        ("roughness", EffectValue::Float(fixed(m.roughness * 100.0))),
        ("emission_colour", colour(m.emissive)),
        (
            "emission_strength",
            EffectValue::Float(fixed(m.emissive_strength)),
        ),
        ("opacity", EffectValue::Float(fixed(a * 100.0))),
        ("normal_strength", EffectValue::Float(fixed(m.normal_scale))),
        ("textures", EffectValue::Bool(true)),
        (
            "transmission",
            EffectValue::Float(fixed(m.transmission * 100.0)),
        ),
        ("ior", EffectValue::Float(fixed(m.ior))),
        ("clearcoat", EffectValue::Float(fixed(m.clearcoat * 100.0))),
        (
            "clearcoat_roughness",
            EffectValue::Float(fixed(m.clearcoat_roughness * 100.0)),
        ),
    ] {
        if let Some(p) = rows.params.iter_mut().find(|p| p.id == id) {
            p.value = value;
        }
    }
    Some(rows)
}

fn fixed(v: f32) -> lumit_core::anim::Property {
    lumit_core::anim::Property::fixed(f64::from(v))
}

/// Every material's rows on Model layer `layer`, in the file's order: the
/// ones it keeps, and the file's own values for the rest. `None` while its
/// file hasn't been read, which this starts.
pub fn rows(doc: &Document, layer: &Layer) -> Option<Vec<EffectInstance>> {
    let LayerKind::Model { item, settings } = &layer.kind else {
        return Some(Vec::new());
    };
    let scene = crate::model::scene_if_read(doc, *item)?;
    Some(scene.map_or_else(Vec::new, |s| rows_of(layer.id, settings, &s)))
}

/// Material `id`'s rows on Model layer `layer` as its file has them, which
/// a preset and a Reset start from.
pub fn file_rows(doc: &Document, layer: &Layer, id: Uuid) -> Option<EffectInstance> {
    let LayerKind::Model { item, .. } = &layer.kind else {
        return None;
    };
    let scene = crate::model::scene_if_read(doc, *item)??;
    let i = index_of(layer.id, id)?;
    offered(layer.id, i, materials_of(&scene).get(i)?)
}

fn rows_of(layer: Uuid, settings: &ModelSettings, scene: &ModelScene) -> Vec<EffectInstance> {
    let file = materials_of(scene);
    (file.iter().enumerate())
        .filter_map(|(i, m)| match kept_for(settings, &file, i) {
            // Named for its layer and its file as they are now, whatever
            // layer it was first kept on.
            Some(kept) => {
                let mut rows = EffectInstance {
                    id: material_id(layer, i),
                    custom_name: (!m.name.is_empty()).then(|| m.name.clone()),
                    ..kept.clone()
                };
                // Rows kept before a row was added get it at its default.
                lumit_core::fx::builtins::backfill_builtin_params(std::slice::from_mut(&mut rows));
                Some(rows)
            }
            None => offered(layer, i, m),
        })
        .collect()
}

/// `list`, a Model layer's rows as [`rows`] gave them and since edited, as
/// the layer keeps them: only the materials that differ from the file.
/// `None` while its file hasn't been read, or for rows that aren't this
/// layer's.
pub fn kept(
    doc: &Document,
    layer: &Layer,
    list: &[EffectInstance],
) -> Option<Vec<Option<EffectInstance>>> {
    let LayerKind::Model { item, .. } = &layer.kind else {
        return None;
    };
    let scene = crate::model::scene_if_read(doc, *item)??;
    kept_of(layer.id, &scene, list)
}

fn kept_of(
    layer: Uuid,
    scene: &ModelScene,
    list: &[EffectInstance],
) -> Option<Vec<Option<EffectInstance>>> {
    let file = materials_of(scene);
    let mut out: Vec<Option<EffectInstance>> = vec![None; file.len()];
    for rows in list {
        let i = index_of(layer, rows.id)?;
        let (Some(m), Some(slot)) = (file.get(i), out.get_mut(i)) else {
            return None;
        };
        let file = offered(layer, i, m)?;
        if rows.params != file.params || !rows.enabled {
            *slot = Some(rows.clone());
        }
    }
    while out.last().is_some_and(Option::is_none) {
        out.pop();
    }
    Some(out)
}

/// Whether staged `list` is Model layer `layer`'s rows.
#[must_use]
pub fn owns(layer: &Layer, list: &[EffectInstance]) -> bool {
    matches!(layer.kind, LayerKind::Model { .. })
        && list
            .first()
            .is_some_and(|rows| index_of(layer.id, rows.id).is_some())
}

/// Lay staged `list`, Model layer `layer`'s rows in the file's order, over
/// what it keeps, for a preview of an edit not yet made.
pub fn stage(layer: &mut Layer, list: Vec<EffectInstance>) {
    if let LayerKind::Model { settings, .. } = &mut layer.kind {
        settings.materials = list.into_iter().map(Some).collect();
    }
}

/// One material as its rows have it at a moment, and the pictures its
/// layer rows name ([`PICTURES`]).
pub(crate) struct Look {
    pub index: usize,
    pub material: Material,
    pub pictures: Vec<LayerInputDraw>,
    /// Whether the file's base colour and metal-rough pictures still show.
    pub textures: bool,
    /// Where its base colour is seen from, when it isn't on the model's uvs.
    pub projector: Option<crate::projection::Projector>,
    /// The look whose picture it projects too, in place of one of its own.
    pub plate_from: Option<usize>,
}

/// Every material of `scene` that a Model layer's `settings` change, or that
/// is glass, at layer time `lt`. `picture` resolves a row naming a layer, as
/// an effect's layer rows are.
pub(crate) fn looks(
    settings: &ModelSettings,
    scene: &ModelScene,
    lt: f64,
    context: &Arc<ExpressionContext>,
    picture: &dyn Fn(&EffectInstance, &str) -> Option<DofInputDraw>,
) -> Vec<Look> {
    let read =
        |p: &lumit_core::anim::Property| p.value_at_with_context(lt, Arc::clone(context)) as f32;
    let mut out = Vec::new();
    let materials = materials_of(scene);
    for (index, file) in materials.iter().enumerate() {
        let rows = kept_for(settings, &materials, index);
        let Some(rows) = rows.filter(|r| r.enabled) else {
            if file.transmission > 0.0 {
                let material = file.clone();
                out.push(Look {
                    index,
                    material,
                    pictures: Vec::new(),
                    textures: true,
                    projector: None,
                    plate_from: None,
                });
            }
            continue;
        };
        let float = |id: &str| match rows.param(id) {
            Some(EffectValue::Float(p)) => Some(read(p)),
            _ => None,
        };
        let colour = |id: &str| match rows.param(id) {
            Some(EffectValue::Colour(c)) => Some([read(&c[0]), read(&c[1]), read(&c[2])]),
            _ => None,
        };
        let share = |id: &str| float(id).map(|v| (v / 100.0).clamp(0.0, 1.0));
        let mut m = file.clone();
        if let Some([r, g, b]) = colour("base_colour") {
            m.base_colour = [r, g, b, m.base_colour[3]];
        }
        m.base_colour[3] = share("opacity").unwrap_or(m.base_colour[3]);
        m.metallic = share("metallic").unwrap_or(m.metallic);
        m.roughness = share("roughness").unwrap_or(m.roughness);
        m.emissive = colour("emission_colour").unwrap_or(m.emissive);
        m.emissive_strength =
            float("emission_strength").map_or(m.emissive_strength, |v| v.max(0.0));
        m.normal_scale = float("normal_strength").unwrap_or(m.normal_scale);
        m.transmission = share("transmission").unwrap_or(m.transmission);
        m.ior = float("ior").map_or(m.ior, |v| v.max(1.0));
        m.clearcoat = share("clearcoat").unwrap_or(m.clearcoat);
        m.clearcoat_roughness = share("clearcoat_roughness").unwrap_or(m.clearcoat_roughness);
        let pictures: Vec<LayerInputDraw> = PICTURES
            .iter()
            .map(|row| picture(rows, row).map_or(LayerInputDraw::Absent, LayerInputDraw::Layer))
            .collect();
        // A layer glowing on a material whose file glows black shows as it is.
        if matches!(pictures.get(1), Some(LayerInputDraw::Layer(_))) && m.emissive == [0.0; 3] {
            m.emissive = [1.0; 3];
        }
        // Seen through anywhere, it blends.
        let opacity_picture = matches!(pictures.get(2), Some(LayerInputDraw::Layer(_)));
        if m.alpha_mode == AlphaMode::Opaque && (m.base_colour[3] < 1.0 || opacity_picture) {
            m.alpha_mode = AlphaMode::Blend;
        }
        out.push(Look {
            index,
            material: m,
            pictures,
            textures: !matches!(rows.param("textures"), Some(EffectValue::Bool(false))),
            projector: None,
            plate_from: None,
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// An edited material is kept with its keys, an untouched one isn't, and
    /// the kept rows come back as they were edited, for the mesh pass too.
    #[test]
    fn only_a_changed_material_is_kept() {
        let layer = Uuid::now_v7();
        let scene = ModelScene {
            materials: vec![
                Material {
                    name: "Visor".into(),
                    roughness: 0.25,
                    ..Material::default()
                },
                Material::default(),
            ],
            ..ModelScene::default()
        };
        let mut settings = ModelSettings::default();
        let mut rows = rows_of(layer, &settings, &scene);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].custom_name.as_deref(), Some("Visor"));
        assert_eq!(kept_of(layer, &scene, &rows), Some(Vec::new()));

        let p = rows[1]
            .params
            .iter_mut()
            .find(|p| p.id == "roughness")
            .unwrap();
        p.value = EffectValue::Float(lumit_core::anim::Property::fixed(80.0));
        settings.materials = kept_of(layer, &scene, &rows).unwrap();
        assert!(settings.materials[0].is_none() && settings.materials[1].is_some());
        assert_eq!(rows_of(layer, &settings, &scene), rows);

        let context = Arc::new(ExpressionContext::detached());
        let seen = looks(&settings, &scene, 0.0, &context, &|_, _| None);
        assert_eq!(seen.len(), 1);
        assert_eq!((seen[0].index, seen[0].material.roughness), (1, 0.8));

        // A named material keeps its edits when the file moves it.
        let mut named = scene.clone();
        named.materials[1].name = "Body".into();
        let mut rows = rows_of(layer, &ModelSettings::default(), &named);
        let p = rows[1]
            .params
            .iter_mut()
            .find(|p| p.id == "roughness")
            .unwrap();
        p.value = EffectValue::Float(lumit_core::anim::Property::fixed(80.0));
        settings.materials = kept_of(layer, &named, &rows).unwrap();
        named.materials.swap(0, 1);
        let looks = looks(&settings, &named, 0.0, &context, &|_, _| None);
        assert_eq!(
            (looks[0].index, looks[0].material.name.as_str()),
            (0, "Body")
        );
    }
}
