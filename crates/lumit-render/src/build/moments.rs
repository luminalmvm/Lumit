//! The pictures an effect that shows its own input at other moments reads
//! (`lumit_core::fx::moments`): the layer built again at each moment, as the
//! effects above it leave it, or on an adjustment layer the composite below.

use std::collections::HashMap;
use std::sync::Arc;

use lumit_core::model::{Composition, Document, EffectInstance};
use uuid::Uuid;

use super::{below_draws_cloned, comp_walk, Probe, Walk};
use crate::decode::CompLayerPixels;
use crate::draw::{DofInputDraw, LayerInputDraw, NestedInputDraw};

/// The composite below layer `idx` of `comp` at comp time `t` (with `frame_t`
/// the frame's own), as a comp-sized picture.
pub(super) fn below_picture(
    doc: &Arc<Document>,
    comp: &Composition,
    idx: usize,
    (t, frame_t): (f64, f64),
    frame_pixels: &HashMap<Uuid, &CompLayerPixels>,
    visited: &[Uuid],
    clone: (u32, u32),
) -> DofInputDraw {
    let below = comp.layers.get(idx + 1..).unwrap_or(&[]);
    let (draws, camera) = below_draws_cloned(
        doc,
        comp,
        below,
        t,
        frame_t,
        None,
        frame_pixels,
        &mut visited.to_vec(),
        clone,
        None,
    );
    picture_of(comp, draws, camera, t)
}

/// `draws` as a comp-sized picture.
pub(super) fn picture_of(
    comp: &Composition,
    draws: Vec<crate::draw::CompLayerDraw>,
    camera: Option<lumit_core::model::CameraPose>,
    t: f64,
) -> DofInputDraw {
    DofInputDraw {
        rgba: Vec::new(),
        tex_w: comp.width,
        tex_h: comp.height,
        format: lumit_media::decode::PixelFormat::Srgb8,
        fx: Default::default(),
        colour_tables: Vec::new(),
        flare_lens_files: Vec::new(),
        roto_mattes: Vec::new(),
        planes: Vec::new(),
        nested: Some(Box::new(NestedInputDraw {
            width: comp.width,
            height: comp.height,
            background: [0.0, 0.0, 0.0, 0.0],
            draws,
            camera,
            key: None,
            px_per_metre: comp.px_per_metre(t),
            read: None,
            trace_layers: false,
            paint: Vec::new(),
            paint_time: 0.0,
        })),
        colour_space: None,
        natural: [comp.width as f32, comp.height as f32],
        graph_fx: Vec::new(),
        place: None,
        points_schedules: Vec::new(),
    }
}

/// Effect `effect`'s layer row `row` as a picture at comp time `at`, made by
/// walking the comp again at that moment. The third argument is the walk's copy
/// and whether it is itself a rebuild.
///
/// No keyer: a nested comp built for another moment holds its footage a whole
/// frame at a time, which is not always the picture its name would claim.
pub(super) fn picture_at(
    doc: &Arc<Document>,
    comp: &Composition,
    (at, frame_t): (f64, f64),
    frame_pixels: &HashMap<Uuid, &CompLayerPixels>,
    visited: &[Uuid],
    (clone, rebuild): ((u32, u32), bool),
    (effect, row): (&EffectInstance, &str),
) -> Option<DofInputDraw> {
    let mut probe = Probe::Picture {
        effect,
        row,
        found: None,
    };
    comp_walk(
        doc,
        comp,
        at,
        frame_t,
        frame_pixels,
        &mut visited.to_vec(),
        None,
        true,
        Walk {
            clone,
            values: &[],
            rebuild,
            seen_through: None,
        },
        Some(&mut probe),
    );
    match probe {
        Probe::Picture { found, .. } => found,
        _ => None,
    }
}

/// A layer's own pixels as an input slot, with nothing done to them.
pub(super) fn plain_input(
    doc: &Document,
    layer: &lumit_core::model::Layer,
    (rgba, tex_w, tex_h, natural, format): super::LayerPixels,
) -> DofInputDraw {
    DofInputDraw {
        rgba: rgba.to_vec(),
        tex_w,
        tex_h,
        format,
        fx: Default::default(),
        colour_tables: Default::default(),
        flare_lens_files: Vec::new(),
        roto_mattes: Vec::new(),
        planes: Vec::new(),
        nested: None,
        colour_space: crate::colour::footage_colour_space(doc, &layer.kind),
        natural: [natural.0, natural.1],
        graph_fx: Vec::new(),
        place: None,
        points_schedules: Vec::new(),
    }
}

/// A scene pass of Model layer `layer` alone, seen through the comp's camera:
/// the layer walked with none of its effects, so a pass read from its own
/// stack can't come back round to itself.
pub(super) fn model_pass(
    doc: &Arc<Document>,
    comp: &Composition,
    layer: &lumit_core::model::Layer,
    (t, frame_t): (f64, f64),
    frame_pixels: &HashMap<Uuid, &CompLayerPixels>,
    (visited, clone): (&[Uuid], (u32, u32)),
    read: lumit_core::fx::effects::scene_pass::SceneRead,
) -> LayerInputDraw {
    let mut bare = layer.clone();
    bare.effects.clear();
    // A matte layer is often hidden, and the row reads it all the same.
    bare.switches.visible = true;
    bare.switches.solo = false;
    let (draws, camera) = below_draws_cloned(
        doc,
        comp,
        std::slice::from_ref(&bare),
        t,
        frame_t,
        None,
        frame_pixels,
        &mut visited.to_vec(),
        clone,
        None,
    );
    let mut input = picture_of(comp, draws, camera, t);
    if let Some(n) = input.nested.as_mut() {
        n.read = Some(read);
    }
    LayerInputDraw::Layer(input)
}

/// Effect `e` on layer `owner`'s pictures at the moments its stack names at
/// comp time `t`, first first, each made by walking the comp again at that
/// moment. Absent unless `e` is the effect in its stack that shows them.
pub(super) fn own_moments_pictures(
    doc: &Arc<Document>,
    comp: &Composition,
    (t, frame_t): (f64, f64),
    frame_pixels: &HashMap<Uuid, &CompLayerPixels>,
    visited: &[Uuid],
    clone: (u32, u32),
    (owner, e): (Uuid, &EffectInstance),
) -> LayerInputDraw {
    let Some(layer) = comp.layers.iter().find(|l| l.id == owner) else {
        return LayerInputDraw::Absent;
    };
    let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
    let Some((upto, moments)) = lumit_core::fx::moments::stack_own_moments(
        owner,
        &layer.effects,
        layer.switches.fx,
        lt,
        &lumit_core::fx::markers::MarkerContext::for_layer(comp, layer),
    )
    .filter(|(i, _)| layer.effects.get(*i).is_some_and(|x| x.id == e.id)) else {
        return LayerInputDraw::Absent;
    };
    let start = layer.start_offset.0.to_f64();
    let picture = |at: f64| -> LayerInputDraw {
        let mut probe = Probe::Own {
            owner,
            upto,
            found: None,
        };
        comp_walk(
            doc,
            comp,
            at + start,
            frame_t,
            frame_pixels,
            &mut visited.to_vec(),
            None,
            true,
            Walk {
                clone,
                values: &[],
                rebuild: true,
                seen_through: None,
            },
            Some(&mut probe),
        );
        match probe {
            Probe::Own { found: Some(d), .. } => LayerInputDraw::Layer(d),
            _ => LayerInputDraw::Absent,
        }
    };
    LayerInputDraw::Several(moments.times().into_iter().map(picture).collect())
}
