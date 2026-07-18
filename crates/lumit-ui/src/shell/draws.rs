//! `shell::draws` — split out of the monolithic shell.rs (mechanical,
//! no logic changes). Shared names resolve through the parent module
//! via `use super::*` and the glob re-exports in `shell/mod.rs`.

use super::*;

/// A copy of `comp` with one layer's transform property overridden to a fixed
/// `value` — the live value-drag preview renders this so the provisional value
/// shows before the edit is committed. Only the previewed frame is rendered, so
/// pinning the property to a constant is exactly its value at that instant.
#[cfg(feature = "media")]
pub(crate) fn patch_layer_prop(
    comp: &lumit_core::model::Composition,
    layer: uuid::Uuid,
    prop: lumit_core::model::TransformProp,
    value: f64,
) -> lumit_core::model::Composition {
    let mut patched = comp.clone();
    if let Some(l) = patched.layers.iter_mut().find(|l| l.id == layer) {
        *l.transform.get_mut(prop) = lumit_core::anim::Property::fixed(value);
    }
    patched
}

/// The world placement matrix of `layer`'s parent chain within `comp` at comp
/// time `t_comp` (K-103 layer parenting): `P_top × … × P_grandparent ×
/// P_parent`, each ancestor's `place_matrix` sampled at its own local time
/// (`t_comp − start_offset`). `None` when the layer has no parent. Used as a
/// draw's `pre`, which the GPU applies as `pre × own_placement` — so the child
/// ends up placed inside its parent's coordinate space (After Effects
/// parenting). Cycle- and missing-parent-safe via `model::layer_parent_chain`.
/// Shared by the preview (here) and the export path so the two stay identical
/// (K-031). v1 composes the full `place_matrix` (2D plus the 2.5D axes it
/// already carries); no behaviour changes for an unparented layer (`None`).
#[cfg(feature = "media")]
pub(crate) fn parent_world_placement(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
) -> Option<[[f32; 4]; 4]> {
    layer.parent?;
    let chain = lumit_core::model::layer_parent_chain(comp, layer.id);
    let mut world: Option<[[f32; 4]; 4]> = None;
    // Fold from the farthest ancestor inward so the topmost transform is the
    // outermost: concat_place(outer, inner) = outer × inner.
    for ancestor_id in chain.iter().rev() {
        let Some(a) = comp.layers.iter().find(|l| l.id == *ancestor_id) else {
            continue;
        };
        let alt = t_comp - a.start_offset.0.to_f64();
        let tr = &a.transform;
        let p = lumit_gpu::place_matrix(
            (
                tr.position_x.value_at(alt) as f32,
                tr.position_y.value_at(alt) as f32,
            ),
            (
                tr.anchor_x.value_at(alt) as f32,
                tr.anchor_y.value_at(alt) as f32,
            ),
            (
                tr.scale_x.value_at(alt) as f32,
                tr.scale_y.value_at(alt) as f32,
            ),
            tr.rotation.value_at(alt) as f32,
            tr.position_z.value_at(alt) as f32,
            tr.rotation_x.value_at(alt) as f32,
            tr.rotation_y.value_at(alt) as f32,
        );
        world = Some(match world {
            Some(w) => lumit_gpu::concat_place(w, p),
            None => p,
        });
    }
    world
}

