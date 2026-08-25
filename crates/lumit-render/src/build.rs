//! Turning a document into a draw list.
//!
//! # In plain terms
//!
//! This is the step that reads the project and works out what the frame should
//! look like: where each layer sits at this instant, how opaque it is, which
//! blend mode it uses, what its effect stack resolves to in plain numbers, which
//! layer mattes it, and how its masks cover it. Nested comps recurse; a
//! collapsed Precomp splices its contents straight into the parent list; an
//! adjustment layer becomes a staging marker the compositor splits on.
//!
//! Nothing here decodes video or touches the graphics card. The already-decoded
//! pixels arrive as `pixels_by_layer` and are simply *referred to*. That is what
//! makes a live value drag cheap: the drag patches the composition, re-runs this
//! builder against the pixels it already has, and re-composites — no file is
//! read again (see [`crate::plan::same_decode`]).
//!
//! Preview and export both build through here, so a comp cannot look different
//! in the viewport and the file (K-031).

use crate::decode::CompLayerPixels;
use crate::draw::{
    AccumulationBelow, CompLayerDraw, DofInputDraw, DrawSource, LayerInputDraw, MatteDraw,
    TemporalBelow,
};
use crate::export::mask_rgba;
use crate::realise::Realiser;
use lumit_core::expression::ExpressionContext;
use lumit_core::pixels::{px_tile, solid_rgba};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// One layer's source pixels ready for a draw: `(rgba, tex_w, tex_h, natural
/// size)`. The natural size is the layer's *own* pixel size, which is what
/// transforms act in — never the decoded size, which shrinks and grows with the
/// preview resolution.
pub type LayerPixels = (Vec<u8>, u32, u32, (f32, f32));

/// The map of already-decoded pixels a build reads, keyed by layer id.
pub type PixelsByLayer<'a> = HashMap<Uuid, &'a CompLayerPixels>;

/// The single `model::BlendMode` → `gpu::Blend` mapping shared by every path
/// that composites (K-031: they must never disagree). Every mode maps to its
/// like-named GPU variant (K-162, T24).
#[must_use]
pub fn blend_of(b: lumit_core::model::BlendMode) -> lumit_gpu::Blend {
    use lumit_core::model::BlendMode as M;
    use lumit_gpu::Blend as G;
    match b {
        M::Normal => G::Normal,
        M::Add => G::Add,
        M::Multiply => G::Multiply,
        M::Screen => G::Screen,
        M::Overlay => G::Overlay,
        M::SoftLight => G::SoftLight,
        M::HardLight => G::HardLight,
        M::Lighten => G::Lighten,
        M::Darken => G::Darken,
        M::Subtract => G::Subtract,
        M::ColourBurn => G::ColourBurn,
        M::LinearBurn => G::LinearBurn,
        M::DarkerColour => G::DarkerColour,
        M::ColourDodge => G::ColourDodge,
        M::LighterColour => G::LighterColour,
        M::LinearLight => G::LinearLight,
        M::VividLight => G::VividLight,
        M::PinLight => G::PinLight,
        M::HardMix => G::HardMix,
        M::Difference => G::Difference,
        M::Exclusion => G::Exclusion,
        M::Divide => G::Divide,
        M::Hue => G::Hue,
        M::Saturation => G::Saturation,
        M::Colour => G::Colour,
        M::Luminosity => G::Luminosity,
    }
}

