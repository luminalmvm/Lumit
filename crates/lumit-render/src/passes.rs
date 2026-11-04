//! What a 3D precomp hands to the effects that read its passes, as the build
//! and the bridge see it.

use crate::gpufx::{AuxSlot, GpuEffect};
use lumit_core::fx::effects::scene_pass::{
    parts_of as parts_of_text, PartMatte, PassKind, ScenePass, SceneRead,
};
use lumit_core::fx::{params::Params, registry::EffectMetadata};
use lumit_core::model::{Composition, Document, EffectInstance, EffectValue, Layer, LayerKind};
use lumit_gpu::fx::FxEngine;
use lumit_gpu::GpuContext;

/// A Depth row's read: distance from the camera.
pub(crate) fn depth_read() -> Option<SceneRead> {
    Some(SceneRead {
        pass: PassKind::Depth,
        from_camera: false,
        parts: Vec::new(),
    })
}

/// A Scene pass's picture is the pass, once there is one to show.
pub(crate) struct ScenePassFx;
impl GpuEffect for ScenePassFx {
    fn match_name(&self) -> &'static str {
        <ScenePass as EffectMetadata>::SCHEMA.match_name
    }

    fn run(
        &self,
        _fx: &FxEngine,
        _ctx: &GpuContext,
        tex: &wgpu::Texture,
        _w: u32,
        _h: u32,
        _p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> wgpu::Texture {
        aux.layer_input().unwrap_or(tex).clone()
    }
}

/// A Part matte is Set matte with a matte from the scene.
pub(crate) struct PartMatteFx;
impl GpuEffect for PartMatteFx {
    fn match_name(&self) -> &'static str {
        <PartMatte as EffectMetadata>::SCHEMA.match_name
    }

    fn run(
        &self,
        fx: &FxEngine,
        ctx: &GpuContext,
        tex: &wgpu::Texture,
        w: u32,
        h: u32,
        p: Params<'_>,
        aux: AuxSlot<'_>,
    ) -> wgpu::Texture {
        let part = PartMatte::read(p);
        fx.set_matte(
            ctx,
            tex,
            w,
            h,
            aux.layer_input(),
            &lumit_gpu::fx::utility::SetMatteOp {
                channel: 0,
                combine: true,
                invert: part.invert,
                mix: (part.mix / 100.0).clamp(0.0, 1.0),
            },
        )
    }
}

/// Whether `l` is a Precomp whose comp has a 3D layer in it, which is what a
/// row needs to read a distance or a pass from.
pub(crate) fn is_3d_precomp(doc: &Document, l: &Layer) -> bool {
    match l.kind {
        LayerKind::Precomp { comp } => doc
            .comp(comp)
            .is_some_and(|c| c.layers.iter().any(|l| l.switches.three_d)),
        _ => false,
    }
}

/// The parts of a Model layer, or of the 3D layers in the comp a Precomp layer
/// holds, top layer first: each one's number and the name to show. `None` for
/// any other layer. A model that hasn't been read yet is read now.
#[must_use]
pub fn scene_parts(doc: &Document, l: &Layer) -> Option<Vec<(u32, String)>> {
    let mut parts = Vec::new();
    match l.kind {
        LayerKind::Model { .. } => parts_of(doc, l, &mut parts),
        LayerKind::Precomp { comp } if is_3d_precomp(doc, l) => doc
            .comp(comp)?
            .layers
            .iter()
            .filter(|l| l.switches.three_d)
            .for_each(|inner| parts_of(doc, inner, &mut parts)),
        _ => return None,
    }
    Some(parts)
}

/// One 3D layer's parts: a model's, or the layer itself.
fn parts_of(doc: &Document, l: &Layer, parts: &mut Vec<(u32, String)>) {
    let id = l.id.as_bytes();
    match &l.kind {
        LayerKind::Model { item, .. } => {
            let Some(scene) = crate::model::scene(doc, *item) else {
                return;
            };
            parts.extend(scene.parts(id, &l.name));
        }
        LayerKind::Camera { .. } | LayerKind::Light { .. } => {}
        _ => parts.push((lumit_scene::parts::layer_number(id), l.name.clone())),
    }
}

/// Fresh ids for comps about to be added as copies, with each Part matte's
/// parts going by their new numbers, as a part's number comes from its layer's
/// id. A pick of a layer outside the copies keeps its number.
pub fn fresh_ids_keeping_parts(doc: &Document, comps: &mut [Composition]) -> Option<()> {
    let before: Vec<Vec<Layer>> = comps.iter().map(|c| c.layers.clone()).collect();
    lumit_core::edits::fresh_ids(comps)?;
    let picks = |c: &Composition| {
        c.layers
            .iter()
            .flat_map(|l| &l.effects)
            .any(|e| e.effect.match_name == "part_matte" && e.text("parts").is_some())
    };
    if !comps.iter().any(picks) {
        return Some(());
    }
    let pairs = before
        .iter()
        .flatten()
        .zip(comps.iter().flat_map(|c| &c.layers));
    let numbers = numbers_between(doc, pairs);
    for e in comps
        .iter_mut()
        .flat_map(|c| &mut c.layers)
        .flat_map(|l| &mut l.effects)
    {
        renumber(e, &numbers);
    }
    Some(())
}

/// A Scene pass or Part matte on `copy` whose Source names `old`, the layer it
/// was copied from, names the copy instead, with its picks going by the copy's
/// numbers. A row naming any other layer rightly keeps naming it.
pub fn follow_copy(doc: &Document, old: &Layer, copy: &mut Layer) {
    let follows = |e: &EffectInstance| {
        matches!(e.effect.match_name.as_str(), "scene_pass" | "part_matte")
            && e.layer_ref("source") == Some(old.id)
    };
    if !copy.effects.iter().any(follows) {
        return;
    }
    let numbers = numbers_between(doc, std::iter::once((old, &*copy)));
    let id = copy.id;
    for e in copy.effects.iter_mut().filter(|e| follows(e)) {
        renumber(e, &numbers);
        for p in e.params.iter_mut().filter(|p| p.id == "source") {
            p.value = EffectValue::Layer(Some(id));
        }
    }
}

/// What each part of the old layers is numbered on the new ones.
fn numbers_between<'a>(
    doc: &Document,
    pairs: impl Iterator<Item = (&'a Layer, &'a Layer)>,
) -> std::collections::HashMap<u32, u32> {
    let mut numbers = std::collections::HashMap::new();
    for (old, new) in pairs {
        let (mut was, mut now) = (Vec::new(), Vec::new());
        parts_of(doc, old, &mut was);
        parts_of(doc, new, &mut now);
        numbers.extend(
            was.into_iter()
                .map(|p| p.0)
                .zip(now.into_iter().map(|p| p.0)),
        );
    }
    numbers
}

/// A Part matte's picks by the numbers in `numbers`.
fn renumber(e: &mut EffectInstance, numbers: &std::collections::HashMap<u32, u32>) {
    if e.effect.match_name != "part_matte" {
        return;
    }
    for p in e.params.iter_mut().filter(|p| p.id == "parts") {
        if let EffectValue::Text(text) = &mut p.value {
            let kept: Vec<String> = parts_of_text(text)
                .into_iter()
                .map(|n| numbers.get(&n).copied().unwrap_or(n).to_string())
                .collect();
            *text = kept.join(" ");
        }
    }
}
