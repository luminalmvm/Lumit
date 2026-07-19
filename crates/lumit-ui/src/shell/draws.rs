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

/// A copy of `comp` with one Float effect parameter overridden to a fixed
/// `value` — the effect twin of [`patch_layer_prop`], for the live effect-
/// value drag. Only the previewed frame renders this, so pinning the param to
/// a constant is exactly its value at that instant; the effect stack re-runs
/// with it (`build_comp_draws` re-resolves the layer's effects). Out-of-range
/// indices or a non-Float param leave the comp unchanged (a no-op, never a
/// panic).
#[cfg(feature = "media")]
pub(crate) fn patch_layer_effect_param(
    comp: &lumit_core::model::Composition,
    layer: uuid::Uuid,
    effect_idx: usize,
    param_idx: usize,
    value: f64,
) -> lumit_core::model::Composition {
    let mut patched = comp.clone();
    if let Some(l) = patched.layers.iter_mut().find(|l| l.id == layer) {
        if let Some(p) = l
            .effects
            .get_mut(effect_idx)
            .and_then(|e| e.params.get_mut(param_idx))
        {
            if matches!(p.value, lumit_core::model::EffectValue::Float(_)) {
                p.value =
                    lumit_core::model::EffectValue::Float(lumit_core::anim::Property::fixed(value));
            }
        }
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

/// The per-layer motion-blur sub-frame placements for `layer` at comp time
/// `t_comp` (docs/06 §4, K-120): the layer's own transform re-evaluated at each
/// shutter sample time. Empty — so the layer draws normally — unless the comp
/// master (`comp.motion_blur.enabled`) and the layer's own switch are both on
/// and `samples` ≥ 2.
///
/// Each sample's comp time is `t_comp + offset · dt` (dt = one frame in comp
/// seconds; offsets from [`MotionBlur::sample_offsets`], centred on the frame),
/// and its layer time subtracts the layer's `start_offset`. Shared by the
/// preview (build_comp_draws) and the export path (render_comp_linear) so the
/// two smear identically (K-031). Parent motion within the shutter is a
/// follow-up: only the layer's OWN transform is sampled here — a parented
/// layer keeps its frame-time parent placement (`pre`) for every sub-copy.
#[cfg(feature = "media")]
pub(crate) fn motion_blur_samples(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
) -> Vec<lumit_gpu::MbSample> {
    if !layer.switches.motion_blur {
        return Vec::new();
    }
    let offsets = comp.motion_blur.sample_offsets();
    if offsets.is_empty() {
        return Vec::new();
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let start_offset = layer.start_offset.0.to_f64();
    let tr = &layer.transform;
    offsets
        .iter()
        .map(|off| {
            let lt = t_comp + off * dt - start_offset;
            lumit_gpu::MbSample {
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
                z: tr.position_z.value_at(lt) as f32,
                rotation_x_deg: tr.rotation_x.value_at(lt) as f32,
                rotation_y_deg: tr.rotation_y.value_at(lt) as f32,
            }
        })
        .collect()
}

/// Build a comp's draw list recursively (preview side of Precomp layers).
/// Bottom-up order; matte sources come from decoded pixels (precomp mattes
/// await the GPU mask pass, mirroring export). The ordinary render entry: draws
/// at comp time `t_comp` with every effect resolved at `t_comp` too — a thin
/// wrapper over [`build_comp_draws_at`] with the sample and frame times equal.
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
    build_comp_draws_at(doc, comp, t_comp, t_comp, pixels_by_layer, visited)
}

/// Build a comp's draw list at sample comp time `t_comp`, resolving each layer's
/// effects at `t_comp` **except** those flagged `sample_temporally == false`,
/// which resolve at the true frame time `frame_t` instead (docs/impl/
/// temporal-rerender.md §5). For an ordinary render `frame_t == t_comp`, so the
/// two times coincide and nothing changes; only a held/sub-frame temporal
/// re-render (Posterize time, accumulation motion blur) passes a `frame_t` that
/// differs, letting a costly/stochastic effect stay pinned to the playhead while
/// the rest of the scene is sampled. `frame_t` threads through nested Precomps
/// (each layer's own `start_offset` subtracted) so the flag is honoured at every
/// depth.
#[cfg(feature = "media")]
pub(crate) fn build_comp_draws_at(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    t_comp: f64,
    frame_t: f64,
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

    // The depth inputs of a stack's enabled built-in `dof` effects (docs/08
    // §3.22, docs/impl/layer-input.md), 1:1 and in order with the stack's
    // Resolved::Dof ops — the same `enabled && Builtin && match_name` filter
    // resolve_stack applies, and a `dof` effect always resolves to exactly one
    // op. Each slot carries the referenced layer's SOURCE pixels (via the same
    // `pixels_for` a matte uses, so effects are not applied and a depth
    // reference can never recurse); an unset or dangling reference is None (a
    // passthrough). The depth layer does NOT need to be visible — a depth map
    // is usually hidden so it doesn't render — only in-span; the decode
    // planner (app_state::collect_comp_jobs) decodes layer-input references
    // exactly like matte sources, and export applies the same in-span-only
    // gate (K-031).
    let dof_inputs_for =
        |effects: &[lumit_core::model::EffectInstance]| -> Vec<Option<DofInputDraw>> {
            use lumit_core::model::EffectNamespace;
            effects
                .iter()
                .filter(|e| {
                    e.enabled
                        && e.effect.namespace == EffectNamespace::Builtin
                        && e.effect.match_name == "dof"
                })
                .map(|e| {
                    let id = e.layer_ref("depth")?;
                    let src = comp.layers.iter().find(|l| l.id == id)?;
                    if !in_span(src) {
                        return None;
                    }
                    let (rgba, tex_w, tex_h, natural) = pixels_for(src)?;
                    // After-effects depth (K-125): when the dof effect sets
                    // depth_after_effects, resolve the depth layer's own stack at
                    // its layer time so render_dof_inputs runs it on the depth
                    // texture before resampling. Uses the depth layer's decode
                    // scale (its px@comp radii stay honest), the same resolve
                    // export uses (K-031). Empty when off or its fx switch is off.
                    let (fx, lut_files) =
                        if e.bool_of("depth_after_effects").unwrap_or(false) && src.switches.fx {
                            let slt = t_comp - src.start_offset.0.to_f64();
                            let comp_diag =
                                ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                            let scale = tex_w as f32 / natural.0.max(1.0);
                            let markers = lumit_core::fx::MarkerContext::for_layer(comp, src);
                            (
                                lumit_core::fx::resolve_stack(
                                    &src.effects,
                                    slt,
                                    comp_diag * scale,
                                    scale,
                                    &markers,
                                ),
                                lut_files(&src.effects, slt),
                            )
                        } else {
                            (Vec::new(), Vec::new())
                        };
                    Some(DofInputDraw {
                        rgba,
                        tex_w,
                        tex_h,
                        fx,
                        lut_files,
                    })
                })
                .collect()
        };

    // Solo / isolate (K-105): while any layer is soloed, only soloed layers
    // render — computed once for the whole comp.
    let any_solo = lumit_core::model::any_solo(comp);
    let mut draws: Vec<CompLayerDraw> = Vec::new();
    for (idx, layer) in comp.layers.iter().enumerate().rev() {
        if !layer.switches.visible || !in_span(layer) || (any_solo && !layer.switches.solo) {
            continue;
        }
        let lt = t_comp - layer.start_offset.0.to_f64();
        // The true frame time in this layer's own time base, for effects a
        // held/sub-frame re-render must not re-sample (docs/impl/
        // temporal-rerender.md §5). Equal to `lt` on an ordinary render.
        let frame_lt = frame_t - layer.start_offset.0.to_f64();
        // This layer's effects (docs/08 §3.25): a Posterize time scoped to *this
        // layer* holds this layer's OWN effect stack on the coarse grid — its
        // effects sample the held time while the transform and source below stay
        // live. Fed to resolve_stack_temporal as the *sample* time, so a
        // sample_temporally == false effect still holds at the true playhead
        // (§5); equal to `lt` when the stack has no this-layer Posterize.
        let effect_lt = lumit_core::fx::this_layer_effect_time(
            &layer.effects,
            layer.switches.fx,
            lt,
            layer.start_offset.0.to_f64(),
        );
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
                    let mut inner =
                        build_comp_draws_at(doc, nested, lt, frame_lt, pixels_by_layer, visited);
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
                        // Per-layer motion blur on an inner layer of a collapsed
                        // Precomp is a follow-up (docs/06 §4, K-120): the export
                        // splice (collect_collapsed) carries no sub-frame
                        // samples, so clearing them here keeps preview and export
                        // identical (K-031). A non-collapsed Precomp layer still
                        // blurs via its own switch on the main path.
                        d.mb = Vec::new();
                        // A Posterize Time adjustment inside a collapsed Precomp
                        // is a follow-up too (docs/08 §3.25): its held below-draws
                        // were sized for the nested comp, so splicing them into
                        // the parent would mis-size the re-render. Clear it — the
                        // effect degrades to a no-op here, a documented boundary;
                        // a non-collapsed Precomp posterises on its own path.
                        // Accumulation motion blur (§3.26) takes the same boundary
                        // for the same sizing reason.
                        d.temporal_below = None;
                        d.accumulation_below = None;
                    }
                    draws.extend(inner);
                    continue;
                }
                visited.push(*nested_id);
                let nested_draws =
                    build_comp_draws_at(doc, nested, lt, frame_lt, pixels_by_layer, visited);
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
                    // constructor export uses (K-031). Effects flagged
                    // sample_temporally == false resolve at the frame time in a
                    // held re-render (§5); equal to `lt` on an ordinary render.
                    let markers = lumit_core::fx::MarkerContext::for_layer(comp, layer);
                    lumit_core::fx::resolve_stack_temporal(
                        &layer.effects,
                        effect_lt,
                        frame_lt,
                        comp_diag,
                        1.0,
                        &markers,
                    )
                } else {
                    Vec::new()
                };
                // Posterize Time everything-below (docs/08 §3.25): the below
                // stack re-rendered at the held time, built by the shared
                // `below_draws_at` export also drives (K-031). A Posterize Time
                // effect has no Resolved op, so this — not `fx` — is what makes
                // such an adjustment live. `frame_t` carries the playhead through
                // so the held below honours sample_temporally too (§5).
                let temporal_below = posterize_below(
                    doc,
                    comp,
                    layer,
                    idx,
                    t_comp,
                    frame_t,
                    pixels_by_layer,
                    visited,
                );
                // Accumulation motion blur everything-below (docs/08 §3.26): N
                // sub-frame below-stacks realise averages, standing in for the
                // plain below-composite. Like Posterize it resolves to no op, so
                // this — not `fx` — is what keeps such an adjustment live.
                let accumulation_below = accumulation_mb_below(
                    doc,
                    comp,
                    layer,
                    idx,
                    t_comp,
                    frame_t,
                    pixels_by_layer,
                    visited,
                );
                if fx.is_empty() && temporal_below.is_none() && accumulation_below.is_none() {
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
                    // Ordered file paths of the enabled built-in `lut` effects,
                    // 1:1 with the stack's Resolved::Lut ops (docs/08 §3.11);
                    // the same `lt` resolve_stack used above.
                    lut_files: lut_files(&layer.effects, lt),
                    // Depth inputs of the enabled built-in `dof` effects, 1:1
                    // with the stack's Resolved::Dof ops (docs/08 §3.22).
                    dof_inputs: dof_inputs_for(&layer.effects),
                    // An adjustment layer is a staging point, not a picture —
                    // motion blur has no image of its own to smear (docs/06 §4).
                    mb: Vec::new(),
                    temporal_below,
                    accumulation_below,
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
            // After-effects matte (K-decision): resolve the matte source's own
            // stack at its layer time so gpu.rs runs it on the matte texture
            // before the matte gates the consumer. Uses the source's decode scale
            // (its px@comp radii stay honest under reduced-res preview), the same
            // §1.4 markers and the same resolve export uses (K-031). Empty when
            // the toggle is off or the source's fx switch is off — source-only.
            let (fx, lut_files) = if mr.after_effects && src.switches.fx {
                let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let scale = m_w as f32 / m_nat.0.max(1.0);
                let markers = lumit_core::fx::MarkerContext::for_layer(comp, src);
                (
                    lumit_core::fx::resolve_stack(
                        &src.effects,
                        mlt,
                        comp_diag * scale,
                        scale,
                        &markers,
                    ),
                    lut_files(&src.effects, mlt),
                )
            } else {
                (Vec::new(), Vec::new())
            };
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
                fx,
                lut_files,
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
                // constructor export uses (K-031). In a held/sub-frame temporal
                // re-render, an effect flagged sample_temporally == false stays
                // at the frame time `frame_lt` (§5); on an ordinary render
                // `frame_lt == lt`, so this is the plain resolve.
                let markers = lumit_core::fx::MarkerContext::for_layer(comp, layer);
                lumit_core::fx::resolve_stack_temporal(
                    &layer.effects,
                    effect_lt,
                    frame_lt,
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
        // The dense motion field for Fast motion blur, carried from the same
        // decode job (its `(u, v, conf)` are at the layer's decoded size).
        let flow_field = pixels_by_layer.get(&layer.id).and_then(|lp| {
            lp.flow_field
                .as_ref()
                .map(|(u, v, conf)| (u.clone(), v.clone(), conf.clone(), lp.width, lp.height))
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
            // Ordered file paths of the enabled built-in `lut` effects, 1:1
            // with the stack's Resolved::Lut ops (docs/08 §3.11); the same `lt`
            // resolve_stack used for `fx`.
            lut_files: lut_files(&layer.effects, lt),
            // Depth inputs of the enabled built-in `dof` effects, 1:1 with the
            // stack's Resolved::Dof ops (docs/08 §3.22); built the same way
            // export does, so the two blur identically (K-031).
            dof_inputs: dof_inputs_for(&layer.effects),
            // Per-layer motion blur (docs/06 §4, K-120): the layer's own
            // transform sampled across the open shutter, empty unless it blurs.
            // Built the same way export does, so the two smear identically.
            mb: motion_blur_samples(comp, layer, t_comp),
            // Ordinary layers never carry a temporal re-render — that is an
            // adjustment-only capability in v1 (docs/08 §3.25, §3.26).
            temporal_below: None,
            accumulation_below: None,
        });
    }
    draws
}

/// The ordered file paths of a layer's enabled built-in `lut` effects
/// (docs/08 §3.11, K-114), each resolved at layer time `lt` (None = unset).
/// `resolve_stack` filters on the identical `e.enabled && namespace == Builtin`
/// predicate and preserves order, and a `lut` effect always resolves to exactly
/// one `Resolved::Lut`, so this list is 1:1 and in the same order as the stack's
/// `Resolved::Lut` ops — the alignment `run_ops` relies on to bind LUT k to op
/// k. Preview (here) and export build it the same way, so the two match (K-031).
#[cfg(feature = "media")]
fn lut_files(effects: &[lumit_core::model::EffectInstance], lt: f64) -> Vec<Option<String>> {
    use lumit_core::model::EffectNamespace;
    effects
        .iter()
        .filter(|e| {
            e.enabled
                && e.effect.namespace == EffectNamespace::Builtin
                && e.effect.match_name == "lut"
        })
        .map(|e| e.path_at("file", lt).map(str::to_owned))
        .collect()
}

/// Render the composite of `below` (the layers beneath a temporal adjustment,
/// in document order) at the held/sample comp time `tau`, reusing the SAME
/// decoded `pixels_by_layer` — footage frames are held; only transforms,
/// effects and the camera re-resolve at `tau` (docs/impl/temporal-rerender.md
/// §2). This is the one re-render both the preview (`build_comp_draws` +
/// [`Realiser::realise`]) and export drive, so a Posterize Time (and, later,
/// accumulation motion blur) frame is identical in the viewport and the file
/// (K-031). Re-resolving decodes nothing: the same held pixels are reused, so
/// the decode planner is never re-entered (docs/impl/temporal-rerender.md
/// Traps).
///
/// Temporal effects inside the below-stack (echo, flow motion blur, datamosh)
/// are held to a still here — their neighbour frames and flow fields are
/// dropped by [`strip_temporal_inputs`], because the held re-render reuses the
/// frame-time decode and export carries no neighbour decode for it. A
/// documented v1 boundary (docs/08 §3.25), matching the after-effects matte's
/// own temporal boundary (K-125).
///
/// `frame_t` is the true playhead, threaded so an effect in the below-stack
/// flagged `sample_temporally == false` holds at the frame time rather than
/// `tau` (docs/impl/temporal-rerender.md §5); for a plain re-render at the same
/// time the caller passes `frame_t == tau`.
#[cfg(feature = "media")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_below_at(
    realiser: &Realiser,
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    below: &[lumit_core::model::Layer],
    tau: f64,
    frame_t: f64,
    force_mb: Option<lumit_core::model::MotionBlur>,
    pixels_by_layer: &std::collections::HashMap<
        uuid::Uuid,
        &crate::app_state::preview::CompLayerPixels,
    >,
    visited: &mut Vec<uuid::Uuid>,
) -> egui_wgpu::wgpu::Texture {
    let (draws, camera) = below_draws_at(
        doc,
        comp,
        below,
        tau,
        frame_t,
        force_mb,
        pixels_by_layer,
        visited,
    );
    let background = comp.background.0.map(f64::from);
    realiser.realise(camera, comp.width, comp.height, background, &draws)
}