/// A copy of `comp` with one layer's transform property overridden to a fixed
/// `value` — the live value-drag preview renders this so the provisional value
/// shows before the edit is committed. Only the previewed frame is rendered, so
/// pinning the property to a constant is exactly its value at that instant.
pub fn patch_layer_prop(
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
pub fn patch_layer_effect_param(
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
pub fn parent_world_placement(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
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
        let alt = lumit_core::time::layer_time(t_comp, a.start_offset.0);
        let tr = &a.transform;
        // An ancestor's own expressions are about the ancestor: `layer()`,
        // `cut_in` and `cut_out` must resolve to the layer being evaluated, not
        // to the child that started the walk up the chain.
        let context = Arc::new(ExpressionContext {
            layer: Some(a.id),
            ..(*context).clone()
        });
        let p = lumit_gpu::place_matrix(
            (
                tr.position_x.value_at_with_context(alt, context.clone()) as f32,
                tr.position_y.value_at_with_context(alt, context.clone()) as f32,
            ),
            (
                tr.anchor_x.value_at_with_context(alt, context.clone()) as f32,
                tr.anchor_y.value_at_with_context(alt, context.clone()) as f32,
            ),
            (
                tr.scale_x.value_at_with_context(alt, context.clone()) as f32,
                tr.scale_y.value_at_with_context(alt, context.clone()) as f32,
            ),
            tr.rotation.value_at_with_context(alt, context.clone()) as f32,
            tr.position_z.value_at_with_context(alt, context.clone()) as f32,
            tr.rotation_x.value_at_with_context(alt, context.clone()) as f32,
            tr.rotation_y.value_at_with_context(alt, context.clone()) as f32,
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
/// every caller of the one comp walk (K-031) so all paths smear identically. Parent motion within the shutter is a
/// follow-up: only the layer's OWN transform is sampled here — a parented
/// layer keeps its frame-time parent placement (`pre`) for every sub-copy.
pub fn motion_blur_samples(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> Vec<lumit_gpu::MbSample> {
    if !layer.switches.motion_blur {
        return Vec::new();
    }
    let offsets = comp.motion_blur.sample_offsets();
    if offsets.is_empty() {
        return Vec::new();
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let tr = &layer.transform;

    offsets
        .iter()
        .map(|off| {
            let lt = lumit_core::time::layer_time(t_comp + off * dt, layer.start_offset.0);
            // Each shutter sample is a different moment, so an expression that
            // reads `time` has to be evaluated at that moment. Reusing the
            // frame's context would return the same placement for every sample
            // and an expression-driven layer would simply not smear.
            let context = Arc::new(ExpressionContext {
                comp_time: t_comp + off * dt,
                ..(*context).clone()
            });
            lumit_gpu::MbSample {
                position: (
                    tr.position_x.value_at_with_context(lt, context.clone()) as f32,
                    tr.position_y.value_at_with_context(lt, context.clone()) as f32,
                ),
                anchor: (
                    tr.anchor_x.value_at_with_context(lt, context.clone()) as f32,
                    tr.anchor_y.value_at_with_context(lt, context.clone()) as f32,
                ),
                scale: (
                    tr.scale_x.value_at_with_context(lt, context.clone()) as f32,
                    tr.scale_y.value_at_with_context(lt, context.clone()) as f32,
                ),
                rotation_deg: tr.rotation.value_at_with_context(lt, context.clone()) as f32,
                z: tr.position_z.value_at_with_context(lt, context.clone()) as f32,
                rotation_x_deg: tr.rotation_x.value_at_with_context(lt, context.clone()) as f32,
                rotation_y_deg: tr.rotation_y.value_at_with_context(lt, context.clone()) as f32,
            }
        })
        .collect()
}

/// The comp's Light layers, reduced to what the lighting pass needs (docs/06,
/// K-361): each light's emitting rectangle as four corners in comp pixels,
/// wound the same way for every light so the form-factor integral has a
/// consistent sign.
///
/// Returns empty — the pass's no-op — when the layer's Accepts lights switch
/// is off, when the layer is itself a light or a camera (a light does not light
/// itself, and neither draws pixels to shade), or when the comp holds no
/// lights at all. That last case is the one that matters for compatibility: a
/// project made before lighting existed has no Light layers, so this is empty
/// on every draw, so no lighting pass runs and the frame is unchanged to the
/// byte.
///
/// The nearest `MAX_LIT_LIGHTS` win when there are more, measured from the
/// layer's position — a budget rather than an error, because running out of
/// uniform slots must not make a frame fail (docs/13).
pub fn shading_lights(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
) -> Vec<lumit_gpu::fx::LightingLight> {
    use lumit_core::model::{LayerKind, LightKind};

    if !layer.switches.accepts_lights
        || matches!(
            layer.kind,
            LayerKind::Light { .. } | LayerKind::Camera { .. }
        )
    {
        return Vec::new();
    }
    let mut lights = comp.lights_at(t_comp);
    if lights.is_empty() {
        return Vec::new();
    }
    if lights.len() > lumit_gpu::fx::MAX_LIT_LIGHTS {
        let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
        let (lx, ly) = (
            layer.transform.position_x.value_at(lt),
            layer.transform.position_y.value_at(lt),
        );
        // Total order, so two runs pick the same lights: distance first, and
        // the original stacking order breaks any tie.
        let mut keyed: Vec<_> = lights
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let (dx, dy) = (l.position.0 - lx, l.position.1 - ly);
                (dx * dx + dy * dy, i)
            })
            .collect();
        keyed.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        keyed.truncate(lumit_gpu::fx::MAX_LIT_LIGHTS);
        keyed.sort_by_key(|&(_, i)| i);
        lights = keyed.iter().map(|&(_, i)| lights[i]).collect();
    }

    lights
        .iter()
        .map(|l| {
            // The same placement maths a layer gets, so a light and a layer
            // that share a transform occupy the same place in space.
            let place = lumit_gpu::place_matrix(
                (l.position.0 as f32, l.position.1 as f32),
                (0.0, 0.0),
                (100.0, 100.0),
                l.rotation_deg as f32,
                l.z as f32,
                l.rotation_x_deg as f32,
                l.rotation_y_deg as f32,
            );
            let (hw, hh) = (l.half_size.0 as f32, l.half_size.1 as f32);
            let at = |x: f32, y: f32| place_point(&place, x, y, 0.0);
            lumit_gpu::fx::LightingLight {
                corners: [at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)],
                colour: l.colour,
                falloff_px: l.falloff_px as f32,
                is_area: l.kind == LightKind::Area && hw > 0.0 && hh > 0.0,
                // A spot is aimed along its own local +z, away from the
                // camera; anything below -1 tells the kernel "not a spot".
                cone_cos: match l.kind {
                    LightKind::Spot => (l.cone_deg as f32).to_radians().cos(),
                    _ => -2.0,
                },
                axis: unit(place_dir(&place, 0.0, 0.0, 1.0)),
            }
        })
        .collect()
}

/// A point through a column-major placement matrix, done by hand so the render
/// crate does not take a maths dependency for nine multiplies. Placements are
/// affine — the camera's perspective is applied later — so there is no w to
/// divide by.
pub(crate) fn place_point(m: &[[f32; 4]; 4], x: f32, y: f32, z: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| m[0][k] * x + m[1][k] * y + m[2][k] * z + m[3][k])
}

/// A direction through the same matrix: the translation column is left out,
/// which is what makes it a direction rather than a position.
pub(crate) fn place_dir(m: &[[f32; 4]; 4], x: f32, y: f32, z: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| m[0][k] * x + m[1][k] * y + m[2][k] * z)
}

pub(crate) fn unit(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-6 || !len.is_finite() {
        return [0.0, 0.0, 1.0];
    }
    v.map(|c| c / len)
}

/// Build a comp's draw list recursively (preview side of Precomp layers).
/// Bottom-up order; matte sources come from decoded pixels (precomp mattes
/// await the GPU mask pass, mirroring export). The ordinary render entry: draws
/// at comp time `t_comp` with every effect resolved at `t_comp` too — a thin
/// wrapper over [`build_comp_draws_at`] with the sample and frame times equal
/// and no nested-frame keyer, so every Precomp realises afresh (the test
/// entry; the renderer calls [`build_comp_draws_at`] with its keyer).
pub fn build_comp_draws(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    t_comp: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
) -> Vec<CompLayerDraw> {
    build_comp_draws_at(
        doc,
        comp,
        t_comp,
        t_comp,
        pixels_by_layer,
        visited,
        None,
        false,
    )
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
///
/// `keys` names each non-collapsed Precomp's frame (K-422) so the realiser can
/// serve it from the nested-frame store; `None` leaves every nested draw
/// unnamed, which realises it every time.
///
/// `spliced` says this comp is being spliced into its parent by a collapsed
/// Precomp layer (docs/06 §1.4): its layers are not clipped to its own
/// rectangle, so a layer that covers *this* comp's frame proves nothing about
/// the parent's, and the occlusion cull (K-423) is switched off.
#[allow(clippy::too_many_arguments)]
pub fn build_comp_draws_at(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
    keys: Option<&dyn crate::cache::NestedKeyer>,
    spliced: bool,
) -> Vec<CompLayerDraw> {
    use lumit_core::model::LayerKind;
    let in_span = |l: &lumit_core::model::Layer| {
        t_comp >= l.in_point.0.to_f64() && t_comp < l.out_point.0.to_f64()
    };
    // Occlusion cull (K-423, docs/06 §1.1): the same question the decode
    // planner asks, so a layer skipped here was never decoded either.
    let occluder = (!spliced)
        .then(|| lumit_core::occlusion::occluder_index(doc, comp, t_comp))
        .flatten();

    // The caller's Arc: an expression context takes an owned handle, and this
    // shares one rather than deep-cloning the project per build (and per
    // nesting level — this function recurses through Precomps).
    let expr_doc = doc;

    // Where the Audio level driver's samples come from (K-471 §1.3). Made here,
    // from the document this walk already holds, so the preview and the export
    // — which both build their draws through this function — hand the driver
    // the same sound and reach the same number (K-031).
    let audio = crate::audio_tap::DocumentAudio::new(doc, comp);

    let pixels_for = |layer: &lumit_core::model::Layer| -> Option<LayerPixels> {
        let context = Arc::new(ExpressionContext {
            document: expr_doc.clone(),
            layer: Some(layer.id),
            comp: Some(comp.id),
            comp_time: t_comp,
            current_depth: 0,
        });

        let raw = match &layer.kind {
            // Neither kind has pixels of its own. An Adjustment layer is a
            // pass-through until its effect stack exists; a Null never draws at
            // all — it exists so other layers can be parented to it.
            LayerKind::Adjustment | LayerKind::Null => return None,
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
                let px = solid_rgba(sd.colour);
                // A flat colour is normally an 8×8 tile stretched to size —
                // but a mask gates *pixels* and a stroke paints them, so both
                // want the solid at its real size to work on (K-227).
                //
                // **And so does an effect stack.** The tile is a promise that
                // nothing between here and the compositor cares *where* a pixel
                // is, and an effect breaks that promise the moment it draws
                // rather than grades: the layer's whole stack runs at the
                // texture's raster, so on a tile it runs at 8×8 with a
                // `px_scale` of 1/240 and is stretched back up afterwards. A
                // low-frequency effect survives that as a soft smear; anything
                // with real detail does not. Particulate's default 4 px motes
                // land at 0.017 px and vanish altogether — the effect appeared
                // to do nothing at all, which is how this was found.
                //
                // The predicate is "any enabled effect", not "any effect that
                // happens to be positional", because nothing in the registry
                // declares the difference. `Roi` is about the *input*
                // neighbourhood a pixel reads, not about whether the output
                // depends on the coordinate — Gradient, Vignette, Lightning and
                // Scribble are all `Roi::Exact` and all positional — so reading
                // it here would keep the tile for exactly the effects it
                // destroys. A colour plate with nothing on it still gets the
                // tile, which is the case the optimisation was written for.
                //
                // **What this costs**, named so nobody has to measure it twice:
                // a solid carrying any effect now builds and uploads its real
                // raster every frame — about eight megabytes at 1080p — where
                // it used to build 256 bytes. That is the same order as one
                // working texture of the stack that is about to run, so it is
                // not the dominant cost of an effected solid, but it is not
                // free either. If it ever shows up in a budget, the fix is to
                // keep the small upload and stretch it on the card before the
                // stack runs, rather than to make this predicate cleverer —
                // a cleverer predicate is how the bug happened.
                let plain = layer.masks.is_empty()
                    && layer.paint.is_empty()
                    && !layer.effects.iter().any(|e| e.enabled);
                let (tw, th) = if plain { (8, 8) } else { (sd.width, sd.height) };
                (
                    px_tile(&px, tw, th),
                    tw,
                    th,
                    (sd.width as f32, sd.height as f32),
                )
            }),
            LayerKind::Text { document } => in_span(layer).then(|| {
                let fill = solid_rgba(document.fill);
                // The words at *this* layer time — a plain document hands back
                // what was typed; an expression-driven one is evaluated here,
                // which is what makes a caption able to print a live value.

                let r = lumit_text::rasterise_line(
                    &document.resolved_text(context.clone()),
                    document.size as f32,
                    [fill[0], fill[1], fill[2]],
                );
                (r.rgba, r.width, r.height, (r.width as f32, r.height as f32))
            }),
            // Vector art: rasterised at the size the frame is being drawn at,
            // into its own bounding box, which is also the layer's natural size
            // (K-237). Unlike every other kind, that size moves when the art is
            // edited.
            LayerKind::Shape { contents } => in_span(layer)
                .then(|| lumit_core::shape::contents_bounds(contents))
                .flatten()
                .map(|(x0, y0, x1, y1)| {
                    let natural_w = (x1 - x0).max(1.0);
                    let natural_h = (y1 - y0).max(1.0);
                    // The same reduced-resolution rule the other rasterised
                    // kinds follow: draw at the working scale, and let the
                    // placement matrix carry the natural size.
                    let w = natural_w.round().max(1.0) as u32;
                    let h = natural_h.round().max(1.0) as u32;
                    (
                        lumit_core::shape::rasterise_contents(contents, w, h, x0, y0, x1, y1),
                        w,
                        h,
                        (natural_w as f32, natural_h as f32),
                    )
                }),
            LayerKind::Precomp { .. } => None, // handled as Nested below
            LayerKind::Camera { .. } => None,  // shapes the view, draws nothing
            // A light is something other layers SEE (K-360). It has no picture
            // of its own; the effects that read lights take them from
            // `Composition::lights_at`, not from the draw list.
            LayerKind::Light { .. } => None,
        };
        // The layer's own clock, the same one its transform and effects are
        // read at (K-213) — a keyframed mask on a layer dragged along the
        // timeline travels with the layer.
        let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
        raw.map(|(mut rgba, w, h, natural)| {
            // Paint first, masks second: a stroke is part of the layer's
            // picture, and a mask gates the picture (K-227, docs/06 render
            // order). Painting after the mask would let a brush draw outside
            // the shape the mask cut.
            lumit_core::paint::apply_strokes(
                &mut rgba,
                w,
                h,
                f64::from(natural.0),
                f64::from(natural.1),
                &layer.paint,
            );
            lumit_core::mask::apply_masks(
                &mut rgba,
                w,
                h,
                f64::from(natural.0),
                f64::from(natural.1),
                &layer.masks,
                lt,
            );
            (rgba, w, h, natural)
        })
    };

    // The layer inputs of a stack's enabled built-in `dof` and `light_wrap`
    // effects (docs/08 §3.22, §3.28, docs/impl/layer-input.md), 1:1 and in order
    // with the stack's layer-input-consuming ops — the same
    // `enabled && Builtin && match_name` filter resolve_stack applies, and each
    // of those effects always resolves to exactly one op. Each slot carries the
    // referenced layer's SOURCE pixels (via the same
    // `pixels_for` a matte uses, so effects are not applied and a depth
    // reference can never recurse); an unset or dangling reference is None (a
    // passthrough). The depth layer does NOT need to be visible — a depth map
    // is usually hidden so it doesn't render — only in-span; the decode
    // planner (app_state::collect_comp_jobs) decodes layer-input references
    // exactly like matte sources, and export applies the same in-span-only
    // gate (K-031).
    // A reference to a PRECOMP — as a layer input (K-266) or as a track matte
    // (K-268): its picture exists only as a render, so package the nested
    // comp's draw list for realise to run recursively — the DrawSource::Nested
    // shape, on the referencing slot.
    // `visited_path` is a snapshot of the ancestor chain at this comp's
    // entry, so a matte that (transitively) contains its own comp stops at
    // the cycle instead of recursing forever; a fresh clone per reference keeps
    // the closure borrow-free. Footage INSIDE such a precomp decodes with the
    // rest of the frame: the decode planner walks matte and layer-input
    // references (plan::collect_comp_jobs) whether or not the referenced
    // layer is visible.
    let visited_path: Vec<uuid::Uuid> = visited.clone();
    let nested_comp_draw =
        |src: &lumit_core::model::Layer| -> Option<Box<crate::draw::NestedInputDraw>> {
            let lumit_core::model::LayerKind::Precomp { comp: nested_id } = &src.kind else {
                return None;
            };
            if visited_path.contains(nested_id) {
                return None;
            }
            let nested = doc.comp(*nested_id)?;
            let slt = lumit_core::time::layer_time(t_comp, src.start_offset.0);
            let frame_slt = lumit_core::time::layer_time(frame_t, src.start_offset.0);
            let mut path = visited_path.clone();
            path.push(*nested_id);
            let draws = build_comp_draws_at(
                doc,
                nested,
                slt,
                frame_slt,
                pixels_by_layer,
                &mut path,
                keys,
                false,
            );
            Some(Box::new(crate::draw::NestedInputDraw {
                width: nested.width,
                height: nested.height,
                background: [0.0, 0.0, 0.0, 0.0],
                draws,
                camera: crate::track::camera_pose(doc, nested, slt),
                key: keys.and_then(|k| k.nested_key(nested, slt)),
            }))
        };
    let nested_input_for = |src: &lumit_core::model::Layer| -> Option<DofInputDraw> {
        let nested = nested_comp_draw(src)?;
        Some(DofInputDraw {
            rgba: Vec::new(),
            tex_w: nested.width,
            tex_h: nested.height,
            fx: Default::default(),
            lut_files: Vec::new(),
            nested: Some(nested),
        })
    };

    // One referenced layer resolved into an input slot — the body
    // `dof_inputs_for` and `mattes_for` share (docs/impl/layer-input.md §2): the
    // span gate, the K-266 nested-precomp render, and the K-142
    // masks-and-effects folding, so a matte and a background plate can never
    // disagree about what "a layer rendered alone" means.
    let layer_slot = |e: &lumit_core::model::EffectInstance, param: &str| -> Option<DofInputDraw> {
        let id = e.layer_ref(param)?;
        let src = comp.layers.iter().find(|l| l.id == id)?;
        if !in_span(src) {
            return None;
        }
        // A Precomp reference renders its comp (K-266) — "a white circle
        // in a precomp" is the natural way to author a flare source, and
        // a depth pass authored as a comp is the same shape.
        if let Some(nested) = nested_input_for(src) {
            return Some(nested);
        }
        let mode = e.layer_source(param);
        // Layer source (K-142). None samples the layer's raw pixels —
        // clear its masks so `pixels_for` skips them; Masks and Effects
        // and masks keep them.
        let (rgba, tex_w, tex_h, natural) = if mode.applies_masks() {
            pixels_for(src)?
        } else {
            let mut bare = src.clone();
            bare.masks.clear();
            pixels_for(&bare)?
        };
        // Effects and masks (K-142): resolve the referenced layer's own
        // stack at its layer time so render_dof_inputs runs it on the
        // texture before resampling. Uses that layer's decode scale (its
        // px@comp radii stay honest), the same resolve export uses
        // (K-031). Empty otherwise.
        let (fx, lut_files) = if mode.folds_effects() && src.switches.fx {
            let slt = lumit_core::time::layer_time(t_comp, src.start_offset.0);
            let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
            let scale = tex_w as f32 / natural.0.max(1.0);
            let markers = lumit_core::fx::MarkerContext::for_layer(comp, src);
            // The referenced layer's own effects, so its own expressions
            // resolve about it rather than about the layer that pointed
            // at it.
            let context = Arc::new(ExpressionContext {
                document: expr_doc.clone(),
                comp: Some(comp.id),
                layer: Some(src.id),
                comp_time: t_comp,
                current_depth: 0,
            });
            // The referenced layer's own driver graph too (K-471): a wire
            // substitutes where a keyframe would have been read, so it belongs
            // to whichever stack is being resolved.
            let drivers =
                lumit_core::fx::resolve_drivers(&src.graph, slt, context.clone(), Some(&audio));
            (
                lumit_core::fx::resolve_stack_temporal_named(
                    &src.effects,
                    &drivers,
                    slt,
                    slt,
                    comp_diag * scale,
                    scale,
                    &markers,
                    context,
                )
                .1,
                lut_files(&src.effects, slt),
            )
        } else {
            Default::default()
        };
        Some(DofInputDraw {
            rgba,
            tex_w,
            tex_h,
            fx,
            lut_files,
            nested: None,
        })
    };

    // **The auxiliary-layer inputs** (K-123, docs/impl/layer-input.md §2): one
    // slot per enabled built-in whose declaration names a Layer row that is not
    // its matte — Light wrap's Background, Texturize's Texture, Fast motion
    // blur's Motion vectors, Set matte's source — AND that resolves to an op at
    // all, exactly the pair of conditions `mattes_for` below applies, so this
    // list stays 1:1 with the ops `run_ops` walks.
    //
    // The parameter comes from the schema's own
    // [`EffectSchema::layer_input`] rather than a table of match names here
    // (K-429): a table is a second rule, and a second rule is a thing to forget
    // when an effect gains a layer row.
    let dof_inputs_for =
        |owner: uuid::Uuid, effects: &[lumit_core::model::EffectInstance]| -> Vec<LayerInputDraw> {
            use lumit_core::model::EffectNamespace;
            effects
                .iter()
                .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
                .filter_map(|e| {
                    let def = lumit_core::fx::BUILTIN_DEFS.get(&e.effect.match_name)?;
                    let param = def.schema().layer_input()?;
                    def.is_image_op().then_some((e, param))
                })
                .map(|(e, param)| {
                    // "This layer" (K-288): a reference to the layer the effect
                    // is ON is not a second render — it is the effect's own
                    // input, which `run_ops` already holds.
                    if e.layer_ref(param) == Some(owner) {
                        return LayerInputDraw::ThisLayer;
                    }
                    layer_slot(e, param).map_or(LayerInputDraw::Absent, LayerInputDraw::Layer)
                })
                .collect()
        };

    // **The Matte inputs — the one carriage** (K-395, docs/08 §2.6). One slot
    // per enabled built-in whose declaration names a matte parameter AND
    // resolves to an op at all — the two conditions `resolve_stack` itself
    // applies, so this list stays 1:1 with the ops `run_ops` walks. An
    // orchestration-only effect (Posterize time, Accumulation motion blur) has a
    // Matte row like everything else but no op to hang it on, and is skipped on
    // both sides.
    //
    // The parameter comes from the schema's own role rather than a table here,
    // which is what folds Depth of field's `depth` and the Lens flare's `matte`
    // into this list instead of the two private lists they used to have. They
    // are not special cases: they are effects whose matte means something deeper
    // than strength, and the only thing that differs is which parameter holds
    // the reference.
    // `graph` carries the layer's driver wiring: an in-graph **SourceMatte**
    // edge (K-471 §1.4) feeds an effect the layer's OWN masked source alpha at
    // that point in the chain, and overrides the Matte parameter while it
    // exists. That is precisely what K-288's "this layer" already means to the
    // render — the effect's own input rather than a second pass over another
    // layer — so the wire lowers to the answer this builder has always had, and
    // nothing downstream learns a new shape.
    let mattes_for = |owner: uuid::Uuid,
                      effects: &[lumit_core::model::EffectInstance],
                      graph: &lumit_core::graph::LayerGraph|
     -> Vec<LayerInputDraw> {
        use lumit_core::model::EffectNamespace;
        effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|e| {
                let def = lumit_core::fx::BUILTIN_DEFS.get(&e.effect.match_name)?;
                let param = def.schema().matte.param()?;
                def.is_image_op().then_some((e, param))
            })
            .map(|(e, param)| {
                // The wire wins over the parameter while it is there.
                if graph.source_matte(e.id) {
                    return LayerInputDraw::ThisLayer;
                }
                // A row the panel does not show, or shows greyed, is a row
                // nobody meant: the Lens flare's Matte rows only exist while
                // its Source type is Matte, and rendering the layer one names
                // in Manual mode would cost a whole pass per frame for a
                // picture the kernel ignores. One rule, every effect — the
                // flare's own named gate, generalised.
                if !lumit_core::fx::param_visible(e, param)
                    || !lumit_core::fx::param_enabled(e, param)
                {
                    return LayerInputDraw::Absent;
                }
                // "This layer" (K-288): a matte pointed at the layer the
                // effect is on is the effect's own input, not a re-render —
                // on an adjustment layer, the composite below.
                if e.layer_ref(param) == Some(owner) {
                    return LayerInputDraw::ThisLayer;
                }
                layer_slot(e, param).map_or(LayerInputDraw::Absent, LayerInputDraw::Layer)
            })
            .collect()
    };

    // **The mask paths — the geometry carriage** (K-408, docs/08 §1.2). One
    // polyline per enabled built-in whose declaration names a MaskPath row AND
    // resolves to an op at all — the same two conditions `mattes_for` applies,
    // so this list stays 1:1 with the ops `run_ops` walks, with its own counter
    // there because the predicate is a different one.
    //
    // The masks are the *layer's own*: a mask belongs to the layer it is drawn
    // on, and an effect walking another layer's shape is a question nobody has
    // asked. That is why nothing is rendered here — unlike a matte, this input
    // is not a picture, and the whole point of the seam is that a coverage
    // buffer cannot say which way is *along* a curve.
    let mask_paths_for = |layer: &lumit_core::model::Layer,
                          slt: f64|
     -> Vec<lumit_core::mask::MaskPolyline> {
        use lumit_core::model::EffectNamespace;
        layer
            .effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|e| {
                let def = lumit_core::fx::BUILTIN_DEFS.get(&e.effect.match_name)?;
                let (param, self_default) = def.schema().mask_path()?;
                def.is_image_op().then_some((e, param, self_default))
            })
            .map(|(e, param, self_default)| {
                // A row the panel does not show, or shows greyed, is a row
                // nobody meant — the same rule the matte carriage applies.
                if !lumit_core::fx::param_visible(e, param)
                    || !lumit_core::fx::param_enabled(e, param)
                {
                    return lumit_core::mask::MaskPolyline::default();
                }
                lumit_core::mask::mask_path_at(&layer.masks, e.mask_ref(param), self_default, slt)
            })
            .collect()
    };

    // **The birth schedules — the timing carriage** (points-stream.md §3.3,
    // K-474). One per enabled built-in that declares a Points output AND
    // resolves to an op at all — the same two conditions `mask_paths_for`
    // applies, so this list stays 1:1 with the ops `run_ops` walks, with its
    // own counter there because the predicate is a different one.
    //
    // Scanned here for the reason the polylines are flattened here: it is not
    // pixels, and it is not a number in the bag. The scan walks ONE SCALAR per
    // frame from the layer's in point to this one — a minute of comp at 60 fps
    // is 3 600 additions — and what it records is only the window a particle
    // could still be alive from.
    //
    // The rate is read off the stored track with the frame's own expression
    // context. A *driven* Emit rate reads its wire at this frame like any other
    // parameter; the wire's value at earlier frames is not re-walked, because
    // the driver graph is resolved once per frame and re-resolving it per
    // scanned frame would make one picture cost a thousand driver walks. The
    // birth schedule of a driven rate therefore follows the track it was
    // authored on — named here, and PS4's business when a wire can reach it.
    let points_schedules_for = |layer: &lumit_core::model::Layer,
                                slt: f64,
                                frame_slt: f64|
     -> Vec<lumit_core::fx::points::PointsSchedule> {
        use lumit_core::model::EffectNamespace;
        let dt = 1.0 / comp.frame_rate.fps().max(1.0);
        layer
            .effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|e| {
                let def = lumit_core::fx::BUILTIN_DEFS.get(&e.effect.match_name)?;
                let wants =
                    lumit_core::fx::points::wants_schedule(def.signature()) && def.is_image_op();
                wants.then_some((e, def))
            })
            .map(|(e, _def)| {
                // A pinned effect (K-132) is evaluated at the true playhead, so
                // its schedule is scanned there too: the picture and the
                // particles it draws must be of one moment.
                let t = if e.sample_temporally { slt } else { frame_slt };
                let upto = (t / dt).floor() as i64;
                let context = Arc::new(ExpressionContext {
                    document: expr_doc.clone(),
                    comp: Some(comp.id),
                    layer: Some(layer.id),
                    comp_time: t_comp,
                    current_depth: 0,
                });
                let rate_at = |lt: f64| -> f64 {
                    e.float_at_with_context("emit_rate", lt, context.clone())
                        .unwrap_or(150.0)
                };
                // How far back a particle born then could still be alive now:
                // Life plus its jitter is the ceiling exactly, and a frame more
                // because the frame the window opens on is one whose births are
                // partly inside it.
                let at = |id: &str, fallback: f64| -> f64 {
                    e.float_at_with_context(id, t, context.clone())
                        .unwrap_or(fallback)
                };
                let jitter = (at("life_jitter", 30.0) / 100.0).clamp(0.0, 1.0);
                let longest = at("life", 2.0).max(0.0) * (1.0 + jitter);
                let window = ((longest / dt).ceil() as i64).saturating_add(1);
                let mut schedule =
                    lumit_core::fx::points::Schedule::scan(dt, upto, window, &rate_at);
                schedule.trim_to_newest(lumit_gpu::fx::MAX_CANDIDATES);
                lumit_core::fx::points::PointsSchedule { schedule, t }
            })
            .collect()
    };

    // Solo / isolate (K-105): while any layer is soloed, only soloed layers
    // render — computed once for the whole comp.
    let any_solo = lumit_core::model::any_picture_solo(comp);
    let mut draws: Vec<CompLayerDraw> = Vec::new();
    for (idx, layer) in comp.layers.iter().enumerate().rev() {
        let context = Arc::new(ExpressionContext {
            document: expr_doc.clone(),
            comp: Some(comp.id),
            layer: Some(layer.id),
            comp_time: t_comp,
            current_depth: 0,
        });

        // An Audio layer draws nothing at all (K-435) — no source, no solid, no
        // effects on an empty canvas. The mixer has already taken what it needs.
        if layer.audio_only {
            continue;
        }
        if !layer.switches.visible || !in_span(layer) || (any_solo && !layer.switches.solo) {
            continue;
        }
        // Under a full-frame opaque layer: never seen, so never built.
        if occluder.is_some_and(|o| idx > o) {
            continue;
        }
        let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
        // The true frame time in this layer's own time base, for effects a
        // held/sub-frame re-render must not re-sample (docs/impl/
        // temporal-rerender.md §5). Equal to `lt` on an ordinary render.
        let frame_lt = lumit_core::time::layer_time(frame_t, layer.start_offset.0);
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
            layer.start_offset.0,
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
                    let mut inner = build_comp_draws_at(
                        doc,
                        nested,
                        lt,
                        frame_lt,
                        pixels_by_layer,
                        visited,
                        keys,
                        true,
                    );
                    visited.pop();

                    let own = lumit_gpu::place_matrix(
                        (
                            tr.position_x.value_at_with_context(lt, context.clone()) as f32,
                            tr.position_y.value_at_with_context(lt, context.clone()) as f32,
                        ),
                        (
                            tr.anchor_x.value_at_with_context(lt, context.clone()) as f32,
                            tr.anchor_y.value_at_with_context(lt, context.clone()) as f32,
                        ),
                        (
                            tr.scale_x.value_at_with_context(lt, context.clone()) as f32,
                            tr.scale_y.value_at_with_context(lt, context.clone()) as f32,
                        ),
                        tr.rotation.value_at_with_context(lt, context.clone()) as f32,
                        tr.position_z.value_at_with_context(lt, context.clone()) as f32,
                        tr.rotation_x.value_at_with_context(lt, context.clone()) as f32,
                        tr.rotation_y.value_at_with_context(lt, context.clone()) as f32,
                    );
                    // If the collapsed precomp is itself parented, its parent's
                    // world placement wraps its own before it wraps the inner
                    // draws (K-103).

                    let parent = match parent_world_placement(comp, layer, t_comp, context.clone())
                    {
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
                let nested_draws = build_comp_draws_at(
                    doc,
                    nested,
                    lt,
                    frame_lt,
                    pixels_by_layer,
                    visited,
                    keys,
                    false,
                );
                visited.pop();
                (
                    DrawSource::Nested {
                        width: nested.width,
                        height: nested.height,
                        // A nested comp's intermediate is transparent where
                        // nothing covers it (K-241). A comp's background colour
                        // is a viewing backdrop for the comp being looked at,
                        // not a layer of its own, so filling the intermediate
                        // with it would turn every gap in a Precomp into opaque
                        // black and hide the parent's stack behind it.
                        background: [0.0, 0.0, 0.0, 0.0],
                        draws: nested_draws,
                        camera: crate::track::camera_pose(doc, nested, lt),
                        // The nested frame's own name (K-422). `lt` is on the
                        // flick grid already (`layer_time`), so a Precomp
                        // layer moved by whole frames keeps its names.
                        key: keys.and_then(|k| k.nested_key(nested, lt)),
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
                let (fx_ids, fx) = if layer.switches.fx {
                    // The §1.4 marker context, built by the same shared
                    // constructor export uses (K-031). Effects flagged
                    // sample_temporally == false resolve at the frame time in a
                    // held re-render (§5); equal to `lt` on an ordinary render.
                    let markers = lumit_core::fx::MarkerContext::for_layer(comp, layer);
                    lumit_core::fx::resolve_stack_temporal_named(
                        &layer.effects,
                        &lumit_core::fx::resolve_drivers(
                            &layer.graph,
                            effect_lt,
                            context.clone(),
                            Some(&audio),
                        ),
                        effect_lt,
                        frame_lt,
                        comp_diag,
                        1.0,
                        &markers,
                        context.clone(),
                    )
                } else {
                    Default::default()
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
                )
                .map(|mut ab| {
                    // Its Matte (K-429), rendered by the same helper every
                    // other matte and layer input goes through. It is filled in
                    // here rather than inside `accumulation_mb_below` because
                    // that is a free function and this is where `layer_slot`
                    // lives. Pointed at the adjustment itself (K-288), the
                    // matte is the composite below — which is what an
                    // adjustment layer's own input is.
                    if let Some(e) = layer.effects.iter().find(|e| {
                        e.enabled
                            && e.effect.namespace == lumit_core::model::EffectNamespace::Builtin
                            && e.effect.match_name == "accumulation_mb"
                    }) {
                        ab.matte = if e.layer_ref(lumit_core::fx::MATTE_PARAM) == Some(layer.id) {
                            LayerInputDraw::ThisLayer
                        } else {
                            layer_slot(e, lumit_core::fx::MATTE_PARAM)
                                .map_or(LayerInputDraw::Absent, LayerInputDraw::Layer)
                        };
                    }
                    ab
                });
                if fx.is_empty() && temporal_below.is_none() && accumulation_below.is_none() {
                    continue;
                }

                draws.push(CompLayerDraw {
                    layer: layer.id,
                    source: DrawSource::Adjust,
                    natural_size: (comp.width as f32, comp.height as f32),
                    position: (
                        tr.position_x.value_at_with_context(lt, context.clone()) as f32,
                        tr.position_y.value_at_with_context(lt, context.clone()) as f32,
                    ),
                    anchor: (
                        tr.anchor_x.value_at_with_context(lt, context.clone()) as f32,
                        tr.anchor_y.value_at_with_context(lt, context.clone()) as f32,
                    ),
                    scale: (
                        tr.scale_x.value_at_with_context(lt, context.clone()) as f32,
                        tr.scale_y.value_at_with_context(lt, context.clone()) as f32,
                    ),
                    rotation_deg: tr.rotation.value_at_with_context(lt, context.clone()) as f32,
                    opacity: tr.opacity.value_at_with_context(lt, context.clone()) as f32,
                    z: tr.position_z.value_at_with_context(lt, context.clone()) as f32,
                    rotation_x_deg: tr.rotation_x.value_at_with_context(lt, context.clone()) as f32,
                    rotation_y_deg: tr.rotation_y.value_at_with_context(lt, context.clone()) as f32,
                    three_d: layer.switches.three_d,
                    matte: None,
                    blend: lumit_gpu::Blend::Normal,
                    mask_cov: (!layer.masks.is_empty()).then(|| {
                        // Adjustment masks live in comp space (comp-sized
                        // natural), same as the property panel treats them.
                        (
                            mask_rgba(&lumit_core::mask::combined_coverage(
                                &layer.masks,
                                comp.width,
                                comp.height,
                                f64::from(comp.width),
                                f64::from(comp.height),
                                lt,
                            )),
                            comp.width,
                            comp.height,
                        )
                    }),
                    pre: parent_world_placement(comp, layer, t_comp, context.clone()),
                    fx,
                    fx_ids,
                    // Adjustment layers process the composite below, not
                    // footage frames — no neighbours or flow field here.
                    neighbours: Vec::new(),
                    flow_field: None,
                    // Ordered file paths of the enabled built-in `lut` effects,
                    // 1:1 with the stack's `lut` ops (docs/08 §3.11);
                    // the same `lt` resolve_stack used above.
                    lut_files: lut_files(&layer.effects, lt),
                    // Depth inputs of the enabled built-in `dof` and
                    // `light_wrap` effects, 1:1 with the stack's
                    // layer-input-consuming ops (docs/08 §3.22, §3.28).
                    dof_inputs: dof_inputs_for(layer.id, &layer.effects),
                    mattes: mattes_for(layer.id, &layer.effects, &layer.graph),
                    mask_paths: mask_paths_for(layer, lt),
                    points_schedules: points_schedules_for(layer, lt, frame_lt),
                    flare_lens_files: flare_lens_files(&layer.effects, lt),
                    // The adjust stack resolves at comp scale but runs on
                    // the render target (K-266) — realise rescales.
                    fx_ref_width: Some(comp.width as f32),
                    // The composite below has no name of its own in v1
                    // (K-421): an adjustment's stack runs uncached.
                    fx_input_key: None,
                    // An adjustment layer is a staging point, not a picture —
                    // motion blur has no image of its own to smear (docs/06 §4).
                    mb: Vec::new(),
                    // An adjustment layer has no surface of its own — it is
                    // the composite of everything below, whose layers were
                    // each already lit. Shading it would light them twice.
                    lights: Vec::new(),
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
                        colour_space: crate::colour::footage_colour_space(doc, &layer.kind),
                    },
                    natural,
                )
            }
        };

        let matte = layer.matte.as_ref().and_then(|mr| {
            let src = comp.layers.iter().find(|l| l.id == mr.layer)?;
            // A Precomp matte renders its comp (K-268): a comp has no pixels
            // until it is rendered, so `pixels_for` gives up on one and the
            // matte silently gated nothing — a layer set to a precomp matte
            // simply vanished. The nested render stands in for the source
            // texture; the source-mode toggles below do not apply to it,
            // because a comp already carries its layers' own masks and
            // effects (the K-266 layer-input boundary, unchanged).
            let nested = in_span(src).then(|| nested_comp_draw(src)).flatten();
            // Matte source mode (K-142). None reads the source's raw pixels —
            // clear its masks so `pixels_for` skips them; Masks and Effects and
            // masks keep them.
            let (m_rgba, m_w, m_h, m_nat) = if let Some(n) = &nested {
                (
                    Vec::new(),
                    n.width,
                    n.height,
                    (n.width as f32, n.height as f32),
                )
            } else if mr.source.applies_masks() {
                pixels_for(src)?
            } else {
                let mut bare = src.clone();
                bare.masks.clear();
                pixels_for(&bare)?
            };
            let mlt = lumit_core::time::layer_time(t_comp, src.start_offset.0);
            let mtr = &src.transform;
            // Effects and masks matte (K-142): resolve the matte source's own
            // stack at its layer time so gpu.rs runs it on the matte texture
            // before the matte gates the consumer. Uses the source's decode scale
            // (its px@comp radii stay honest under reduced-res preview), the same
            // §1.4 markers and the same resolve export uses (K-031). Empty for
            // None / Masks or when the source's fx switch is off.
            let (fx, lut_files) = if nested.is_some() {
                // The nested render already ran every layer's own stack.
                Default::default()
            } else if mr.source.folds_effects() && src.switches.fx {
                let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let scale = m_w as f32 / m_nat.0.max(1.0);

                let markers = lumit_core::fx::MarkerContext::for_layer(comp, src);
                let drivers =
                    lumit_core::fx::resolve_drivers(&src.graph, mlt, context.clone(), Some(&audio));
                (
                    lumit_core::fx::resolve_stack_temporal_named(
                        &src.effects,
                        &drivers,
                        mlt,
                        mlt,
                        comp_diag * scale,
                        scale,
                        &markers,
                        context.clone(),
                    )
                    .1,
                    lut_files(&src.effects, mlt),
                )
            } else {
                Default::default()
            };
            Some(MatteDraw {
                rgba: m_rgba,
                tex_w: m_w,
                tex_h: m_h,
                natural_size: m_nat,
                position: (
                    mtr.position_x.value_at_with_context(mlt, context.clone()) as f32,
                    mtr.position_y.value_at_with_context(mlt, context.clone()) as f32,
                ),
                anchor: (
                    mtr.anchor_x.value_at_with_context(mlt, context.clone()) as f32,
                    mtr.anchor_y.value_at_with_context(mlt, context.clone()) as f32,
                ),
                scale: (
                    mtr.scale_x.value_at_with_context(mlt, context.clone()) as f32,
                    mtr.scale_y.value_at_with_context(mlt, context.clone()) as f32,
                ),
                rotation_deg: mtr.rotation.value_at_with_context(mlt, context.clone()) as f32,
                opacity: mtr.opacity.value_at_with_context(mlt, context.clone()) as f32,
                z: mtr.position_z.value_at_with_context(mlt, context.clone()) as f32,
                rotation_x_deg: mtr.rotation_x.value_at_with_context(mlt, context.clone()) as f32,
                rotation_y_deg: mtr.rotation_y.value_at_with_context(mlt, context.clone()) as f32,
                three_d: src.switches.three_d,
                luma: matches!(mr.channel, lumit_core::model::MatteChannel::Luma),
                inverted: mr.inverted,
                fx,
                lut_files,
                nested,
            })
        });

        // Spatial units are px@comp (docs/08 §2.3); the effect runs on the
        // layer's decoded texture, so the preview factor is decode/natural, to
        // stay honest under reduced-resolution preview.
        let (fx_ids, fx) = {
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
                lumit_core::fx::resolve_stack_temporal_named(
                    &layer.effects,
                    &lumit_core::fx::resolve_drivers(
                        &layer.graph,
                        effect_lt,
                        context.clone(),
                        Some(&audio),
                    ),
                    effect_lt,
                    frame_lt,
                    comp_diag * scale,
                    scale,
                    &markers,
                    context.clone(),
                )
            } else {
                Default::default()
            }
        };
        // Effects ON a Precomp layer run on the nested comp's raster, and that
        // raster shrinks with the preview scale while the stack above resolved
        // px@comp parameters at factor 1 — the K-266 disease on the nested arm
        // (K-268). Hand realise the width the stack was resolved against (the
        // nested comp's own width) and it rescales to whatever it renders at,
        // exactly as it does for an adjustment layer. `None` for every other
        // kind: a Pixels stack already resolved at its decode scale above.
        let fx_ref_width = match &source {
            DrawSource::Nested { width, .. } => Some(*width as f32),
            DrawSource::Pixels { .. } | DrawSource::Adjust => None,
        };
        // The name the per-effect cache files this stack's outputs under
        // (K-421): a picture made from bytes, or — since K-422 named it — a
        // nested comp's frame, whose texture is exactly what the stack runs on
        // (`op_keys` folds in the raster size, so the render scale is covered).
        let fx_input_key = match &source {
            DrawSource::Pixels { tex_w, tex_h, .. } => {
                fx_input_key(doc, layer, pixels_by_layer, lt, *tex_w, *tex_h)
            }
            DrawSource::Nested { key, .. } => *key,
            DrawSource::Adjust => None,
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
            layer: layer.id,
            source,
            natural_size: natural,
            position: (
                tr.position_x.value_at_with_context(lt, context.clone()) as f32,
                tr.position_y.value_at_with_context(lt, context.clone()) as f32,
            ),
            anchor: (
                tr.anchor_x.value_at_with_context(lt, context.clone()) as f32,
                tr.anchor_y.value_at_with_context(lt, context.clone()) as f32,
            ),
            scale: (
                tr.scale_x.value_at_with_context(lt, context.clone()) as f32,
                tr.scale_y.value_at_with_context(lt, context.clone()) as f32,
            ),
            rotation_deg: tr.rotation.value_at_with_context(lt, context.clone()) as f32,
            opacity: tr.opacity.value_at_with_context(lt, context.clone()) as f32,
            z: tr.position_z.value_at_with_context(lt, context.clone()) as f32,
            rotation_x_deg: tr.rotation_x.value_at_with_context(lt, context.clone()) as f32,
            rotation_y_deg: tr.rotation_y.value_at_with_context(lt, context.clone()) as f32,
            three_d: layer.switches.three_d,
            matte,
            blend: blend_of(layer.blend),
            mask_cov: match &layer.kind {
                LayerKind::Precomp { .. } if !layer.masks.is_empty() => {
                    let (w, h) = (natural.0 as u32, natural.1 as u32);
                    Some((
                        mask_rgba(&lumit_core::mask::combined_coverage(
                            &layer.masks,
                            w,
                            h,
                            f64::from(w),
                            f64::from(h),
                            lt,
                        )),
                        w,
                        h,
                    ))
                }
                _ => None,
            },
            pre: parent_world_placement(comp, layer, t_comp, context.clone()),
            fx,
            fx_ids,
            neighbours,
            flow_field,
            // Ordered file paths of the enabled built-in `lut` effects, 1:1
            // with the stack's `lut` ops (docs/08 §3.11); the same `lt`
            // resolve_stack used for `fx`.
            lut_files: lut_files(&layer.effects, lt),
            // Depth inputs of the enabled built-in `dof` and `light_wrap`
            // effects, 1:1 with the stack's layer-input-consuming ops (docs/08
            // §3.22, §3.28); built the same way export does, so the two blur
            // identically (K-031).
            dof_inputs: dof_inputs_for(layer.id, &layer.effects),
            mattes: mattes_for(layer.id, &layer.effects, &layer.graph),
            mask_paths: mask_paths_for(layer, lt),
            points_schedules: points_schedules_for(layer, lt, frame_lt),
            flare_lens_files: flare_lens_files(&layer.effects, lt),
            fx_ref_width,
            fx_input_key,
            // Per-layer motion blur (docs/06 §4, K-120): the layer's own
            // transform sampled across the open shutter, empty unless it blurs.
            // Built the same way export does, so the two smear identically.
            mb: motion_blur_samples(comp, layer, t_comp, context.clone()),
            // The comp's lights, if this layer takes them (docs/06, K-361).
            lights: shading_lights(comp, layer, t_comp),
            // Ordinary layers never carry a temporal re-render — that is an
            // adjustment-only capability in v1 (docs/08 §3.25, §3.26).
            temporal_below: None,
            accumulation_below: None,
        });
    }
    draws
}

