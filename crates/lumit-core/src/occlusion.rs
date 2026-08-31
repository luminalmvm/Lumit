//! Occlusion culling (K-423, docs/06-RENDER-PIPELINE.md §1.1): when a layer
//! provably paints every pixel of the frame, the layers beneath it are never
//! seen and need not be decoded, uploaded, effected or composited.
//!
//! # In plain terms
//!
//! A full-frame opaque solid hides everything under it. Rendering what it
//! hides is wasted work, so the draw builder and the decode planner both ask
//! this one question and skip the layers below the answer. The question is
//! asked as narrowly as possible: the cull must be invisible in the picture,
//! so every case where a layer below could still reach the frame — a matte,
//! an effect reading another layer, an adjustment above, a camera, a blend
//! mode, a mask, anything animated by an expression — refuses to cull. Being
//! over-cautious costs nothing but speed; being wrong costs pixels.
//!
//! Thread role: pure function over the snapshot; any thread.

use crate::anim::Animation;
use crate::model::{BlendMode, Composition, Document, EffectValue, Layer, LayerKind};
use crate::time::layer_time;

/// The index (0 = top of the stack) of the topmost layer in `comp` that, at
/// comp time `t`, fully covers the frame with opaque pixels — so that every
/// layer below it may be skipped. `None` when no layer qualifies or when
/// anything in the comp could make a skipped layer matter.
///
/// v1 accepts only a Solid layer (a solid colour whose alpha is 1) that is
/// visible, in span, soloed if anything is, 2D with no rotation, Normal blend
/// at full opacity, with no masks, paint, enabled effects or motion blur, whose
/// axis-aligned placement (its own transform and its parent chain, none of
/// them rotated, 3D or driven by an expression) covers the comp rectangle.
/// The comp must have no active camera and no visible Adjustment layer above
/// the candidate, and no visible layer above it may reference a layer below
/// it as a matte or as an effect's layer input.
#[must_use]
pub fn occluder_index(doc: &Document, comp: &Composition, t: f64) -> Option<usize> {
    if comp.active_camera(t).is_some() {
        return None;
    }
    let any_solo = crate::model::any_picture_solo(comp);
    let drawn = |l: &Layer| {
        // An Audio layer never draws (K-435), so it is never an occluder and
        // never one of the layers above one that could spoil the cull.
        !l.audio_only
            && l.switches.visible
            && t >= l.in_point.0.to_f64()
            && t < l.out_point.0.to_f64()
            && (!any_solo || l.switches.solo)
    };
    let candidate = comp.layers.iter().enumerate().find(|(_, l)| {
        drawn(l)
            // A solid acting as an adjustment (K-537) has set its own colour
            // aside — it shows what is under it, so it hides nothing.
            && !l.is_adjustment()
            && matches!(l.kind, LayerKind::Solid { .. })
            && covers_frame(doc, comp, l, t)
    })?;
    let (idx, _) = candidate;
    let below = |id: uuid::Uuid| comp.layers.iter().skip(idx + 1).any(|l| l.id == id);
    let above = comp.layers.iter().take(idx).filter(|l| drawn(l));
    for l in above {
        if l.is_adjustment() {
            return None;
        }
        if l.matte.as_ref().is_some_and(|m| below(m.layer)) {
            return None;
        }
        let refs_below = l.effects.iter().filter(|e| e.enabled).any(|e| {
            e.params
                .iter()
                .any(|p| matches!(p.value, EffectValue::Layer(Some(id)) if below(id)))
        });
        if refs_below {
            return None;
        }
    }
    Some(idx)
}

/// An axis-aligned 2D placement: `x' = offset + scale · x`, per axis.
#[derive(Clone, Copy)]
struct Affine {
    scale: (f64, f64),
    offset: (f64, f64),
}

/// The layer's own placement at layer time `lt` as an axis-aligned affine, or
/// `None` when it rotates, leaves the plane, or is driven by an expression.
fn flat_placement(layer: &Layer, lt: f64) -> Option<Affine> {
    let tr = &layer.transform;
    let props = [
        &tr.anchor_x,
        &tr.anchor_y,
        &tr.position_x,
        &tr.position_y,
        &tr.scale_x,
        &tr.scale_y,
        &tr.rotation,
        &tr.position_z,
        &tr.rotation_x,
        &tr.rotation_y,
        &tr.opacity,
    ];
    if props
        .iter()
        .any(|p| matches!(p.animation, Animation::Expression(_)))
    {
        return None;
    }
    let flat = tr.rotation.value_at(lt) == 0.0
        && tr.rotation_x.value_at(lt) == 0.0
        && tr.rotation_y.value_at(lt) == 0.0
        && tr.position_z.value_at(lt) == 0.0;
    if !flat {
        return None;
    }
    let scale = (
        tr.scale_x.value_at(lt) / 100.0,
        tr.scale_y.value_at(lt) / 100.0,
    );
    Some(Affine {
        scale,
        offset: (
            tr.position_x.value_at(lt) - scale.0 * tr.anchor_x.value_at(lt),
            tr.position_y.value_at(lt) - scale.1 * tr.anchor_y.value_at(lt),
        ),
    })
}