/// Build the below-stack's draw list at the held/sample comp time `tau`, plus
/// the comp's camera at `tau` — the shared CPU step both the preview (embedded
/// on the adjustment draw as [`TemporalBelow`]) and export (`render_below_at`)
/// drive, so the two re-render the identical stack (K-031). Footage is held
/// (the same `pixels_by_layer`); temporal effects in the below-stack are
/// dropped to stills ([`strip_temporal_inputs`]).
#[cfg(feature = "media")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn below_draws_at(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    below: &[lumit_core::model::Layer],
    tau: f64,
    frame_t: f64,
    force_mb: Option<lumit_core::model::MotionBlur>,
    pixels_by_layer: &std::collections::HashMap<
        uuid::Uuid,
        &crate::app_state::preview::CompLayerPixels,
    >,
    visited: &mut Vec<uuid::Uuid>,
) -> (Vec<CompLayerDraw>, Option<lumit_core::model::CameraPose>) {
    // A below-only view of the comp: the same size, background, frame rate,
    // markers and camera, but only the layers beneath the adjustment. The
    // camera is read from the original comp at `tau` (a Camera layer inside
    // `below` draws nothing itself). `frame_t` is the true playhead, so an effect
    // in the below-stack flagged sample_temporally == false holds at the frame
    // time instead of the sample time `tau` (docs/impl/temporal-rerender.md §5).
    let mut below_comp = comp.clone();
    below_comp.layers = below.to_vec();
    // Accumulation MB *Force on all layers* (docs/08 §3.26): drop the effect's
    // shutter onto this SAMPLE-ONLY comp clone and turn every layer's own
    // motion-blur switch on, so per-layer motion blur (K-120) smears each layer
    // in every sub-frame sample — the real comp is never touched. None leaves
    // the sample render exactly as before (Posterize, or accumulation without
    // the toggle).
    if let Some(mb) = force_mb {
        below_comp.motion_blur = mb;
        for l in &mut below_comp.layers {
            l.switches.motion_blur = true;
        }
    }
    let mut draws = build_comp_draws_at(doc, &below_comp, tau, frame_t, pixels_by_layer, visited);
    strip_temporal_inputs(&mut draws);
    (draws, comp.camera_pose(tau))
}