/// The content name of the picture a layer's effect stack runs on (K-421,
/// [`CompLayerDraw::fx_input_key`]): what the source is, then everything
/// `pixels_for` bakes into it before the stack sees it — the paint strokes,
/// the masks at this layer time — and the raster it was made at. Footage and
/// Sequence layers name their source by the decode job's identity
/// ([`CompLayerPixels::source_key`]); a solid by its colour and size. `None`
/// for the kinds nothing names yet (text, shapes), which run uncached.
///
/// The masks and paint go in serialised, as the file format writes them: a
/// keyframed mask is named by its whole animation plus the time it is read
/// at, which is coarser than the evaluated shape but never wrong.
fn fx_input_key(
    doc: &lumit_core::model::Document,
    layer: &lumit_core::model::Layer,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    lt: f64,
    tex_w: u32,
    tex_h: u32,
) -> Option<u128> {
    use lumit_core::model::LayerKind;
    let mut h = blake3::Hasher::new();
    h.update(b"fxinput/1/");
    match &layer.kind {
        LayerKind::Footage { .. } | LayerKind::Sequence { .. } => {
            h.update(b"footage/");
            h.update(&pixels_by_layer.get(&layer.id)?.source_key.to_le_bytes());
        }
        LayerKind::Solid { def } => {
            let sd = doc.solid(*def)?;
            h.update(b"solid/");
            for ch in sd.colour.0 {
                h.update(&ch.to_le_bytes());
            }
            h.update(&sd.width.to_le_bytes());
            h.update(&sd.height.to_le_bytes());
        }
        _ => return None,
    }
    h.update(&bincode::serialize(&layer.masks).ok()?);
    h.update(&bincode::serialize(&layer.paint).ok()?);
    h.update(&lt.to_le_bytes());
    h.update(&tex_w.to_le_bytes());
    h.update(&tex_h.to_le_bytes());
    let mut k = [0u8; 16];
    k.copy_from_slice(&h.finalize().as_bytes()[..16]);
    Some(u128::from_le_bytes(k))
}