/// Whether `layer` — a Solid — paints every pixel of `comp` opaquely at comp
/// time `t`.
fn covers_frame(doc: &Document, comp: &Composition, layer: &Layer, t: f64) -> bool {
    let LayerKind::Solid { def } = &layer.kind else {
        return false;
    };
    let Some(solid) = doc.solid(*def) else {
        return false;
    };
    let lt = layer_time(t, layer.start_offset.0);
    let plain = solid.colour.0[3] >= 1.0
        && layer.blend == BlendMode::Normal
        && layer.matte.is_none()
        && layer.masks.is_empty()
        && layer.paint.is_empty()
        && !layer.switches.three_d
        && !layer.switches.motion_blur
        && !layer.effects.iter().any(|e| e.enabled)
        && layer.transform.opacity.value_at(lt) >= 100.0;
    if !plain {
        return false;
    }
    let Some(own) = flat_placement(layer, lt) else {
        return false;
    };
    // The parent chain wraps the layer's own placement, nearest parent first,
    // each sampled at its own layer time — the order `parent_world_placement`
    // composes in the renderer.
    let mut corners = [
        (own.offset.0, own.offset.1),
        (
            own.offset.0 + own.scale.0 * f64::from(solid.width),
            own.offset.1 + own.scale.1 * f64::from(solid.height),
        ),
    ];
    for id in crate::model::layer_parent_chain(comp, layer.id) {
        let Some(parent) = comp.layers.iter().find(|l| l.id == id) else {
            continue;
        };
        let Some(p) = flat_placement(parent, layer_time(t, parent.start_offset.0)) else {
            return false;
        };
        for c in &mut corners {
            *c = (p.offset.0 + p.scale.0 * c.0, p.offset.1 + p.scale.1 * c.1);
        }
    }
    let (x0, x1) = (
        corners[0].0.min(corners[1].0),
        corners[0].0.max(corners[1].0),
    );
    let (y0, y1) = (
        corners[0].1.min(corners[1].1),
        corners[0].1.max(corners[1].1),
    );
    x0 <= 0.0 && y0 <= 0.0 && x1 >= f64::from(comp.width) && y1 >= f64::from(comp.height)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::type_complexity
)]
mod tests {
    use super::*;
    use crate::anim::Property;
    use crate::model::{
        EffectInstance, EffectKey, EffectNamespace, EffectParam, LinearColour, MatteChannel,
        MatteRef, ProjectItem, SolidDef, Switches, TransformGroup,
    };
    use crate::time::{CompTime, Duration, FrameRate, Rational};
    use uuid::Uuid;