/// The held below-stack for a temporal adjustment (Posterize Time everything-
/// below, docs/08 §3.25), or None when `layer` carries no such effect. `idx` is
/// the layer's document index, so the below-set is `comp.layers[idx + 1..]`
/// (everything lower in the stack). The *this layer's effects* scope is not an
/// adjustment re-render (it substitutes time into the layer's own stack, a
/// later step), so it returns None here. Shared by the preview
/// (`build_comp_draws`) and export, which detects the same effect in
/// `render_comp_linear` and calls [`render_below_at`] directly.
#[cfg(feature = "media")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn posterize_below(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    idx: usize,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<
        uuid::Uuid,
        &crate::app_state::preview::CompLayerPixels,
    >,
    visited: &mut Vec<uuid::Uuid>,
) -> Option<TemporalBelow> {
    let lt = t_comp - layer.start_offset.0.to_f64();
    let p = lumit_core::fx::stack_posterize(&layer.effects, layer.switches.fx, lt)?;
    if p.scope != lumit_core::fx::PosterizeScope::EverythingBelow {
        return None;
    }
    let tau = lumit_core::fx::posterize_held_time(t_comp, p.rate, p.phase);
    let below = &comp.layers[idx + 1..];
    // Posterize never forces per-layer motion blur (that is accumulation MB's
    // Force on all layers).
    let (draws, camera) = below_draws_at(
        doc,
        comp,
        below,
        tau,
        frame_t,
        None,
        pixels_by_layer,
        visited,
    );
    Some(TemporalBelow { draws, camera })
}