/// The ordered file paths of a layer's enabled built-in `lut` effects
/// (docs/08 §3.11, K-114), each resolved at layer time `lt` (None = unset).
/// `resolve_stack` filters on the identical `e.enabled && namespace == Builtin`
/// predicate and preserves order, and a `lut` effect always resolves to exactly
/// one op, so this list is 1:1 and in the same order as the stack's
/// `lut` ops — the alignment `run_ops` relies on to bind LUT k to op
/// k. Preview (here) and export build it the same way, so the two match (K-031).
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

/// The `lens_file` paths of the enabled built-in `lens_flare` effects, 1:1
/// and in order with the stack's `lens_flare` ops (K-264) — the LUT-files
/// pattern for the flare's custom prescription.
fn flare_lens_files(effects: &[lumit_core::model::EffectInstance], lt: f64) -> Vec<Option<String>> {
    use lumit_core::model::EffectNamespace;
    effects
        .iter()
        .filter(|e| {
            e.enabled
                && e.effect.namespace == EffectNamespace::Builtin
                && e.effect.match_name == "lens_flare"
        })
        .map(|e| e.path_at("lens_file", lt).map(str::to_owned))
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
#[allow(clippy::too_many_arguments)]
pub fn render_below_at(
    realiser: &Realiser,
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    below: &[lumit_core::model::Layer],
    tau: f64,
    frame_t: f64,
    force_mb: Option<lumit_core::model::MotionBlur>,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
) -> wgpu::Texture {
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
#[allow(clippy::too_many_arguments)]
pub fn below_draws_at(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    below: &[lumit_core::model::Layer],
    tau: f64,
    frame_t: f64,
    force_mb: Option<lumit_core::model::MotionBlur>,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
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
    // No nested-frame keyer (K-422): a held re-render strips the temporal
    // inputs below, so a Precomp in it is not the picture its name would claim.
    let mut draws = build_comp_draws_at(
        doc,
        &below_comp,
        tau,
        frame_t,
        pixels_by_layer,
        visited,
        None,
        false,
    );
    strip_temporal_inputs(&mut draws);
    (draws, crate::track::camera_pose(doc, comp, tau))
}

/// The held below-stack for a temporal adjustment (Posterize Time everything-
/// below, docs/08 §3.25), or None when `layer` carries no such effect. `idx` is
/// the layer's document index, so the below-set is `comp.layers[idx + 1..]`
/// (everything lower in the stack). The *this layer's effects* scope is not an
/// adjustment re-render (it substitutes time into the layer's own stack, a
/// later step), so it returns None here.
#[allow(clippy::too_many_arguments)]
pub fn posterize_below(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    idx: usize,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
) -> Option<TemporalBelow> {
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let p = lumit_core::fx::stack_posterize(&layer.effects, layer.switches.fx, lt)?;
    // The below-render reach is implied by the carrier (K-166): only an
    // adjustment layer's Posterize holds the composite beneath it.
    if !matches!(layer.kind, lumit_core::model::LayerKind::Adjustment) {
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
#[allow(clippy::too_many_arguments)]
pub fn accumulation_mb_below(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    idx: usize,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
) -> Option<AccumulationBelow> {
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
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
        // Filled in by the caller, which has the layer-rendering helper this
        // free function does not (K-429).
        matte: LayerInputDraw::Absent,
        matte_channel: p.matte_channel,
        matte_invert: p.matte_invert,
        anchor: p.shutter_anchor() as f32,
    })
}

/// Drop the neighbour frames and flow field a temporal effect reads, recursing
/// into nested-comp draws — so a held/sub-frame re-render treats echo, flow
/// motion blur and datamosh as stills and the preview matches export, which
/// carries no such decode for the re-render (docs/impl/temporal-rerender.md
/// Traps). Spatial effects (blur, glow, colour, transform) are untouched, so a
/// posterised or motion-blurred scene still holds its full spatial animation.
fn strip_temporal_inputs(draws: &mut [CompLayerDraw]) {
    for d in draws.iter_mut() {
        d.neighbours = Vec::new();
        d.flow_field = None;
        if let DrawSource::Nested { draws: inner, .. } = &mut d.source {
            strip_temporal_inputs(inner);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod build_tests;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod parent_placement_tests {
    use super::*;
    use lumit_core::anim::Property;
    use lumit_core::model::*;
    use lumit_core::{CompTime, Duration, FrameRate, Rational};

    fn layer(px: f64, py: f64, parent: Option<uuid::Uuid>) -> Layer {
        Layer {
            graph: Default::default(),
            markers: Vec::new(),
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
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
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
        assert!(
            parent_world_placement(&c, &gp, 0.0, Arc::new(ExpressionContext::detached())).is_none()
        );
        // The child's world placement is grandparent × parent (top outermost),
        // exactly the manual concat — proving the walk and fold order.
        let world =
            parent_world_placement(&c, &child, 0.0, Arc::new(ExpressionContext::detached()))
                .unwrap();
        let expected = lumit_gpu::concat_place(place_of(&gp), place_of(&parent));
        assert_eq!(world, expected);
    }

    /// Backlog 7.53: a keyframed parent slid three frames along the timeline
    /// places its child at frame f+3 exactly as it did at frame f before the
    /// move — bit for bit — because layer time is taken on the flick grid
    /// rather than as an f64 subtraction.
    #[test]
    fn a_moved_keyframed_parent_places_its_child_bit_identically() {
        use lumit_core::anim::{Animation, Keyframe, SideInterp};
        let kf = |t: i64, v: f64| Keyframe {
            time: Rational::new(t, 1).unwrap(),
            value: v,
            interp_in: SideInterp::Linear,
            interp_out: SideInterp::Linear,
        };
        let mut parent = layer(0.0, 0.0, None);
        parent.transform.position_x = Property {
            extra: serde_json::Map::new(),
            animation: Animation::Keyframed(vec![kf(0, 0.0), kf(1, 37.0), kf(4, 100.0)]),
        };
        let child = layer(5.0, 5.0, Some(parent.id));
        let before = comp(vec![parent.clone(), child.clone()]);
        let fr = before.frame_rate;
        let three = fr.time_of_frame(3).unwrap();
        let mut moved = parent.clone();
        moved.start_offset = three;
        moved.in_point = three;
        moved.out_point = CompTime(moved.out_point.0.checked_add(three.0).unwrap());
        let after = comp(vec![moved, child.clone()]);
        for f in 0..120 {
            let t0 = fr.time_of_frame(f).unwrap().0.to_f64();
            let t3 = fr.time_of_frame(f + 3).unwrap().0.to_f64();
            let ctx = || Arc::new(ExpressionContext::detached());
            let a = parent_world_placement(&before, &child, t0, ctx()).unwrap();
            let b = parent_world_placement(&after, &child, t3, ctx()).unwrap();
            assert_eq!(a, b, "frame {f}");
        }
    }

    /// The point of a Null: it draws nothing, yet a layer parented to it is
    /// placed by its transform exactly as it would be by any other parent's.
    /// If the parent walk ever skipped the pixel-less kinds, a whole rig would
    /// silently stop following its null.
    #[test]
    fn a_child_follows_a_null_parent() {
        let mut null = layer(120.0, 40.0, None);
        null.kind = LayerKind::Null;
        null.name = "Null".into();
        let child = layer(5.0, 5.0, Some(null.id));
        let c = comp(vec![null.clone(), child.clone()]);
        let world =
            parent_world_placement(&c, &child, 0.0, Arc::new(ExpressionContext::detached()))
                .unwrap();
        assert_eq!(world, place_of(&null));
    }
}

#[cfg(test)]
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
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "t".into(),
            kind: LayerKind::Text {
                document: TextDocument {
                    text: "hello".into(),
                    expression: None,
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
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
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

    // An expression-driven text layer must actually rasterise the line the
    // expression works out, at the time the frame is being drawn — otherwise
    // the feature is a document field the picture never reads. The two times
    // give lines of different lengths, so the raster's own size proves it
    // without inspecting glyphs.
    #[test]
    fn expression_driven_text_rasterises_the_line_at_this_time() {
        let mut l = text_layer(0.0);
        if let LayerKind::Text { document } = &mut l.kind {
            document.expression = Some("time * 100000".into());
        }
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour::BLACK,
            work_area: None,
            layers: vec![l],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = Document::new();
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();

        let width_at = |t: f64| {
            let mut visited = vec![comp.id];
            let draws = build_comp_draws(
                &std::sync::Arc::new(doc.clone()),
                &comp,
                t,
                &pixels,
                &mut visited,
            );
            match &draws.first().expect("one draw").source {
                DrawSource::Pixels { tex_w, .. } => *tex_w,
                _ => panic!("a text layer draws its own pixels"),
            }
        };

        // "0.0" against "500000.0" — a longer line, so a wider raster.
        assert!(width_at(5.0) > width_at(0.0));
        assert_eq!(width_at(5.0), width_at(5.0), "and it is deterministic");
    }

    /// A Light layer in a comp must actually change the picture — and a comp
    /// with no lights must render byte-for-byte as it did before lighting
    /// existed (docs/06, K-361). The second half is the compatibility promise:
    /// every project ever saved has no Light layers, and none of them may
    /// shift by a byte.
    ///
    /// Also pinned here: the Accepts lights switch really switches off. That
    /// is the escape hatch a compositor reaches for when a layer must not be
    /// touched, and a switch that quietly does nothing is worse than none.
    #[test]
    fn a_light_layer_lights_the_comp_and_no_lights_changes_nothing() {
        use lumit_core::model::{LightDef, LightKind};

        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = lumit_gpu::ColourEngine::new(&ctx);
        let compositor = lumit_gpu::Compositor::new(&ctx);
        let fx = lumit_gpu::fx::FxEngine::new(&ctx);
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
        };
        // A softbox in front of the comp, big enough to rake across it.
        let mut light = text_layer(160.0);
        light.kind = LayerKind::Light {
            light: Box::new(LightDef {
                kind: LightKind::Area,
                half_size: [Property::fixed(120.0), Property::fixed(80.0)],
                ..LightDef::default()
            }),
        };
        light.transform.position_z = Property::fixed(-200.0);

        let comp_with = |layers: Vec<Layer>| Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.1, 0.1, 1.0]),
            work_area: None,
            layers,
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = std::sync::Arc::new(Document::new());
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let t = 0.3;
        let render = |comp: &Composition| {
            let mut visited = vec![comp.id];
            let draws = build_comp_draws(&doc, comp, t, &pixels, &mut visited);
            let tex = realiser.realise(
                comp.camera_pose(t),
                comp.width,
                comp.height,
                comp.background.0.map(f64::from),
                &draws,
            );
            engine
                .readback8(
                    &ctx,
                    &engine.display(&ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL),
                )
                .unwrap()
        };

        let subject = text_layer(160.0);
        let dark = render(&comp_with(vec![subject.clone()]));
        let dark_again = render(&comp_with(vec![subject.clone()]));
        assert_eq!(
            dark, dark_again,
            "a comp with no lights is deterministic, as it always was"
        );

        let lit = render(&comp_with(vec![subject.clone(), light.clone()]));
        assert_ne!(dark, lit, "a Light layer must actually light the comp");
        assert!(
            lit.iter().zip(&dark).all(|(a, b)| a >= b),
            "light adds, so no channel may come out darker than it went in"
        );

        // The switch off puts it back exactly where it started.
        let mut unlit_subject = subject.clone();
        unlit_subject.switches.accepts_lights = false;
        let unlit = render(&comp_with(vec![unlit_subject, light]));
        assert_eq!(
            dark, unlit,
            "Accepts lights off must render byte-identically to no light at all"
        );
    }

    /// A region of interest composites a window of the comp, and that window
    /// must be **the same pixels** the full frame has there (K-362). This is
    /// the whole promise: a region changes how much is computed, never what
    /// the picture is. If the two ever disagree, working inside a region is
    /// working on a lie.
    ///
    /// Also pinned: the returned texture is the region's size, and a region
    /// covering everything is refused so its frames keep the full frame's
    /// names rather than banking a duplicate set under new ones.
    #[test]
    fn a_region_of_interest_is_the_same_pixels_the_full_frame_has_there() {
        let Ok(ctx) = lumit_gpu::GpuContext::headless() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = lumit_gpu::ColourEngine::new(&ctx);
        let compositor = lumit_gpu::Compositor::new(&ctx);
        let fx = lumit_gpu::fx::FxEngine::new(&ctx);
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
        };
        let comp = Composition {
            id: Uuid::now_v7(),
            name: "c".into(),
            width: 320,
            height: 180,
            frame_rate: FrameRate::new(30, 1).unwrap(),
            duration: Duration(Rational::new(10, 1).unwrap()),
            background: LinearColour([0.1, 0.2, 0.3, 1.0]),
            work_area: None,
            layers: vec![text_layer(200.0), text_layer(80.0)],
            markers: Vec::new(),
            motion_blur: Default::default(),
            extra: serde_json::Map::new(),
        };
        let doc = std::sync::Arc::new(Document::new());
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let t = 0.3;
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(&doc, &comp, t, &pixels, &mut visited);
        let bg = comp.background.0.map(f64::from);

        let bytes = |tex: &wgpu::Texture| {
            engine
                .readback8(
                    &ctx,
                    &engine.display(&ctx, tex, lumit_gpu::DisplayParams::NEUTRAL),
                )
                .unwrap()
        };

        let full = realiser.realise(comp.camera_pose(t), comp.width, comp.height, bg, &draws);
        let full_px = bytes(&full);

        // A window over the busiest quarter of the frame.
        let region = lumit_gpu::Region {
            x: 64.0,
            y: 32.0,
            w: 160.0,
            h: 96.0,
        };
        let windowed = realiser.realise_region(
            comp.camera_pose(t),
            comp.width,
            comp.height,
            bg,
            &draws,
            Some(region),
        );
        assert_eq!(
            (windowed.width(), windowed.height()),
            (region.w as u32, region.h as u32),
            "the texture is the region's size, not the comp's"
        );
        let win_px = bytes(&windowed);

        // Every row of the window against the same row of the full frame.
        let stride = comp.width as usize * 4;
        let mut worst = 0u8;
        for row in 0..region.h as usize {
            let src = (row + region.y as usize) * stride + region.x as usize * 4;
            let dst = row * region.w as usize * 4;
            for i in 0..region.w as usize * 4 {
                worst = worst.max(full_px[src + i].abs_diff(win_px[dst + i]));
            }
        }
        assert!(
            worst <= 1,
            "a windowed composite must match the full frame there; worst channel difference {worst}"
        );

        // And it is deterministic, as every composite must be.
        let again = realiser.realise_region(
            comp.camera_pose(t),
            comp.width,
            comp.height,
            bg,
            &draws,
            Some(region),
        );
        assert_eq!(win_px, bytes(&again), "a windowed composite is bit-stable");
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
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
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
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        // Compute the background exactly as render_below_at does (from the f32
        // LinearColour via f64::from), so the plain composite and the re-render
        // clear to identical values and the comparison is honest.
        let bg = comp.background.0.map(f64::from);
        let t = 0.3;

        let mut v1 = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            t,
            &pixels,
            &mut v1,
        );
        let normal = realiser.realise(comp.camera_pose(t), comp.width, comp.height, bg, &draws);
        let normal_bytes = engine
            .readback8(
                &ctx,
                &engine.display(&ctx, &normal, lumit_gpu::DisplayParams::NEUTRAL),
            )
            .unwrap();

        // Re-render the whole stack (every layer counts as "below") at the same
        // time through the shared helper.
        let mut v2 = vec![comp.id];
        let below = render_below_at(
            &realiser,
            &std::sync::Arc::new(doc.clone()),
            &comp,
            &comp.layers,
            t,
            t,
            None,
            &pixels,
            &mut v2,
        );
        let below_bytes = engine
            .readback8(
                &ctx,
                &engine.display(&ctx, &below, lumit_gpu::DisplayParams::NEUTRAL),
            )
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

    // The one blur op's radius in raster pixels. Blur lives in the registry
    // (docs/impl/effect-registry.md §2.1), so its numbers come back out of the
    // arena through its own typed reader rather than out of a `Resolved` variant.
    fn blur_radius_px(fx: &lumit_core::fx::ResolvedStack) -> f32 {
        use lumit_core::fx::{effects::blur::Blur, EffectMetadata};
        let op = fx.get(0).expect("expected a blur op");
        assert_eq!(op.def.schema().match_name, "blur");
        Blur::read(op.params).packed().0
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
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "posterize".into(),
            kind: LayerKind::Adjustment,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
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
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        // t = 0.35, 10 fps grid → held tau = floor(3.5)/10 = 0.3.
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.35,
            &pixels,
            &mut visited,
        );
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
    // ramps 0→100 px over a second and opts out of sampling; under a 10 fps
    // posterise at t = 0.35 (held tau = 0.3) its transform holds at x = 30 but its
    // blur resolves at the frame time 0.35 (35 px), not 0.3.
    #[test]
    fn a_non_sampling_below_effect_holds_at_the_frame_time_not_the_grid() {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 100.0); // x = 100·t
        let mut blur = lumit_core::fx::instantiate("blur").unwrap();
        blur.sample_temporally = false;
        for p in &mut blur.params {
            if p.id == "radius" {
                p.value = lumit_core::model::EffectValue::Float(ramp(0.0, 100.0));
                // radius px = 100·t
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
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.35,
            &pixels,
            &mut visited,
        );
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
        // The blur, opting out, resolves at the frame time 0.35 (35 px).
        let radius = blur_radius_px(&tb.draws[0].fx);
        assert!(
            (radius - 35.0).abs() < 0.5,
            "blur must hold at the frame time 0.35 (35 px), got {radius}"
        );
        assert!(
            (radius - 30.0).abs() > 4.0,
            "blur must NOT sample the held time 0.30; got {radius}"
        );
    }

    // docs/08 §3.25: a Posterize time scoped to *This layer's effects* holds only
    // the layer's OWN effect stack on the coarse grid — no re-render of others,
    // no adjustment (no orchestration re-entry). The text carries a blur (radius
    // ramps 0→100 px over a second) and a 10 fps this-layer Posterize; at t = 0.35
    // its transform stays live (x = 35) while the blur resolves at the held time
    // 0.3 (30 px), not 0.35. GPU-free structural check.
    #[test]
    fn this_layer_posterize_holds_the_layers_own_effects_but_not_its_transform() {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 100.0); // x = 100·t (stays live)
        let mut blur = lumit_core::fx::instantiate("blur").unwrap();
        for p in &mut blur.params {
            if p.id == "radius" {
                p.value = lumit_core::model::EffectValue::Float(ramp(0.0, 100.0));
                // px = 100·t
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
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.35,
            &pixels,
            &mut visited,
        );
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
        // The blur resolves at the held time 0.3 (30 px), not 0.35.
        let radius = blur_radius_px(&d.fx);
        assert!(
            (radius - 30.0).abs() < 0.5,
            "blur held at the grid time 0.3 (30 px); got {radius}"
        );
        assert!(
            (radius - 35.0).abs() > 4.0,
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
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
        };
        let comp = posterize_comp();
        let doc = Document::new();
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let bg = comp.background.0.map(f64::from);

        // The posterised frame at t = 0.35.
        let mut v1 = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.35,
            &pixels,
            &mut v1,
        );
        let posterised =
            realiser.realise(comp.camera_pose(0.35), comp.width, comp.height, bg, &draws);
        let posterised_bytes = engine
            .readback8(
                &ctx,
                &engine.display(&ctx, &posterised, lumit_gpu::DisplayParams::NEUTRAL),
            )
            .unwrap();

        // A plain render of the below-stack (just the text) at tau = 0.3.
        let below = &comp.layers[1..];
        let mut v2 = vec![comp.id];
        // frame_t = 0.35 matches what the posterise adjustment passes (its own
        // frame time), so the two below-renders build the identical draws.
        let held = render_below_at(
            &realiser,
            &std::sync::Arc::new(doc.clone()),
            &comp,
            below,
            0.3,
            0.35,
            None,
            &pixels,
            &mut v2,
        );
        let held_bytes = engine
            .readback8(
                &ctx,
                &engine.display(&ctx, &held, lumit_gpu::DisplayParams::NEUTRAL),
            )
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
            graph: Default::default(),
            markers: Vec::new(),
            id: Uuid::now_v7(),
            name: "accumulation".into(),
            kind: LayerKind::Adjustment,
            in_point: CompTime(Rational::ZERO),
            out_point: CompTime(Rational::new(10, 1).unwrap()),
            start_offset: CompTime(Rational::ZERO),
            transform: TransformGroup::default(),
            matte: None,
            parent: None,
            label: 0,
            volume_db: lumit_core::anim::Property::zero(),
            audio_only: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
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
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let mut visited = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.5,
            &pixels,
            &mut visited,
        );
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
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine: &engine,
            compositor: &compositor,
            fx: &fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
        };
        let doc = Document::new();
        let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
        let render = |comp: &Composition, t: f64| -> Vec<u8> {
            let mut v = vec![comp.id];
            let draws =
                build_comp_draws(&std::sync::Arc::new(doc.clone()), comp, t, &pixels, &mut v);
            let bg = comp.background.0.map(f64::from);
            let tex = realiser.realise(comp.camera_pose(t), comp.width, comp.height, bg, &draws);
            engine
                .readback8(
                    &ctx,
                    &engine.display(&ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL),
                )
                .unwrap()
        };

        // STILL scene: a static text below a 4-sample accumulation adjustment must
        // be a bit-exact identity — every sub-frame render is equal, so their
        // average is the plain composite (1/4 is exact in fp16, four copies sum
        // back exactly), and the full-coverage blend lays it back unchanged.
        //
        // On a software rasteriser (lavapipe in Linux CI) the two paths can land
        // one 8-bit step apart: the sum-and-divide runs through different fp16
        // rounding than the single composite, and an implementation is free to
        // round intermediates differently from hardware. The identity is still
        // checked there — within one step, which a genuinely broken accumulation
        // (wrong weights, dropped samples) would blow past — while real GPUs,
        // the platform Lumit actually renders on, must match to the bit.
        let still_text = text_layer(120.0);
        let still_plain = comp_with(30, vec![still_text.clone()]);
        let still_acc = comp_with(30, vec![accumulation_adjustment(4.0), still_text]);
        let (plain_still, acc_still) = (render(&still_plain, 0.5), render(&still_acc, 0.5));
        if ctx.software {
            let worst = plain_still
                .iter()
                .zip(&acc_still)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(
                worst <= 1,
                "a still scene averaged over N must equal the plain composite \
                 (software adapter: within one 8-bit step; worst was {worst})"
            );
        } else {
            assert_eq!(
                plain_still, acc_still,
                "a still scene averaged over N must equal the plain composite bit-for-bit"
            );
        }

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