/// Build a comp's draw list recursively (preview side of Precomp layers).
/// Bottom-up order; matte sources come from decoded pixels (precomp mattes
/// await the GPU mask pass, mirroring export).
#[cfg(feature = "media")]
pub(crate) fn build_comp_draws(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    t_comp: f64,
    pixels_by_layer: &std::collections::HashMap<
        uuid::Uuid,
        &crate::app_state::preview::CompLayerPixels,
    >,
    visited: &mut Vec<uuid::Uuid>,
) -> Vec<CompLayerDraw> {
    use lumit_core::model::LayerKind;
    let in_span = |l: &lumit_core::model::Layer| {
        t_comp >= l.in_point.0.to_f64() && t_comp < l.out_point.0.to_f64()
    };
    let pixels_for = |layer: &lumit_core::model::Layer| -> Option<LayerPixels> {
        let raw = match &layer.kind {
            // An adjustment layer has no pixels of its own; until its effect
            // stack exists it is a pass-through and draws nothing.
            LayerKind::Adjustment => return None,
            // Footage and Sequence footage clips both arrive decoded, keyed by
            // the layer id (collect_comp_jobs pushes one job per layer/frame).
            LayerKind::Footage { .. } | LayerKind::Sequence { .. } => {
                pixels_by_layer.get(&layer.id).map(|lp| {
                    // Geometry uses the native source size, never the decoded
                    // size: under auto res the decode shrinks and grows with
                    // viewport zoom, and sizing the layer by that made it
                    // scale with zoom (a small layer ballooned when zoomed in).
                    (
                        lp.rgba.clone(),
                        lp.width,
                        lp.height,
                        (lp.natural_w as f32, lp.natural_h as f32),
                    )
                })
            }
            LayerKind::Solid { def } => doc.solid(*def).filter(|_| in_span(layer)).map(|sd| {
                let px = crate::export::solid_rgba(sd.colour);
                let (tw, th) = if layer.masks.is_empty() {
                    (8, 8)
                } else {
                    (sd.width, sd.height)
                };
                (
                    crate::export::px_tile(&px, tw, th),
                    tw,
                    th,
                    (sd.width as f32, sd.height as f32),
                )
            }),
            LayerKind::Text { document } => in_span(layer).then(|| {
                let fill = crate::export::solid_rgba(document.fill);
                let r = lumit_text::rasterise_line(
                    &document.text,
                    document.size as f32,
                    [fill[0], fill[1], fill[2]],
                );
                (r.rgba, r.width, r.height, (r.width as f32, r.height as f32))
            }),
            LayerKind::Precomp { .. } => None, // handled as Nested below
            LayerKind::Camera { .. } => None,  // shapes the view, draws nothing
        };
        raw.map(|(mut rgba, w, h, natural)| {
            lumit_core::mask::apply_masks(
                &mut rgba,
                w,
                h,
                f64::from(natural.0),
                f64::from(natural.1),
                &layer.masks,
            );
            (rgba, w, h, natural)
        })
    };

    // Solo / isolate (K-105): while any layer is soloed, only soloed layers
    // render — computed once for the whole comp.
    let any_solo = lumit_core::model::any_solo(comp);
    let mut draws: Vec<CompLayerDraw> = Vec::new();
    for layer in comp.layers.iter().rev() {
        if !layer.switches.visible || !in_span(layer) || (any_solo && !layer.switches.solo) {
            continue;
        }
        let lt = t_comp - layer.start_offset.0.to_f64();
        let tr = &layer.transform;

        let (source, natural) = match &layer.kind {
            LayerKind::Precomp { comp: nested_id } => {
                if visited.contains(nested_id) {
                    continue; // cycle guard
                }
                let Some(nested) = doc.comp(*nested_id) else {
                    continue;
                };
                // Collapse (docs/06 §1.4): splice the inner layers straight
                // into this list with the Precomp layer's placement multiplied
                // in front — no intermediate raster, no clipping to the nested
                // bounds, inner blend modes composite against the parent stack.
                if matches!(
                    lumit_core::model::collapse_state(doc, comp, layer, lt),
                    lumit_core::model::CollapseState::Active
                ) {
                    visited.push(*nested_id);
                    let mut inner = build_comp_draws(doc, nested, lt, pixels_by_layer, visited);
                    visited.pop();
                    let own = lumit_gpu::place_matrix(
                        (
                            tr.position_x.value_at(lt) as f32,
                            tr.position_y.value_at(lt) as f32,
                        ),
                        (
                            tr.anchor_x.value_at(lt) as f32,
                            tr.anchor_y.value_at(lt) as f32,
                        ),
                        (
                            tr.scale_x.value_at(lt) as f32,
                            tr.scale_y.value_at(lt) as f32,
                        ),
                        tr.rotation.value_at(lt) as f32,
                        tr.position_z.value_at(lt) as f32,
                        tr.rotation_x.value_at(lt) as f32,
                        tr.rotation_y.value_at(lt) as f32,
                    );
                    // If the collapsed precomp is itself parented, its parent's
                    // world placement wraps its own before it wraps the inner
                    // draws (K-103).
                    let parent = match parent_world_placement(comp, layer, t_comp) {
                        Some(pw) => lumit_gpu::concat_place(pw, own),
                        None => own,
                    };
                    for d in &mut inner {
                        d.pre = Some(match d.pre {
                            // A collapsed chain: this parent wraps the child's
                            // own parent placement.
                            Some(p) => lumit_gpu::concat_place(parent, p),
                            None => parent,
                        });
                    }
                    draws.extend(inner);
                    continue;
                }
                visited.push(*nested_id);
                let nested_draws = build_comp_draws(doc, nested, lt, pixels_by_layer, visited);
                visited.pop();
                let nbg = nested.background.0;
                (
                    DrawSource::Nested {
                        width: nested.width,
                        height: nested.height,
                        background: [
                            f64::from(nbg[0]),
                            f64::from(nbg[1]),
                            f64::from(nbg[2]),
                            f64::from(nbg[3]),
                        ],
                        draws: nested_draws,
                        camera: nested.camera_pose(lt),
                    },
                    (nested.width as f32, nested.height as f32),
                )
            }
            LayerKind::Adjustment => {
                // A staging point, not a picture (docs/06 §1.5): realise
                // composites everything below, runs this stack on it, and
                // blends back by coverage — masks × opacity, placed by the
                // transform. A dead stack contributes nothing at all.
                let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let fx = if layer.switches.fx {
                    // The §1.4 marker context, built by the same shared
                    // constructor export uses (K-031).
                    let markers = lumit_core::fx::MarkerContext::for_layer(comp, layer);
                    lumit_core::fx::resolve_stack(&layer.effects, lt, comp_diag, 1.0, &markers)
                } else {
                    Vec::new()
                };
                if fx.is_empty() {
                    continue;
                }
                draws.push(CompLayerDraw {
                    source: DrawSource::Adjust,
                    natural_size: (comp.width as f32, comp.height as f32),
                    position: (
                        tr.position_x.value_at(lt) as f32,
                        tr.position_y.value_at(lt) as f32,
                    ),
                    anchor: (
                        tr.anchor_x.value_at(lt) as f32,
                        tr.anchor_y.value_at(lt) as f32,
                    ),
                    scale: (
                        tr.scale_x.value_at(lt) as f32,
                        tr.scale_y.value_at(lt) as f32,
                    ),
                    rotation_deg: tr.rotation.value_at(lt) as f32,
                    opacity: tr.opacity.value_at(lt) as f32,
                    z: tr.position_z.value_at(lt) as f32,
                    rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
                    rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
                    three_d: layer.switches.three_d,
                    matte: None,
                    blend: lumit_gpu::Blend::Normal,
                    mask_cov: (!layer.masks.is_empty()).then(|| {
                        // Adjustment masks live in comp space (comp-sized
                        // natural), same as the property panel treats them.
                        (
                            crate::export::mask_rgba(&lumit_core::mask::combined_coverage(
                                &layer.masks,
                                comp.width,
                                comp.height,
                                f64::from(comp.width),
                                f64::from(comp.height),
                            )),
                            comp.width,
                            comp.height,
                        )
                    }),
                    pre: parent_world_placement(comp, layer, t_comp),
                    fx,
                    // Adjustment layers process the composite below, not
                    // footage frames — no neighbours or flow field here.
                    neighbours: Vec::new(),
                    flow_field: None,
                });
                continue;
            }
            _ => {
                let Some((rgba, w, h, natural)) = pixels_for(layer) else {
                    continue;
                };
                (
                    DrawSource::Pixels {
                        rgba,
                        tex_w: w,
                        tex_h: h,
                    },
                    natural,
                )
            }
        };

        let matte = layer.matte.as_ref().and_then(|mr| {
            let src = comp.layers.iter().find(|l| l.id == mr.layer)?;
            let (m_rgba, m_w, m_h, m_nat) = pixels_for(src)?;
            let mlt = t_comp - src.start_offset.0.to_f64();
            let mtr = &src.transform;
            Some(MatteDraw {
                rgba: m_rgba,
                tex_w: m_w,
                tex_h: m_h,
                natural_size: m_nat,
                position: (
                    mtr.position_x.value_at(mlt) as f32,
                    mtr.position_y.value_at(mlt) as f32,
                ),
                anchor: (
                    mtr.anchor_x.value_at(mlt) as f32,
                    mtr.anchor_y.value_at(mlt) as f32,
                ),
                scale: (
                    mtr.scale_x.value_at(mlt) as f32,
                    mtr.scale_y.value_at(mlt) as f32,
                ),
                rotation_deg: mtr.rotation.value_at(mlt) as f32,
                opacity: mtr.opacity.value_at(mlt) as f32,
                z: mtr.position_z.value_at(mlt) as f32,
                rotation_x_deg: mtr.rotation_x.value_at(mlt) as f32,
                rotation_y_deg: mtr.rotation_y.value_at(mlt) as f32,
                three_d: src.switches.three_d,
                luma: matches!(mr.channel, lumit_core::model::MatteChannel::Luma),
                inverted: mr.inverted,
            })
        });

        // Radius units are % of the comp diagonal (docs/08 §2.3); the effect
        // runs on the layer's decoded texture, so scale the diagonal by
        // decode/natural to stay honest under reduced-resolution preview.
        let fx = {
            let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
            let scale = match &source {
                DrawSource::Pixels { tex_w, .. } => *tex_w as f32 / natural.0.max(1.0),
                // Adjust never reaches here (its arm pushes and continues);
                // its stack runs on the comp-sized intermediate, factor 1.
                DrawSource::Nested { .. } | DrawSource::Adjust => 1.0,
            };
            if layer.switches.fx {
                // scale doubles as the §2.3 preview-resolution factor:
                // raster pixels per comp pixel for px@comp parameters. The
                // §1.4 marker context comes from the same shared
                // constructor export uses (K-031).
                let markers = lumit_core::fx::MarkerContext::for_layer(comp, layer);
                lumit_core::fx::resolve_stack(
                    &layer.effects,
                    lt,
                    comp_diag * scale,
                    scale,
                    &markers,
                )
            } else {
                Vec::new()
            }
        };
        // Decoded neighbour frames for a temporal effect (echo), carried from
        // the layer's decode job; empty for a plain stack.
        let neighbours: Vec<(i32, Vec<u8>, u32, u32)> = pixels_by_layer
            .get(&layer.id)
            .map(|lp| {
                lp.temporal
                    .iter()
                    .map(|(o, rgba)| (*o, rgba.clone(), lp.width, lp.height))
                    .collect()
            })
            .unwrap_or_default();
        // The dense motion field for Flow motion blur, carried from the same
        // decode job (its `(u, v)` are at the layer's decoded size).
        let flow_field = pixels_by_layer.get(&layer.id).and_then(|lp| {
            lp.flow_field
                .as_ref()
                .map(|(u, v)| (u.clone(), v.clone(), lp.width, lp.height))
        });
        draws.push(CompLayerDraw {
            source,
            natural_size: natural,
            position: (
                tr.position_x.value_at(lt) as f32,
                tr.position_y.value_at(lt) as f32,
            ),
            anchor: (
                tr.anchor_x.value_at(lt) as f32,
                tr.anchor_y.value_at(lt) as f32,
            ),
            scale: (
                tr.scale_x.value_at(lt) as f32,
                tr.scale_y.value_at(lt) as f32,
            ),
            rotation_deg: tr.rotation.value_at(lt) as f32,
            opacity: tr.opacity.value_at(lt) as f32,
            z: tr.position_z.value_at(lt) as f32,
            rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
            rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
            three_d: layer.switches.three_d,
            matte,
            blend: blend_of(layer.blend),
            mask_cov: match &layer.kind {
                LayerKind::Precomp { .. } if !layer.masks.is_empty() => {
                    let (w, h) = (natural.0 as u32, natural.1 as u32);
                    Some((
                        crate::export::mask_rgba(&lumit_core::mask::combined_coverage(
                            &layer.masks,
                            w,
                            h,
                            f64::from(w),
                            f64::from(h),
                        )),
                        w,
                        h,
                    ))
                }
                _ => None,
            },
            pre: parent_world_placement(comp, layer, t_comp),
            fx,
            neighbours,
            flow_field,
        });
    }
    draws
}

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used)]
mod parent_placement_tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::*;
    use lumit_core::{CompTime, Duration, FrameRate, Rational};

    fn layer(px: f64, py: f64, parent: Option<uuid::Uuid>) -> Layer {
        Layer {
            id: uuid::Uuid::now_v7(),
            name: "l".into(),
            kind: LayerKind::Solid {
                def: uuid::Uuid::now_v7(),
            },
            in_point: CompTime(Rational::new(0, 1).unwrap()),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::new(0, 1).unwrap()),
            transform: TransformGroup {
                position_x: Property::fixed(px),
                position_y: Property::fixed(py),
                ..TransformGroup::default()
            },
            matte: None,
            parent,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn place_of(l: &Layer) -> [[f32; 4]; 4] {
        let tr = &l.transform;
        lumit_gpu::place_matrix(
            (
                tr.position_x.value_at(0.0) as f32,
                tr.position_y.value_at(0.0) as f32,
            ),
            (0.0, 0.0),
            (100.0, 100.0),
            0.0,
            0.0,
            0.0,
            0.0,
        )
    }

    fn comp(layers: Vec<Layer>) -> Composition {
        Composition {
            id: uuid::Uuid::now_v7(),
            name: "c".into(),
            width: 100,
            height: 100,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.0, 0.0, 0.0, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    #[test]
    fn unparented_is_none_and_a_chain_composes_top_outermost() {
        let gp = layer(10.0, 20.0, None);
        let parent = layer(100.0, 0.0, Some(gp.id));
        let child = layer(5.0, 5.0, Some(parent.id));
        let c = comp(vec![gp.clone(), parent.clone(), child.clone()]);
        // No parent → no placement.
        assert!(parent_world_placement(&c, &gp, 0.0).is_none());
        // The child's world placement is grandparent × parent (top outermost),
        // exactly the manual concat — proving the walk and fold order.
        let world = parent_world_placement(&c, &child, 0.0).unwrap();
        let expected = lumit_gpu::concat_place(place_of(&gp), place_of(&parent));
        assert_eq!(world, expected);
    }
}