/// The N sub-frame below-stacks for an accumulation motion blur adjustment
/// (docs/08 §3.26, docs/impl/temporal-rerender.md §3), or None when `layer`
/// carries no such effect (or its Samples < 2, which is no blur — the adjustment
/// then falls back to the plain below-composite). `idx` is the layer's document
/// index, so the below-set is `comp.layers[idx + 1..]`. Each sample time is
/// `τ_k = t_comp + off_k·dt` with the offsets from [`lumit_core::fx::
/// AccumulationMbParams::sample_offsets`] (the shared per-layer motion-blur
/// shutter maths), and each below-stack is built by the same `below_draws_at`
/// export drives, so preview equals export (K-031). `frame_t` threads the
/// playhead so a sample_temporally == false effect in the below-stack still holds
/// at the frame time (§5).
#[cfg(feature = "media")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn accumulation_mb_below(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    idx: usize,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<
        uuid::Uuid,
        &crate::app_state::preview::CompLayerPixels,
    >,
    visited: &mut Vec<uuid::Uuid>,
) -> Option<AccumulationBelow> {
    let lt = t_comp - layer.start_offset.0.to_f64();
    let p = lumit_core::fx::stack_accumulation_mb(&layer.effects, layer.switches.fx, lt)?;
    let offsets = p.sample_offsets();
    if offsets.is_empty() {
        return None;
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    // Force on all layers (docs/08 §3.26): when set, every layer in each sample
    // render also smears along its own transform (the effect's shutter forced on
    // the sample-only comp clone). None otherwise, so the samples render plainly.
    let force_mb = p.forced_layer_mb();
    let below = &comp.layers[idx + 1..];
    let samples = offsets
        .iter()
        .map(|off| {
            let tau = t_comp + off * dt;
            below_draws_at(
                doc,
                comp,
                below,
                tau,
                frame_t,
                force_mb,
                pixels_by_layer,
                visited,
            )
        })
        .collect();
    Some(AccumulationBelow {
        samples,
        mix: p.mix as f32,
    })
}

/// Drop the neighbour frames and flow field a temporal effect reads, recursing
/// into nested-comp draws — so a held/sub-frame re-render treats echo, flow
/// motion blur and datamosh as stills and the preview matches export, which
/// carries no such decode for the re-render (docs/impl/temporal-rerender.md
/// Traps). Spatial effects (blur, glow, colour, transform) are untouched, so a
/// posterised or motion-blurred scene still holds its full spatial animation.
#[cfg(feature = "media")]
fn strip_temporal_inputs(draws: &mut [CompLayerDraw]) {
    for d in draws.iter_mut() {
        d.neighbours = Vec::new();
        d.flow_field = None;
        if let DrawSource::Nested { draws: inner, .. } = &mut d.source {
            strip_temporal_inputs(inner);
        }
    }
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
            motion_blur: Default::default(),
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

#[cfg(all(test, feature = "media"))]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod render_below_at_tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::{
        Composition, Document, Layer, LayerKind, LinearColour, Switches, TextDocument,
        TransformGroup,
    };
    use lumit_core::time::{CompTime, Duration, FrameRate, Rational};
    use std::collections::HashMap;
    use uuid::Uuid;

    fn text_layer(x: f64) -> Layer {
        Layer {
            id: Uuid::now_v7(),
            name: "t".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: "hello".into(),
                    size: 48.0,
                    fill: LinearColour([1.0, 0.5, 0.2, 1.0]),
                    extra: serde_json::Map::new(),
                },
            },
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup {
                position_x: Property::fixed(x),
                position_y: Property::fixed(60.0),
                ..TransformGroup::default()
            },
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    // docs/impl/temporal-rerender.md §7 step 1: a re-render of a still scene at
    // the SAME time must be bit-identical to compositing it the normal way.
    // `render_below_at` — the one shared re-render helper — reuses
    // `build_comp_draws` and `Realiser::realise`, so at `tau == t` it must
    // reproduce the plain composite exactly. This is the identity the whole
    // preview==export promise (K-031) rests on; it is proved before anything is
    // built on top of the helper.
    #[test]
    fn still_scene_rerender_at_same_time_is_bit_identical() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            return; // no GPU here — skip, exactly as the gpu crate's own tests do
        };
        let engine = lumit_gpu::ColourEngine::new(&ctx);
        let compositor = lumit_gpu::Compositor::new(&ctx);
        let fx = lumit_gpu::fx::FxEngine::new(&ctx);
        let lut_cache = std::cell::RefCell::new(HashMap::new());
        let realiser = Realiser {
            ctx: lumit_gpu::GpuContext::from_parts(ctx.device.clone(), ctx.queue.clone()),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            work_area: None,
            layers: vec![text_layer(200.0), text_layer(80.0)],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        // Compute the background exactly as render_below_at does (from the f32
        // LinearColour via f64::from), so the plain composite and the re-render
        // clear to identical values and the comparison is honest.
        let bg = comp.background.0.map(f64::from);
        let t = 0.3;

        let mut v1 = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, t, &pixels, &mut v1);
        let normal = realiser.realise(comp.camera_pose(t), comp.width, comp.height, bg, &draws);
        let normal_bytes = engine
            .readback8(&ctx, &engine.display(&ctx, &normal))
            .unwrap();

        // Re-render the whole stack (every layer counts as "below") at the same
        // time through the shared helper.
        let mut v2 = vec![comp.id];
        let below = render_below_at(
            &realiser,
            &doc,
            &comp,
            &comp.layers,
            t,
            t,
            None,
            &pixels,
            &mut v2,
        );
        let below_bytes = engine
            .readback8(&ctx, &engine.display(&ctx, &below))
            .unwrap();

        assert_eq!(
            normal_bytes, below_bytes,
            "render_below_at at tau == t must reproduce the plain composite bit-for-bit"
        );
    }

    // A property that ramps linearly from `from` at t=0 to `to` at t=1, so a
    // held time differs visibly from the frame time.
    fn ramp(from: f64, to: f64) -> Property {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        let key = |time: Rational, value: f64| Keyframe {
            time,
            value,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        Property {
            animation: Animation::Keyframed(vec![
                key(Rational::ZERO, from),
                key(Rational::new(1, 1).unwrap(), to),
            ]),
            extra: serde_json::Map::new(),
        }
    }

    // An adjustment layer carrying a Posterize Time effect (everything-below) at
    // the given posterised frame rate.
    fn posterize_adjustment(rate: f64) -> Layer {
        let mut post = lumit_core::fx::instantiate("posterize_time").unwrap();
        for p in &mut post.params {
            if p.id == "rate" {
                p.value = lumit_core::model::EffectValue::Float(Property::fixed(rate));
            }
        }
        Layer {
            id: Uuid::now_v7(),
            name: "posterize".into(),
            kind: LayerKind::Adjustment,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: vec![post],
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn posterize_comp() -> Composition {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 100.0); // x = 100·t
        Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            work_area: None,
            // Adjustment on top (index 0), the animated text below (index 1).
            layers: vec![posterize_adjustment(10.0), text],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    // docs/08 §3.25: a Posterize Time adjustment (everything-below) is detected
    // at build_comp_draws time and carries a held below-stack. The held draws
    // must sit at the posterised time tau = 0.3 (x = 30), NOT the frame time
    // 0.35 (x = 35) — proving the below re-resolves at the held grid, not the
    // playhead. A GPU-free structural check (the moving-scene coverage).
    #[test]
    fn posterize_adjustment_holds_the_below_stack_at_the_grid_time() {
        let comp = posterize_comp();
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        // t = 0.35, 10 fps grid → held tau = floor(3.5)/10 = 0.3.
        let draws = build_comp_draws(&doc, &comp, 0.35, &pixels, &mut visited);
        let adj = draws
            .iter()
            .find(|d| matches!(d.source, DrawSource::Adjust))
            .expect("the posterize adjustment emits a staging draw");
        let tb = adj
            .temporal_below
            .as_ref()
            .expect("an everything-below posterize carries a held below-stack");
        assert_eq!(tb.draws.len(), 1, "the one text layer below is held");
        assert!(
            (tb.draws[0].position.0 - 30.0).abs() < 0.01,
            "held at tau = 0.3 (x = 30), not the frame time (x = 35); got {}",
            tb.draws[0].position.0
        );
    }

    // docs/impl/temporal-rerender.md §5: an effect in the held below-stack flagged
    // sample_temporally == false stays pinned to the frame time while the scene's
    // transforms sample the held time. The text below carries a blur whose radius
    // ramps 0%→100% over a second and opts out of sampling; under a 10 fps
    // posterise at t = 0.35 (held tau = 0.3) its transform holds at x = 30 but its
    // blur resolves at the frame time 0.35 (35% of the diagonal), not 0.3.
    #[test]
    fn a_non_sampling_below_effect_holds_at_the_frame_time_not_the_grid() {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 100.0); // x = 100·t
        let mut blur = lumit_core::fx::instantiate("blur").unwrap();
        blur.sample_temporally = false;
        for p in &mut blur.params {
            if p.id == "radius" {
                p.value = lumit_core::model::EffectValue::Float(ramp(0.0, 100.0));
                // radius% = 100·t
            }
        }
        text.effects = vec![blur];
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            work_area: None,
            layers: vec![posterize_adjustment(10.0), text],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.35, &pixels, &mut visited);
        let adj = draws
            .iter()
            .find(|d| matches!(d.source, DrawSource::Adjust))
            .expect("the posterize adjustment emits a staging draw");
        let tb = adj
            .temporal_below
            .as_ref()
            .expect("an everything-below posterize carries a held below-stack");
        assert_eq!(tb.draws.len(), 1, "the one text layer below is held");
        // The transform samples the held time (x = 30).
        assert!(
            (tb.draws[0].position.0 - 30.0).abs() < 0.01,
            "transform held at tau = 0.3; got {}",
            tb.draws[0].position.0
        );
        // The blur, opting out, resolves at the frame time 0.35 (35% of diag).
        let diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
        let radius = match tb.draws[0].fx.first() {
            Some(lumit_core::fx::Resolved::Blur { radius_px, .. }) => *radius_px,
            other => panic!("expected a blur op, got {other:?}"),
        };
        assert!(
            (radius - 0.35 * diag).abs() < 0.5,
            "blur must hold at the frame time 0.35 ({}), got {radius}",
            0.35 * diag
        );
        assert!(
            (radius - 0.30 * diag).abs() > 5.0,
            "blur must NOT sample the held time 0.30; got {radius}"
        );
    }

    // docs/08 §3.25: a Posterize time scoped to *This layer's effects* holds only
    // the layer's OWN effect stack on the coarse grid — no re-render of others,
    // no adjustment (no orchestration re-entry). The text carries a blur (radius
    // ramps 0%→100% over a second) and a 10 fps this-layer Posterize; at t = 0.35
    // its transform stays live (x = 35) while the blur resolves at the held time
    // 0.3 (30% of the diagonal), not 0.35. GPU-free structural check.
    #[test]
    fn this_layer_posterize_holds_the_layers_own_effects_but_not_its_transform() {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 100.0); // x = 100·t (stays live)
        let mut blur = lumit_core::fx::instantiate("blur").unwrap();
        for p in &mut blur.params {
            if p.id == "radius" {
                p.value = lumit_core::model::EffectValue::Float(ramp(0.0, 100.0));
                // % = 100·t
            }
        }
        let mut post = lumit_core::fx::instantiate("posterize_time").unwrap();
        for p in &mut post.params {
            match p.id.as_str() {
                "rate" => p.value = lumit_core::model::EffectValue::Float(Property::fixed(10.0)),
                // 1 = This layer's effects.
                "scope" => p.value = lumit_core::model::EffectValue::Choice(1),
                _ => {}
            }
        }
        text.effects = vec![blur, post];
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            work_area: None,
            layers: vec![text],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.35, &pixels, &mut visited);
        let d = draws
            .iter()
            .find(|d| !matches!(d.source, DrawSource::Adjust))
            .expect("the text layer draws");
        // The transform stays at the playhead — only the effects are held.
        assert!(
            (d.position.0 - 35.0).abs() < 0.01,
            "transform live at t = 0.35 (x = 35); got {}",
            d.position.0
        );
        // The blur resolves at the held time 0.3 (30% of diag), not 0.35.
        let diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
        let radius = match d.fx.first() {
            Some(lumit_core::fx::Resolved::Blur { radius_px, .. }) => *radius_px,
            other => panic!("expected a blur op, got {other:?}"),
        };
        assert!(
            (radius - 0.30 * diag).abs() < 0.5,
            "blur held at the grid time 0.3 ({}); got {radius}",
            0.30 * diag
        );
        assert!(
            (radius - 0.35 * diag).abs() > 5.0,
            "blur must NOT resolve at the live time 0.35; got {radius}"
        );
        // The Posterize itself has no per-pixel op — only the blur survives.
        assert_eq!(
            d.fx.len(),
            1,
            "posterize resolves to nothing; only the blur"
        );
    }

    // docs/08 §3.25 + K-031: the whole preview Posterize path (detect → held
    // below → adjustment blend) must reduce, at full coverage, to a plain render
    // of the below-stack at the held time. So a posterised frame at t = 0.35
    // equals `render_below_at` at tau = 0.3 bit-for-bit — the moving-scene
    // pixel check. (If the code held at the frame time instead, the two would
    // differ, because the text has moved between 0.3 and 0.35.)
    #[test]
    fn posterised_frame_equals_a_plain_render_at_the_held_time() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            return;
        };
        let engine = lumit_gpu::ColourEngine::new(&ctx);
        let compositor = lumit_gpu::Compositor::new(&ctx);
        let fx = lumit_gpu::fx::FxEngine::new(&ctx);
        let lut_cache = std::cell::RefCell::new(HashMap::new());
        let realiser = Realiser {
            ctx: lumit_gpu::GpuContext::from_parts(ctx.device.clone(), ctx.queue.clone()),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
        };
        let comp = posterize_comp();
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        let bg = comp.background.0.map(f64::from);

        // The posterised frame at t = 0.35.
        let mut v1 = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.35, &pixels, &mut v1);
        let posterised =
            realiser.realise(comp.camera_pose(0.35), comp.width, comp.height, bg, &draws);
        let posterised_bytes = engine
            .readback8(&ctx, &engine.display(&ctx, &posterised))
            .unwrap();

        // A plain render of the below-stack (just the text) at tau = 0.3.
        let below = &comp.layers[1..];
        let mut v2 = vec![comp.id];
        // frame_t = 0.35 matches what the posterise adjustment passes (its own
        // frame time), so the two below-renders build the identical draws.
        let held = render_below_at(
            &realiser, &doc, &comp, below, 0.3, 0.35, None, &pixels, &mut v2,
        );
        let held_bytes = engine
            .readback8(&ctx, &engine.display(&ctx, &held))
            .unwrap();

        assert_eq!(
            posterised_bytes, held_bytes,
            "a full-coverage posterised frame must equal a plain render at the held time"
        );
    }

    // An adjustment layer carrying an accumulation motion blur effect at the
    // given sample count (defaults otherwise: 180° shutter centred on the frame).
    fn accumulation_adjustment(samples: f64) -> Layer {
        let mut e = lumit_core::fx::instantiate("accumulation_mb").unwrap();
        for p in &mut e.params {
            if p.id == "samples" {
                p.value = lumit_core::model::EffectValue::Float(Property::fixed(samples));
            }
        }
        Layer {
            id: Uuid::now_v7(),
            name: "accumulation".into(),
            kind: LayerKind::Adjustment,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            blend: Default::default(),
            masks: Vec::new(),
            effects: vec![e],
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn comp_with(fps: u32, layers: Vec<Layer>) -> Composition {
        Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(fps, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        }
    }

    // docs/08 §3.26: an accumulation motion blur adjustment carries N sub-frame
    // below-stacks, one per shutter sample, centred on the frame. A moving text
    // (x = 200·t) in a 2 fps comp (dt = 0.5 s) spreads visibly; at t = 0.5 the 4
    // below-stacks straddle x = 100, their positions strictly increasing across
    // the centred shutter. GPU-free structural check.
    #[test]
    fn accumulation_adjustment_holds_n_subframe_below_stacks_centred_on_the_frame() {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 200.0); // x = 200·t
        let comp = comp_with(2, vec![accumulation_adjustment(4.0), text]);
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, 0.5, &pixels, &mut visited);
        let adj = draws
            .iter()
            .find(|d| matches!(d.source, DrawSource::Adjust))
            .expect("the accumulation adjustment emits a staging draw");
        let ab = adj
            .accumulation_below
            .as_ref()
            .expect("an accumulation adjustment carries N sub-frame below-stacks");
        assert_eq!(ab.samples.len(), 4, "one below-stack per shutter sample");
        let xs: Vec<f32> = ab
            .samples
            .iter()
            .map(|(draws, _)| draws[0].position.0)
            .collect();
        // Strictly increasing (the centred shutter sweeps forward in time).
        assert!(
            xs.windows(2).all(|w| w[0] < w[1]),
            "sub-frame positions increase across the shutter: {xs:?}"
        );
        // Centred on the frame: the samples straddle x = 100 (the frame-time
        // position at t = 0.5).
        assert!(
            xs[0] < 100.0 && *xs.last().unwrap() > 100.0,
            "the shutter is centred on x = 100: {xs:?}"
        );
        assert!((ab.mix - 1.0).abs() < 1e-6, "full Mix by default");
    }

    // docs/08 §3.26 + K-031: a still scene averaged over N is bit-identical to the
    // plain composite (the accumulation adjustment is a pure identity when nothing
    // moves), while a moving scene smears — differs from the plain composite and
    // covers a wider horizontal extent. The same combine drives the export path.
    #[test]
    fn accumulation_still_scene_is_identity_and_moving_scene_smears() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = lumit_gpu::ColourEngine::new(&ctx);
        let compositor = lumit_gpu::Compositor::new(&ctx);
        let fx = lumit_gpu::fx::FxEngine::new(&ctx);
        let lut_cache = std::cell::RefCell::new(HashMap::new());
        let realiser = Realiser {
            ctx: lumit_gpu::GpuContext::from_parts(ctx.device.clone(), ctx.queue.clone()),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
        };
        let doc = Document::new();
        let pixels: HashMap<Uuid, &crate::app_state::preview::CompLayerPixels> = HashMap::new();
        let render = |comp: &Composition, t: f64| -> Vec<u8> {
            let mut v = vec![comp.id];
            let draws = build_comp_draws(&doc, comp, t, &pixels, &mut v);
            let bg = comp.background.0.map(f64::from);
            let tex = realiser.realise(comp.camera_pose(t), comp.width, comp.height, bg, &draws);
            engine.readback8(&ctx, &engine.display(&ctx, &tex)).unwrap()
        };

        // STILL scene: a static text below a 4-sample accumulation adjustment must
        // be a bit-exact identity — every sub-frame render is equal, so their
        // average is the plain composite (1/4 is exact in fp16, four copies sum
        // back exactly), and the full-coverage blend lays it back unchanged.
        let still_text = text_layer(120.0);
        let still_plain = comp_with(30, vec![still_text.clone()]);
        let still_acc = comp_with(30, vec![accumulation_adjustment(4.0), still_text]);
        assert_eq!(
            render(&still_plain, 0.5),
            render(&still_acc, 0.5),
            "a still scene averaged over N must equal the plain composite bit-for-bit"
        );

        // MOVING scene: text sweeping x = 200·t in a 2 fps comp (dt = 0.5 s) so the
        // shutter spreads ~37 px. The accumulation frame must differ from the plain
        // composite (the smear) and cover a wider horizontal extent.
        let mut moving_text = text_layer(0.0);
        moving_text.transform.position_x = ramp(0.0, 200.0);
        let moving_plain = comp_with(2, vec![moving_text.clone()]);
        let moving_acc = comp_with(2, vec![accumulation_adjustment(4.0), moving_text]);
        let plain = render(&moving_plain, 0.5);
        let smeared = render(&moving_acc, 0.5);
        assert_ne!(
            plain, smeared,
            "a moving scene must smear (differ from the plain composite)"
        );
        // Columns carrying visible text (red well above the dark background).
        let (w, h) = (320usize, 180usize);
        let text_cols = |b: &[u8]| {
            (0..w)
                .filter(|&x| (0..h).any(|y| b[(y * w + x) * 4] > 130))
                .count()
        };
        assert!(
            text_cols(&smeared) > text_cols(&plain),
            "the smear must widen the covered columns: plain {}, smeared {}",
            text_cols(&plain),
            text_cols(&smeared)
        );
    }
}