    fn layer(kind: LayerKind, w: u32, h: u32) -> Layer {
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "l".into(),
            kind,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup {
                anchor_x: Property::fixed(f64::from(w) * 0.5),
                anchor_y: Property::fixed(f64::from(h) * 0.5),
                position_x: Property::fixed(32.0),
                position_y: Property::fixed(32.0),
                ..Default::default()
            },
            matte: None,
            parent: None,
            label: 0,
            volume_db: Property::zero(),
            pan: Property::zero(),
            audio_only: false,
            adjustment: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    /// A 64×64 comp: a full-frame solid over a smaller solid.
    fn scene() -> (Document, Composition) {
        let mut doc = Document::new();
        let mut solid = |w: u32, h: u32| {
            let id = Uuid::now_v7();
            doc.items.push(ProjectItem::Solid(SolidDef {
                id,
                name: "s".into(),
                colour: LinearColour([1.0, 0.0, 0.0, 1.0]),
                width: w,
                height: h,
                extra: serde_json::Map::new(),
            }));
            id
        };
        let top = layer(LayerKind::Solid { def: solid(64, 64) }, 64, 64);
        let under = layer(LayerKind::Solid { def: solid(16, 16) }, 16, 16);
        let comp = Composition {
            master_volume_db: 0.0,
            groups: Vec::new(),
            beat_grid: None,
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 64,
            height: 64,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![top, under],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        (doc, comp)
    }

    fn layer_ref_effect(target: Uuid) -> EffectInstance {
        EffectInstance {
            id: Uuid::now_v7(),
            effect: EffectKey {
                namespace: EffectNamespace::Builtin,
                match_name: "dof".into(),
                version: 1,
                extra: serde_json::Map::new(),
            },
            enabled: true,
            params: vec![EffectParam {
                id: "depth".into(),
                value: EffectValue::Layer(Some(target)),
                extra: serde_json::Map::new(),
            }],
            sample_temporally: true,
            custom_name: None,
            linked_pairs: Vec::new(),
            plugin_state: None,
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn a_full_frame_opaque_solid_occludes_what_is_under_it() {
        let (doc, comp) = scene();
        assert_eq!(occluder_index(&doc, &comp, 1.0), Some(0));
    }

    #[test]
    fn a_solid_that_does_not_reach_every_edge_does_not() {
        let (doc, mut comp) = scene();
        comp.layers[0].transform.position_x = Property::fixed(33.0);
        assert_eq!(occluder_index(&doc, &comp, 1.0), None);
        // Scaled up past the edges it covers again, even when parented.
        comp.layers[0].transform.scale_x = Property::fixed(200.0);
        assert_eq!(occluder_index(&doc, &comp, 1.0), Some(0));
        let parent = comp.layers[1].id;
        comp.layers[0].parent = Some(parent);
        comp.layers[1].transform.scale_x = Property::fixed(10.0);
        assert_eq!(occluder_index(&doc, &comp, 1.0), None);
    }

    #[test]
    fn anything_that_could_show_the_layers_below_refuses() {
        let disqualify: Vec<(&str, Box<dyn Fn(&mut Composition)>)> = vec![
            (
                "rotation",
                Box::new(|c| c.layers[0].transform.rotation = Property::fixed(1.0)),
            ),
            ("3D", Box::new(|c| c.layers[0].switches.three_d = true)),
            (
                "depth",
                Box::new(|c| c.layers[0].transform.position_z = Property::fixed(1.0)),
            ),
            (
                "opacity",
                Box::new(|c| c.layers[0].transform.opacity = Property::fixed(99.0)),
            ),
            (
                "blend",
                Box::new(|c| c.layers[0].blend = BlendMode::Multiply),
            ),
            (
                "motion blur",
                Box::new(|c| c.layers[0].switches.motion_blur = true),
            ),
            (
                "mask",
                Box::new(|c| {
                    c.layers[0]
                        .masks
                        .push(crate::mask::Mask::rectangle(0.0, 0.0, 64.0, 64.0))
                }),
            ),
            ("hidden", Box::new(|c| c.layers[0].switches.visible = false)),
            (
                "soloed below",
                Box::new(|c| c.layers[1].switches.solo = true),
            ),
            (
                "expression",
                Box::new(|c| {
                    c.layers[0].transform.position_x.animation = Animation::Expression("32".into())
                }),
            ),
            (
                "effect",
                Box::new(|c| {
                    let id = c.layers[0].id;
                    c.layers[0].effects.push(layer_ref_effect(id));
                }),
            ),
            (
                "matte from above",
                Box::new(|c| {
                    let under = c.layers[1].id;
                    let mut l = layer(LayerKind::Null, 1, 1);
                    l.matte = Some(MatteRef {
                        layer: under,
                        channel: MatteChannel::Alpha,
                        inverted: false,
                        source: Default::default(),
                    });
                    c.layers.insert(0, l);
                }),
            ),
            (
                "layer input from above",
                Box::new(|c| {
                    let under = c.layers[1].id;
                    let mut l = layer(LayerKind::Null, 1, 1);
                    l.effects.push(layer_ref_effect(under));
                    c.layers.insert(0, l);
                }),
            ),
            (
                "adjustment above",
                Box::new(|c| c.layers.insert(0, layer(LayerKind::Adjustment, 1, 1))),
            ),
            (
                "camera",
                Box::new(|c| {
                    c.layers.insert(
                        0,
                        layer(
                            LayerKind::Camera {
                                zoom: Property::fixed(100.0),
                                solve_link: None,
                                correction_base: None,
                            },
                            1,
                            1,
                        ),
                    )
                }),
            ),
        ];
        for (why, change) in disqualify {
            let (doc, mut comp) = scene();
            change(&mut comp);
            assert_eq!(
                occluder_index(&doc, &comp, 1.0),
                None,
                "{why} must refuse the cull"
            );
        }
    }

    #[test]
    fn references_that_stay_above_the_occluder_do_not_refuse() {
        let (doc, mut comp) = scene();
        let mut l = layer(LayerKind::Null, 1, 1);
        let other = layer(LayerKind::Null, 1, 1);
        l.effects.push(layer_ref_effect(other.id));
        comp.layers.insert(0, other);
        comp.layers.insert(0, l);
        assert_eq!(occluder_index(&doc, &comp, 1.0), Some(2));
    }
}
