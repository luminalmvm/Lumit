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
//! in the viewport and the file.

use crate::decode::CompLayerPixels;
use crate::draw::{
    AccumulationBelow, CompLayerDraw, DofInputDraw, DrawSource, GraphDraw, GraphStep,
    LayerInputDraw, MatteDraw, TemporalBelow,
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
/// One layer's own picture: the bytes, the decoded size, the native source
/// size, and how wide the bytes' samples are. Everything but a float footage
/// source is `Srgb8`, which is what it has always been.
pub type LayerPixels = (
    Arc<Vec<u8>>,
    u32,
    u32,
    (f32, f32),
    lumit_media::decode::PixelFormat,
);

/// The map of already-decoded pixels a build reads, keyed by layer id.
pub type PixelsByLayer<'a> = HashMap<Uuid, &'a CompLayerPixels>;

/// The meshes and factorisations every puppeted layer in the process shares
/// (docs/impl/puppet.md §1.4, §2.5). Both are keyed by content — the mesh by
/// the alpha it was built over, the factorisation by the mesh and the pin
/// structure — so a shared cache can only ever hand back the thing that would
/// have been computed, and a mesh costing up to 100 ms is built once rather
/// than once a frame.
static PUPPET: std::sync::LazyLock<lumit_core::puppet::PuppetCache> =
    std::sync::LazyLock::new(lumit_core::puppet::PuppetCache::new);

/// Warp one layer's own pixels through its puppet (docs/impl/puppet.md §3),
/// in place, at the paint/masks seam.
///
/// `solo_at` renders the layer alone at a layer time — the mesh is built over
/// that picture at the block's **reference time**, which is the alpha the pins
/// were placed on, never this frame's.
///
/// Every refusal here is a value and leaves the layer exactly as it was: no
/// pins, an empty picture at the reference time, a mesh too dense to build
/// (docs/14 §4).
fn warp_puppet(
    pixels: &mut LayerPixels,
    layer: &lumit_core::model::Layer,
    block: &lumit_core::puppet::PuppetBlock,
    lt: f64,
    solo_at: &impl Fn(&lumit_core::model::Layer, f64) -> Option<LayerPixels>,
) {
    let (_, w, h, natural, _) = pixels;
    let (w, h, natural) = (*w, *h, *natural);
    let pins = block.pins_at(lt);
    // Nothing pinned warps nothing — unless a puppet tool is armed on this
    // layer, in which case the overlay still wants the mesh to aim the next pin
    // at, and the block's own reference time is where it is cut from.
    if pins.is_empty() && crate::puppet::previewing(layer.id).is_none() {
        return;
    }
    let nw = natural.0.round().max(1.0) as u32;
    let nh = natural.1.round().max(1.0) as u32;
    // ponytail: the layer is rendered solo at the reference time on every
    // frame, only to be thrown away when the mesh cache hits on its alpha. The
    // upgrade is a per-layer memo of that alpha's hash, keyed on the layer's
    // own content stamp. Observable trigger: a puppeted footage or text layer
    // showing up in a docs/13 B3/B6 trace, where the doubled source cost is
    // what the trace names.
    //
    // ponytail: and a **footage** layer's pixels here are this frame's decoded
    // ones, not the reference time's — the decode planner schedules one frame
    // per layer per rendered frame, and asking it for a second moment is a
    // change to the plan rather than to this seam (docs/impl/puppet.md §1.1).
    // Every self-rasterising kind is exact; footage whose silhouette does not
    // move is exact too. Observable trigger: a mesh that visibly follows the
    // shot instead of holding still — pins going inert as a figure walks out of
    // where it stood at the reference time.
    let Some((reference, rw, rh, _, rfmt)) = solo_at(layer, block.reference_time.to_f64()) else {
        return;
    };
    // The reference frame is read for its alpha alone, at whichever width it
    // arrived at — the bitmap that comes out is a silhouette either way.
    let alpha = match rfmt {
        lumit_media::decode::PixelFormat::LinearF32 => {
            lumit_core::puppet::alpha_at_natural_f32(&reference, rw, rh, nw, nh)
        }
        lumit_media::decode::PixelFormat::Srgb8 => {
            lumit_core::puppet::alpha_at_natural(&reference, rw, rh, nw, nh)
        }
    };
    let Ok(mesh) = PUPPET.mesh(&alpha, nw, nh, block.density, block.expansion) else {
        crate::puppet::forget(layer.id);
        return;
    };
    if pins.is_empty() {
        let deformed = mesh.vertices.clone();
        crate::puppet::publish(
            layer.id,
            crate::puppet::Ghost {
                mesh,
                deformed,
                inert: Vec::new(),
            },
        );
        return;
    }
    let factorisation = PUPPET.factorisation(&mesh, &pins);
    let solution = lumit_core::puppet::solve(&mesh, &pins, &factorisation);
    // What the Viewer's wireframe draws is the mesh that drew the pixels, not a
    // second copy of it (docs/impl/puppet.md §5): left here, read by the bridge.
    crate::puppet::publish(
        layer.id,
        crate::puppet::Ghost {
            mesh: Arc::clone(&mesh),
            deformed: solution.vertices.clone(),
            inert: solution.inert.clone(),
        },
    );
    let warp = match pixels.4 {
        lumit_media::decode::PixelFormat::LinearF32 => lumit_core::puppet::apply_puppet_f32,
        lumit_media::decode::PixelFormat::Srgb8 => lumit_core::puppet::apply_puppet,
    };
    warp(
        Arc::make_mut(&mut pixels.0).as_mut_slice(),
        w,
        h,
        f64::from(natural.0),
        f64::from(natural.1),
        &mesh,
        &solution,
    );
}

/// The unmoved mesh of a layer a puppet tool is armed on but nobody has pinned,
/// published for the overlay alone (docs/impl/puppet.md §5).
///
/// The pixels are the layer's own, at this frame, already made — so this costs
/// one alpha lift and one *cached* mesh build and never a render. A mesh that
/// cannot be built (nothing at or above 10 % alpha) takes the layer's wireframe
/// away rather than leaving the last one lying over a picture it no longer fits;
/// that absence is what the first click reads as its refusal.
fn preview_puppet(pixels: &LayerPixels, layer: Uuid, density: f64, expansion: f64) {
    let (rgba, w, h, natural, _) = pixels;
    let nw = natural.0.round().max(1.0) as u32;
    let nh = natural.1.round().max(1.0) as u32;
    let alpha = lumit_core::puppet::alpha_at_natural(rgba, *w, *h, nw, nh);
    let Ok(mesh) = PUPPET.mesh(&alpha, nw, nh, density, expansion) else {
        crate::puppet::forget(layer);
        return;
    };
    let deformed = mesh.vertices.clone();
    crate::puppet::publish(
        layer,
        crate::puppet::Ghost {
            mesh,
            deformed,
            inert: Vec::new(),
        },
    );
}

/// The single `model::BlendMode` → `gpu::Blend` mapping shared by every path
/// that composites (they must never disagree). Every mode maps to its
/// like-named GPU variant (T24).
#[must_use]
pub fn blend_of(b: lumit_core::model::BlendMode) -> lumit_gpu::composite::Blend {
    use lumit_core::model::BlendMode as M;
    use lumit_gpu::composite::Blend as G;
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
    if let Some(slot) = patched
        .layers
        .iter_mut()
        .find(|l| l.id == layer)
        .and_then(|l| l.prop_mut(prop))
    {
        *slot = lumit_core::anim::Property::fixed(value);
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
/// time `t_comp` (layer parenting): `P_top × … × P_grandparent ×
/// P_parent`, each ancestor's `place_matrix` sampled at its own local time
/// (`t_comp − start_offset`). `None` when the layer has no parent. Used as a
/// draw's `pre`, which the GPU applies as `pre × own_placement` — so the child
/// ends up placed inside its parent's coordinate space (After Effects
/// parenting). Cycle- and missing-parent-safe via `model::layer_parent_chain`.
/// Shared by the preview (here) and the export path so the two stay identical.
/// v1 composes the full `place_matrix` (2D plus the 2.5D axes it already
/// carries); no behaviour changes for an unparented layer (`None`).
pub fn parent_world_placement(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> Option<[[f32; 4]; 4]> {
    let mut world: Option<[[f32; 4]; 4]> = None;
    // Outermost first, so concat_place(outer, inner) = outer × inner.
    let read = lumit_core::model::context_reader(&context, t_comp);
    let solves = crate::track::solves_for(&context.document);
    for (_, p) in lumit_core::model::parent_placements(comp, layer, t_comp, &read, solves) {
        let p = place_of(&p);
        world = Some(match world {
            Some(w) => lumit_gpu::composite::concat_place(w, p),
            None => p,
        });
    }
    world
}

/// Where `layer` sits in `comp` at comp time `t_comp`, parents and all: its
/// own pixels to comp pixels, as the compositor places it.
/// Whether `layer` carries a live effect drawn over the composite below
/// ([`EffectDef::backdrop`](lumit_core::fx::registry::EffectDef::backdrop)),
/// which makes it an adjustment for this frame without being one in the
/// project: its picture is still decoded, and is that effect's shape.
fn backdrop_fx(layer: &lumit_core::model::Layer) -> bool {
    layer.switches.fx
        && layer.effects.iter().any(|e| {
            e.enabled
                && e.effect.namespace == lumit_core::model::EffectNamespace::Lfx
                && lumit_core::fx::catalogue::BUILTIN_DEFS
                    .get(&e.effect.match_name)
                    .is_some_and(|d| d.backdrop())
        })
}

fn world_placement(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> [[f32; 4]; 4] {
    let own = own_placement(layer, t_comp, &context);
    match parent_world_placement(comp, layer, t_comp, context) {
        Some(pre) => lumit_gpu::composite::concat_place(pre, own),
        None => own,
    }
}

/// `layer`'s own placement at comp time `t_comp`, before any parent's.
fn own_placement(
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: &Arc<ExpressionContext>,
) -> [[f32; 4]; 4] {
    place_of(&lumit_core::model::placement(
        layer,
        t_comp,
        &lumit_core::model::context_reader(context, t_comp),
    ))
}

fn place_of(p: &lumit_core::model::Placement) -> [[f32; 4]; 4] {
    let f = |v: (f64, f64, f64)| (v.0 as f32, v.1 as f32, v.2 as f32);
    lumit_gpu::composite::place_matrix_3d(f(p.position), f(p.anchor), f(p.scale), f(p.rotation_deg))
}

/// What the compositor draws `layer`'s own picture with at layer time `lt`,
/// each property read under `context`: any Orientation turned into the
/// rotations and Anchor Z into the position. A 2D layer's hidden 3D values
/// count as nought, as the compositor draws them.
fn drawn(
    layer: &lumit_core::model::Layer,
    lt: f64,
    context: &Arc<ExpressionContext>,
) -> lumit_gpu::composite::MbSample {
    let read = |p: &lumit_core::anim::Property| p.value_at_with_context(lt, context.clone());
    let p = layer
        .transform
        .placement_at(lt, !layer.switches.three_d, &read);
    let at = p.flat_position();
    lumit_gpu::composite::MbSample {
        position: (at.0 as f32, at.1 as f32),
        anchor: (p.anchor.0 as f32, p.anchor.1 as f32),
        scale: (p.scale.0 as f32, p.scale.1 as f32),
        rotation_deg: p.rotation_deg.2 as f32,
        z: at.2 as f32,
        rotation_x_deg: p.rotation_deg.0 as f32,
        rotation_y_deg: p.rotation_deg.1 as f32,
    }
}

/// The transform edits that keep `layer` where it is at comp time `t_comp`
/// when its parent becomes `new_parent`. Taking a parent, losing one or
/// swapping one changes what the layer follows from then on. It must not move
/// the picture on the frame it happens.
///
/// A parented layer's values are read inside its parent's space, so the same
/// numbers under a new parent land somewhere else. This works out the numbers
/// that land it where it already was: position, scale and the rotation, or on
/// a 3D layer the Orientation, its X, Y and Z Rotation staying as they are.
/// The anchor point stays, and opacity is not inherited. A property that
/// already reads right is left out, and so is one driven by an expression.
///
/// ponytail: a keyed property moves as a whole, each axis on its own, so its
/// other keys only hold their place under a parent that is not turned. Carry
/// position keys as x, y pairs if a turned parent needs it.
pub fn reparented(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    new_parent: Option<Uuid>,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> Vec<(
    lumit_core::model::TransformProp,
    lumit_core::anim::Animation,
)> {
    use lumit_core::model::TransformProp as P;

    if matches!(
        layer.kind,
        lumit_core::model::LayerKind::Camera { .. } | lumit_core::model::LayerKind::Light { .. }
    ) {
        return reparented_eye(comp, layer, new_parent, t_comp, &context);
    }
    let identity = glam::Mat4::IDENTITY.to_cols_array_2d();
    let was = parent_world_placement(comp, layer, t_comp, context.clone()).unwrap_or(identity);
    let now = match new_parent.and_then(|id| comp.layers.iter().find(|l| l.id == id)) {
        Some(parent) => {
            let own = own_placement(parent, t_comp, &context);
            match parent_world_placement(comp, parent, t_comp, context.clone()) {
                Some(world) => lumit_gpu::composite::concat_place(world, own),
                None => own,
            }
        }
        None => identity,
    };
    let Some(out) = lumit_gpu::composite::invert_place(now).filter(|_| was != now) else {
        return Vec::new();
    };
    // Out of the old parent's space and into the new one's.
    let carry = lumit_gpu::composite::concat_place(out, was);

    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let tr = &layer.transform;
    let at = |p: &lumit_core::anim::Property| p.value_at_with_context(lt, context.clone());
    let placed = lumit_gpu::composite::concat_place(carry, own_placement(layer, t_comp, &context));
    let mut to = lumit_gpu::composite::unplace_matrix(
        placed,
        (at(&tr.anchor_x) as f32, at(&tr.anchor_y) as f32),
    );
    // The anchor's own z stays where it is, so the position takes what it
    // leaves.
    if layer.switches.three_d {
        let a = [&tr.anchor_x, &tr.anchor_y, &tr.anchor_z].map(|p| at(p) as f32);
        let p = |r: usize| {
            placed[0][r] * a[0] + placed[1][r] * a[1] + placed[2][r] * a[2] + placed[3][r]
        };
        (to.position, to.z) = ((p(0), p(1)), p(2));
    }
    // Scale Z stretches what is parented to the layer and how far Anchor Z
    // pushes it, so it is carried as the other two are.
    let sz = {
        let c = placed[2];
        let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() * 100.0;
        if at(&tr.scale_z) < 0.0 {
            -len
        } else {
            len
        }
    };
    // A flip can be written two ways: one axis negative, or the other one
    // negative and half a turn. Keep the way the layer already has it.
    let (sx, sy, mut rz) = if f64::from(to.scale.0) * at(&tr.scale_x) < 0.0 {
        (-to.scale.0, -to.scale.1, to.rotation_deg + 180.0)
    } else {
        (to.scale.0, to.scale.1, to.rotation_deg)
    };

    // A 3D layer keeps its rotations and its Orientation takes the turn, as
    // After Effects does it, unless an expression drives the Orientation.
    let reorient = layer.switches.three_d
        && !tr
            .orientation
            .iter()
            .any(|p| matches!(p.animation, lumit_core::anim::Animation::Expression(_)));
    let mut edits = Vec::new();
    if layer.switches.three_d {
        let orientation = tr.orientation_at(lt, &at);
        let turn = (
            f64::from(to.rotation_x_deg),
            f64::from(to.rotation_y_deg),
            f64::from(rz),
        );
        let rows = (at(&tr.rotation_x), at(&tr.rotation_y), at(&tr.rotation));
        let ae_order = tr.after_effects_order;
        if reorient {
            let o = lumit_core::camera::reoriented(turn, rows, ae_order, orientation);
            edits = orientation_edits(tr, orientation, o);
        } else {
            let (x, y, z) = lumit_core::camera::unoriented(orientation, turn, ae_order, rows);
            (to.rotation_x_deg, to.rotation_y_deg, rz) = (x as f32, y as f32, z as f32);
        }
    }

    let mut edit = |prop: P, to: f32, turns: bool, gain: Option<f64>| {
        let Some(property) = tr.get(prop) else { return };
        let animation = made_up(property, at(property), f64::from(to), turns, gain);
        edits.extend(animation.map(|a| (prop, a)));
    };
    let along = |axis: usize| Some(f64::from(carry[axis][axis]));
    edit(P::PositionX, to.position.0, false, along(0));
    edit(P::PositionY, to.position.1, false, along(1));
    edit(P::ScaleX, sx, false, None);
    edit(P::ScaleY, sy, false, None);
    if !reorient {
        edit(P::Rotation, rz, true, Some(1.0));
    }
    // A 2D layer's hidden 3D values are kept for when it goes 3D again.
    if layer.switches.three_d {
        edit(P::PositionZ, to.z, false, along(2));
        if !reorient {
            edit(P::RotationX, to.rotation_x_deg, true, Some(1.0));
            edit(P::RotationY, to.rotation_y_deg, true, Some(1.0));
        }
        edit(P::ScaleZ, sz, false, None);
    }
    edits
}

/// The edits that take `tr`'s Orientation from `from` to `to` at the moment
/// of a reparent. A still one is written as it is; a keyed one has every key
/// turned by the same turn, so its whole move comes along, with all three
/// angles keyed together as After Effects keys them.
fn orientation_edits(
    tr: &lumit_core::model::TransformGroup,
    from: (f64, f64, f64),
    to: (f64, f64, f64),
) -> Vec<(
    lumit_core::model::TransformProp,
    lumit_core::anim::Animation,
)> {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    use lumit_core::model::TransformProp as P;
    const AXES: [P; 3] = [P::OrientationX, P::OrientationY, P::OrientationZ];

    // The f64 maths is exact well past a thousandth; past that it is noise.
    let differs = |a: f64, b: f64| ((a - b) * 1e3).round() != 0.0;
    if !(differs(from.0, to.0) || differs(from.1, to.1) || differs(from.2, to.2)) {
        return Vec::new();
    }
    let lanes = tr.orientation.each_ref().map(|p| match &p.animation {
        Animation::Keyframed(keys) => keys.as_slice(),
        _ => &[],
    });
    let mut times: Vec<_> = lanes.iter().copied().flatten().map(|k| k.time).collect();
    times.sort();
    times.dedup();
    if times.is_empty() {
        let to = [to.0, to.1, to.2];
        return (0..3)
            .filter(|&i| differs(tr.orientation[i].value_at(0.0), to[i]))
            .map(|i| (AXES[i], Animation::Static(to[i])))
            .collect();
    }
    let turned: Vec<_> = times
        .iter()
        .map(|time| {
            let lt = time.to_f64();
            let o = tr.orientation_at(lt, &|p| p.value_at(lt));
            let o = lumit_core::camera::orientation_turned(o, from, to);
            [o.0, o.1, o.2]
        })
        .collect();
    // A key an angle didn't have borrows another's ease, but not its speed,
    // which belonged to that one angle.
    let borrowed = |side: SideInterp| match side {
        SideInterp::Bezier { speed, .. } if speed != 0.0 => SideInterp::Linear,
        other => other,
    };
    (0..3)
        .map(|i| {
            let keys = times
                .iter()
                .zip(&turned)
                .map(|(&time, o)| {
                    let own = lanes[i].iter().find(|k| k.time == time);
                    let (interp_in, interp_out) = match own {
                        Some(k) => (k.interp_in, k.interp_out),
                        None => lanes
                            .iter()
                            .find_map(|lane| lane.iter().find(|k| k.time == time))
                            .map_or((SideInterp::Linear, SideInterp::Linear), |k| {
                                (borrowed(k.interp_in), borrowed(k.interp_out))
                            }),
                    };
                    Keyframe {
                        time,
                        value: o[i],
                        interp_in,
                        interp_out,
                    }
                })
                .collect();
            (AXES[i], Animation::Keyframed(keys))
        })
        .collect()
}

/// The ops that make `parent` the parent of `layer` in `comp` and keep the
/// layer where it is at comp time `t_comp`. The parent goes first: it names
/// the undo step, and a refused one takes the transform edits down with it.
pub fn parent_ops(
    doc: &Arc<lumit_core::model::Document>,
    comp: Uuid,
    layer: Uuid,
    parent: Option<Uuid>,
    t_comp: f64,
) -> Vec<lumit_core::ops::Op> {
    let mut ops = vec![lumit_core::ops::Op::SetLayerParent {
        comp,
        layer,
        parent,
    }];
    let Some(c) = doc.comp(comp) else {
        return ops;
    };
    let Some(l) = c.layers.iter().find(|l| l.id == layer) else {
        return ops;
    };
    let context = Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(comp),
        layer: Some(layer),
        comp_time: t_comp,
        current_depth: 0,
        inputs: None,
    });
    ops.extend(
        reparented(c, l, parent, t_comp, context)
            .into_iter()
            .map(
                |(prop, animation)| lumit_core::ops::Op::SetTransformProperty {
                    comp,
                    layer,
                    prop,
                    animation,
                },
            ),
    );
    ops
}

/// [`reparented`] for a camera or a light, which has no scale or anchor of its
/// own: the eye, the Orientation and an aimed one's point of interest, carried
/// out of the old parent's space and into the new one's.
///
/// As After Effects does it, every key is carried through the same two parents
/// as they stand at `t_comp`, so a keyed camera keeps its whole move. The
/// channels are carried together as one eye, so a key on any of them is a key
/// on all of them, and a key already there keeps its ease.
fn reparented_eye(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    new_parent: Option<Uuid>,
    t_comp: f64,
    context: &Arc<ExpressionContext>,
) -> Vec<(
    lumit_core::model::TransformProp,
    lumit_core::anim::Animation,
)> {
    use lumit_core::anim::{Animation, Keyframe, SideInterp};
    use lumit_core::camera::{place, Eye};
    use lumit_core::model::{parent_space, LayerKind, TransformProp as P};

    let identity = place(
        (0.0, 0.0, 0.0),
        (0.0, 0.0, 0.0),
        (100.0, 100.0, 100.0),
        (0.0, 0.0, 0.0),
    );
    let read = lumit_core::model::context_reader(context, t_comp);
    let solves = crate::track::solves_for(&context.document);
    let was = parent_space(comp, layer, t_comp, &read, solves).unwrap_or(identity);
    let now = match new_parent.and_then(|id| comp.layers.iter().find(|l| l.id == id)) {
        Some(parent) => lumit_core::model::layer_space(comp, parent, t_comp, &read, solves),
        None => identity,
    };
    if was == now {
        return Vec::new();
    }
    let Some(carry) = now.invert().map(|back| was.within(&back)) else {
        return Vec::new();
    };
    let two_node = match &layer.kind {
        LayerKind::Camera { options, .. } => options.two_node,
        LayerKind::Light { light } => light.two_node,
        _ => false,
    };
    // The rows stay and the Orientation takes the turn, as on any 3D layer,
    // unless an expression drives the Orientation.
    let reorient = !layer
        .transform
        .orientation
        .iter()
        .any(|p| matches!(p.animation, Animation::Expression(_)));
    let mut props = vec![P::PositionX, P::PositionY, P::PositionZ];
    if reorient {
        props.extend([P::OrientationX, P::OrientationY, P::OrientationZ]);
    } else {
        props.extend([P::RotationX, P::RotationY, P::Rotation]);
    }
    if two_node {
        props.extend([P::PoiX, P::PoiY, P::PoiZ]);
    }
    let ae_order = layer.transform.after_effects_order;
    // Every channel's value at layer time `lt`, carried as one eye.
    let carried = |lt: f64| {
        let at = |prop: P| {
            layer
                .prop(prop)
                .map_or(0.0, |p| p.value_at_with_context(lt, context.clone()))
        };
        let o = layer
            .transform
            .orientation_at(lt, &|p| p.value_at_with_context(lt, context.clone()));
        let rows = (at(P::RotationX), at(P::RotationY), at(P::Rotation));
        let eye = carry.carry(Eye {
            position: (at(P::PositionX), at(P::PositionY), at(P::PositionZ)),
            rotation_deg: lumit_core::camera::oriented(o, rows, ae_order),
            point_of_interest: two_node.then(|| (at(P::PoiX), at(P::PoiY), at(P::PoiZ))),
        });
        let poi = eye.point_of_interest.unwrap_or_default();
        let (p, r) = if reorient {
            let o = lumit_core::camera::reoriented(eye.rotation_deg, rows, ae_order, o);
            (eye.position, o)
        } else {
            let rows = lumit_core::camera::unoriented(o, eye.rotation_deg, ae_order, rows);
            (eye.position, rows)
        };
        [p.0, p.1, p.2, r.0, r.1, r.2, poi.0, poi.1, poi.2]
    };
    let slot = |prop: P| props.iter().position(|p| *p == prop).unwrap_or(0);
    // The f64 maths is exact well past a thousandth; past that it is noise.
    let differs = |a: f64, b: f64| ((a - b) * 1e3).round() != 0.0;

    let keys = |property: &lumit_core::anim::Property| match &property.animation {
        Animation::Keyframed(keys) => keys.clone(),
        _ => Vec::new(),
    };
    let times_of = |props: &[P]| {
        let mut times: Vec<_> = props
            .iter()
            .filter_map(|&p| layer.prop(p))
            .flat_map(keys)
            .map(|k| k.time)
            .collect();
        times.sort();
        times.dedup();
        times
    };
    // Where the carry turns every key alike, which it does unless an aim or a
    // stretched parent bends it, the Orientation keeps its own keys and a
    // still one stays still.
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let read_o = |lt: f64| {
        layer
            .transform
            .orientation_at(lt, &|p| p.value_at_with_context(lt, context.clone()))
    };
    let (from, to) = {
        let v = carried(lt);
        (read_o(lt), (v[3], v[4], v[5]))
    };
    let mut all = props.clone();
    all.extend([P::RotationX, P::RotationY, P::Rotation]);
    let alike = reorient
        && times_of(&all).iter().all(|time| {
            let (lt, v) = (time.to_f64(), carried(time.to_f64()));
            let want = lumit_core::camera::orientation_turned(read_o(lt), from, to);
            lumit_core::camera::turn_between_deg((v[3], v[4], v[5]), want) < 1e-3
        });
    let mut edits = if alike {
        orientation_edits(&layer.transform, from, to)
    } else {
        Vec::new()
    };
    let channels: Vec<_> = props
        .iter()
        .filter(|p| !(alike && matches!(p, P::OrientationX | P::OrientationY | P::OrientationZ)))
        .filter_map(|&p| Some((p, layer.prop(p)?)))
        .collect();
    let lanes: Vec<Vec<Keyframe>> = channels.iter().map(|(_, p)| keys(p)).collect();
    let mut times: Vec<_> = lanes.iter().flatten().map(|k| k.time).collect();
    times.sort();
    times.dedup();

    if times.is_empty() {
        let now = carried(lt);
        for (prop, property) in &channels {
            if let Animation::Static(v) = property.animation {
                if differs(v, now[slot(*prop)]) {
                    edits.push((*prop, Animation::Static(now[slot(*prop)])));
                }
            }
        }
        return edits;
    }
    // A key already here lends its ease, unless that ease carries a speed of
    // its own that belonged to the one axis.
    let side = |side: SideInterp| match side {
        SideInterp::Bezier { speed, .. } if speed != 0.0 => SideInterp::Linear,
        other => other,
    };
    let holds = |lane: &[Keyframe], time| {
        lane.iter()
            .rev()
            .find(|k| k.time <= time)
            .is_none_or(|k| k.is_hold() || lane.last().is_some_and(|last| last.time <= time))
    };
    // Position and the point of interest are points, so where all three axes
    // share a key, the speeds of their eases are carried as one vector.
    let pointed = |i: usize, time, pick: fn(&Keyframe) -> SideInterp| {
        let first = match i {
            0..=2 => 0,
            6..=8 => 6,
            _ => return None,
        };
        let eases: Option<Vec<(f64, f64)>> = (first..first + 3)
            .map(
                |axis| match lanes.get(axis)?.iter().find(|k| k.time == time).map(pick) {
                    Some(SideInterp::Bezier { speed, influence }) => Some((speed, influence)),
                    _ => None,
                },
            )
            .collect();
        let eases = eases?;
        let row = carry.m[i - first];
        Some(SideInterp::Bezier {
            speed: row[0] * eases[0].0 + row[1] * eases[1].0 + row[2] * eases[2].0,
            influence: eases[i - first].1,
        })
    };
    let values: Vec<_> = times.iter().map(|time| carried(time.to_f64())).collect();
    for ((prop, property), lane) in channels.iter().zip(&lanes) {
        if matches!(property.animation, Animation::Expression(_)) {
            continue;
        }
        let i = slot(*prop);
        let new: Vec<Keyframe> = times
            .iter()
            .zip(&values)
            .map(|(&time, v)| {
                let here = lane.iter().find(|k| k.time == time);
                let interp_out = if lanes.iter().all(|l| holds(l, time)) {
                    SideInterp::Hold
                } else if let Some(ease) = pointed(i, time, |k| k.interp_out) {
                    ease
                } else {
                    match here.map(|k| side(k.interp_out)) {
                        Some(SideInterp::Hold) | None => SideInterp::Linear,
                        Some(other) => other,
                    }
                };
                let interp_in = pointed(i, time, |k| k.interp_in)
                    .unwrap_or_else(|| here.map_or(SideInterp::Linear, |k| side(k.interp_in)));
                Keyframe {
                    time,
                    value: v[i],
                    interp_in,
                    interp_out,
                }
            })
            .collect();
        // A still channel the carry leaves still stays a single value.
        if let Animation::Static(v) = property.animation {
            let first = new.first().map_or(v, |k| k.value);
            if new.iter().all(|k| !differs(k.value, first)) {
                if differs(first, v) {
                    edits.push((*prop, Animation::Static(first)));
                }
                continue;
            }
        }
        // A lane the carry leaves where it was keeps its own keys and eases.
        let same = new
            .iter()
            .all(|k| !differs(k.value, property.value_at(k.time.to_f64())));
        if !same {
            edits.push((*prop, Animation::Keyframed(new)));
        }
    }
    edits
}

/// The animation that takes `property` from `from` to `to` at the moment of a
/// reparent, or `None` when it already reads right or an expression drives
/// it. `gain` is what a keyed property's other keys are scaled by; `None` is
/// a scale, which goes by its own ratio.
fn made_up(
    property: &lumit_core::anim::Property,
    from: f64,
    to: f64,
    turns: bool,
    gain: Option<f64>,
) -> Option<lumit_core::anim::Animation> {
    use lumit_core::anim::Animation;

    let mut delta = to - from;
    if turns {
        // The nearest way round, so a layer three turns in stays there.
        delta = (delta + 180.0).rem_euclid(360.0) - 180.0;
    }
    // The matrices are f32. Past a thousandth it is their noise.
    let delta = (delta * 1e3).round() / 1e3;
    if delta == 0.0 || !delta.is_finite() {
        return None;
    }
    let gain = gain.unwrap_or((from + delta) / from);
    match &property.animation {
        Animation::Static(v) => Some(Animation::Static(v + delta)),
        keyed => lumit_core::ops::remapped(keyed, gain, from + delta - gain * from),
    }
}

/// Where the composition's camera puts a particle that sits off `layer`'s own
/// plane — the points stream's third axis, made visible.
///
/// # In plain terms
///
/// An effect draws into the layer's own rectangle of pixels, and the compositor
/// then turns that rectangle in space and photographs it with the comp's
/// camera. A particle a hundred pixels in front of the rectangle has to be
/// drawn not where its x and y say but where the camera would *see* it — which
/// is somewhere else on the rectangle, and smaller or larger for being further
/// off or nearer.
///
/// This works that "somewhere else" out once a frame, as one 3×4 table.
/// **Nothing here derives a camera**: it is the compositor's own
/// [`lumit_gpu::composite::camera_matrix`] and [`lumit_gpu::composite::place_matrix`], multiplied
/// together and then restricted back onto the layer's plane — the plane's own
/// map `H` (the full product with `z` struck out) inverted and put in front, so
/// that a particle at `z = 0` comes back to exactly where it started and one
/// off the plane comes back to where it is seen.
///
/// `None` — a 2D layer, or one seen edge on — is not "the identity
/// matrix": the render paths branch on it and leave the positions' bits alone,
/// which is what makes an old project's picture bit-identical rather than
/// nearly so.
pub fn points_projection(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    lt: f64,
    context: Arc<ExpressionContext>,
) -> Option<lumit_core::fx::points::Projection> {
    if !layer.switches.three_d {
        return None;
    }
    let pose = crate::track::camera_pose(doc, comp, t_comp);
    let cam = crate::export::camera_mat(comp.width, comp.height, pose);
    let d = drawn(layer, lt, &context);
    let place = lumit_gpu::composite::place_matrix(
        d.position,
        d.anchor,
        d.scale,
        d.rotation_deg,
        d.z,
        d.rotation_x_deg,
        d.rotation_y_deg,
    );
    // Parenting composes exactly as the compositor composes it: `pre × own`.
    let place = match parent_world_placement(comp, layer, t_comp, context) {
        Some(pre) => lumit_gpu::composite::concat_place(pre, place),
        None => place,
    };
    let a = (cam * glam::Mat4::from_cols_array_2d(&place)).to_cols_array_2d();
    // Rows 0, 1 and 3 of the product — x, y and the homogeneous divide; the z
    // output is flattened by the perspective matrix and never read.
    let row = |r: usize| [a[0][r], a[1][r], a[2][r], a[3][r]];
    let (rx, ry, rw) = (row(0), row(1), row(3));
    // `H` is the same three rows with the z column struck out: what the plane
    // itself maps to. Inverting it is what brings the answer back into the
    // layer's own pixels.
    let h = [
        [rx[0], rx[1], rx[3]],
        [ry[0], ry[1], ry[3]],
        [rw[0], rw[1], rw[3]],
    ];
    let inv = invert3(&h)?;
    let mut m = [[0.0f32; 4]; 3];
    for (r, mr) in m.iter_mut().enumerate() {
        for (c, out) in mr.iter_mut().enumerate() {
            *out = inv[r][0] * rx[c] + inv[r][1] * ry[c] + inv[r][2] * rw[c];
        }
    }
    Some(lumit_core::fx::points::Projection { m })
}

/// The empty room a points effect is given round a Shape or Text layer's art:
/// texels on each side, across and down, and the raster's px per comp px.
/// `None` where the art already fills the frame.
///
/// The room reaches the frame's edges as the layer's position and anchor put
/// them, so the effect draws into the frame as it does on a solid that fills
/// it. `corner` is where the art's own corner sits in the frame. Scale and
/// rotation then turn the wider picture as they would turn that solid.
// ponytail: a grown picture is placed by its centre, so both sides get the
// larger margin and art at one edge of the frame makes a picture near twice
// the frame's width. A parent's move is not read either. Grow one side at a
// time, and follow the parent, if a profile or a project shows it.
fn points_room(
    frame: (f32, f32),
    natural: (f32, f32),
    tex: (u32, u32),
    corner: (f32, f32),
) -> Option<([u32; 2], f32)> {
    let scale = tex.0 as f32 / natural.0.max(1.0);
    let side = |corner: f32, art: f32, frame: f32, tex: u32| {
        // To the further of the frame's two edges, and never past a frame's
        // width, however far off the layer has been moved.
        let px = corner.max(frame - corner - art).clamp(0.0, frame);
        let most = crate::plan::MAX_TEXTURE_SIDE.saturating_sub(tex) / 2;
        ((px * scale).ceil() as u32).min(most)
    };
    let room = [
        side(corner.0, natural.0, frame.0, tex.0),
        side(corner.1, natural.1, frame.1, tex.1),
    ];
    (room != [0, 0]).then_some((room, scale))
}

/// **The stream a points consumer's wire brings it** (points-stream.md §3.3),
/// in px@comp, and which effect it came from.
///
/// # In plain terms
///
/// Clone to points needs particles, and the wire says whose. This walks the
/// layer's graph for the edge feeding the effect's Points socket, finds the
/// producer it names, and asks the engine for that producer's stream — the very
/// function the Points sample driver asks, so a consumer stamps the particles a
/// driver would have counted and the producer did draw.
///
/// **The back-samples**: a consumer that declares Trail's two rows —
/// `back_samples` and `back_step` — is asking for the same producer at
/// `t − k·step` as well, newest first. Named by their parameter ids rather than
/// by an effect's name, the way a producer's birth scan is found by its
/// `emit_rate` row, so a second consumer that wants a history declares
/// the same two ids and needs no edit here.
///
/// `(vec![], None)` — the documented calm — for every producer and for a
/// consumer with nothing wired. The effect renders as a passthrough and the box
/// wears the "no stream" mark; nothing faults, and nothing is guessed.
///
/// An empty list with the producer still named is a stream that cannot be made
/// yet, because its points depend on a picture (Scatter). The walk asks for it
/// on the card.
/// SICELIOT: the rows of a points consumer that wired drivers give each point
/// its own value for, and the stream columns they are written into.
fn driven_rows(
    consumer: &lumit_core::model::EffectInstance,
) -> Option<&'static [(&'static str, &'static str)]> {
    match consumer.effect.match_name.as_str() {
        "clone_to_points" => Some(lumit_core::fx::effects::clone_to_points::COPY_ROWS),
        "label_points" => Some(lumit_core::fx::effects::label_points::LABEL_DRIVEN_ROWS),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn points_input_for(
    layer: &lumit_core::model::Layer,
    consumer: &lumit_core::model::EffectInstance,
    def: &'static dyn lumit_core::fx::registry::EffectDef,
    t: f64,
    dt: f64,
    projection: lumit_core::fx::points::Projection,
    context: &Arc<ExpressionContext>,
    audio: &dyn lumit_core::fx::registry::AudioTap,
    clone: (u32, u32),
) -> PointsInput {
    let wiring = (&layer.graph, &layer.effects[..], None);
    points_input_in(
        Some(layer),
        wiring,
        consumer,
        def,
        t,
        dt,
        projection,
        context,
        audio,
        clone,
    )
}

/// [`points_input_for`] over any wiring: a layer's graph and stack, or a node
/// graph's own wires and boxes, which the driver walk then finds the producer
/// among. `layer` is the layer the consumer is on, which a node graph has none of.
#[allow(clippy::too_many_arguments)]
fn points_input_in(
    layer: Option<&lumit_core::model::Layer>,
    (graph, effects, stack): (
        &lumit_core::graph::LayerGraph,
        &[lumit_core::model::EffectInstance],
        Option<&[lumit_core::model::EffectInstance]>,
    ),
    consumer: &lumit_core::model::EffectInstance,
    def: &'static dyn lumit_core::fx::registry::EffectDef,
    t: f64,
    _dt: f64,
    projection: lumit_core::fx::points::Projection,
    context: &Arc<ExpressionContext>,
    audio: &dyn lumit_core::fx::registry::AudioTap,
    clone: (u32, u32),
) -> PointsInput {
    use lumit_core::graph::{InputRef, NodeRef, OutputRef};
    let mut found = PointsInput::default();
    // A Bake points reading its bake is handed that in place of its wire, so
    // nothing above it has to make the stream again.
    if let Some(layer) = layer
        .filter(|_| consumer.effect.match_name == lumit_core::fx::effects::bake_points::MATCH_NAME)
    {
        let read = context
            .comp
            .and_then(|id| context.document.comp(id))
            .and_then(|comp| {
                lumit_core::fx::effects::bake_points::reading(
                    &context.document,
                    comp,
                    layer,
                    consumer,
                    t,
                )
                .stream(projection)
            });
        if let Some(stream) = read {
            found.streams.push(stream);
            found.from.push(None);
            return found;
        }
    }
    // Which sockets, from the signature rather than from a name this file
    // would have to keep in step with the effect's own declaration.
    let ports: Vec<&lumit_core::fx::schema::Port> = def
        .signature()
        .inputs()
        .iter()
        .filter(|p| p.ty == lumit_core::fx::schema::PortType::Points)
        .collect();
    // Two things a points wire may come out of, and nothing else: a producer in
    // this stack, or a **cross-layer tap** naming another layer's. A
    // number or the source matte is neither; the type check refused those at
    // commit, so they are stale lines to ignore rather than cases to guess.
    enum Source {
        /// A producer in this layer's own stack, by id.
        Effect(uuid::Uuid),
        /// A Layer points node in this layer's graph, by node id.
        Tap(uuid::Uuid),
    }
    for port in &ports {
        let wire = graph.wire_into(&InputRef::Param {
            node: NodeRef::Effect(consumer.id),
            port: port.id.to_owned(),
        });
        let source = match wire {
            Some(OutputRef::EffectData { effect, .. }) => Some(Source::Effect(*effect)),
            Some(OutputRef::Driver { node, .. }) => Some(Source::Tap(*node)),
            _ => None,
        };
        // **Which producer the wire names, for the frame key**: the stack
        // index of a same-layer producer, and `None` for a tap — whose own
        // node, and with it the layer it names and that layer's whole stack,
        // is already folded into this layer's key by the graph's driver-node
        // hashing (node-graph.md §2.3).
        let from = match &source {
            Some(Source::Effect(effect)) => effects
                .iter()
                .position(|e| e.id == *effect)
                .and_then(|i| u32::try_from(i).ok()),
            _ => None,
        };
        let at = |when: f64| match &source {
            // The clone number goes with it, so a producer driven by a
            // Clone index hands on the points it draws.
            Some(Source::Effect(effect)) => lumit_core::fx::drivers::effect_stream_in(
                graph,
                *effect,
                when,
                context.clone(),
                Some(audio),
                projection,
                stack,
                clone,
            ),
            Some(Source::Tap(node)) => lumit_core::fx::drivers::driver_stream_in(
                graph,
                *node,
                when,
                context.clone(),
                Some(audio),
                projection,
                stack,
                clone,
            ),
            None => None,
        };
        found.from.push(from);
        let now = at(t);
        // Several sockets: one stream each, empty where there is none, so the
        // list stays in step with the sockets.
        if ports.len() > 1 {
            found.streams.push(now.unwrap_or_default());
            continue;
        }
        // No stream yet, but the wire may still name its producer: one whose
        // points depend on a picture is asked again on the card.
        let Some(mut stream) = now else {
            break;
        };
        // SICELIOT: drivers wired into Clone to points' Scale, Rotation or
        // Opacity give each stamp its own value, and into Label points'
        // Change or Label scale each label its own.
        if let Some(rows) = driven_rows(consumer) {
            lumit_core::fx::effects::clone_to_points::drive_rows(
                graph,
                consumer.id,
                &mut stream,
                rows,
                t,
                context,
                Some(audio),
                projection,
            );
        }
        found.streams.push(stream);

        // The history, if this consumer asks for one. `Samples` counts the
        // present moment as the first of them, so the walk starts at 1.
        use lumit_core::fx::effects::trail::Trail;
        let declares = |id: &str| def.schema().params.iter().any(|p| p.id == id);
        if declares(Trail::SAMPLES_PARAM) && declares(Trail::STEP_PARAM) {
            let read = |id: &str, fallback: f64| {
                consumer
                    .float_at_with_context(id, t, context.clone())
                    .unwrap_or(fallback)
            };
            // Clamped to the row's own hard ceiling here as well as at the
            // socket: every sample is another whole evaluation of the
            // producer, so this is the one number that decides what this op
            // costs.
            let samples = read(Trail::SAMPLES_PARAM, 1.0).clamp(1.0, 256.0) as usize;
            let step = read(Trail::STEP_PARAM, 0.0).max(0.0);
            // ponytail: one host evaluation per sample. A GPU carriage
            // (points-stream.md §3.3) would make this one dispatch per sample
            // instead; the trigger is a profile showing a real comp spending it.
            for k in 1..samples {
                // Before the layer's own clock started there is nothing to ask
                // for, and a tail that stops there is the honest picture.
                let when = t - k as f64 * step;
                match (when >= 0.0).then(|| at(when)).flatten() {
                    Some(past) => found.streams.push(past),
                    None => break,
                }
            }
        }
    }
    found
}

/// The births a producer with an Emit rate row can still see at layer time
/// `t`, scanned from the layer's in point. Trigger follows its wire in
/// `graph` over the frames its bursts look back on, as the driver walk reads it.
fn birth_schedule(
    e: &lumit_core::model::EffectInstance,
    t: f64,
    dt: f64,
    context: &Arc<ExpressionContext>,
    graph: Option<&lumit_core::graph::LayerGraph>,
    audio: &dyn lumit_core::fx::registry::AudioTap,
) -> lumit_core::fx::points::Schedule {
    let upto = (t / dt).floor() as i64;
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
    let mut schedule = lumit_core::fx::points::Schedule::scan(dt, upto, window, &rate_at);
    // Bursts read their rows over the window, and Trigger
    // follows its wire there: the driver walk reads the same.
    let row_at = |id: &str, lt: f64| -> f64 {
        let wired = graph.filter(|_| id == "trigger").and_then(|graph| {
            lumit_core::fx::drivers::driven_param(graph, e.id, id, lt, context.clone(), Some(audio))
        });
        wired.unwrap_or_else(|| {
            e.float_at_with_context(id, lt, context.clone())
                .unwrap_or(0.0)
        })
    };
    lumit_core::fx::effects::particulate::Particulate::add_bursts(e, &mut schedule, &row_at);
    schedule.trim_to_newest(lumit_gpu::fx::particulate::MAX_CANDIDATES);
    schedule
}

/// What Text to points and Label points are handed beside the op: the letters
/// of the Text layer their row names as that layer draws them at this frame,
/// or the layer's document with its words as they read now. Nothing for any
/// other effect, and nothing when the layer named is not text.
fn text_carriage(
    comp: &lumit_core::model::Composition,
    owner: &lumit_core::model::Layer,
    e: &lumit_core::model::EffectInstance,
    t_comp: f64,
    doc: &Arc<lumit_core::model::Document>,
) -> (
    Vec<lumit_core::fx::points::TextGlyph>,
    Option<Box<lumit_core::model::TextDocument>>,
) {
    let name = e.effect.match_name.as_str();
    if name != "text_to_points" && name != "label_points" && name != "siceliot_type_fill" {
        return Default::default();
    }
    // An unset row means the layer the effect is on.
    let layer = match e.layer_ref("text_layer") {
        Some(id) => comp.layers.iter().find(|l| l.id == id),
        None => Some(owner),
    };
    let Some((layer, document)) = layer.and_then(|l| match &l.kind {
        lumit_core::model::LayerKind::Text { document } => Some((l, document)),
        _ => None,
    }) else {
        return Default::default();
    };
    // The same clock and the same words the layer's own picture is drawn at.
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let line = document.resolved_text(Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: t_comp,
        current_depth: 0,
        inputs: None,
    }));
    if name == "label_points" || name == "siceliot_type_fill" {
        // A label is set straight and still, so the path and the animators
        // are left behind.
        let label = lumit_core::model::TextDocument {
            text: line.into_owned(),
            expression: None,
            path: None,
            animators: Vec::new(),
            ..document.clone()
        };
        return (Vec::new(), Some(Box::new(label)));
    }

    let xforms = lumit_core::text::glyph_xforms(&document.animators, &line, lt);
    let spine = document
        .path
        .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
        .filter(|p| !p.is_empty());
    // Animated letters on a straight line sit in from the layer's edge. On a
    // path they do not.
    let laid = lumit_text::layout(
        &lumit_text::TextBlock::of(document, &line),
        spine.is_none() && !xforms.is_empty(),
    );
    let chars: Vec<char> = line.chars().collect();
    let (words, _) = lumit_core::text::unit_indices(&line, lumit_core::text::SelectorBasis::Words);
    let half = (laid.ascent - laid.descent) * 0.5;
    // How far along the path this line starts. Lines run on one after another.
    let mut along = document.path_offset.value_at(lt) as f32;
    let mut glyphs = Vec::new();
    // Include spaces hands a space over as a letter, which only a point per
    // character can use: in a word or a line it would shift the middle.
    let spaces = e.bool_of("include_spaces").unwrap_or(false)
        && !matches!(
            e.param("per"),
            Some(lumit_core::model::EffectValue::Choice(1 | 2))
        );
    for (line_no, row) in laid.lines.iter().enumerate() {
        let first = row.carets.first().copied().unwrap_or(0.0);
        for (k, pair) in row.carets.windows(2).enumerate() {
            let ch = row.start + k;
            let ink = chars
                .get(ch)
                .is_some_and(|c| !c.is_whitespace() || (spaces && !matches!(c, '\n' | '\r')));
            if !ink || glyphs.len() >= lumit_core::fx::points::CAP_HARD as usize {
                continue;
            }
            // Where the pen is and which way the baseline runs there.
            let (pen, tan) = match &spine {
                None => ([pair[0], row.baseline], [1.0, 0.0]),
                Some(path) => {
                    let total = path.length();
                    let mut s = along + pair[0] - first;
                    if path.closed && total > 0.0 {
                        s = s.rem_euclid(total);
                    } else if !(0.0..=total).contains(&s) {
                        // Off the end of an open path, where no letter is drawn.
                        continue;
                    }
                    (path.point_at(s), path.tangent_at(s))
                }
            };
            let nrm = [-tan[1], tan[0]];
            // An animator pushes a letter along and away from its baseline.
            let push = xforms.get(ch).map_or([0.0; 2], |x| x.position);
            let origin = [
                pen[0] + push[0] * tan[0] + push[1] * nrm[0],
                pen[1] + push[0] * tan[1] + push[1] * nrm[1],
            ];
            let advance = pair[1] - pair[0];
            glyphs.push(lumit_core::fx::points::TextGlyph {
                origin,
                end: [origin[0] + advance * tan[0], origin[1] + advance * tan[1]],
                up: [-nrm[0] * half, -nrm[1] * half],
                word: words.get(ch).copied().unwrap_or(0) as u32,
                line: line_no as u32,
            });
        }
        along += row.carets.last().copied().unwrap_or(first) - first;
    }
    (glyphs, None)
}

/// What the wires into an effect's Points inputs bring it.
#[derive(Default)]
struct PointsInput {
    /// One stream and its history for an effect with one input, or one stream
    /// per input for an effect with several. Empty where nothing could be
    /// made yet.
    streams: Vec<lumit_core::fx::points::PointsStream>,
    /// Which effect in the stack each input is wired from, by index.
    from: Vec<Option<u32>>,
}

/// The stream wired into `effect` on `layer` at composition time `t_comp`,
/// made the way a frame's builder makes it and without drawing anything.
/// `None` where it can only be made during the render, because its points
/// depend on a picture. What the Bake points job asks first.
#[must_use]
pub fn points_input_at(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    effect: &lumit_core::model::EffectInstance,
    t_comp: f64,
) -> Option<lumit_core::fx::points::PointsStream> {
    let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&effect.effect.match_name)?;
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let context = Arc::new(ExpressionContext {
        document: doc.clone(),
        comp: Some(comp.id),
        layer: Some(layer.id),
        comp_time: t_comp,
        current_depth: 0,
        inputs: None,
    });
    let projection = points_projection(doc, comp, layer, t_comp, lt, context.clone());
    let audio = crate::audio_tap::DocumentAudio::new(doc, comp, t_comp);
    let found = points_input_for(
        layer,
        effect,
        def,
        lt,
        1.0 / comp.frame_rate.fps().max(1.0),
        projection.unwrap_or(lumit_core::fx::points::Projection::FLAT),
        &context,
        &audio,
        lumit_core::fx::drivers::NO_CLONE,
    );
    // A wire that names an effect in this stack and brought nothing is a
    // stream made on the card. Anything else that brought nothing is empty.
    let late = found.from.first().is_some_and(Option::is_some);
    match found.streams.into_iter().next() {
        Some(stream) => Some(stream),
        None if late => None,
        None => Some(Default::default()),
    }
}

/// A 3×3 inverse, or `None` when the matrix is singular — a layer scaled to
/// nothing, or turned exactly edge-on. An engine crate answers the degenerate
/// case rather than dividing by nought (14-ENGINEERING-RULES §4), and `None`
/// here reads as the flat projection, which draws the particles unprojected on
/// a layer that is drawing no pixels anyway.
fn invert3(m: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let c = |r: usize, k: usize| -> f32 {
        let (r0, r1) = ((r + 1) % 3, (r + 2) % 3);
        let (k0, k1) = ((k + 1) % 3, (k + 2) % 3);
        m[r0][k0] * m[r1][k1] - m[r0][k1] * m[r1][k0]
    };
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    // The adjugate is the cofactor matrix transposed.
    let mut out = [[0.0f32; 3]; 3];
    for (r, row) in out.iter_mut().enumerate() {
        for (k, v) in row.iter_mut().enumerate() {
            *v = c(k, r) * inv_det;
        }
    }
    Some(out)
}

/// The per-layer motion-blur sub-frame placements for `layer` at comp time
/// `t_comp` (docs/06 §4): the layer's own transform re-evaluated at each
/// shutter sample time. Empty — so the layer draws normally — unless the comp
/// master (`comp.motion_blur.enabled`) and the layer's own switch are both on
/// and `samples` ≥ 2.
///
/// Each sample's comp time is `t_comp + offset · dt` (dt = one frame in comp
/// seconds; offsets from [`MotionBlur::sample_offsets`], centred on the frame),
/// and its layer time subtracts the layer's `start_offset`. Shared by every
/// caller of the one comp walk so all paths smear identically. Parent motion
/// within the shutter is a follow-up: only the layer's OWN transform is sampled
/// here — a parented layer keeps its frame-time parent placement (`pre`) for
/// every sub-copy.
pub fn motion_blur_samples(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
    context: Arc<ExpressionContext>,
) -> Vec<lumit_gpu::composite::MbSample> {
    if !layer.switches.motion_blur {
        return Vec::new();
    }
    let offsets = comp.motion_blur.sample_offsets();
    if offsets.is_empty() {
        return Vec::new();
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);

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
            drawn(layer, lt, &context)
        })
        .collect()
}

/// The comp's Light layers, reduced to what the lighting pass needs (docs/06):
/// each light's emitting rectangle as four corners in comp pixels,
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
    context: &Arc<ExpressionContext>,
) -> Vec<lumit_gpu::fx::lighting::LightingLight> {
    use lumit_core::model::{LayerKind, LightKind};

    // Accepts lights off still takes the shadows of the lights that cast
    // them, as in After Effects. And After Effects never lights a 2D layer.
    let unlit = !layer.switches.accepts_lights;
    let takes_shadows = layer.switches.three_d
        && layer.transform.material.accepts_shadows != lumit_core::model::ShadowMode::Off
        && comp.layers.iter().any(|l| {
            l.switches.three_d
                && l.transform.material.casts_shadows != lumit_core::model::ShadowMode::Off
        });
    if (unlit && !takes_shadows)
        || matches!(
            layer.kind,
            LayerKind::Light { .. } | LayerKind::Camera { .. }
        )
        || (comp.lighting != lumit_core::model::CompLighting::Adds && !layer.switches.three_d)
    {
        return Vec::new();
    }
    let encoded = comp.lighting == lumit_core::model::CompLighting::AfterEffects { linear: false };
    let read = lumit_core::model::context_reader(context, t_comp);
    let mut lights = comp.lights_at(t_comp, &read, crate::track::solves_for(&context.document));
    if unlit {
        lights.retain(|l| l.shadow_darkness > 0.0);
    }
    if lights.is_empty() {
        return Vec::new();
    }
    if lights.len() > lumit_gpu::fx::lighting::MAX_LIT_LIGHTS {
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
        keyed.truncate(lumit_gpu::fx::lighting::MAX_LIT_LIGHTS);
        keyed.sort_by_key(|&(_, i)| i);
        lights = keyed.iter().map(|&(_, i)| lights[i]).collect();
    }

    lights
        .iter()
        .map(|l| {
            // The same placement maths a layer gets, so a light and a layer
            // that share a transform occupy the same place in space.
            let place = lumit_gpu::composite::place_matrix(
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
            // A spot is aimed along its own local +z, away from the camera;
            // anything below -1 tells the kernel "not a spot".
            let cone_cos = match l.kind {
                LightKind::Spot => (l.cone_deg as f32).to_radians().cos(),
                _ => -2.0,
            };
            use lumit_gpu::fx::lighting::LightingKind as K;
            lumit_gpu::fx::lighting::LightingLight {
                corners: [at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)],
                // A picture lit in display encoding takes the light's colour
                // encoded too, and its intensity as it is, as After Effects does.
                colour: if encoded {
                    l.tint.map(|c| lumit_core::lighting::encode(c) * l.gain)
                } else {
                    l.colour
                },
                kind: match l.kind {
                    LightKind::Area if hw > 0.0 && hh > 0.0 => K::Area,
                    LightKind::Parallel => K::Parallel,
                    LightKind::Ambient => K::Ambient,
                    _ => K::Point,
                },
                falloff: l.falloff as u32,
                radius_px: l.radius_px as f32,
                falloff_px: l.falloff_px as f32,
                legacy: l.legacy_shape,
                cone_cos,
                // The feather is a share of the cone's angle, as After Effects
                // has it. An old light's is a share of its cosine, which at a
                // tenth is the edge spots had before it was a row.
                cone_inner: if l.legacy_shape {
                    cone_cos + (1.0 - cone_cos) * (l.cone_feather / 100.0) as f32
                } else {
                    (l.cone_deg * (1.0 - l.cone_feather / 100.0))
                        .to_radians()
                        .cos() as f32
                },
                axis: unit(place_dir(&place, 0.0, 0.0, 1.0)),
                shadow_darkness: l.shadow_darkness,
                shadow_diffusion: l.shadow_diffusion,
            }
        })
        .collect()
}

/// A layer's Material options at comp time `t_comp`, as what the lighting
/// pass reads. The eye is the realiser's to fill in. A 2D layer has no
/// Material options to set, so it takes the lights as they always were, with
/// no highlight.
pub fn shading_material(
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    t_comp: f64,
) -> lumit_gpu::fx::lighting::LightingMaterial {
    if !layer.switches.three_d {
        return lumit_gpu::fx::lighting::LightingMaterial::default();
    }
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let m = &layer.transform.material;
    let pct = |p: &lumit_core::anim::Property| (p.value_at(lt) / 100.0).max(0.0) as f32;
    // Where light adds to the picture, Diffuse at a half is the light as it
    // always was; After Effects lights from black at the Diffuse given.
    let adds = comp.lighting == lumit_core::model::CompLighting::Adds;
    let encoded = comp.lighting == lumit_core::model::CompLighting::AfterEffects { linear: false };
    let shadows = |s: lumit_core::model::ShadowMode| match s {
        lumit_core::model::ShadowMode::Off => lumit_gpu::fx::lighting::LightingShadows::Off,
        lumit_core::model::ShadowMode::On => lumit_gpu::fx::lighting::LightingShadows::On,
        lumit_core::model::ShadowMode::Only => lumit_gpu::fx::lighting::LightingShadows::Only,
    };
    lumit_gpu::fx::lighting::LightingMaterial {
        base: if adds { 1.0 } else { 0.0 },
        ambient: pct(&m.ambient),
        diffuse: if adds { 2.0 } else { 1.0 } * pct(&m.diffuse),
        specular: pct(&m.specular),
        exponent: lumit_core::lighting::ShadingMaterial::exponent_for(m.shininess.value_at(lt)),
        metal: pct(&m.metal).min(1.0),
        eye: [0.0; 3],
        encoded,
        accepts_shadows: shadows(m.accepts_shadows),
        casts_shadows: shadows(m.casts_shadows),
        transmission: pct(&m.light_transmission).min(1.0),
        unlit: !layer.switches.accepts_lights,
    }
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

/// Resolve a layer's **effect stack and then its styles**, as one op list
/// (docs/impl/layer-styles.md §3).
///
/// **In plain terms.** A layer's styles are a second little stack that runs on
/// the layer's own picture after its effects and before the transform
/// photographs it. This resolves both at the same layer time and hands back one
/// list of ops with one list of instance ids beside it, so nothing downstream —
/// `run_ops`, the per-effect cache, the profiler's per-row milliseconds — learns
/// that styles exist at all.
///
/// **The order the ops run is not the order the list is stored in**, and the
/// difference is deliberate. §2's stored order is a *painting* order, back to
/// front, which is what the panel and the Timeline show. But these ops run one
/// after another on a single raster, and an interior style (Colour overlay)
/// floods whatever alpha it finds: run a Drop shadow first and the overlay would
/// paint the shadow too. So the interiors go first, on the layer's own alpha,
/// and the outer styles — which add pixels *underneath* — come after, in reverse
/// order so that the last one composited ends up furthest back. Because the
/// stored list is sorted, the outers are exactly its leading pair, and the split
/// costs a `take_while` rather than an allocation.
//
// ponytail: each outer style reads the raster's alpha at its point in the
// chain, so a second outer style casts its shadow from the first one's soft
// edge as well as from the layer. Ceiling: visible only with Drop shadow AND
// Outer glow both live, which is package 2's pair — one of them ships in this
// package. Upgrade: hold the pre-outer alpha in a texture and hand it to the
// outer kernels. Trigger: the §2 order pixel test for shadow-under-glow.
#[allow(clippy::too_many_arguments)]
fn resolve_layer_fx(
    layer: &lumit_core::model::Layer,
    drivers: &lumit_core::fx::drivers::ResolvedDrivers,
    effect_lt: f64,
    frame_lt: f64,
    diag_px: f32,
    px_scale: f32,
    markers: &lumit_core::fx::markers::MarkerContext,
    context: Arc<ExpressionContext>,
) -> (Vec<Uuid>, lumit_core::fx::params::ResolvedStack) {
    let (mut ids, mut ops) = lumit_core::fx::resolved::resolve_stack_temporal_named(
        &layer.effects,
        drivers,
        effect_lt,
        frame_lt,
        diag_px,
        px_scale,
        markers,
        context.clone(),
    );
    if layer.styles.is_empty() {
        return (ids, ops);
    }
    let outers = lumit_core::fx::styles::outer_prefix(&layer.styles);
    let (outer, interior) = layer.styles.split_at(outers);
    // The interiors as one walk, then the outers one at a time and in reverse:
    // each composites underneath what is already there, so the one run last is
    // the one that ends up furthest back, and §2 puts the Drop shadow there.
    let walks = std::iter::once(interior).chain((0..outer.len()).rev().map(|i| &outer[i..=i]));
    for slice in walks {
        let (style_ids, style_ops) = lumit_core::fx::resolved::resolve_stack_temporal_named(
            slice,
            drivers,
            effect_lt,
            frame_lt,
            diag_px,
            px_scale,
            markers,
            context.clone(),
        );
        ids.extend(style_ids);
        ops.append(style_ops);
    }
    (ids, ops)
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
/// `keys` names each non-collapsed Precomp's frame so the realiser can
/// serve it from the nested-frame store; `None` leaves every nested draw
/// unnamed, which realises it every time.
///
/// `spliced` says this comp is being spliced into its parent by a collapsed
/// Precomp layer (docs/06 §1.4): its layers are not clipped to its own
/// rectangle, so a layer that covers *this* comp's frame proves nothing about
/// the parent's, and the occlusion cull is switched off.
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
    comp_walk(
        doc,
        comp,
        t_comp,
        frame_t,
        pixels_by_layer,
        visited,
        keys,
        spliced,
        Walk::PLAIN,
        None,
    )
}

/// What a walk carries beyond the frame, handed on to every comp it walks
/// into.
#[derive(Clone, Copy)]
struct Walk<'a> {
    /// Which copy of how many this walk's layers are rendered as, for their
    /// Clone index drivers. It goes down into every precomp and node graph
    /// the walk reaches, so a Clone index anywhere in a clone picture reads
    /// that picture's number.
    clone: (u32, u32),
    /// A placed node graph's own Input values, when the comp being walked is
    /// that graph (docs/impl/node-graph-comp.md §5.3). Empty on every other
    /// walk, which is a graph on its own defaults.
    values: &'a [(String, lumit_core::model::EffectValue)],
    /// This walk is a comp built again at another moment, for a temporal
    /// effect further up to read as its neighbour. A neighbour carries no
    /// neighbours of its own, so nothing in it builds any: otherwise an Echo
    /// over a Precomp that holds an Echo builds the whole of the inside once
    /// for every frame of both windows, only to throw it away.
    rebuild: bool,
    /// The camera a collapsed precomp's layers are seen through, the one of
    /// the comp it was collapsed into. `None` is this comp's own.
    seen_through: Option<lumit_core::model::CameraPose>,
}

impl Walk<'static> {
    /// An ordinary walk: the first copy of one, and no values handed in.
    const PLAIN: Walk<'static> = Walk {
        clone: lumit_core::fx::drivers::NO_CLONE,
        values: &[],
        rebuild: false,
        seen_through: None,
    };
}

/// Something asked of a walk on its own, in place of its draws. Both are
/// Clone to points': a picture at another moment or as one copy of several,
/// and the whole list for a stream that was only made during the render.
enum Probe<'a> {
    /// One layer row of an effect, as a picture.
    Picture {
        effect: &'a lumit_core::model::EffectInstance,
        row: &'a str,
        found: Option<DofInputDraw>,
    },
    /// Every picture of the Clone to points `effect` on layer `owner`, now
    /// that its stream is known to hold `points` points.
    Clones {
        owner: uuid::Uuid,
        effect: uuid::Uuid,
        points: usize,
        found: Vec<LayerInputDraw>,
    },
}

/// Clone to points' pictures for a stream that was only made during the
/// render. The realiser asks once the stream exists, since how many copies
/// there are, and so what each one's Clone index reads, is not known before.
///
/// `pixels_by_layer` is the frame's own decoded pixels, as the first walk had
/// them.
#[must_use]
pub fn late_clone_pictures(
    doc: &Arc<lumit_core::model::Document>,
    spec: &crate::draw::LateClones,
    points: usize,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
) -> Vec<LayerInputDraw> {
    let Some(comp) = doc.comp(spec.comp) else {
        return Vec::new();
    };
    let mut probe = Probe::Clones {
        owner: spec.owner,
        effect: spec.effect,
        points,
        found: Vec::new(),
    };
    comp_walk(
        doc,
        comp,
        spec.t,
        spec.frame_t,
        pixels_by_layer,
        &mut spec.visited.clone(),
        None,
        true,
        Walk {
            clone: spec.clone,
            values: &[],
            rebuild: false,
            seen_through: None,
        },
        Some(&mut probe),
    );
    match probe {
        Probe::Clones { found, .. } => found,
        Probe::Picture { .. } => Vec::new(),
    }
}

/// The walk behind [`build_comp_draws_at`]. Handed a `probe`, it makes what
/// the probe asks for at `t_comp` and no draws at all, so a layer at another
/// moment is built by the very code that builds it at this one.
#[allow(clippy::too_many_arguments)]
fn comp_walk(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
    keys: Option<&dyn crate::cache::NestedKeyer>,
    spliced: bool,
    with: Walk<'_>,
    probe: Option<&mut Probe<'_>>,
) -> Vec<CompLayerDraw> {
    use lumit_core::model::LayerKind;
    // Measure and `sourceRectAtTime` read a Text layer's words through this,
    // and every render comes through here before a driver is asked.
    lumit_core::text::install_words_box(lumit_text::words_box);
    // The frame's own pixels, kept for a probe at another moment to find its
    // clip in.
    let frame_pixels = pixels_by_layer;
    // The copy this walk renders its layers as, and what it hands the comps
    // it walks into.
    let clone_number = with.clone;
    let seen_through = with
        .seen_through
        .filter(|_| spliced)
        .unwrap_or_else(|| crate::track::camera_pose(doc, comp, t_comp));
    let inner = Walk {
        clone: clone_number,
        values: &[],
        rebuild: with.rebuild,
        seen_through: Some(seen_through),
    };
    // A nested frame's name says nothing of which copy it is, so a numbered
    // copy is never named and never served from the nested store.
    let keys = keys.filter(|_| clone_number == lumit_core::fx::drivers::NO_CLONE);
    // A rebuild at another moment names one thing only: a Precomp layer's
    // frame that comes out as that frame itself would ([`rebuilt_as_itself`]).
    // Nothing else in it is named, which is how every rebuild used to be.
    let rebuild_keys = keys;
    let keys = keys.filter(|_| !with.rebuild);
    // A rebuild at another moment draws each clip as it is at that moment,
    // where the planner fetched it.
    let at_moment = pixels_at_moment(doc, comp, t_comp, frame_t, pixels_by_layer);
    let pixels_by_layer = at_moment.as_ref().unwrap_or(pixels_by_layer);
    // A Sequence layer in a gap now draws nothing, and may still be in the
    // map for a clip it shows a frame away. `fetched` keeps it, for the
    // neighbours a temporal effect on an adjustment layer builds.
    let fetched = pixels_by_layer;
    let shown = without_gaps(comp, fetched);
    let pixels_by_layer = shown.as_ref().unwrap_or(fetched);
    let in_span = |l: &lumit_core::model::Layer| {
        t_comp >= l.in_point.0.to_f64() && t_comp < l.out_point.0.to_f64()
    };
    // Occlusion cull (docs/06 §1.1): the same question the decode
    // planner asks, so a layer skipped here was never decoded either.
    let occluder = (!spliced)
        .then(|| lumit_core::occlusion::occluder_index(doc, comp, t_comp))
        .flatten();

    // The caller's Arc: an expression context takes an owned handle, and this
    // shares one rather than deep-cloning the project per build (and per
    // nesting level — this function recurses through Precomps).
    let expr_doc = doc;

    // Where the Audio level driver's samples come from. Made here,
    // from the document this walk already holds, so the preview and the export
    // — which both build their draws through this function — hand the driver
    // the same sound and reach the same number.
    let audio = crate::audio_tap::DocumentAudio::new(doc, comp, t_comp);

    // A layer's driver graph, resolved with the camera its own particles would
    // be drawn through. One closure for all four resolve sites: the
    // Points sample driver measures a distance **on the frame**, so a stream it
    // samples has to be projected exactly as the picture projects it, and a
    // rule spelled four ways is a rule three of them will eventually break.
    let drivers_for = |l: &lumit_core::model::Layer,
                       dlt: f64,
                       ctx: Arc<ExpressionContext>|
     -> lumit_core::fx::drivers::ResolvedDrivers {
        let projection = points_projection(expr_doc, comp, l, t_comp, dlt, ctx.clone())
            .unwrap_or(lumit_core::fx::points::Projection::FLAT);
        lumit_core::fx::drivers::resolve_drivers_cloned(
            &l.graph,
            dlt,
            ctx,
            Some(&audio),
            projection,
            clone_number,
        )
    };

    // One layer's own picture at layer time `lt`, alone: its source pixels with
    // its paint stamped in and its masks gating them, and nothing else — no
    // effects, no transform, no puppet. What [`pixels_for`] is built on, and
    // what a puppet's mesh is built over, which is why the time is a parameter
    // rather than this frame's (docs/impl/puppet.md §1.1: the mesh is taken at
    // the block's reference time).
    let solo_at = |layer: &lumit_core::model::Layer, lt: f64| -> Option<LayerPixels> {
        let context = Arc::new(ExpressionContext {
            document: expr_doc.clone(),
            layer: Some(layer.id),
            comp: Some(comp.id),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });

        // A layer acting as an adjustment has no picture of its own to
        // give — by kind or by the flag, the same answer, so a footage layer
        // switched on stops handing its frames to a matte too.
        if layer.is_adjustment() {
            return None;
        }

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
                        own_shutter_average(layer, lp, lt),
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
                // want the solid at its real size to work on.
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
                    && !layer.effects.iter().any(|e| e.enabled)
                    // A style draws on the layer's own raster exactly as an
                    // effect does, so a styled solid needs its real
                    // pixels for the same reason an effected one does — an
                    // 8 × 8 tile has no edge for a shadow to fall off.
                    && !layer.styles.iter().any(|e| e.enabled)
                    // A light falls differently on every pixel, so lit on the
                    // tile its pool would come out as a smear of sixty-four
                    // pixels stretched over the whole solid. So does a shadow.
                    && shading_lights(comp, layer, t_comp, &context).is_empty();
                let (tw, th) = if plain { (8, 8) } else { (sd.width, sd.height) };
                (
                    Arc::new(px_tile(&px, tw, th)),
                    tw,
                    th,
                    (sd.width as f32, sd.height as f32),
                )
            }),
            LayerKind::Text { document } => in_span(layer).then(|| {
                // The words at *this* layer time — a plain document hands back
                // what was typed; an expression-driven one is evaluated here,
                // which is what makes a caption able to print a live value.

                let line = document.resolved_text(context.clone());
                let size = document.size as f32;
                // **Text on a path**. The named mask's own flattened
                // curve, read through the carriage every path-walking effect
                // reads — so the glyphs and a Stroke on the same mask
                // agree about where the curve is. A layer whose row names
                // nothing, or a mask since deleted, flattens to an empty
                // polyline and the line lays straight: refuse calmly, never
                // an empty layer (docs/14 §4).
                let spine = document
                    .path
                    .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
                    .filter(|p| !p.is_empty());
                // **The letters move separately**. What each one is
                // asked to do is worked out here, from the animators' own
                // keyframed numbers at this layer time; a layer with none hands
                // back an empty list and the rasteriser takes the path it
                // always took, byte for byte.
                let xforms = lumit_core::text::glyph_xforms(&document.animators, &line, lt);
                // An unstyled single line draws the way it always has. A
                // styled one, or one with a line break, is set by the shaper.
                let block = lumit_text::TextBlock::of(document, &line);
                let r = match &spine {
                    Some(path) => {
                        let (w, h) = lumit_text::path_box(path, size);
                        lumit_text::rasterise_along(
                            &block,
                            path,
                            document.path_offset.value_at(lt) as f32,
                            w,
                            h,
                            &xforms,
                        )
                    }
                    None => lumit_text::rasterise(&block, &xforms),
                };
                (
                    Arc::new(r.rgba),
                    r.width,
                    r.height,
                    (r.width as f32, r.height as f32),
                )
            }),
            // Vector art: rasterised at the size the frame is being drawn at,
            // into its own bounding box, which is also the layer's natural
            // size. Unlike every other kind, that size moves when the art is
            // edited.
            LayerKind::Shape { contents } => in_span(layer)
                .then(|| lumit_core::shape::contents_at(contents, lt, &context))
                .and_then(|contents| {
                    let (x0, y0, x1, y1) = lumit_core::shape::contents_bounds(&contents, lt)?;
                    let natural_w = (x1 - x0).max(1.0);
                    let natural_h = (y1 - y0).max(1.0);
                    // The same reduced-resolution rule the other rasterised
                    // kinds follow: draw at the working scale, and let the
                    // placement matrix carry the natural size.
                    let w = natural_w.round().max(1.0) as u32;
                    let h = natural_h.round().max(1.0) as u32;
                    Some((
                        Arc::new(lumit_core::shape::rasterise_contents(
                            &contents, w, h, x0, y0, x1, y1, lt,
                        )),
                        w,
                        h,
                        (natural_w as f32, natural_h as f32),
                    ))
                }),
            LayerKind::Precomp { .. } => None, // handled as Nested below
            LayerKind::Camera { .. } => None,  // shapes the view, draws nothing
            // A light is something other layers SEE. It has no picture
            // of its own; the effects that read lights take them from
            // `Composition::lights_at`, not from the draw list.
            LayerKind::Light { .. } => None,
        };
        // Footage is the only kind that can arrive as anything but bytes.
        let format = match &layer.kind {
            LayerKind::Footage { .. } | LayerKind::Sequence { .. } => pixels_by_layer
                .get(&layer.id)
                .map_or(lumit_media::decode::PixelFormat::Srgb8, |lp| lp.format),
            _ => lumit_media::decode::PixelFormat::Srgb8,
        };
        raw.map(|(mut rgba, w, h, natural)| {
            // Nothing to stamp in is the ordinary layer, and it keeps the
            // bytes it arrived with: both stages return on an empty list
            // anyway, and asking for them by name is what lets a decoded frame
            // travel to the draw shared rather than copied.
            if !layer.paint.is_empty() || !layer.masks.is_empty() {
                let rgba = Arc::make_mut(&mut rgba);
                // Paint first, masks second: a stroke is part of the layer's
                // picture, and a mask gates the picture (docs/06 render
                // order). Painting after the mask would let a brush draw
                // outside the shape the mask cut.
                //
                // Both stages come in a width to match the plate, so a float
                // layer stays float through either of them.
                let strokes = match format {
                    lumit_media::decode::PixelFormat::LinearF32 => {
                        lumit_core::paint::apply_strokes_f32
                    }
                    lumit_media::decode::PixelFormat::Srgb8 => lumit_core::paint::apply_strokes,
                };
                strokes(
                    rgba,
                    w,
                    h,
                    f64::from(natural.0),
                    f64::from(natural.1),
                    &layer.paint,
                    // The layer's own clock, as a mask's keys are read on:
                    // a stroke keyed to draw itself on travels with the layer.
                    lt,
                );
                // A mask only ever touches alpha, so both widths keep their
                // colour exactly as it arrived — a masked OpenEXR is still an
                // OpenEXR.
                let gate = match format {
                    lumit_media::decode::PixelFormat::LinearF32 => {
                        lumit_core::mask::apply_masks_f32
                    }
                    lumit_media::decode::PixelFormat::Srgb8 => lumit_core::mask::apply_masks,
                };
                gate(
                    rgba,
                    w,
                    h,
                    f64::from(natural.0),
                    f64::from(natural.1),
                    &layer.masks,
                    lt,
                );
            }
            (rgba, w, h, natural, format)
        })
    };

    let pixels_for = |layer: &lumit_core::model::Layer| -> Option<LayerPixels> {
        // The layer's own clock, the same one its transform, its masks and its
        // paint are read at — a keyframed shape modifier travels with
        // the layer exactly as a keyframed mask does.
        let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
        let mut pixels = solo_at(layer, lt)?;
        // Puppet last of the three (docs/impl/puppet.md §3): paint and masks
        // decide what the picture *is*, and the puppet carries that picture
        // through its mesh. The mesh is built over the same solo picture at the
        // block's reference time, which is the alpha the pins were placed on.
        if let Some(block) = &layer.puppet {
            warp_puppet(&mut pixels, layer, block, lt, &solo_at);
        } else if let Some((density, expansion)) = crate::puppet::previewing(layer.id) {
            // A puppet tool is armed on a layer nobody has pinned yet: the mesh
            // the first click has to land inside does not exist until this
            // builds it (docs/impl/puppet.md §5). No second render — the alpha
            // is the picture already in hand.
            preview_puppet(&pixels, layer.id, density, expansion);
        }
        Some(pixels)
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
    // gate.
    // A reference to a PRECOMP — as a layer input or as a track matte: its
    // picture exists only as a render, so package the nested comp's draw list
    // for realise to run recursively — the DrawSource::Nested shape, on the
    // referencing slot.
    // `visited_path` is a snapshot of the ancestor chain at this comp's
    // entry, so a matte that (transitively) contains its own comp stops at
    // the cycle instead of recursing forever; a fresh clone per reference keeps
    // the closure borrow-free. Footage INSIDE such a precomp decodes with the
    // rest of the frame: the decode planner walks matte and layer-input
    // references (plan::collect_comp_jobs) whether or not the referenced
    // layer is visible.
    let visited_path: Vec<uuid::Uuid> = visited.clone();

    // The node graph lowering's own carriage (docs/impl/node-graph-comp.md
    // §2.3), holding what every depth of it reads. Every road that reaches a
    // comp with a graph goes through [`graph_comp_draw`] with this, so the
    // Precomp arm, the matte arm and a Read box cannot lower one differently.
    let lower = GraphLower {
        doc,
        t: t_comp,
        frame_t,
        pixels_by_layer,
        keys,
        audio: &audio,
        pixels_for: &pixels_for,
        clone: clone_number,
    };

    // The values a Precomp layer of a node graph hands the graph's Inputs
    // (§5.3): its own instance's rows, with a driver wire's number in place of
    // the stored one, exactly as a Node graph effect's rows are read. Empty for
    // a layer carrying no instance, which is every layer over a layer comp.
    let graph_values = |src: &lumit_core::model::Layer,
                        graph: &lumit_core::comp_graph::CompGraph,
                        lt: f64|
     -> Vec<(String, lumit_core::model::EffectValue)> {
        let Some(inst) = src.graph_inputs.as_ref() else {
            return Vec::new();
        };
        // Resolved only where the layer has wires at all: a resolve builds a
        // points projection, and the overwhelming case has nothing to
        // substitute.
        let drivers = (!src.graph.edges.is_empty()).then(|| {
            drivers_for(
                src,
                lt,
                Arc::new(ExpressionContext {
                    document: expr_doc.clone(),
                    comp: Some(comp.id),
                    layer: Some(src.id),
                    comp_time: t_comp,
                    current_depth: 0,
                    inputs: None,
                }),
            )
        });
        lumit_core::fx::effects::node_graph::overrides_of(inst, graph, drivers.as_ref())
    };

    // Whether a driver wire feeds this layer's placed-graph instance. Such a
    // value is substituted where a stored one would be read, and the nested
    // frame's name is made from the stored instance, so a driven placement is
    // named not at all: it renders live and banks nothing, which is the same
    // answer unprobed footage gets.
    //
    // ponytail: no nested caching for a driven placed graph. Fold the
    // substituted values into the name if scrubbing one ever feels slow.
    let driven_inputs = |src: &lumit_core::model::Layer| -> bool {
        use lumit_core::graph::{InputRef, NodeRef};
        src.graph_inputs.as_ref().is_some_and(|inst| {
            src.graph.edges.iter().any(|e| {
                matches!(&e.to, InputRef::Param { node: NodeRef::Effect(id), .. } if *id == inst.id)
            })
        })
    };

    let nested_comp_draw =
        |src: &lumit_core::model::Layer| -> Option<Box<crate::draw::NestedInputDraw>> {
            let lumit_core::model::LayerKind::Precomp { comp: nested_id } = &src.kind else {
                return None;
            };
            if visited_path.contains(nested_id) {
                return None;
            }
            let nested = doc.comp(*nested_id)?;
            // The Retime map (§5.6): a matte source and a background plate read
            // the moment their layer shows, exactly as the picture road does.
            let slt = lumit_core::model::nested_source_time(
                src,
                nested,
                lumit_core::time::layer_time(t_comp, src.start_offset.0),
            );
            let frame_slt = lumit_core::model::nested_source_time(
                src,
                nested,
                lumit_core::time::layer_time(frame_t, src.start_offset.0),
            );
            let mut path = visited_path.clone();
            path.push(*nested_id);
            // A placed node graph is walked with its own Input values
            // (§5.3). It goes through the nested comp's own walk like any
            // other, so its Reads of footage are found at this moment too.
            let values = nested
                .graph
                .as_ref()
                .filter(|_| src.graph_inputs.is_some())
                .map_or_else(Vec::new, |graph| graph_values(src, graph, slt));
            let draws = comp_walk(
                doc,
                nested,
                slt,
                frame_slt,
                pixels_by_layer,
                &mut path,
                keys,
                false,
                Walk {
                    values: &values,
                    ..inner
                },
                None,
            );
            Some(Box::new(crate::draw::NestedInputDraw {
                width: nested.width,
                height: nested.height,
                background: [0.0, 0.0, 0.0, 0.0],
                draws,
                camera: Some(crate::track::camera_pose(doc, nested, slt)),
                key: keys
                    .filter(|_| !driven_inputs(src))
                    .and_then(|k| k.nested_key_with(nested, slt, src.graph_inputs.as_ref())),
                px_per_metre: nested.px_per_metre(slt),
                scene_depth: false,
                trace_layers: false,
            }))
        };
    let nested_input_for = |src: &lumit_core::model::Layer| -> Option<DofInputDraw> {
        let nested = nested_comp_draw(src)?;
        Some(DofInputDraw {
            natural: [nested.width as f32, nested.height as f32],
            rgba: Vec::new(),
            tex_w: nested.width,
            tex_h: nested.height,
            // A comp is realised on the card; there are no source bytes here.
            format: lumit_media::decode::PixelFormat::Srgb8,
            fx: Default::default(),
            colour_tables: Vec::new(),
            // A comp reference runs no stack of its own here: its layers were
            // walked whole, each with its own carriages already filled.
            roto_mattes: Vec::new(),
            planes: Vec::new(),
            nested: Some(nested),
            graph_fx: Vec::new(),
            // A comp's layers were each interpreted as themselves while it was
            // realised; the picture that comes out is already working-space.
            colour_space: None,
            // Filled in by `layer_slot`, which is the slot everything reads.
            place: None,
        })
    };

    // **The roto mattes, the coverage carriage** (docs/impl/roto.md §5).
    // One slot per enabled `roto_brush` op that resolves to an op at all, the
    // same two conditions `mask_paths_for` applies, so this list stays 1:1 with
    // the ops `run_ops` walks, with its own counter there.
    //
    // The lookup is by (instance, **source** frame): a matte describes the
    // file's frames, so it survives every transform, retime and preview
    // tier this layer applies, and one shot's mattes serve every comp cutting
    // it. `None`, outside the propagated span, nothing propagated yet, the
    // cache folder deleted, is the effect's passthrough and never a fault.
    //
    // No lock is held past this call: the store clones an `Arc` out from under
    // its own guard, and what lands here is that same allocation.
    let roto_mattes_for = |effects: &[lumit_core::model::EffectInstance],
                           source_frame: i64|
     -> Vec<Option<crate::draw::RotoMatteDraw>> {
        use lumit_core::model::EffectNamespace;
        effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter(|e| e.effect.match_name == lumit_core::roto::ROTO_BRUSH)
            .filter(|e| {
                lumit_core::fx::catalogue::BUILTIN_DEFS
                    .get(&e.effect.match_name)
                    .is_some_and(|def| def.is_image_op())
            })
            .map(|e| roto_matte_of(e, source_frame))
            .collect()
    };

    // **The planes - the analysis carriage** (docs/impl/addons.md §6.1).
    // One slot per enabled planes-tier op that resolves to an op at all, on
    // the one predicate `lumit_core::planes` owns, so this list stays 1:1 with
    // the ops `run_ops` walks and has its own counter there.
    //
    // Looked up by (instance, **source** frame) for `roto_mattes_for`'s reason:
    // a plane describes the file's frames, so it survives every transform,
    // retime and preview tier the layer applies. `None` - outside the analysed
    // span, nothing analysed yet, no pack installed, the cache folder deleted -
    // is the effect's passthrough and never a fault.
    //
    // No lock is held past this call: the store clones an `Arc` out from under
    // its own guard, and what lands here is that same allocation.
    let planes_for = |effects: &[lumit_core::model::EffectInstance],
                      source_frame: i64|
     -> Vec<Option<crate::draw::PlaneDraw>> {
        effects
            .iter()
            .filter(|e| e.enabled && lumit_core::planes::task_of(e).is_some())
            .filter(|e| {
                lumit_core::fx::catalogue::BUILTIN_DEFS
                    .get(&e.effect.match_name)
                    .is_some_and(|def| def.is_image_op())
            })
            .map(|e| plane_of(e, source_frame))
            .collect()
    };

    // One referenced layer resolved into an input slot — the body
    // `dof_inputs_for` and `mattes_for` share (docs/impl/layer-input.md §2): the
    // span gate, the nested-precomp render, and the masks-and-effects folding,
    // so a matte and a background plate can never disagree about what "a layer
    // rendered alone" means.
    let layer_slot_plain =
        |e: &lumit_core::model::EffectInstance, param: &str| -> Option<DofInputDraw> {
            let id = e.layer_ref(param)?;
            let src = comp.layers.iter().find(|l| l.id == id)?;
            if !in_span(src) {
                return None;
            }
            // A Precomp reference renders its comp — "a white circle
            // in a precomp" is the natural way to author a flare source, and
            // a depth pass authored as a comp is the same shape.
            if let Some(nested) = nested_input_for(src) {
                return Some(nested);
            }
            let mode = e.layer_source(param);
            // Layer source. None samples the layer's raw pixels —
            // clear its masks so `pixels_for` skips them; Masks and Effects
            // and masks keep them.
            let (rgba, tex_w, tex_h, natural, format) = if mode.applies_masks() {
                pixels_for(src)?
            } else {
                let mut bare = src.clone();
                bare.masks.clear();
                pixels_for(&bare)?
            };
            // Effects and masks: resolve the referenced layer's own
            // stack at its layer time so render_dof_inputs runs it on the
            // texture before resampling. Uses that layer's decode scale (its
            // px@comp radii stay honest), the same resolve export uses. Empty
            // otherwise.
            let (fx, colour_tables) = if mode.folds_effects() && src.switches.fx {
                let slt = lumit_core::time::layer_time(t_comp, src.start_offset.0);
                let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let scale = tex_w as f32 / natural.0.max(1.0);
                let markers = lumit_core::fx::markers::MarkerContext::for_layer(comp, src);
                // The referenced layer's own effects, so its own expressions
                // resolve about it rather than about the layer that pointed
                // at it.
                let context = Arc::new(ExpressionContext {
                    document: expr_doc.clone(),
                    comp: Some(comp.id),
                    layer: Some(src.id),
                    comp_time: t_comp,
                    current_depth: 0,
                    inputs: None,
                });
                // The referenced layer's own driver graph too: a wire
                // substitutes where a keyframe would have been read, so it belongs
                // to whichever stack is being resolved.
                let drivers = drivers_for(src, slt, context.clone());
                (
                    lumit_core::fx::resolved::resolve_stack_temporal_named(
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
                    colour_tables(&src.effects, slt),
                )
            } else {
                Default::default()
            };
            // The referenced layer's own baked pictures, on the same two
            // predicates and the same (instance, source frame) lookup its own draw
            // uses. Empty when its stack is not being folded in, which is when
            // there are no ops to carry them to.
            let src_frame = pixels_by_layer.get(&src.id).map_or(0, |px| px.source_frame);
            let (roto_mattes, planes) = if fx.is_empty() {
                (Vec::new(), Vec::new())
            } else {
                (
                    roto_mattes_for(&src.effects, src_frame),
                    planes_for(&src.effects, src_frame),
                )
            };
            Some(DofInputDraw {
                rgba: rgba.to_vec(),
                tex_w,
                tex_h,
                format,
                fx,
                colour_tables,
                roto_mattes,
                planes,
                nested: None,
                colour_space: crate::colour::footage_colour_space(doc, &src.kind),
                natural: [natural.0, natural.1],
                // Filled in by `layer_slot`, which is the slot everything reads.
                graph_fx: Vec::new(),
                place: None,
            })
        };

    // **The Node graph effects' plans** (§2.4): one per enabled `node_graph`
    // op in stack order, on the same two conditions `mattes_for` applies, so
    // the list stays 1:1 with the ops `run_ops` walks and the k-th closure
    // lands on the k-th such op.
    //
    // The first picture Input is the layer's own picture, which only `run_ops`
    // holds; every further one is a row of this instance, rendered alone on
    // the carriage a depth pass travels. A comp that is missing, is not a node
    // graph, or is already on the visited path lowers to a plan with no
    // Output, which the walk renders as the picture unchanged.
    //
    // `feed` is how a further picture Input's layer is fetched, so the plans
    // of a layer that is itself being read as an input can fetch theirs
    // plainly and the two cannot call each other for ever.
    let graph_fx_with =
        |owner: Option<&lumit_core::model::Layer>,
         effects: &[lumit_core::model::EffectInstance],
         lt: f64,
         frame_lt: f64,
         feed: &dyn Fn(&lumit_core::model::EffectInstance, &str) -> Option<DofInputDraw>|
         -> Vec<GraphDraw> {
            use lumit_core::model::EffectNamespace;
            // **The host layer's clock, not the comp's.** The decode plan and the
            // frame key both read the named comp at the layer time, so a layer
            // with a start offset would otherwise take its picture from one clock
            // and its frame's name from another.
            let lower = GraphLower {
                t: lt,
                frame_t: frame_lt,
                ..lower
            };
            let instances: Vec<&lumit_core::model::EffectInstance> = effects
                .iter()
                .filter(|e| {
                    e.enabled
                        && e.effect.namespace == EffectNamespace::Builtin
                        && e.effect.match_name == lumit_core::comp_graph::NODE_GRAPH
                })
                .filter(|e| {
                    lumit_core::fx::catalogue::BUILTIN_DEFS
                        .get(&e.effect.match_name)
                        .is_some_and(|def| def.is_image_op())
                })
                .collect();
            // The overwhelming case, and it costs a filter over the stack.
            if instances.is_empty() {
                return Vec::new();
            }
            // The layer's own wires, resolved once for all its graph ops: a
            // driver into a graph's row is substituted where a keyframe would
            // have been read, exactly as it is on any other effect. A group
            // header carries no graph, so nothing substitutes there.
            let drivers = owner.map(|l| {
                drivers_for(
                    l,
                    lt,
                    Arc::new(ExpressionContext {
                        document: expr_doc.clone(),
                        comp: Some(comp.id),
                        layer: Some(l.id),
                        comp_time: t_comp,
                        current_depth: 0,
                        inputs: None,
                    }),
                )
            });
            instances
                .into_iter()
                .map(|e| {
                    let mut path = visited_path.clone();
                    let inner = lumit_core::fx::effects::node_graph::comp_of(e)
                        .filter(|id| !path.contains(id))
                        .and_then(|id| doc.comp(id))
                        .and_then(|c| c.graph.as_ref().map(|g| (c, g)));
                    let Some((inner_comp, inner_graph)) = inner else {
                        // A dangling, cyclic or layer-comp reference: the
                        // passthrough, and no plan to walk.
                        return GraphDraw {
                            width: comp.width,
                            height: comp.height,
                            room: doc.graph_room.frames(),
                            steps: Vec::new(),
                            output: None,
                        };
                    };
                    let mut plan = GraphDraw {
                        width: inner_comp.width,
                        height: inner_comp.height,
                        room: doc.graph_room.frames(),
                        steps: Vec::new(),
                        output: None,
                    };
                    // The first picture Input is the layer's own picture; every
                    // further one is a layer row of this instance, which the
                    // schema never sees because those rows are derived (§1.5).
                    let mut feeds = vec![GraphFeed::Provided];
                    for picture in inner_graph
                        .inputs()
                        .filter(|i| i.kind == lumit_core::comp_graph::InputKind::Picture)
                        .skip(1)
                    {
                        feeds.push(GraphFeed::Picture(Box::new(
                            feed(e, &picture.id)
                                .map_or(LayerInputDraw::Absent, LayerInputDraw::Layer),
                        )));
                    }
                    let overrides = lumit_core::fx::effects::node_graph::overrides_of(
                        e,
                        inner_graph,
                        drivers.as_ref(),
                    );
                    path.push(inner_comp.id);
                    plan.output = lower.lower(
                        inner_comp,
                        inner_graph,
                        lumit_core::comp_graph::GraphView {
                            values: &overrides,
                            // Applied as an effect: every picture Input is fed
                            // from the host, so no preview item stands in.
                            pictures_fed: true,
                        },
                        &mut feeds,
                        &mut path,
                        &mut plan,
                    );
                    plan
                })
                .collect()
        };

    // **Where a map sits**, for an effect with a Map placement row beside the
    // Layer row. As placed, the referenced layer's own pixels are carried
    // into the pixels of the layer the effect is on, so the two line up in
    // the comp however either is moved, scaled or parented. An adjustment
    // layer and a group header run their stacks on the comp itself.
    let map_place = |e: &lumit_core::model::EffectInstance,
                     param: &str|
     -> Option<Box<crate::draw::MapPlace>> {
        use lumit_core::fx::builtins::MapPlacement;
        // Mid-grey is no push at all, whichever channel steers it, so the
        // frame round a placed map stays still.
        let ground = if matches!(
            e.effect.match_name.as_str(),
            "displacement_map" | "vector_blur"
        ) {
            [0.5; 4]
        } else {
            [0.0; 4]
        };
        let placed = match lumit_core::fx::builtins::map_placement(e, param) {
            MapPlacement::Stretch => return None,
            MapPlacement::Centre => None,
            MapPlacement::AsPlaced => {
                let src = comp
                    .layers
                    .iter()
                    .find(|l| Some(l.id) == e.layer_ref(param))?;
                let owner = comp
                    .layers
                    .iter()
                    .find(|l| l.effects.iter().any(|x| x.id == e.id))
                    .filter(|l| !l.is_adjustment());
                let context = Arc::new(ExpressionContext {
                    document: expr_doc.clone(),
                    comp: Some(comp.id),
                    layer: Some(src.id),
                    comp_time: t_comp,
                    current_depth: 0,
                    inputs: None,
                });
                let to_comp = world_placement(comp, src, t_comp, context.clone());
                Some(match owner {
                    // A layer scaled to nothing shows nothing, so any answer
                    // will do.
                    Some(owner) => lumit_gpu::composite::invert_place(world_placement(
                        comp, owner, t_comp, context,
                    ))
                    .map_or(to_comp, |from_comp| {
                        lumit_gpu::composite::concat_place(from_comp, to_comp)
                    }),
                    None => to_comp,
                })
            }
        };
        Some(Box::new(crate::draw::MapPlace { placed, ground }))
    };

    // **The slot everything reads**: the referenced layer as above, with the
    // plans of its own Node graph effects beside its stack. Without them a
    // layer that applies a graph lost it the moment another effect read the
    // layer, so a matte or a clone of it was the wrong picture.
    //
    // The plans' own further pictures are fetched plainly, one layer deep and
    // no further, which is the bound a referenced layer's rows already keep.
    let layer_slot = |e: &lumit_core::model::EffectInstance, param: &str| -> Option<DofInputDraw> {
        let mut input = layer_slot_plain(e, param)?;
        // Only where its stack is folded in: those are the ops the plans go to.
        let src = e
            .layer_ref(param)
            .and_then(|id| comp.layers.iter().find(|l| l.id == id))
            .filter(|_| !input.fx.is_empty());
        if let Some(src) = src {
            let slt = lumit_core::time::layer_time(t_comp, src.start_offset.0);
            // The frame's own time on the layer's clock, so a comp the graph
            // reads finds its clips at this walk's moment as well.
            let frame_slt = lumit_core::time::layer_time(frame_t, src.start_offset.0);
            input.graph_fx =
                graph_fx_with(Some(src), &src.effects, slt, frame_slt, &layer_slot_plain);
        }
        input.place = map_place(e, param);
        Some(input)
    };
    // **A backdrop effect's shape**: the layer it sits on, alone and before
    // any of its effects, placed in the comp as the layer is, since the effect
    // runs over the comp-sized composite below. `None` on an adjustment layer,
    // which has no picture to give.
    let own_shape = |owner: uuid::Uuid| -> Option<DofInputDraw> {
        let src = comp
            .layers
            .iter()
            .find(|l| l.id == owner)
            .filter(|l| !l.is_adjustment())?;
        let context = Arc::new(ExpressionContext {
            document: expr_doc.clone(),
            comp: Some(comp.id),
            layer: Some(src.id),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });
        let place = Some(Box::new(crate::draw::MapPlace {
            placed: Some(world_placement(comp, src, t_comp, context)),
            ground: [0.0; 4],
        }));
        if let Some(mut nested) = nested_input_for(src) {
            nested.place = place;
            return Some(nested);
        }
        let (rgba, tex_w, tex_h, natural, format) = pixels_for(src)?;
        Some(DofInputDraw {
            rgba: rgba.to_vec(),
            tex_w,
            tex_h,
            format,
            fx: Default::default(),
            colour_tables: Default::default(),
            roto_mattes: Vec::new(),
            planes: Vec::new(),
            nested: None,
            colour_space: crate::colour::footage_colour_space(doc, &src.kind),
            natural: [natural.0, natural.1],
            graph_fx: Vec::new(),
            place,
        })
    };
    let graph_fx_for =
        |owner: Option<&lumit_core::model::Layer>,
         effects: &[lumit_core::model::EffectInstance],
         lt: f64,
         frame_lt: f64|
         -> Vec<GraphDraw> { graph_fx_with(owner, effects, lt, frame_lt, &layer_slot) };

    // **Clone to points' pictures.** Its slot is not a picture a row but the
    // list the effect lays out itself: each layer at each moment a stamp may
    // show, or one render a clone. `CloneToPoints::planned` says what they
    // are, and the draw reads the same layout back off the list.
    //
    // `known` is the stream's point count when the realiser is asking again
    // for a stream that was made during the render. On the first ask such a
    // list is left for then (`LayerInputDraw::Late`) where the copies are
    // numbered: how many renders there are, and what each Clone index reads
    // as its Count, both follow from a count nobody has yet.
    //
    // ponytail: every picture is a whole render of the clone layer, up to 32
    // of them a frame, and one at another moment or with a clone number walks
    // this comp again for that layer alone. Nothing is kept between frames,
    // and a nested comp at another moment is never served from the nested
    // store. Keep the pictures by layer, time and clone number if a profile
    // shows a cascade costing.
    let clone_pictures = |owner: uuid::Uuid,
                          e: &lumit_core::model::EffectInstance,
                          known: Option<usize>|
     -> LayerInputDraw {
        use lumit_core::fx::effects::clone_to_points::CloneToPoints as C;
        let host = comp.layers.iter().find(|l| l.id == owner);
        let lt = host.map_or(t_comp, |l| {
            lumit_core::time::layer_time(t_comp, l.start_offset.0)
        });
        // The whole comp, which is what the effect's own resolve reads. A walk
        // of the layers below an adjustment is handed only some of it.
        let whole = doc.comp(comp.id).unwrap_or(comp);
        // How many points there are to number. Only asked when the copies are
        // numbered, since it is a second evaluation of the producer.
        let numbered = C::stored(e, lt).per_clone_in(e, doc, whole);
        let points = match (numbered, known) {
            (false, _) => None,
            (true, Some(n)) => Some(n),
            (true, None) => {
                let wired = host
                    .zip(lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name))
                    .map(|(layer, def)| {
                        let context = Arc::new(ExpressionContext {
                            document: expr_doc.clone(),
                            comp: Some(comp.id),
                            layer: Some(layer.id),
                            comp_time: t_comp,
                            current_depth: 0,
                            inputs: None,
                        });
                        points_input_for(
                            layer,
                            e,
                            def,
                            lt,
                            1.0 / comp.frame_rate.fps().max(1.0),
                            lumit_core::fx::points::Projection::FLAT,
                            &context,
                            &audio,
                            clone_number,
                        )
                    })
                    .unwrap_or_default();
                match wired.streams.first().map_or(0, |s| s.len()) {
                    // The wire names an effect above whose points are made on
                    // the card: the list waits for them.
                    0 if wired.from.first().copied().flatten().is_some() => {
                        return LayerInputDraw::Late(Box::new(crate::draw::LateClones {
                            comp: comp.id,
                            owner,
                            effect: e.id,
                            t: t_comp,
                            frame_t,
                            visited: visited_path.clone(),
                            clone: clone_number,
                        }));
                    }
                    // Nothing wired, or no points: nothing is stamped, so
                    // nothing is rendered.
                    0 => return LayerInputDraw::Several(Vec::new()),
                    n => Some(n),
                }
            }
        };
        LayerInputDraw::Several(
            C::planned(e, doc, whole, t_comp, lt, points)
                .into_iter()
                .map(|planned| {
                    let Some(p) = planned else {
                        return LayerInputDraw::Absent;
                    };
                    // "This layer" is the effect's own input, which has no other
                    // moment to show.
                    if p.layer == owner {
                        return LayerInputDraw::ThisLayer;
                    }
                    if p.clone.is_none() && p.time.to_bits() == t_comp.to_bits() {
                        return layer_slot(e, p.row)
                            .map_or(LayerInputDraw::Absent, LayerInputDraw::Layer);
                    }
                    let mut probe = Probe::Picture {
                        effect: e,
                        row: p.row,
                        found: None,
                    };
                    // No keyer: a nested comp built for another moment holds its
                    // footage a whole frame at a time, which is not always the
                    // picture its name would claim. A copy with no number of
                    // its own keeps the one this walk is already rendered as.
                    comp_walk(
                        doc,
                        comp,
                        p.time,
                        frame_t,
                        frame_pixels,
                        &mut visited_path.clone(),
                        None,
                        true,
                        Walk {
                            clone: p.clone.unwrap_or(clone_number),
                            values: &[],
                            rebuild: with.rebuild,
                            seen_through: None,
                        },
                        Some(&mut probe),
                    );
                    match probe {
                        Probe::Picture { found: Some(d), .. } => LayerInputDraw::Layer(d),
                        _ => LayerInputDraw::Absent,
                    }
                })
                .collect(),
        )
    };

    // A probe wants one thing made at this walk's time and nothing else.
    match probe {
        Some(Probe::Picture { effect, row, found }) => {
            *found = layer_slot(effect, row);
            return Vec::new();
        }
        Some(Probe::Clones {
            owner,
            effect,
            points,
            found,
        }) => {
            let instance = comp
                .layers
                .iter()
                .find(|l| l.id == *owner)
                .and_then(|l| l.effects.iter().find(|e| e.id == *effect));
            if let Some(LayerInputDraw::Several(list)) =
                instance.map(|e| clone_pictures(*owner, e, Some(*points)))
            {
                *found = list;
            }
            return Vec::new();
        }
        None => {}
    }

    // **The auxiliary-layer inputs** (docs/impl/layer-input.md §2): one
    // slot per enabled built-in whose declaration names a Layer row that is not
    // its matte — Light wrap's Background, Texturize's Texture, Fast motion
    // blur's Motion vectors, Set matte's source — AND that resolves to an op at
    // all, exactly the pair of conditions `mattes_for` below applies, so this
    // list stays 1:1 with the ops `run_ops` walks.
    //
    // The parameter comes from the schema's own
    // [`EffectSchema::layer_input`] rather than a table of match names here: a
    // table is a second rule, and a second rule is a thing to forget when an
    // effect gains a layer row.
    // `below` is the comp's pixels per metre on an adjustment layer with 3D
    // layers below it, whose Depth row pointed at itself reads their depth.
    let dof_inputs_for =
        |owner: uuid::Uuid, effects: &[lumit_core::model::EffectInstance], below: Option<f64>| {
            use lumit_core::model::EffectNamespace;
            effects
                .iter()
                // An LFX effect's second input rides here too.
                .filter(|e| {
                    e.enabled
                        && matches!(
                            e.effect.namespace,
                            EffectNamespace::Builtin | EffectNamespace::Lfx
                        )
                })
                .filter_map(|e| {
                    let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name)?;
                    let param = def.schema().layer_input()?;
                    def.is_image_op().then_some((e, def, param))
                })
                .map(|(e, def, param)| {
                    // A backdrop effect on a layer with a picture takes that
                    // picture wherever its row picks no other layer.
                    if def.backdrop() && e.layer_ref(param).is_none_or(|id| id == owner) {
                        if let Some(d) = own_shape(owner) {
                            return LayerInputDraw::Layer(d);
                        }
                    }
                    let schema = def.schema();
                    let slot = |param: &str| {
                        // A row read for its layer's shapes has no picture.
                        if def.shape_inputs().contains(&param) {
                            return LayerInputDraw::Absent;
                        }
                        // A Depth row pointed at a Precomp with 3D layers in it
                        // reads their distance rather than its picture. A flat
                        // precomp is a picture, a depth pass made as a comp.
                        let depth = def.depth_inputs().contains(&param);
                        let three_d = e
                            .layer_ref(param)
                            .and_then(|id| comp.layers.iter().find(|l| l.id == id))
                            .and_then(|l| match l.kind {
                                LayerKind::Precomp { comp } => doc.comp(comp),
                                _ => None,
                            })
                            .is_some_and(|c| c.layers.iter().any(|l| l.switches.three_d));
                        let own = e.layer_ref(param) == Some(owner);
                        if let Some(ppm) = below.filter(|_| own && depth) {
                            return LayerInputDraw::Below(ppm);
                        }
                        // "This layer": a reference to the layer the effect
                        // is ON is not a second render — it is the effect's own
                        // input, which `run_ops` already holds. Except a Depth
                        // row on a 3D precomp, which wants that precomp's
                        // distance rather than its picture.
                        if own && !(depth && three_d) {
                            return LayerInputDraw::ThisLayer;
                        }
                        let Some(mut d) = layer_slot(e, param) else {
                            return LayerInputDraw::Absent;
                        };
                        if depth && three_d {
                            if let Some(n) = d.nested.as_mut() {
                                n.scene_depth = true;
                                n.trace_layers = own && def.traces_scene(e);
                            }
                        }
                        LayerInputDraw::Layer(d)
                    };
                    // Clone to points lays its own list out.
                    if e.effect.match_name == "clone_to_points" {
                        return clone_pictures(owner, e, None);
                    }
                    // An effect with several layer rows hands them over as one
                    // list in row order, so it is still one slot for one op.
                    if schema.layer_inputs().nth(1).is_some() {
                        LayerInputDraw::Several(schema.layer_inputs().map(slot).collect())
                    } else {
                        slot(param)
                    }
                })
                .collect()
        };

    // **The Matte inputs — the one carriage** (docs/08 §2.6). One slot
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
    // edge feeds an effect the layer's OWN masked source alpha at that point in
    // the chain, and overrides the Matte parameter while it exists. That is
    // precisely what "this layer" already means to the render — the effect's
    // own input rather than a second pass over another layer — so the wire
    // lowers to the answer this builder has always had, and nothing downstream
    // learns a new shape.
    let mattes_for = |owner: uuid::Uuid,
                      effects: &[lumit_core::model::EffectInstance],
                      graph: &lumit_core::graph::LayerGraph|
     -> Vec<LayerInputDraw> {
        use lumit_core::model::EffectNamespace;
        effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|e| {
                let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name)?;
                let param = def.schema().matte.param()?;
                def.is_image_op().then_some((e, param))
            })
            .map(|(e, param)| {
                // The wire wins over the parameter while it is there.
                if graph.source_matte(e.id) {
                    return LayerInputDraw::ThisLayer;
                }
                if let Some(from) = graph.matte_from(e.id) {
                    return LayerInputDraw::EffectOutput(from);
                }
                // A row the panel does not show, or shows greyed, is a row
                // nobody meant: the Lens flare's Matte rows only exist while
                // its Source type is Matte, and rendering the layer one names
                // in Manual mode would cost a whole pass per frame for a
                // picture the kernel ignores. One rule, every effect — the
                // flare's own named gate, generalised.
                if !lumit_core::fx::builtins::param_visible(e, param)
                    || !lumit_core::fx::builtins::param_enabled(e, param)
                {
                    return LayerInputDraw::Absent;
                }
                // "This layer": a matte pointed at the layer the
                // effect is on is the effect's own input, not a re-render —
                // on an adjustment layer, the composite below.
                if e.layer_ref(param) == Some(owner) {
                    return LayerInputDraw::ThisLayer;
                }
                layer_slot(e, param).map_or(LayerInputDraw::Absent, LayerInputDraw::Layer)
            })
            .collect()
    };

    // **The mask paths — the geometry carriage** (docs/08 §1.2).
    // One polyline per MaskPath **row** of every enabled built-in that resolves
    // to an op at all — the same two conditions `mattes_for` applies, so this
    // list stays in step with the ops `run_ops` walks, with its own counter
    // there because the predicate is a different one. Per row, not per op: the
    // Matte key declares two (an inside hold-out and an outside one), and
    // `EffectSchema::mask_paths` is the one enumeration both sides run.
    //
    // The masks are the *layer's own*: a mask belongs to the layer it is drawn
    // on, and an effect walking another layer's shape is a question nobody has
    // asked. That is why nothing is rendered here — unlike a matte, this input
    // is not a picture, and the whole point of the seam is that a coverage
    // buffer cannot say which way is *along* a curve. The layer is optional
    // so a group header's stack, which has no layer and so no masks, walks
    // the same carriage (docs/impl/group-effects.md §2): every
    // declared row still gets its slot, each an empty polyline, which is the
    // effect's documented no-op.
    let mask_paths_for = |effects: &[lumit_core::model::EffectInstance],
                          layer: Option<&lumit_core::model::Layer>,
                          slt: f64|
     -> Vec<lumit_core::mask::MaskPolyline> {
        use lumit_core::model::EffectNamespace;
        effects
            .iter()
            .filter(|e| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|e| {
                let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name)?;
                def.is_image_op().then_some((e, def))
            })
            .flat_map(|(e, def)| {
                def.schema()
                    .mask_paths()
                    .map(move |(param, self_default)| (e, param, self_default))
                    .collect::<Vec<_>>()
            })
            .map(|(e, param, self_default)| {
                // The one rule the driver walk also asks, so a wire reads
                // the path the picture draws.
                lumit_core::mask::effect_path_at(e, param, self_default, layer, slt)
            })
            .collect()
    };

    // **The birth schedules — the timing carriage** (points-stream.md §3.3).
    // One per enabled built-in that declares a Points output AND
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
    // `owner` is the layer the stack sits on — `None` for a group header's
    // stack (docs/impl/group-effects.md §2), which has no camera-projected
    // stream, no wires to bring one in, and no layer id for an expression to
    // name: its schedules still fill one slot per declared row, so the 1:1
    // walk in `run_ops` holds.
    // `room` is the empty margin a points effect is given round a Shape or
    // Text layer's art, in texels each side, and the raster's px per comp px.
    let points_schedules_for = |owner: Option<&lumit_core::model::Layer>,
                                effects: &[lumit_core::model::EffectInstance],
                                slt: f64,
                                frame_slt: f64,
                                room: Option<([u32; 2], f32)>|
     -> Vec<lumit_core::fx::points::PointsSchedule> {
        use lumit_core::model::EffectNamespace;
        let dt = 1.0 / comp.frame_rate.fps().max(1.0);
        // **The camera carriage**, worked out once for the layer: a
        // stream's third axis is only visible through the comp's camera, and
        // the camera is no more a number in the bag than the layer's clock is.
        let projection = owner.and_then(|layer| {
            points_projection(
                expr_doc,
                comp,
                layer,
                t_comp,
                slt,
                Arc::new(ExpressionContext {
                    document: expr_doc.clone(),
                    comp: Some(comp.id),
                    layer: Some(layer.id),
                    comp_time: t_comp,
                    current_depth: 0,
                    inputs: None,
                }),
            )
        });
        let carried: Vec<_> = effects
            .iter()
            .enumerate()
            .filter(|(_, e)| e.enabled && e.effect.namespace == EffectNamespace::Builtin)
            .filter_map(|(i, e)| {
                let def = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&e.effect.match_name)?;
                let wants = lumit_core::fx::points::wants_carriage_of(def) && def.is_image_op();
                wants.then_some((i, e, def))
            })
            .collect();
        // The room is for the points effects the stack ends on, and the first
        // of them is where it starts. Anything drawn after a points effect
        // was placed on the art's own picture, so the points before it keep
        // that picture and draw as they always did. Scatter and Emit from
        // image count with the rest: they throw their candidates across the
        // picture they are handed, and a wider one would move every point.
        let draws = |e: &lumit_core::model::EffectInstance| {
            e.enabled
                && (e.effect.namespace != EffectNamespace::Builtin
                    || lumit_core::fx::catalogue::BUILTIN_DEFS
                        .get(&e.effect.match_name)
                        .is_none_or(|def| def.is_image_op()))
        };
        let asks = |i: usize, e: &lumit_core::model::EffectInstance| {
            carried.iter().any(|(c, ..)| *c == i)
                && !matches!(e.effect.match_name.as_str(), "scatter" | "emit_from_image")
        };
        let last_other = effects
            .iter()
            .enumerate()
            .rev()
            .find(|(i, e)| draws(e) && !asks(*i, e))
            .map(|(i, _)| i);
        let room_from = room.and_then(|_| {
            carried
                .iter()
                .position(|(i, ..)| last_other.is_none_or(|other| *i > other))
        });
        // Which slot of this list an effect holds, by its place in the stack.
        // A wire whose stream could not be made here names its producer's
        // slot, so the walk can ask that op for it on the card.
        let late = |wired: &PointsInput| -> Vec<(u32, u32)> {
            wired
                .from
                .iter()
                .enumerate()
                .filter(|(k, _)| wired.streams.get(*k).is_none_or(|s| s.is_empty()))
                .filter_map(|(k, from)| {
                    let from = (*from)? as usize;
                    let slot = carried.iter().position(|(i, ..)| *i == from)?;
                    Some((u32::try_from(k).ok()?, u32::try_from(slot).ok()?))
                })
                .collect()
        };
        carried
            .iter()
            .enumerate()
            .map(|(k, &(_, e, def))| {
                // A pinned effect is evaluated at the true playhead, so
                // its schedule is scanned there too: the picture and the
                // particles it draws must be of one moment.
                let t = if e.sample_temporally { slt } else { frame_slt };
                let context = Arc::new(ExpressionContext {
                    document: expr_doc.clone(),
                    comp: Some(comp.id),
                    layer: owner.map(|l| l.id),
                    comp_time: t_comp,
                    current_depth: 0,
                    inputs: None,
                });
                // **The stream a wire brings in**: nothing at all for a
                // producer, and for a consumer the producer's own stream
                // evaluated by the very function the driver walk reads
                // (points-stream.md §3.3). In px@comp — the units a stream is
                // data in — and rescaled into the raster by whichever
                // consumer draws it.
                let drive_projection =
                    projection.unwrap_or(lumit_core::fx::points::Projection::FLAT);
                let mut wired = match owner {
                    Some(layer) => points_input_for(
                        layer,
                        e,
                        def,
                        t,
                        dt,
                        projection.unwrap_or(lumit_core::fx::points::Projection::FLAT),
                        &context,
                        &audio,
                        clone_number,
                    ),
                    // A group carries no graph, so no wire can bring a stream
                    // in: the consumer's documented empty input.
                    None => PointsInput::default(),
                };
                // The room this op's picture has round the art. The points
                // are drawn that far in, which is one more move on the end
                // of the camera's. A stream keeps its own positions, so a
                // driver and another layer read what they always did.
                let grow = room
                    .filter(|_| room_from.is_some_and(|from| k >= from))
                    .map_or([0, 0], |(texels, _)| texels);
                let scale = room.map_or(1.0, |(_, scale)| scale.max(1e-6));
                let inset = [grow[0] as f32 / scale, grow[1] as f32 / scale];
                let moved = |p: lumit_core::fx::points::Projection| {
                    let [mut x, mut y, w] = p.m;
                    for ((x, y), w) in x.iter_mut().zip(y.iter_mut()).zip(w) {
                        *x += inset[0] * w;
                        *y += inset[1] * w;
                    }
                    lumit_core::fx::points::Projection { m: [x, y, w] }
                };
                let projection = if grow == [0, 0] {
                    projection
                } else {
                    for stream in &mut wired.streams {
                        stream.projection = moved(stream.projection);
                    }
                    Some(moved(projection.unwrap_or_default()))
                };
                // The Text layer that Text to points and Label points read.
                let (text, label) = owner.map_or_else(Default::default, |layer| {
                    text_carriage(comp, layer, e, t_comp, expr_doc)
                });
                let late = late(&wired);
                // SICELIOT: a stream made on the card gets its drivers when
                // it arrives, as one made here got them in points_input_for.
                let late_drive = owner
                    .filter(|_| late.first().is_some_and(|(entry, _)| *entry == 0))
                    .and_then(|layer| {
                        let rows = driven_rows(e)?;
                        let (graph, consumer, context) =
                            (layer.graph.clone(), e.id, context.clone());
                        let flat = drive_projection;
                        Some(lumit_core::fx::points::LateDrive(Arc::new(
                            move |stream: &mut lumit_core::fx::points::PointsStream| {
                                lumit_core::fx::effects::clone_to_points::drive_rows(
                                    &graph, consumer, stream, rows, t, &context, None, flat,
                                );
                            },
                        )))
                    });
                let input_from = wired.from.first().copied().flatten();
                let ports_from: Vec<u32> = wired
                    .from
                    .iter()
                    .skip(1)
                    .map(|f| f.unwrap_or(u32::MAX))
                    .collect();
                let input = wired.streams;
                // **A generator has no births to schedule**: its points
                // are arithmetic over its own parameters, and what it wants
                // from this carriage is the camera and the clock. A *consumer*
                // has none either, for the plainer reason that it makes no
                // points at all. Asked by the schema rather than by name, so a
                // producer that grows an Emit rate later needs no edit here.
                if !def.schema().params.iter().any(|p| p.id == "emit_rate") {
                    return lumit_core::fx::points::PointsSchedule {
                        schedule: lumit_core::fx::points::Schedule::default(),
                        t,
                        projection,
                        late,
                        late_drive: late_drive.clone(),
                        ports_from,
                        input,
                        input_from,
                        text,
                        label,
                        grow,
                        stepped: None,
                        trails: None,
                    };
                }
                // **A Particulate with Trails on** draws every frame from the
                // in point on the card, each from the driver walk at its own
                // time, so there is nothing to scan here.
                if let Some(layer) =
                    owner.filter(|_| lumit_core::fx::points::stepped::trails(comp, e))
                {
                    use lumit_core::fx::points::stepped::{definition, target};
                    return lumit_core::fx::points::PointsSchedule {
                        schedule: lumit_core::fx::points::Schedule::default(),
                        t,
                        projection,
                        late,
                        late_drive: late_drive.clone(),
                        ports_from,
                        input,
                        input_from,
                        text,
                        label,
                        grow,
                        stepped: None,
                        trails: Some(lumit_core::fx::points::Trails {
                            key: definition(expr_doc, comp, layer, e, target(comp, e)),
                            doc: expr_doc.clone(),
                            comp: comp.id,
                            layer: layer.id,
                            clone: clone_number,
                            inset,
                        }),
                    };
                }
                // **A Particulate that walks does it on the host**, through
                // the driver walk a wire reads, and the card only draws where
                // it got to.
                let walked = owner
                    .filter(|_| lumit_core::fx::points::stepped::wanted(comp, e))
                    .map(|layer| {
                        lumit_core::fx::drivers::effect_stream_in(
                            &layer.graph,
                            e.id,
                            t,
                            context.clone(),
                            Some(&audio),
                            lumit_core::fx::points::Projection::FLAT,
                            None,
                            clone_number,
                        )
                        .unwrap_or_default()
                    });
                if walked.is_some() {
                    return lumit_core::fx::points::PointsSchedule {
                        schedule: lumit_core::fx::points::Schedule::default(),
                        t,
                        projection,
                        late,
                        late_drive: late_drive.clone(),
                        ports_from,
                        input,
                        input_from,
                        text,
                        label,
                        grow,
                        stepped: walked,
                        trails: None,
                    };
                }
                let schedule = birth_schedule(e, t, dt, &context, owner.map(|l| &l.graph), &audio);
                lumit_core::fx::points::PointsSchedule {
                    schedule,
                    t,
                    projection,
                    late,
                    late_drive,
                    ports_from,
                    input,
                    input_from,
                    text,
                    label,
                    grow,
                    stepped: None,
                    trails: None,
                }
            })
            .collect()
    };

    // **A node graph composition** (§2.3). Its whole draw list is one draw at
    // identity placement, so the Precomp layer arm, the matte arm and the
    // layer-input arm all reach it through the calls they already make. A node
    // graph has no layers, so nothing below this would draw anything anyway.
    if let Some(graph) = &comp.graph {
        // Viewed on its own: a picture Input reads its preview item or
        // transparent, and a value Input its own default (§1.5, §5.11).
        // A placed graph is handed its host's own Input values instead (§5.3).
        return vec![graph_comp_draw(
            &lower,
            comp,
            graph,
            lumit_core::comp_graph::GraphView {
                values: with.values,
                pictures_fed: false,
            },
            &visited_path,
        )];
    }

    // **Live groups** (docs/impl/group-effects.md §2): a group whose
    // header stack has an enabled instance and whose drawn run is non-empty
    // renders as an implicit per-frame precompose — the run's draws collected
    // and wrapped in one comp-sized Nested draw carrying the header's stack.
    // This is the only reading of `Composition.groups` the render ever makes,
    // so on every frame where no header is live the walk stays group-blind
    // and the byte-identical promise holds.
    let stack_ids: Vec<uuid::Uuid> = comp.layers.iter().map(|l| l.id).collect();
    let mut live_spans: Vec<(usize, usize, &lumit_core::group::LayerGroup)> = Vec::new();
    for g in &comp.groups {
        if !lumit_core::group::header_live(g) {
            continue;
        }
        let run = lumit_core::group::drawn_members(&stack_ids, g);
        let Some(first) = run.first() else {
            continue;
        };
        let Some(start) = stack_ids.iter().position(|id| id == first) else {
            continue;
        };
        let end = start + run.len();
        // A layer sits in one group's run (`group_of`'s first-wins rule); a
        // hand-edited overlap keeps the first span and reads the loser as the
        // ungrouped layers it looks like — degrade, not fault.
        if live_spans.iter().any(|(s, e, _)| start < *e && *s < end) {
            continue;
        }
        live_spans.push((start, end, g));
    }
    // A group carries no driver graph (§1), so its matte carriage resolves
    // against an empty one.
    let group_graph = lumit_core::graph::LayerGraph::default();
    // Close a live group's unit: everything the walk collected past `mark`
    // becomes the `draws` of one comp-sized Nested draw — the Precomp layer's
    // own render path from the first line to the last — whose `fx` is the
    // header's stack resolved at comp time. If every member was gated out
    // (eyes off, solo elsewhere), or the stack resolves to no op at all, the
    // group contributes nothing / the members stay as they were: no members,
    // no act, and no op, no isolation.
    let wrap_unit =
        |draws: &mut Vec<CompLayerDraw>, mark: usize, group: &lumit_core::group::LayerGroup| {
            if draws.len() <= mark {
                return;
            }
            let context = Arc::new(ExpressionContext {
                document: expr_doc.clone(),
                comp: Some(comp.id),
                layer: None,
                comp_time: t_comp,
                current_depth: 0,
                inputs: None,
            });
            let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
            // Resolve time is comp time (§1): a group has no in point, start
            // offset or Retime, so there is no layer clock to convert through,
            // and its marker context is the comp's own.
            let markers = lumit_core::fx::markers::MarkerContext::for_comp(comp);
            let (fx_ids, fx) = lumit_core::fx::resolved::resolve_stack_temporal_named(
                &group.effects,
                lumit_core::fx::drivers::ResolvedDrivers::NONE,
                t_comp,
                frame_t,
                comp_diag,
                1.0,
                &markers,
                context,
            );
            if fx.is_empty() {
                // Enabled instances that all resolve to nothing (a Posterize
                // Time alone, an unknown plugin): no op would run, so wrapping
                // would isolate the members' blends for no picture gain.
                return;
            }
            let members = draws.split_off(mark);
            let (w, h) = (comp.width as f32, comp.height as f32);
            draws.push(CompLayerDraw {
                layer: group.id,
                source: DrawSource::Nested {
                    width: comp.width,
                    height: comp.height,
                    // Transparent where nothing covers it, exactly as
                    // a Precomp's intermediate is.
                    background: [0.0, 0.0, 0.0, 0.0],
                    draws: members,
                    // The comp's own active camera, so 3D members land exactly
                    // where they did; the finished slab is one 2D picture.
                    camera: Some(crate::track::camera_pose(doc, comp, t_comp)),
                    // Uncached in v1 (§4): the stance the adjustment
                    // path takes for its below-composite.
                    key: None,
                    paint: Vec::new(),
                    paint_time: t_comp,
                },
                natural_size: (w, h),
                // Identity placement: comp-sized, drawn where the members
                // always landed.
                position: (w * 0.5, h * 0.5),
                anchor: (w * 0.5, h * 0.5),
                scale: (100.0, 100.0),
                rotation_deg: 0.0,
                // Pinned 100/Normal (the owner's answer in the note's Open
                // questions): effects only, no group opacity or blend dial.
                opacity: 100.0,
                z: 0.0,
                rotation_x_deg: 0.0,
                rotation_y_deg: 0.0,
                three_d: false,
                ends_run: false,
                matte: None,
                blend: lumit_gpu::composite::Blend::Normal,
                mask_cov: None,
                pre: None,
                fx,
                fx_ids,
                neighbours: Vec::new(),
                // Temporal/flow effects on a header are inert or pass through
                // in v1 (§3, the documented degrade): nothing builds
                // neighbours or flow for the unit.
                flow_fields: Vec::new(),
                colour_tables: colour_tables(&group.effects, t_comp),
                dof_inputs: dof_inputs_for(group.id, &group.effects, None),
                mattes: mattes_for(group.id, &group.effects, &group_graph),
                mask_paths: mask_paths_for(&group.effects, None, t_comp),
                // No source frames of a group's own, so nothing was ever
                // propagated: a Roto brush on a header passes through.
                roto_mattes: roto_mattes_for(&group.effects, 0),
                planes: planes_for(&group.effects, 0),
                points_schedules: points_schedules_for(None, &group.effects, t_comp, frame_t, None),
                flare_lens_files: flare_lens_files(&group.effects, t_comp),
                // A Node graph effect on a group header applies to the
                // wrapped unit, exactly as any other of its effects does.
                graph_fx: graph_fx_for(None, &group.effects, t_comp, frame_t),
                // The stack resolved at comp scale but runs on the render
                // target — realise rescales, exactly the adjustment
                // arm's setting.
                fx_ref_width: Some(comp.width as f32),
                fx_input_key: None,
                mb: Vec::new(),
                // The members were each already lit; shading the slab would
                // light them twice.
                lights: Vec::new(),
                material: Default::default(),
                temporal_below: None,
                accumulation_below: None,
                flow_below: Vec::new(),
            });
        };
    // The open unit while the walk is inside a live span: the span's start
    // index, its group, and where in `draws` its members began.
    let mut open_unit: Option<(usize, &lumit_core::group::LayerGroup, usize)> = None;

    // Solo / isolate: while any layer is soloed, only soloed layers
    // render — computed once for the whole comp.
    let any_solo = lumit_core::model::any_picture_solo(comp);
    let mut draws: Vec<CompLayerDraw> = Vec::new();
    for (idx, layer) in comp.layers.iter().enumerate().rev() {
        // A frame nobody wants any more stops here, and the renderer throws
        // the half-built list away (see `crate::cancel`).
        if crate::cancel::requested() {
            break;
        }
        // The walk is bottom-up, so a unit opens at its span's bottom row and
        // closes the moment the walk moves above its top — checked before any
        // of this row's own gates, so a skipped row still closes a unit it
        // does not belong to.
        if let Some((start, group, mark)) = open_unit {
            if idx < start {
                open_unit = None;
                wrap_unit(&mut draws, mark, group);
            }
        }
        if open_unit.is_none() {
            if let Some((start, _, g)) = live_spans
                .iter()
                .find(|(s, e, _)| (*s..*e).contains(&idx))
                .copied()
            {
                open_unit = Some((start, g, draws.len()));
            }
        }
        let context = Arc::new(ExpressionContext {
            document: expr_doc.clone(),
            comp: Some(comp.id),
            layer: Some(layer.id),
            comp_time: t_comp,
            current_depth: 0,
            inputs: None,
        });

        // An Audio layer draws nothing at all — no source, no solid, no
        // effects on an empty canvas. The mixer has already taken what it needs.
        if layer.audio_only {
            continue;
        }
        // A layer whose Layer out is unplugged draws nothing, which is
        // the same nothing a hidden layer draws and is skipped in the same place.
        if !layer.switches.visible
            || layer.graph.out_unwired
            || !in_span(layer)
            || (any_solo && !layer.switches.solo)
        {
            continue;
        }
        // Under a full-frame opaque layer: never seen, so never built.
        if occluder.is_some_and(|o| idx > o) {
            continue;
        }
        let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
        // A Sequence layer wears the effects and blend mode of the clip it is
        // showing, so everything below reads the one stack.
        let worn = lumit_core::sequence::worn_at(layer, lt);
        let layer = worn.as_ref().unwrap_or(layer);
        // The true frame time in this layer's own time base, for effects a
        // held/sub-frame re-render must not re-sample (docs/impl/
        // temporal-rerender.md §5). Equal to `lt` on an ordinary render.
        let frame_lt = lumit_core::time::layer_time(frame_t, layer.start_offset.0);
        // Which frame of the **file** this layer is showing. The plan
        // worked it out to decide what to decode, the decode carried it here,
        // and a Roto brush's matte is indexed by it — the document holds no
        // frame rate for a media item, so this is the only place the answer is
        // in hand. Zero for a layer with no decoded source, which has no
        // propagated matte to find either.
        let source_frame = analysed_frame(doc, layer, lt, pixels_by_layer);
        // This layer's effects (docs/08 §3.25): a Posterize time scoped to *this
        // layer* holds this layer's OWN effect stack on the coarse grid — its
        // effects sample the held time while the transform and source below stay
        // live. Fed to resolve_stack_temporal as the *sample* time, so a
        // sample_temporally == false effect still holds at the true playhead
        // (§5); equal to `lt` when the stack has no this-layer Posterize.
        let effect_lt = lumit_core::fx::temporal::this_layer_effect_time(
            &layer.effects,
            layer.switches.fx,
            lt,
            layer.start_offset.0,
        );
        let tr = &layer.transform;
        // The neighbour pictures a flow-consuming effect on this layer measures
        // against (docs/08 §3.2). Filled in by the two kinds whose
        // picture the decode worker cannot measure — a Precomp here, an
        // adjustment in its own arm — and left empty by every other, which is
        // every layer that already carries a decoded field.
        let mut flow_below = Vec::new();

        // The layer's own Matte, built before the kind match so that
        // **every** arm can use it — the adjustment arm pushes and `continue`s,
        // and while this lived below the match an adjustment layer's Matte
        // dropdown chose a source that gated nothing at all.
        let matte = layer.matte.as_ref().and_then(|mr| {
            let src = comp.layers.iter().find(|l| l.id == mr.layer)?;
            // A Precomp matte renders its comp: a comp has no pixels
            // until it is rendered, so `pixels_for` gives up on one and the
            // matte silently gated nothing — a layer set to a precomp matte
            // simply vanished. The nested render stands in for the source
            // texture; the source-mode toggles below do not apply to it,
            // because a comp already carries its layers' own masks and
            // effects (the layer-input boundary, unchanged).
            let nested = in_span(src).then(|| nested_comp_draw(src)).flatten();
            // Matte source mode. None reads the source's raw pixels —
            // clear its masks so `pixels_for` skips them; Masks and Effects and
            // masks keep them.
            let (m_rgba, m_w, m_h, m_nat, m_fmt) = if let Some(n) = &nested {
                (
                    Arc::new(Vec::new()),
                    n.width,
                    n.height,
                    (n.width as f32, n.height as f32),
                    // A comp is realised on the card; there are no source
                    // bytes here.
                    lumit_media::decode::PixelFormat::Srgb8,
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
            // Effects and masks matte: resolve the matte source's own
            // stack at its layer time so gpu.rs runs it on the matte texture
            // before the matte gates the consumer. Uses the source's decode scale
            // (its px@comp radii stay honest under reduced-res preview), the same
            // §1.4 markers and the same resolve export uses. Empty for
            // None / Masks or when the source's fx switch is off.
            let (fx, colour_tables) = if nested.is_some() {
                // The nested render already ran every layer's own stack.
                Default::default()
            } else if mr.source.folds_effects() && src.switches.fx {
                let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let scale = m_w as f32 / m_nat.0.max(1.0);

                let markers = lumit_core::fx::markers::MarkerContext::for_layer(comp, src);
                let drivers = drivers_for(src, mlt, context.clone());
                (
                    lumit_core::fx::resolved::resolve_stack_temporal_named(
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
                    colour_tables(&src.effects, mlt),
                )
            } else {
                Default::default()
            };
            // The matte source's own baked pictures, on the same two
            // predicates and the same (instance, source frame) lookup the
            // layer-input path uses. Empty when its stack is not being folded
            // in, which is when there are no ops to carry them to.
            let matte_frame = analysed_frame(doc, src, mlt, pixels_by_layer);
            let (matte_roto, matte_planes) = if fx.is_empty() {
                (Vec::new(), Vec::new())
            } else {
                (
                    roto_mattes_for(&src.effects, matte_frame),
                    planes_for(&src.effects, matte_frame),
                )
            };
            let placed = drawn(src, mlt, &context);
            Some(MatteDraw {
                rgba: m_rgba.to_vec(),
                tex_w: m_w,
                tex_h: m_h,
                format: m_fmt,
                natural_size: m_nat,
                position: placed.position,
                anchor: placed.anchor,
                scale: placed.scale,
                rotation_deg: placed.rotation_deg,
                opacity: mtr.opacity.value_at_with_context(mlt, context.clone()) as f32,
                z: placed.z,
                rotation_x_deg: placed.rotation_x_deg,
                rotation_y_deg: placed.rotation_y_deg,
                three_d: src.switches.three_d,
                pre: parent_world_placement(comp, src, t_comp, context.clone()),
                luma: matches!(mr.channel, lumit_core::model::MatteChannel::Luma),
                inverted: mr.inverted,
                colour_tables,
                // The graphs the matte source's own Node graph effects apply,
                // as a layer read through an effect's row carries them. Their
                // further pictures are fetched plainly, which is the same bound.
                graph_fx: if fx.is_empty() {
                    Vec::new()
                } else {
                    graph_fx_with(Some(src), &src.effects, mlt, mlt, &layer_slot_plain)
                },
                fx,
                roto_mattes: matte_roto,
                planes: matte_planes,
                nested,
                colour_space: crate::colour::footage_colour_space(doc, &src.kind),
            })
        });

        // What a Sequence layer shows where a composition is one of the clips
        // showing. `None` at every other moment, where the decoded pixels
        // are the layer's source as they always were.
        // A layer with a backdrop effect is drawn as an adjustment is: over
        // the composite below, its own picture handed to that effect instead.
        let adjusts = layer.is_adjustment() || backdrop_fx(layer);
        let comp_clips = match &layer.kind {
            LayerKind::Sequence { clips } if !adjusts => {
                use lumit_core::sequence::{Clip, ClipSource, Shown};
                let plays = |c: &Clip| matches!(c.source, ClipSource::Comp(_));
                let shown = lumit_core::sequence::shown_at(clips, lt);
                match shown {
                    Shown::One { clip, .. } if plays(clip) => Some(shown),
                    Shown::Two {
                        outgoing, incoming, ..
                    } if plays(outgoing) || plays(incoming) => Some(shown),
                    _ => None,
                }
            }
            _ => None,
        };

        // How much of the layer's opacity a composition clip inside a fade is
        // showing. One everywhere else.
        let mut clip_share = 1.0f32;
        let (source, natural) = match &layer.kind {
            // **A Sequence clip that plays a composition** draws that comp
            // as a Precomp layer draws its own: walked at the moment the clip
            // shows, named by its own frame key, sized as the comp is, and
            // behind the same guard against a comp inside itself.
            //
            // A dissolve with such a clip is mixed on the card, since a comp
            // has no pixels until it is rendered: the two clips become the
            // draws of one more Nested, each at its share, the second laid
            // on the first with `Blend::Plus`. That is the crossfade the
            // decode worker makes of two footage clips
            // (`lumit_core::pixels::dissolve_rgba`), and two whole pictures
            // sum to a whole one.
            //
            // ponytail: a fade at a lone end is the clip's share of the
            // layer's opacity (`clip_share`), so it comes after the layer's
            // effects, where a footage clip fades before them. A nested
            // picture is read back as straight alpha, so one faded ahead of
            // the stack would be dimmed twice, as anything part transparent
            // inside a Precomp is. The upgrade is a nested picture read as
            // the premultiplied one it is.
            //
            // ponytail: the frame's own picture only. Such a clip read as a
            // matte or as an effect's layer input, and a temporal effect's
            // neighbours on this layer, do not see the comp. The upgrade is
            // the roads a Precomp layer has for each.
            LayerKind::Sequence { .. } if comp_clips.is_some() => {
                use lumit_core::sequence::{Clip, ClipSource, Shown};
                // One clip's picture and its natural size: its comp, or the
                // footage the worker decoded for this layer.
                let mut picture = |clip: &Clip| -> Option<(DrawSource, (f32, f32))> {
                    let nested_id = match clip.source {
                        ClipSource::Footage(_) => {
                            let lp = pixels_by_layer.get(&layer.id)?;
                            return Some((
                                DrawSource::Pixels {
                                    rgba: Arc::clone(&lp.rgba),
                                    tex_w: lp.width,
                                    tex_h: lp.height,
                                    format: lp.format,
                                    colour_space: None,
                                },
                                (lp.natural_w as f32, lp.natural_h as f32),
                            ));
                        }
                        ClipSource::Comp(id) => id,
                    };
                    let nested = doc
                        .comp(nested_id)
                        .filter(|_| !visited.contains(&nested_id))?;
                    let st = clip.nested_time(nested, lt);
                    visited.push(nested_id);
                    let nested_draws = comp_walk(
                        doc,
                        nested,
                        st,
                        clip.nested_time(nested, frame_lt),
                        pixels_by_layer,
                        visited,
                        keys,
                        false,
                        inner,
                        None,
                    );
                    visited.pop();
                    Some((
                        DrawSource::Nested {
                            width: nested.width,
                            height: nested.height,
                            background: [0.0, 0.0, 0.0, 0.0],
                            draws: nested_draws,
                            camera: Some(crate::track::camera_pose(doc, nested, st)),
                            key: keys.and_then(|k| k.nested_key_with(nested, st, None)),
                            paint: Vec::new(),
                            paint_time: lt,
                        },
                        (nested.width as f32, nested.height as f32),
                    ))
                };
                // The clips showing, each with its share of the picture.
                let (under, top) = match comp_clips {
                    Some(Shown::One { clip, gain }) => (None, picture(clip).map(|p| (p, gain))),
                    Some(Shown::Two {
                        outgoing,
                        incoming,
                        gain,
                    }) => (
                        picture(outgoing).map(|p| (p, 1.0 - gain)),
                        picture(incoming).map(|p| (p, gain)),
                    ),
                    _ => continue,
                };
                let (mut source, natural) = match (under, top) {
                    (None, None) => continue,
                    // One picture: it is the layer's source as it stands,
                    // and a fade is taken where the layer is composited.
                    (Some(((source, size), share)), None)
                    | (None, Some(((source, size), share))) => {
                        clip_share = share as f32;
                        (source, size)
                    }
                    // A dissolve: both, each at its share and fitted about
                    // the centre of the incoming clip's size, which is the
                    // layer's across the overlap as it is where two footage
                    // clips are mixed.
                    (Some((going, away)), Some((coming, here))) => {
                        let natural = coming.1;
                        let part = |(source, size): (DrawSource, (f32, f32)), share: f64, blend| {
                            let mut draw = read_draw(layer, source, size);
                            draw.position = (natural.0 * 0.5, natural.1 * 0.5);
                            let fit =
                                (natural.0 / size.0.max(1.0)).min(natural.1 / size.1.max(1.0));
                            draw.scale = (fit * 100.0, fit * 100.0);
                            draw.opacity = (share * 100.0) as f32;
                            draw.blend = blend;
                            draw
                        };
                        let mixed = DrawSource::Nested {
                            width: natural.0 as u32,
                            height: natural.1 as u32,
                            background: [0.0, 0.0, 0.0, 0.0],
                            draws: vec![
                                part(going, away, lumit_gpu::composite::Blend::Normal),
                                part(coming, here, lumit_gpu::composite::Blend::Plus),
                            ],
                            camera: None,
                            // A mix has no name of its own. Each comp
                            // inside it keeps the one it was given above.
                            key: None,
                            paint: Vec::new(),
                            paint_time: lt,
                        };
                        (mixed, natural)
                    }
                };
                // Paint on the layer is stamped into the picture once it is
                // rendered, as a Precomp layer's is.
                if let DrawSource::Nested { paint, .. } = &mut source {
                    paint.clone_from(&layer.paint);
                }
                (source, natural)
            }
            // Guarded so a Precomp acting as an adjustment falls
            // through to the adjustment arm below rather than drawing its comp.
            LayerKind::Precomp { comp: nested_id } if !adjusts => {
                if visited.contains(nested_id) {
                    continue; // cycle guard
                }
                let Some(nested) = doc.comp(*nested_id) else {
                    continue;
                };
                // **The Retime map** (§5.6): which moment of the nested comp
                // this layer shows. Every site that evaluates the comp reads
                // `st`; the layer's own opacity, paint and collapse keep `lt`,
                // because they are not remapped (docs/04 §11.6). Equal to `lt`
                // for a layer with no map, which is byte for byte what it was.
                let st = lumit_core::model::nested_source_time(layer, nested, lt);
                let frame_st = lumit_core::model::nested_source_time(layer, nested, frame_lt);
                // Collapse (docs/06 §1.4): splice the inner layers straight
                // into this list with the Precomp layer's placement multiplied
                // in front — no intermediate raster, no clipping to the nested
                // bounds, inner blend modes composite against the parent stack.
                if matches!(
                    lumit_core::model::collapse_state(doc, comp, layer, lt),
                    lumit_core::model::CollapseState::Active
                ) {
                    visited.push(*nested_id);
                    let mut spliced_in = comp_walk(
                        doc,
                        nested,
                        st,
                        frame_st,
                        pixels_by_layer,
                        visited,
                        keys,
                        true,
                        inner,
                        None,
                    );
                    visited.pop();

                    // A 2D collapsed Precomp is placed flat too.
                    let own = own_placement(layer, t_comp, &context);
                    // If the collapsed precomp is itself parented, its parent's
                    // world placement wraps its own before it wraps the inner
                    // draws.

                    let parent = match parent_world_placement(comp, layer, t_comp, context.clone())
                    {
                        Some(pw) => lumit_gpu::composite::concat_place(pw, own),
                        None => own,
                    };
                    for d in &mut spliced_in {
                        // A 2D layer inside a collapsed 3D precomp lies flat on
                        // the precomp's plane, so the camera sees it there,
                        // and it still ends a run of 3D layers.
                        if layer.switches.three_d && !d.three_d {
                            d.three_d = true;
                            d.ends_run = true;
                            d.z = 0.0;
                            d.rotation_x_deg = 0.0;
                            d.rotation_y_deg = 0.0;
                        }
                        d.pre = Some(match d.pre {
                            // A collapsed chain: this parent wraps the child's
                            // own parent placement.
                            Some(p) => lumit_gpu::composite::concat_place(parent, p),
                            None => parent,
                        });
                        // Per-layer motion blur on an inner layer of a collapsed
                        // Precomp is a follow-up (docs/06 §4): the export
                        // splice (collect_collapsed) carries no sub-frame
                        // samples, so clearing them here keeps preview and export
                        // identical. A non-collapsed Precomp layer still
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
                        // And the flow neighbours (§3.2), for the same
                        // sizing reason: they were built at the nested comp's
                        // raster, so splicing them into the parent would
                        // measure motion against a mis-sized picture.
                        d.flow_below = Vec::new();
                    }
                    draws.extend(spliced_in);
                    continue;
                }
                visited.push(*nested_id);
                // A placed node graph with its own Input values (§5.3) is
                // lowered under them rather than under its defaults, by the
                // nested comp's own walk.
                let values = nested
                    .graph
                    .as_ref()
                    .filter(|_| layer.graph_inputs.is_some())
                    .map_or_else(Vec::new, |graph| graph_values(layer, graph, st));
                // In a rebuild the frame keeps its name when it is the picture
                // its own frame would show, so the frame after this one finds
                // it made already.
                let named = rebuild_keys.filter(|_| {
                    !with.rebuild
                        || (plain_stack(layer, lt)
                            && rebuilt_as_itself(doc, nested, st, frame_st, pixels_by_layer, 0))
                });
                let nested_draws = comp_walk(
                    doc,
                    nested,
                    st,
                    frame_st,
                    pixels_by_layer,
                    visited,
                    named,
                    false,
                    Walk {
                        values: &values,
                        ..inner
                    },
                    None,
                );
                // The nested picture again at each neighbour time (docs/08
                // §3.2), for a temporal effect on the Precomp layer itself:
                // a comp has no decoded frames for the worker to hand over or
                // measure between, but it can be built again at
                // another moment. The offset steps by the *parent* comp's
                // frame, because that is the shutter the effect smears over.
                // No frame keyer and the temporal inputs stripped, exactly as
                // `below_draws_at` does and for the same reason: a neighbour
                // carries no neighbours of its own, so this is not the
                // picture that name would claim. Footage inside is drawn at
                // the neighbour's own moment (`pixels_at_moment`).
                // Empty unless one of those effects is live.
                let dt = 1.0 / comp.frame_rate.fps().max(1.0);
                let nested_at = |offset: i32| {
                    lumit_core::model::nested_source_time(
                        layer,
                        nested,
                        lumit_core::time::layer_time(
                            t_comp + f64::from(offset) * dt,
                            layer.start_offset.0,
                        ),
                    )
                };
                let crosses =
                    |o| lumit_core::fx::temporal::cut_between(&nested.layers, st, nested_at(o));
                flow_below = crate::plan::rebuild_offsets(layer, lt, dt, crosses)
                    .into_iter()
                    // None inside a walk that is itself a neighbour.
                    .filter(|_| !with.rebuild)
                    .map(|(offset, measure)| {
                        // The neighbour's own layer time, then the Retime
                        // map (§5.6): a retimed Precomp measures its
                        // motion between the moments it actually shows.
                        let nt = nested_at(offset);
                        // Under the same Input values the frame itself is
                        // built under, read at the neighbour's moment. On
                        // the graph's defaults the neighbour was another
                        // picture, and the difference read as motion.
                        let values = nested
                            .graph
                            .as_ref()
                            .filter(|_| layer.graph_inputs.is_some())
                            .map_or_else(Vec::new, |graph| graph_values(layer, graph, nt));
                        let mut neighbour = comp_walk(
                            doc,
                            nested,
                            nt,
                            frame_st,
                            pixels_by_layer,
                            visited,
                            None,
                            false,
                            Walk {
                                rebuild: true,
                                values: &values,
                                ..inner
                            },
                            None,
                        );
                        strip_temporal_inputs(&mut neighbour);
                        (
                            offset,
                            neighbour,
                            Some(crate::track::camera_pose(doc, nested, nt)),
                            measure,
                        )
                    })
                    .collect();
                visited.pop();
                (
                    DrawSource::Nested {
                        width: nested.width,
                        height: nested.height,
                        // A nested comp's intermediate is transparent where
                        // nothing covers it. A comp's background colour
                        // is a viewing backdrop for the comp being looked at,
                        // not a layer of its own, so filling the intermediate
                        // with it would turn every gap in a Precomp into opaque
                        // black and hide the parent's stack behind it.
                        background: [0.0, 0.0, 0.0, 0.0],
                        draws: nested_draws,
                        camera: Some(crate::track::camera_pose(doc, nested, st)),
                        // The nested frame's own name. `lt` is on the
                        // flick grid already (`layer_time`), so a Precomp
                        // layer moved by whole frames keeps its names, and a
                        // placed graph's own Input values are folded in so
                        // two hosts of one graph name two pictures (§5.3).
                        key: named.filter(|_| !driven_inputs(layer)).and_then(|k| {
                            k.nested_key_with(nested, st, layer.graph_inputs.as_ref())
                        }),
                        // Paint on a Precomp: stamped into the nested
                        // picture by the realiser, because a comp has no
                        // pixels until it is rendered. Collapse is already
                        // forced off by any paint (`collapse_state`), so the
                        // strokes always have an intermediate to land in.
                        paint: layer.paint.clone(),
                        // The layer's own clock, so a keyed Start/End on a
                        // Precomp's paint reads where every other keyed value
                        // on that layer reads.
                        paint_time: lt,
                    },
                    (nested.width as f32, nested.height as f32),
                )
            }
            // **The one adjustment path**. Reached both by the layer
            // kind *New adjustment layer* makes and by any drawing layer with
            // the adjustment switch on: either way the layer's own source is
            // set aside, so there is one behaviour here and no second copy of
            // it to drift.
            _ if adjusts => {
                // A staging point, not a picture (docs/06 §1.5): realise
                // composites everything below, runs this stack on it, and
                // blends back by coverage — masks × opacity, placed by the
                // transform. A dead stack contributes nothing at all.
                let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
                let (fx_ids, fx) = if layer.switches.fx {
                    // The §1.4 marker context, built by the same shared
                    // constructor export uses. Effects flagged
                    // sample_temporally == false resolve at the frame time in a
                    // held re-render (§5); equal to `lt` on an ordinary render.
                    let markers = lumit_core::fx::markers::MarkerContext::for_layer(comp, layer);
                    resolve_layer_fx(
                        layer,
                        &drivers_for(layer, effect_lt, context.clone()),
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
                // `below_draws_at` export also drives. A Posterize Time
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
                    clone_number,
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
                    clone_number,
                )
                .map(|mut ab| {
                    // Its Matte, rendered by the same helper every
                    // other matte and layer input goes through. It is filled in
                    // here rather than inside `accumulation_mb_below` because
                    // that is a free function and this is where `layer_slot`
                    // lives. Pointed at the adjustment itself, the
                    // matte is the composite below — which is what an
                    // adjustment layer's own input is.
                    if let Some(e) = layer.effects.iter().find(|e| {
                        e.enabled
                            && e.effect.namespace == lumit_core::model::EffectNamespace::Builtin
                            && e.effect.match_name == "accumulation_mb"
                    }) {
                        ab.matte =
                            if e.layer_ref(lumit_core::fx::schema::MATTE_PARAM) == Some(layer.id) {
                                LayerInputDraw::ThisLayer
                            } else {
                                layer_slot(e, lumit_core::fx::schema::MATTE_PARAM)
                                    .map_or(LayerInputDraw::Absent, LayerInputDraw::Layer)
                            };
                    }
                    ab
                });
                if fx.is_empty() && temporal_below.is_none() && accumulation_below.is_none() {
                    continue;
                }

                // A layer drawn as one only for its backdrop effect covers the
                // whole comp, unmoved: its masks and transform already shaped
                // the picture that effect takes, and it is the effect, not
                // the coverage, that leaves the rest of the comp as it was.
                let native = layer.is_adjustment();
                let placed = if native {
                    drawn(layer, lt, &context)
                } else {
                    lumit_gpu::composite::MbSample {
                        position: (0.0, 0.0),
                        anchor: (0.0, 0.0),
                        scale: (100.0, 100.0),
                        rotation_deg: 0.0,
                        z: 0.0,
                        rotation_x_deg: 0.0,
                        rotation_y_deg: 0.0,
                    }
                };
                draws.push(CompLayerDraw {
                    layer: layer.id,
                    source: DrawSource::Adjust,
                    natural_size: (comp.width as f32, comp.height as f32),
                    position: placed.position,
                    anchor: placed.anchor,
                    scale: placed.scale,
                    rotation_deg: placed.rotation_deg,
                    opacity: tr.opacity.value_at_with_context(lt, context.clone()) as f32,
                    z: placed.z,
                    rotation_x_deg: placed.rotation_x_deg,
                    rotation_y_deg: placed.rotation_y_deg,
                    three_d: native && layer.switches.three_d,
                    ends_run: false,
                    // The adjustment's own Matte gates where its stack lands,
                    // exactly as a matte gates an ordinary layer's picture: it
                    // multiplies into the coverage the blend uses (realise.rs).
                    matte,
                    blend: lumit_gpu::composite::Blend::Normal,
                    mask_cov: (native && !layer.masks.is_empty()).then(|| {
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
                    pre: if native {
                        parent_world_placement(comp, layer, t_comp, context.clone())
                    } else {
                        None
                    },
                    fx,
                    fx_ids,
                    // Adjustment layers process the composite below, not
                    // footage frames — no neighbours or flow fields here.
                    neighbours: Vec::new(),
                    flow_fields: Vec::new(),
                    // Ordered file paths of the enabled built-in `lut` effects,
                    // 1:1 with the stack's `lut` ops (docs/08 §3.11);
                    // the same `lt` resolve_stack used above.
                    colour_tables: colour_tables(&layer.effects, lt),
                    // Depth inputs of the enabled built-in `dof` and
                    // `light_wrap` effects, 1:1 with the stack's
                    // layer-input-consuming ops (docs/08 §3.22, §3.28).
                    dof_inputs: dof_inputs_for(
                        layer.id,
                        &layer.effects,
                        draws
                            .iter()
                            .any(|d| d.three_d)
                            .then(|| comp.px_per_metre(t_comp)),
                    ),
                    mattes: mattes_for(layer.id, &layer.effects, &layer.graph),
                    mask_paths: mask_paths_for(&layer.effects, Some(layer), lt),
                    // An adjustment layer has no source frames of its own, so
                    // nothing was ever propagated through it: a Roto brush
                    // there passes through, honestly.
                    roto_mattes: Vec::new(),
                    planes: Vec::new(),
                    points_schedules: points_schedules_for(
                        Some(layer),
                        &layer.effects,
                        lt,
                        frame_lt,
                        None,
                    ),
                    flare_lens_files: flare_lens_files(&layer.effects, lt),
                    // A Node graph effect on an adjustment layer runs on the
                    // composite below, which is the picture it is handed.
                    graph_fx: graph_fx_for(Some(layer), &layer.effects, effect_lt, frame_lt),
                    // The adjust stack resolves at comp scale but runs on
                    // the render target — realise rescales.
                    fx_ref_width: Some(comp.width as f32),
                    // The composite below has no name of its own in v1: an
                    // adjustment's stack runs uncached.
                    fx_input_key: None,
                    // An adjustment layer is a staging point, not a picture —
                    // motion blur has no image of its own to smear (docs/06 §4).
                    mb: Vec::new(),
                    // An adjustment layer has no surface of its own — it is
                    // the composite of everything below, whose layers were
                    // each already lit. Shading it would light them twice.
                    lights: Vec::new(),
                    material: Default::default(),
                    temporal_below,
                    accumulation_below,
                    // The composite this layer's temporal effects read as
                    // their neighbours, and Motion blur or Datamosh measure
                    // their motion against (docs/08 §3.2): the
                    // below-stack again at each neighbour time. Empty unless
                    // one of those effects is live, which is the whole cost
                    // gate — nothing else on an adjustment layer builds it.
                    flow_below: if with.rebuild {
                        Vec::new()
                    } else {
                        adjustment_flow_below(
                            doc,
                            comp,
                            layer,
                            idx,
                            t_comp,
                            frame_t,
                            fetched,
                            visited,
                            clone_number,
                            keys,
                        )
                    },
                });
                continue;
            }
            _ => {
                let Some((rgba, w, h, natural, format)) = pixels_for(layer) else {
                    continue;
                };
                (
                    DrawSource::Pixels {
                        rgba,
                        tex_w: w,
                        tex_h: h,
                        format,
                        colour_space: crate::colour::footage_colour_space(doc, &layer.kind),
                    },
                    natural,
                )
            }
        };

        // Spatial units are px@comp (docs/08 §2.3); the effect runs on the
        // layer's decoded texture, so the preview factor is decode/natural, to
        // stay honest under reduced-resolution preview.
        let (fx_ids, fx) = {
            let comp_diag = ((comp.width as f32).powi(2) + (comp.height as f32).powi(2)).sqrt();
            let scale = match &source {
                DrawSource::Pixels { tex_w, .. } => *tex_w as f32 / natural.0.max(1.0),
                // Adjust never reaches here (its arm pushes and continues);
                // its stack runs on the comp-sized intermediate, factor 1.
                // A node graph comp's own draw never reaches here: its arm
                // returns the whole list before the layer walk starts.
                DrawSource::Nested { .. } | DrawSource::Adjust | DrawSource::Graph(_) => 1.0,
            };
            if layer.switches.fx {
                // scale doubles as the §2.3 preview-resolution factor:
                // raster pixels per comp pixel for px@comp parameters. The
                // §1.4 marker context comes from the same shared
                // constructor export uses. In a held/sub-frame temporal
                // re-render, an effect flagged sample_temporally == false stays
                // at the frame time `frame_lt` (§5); on an ordinary render
                // `frame_lt == lt`, so this is the plain resolve.
                let markers = lumit_core::fx::markers::MarkerContext::for_layer(comp, layer);
                resolve_layer_fx(
                    layer,
                    &drivers_for(layer, effect_lt, context.clone()),
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
        // px@comp parameters at factor 1 — the same disease on the nested arm.
        // Hand realise the width the stack was resolved against (the
        // nested comp's own width) and it rescales to whatever it renders at,
        // exactly as it does for an adjustment layer. `None` for every other
        // kind: a Pixels stack already resolved at its decode scale above.
        let fx_ref_width = match &source {
            DrawSource::Nested { width, .. } => Some(*width as f32),
            DrawSource::Pixels { .. } | DrawSource::Adjust | DrawSource::Graph(_) => None,
        };
        // The name the per-effect cache files this stack's outputs under: a
        // picture made from bytes, or a nested comp's frame, whose texture is
        // exactly what the stack runs on (`op_keys` folds in the raster size,
        // so the render scale is covered).
        let fx_input_key = match &source {
            DrawSource::Pixels { tex_w, tex_h, .. } => {
                fx_input_key(doc, layer, pixels_by_layer, lt, *tex_w, *tex_h)
            }
            DrawSource::Nested { key, .. } => *key,
            DrawSource::Adjust | DrawSource::Graph(_) => None,
        };
        // Decoded neighbour frames for a temporal effect (echo), carried from
        // the layer's decode job; empty for a plain stack, and where a
        // composition clip's picture is the source and not the decode.
        let neighbours: Vec<(i32, Vec<u8>, u32, u32)> = pixels_by_layer
            .get(&layer.id)
            .filter(|_| comp_clips.is_none())
            .map(|lp| {
                lp.temporal
                    .iter()
                    .map(|(o, rgba)| (*o, rgba.clone(), lp.width, lp.height))
                    .collect()
            })
            .unwrap_or_default();
        // The dense motion fields, one per offset a flow-consuming effect
        // asked for, carried from the same decode job (their
        // `(u, v, conf)` are at the layer's decoded size).
        let flow_fields = pixels_by_layer
            .get(&layer.id)
            .filter(|_| comp_clips.is_none())
            .map(|lp| {
                lp.flow_fields
                    .iter()
                    .map(|(offset, (u, v, conf))| {
                        (
                            *offset,
                            u.clone(),
                            v.clone(),
                            conf.clone(),
                            lp.width,
                            lp.height,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        // A Shape or Text layer's picture is only as big as its art, so the
        // points effects its stack ends on are given room round it to draw
        // in. A layer style is drawn after the stack, on the art's own
        // picture, so a layer with one keeps that picture.
        let room = match (&layer.kind, &source) {
            (
                LayerKind::Shape { .. } | LayerKind::Text { .. },
                DrawSource::Pixels { tex_w, tex_h, .. },
            ) if !layer.styles.iter().any(|s| s.enabled) => points_room(
                (comp.width as f32, comp.height as f32),
                natural,
                (*tex_w, *tex_h),
                (
                    (tr.position_x.value_at_with_context(lt, context.clone())
                        - tr.anchor_x.value_at_with_context(lt, context.clone()))
                        as f32,
                    (tr.position_y.value_at_with_context(lt, context.clone())
                        - tr.anchor_y.value_at_with_context(lt, context.clone()))
                        as f32,
                ),
            ),
            _ => None,
        };

        let placed = drawn(layer, lt, &context);
        draws.push(CompLayerDraw {
            layer: layer.id,
            source,
            natural_size: natural,
            position: placed.position,
            anchor: placed.anchor,
            scale: placed.scale,
            rotation_deg: placed.rotation_deg,
            opacity: tr.opacity.value_at_with_context(lt, context.clone()) as f32 * clip_share,
            z: placed.z,
            rotation_x_deg: placed.rotation_x_deg,
            rotation_y_deg: placed.rotation_y_deg,
            three_d: layer.switches.three_d,
            ends_run: false,
            matte,
            // A layer showing only its shadows multiplies them into what is
            // under it, so a coloured one tints it and never brightens it.
            blend: match layer.blend {
                lumit_core::model::BlendMode::Normal
                    if layer.switches.three_d
                        && layer.transform.material.accepts_shadows
                            == lumit_core::model::ShadowMode::Only =>
                {
                    lumit_gpu::composite::Blend::Multiply
                }
                b => blend_of(b),
            },
            mask_cov: match &layer.kind {
                // A comp has no pixels for a mask to be cut into, a
                // composition clip's as much as a Precomp layer's.
                LayerKind::Precomp { .. } | LayerKind::Sequence { .. }
                    if !layer.masks.is_empty()
                        && (comp_clips.is_some()
                            || matches!(layer.kind, LayerKind::Precomp { .. })) =>
                {
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
            flow_fields,
            // Ordered file paths of the enabled built-in `lut` effects, 1:1
            // with the stack's `lut` ops (docs/08 §3.11); the same `lt`
            // resolve_stack used for `fx`.
            colour_tables: colour_tables(&layer.effects, lt),
            // Depth inputs of the enabled built-in `dof` and `light_wrap`
            // effects, 1:1 with the stack's layer-input-consuming ops (docs/08
            // §3.22, §3.28); built the same way export does, so the two blur
            // identically.
            dof_inputs: dof_inputs_for(layer.id, &layer.effects, None),
            mattes: mattes_for(layer.id, &layer.effects, &layer.graph),
            mask_paths: mask_paths_for(&layer.effects, Some(layer), lt),
            roto_mattes: roto_mattes_for(&layer.effects, source_frame),
            planes: planes_for(&layer.effects, source_frame),
            points_schedules: points_schedules_for(Some(layer), &layer.effects, lt, frame_lt, room),
            flare_lens_files: flare_lens_files(&layer.effects, lt),
            // The graphs this layer's Node graph effects apply, lowered
            // (docs/impl/node-graph-comp.md §2.4); empty on every layer that
            // applies none. At `effect_lt`, so a Posterize time on this layer
            // holds the graph exactly as it holds the ops beside it (§5.2);
            // equal to `lt` on every stack that carries none.
            graph_fx: graph_fx_for(Some(layer), &layer.effects, effect_lt, frame_lt),
            fx_ref_width,
            fx_input_key,
            // Per-layer motion blur (docs/06 §4): the layer's own
            // transform sampled across the open shutter, empty unless it blurs.
            // Built the same way export does, so the two smear identically.
            mb: motion_blur_samples(comp, layer, t_comp, context.clone()),
            // The comp's lights, if this layer takes them (docs/06).
            lights: shading_lights(comp, layer, t_comp, &context),
            material: shading_material(comp, layer, t_comp),
            // Ordinary layers never carry a temporal re-render — that is an
            // adjustment-only capability in v1 (docs/08 §3.25, §3.26).
            temporal_below: None,
            accumulation_below: None,
            flow_below,
        });
    }
    // A live span reaching the top of the stack closes with the walk itself.
    if let Some((_, group, mark)) = open_unit {
        wrap_unit(&mut draws, mark, group);
    }
    // A plain solid is a flat 8 x 8 tile stretched to size, with no outline
    // for the camera's depth of field to blur, so a 3D one it blurs is drawn
    // at its own size. A collapsed precomp's are done here once placed.
    if !spliced && seen_through.dof.is_some() {
        let size = (comp.width, comp.height);
        for d in draws.iter_mut().filter(|d| d.three_d) {
            if !crate::realise::dof_leaves_sharp(seen_through, d, size) {
                at_own_size(d);
            }
        }
    }
    draws
}

/// A flat 8 x 8 tile at the layer's own size, as wide as the card takes.
fn at_own_size(d: &mut CompLayerDraw) {
    let max = crate::plan::MAX_TEXTURE_SIDE;
    let (w, h) = (
        (d.natural_size.0 as u32).min(max),
        (d.natural_size.1 as u32).min(max),
    );
    if let DrawSource::Pixels {
        rgba, tex_w, tex_h, ..
    } = &mut d.source
    {
        let flat = rgba.len() == 256 && rgba.chunks(4).all(|p| p == &rgba[..4]);
        if (*tex_w, *tex_h) == (8, 8) && (w, h) != (8, 8) && flat {
            let px = [rgba[0], rgba[1], rgba[2], rgba[3]];
            (*rgba, *tex_w, *tex_h) = (Arc::new(px_tile(&px, w, h)), w, h);
        }
    }
}

/// The content name of the picture a layer's effect stack runs on
/// ([`CompLayerDraw::fx_input_key`]): what the source is, then everything
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
/// (docs/08 §3.11), each resolved at layer time `lt` (None = unset).
/// `resolve_stack` filters on the identical `e.enabled && namespace == Builtin`
/// predicate and preserves order, and a `lut` effect always resolves to exactly
/// one op, so this list is 1:1 and in the same order as the stack's
/// `lut` ops — the alignment `run_ops` relies on to bind LUT k to op
/// k. Preview (here) and export build it the same way, so the two match.
fn colour_tables(
    effects: &[lumit_core::model::EffectInstance],
    lt: f64,
) -> Vec<Option<crate::colour::TableRequest>> {
    use crate::colour::{Edge, OcioRequest, TableRequest};
    use lumit_core::model::{EffectNamespace, EffectValue};
    let name =
        |e: &lumit_core::model::EffectInstance, id: &str| e.text(id).unwrap_or_default().to_owned();
    let inverse = |e: &lumit_core::model::EffectInstance| {
        matches!(e.param("inverse"), Some(EffectValue::Bool(true)))
    };
    effects
        .iter()
        .filter(|e| {
            e.enabled
                && e.effect.namespace == EffectNamespace::Builtin
                && crate::fxops::TABLE_EFFECTS.contains(&e.effect.match_name.as_str())
        })
        .map(|e| {
            let edge = match e.effect.match_name.as_str() {
                "lut" => {
                    return e
                        .path_at("file", lt)
                        .map(|p| TableRequest::Cube(p.to_owned()))
                }
                "ocio_file" => {
                    return e.path_at("file", lt).map(|p| {
                        TableRequest::Ocio(OcioRequest::File {
                            path: p.to_owned(),
                            inverse: inverse(e),
                        })
                    })
                }
                // Both rows at the working space is nothing to do, and no
                // slot says so more cheaply than an identity table would.
                "ocio_colour_space" => {
                    let (from, to) = (
                        name(e, "input_colour_space"),
                        name(e, "output_colour_space"),
                    );
                    if from.is_empty() && to.is_empty() {
                        return None;
                    }
                    Edge::Convert { from, to }
                }
                "ocio_display" => {
                    let (display, view) = (name(e, "display"), name(e, "view"));
                    if display.is_empty() || view.is_empty() {
                        return None;
                    }
                    Edge::Display {
                        input: name(e, "input_colour_space"),
                        display,
                        view,
                        inverse: inverse(e),
                    }
                }
                "ocio_look" => {
                    let look = name(e, "look");
                    if look.is_empty() {
                        return None;
                    }
                    Edge::Look {
                        input: name(e, "input_colour_space"),
                        look,
                        output: name(e, "output_colour_space"),
                        inverse: inverse(e),
                    }
                }
                _ => return None,
            };
            Some(TableRequest::Ocio(OcioRequest::Config(edge)))
        })
        .collect()
}

/// The lens each enabled built-in lens effect reads other than its Lens row,
/// 1:1 and in order with the stack's lens ops, as the LUT files are.
fn flare_lens_files(
    effects: &[lumit_core::model::EffectInstance],
    lt: f64,
) -> Vec<Option<lumit_core::fx::lens::LensSource>> {
    use lumit_core::fx::lens;
    use lumit_core::model::EffectNamespace;
    effects
        .iter()
        .filter(|e| {
            e.enabled
                && e.effect.namespace == EffectNamespace::Builtin
                && lens::takes_a_lens(&e.effect.match_name)
        })
        .map(|e| lens::source_at(e, lt))
        .collect()
}

/// What one **picture Input** of a node graph reads (docs/impl/
/// node-graph-comp.md §1.5), in Input order. An Input with no entry reads
/// transparent, which is the comp viewed on its own.
enum GraphFeed {
    /// The first picture Input when the graph is applied as an effect: the
    /// layer's own picture at that point in the stack, which only `run_ops`
    /// holds.
    Provided,
    /// A further picture Input of that effect: one of the host comp's layers,
    /// rendered alone on the carriage a depth pass travels. Boxed, since a
    /// layer picture is far larger than the other two.
    Picture(Box<LayerInputDraw>),
    /// A step of the plan being built. A nested graph reads the outer box's
    /// sockets, so its Input boxes are aliases of steps that are already there
    /// and no second plan is made.
    Step(Option<usize>),
}

/// A node graph composition's whole draw list: one draw at identity placement
/// (docs/impl/node-graph-comp.md §2.3), lowered under `view`.
///
/// **Every road that reaches a comp with a graph comes through here** - the
/// comp viewed on its own, a Precomp layer of it, a matte or layer-input
/// reference, and a Read box - so what a placed graph's own Input values are
/// worth cannot depend on which road asked (§5.3).
///
/// `visited` is the ancestor chain of comps with this one already on it, so a
/// graph that reads its own comp stops at the cycle instead of recursing for
/// ever.
///
/// The Audio level driver's tap is made for **this** comp, since a driver
/// inside the graph hears the mix of the composition it sits in.
fn graph_comp_draw(
    lower: &GraphLower<'_>,
    comp: &lumit_core::model::Composition,
    graph: &lumit_core::comp_graph::CompGraph,
    view: lumit_core::comp_graph::GraphView<'_>,
    visited: &[Uuid],
) -> CompLayerDraw {
    let audio = crate::audio_tap::DocumentAudio::new(lower.doc, comp, lower.t);
    let lower = GraphLower {
        audio: &audio,
        ..*lower
    };
    let mut plan = GraphDraw {
        width: comp.width,
        height: comp.height,
        room: lower.doc.graph_room.frames(),
        steps: Vec::new(),
        output: None,
    };
    let mut path = visited.to_vec();
    plan.output = lower.lower(comp, graph, view, &mut [], &mut path, &mut plan);
    let (w, h) = (comp.width as f32, comp.height as f32);
    CompLayerDraw {
        layer: comp.id,
        source: DrawSource::Graph(Box::new(plan)),
        natural_size: (w, h),
        position: (w * 0.5, h * 0.5),
        anchor: (w * 0.5, h * 0.5),
        scale: (100.0, 100.0),
        rotation_deg: 0.0,
        opacity: 100.0,
        z: 0.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: 0.0,
        three_d: false,
        ends_run: false,
        matte: None,
        blend: lumit_gpu::composite::Blend::Normal,
        mask_cov: None,
        pre: None,
        fx: Default::default(),
        fx_ids: Vec::new(),
        neighbours: Vec::new(),
        flow_fields: Vec::new(),
        colour_tables: Vec::new(),
        dof_inputs: Vec::new(),
        mattes: Vec::new(),
        mask_paths: Vec::new(),
        roto_mattes: Vec::new(),
        planes: Vec::new(),
        points_schedules: Vec::new(),
        flare_lens_files: Vec::new(),
        graph_fx: Vec::new(),
        // The graph's boxes resolved in its own frame's pixels; the realiser
        // rescales them to whatever raster it walks at.
        fx_ref_width: Some(comp.width as f32),
        fx_input_key: None,
        mb: Vec::new(),
        lights: Vec::new(),
        material: Default::default(),
        temporal_below: None,
        accumulation_below: None,
        flow_below: Vec::new(),
    }
}

/// Which frame of its source `layer` shows at layer time `lt`: the number an
/// analysis files a matte or a plane under.
///
/// The file's own frame for footage, which the decode carried here. A Precomp
/// layer decodes nothing, so its answer is the nested comp's frame, which is
/// what an analysis of a comp renders and counts by. Nought for a layer with
/// neither, which has no analysis to find.
fn analysed_frame(
    doc: &lumit_core::model::Document,
    layer: &lumit_core::model::Layer,
    lt: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
) -> i64 {
    if let lumit_core::model::LayerKind::Precomp { comp } = layer.kind {
        return doc.comp(comp).map_or(0, |nested| {
            lumit_core::model::nested_source_frame(layer, nested, lt)
        });
    }
    pixels_by_layer
        .get(&layer.id)
        .map_or(0, |px| px.source_frame)
}

/// One Roto brush's matte for one **source** frame, or `None` outside the
/// propagated span, with nothing propagated yet, or with the cache folder
/// deleted - the effect's passthrough and never a fault (docs/impl/roto.md §5).
fn roto_matte_of(
    brush: &lumit_core::model::EffectInstance,
    source_frame: i64,
) -> Option<crate::draw::RotoMatteDraw> {
    let (width, height, gray) = crate::roto::matte(brush.id, source_frame)?;
    Some(crate::draw::RotoMatteDraw {
        width,
        height,
        gray,
    })
}

/// One planes-tier effect's plane for one **source** frame, or `None` outside
/// the analysed span, before any Analyse, and wherever no pack is installed
/// (docs/impl/addons.md §6.1).
fn plane_of(
    fx: &lumit_core::model::EffectInstance,
    source_frame: i64,
) -> Option<crate::draw::PlaneDraw> {
    let (width, height, kind, data, content) = crate::planes::plane(fx.id, source_frame)?;
    Some(crate::draw::PlaneDraw {
        instance: fx.id,
        source_frame,
        content,
        width,
        height,
        kind,
        data,
    })
}

/// The most steps a node graph's plan grows to through its Repeat pairs.
const MAX_REPEAT_STEPS: usize = 4096;

/// Where one output socket of a box landed in a graph's plan: the box, the
/// socket, and the step, `None` where it reads transparent.
type Landed = (Uuid, std::borrow::Cow<'static, str>, Option<usize>);

/// What the node graph lowering carries, unchanged at every depth: the
/// project, the comp being walked, the frame, and the fetch the draw builder
/// already has for one layer's own pixels.
///
/// Bundled for `PlanContext`'s reason - the walk recurses through nested
/// graphs, and a dozen loose parameters at every level is a list to get wrong.
///
/// Copied rather than borrowed where the clock changes: a graph applied as an
/// effect runs on the host layer's time, and everything else it carries is the
/// same at every depth.
#[derive(Clone, Copy)]
struct GraphLower<'a> {
    doc: &'a Arc<lumit_core::model::Document>,
    t: f64,
    frame_t: f64,
    pixels_by_layer: &'a PixelsByLayer<'a>,
    keys: Option<&'a dyn crate::cache::NestedKeyer>,
    /// The Audio level driver's samples, from the walk that already made them,
    /// so a driver inside a graph hears what a driver on a layer hears.
    audio: &'a crate::audio_tap::DocumentAudio<'a>,
    /// One layer's own picture, alone: the draw builder's own `pixels_for`,
    /// handed in so a Read box fetches by the very road a layer does.
    pixels_for: &'a dyn Fn(&lumit_core::model::Layer) -> Option<LayerPixels>,
    /// Which copy of how many the walk is rendered as, for a Clone index box
    /// in the graph and for every comp the graph reads.
    clone: (u32, u32),
}

impl GraphLower<'_> {
    /// Lower `graph` into `plan` and answer the step its Output shows
    /// (docs/impl/node-graph-comp.md §2.3).
    ///
    /// Only what the Output reaches is lowered, so an idle branch costs
    /// nothing, and the order is Kahn's walk in document order - vectors
    /// throughout, never a map, so the plan is the same on every machine.
    ///
    /// `graph_comp` is the comp that owns `graph`, which is where its markers
    /// and its expressions are read from. `visited` is the ancestor chain of
    /// comps, so a graph that reads itself stops at the cycle.
    ///
    /// `view` is how the graph is being looked at (§5.3): the values a host
    /// hands its Inputs, which bake into the parameters they feed and which an
    /// expression reads back through `input()`, and whether the picture Inputs
    /// are fed from outside at all.
    fn lower(
        &self,
        graph_comp: &lumit_core::model::Composition,
        graph: &lumit_core::comp_graph::CompGraph,
        view: lumit_core::comp_graph::GraphView<'_>,
        feeds: &mut [GraphFeed],
        visited: &mut Vec<Uuid>,
        plan: &mut GraphDraw,
    ) -> Option<usize> {
        use lumit_core::comp_graph::{
            GraphNode, InputKind, COMBINE_CHANNELS, NODE_GRAPH, REPEAT_END, REPEAT_START,
            SPLIT_CHANNELS, SPLIT_OUTPUTS,
        };
        use lumit_core::graph::{INPUT_PORT, MATTE_PORT, OUTPUT_PORT};

        let output = graph.output_id()?;

        // The value half, once for the whole graph (§2.2): the Inputs baked
        // into the parameters they feed, and the driver wires as the layer
        // graph the existing walk resolves.
        let projection = graph.project(view.values);
        let context = Arc::new(ExpressionContext {
            document: self.doc.clone(),
            comp: Some(graph_comp.id),
            // A graph has no layer, exactly as a group header has none.
            layer: None,
            comp_time: self.t,
            current_depth: 0,
            // What `input("x")` reads inside the graph (§5.5): the host's own
            // value where it handed one over, and the Input's default
            // otherwise, which the expression module falls back to.
            inputs: (!view.values.is_empty()).then(|| view.values.into()),
        });
        let drivers = lumit_core::fx::drivers::resolve_drivers_cloned(
            &projection.drivers,
            self.t,
            context.clone(),
            Some(self.audio),
            lumit_core::fx::points::Projection::FLAT,
            self.clone,
        );
        let markers = lumit_core::fx::markers::MarkerContext::for_comp(graph_comp);
        // Spatial units are px in the graph's own frame, and the realiser
        // rescales them to whatever raster it walks at - the same correction a
        // Precomp layer's stack takes.
        let diag = ((plan.width as f32).powi(2) + (plan.height as f32).powi(2)).sqrt();
        // The socket each Switch shows at this frame, worked out before
        // anything is lowered, so what sits behind the other sockets is never
        // decoded or drawn. A bypassed Switch shows its first.
        let picks: Vec<(Uuid, i64)> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Fx(inst) if inst.effect.match_name == lumit_core::comp_graph::SWITCH => {
                    let inst = projection
                        .effects
                        .iter()
                        .find(|p| p.id == inst.id)
                        .unwrap_or(inst);
                    let index = if inst.enabled {
                        let rows = lumit_core::fx::resolved::resolve_instance(
                            inst,
                            &drivers,
                            self.t,
                            diag,
                            1.0,
                            &markers,
                            context.clone(),
                        );
                        let index = rows.iter().next().map_or(0, |op| {
                            op.params
                                .int(lumit_core::fx::params::ParamId::new("index"), 0)
                        });
                        index
                    } else {
                        0
                    };
                    Some((inst.id, i64::from(index)))
                }
                _ => None,
            })
            .collect();
        // What the Output reaches, in the order a box's sources land before
        // the box. A box nothing consumes never enters the plan, and one
        // inside a loop - refused at the edit - is simply left out.
        let order = graph.cone_picked(output, &picks);
        // The Repeat pairs. The boxes between each pair, and its start, are
        // lowered by its end, once a time, so the walk below steps over them.
        let repeats = graph.repeats().unwrap_or_default();
        let hidden: Vec<Uuid> = repeats
            .iter()
            .flat_map(|r| r.body.iter().copied().chain([r.start]))
            .collect();

        // Which step each box's output landed on, in the order they were
        // lowered. Keyed by socket too, since a Split channels box has four
        // and a box with a view menu has one per view.
        let mut steps: Vec<Landed> = Vec::with_capacity(order.len());
        let wired = |steps: &[Landed], node: Uuid, port: &str| -> Option<usize> {
            let (from, from_port) = graph.wire_into(node, port)?;
            steps
                .iter()
                .find(|(id, out, _)| id == from && out == from_port)
                .and_then(|(_, _, step)| *step)
        };
        // One Set channels pass over `input` with `source` on its Source row,
        // which is all Split channels and Combine channels are made of.
        let channel_pass = |id: Uuid, input: Option<usize>, source, picks: [u32; 4]| {
            let mut inst = lumit_core::fx::builtins::instantiate("set_channels")?;
            inst.id = id;
            let rows = ["red_from", "green_from", "blue_from", "alpha_from"];
            for (row, pick) in rows.into_iter().zip(picks) {
                if let Some(p) = inst.params.iter_mut().find(|p| p.id == row) {
                    p.value = lumit_core::model::EffectValue::Choice(pick);
                }
            }
            Some(GraphStep::Fx {
                input,
                ops: lumit_core::fx::resolved::resolve_stack(
                    std::slice::from_ref(&inst),
                    self.t,
                    diag,
                    1.0,
                    &markers,
                    context.clone(),
                ),
                fx_ids: vec![id],
                matte: None,
                picture: source,
                picture_depth: false,
                colour_tables: vec![None],
                flare_lens_files: vec![None],
                roto: None,
                plane: None,
                baked_from: None,
                points: Vec::new(),
            })
        };
        // Set channels picks, by their place in `SET_CHANNELS_OPTIONS`.
        const RED: u32 = 0;
        const GREEN: u32 = 1;
        const BLUE: u32 = 2;
        const ALPHA: u32 = 3;
        const LUMINANCE: u32 = 4;
        // Adding this reads the same channel off the Source row.
        const SOURCE: u32 = 5;
        const FULL_ON: u32 = 10;
        const FULL_OFF: u32 = 11;

        for id in order.iter().copied() {
            if hidden.contains(&id) {
                continue;
            }
            let Some(node) = graph.node(id) else { continue };
            let step = match node {
                // The Output shows what is wired into it; it is no step of its
                // own.
                GraphNode::Output { .. } => continue,
                GraphNode::Read { item, .. } => {
                    let depth = lumit_core::comp_graph::READ_DEPTH_PORT.id;
                    if graph
                        .edges
                        .iter()
                        .any(|e| e.from == id && e.from_port == depth && order.contains(&e.to))
                    {
                        let step = self.depth_step(graph_comp, id, *item, visited, plan);
                        steps.push((id, depth.into(), step));
                    }
                    self.read_step(graph_comp, id, *item, visited, plan)
                }
                GraphNode::Input { input, .. } => {
                    if input.kind == InputKind::Picture {
                        // Which picture Input this is, counted over the whole
                        // Input list, because that is the order `feeds` is in.
                        let nth = graph
                            .inputs()
                            .filter(|i| i.kind == InputKind::Picture)
                            .position(|i| i.id == input.id);
                        // Taken rather than borrowed: a picture is a
                        // rendered layer's own pixels, one Input reads it, and
                        // copying a frame of them to satisfy the borrow
                        // checker is a frame of them copied.
                        match nth.and_then(|n| feeds.get_mut(n)) {
                            Some(feed) => match std::mem::replace(feed, GraphFeed::Step(None)) {
                                GraphFeed::Provided => {
                                    plan.steps.push(GraphStep::Provided);
                                    Some(plan.steps.len() - 1)
                                }
                                GraphFeed::Picture(p) => {
                                    plan.steps.push(GraphStep::Picture(*p));
                                    Some(plan.steps.len() - 1)
                                }
                                GraphFeed::Step(s) => s,
                            },
                            // Viewed on its own, or placed as a Precomp layer:
                            // the preview item stands in for the picture
                            // nobody is feeding (§5.11), and transparent when
                            // there is none.
                            None => input
                                .preview
                                .filter(|_| !view.pictures_fed)
                                .and_then(|item| {
                                    self.read_step(graph_comp, id, item, visited, plan)
                                }),
                        }
                    } else {
                        // A value Input carries no picture: it was baked into
                        // the parameters it feeds by the projection above.
                        None
                    }
                }
                GraphNode::Fx(inst) => {
                    // The instance as it is to be resolved: the projection's
                    // copy, with every wired Input written in.
                    let inst = projection
                        .effects
                        .iter()
                        .find(|p| p.id == inst.id)
                        .unwrap_or(inst);
                    let input = wired(&steps, id, INPUT_PORT.id);
                    // The bypass tick. Merge, Switch and a nested Node graph
                    // box have no op for the resolver to leave out, so a
                    // bypassed box hands on its main picture here, which is
                    // where a bypassed ordinary box lands anyway. A Switch's
                    // own picture is its first socket, and a Split hands its
                    // input on at every output, as a box with views does.
                    // A bypassed Repeat end hands on what its start was
                    // given, which is what Times at nought does.
                    let repeat = repeats.iter().find(|r| r.end == id);
                    if !inst.enabled {
                        let main = if inst.effect.match_name == lumit_core::comp_graph::SWITCH {
                            wired(&steps, id, "in0")
                        } else if let Some(r) = repeat {
                            wired(&steps, r.start, INPUT_PORT.id)
                        } else {
                            input
                        };
                        let outs: &[&'static str] = if inst.effect.match_name == SPLIT_CHANNELS {
                            &SPLIT_OUTPUTS
                        } else {
                            &[OUTPUT_PORT.id]
                        };
                        steps.extend(outs.iter().map(|port| (id, (*port).into(), main)));
                        steps.extend(
                            lumit_core::comp_graph::view_ports(inst)
                                .into_iter()
                                .map(|port| (id, port.id.into(), main)),
                        );
                        continue;
                    }
                    match inst.effect.match_name.as_str() {
                        lumit_core::comp_graph::MERGE => {
                            let rows = lumit_core::fx::resolved::resolve_instance(
                                inst,
                                &drivers,
                                self.t,
                                diag,
                                1.0,
                                &markers,
                                context.clone(),
                            );
                            let p = rows.iter().next().map(|op| op.params);
                            plan.steps.push(GraphStep::Merge {
                                a: input,
                                b: wired(&steps, id, lumit_core::comp_graph::BACKGROUND_PORT.id),
                                mode: p.map_or(0, |p| {
                                    p.choice(lumit_core::fx::params::ParamId::new("mode"), 0)
                                }),
                                opacity: p.map_or(100.0, |p| {
                                    p.float(lumit_core::fx::params::ParamId::new("opacity"), 100.0)
                                }),
                            });
                            Some(plan.steps.len() - 1)
                        }
                        lumit_core::comp_graph::SWITCH => {
                            let index = picks.iter().find(|(p, _)| *p == id).map_or(0, |p| p.1);
                            // As many sockets as are wired. Capped for the
                            // reason `ports_of` caps: the highest index comes
                            // out of a file, and a hand-edited one must not
                            // ask for a vector nobody can hold.
                            let highest = graph
                                .edges
                                .iter()
                                .filter(|e| e.to == id)
                                .filter_map(|e| e.to_port.strip_prefix("in")?.parse::<usize>().ok())
                                .max()
                                .map(|n| n.min(64));
                            let inputs = highest
                                .map(|n| {
                                    (0..=n)
                                        .map(|i| wired(&steps, id, &format!("in{i}")))
                                        .collect()
                                })
                                .unwrap_or_default();
                            plan.steps.push(GraphStep::Switch { inputs, index });
                            Some(plan.steps.len() - 1)
                        }
                        // Each output something in the cone reads is one pass:
                        // that channel of the straight picture in red, green
                        // and blue, and alpha full on.
                        SPLIT_CHANNELS => {
                            let mut red = None;
                            for (pick, port) in (RED..).zip(SPLIT_OUTPUTS) {
                                let read = graph.edges.iter().any(|e| {
                                    e.from == id && e.from_port == port && order.contains(&e.to)
                                });
                                if !read {
                                    continue;
                                }
                                let picks = [pick, pick, pick, FULL_ON];
                                let Some(pass) = channel_pass(id, input, None, picks) else {
                                    continue;
                                };
                                plan.steps.push(pass);
                                let step = Some(plan.steps.len() - 1);
                                if port == OUTPUT_PORT.id {
                                    red = step;
                                } else {
                                    steps.push((id, port.into(), step));
                                }
                            }
                            red
                        }
                        // Three passes over the Red picture, each taking the
                        // picked channel of the next one off its Source row.
                        // An empty Source reads nothing, so an unwired Alpha
                        // is full on instead.
                        COMBINE_CHANNELS => {
                            let rows = lumit_core::fx::resolved::resolve_instance(
                                inst,
                                &drivers,
                                self.t,
                                diag,
                                1.0,
                                &markers,
                                context.clone(),
                            );
                            // A `CHANNEL_OPTIONS` pick as Set channels' own
                            // pick, Luminance first.
                            let own = |row: &str| {
                                let at = rows.iter().next().map_or(0, |op| {
                                    op.params
                                        .choice(lumit_core::fx::params::ParamId::new(row), 0)
                                });
                                [LUMINANCE, ALPHA, RED, GREEN, BLUE]
                                    .get(at as usize)
                                    .copied()
                                    .unwrap_or(LUMINANCE)
                            };
                            let alpha = wired(&steps, id, "alpha");
                            let alpha_pick = match alpha {
                                Some(_) => own("alpha_from") + SOURCE,
                                None => FULL_ON,
                            };
                            let passes = [
                                (
                                    wired(&steps, id, "green"),
                                    [
                                        own("red_from"),
                                        own("green_from") + SOURCE,
                                        FULL_OFF,
                                        FULL_ON,
                                    ],
                                ),
                                (
                                    wired(&steps, id, "blue"),
                                    [RED, GREEN, own("blue_from") + SOURCE, FULL_ON],
                                ),
                                (alpha, [RED, GREEN, BLUE, alpha_pick]),
                            ];
                            let mut at = input;
                            for (source, picks) in passes {
                                if let Some(s) = channel_pass(id, at, source, picks) {
                                    plan.steps.push(s);
                                    at = Some(plan.steps.len() - 1);
                                }
                            }
                            at
                        }
                        NODE_GRAPH => self
                            .nested_step(inst, graph, id, input, &steps, &drivers, visited, plan),
                        // A start or an end with no pair hands its picture on.
                        REPEAT_START => input,
                        REPEAT_END => match repeat {
                            Some(r) => self.repeat_step(
                                graph_comp,
                                graph,
                                view,
                                r,
                                inst,
                                &steps,
                                (&drivers, &projection),
                                diag,
                                &context,
                                visited,
                                plan,
                            ),
                            None => input,
                        },
                        _ => {
                            // What an analysis filed for the file this box
                            // reads, looked up once: every view of the box
                            // draws through the same matte.
                            let (roto, plane, baked_from) = self.baked_for(graph, inst, &steps);
                            let mut run = |inst: &lumit_core::model::EffectInstance| {
                                let (fx_ids, ops) =
                                    lumit_core::fx::resolved::resolve_stack_temporal_named(
                                        std::slice::from_ref(inst),
                                        &drivers,
                                        self.t,
                                        self.frame_t,
                                        diag,
                                        1.0,
                                        &markers,
                                        context.clone(),
                                    );
                                // A bypassed box, an entry this build does not
                                // know, an orchestration-only effect: no op, so
                                // the box hands on what it was given.
                                if ops.is_empty() {
                                    return input;
                                }
                                let param = lumit_core::fx::builtins::def(&inst.effect.match_name)
                                    .and_then(|d| d.schema().layer_input());
                                let picture = param.and_then(|param| wired(&steps, id, param));
                                // Whether that wire carries metres, from the
                                // socket it leaves.
                                let picture_depth = param
                                    .and_then(|param| graph.wire_into(id, param))
                                    .and_then(|(from, port)| {
                                        let node = graph.node(*from)?;
                                        lumit_core::comp_graph::ports_of(
                                            graph,
                                            node,
                                            Some(&**self.doc),
                                        )
                                        .1
                                        .into_iter()
                                        .find(|p| p.id == port)
                                    })
                                    .is_some_and(|p| {
                                        p.ty == lumit_core::fx::schema::PortType::Depth
                                    });
                                plan.steps.push(GraphStep::Fx {
                                    input,
                                    ops,
                                    fx_ids,
                                    matte: wired(&steps, id, MATTE_PORT.id),
                                    picture,
                                    picture_depth,
                                    colour_tables: colour_tables(
                                        std::slice::from_ref(inst),
                                        self.t,
                                    ),
                                    flare_lens_files: flare_lens_files(
                                        std::slice::from_ref(inst),
                                        self.t,
                                    ),
                                    roto: roto.clone(),
                                    plane: plane.clone(),
                                    baked_from,
                                    points: self.graph_points(
                                        graph_comp,
                                        &projection,
                                        inst,
                                        &context,
                                    ),
                                });
                                Some(plan.steps.len() - 1)
                            };
                            // A box with a view menu draws once per socket
                            // something in the cone reads: each view with the
                            // menu held on its option, and `output` as the
                            // dropdown has it. Every other box draws once.
                            let views = lumit_core::comp_graph::view_ports(inst);
                            let read = |port: &str| {
                                graph.edges.iter().any(|e| {
                                    e.from == id && e.from_port == port && order.contains(&e.to)
                                })
                            };
                            let mut landed = Vec::new();
                            for port in views.iter().filter(|port| read(&port.id)) {
                                let held = lumit_core::comp_graph::held_on_view(inst, &port.id);
                                if let Some(held) = held {
                                    landed.push((id, port.id.clone().into(), run(&held)));
                                }
                            }
                            let main = if views.is_empty() || read(OUTPUT_PORT.id) {
                                run(inst)
                            } else {
                                None
                            };
                            steps.extend(landed);
                            main
                        }
                    }
                }
            };
            steps.push((id, OUTPUT_PORT.id.into(), step));
        }
        wired(&steps, output, INPUT_PORT.id)
    }

    /// What a background analysis filed for one box at this frame, and the
    /// Read step it was made from: a Roto brush's matte or a planes-tier
    /// plane, looked up by the box and the **source** frame of the footage its
    /// picture comes from, the lookup a layer's own carriage makes.
    ///
    /// All `None` for every other box, and for a box whose picture comes from
    /// no Read. Which kind of item the Read brings in is not asked, as a
    /// layer's carriage does not ask it: only footage can be analysed, so only
    /// footage has anything filed.
    fn baked_for(
        &self,
        graph: &lumit_core::comp_graph::CompGraph,
        inst: &lumit_core::model::EffectInstance,
        steps: &[Landed],
    ) -> (
        Option<crate::draw::RotoMatteDraw>,
        Option<crate::draw::PlaneDraw>,
        Option<usize>,
    ) {
        let brush = inst.effect.match_name == lumit_core::roto::ROTO_BRUSH;
        if !brush && lumit_core::planes::task_of(inst).is_none() {
            return (None, None, None);
        }
        let found = graph.read_behind(inst.id).and_then(|(read, _)| {
            let step = steps
                .iter()
                .find(|(id, port, _)| *id == read && port == lumit_core::graph::OUTPUT_PORT.id)
                .and_then(|(_, _, step)| *step)?;
            // Nought for an item with no frames of its own, which is what a
            // layer of one reads too.
            let frame = self
                .pixels_by_layer
                .get(&read)
                .map_or(0, |px| px.source_frame);
            Some((step, frame))
        });
        let Some((step, frame)) = found else {
            return (None, None, None);
        };
        if brush {
            (roto_matte_of(inst, frame), None, Some(step))
        } else {
            (None, plane_of(inst, frame), Some(step))
        }
    }

    /// One Read box as the layer it behaves like (§2.1): the item at default
    /// placement, realised alone into the graph's frame.
    ///
    /// A missing item, a folder, a cyclic comp: no step at all, which every
    /// consumer reads as the transparent an unwired socket reads.
    fn read_step(
        &self,
        graph_comp: &lumit_core::model::Composition,
        id: Uuid,
        item: Uuid,
        visited: &mut Vec<Uuid>,
        plan: &mut GraphDraw,
    ) -> Option<usize> {
        use lumit_core::model::LayerKind;
        let mut layer = lumit_core::comp_graph::read_layer(id, self.doc.item(item)?, graph_comp)?;
        // **A Read box has no in and out points**: its picture is there
        // whenever the graph is asked. The synthetic layer wears the graph
        // comp's span, and the fetch below gates a solid, a text or a shape on
        // it against the host comp's time, which on a graph applied as an
        // effect can be past a short graph's end. So the span is opened first.
        if let Ok(open) = lumit_core::time::Rational::new(i64::from(u32::MAX), 1) {
            layer.out_point = lumit_core::time::CompTime(open);
        }
        let (source, natural) = match &layer.kind {
            LayerKind::Precomp { comp: nested_id } => {
                if visited.contains(nested_id) {
                    return None;
                }
                let nested = self.doc.comp(*nested_id)?;
                visited.push(*nested_id);
                let draws = match nested.graph.as_ref() {
                    // A Read of a node graph: the same one road every other
                    // reader takes, on its own defaults with no picture fed.
                    Some(graph) => vec![graph_comp_draw(
                        self,
                        nested,
                        graph,
                        lumit_core::comp_graph::GraphView::DEFAULTS,
                        visited,
                    )],
                    None => comp_walk(
                        self.doc,
                        nested,
                        self.t,
                        self.frame_t,
                        self.pixels_by_layer,
                        visited,
                        self.keys,
                        false,
                        Walk {
                            clone: self.clone,
                            values: &[],
                            rebuild: false,
                            seen_through: None,
                        },
                        None,
                    ),
                };
                visited.pop();
                (
                    DrawSource::Nested {
                        width: nested.width,
                        height: nested.height,
                        background: [0.0, 0.0, 0.0, 0.0],
                        draws,
                        camera: Some(crate::track::camera_pose(self.doc, nested, self.t)),
                        // The comp's own name, whichever graph reads it - so a
                        // Read of a comp is cached exactly as a Precomp layer
                        // of it is.
                        key: self.keys.and_then(|k| k.nested_key(nested, self.t)),
                        paint: Vec::new(),
                        paint_time: self.t,
                    },
                    (nested.width as f32, nested.height as f32),
                )
            }
            _ => {
                let (rgba, tex_w, tex_h, natural, format) = (self.pixels_for)(&layer)?;
                (
                    DrawSource::Pixels {
                        rgba,
                        tex_w,
                        tex_h,
                        format,
                        colour_space: crate::colour::footage_colour_space(self.doc, &layer.kind),
                    },
                    natural,
                )
            }
        };
        plan.steps.push(GraphStep::Read(Box::new(read_draw(
            &layer, source, natural,
        ))));
        Some(plan.steps.len() - 1)
    }

    /// A Read's Depth output: the comp it reads, walked as for its picture and
    /// drawn as distance. `None`, which reads as no depth, for anything that is
    /// not a layered comp.
    fn depth_step(
        &self,
        graph_comp: &lumit_core::model::Composition,
        id: Uuid,
        item: Uuid,
        visited: &mut Vec<Uuid>,
        plan: &mut GraphDraw,
    ) -> Option<usize> {
        let layer = lumit_core::comp_graph::read_layer(id, self.doc.item(item)?, graph_comp)?;
        let lumit_core::model::LayerKind::Precomp { comp: nested_id } = layer.kind else {
            return None;
        };
        let nested = self.doc.comp(nested_id)?;
        if visited.contains(&nested_id) || nested.graph.is_some() {
            return None;
        }
        visited.push(nested_id);
        let draws = comp_walk(
            self.doc,
            nested,
            self.t,
            self.frame_t,
            self.pixels_by_layer,
            visited,
            self.keys,
            false,
            Walk {
                clone: self.clone,
                values: &[],
                rebuild: false,
                seen_through: None,
            },
            None,
        );
        visited.pop();
        plan.steps.push(GraphStep::SceneDepth(Box::new(
            crate::draw::NestedInputDraw {
                width: nested.width,
                height: nested.height,
                background: [0.0, 0.0, 0.0, 0.0],
                draws,
                camera: Some(crate::track::camera_pose(self.doc, nested, self.t)),
                key: None,
                px_per_metre: nested.px_per_metre(self.t),
                scene_depth: true,
                trace_layers: false,
            },
        )));
        Some(plan.steps.len() - 1)
    }

    /// What a points box in a graph is handed beside its op, as a layer's
    /// points effects are: its birth schedule, and the streams wired into its
    /// Points sockets, made by the driver walk over the graph's own boxes.
    /// Nothing for any other box.
    fn graph_points(
        &self,
        graph_comp: &lumit_core::model::Composition,
        projection: &lumit_core::comp_graph::Projection,
        inst: &lumit_core::model::EffectInstance,
        context: &Arc<ExpressionContext>,
    ) -> Vec<lumit_core::fx::points::PointsSchedule> {
        let Some(def) = lumit_core::fx::catalogue::BUILTIN_DEFS.get(&inst.effect.match_name) else {
            return Vec::new();
        };
        if !(lumit_core::fx::points::wants_carriage_of(def) && def.is_image_op()) {
            return Vec::new();
        }
        let (t, dt) = (self.t, 1.0 / graph_comp.frame_rate.fps().max(1.0));
        let wired = points_input_in(
            None,
            (
                &projection.drivers,
                &projection.effects,
                Some(&projection.effects),
            ),
            inst,
            def,
            t,
            dt,
            lumit_core::fx::points::Projection::FLAT,
            context,
            self.audio,
            self.clone,
        );
        let schedule = if def.schema().params.iter().any(|p| p.id == "emit_rate") {
            birth_schedule(inst, t, dt, context, Some(&projection.drivers), self.audio)
        } else {
            lumit_core::fx::points::Schedule::default()
        };
        vec![lumit_core::fx::points::PointsSchedule {
            schedule,
            t,
            input_from: wired.from.first().copied().flatten(),
            ports_from: wired
                .from
                .iter()
                .skip(1)
                .map(|f| f.unwrap_or(u32::MAX))
                .collect(),
            input: wired.streams,
            ..Default::default()
        }]
    }

    /// A Repeat end: the boxes between it and its start, lowered once a time
    /// into this plan. In Feedback each time is handed the picture the time
    /// before made, and in Copies each is handed the start's own and laid
    /// over the ones before it.
    #[allow(clippy::too_many_arguments)]
    fn repeat_step(
        &self,
        graph_comp: &lumit_core::model::Composition,
        graph: &lumit_core::comp_graph::CompGraph,
        view: lumit_core::comp_graph::GraphView<'_>,
        r: &lumit_core::comp_graph::Repeat,
        end: &lumit_core::model::EffectInstance,
        steps: &[Landed],
        (drivers, projection): (
            &lumit_core::fx::drivers::ResolvedDrivers,
            &lumit_core::comp_graph::Projection,
        ),
        diag: f32,
        context: &Arc<ExpressionContext>,
        visited: &mut Vec<Uuid>,
        plan: &mut GraphDraw,
    ) -> Option<usize> {
        use lumit_core::comp_graph::GraphNode;
        use lumit_core::fx::effects::repeat::{
            Element, FOR_PATHS, FOR_POINTS, MODE_COPIES, POINTS_PORT, SHAPES_PORT,
        };
        use lumit_core::fx::params::ParamId;
        let landed = |node: Uuid, port: &str| {
            steps
                .iter()
                .find(|(id, out, _)| *id == node && out == port)
                .and_then(|(_, _, step)| *step)
        };
        let into = graph
            .wire_into(r.start, lumit_core::graph::INPUT_PORT.id)
            .and_then(|(from, port)| landed(*from, port));
        let markers = lumit_core::fx::markers::MarkerContext::for_comp(graph_comp);
        let rows = lumit_core::fx::resolved::resolve_instance(
            end,
            drivers,
            self.t,
            diag,
            1.0,
            &markers,
            context.clone(),
        );
        let p = rows.iter().next().map(|op| op.params);
        let times = p.map_or(0, |p| p.int(ParamId::new("times"), 4)).max(0) as u32;
        // What each time is handed: a point of the stream on the start's
        // Points socket, a path of the Read on its Shapes socket, or nothing.
        let source = |port: &str| graph.wire_into(r.start, port);
        let elements: Option<Vec<Element>> = match p
            .map_or(0, |p| p.choice(ParamId::new("for_each"), 0))
        {
            FOR_POINTS => {
                let stream = source(POINTS_PORT).and_then(|(from, _)| {
                    lumit_core::fx::drivers::effect_stream_in(
                        &projection.drivers,
                        *from,
                        self.t,
                        context.clone(),
                        Some(self.audio),
                        lumit_core::fx::points::Projection::FLAT,
                        Some(&projection.effects),
                        self.clone,
                    )
                });
                Some(stream.map_or_else(Vec::new, |s| {
                    (0..s.len())
                        .map(|i| {
                            let c = s.colour.get(i).copied().unwrap_or([1.0; 4]);
                            let a = if c[3] > 0.0 { c[3] } else { 1.0 };
                            Element {
                                at: s.projected(i),
                                size: s.size.get(i).copied().unwrap_or(0.0),
                                rotation: s.rotation.get(i).copied().unwrap_or(0.0).to_degrees(),
                                colour: [c[0] / a, c[1] / a, c[2] / a, c[3]],
                            }
                        })
                        .collect()
                }))
            }
            FOR_PATHS => {
                let item = source(SHAPES_PORT).and_then(|(from, _)| match graph.node(*from) {
                    Some(GraphNode::Read { item, .. }) => Some(*item),
                    _ => None,
                });
                let size = [plan.width as f32, plan.height as f32];
                Some(item.map_or_else(Vec::new, |item| {
                    lumit_core::comp_graph::read_paths(context, item, size, self.t)
                        .into_iter()
                        .map(|[l, t, r, b]| Element {
                            at: [(l + r) * 0.5, (t + b) * 0.5],
                            size: (r - l).max(b - t),
                            ..Element::default()
                        })
                        .collect()
                }))
            }
            _ => None,
        };
        let times = elements
            .as_ref()
            .map_or(times, |e| u32::try_from(e.len()).unwrap_or(u32::MAX));
        let element = |k: u32| {
            elements
                .as_ref()
                .and_then(|e| e.get(k as usize).copied())
                .unwrap_or_default()
        };
        let copies = p.map_or(0, |p| p.choice(ParamId::new("mode"), 0)) == MODE_COPIES;
        let blend = p.map_or(0, |p| p.choice(ParamId::new("copies_blend"), 0));
        let Some(GraphNode::Fx(start)) = graph.node(r.start) else {
            return into;
        };
        let (inner, fed) = graph.repeat_graph(r);
        let outside: Vec<Option<usize>> = fed.iter().map(|(n, port)| landed(*n, port)).collect();
        let (mut last, mut laid) = (into, None);
        for k in 0..times {
            // A document must not be able to ask for more steps than this,
            // however many times it repeats.
            if plan.steps.len() >= MAX_REPEAT_STEPS {
                break;
            }
            let mut values = view.values.to_vec();
            values.extend(lumit_core::comp_graph::repeat_values(
                start,
                k,
                times,
                &element(k),
            ));
            let mut feeds = vec![GraphFeed::Step(if copies { into } else { last })];
            feeds.extend(outside.iter().map(|s| GraphFeed::Step(*s)));
            let out = self.lower(
                graph_comp,
                &inner,
                lumit_core::comp_graph::GraphView {
                    values: &values,
                    pictures_fed: true,
                },
                &mut feeds,
                visited,
                plan,
            );
            last = out;
            laid = match laid {
                // The first time at the bottom, each after it laid over.
                Some(below) if copies => {
                    plan.steps.push(GraphStep::Merge {
                        a: out,
                        b: Some(below),
                        mode: blend,
                        opacity: 100.0,
                    });
                    Some(plan.steps.len() - 1)
                }
                _ => out,
            };
        }
        if times == 0 {
            return into;
        }
        if copies {
            laid
        } else {
            last
        }
    }

    /// A Node graph box inside a graph: the named graph lowered into **this**
    /// plan, its Input boxes aliased to the sockets of the box that names it.
    ///
    /// A comp that is missing, is not a node graph, or is already on the
    /// visited path hands on the box's own input, which is the passthrough a
    /// dangling reference degrades to everywhere else.
    #[allow(clippy::too_many_arguments)]
    fn nested_step(
        &self,
        inst: &lumit_core::model::EffectInstance,
        graph: &lumit_core::comp_graph::CompGraph,
        id: Uuid,
        input: Option<usize>,
        steps: &[Landed],
        drivers: &lumit_core::fx::drivers::ResolvedDrivers,
        visited: &mut Vec<Uuid>,
        plan: &mut GraphDraw,
    ) -> Option<usize> {
        use lumit_core::comp_graph::InputKind;
        // Missing, cyclic, or naming a comp that is not a node graph: the box
        // hands on what it was given, which is the degrade a dangling
        // reference gets everywhere else.
        let Some(inner_id) = lumit_core::fx::effects::node_graph::comp_of(inst) else {
            return input;
        };
        if visited.contains(&inner_id) {
            return input;
        }
        let Some(inner_comp) = self.doc.comp(inner_id) else {
            return input;
        };
        let Some(inner) = inner_comp.graph.as_ref() else {
            return input;
        };
        let socket = |port: &str| -> Option<usize> {
            let (from, from_port) = graph.wire_into(id, port)?;
            steps
                .iter()
                .find(|(node, out, _)| node == from && out == from_port)
                .and_then(|(_, _, step)| *step)
        };
        // The box's own sockets, in the inner Inputs' order: the first picture
        // Input is `input`, every further one is a socket named by its id.
        let mut feeds = Vec::new();
        for picture in inner.inputs().filter(|i| i.kind == InputKind::Picture) {
            feeds.push(GraphFeed::Step(if feeds.is_empty() {
                input
            } else {
                socket(&picture.id)
            }));
        }
        let overrides =
            lumit_core::fx::effects::node_graph::overrides_of(inst, inner, Some(drivers));
        visited.push(inner_id);
        let out = self.lower(
            inner_comp,
            inner,
            lumit_core::comp_graph::GraphView {
                values: &overrides,
                // Nested as a box: its picture Inputs are the box's own
                // sockets, so no preview item stands in.
                pictures_fed: true,
            },
            &mut feeds,
            visited,
            plan,
        );
        visited.pop();
        // An inner graph with nothing wired to its Output passes the picture
        // on, as the same graph applied to a layer does.
        let out = out.or(input);
        let Some(matte) = socket(lumit_core::graph::MATTE_PORT.id) else {
            return out;
        };
        let value = |id: &str| inst.params.iter().find(|p| p.id == id).map(|p| &p.value);
        let channel = match value(lumit_core::fx::schema::MATTE_CHANNEL_PARAM) {
            Some(lumit_core::model::EffectValue::Choice(c)) => *c,
            _ => 0,
        };
        let has_channel = lumit_core::fx::builtins::def(lumit_core::comp_graph::NODE_GRAPH)
            .is_some_and(|d| d.schema().matte_channel());
        plan.steps.push(GraphStep::Dissolve {
            input,
            output: out,
            matte,
            channel: has_channel.then_some(channel),
            invert: matches!(
                value(lumit_core::fx::schema::MATTE_INVERT_PARAM),
                Some(lumit_core::model::EffectValue::Bool(true))
            ),
        });
        Some(plan.steps.len() - 1)
    }
}

/// One Read box's draw: the synthetic layer's placement, no effects, and every
/// carriage empty. What it draws is a picture at default placement, which is
/// the whole of what a Read box means.
///
/// The anchor is the picture's own centre, taken from the size the pixels
/// actually have rather than from the layer: the document holds no size for
/// footage, so `read_layer` can only guess the comp's, and a clip smaller or
/// larger than the comp would otherwise land off centre.
fn read_draw(
    layer: &lumit_core::model::Layer,
    source: DrawSource,
    natural: (f32, f32),
) -> CompLayerDraw {
    let tr = &layer.transform;
    CompLayerDraw {
        layer: layer.id,
        source,
        natural_size: natural,
        position: (
            tr.position_x.value_at(0.0) as f32,
            tr.position_y.value_at(0.0) as f32,
        ),
        anchor: (natural.0 * 0.5, natural.1 * 0.5),
        scale: (100.0, 100.0),
        rotation_deg: 0.0,
        opacity: 100.0,
        z: 0.0,
        rotation_x_deg: 0.0,
        rotation_y_deg: 0.0,
        three_d: false,
        ends_run: false,
        matte: None,
        blend: lumit_gpu::composite::Blend::Normal,
        mask_cov: None,
        pre: None,
        fx: Default::default(),
        fx_ids: Vec::new(),
        neighbours: Vec::new(),
        flow_fields: Vec::new(),
        colour_tables: Vec::new(),
        dof_inputs: Vec::new(),
        mattes: Vec::new(),
        mask_paths: Vec::new(),
        roto_mattes: Vec::new(),
        planes: Vec::new(),
        points_schedules: Vec::new(),
        flare_lens_files: Vec::new(),
        graph_fx: Vec::new(),
        fx_ref_width: None,
        fx_input_key: None,
        mb: Vec::new(),
        lights: Vec::new(),
        material: Default::default(),
        temporal_below: None,
        accumulation_below: None,
        flow_below: Vec::new(),
    }
}

/// Render the composite of `below` (the layers beneath a temporal adjustment,
/// in document order) at the held/sample comp time `tau`, reusing the
/// decoded `pixels_by_layer` the caller hands in — for a Posterize hold that
/// is the frame-time decode, so footage is held and only transforms, effects
/// and the camera re-resolve at `tau`; for an accumulation sample the caller
/// has already swapped in each clip's picture at that moment (docs/impl/
/// temporal-rerender.md §2). This is the one re-render both the preview
/// (`build_comp_draws` + [`Realiser::realise`]) and export drive, so a
/// Posterize Time or accumulation motion blur frame is identical in the
/// viewport and the file. Re-resolving decodes nothing here: every picture it
/// reads was planned with the frame, so the decode planner is never re-entered
/// (docs/impl/temporal-rerender.md Traps).
///
/// Temporal effects inside the below-stack (echo, flow motion blur, datamosh)
/// are held to a still here — their neighbour frames and flow fields are
/// dropped by [`strip_temporal_inputs`], because the held re-render reuses the
/// frame-time decode and export carries no neighbour decode for it. A
/// documented v1 boundary (docs/08 §3.25), matching the after-effects matte's
/// own temporal boundary.
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
/// drive, so the two re-render the identical stack. Footage is whatever
/// `pixels_by_layer` says it is at `tau` (held for a Posterize, the moment's
/// own picture for an accumulation sample); temporal effects in the
/// below-stack are dropped to stills ([`strip_temporal_inputs`]).
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
    below_draws_cloned(
        doc,
        comp,
        below,
        tau,
        frame_t,
        force_mb,
        pixels_by_layer,
        visited,
        lumit_core::fx::drivers::NO_CLONE,
        None,
    )
}

/// [`below_draws_at`] inside a clone picture: `clone` is which copy of how
/// many the walk above is rendered as, so the layers below read it too.
#[allow(clippy::too_many_arguments)]
fn below_draws_cloned(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    below: &[lumit_core::model::Layer],
    tau: f64,
    frame_t: f64,
    force_mb: Option<lumit_core::model::MotionBlur>,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
    clone: (u32, u32),
    keys: Option<&dyn crate::cache::NestedKeyer>,
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
    // motion-blur switch on, so per-layer motion blur smears each layer
    // in every sub-frame sample — the real comp is never touched. None leaves
    // the sample render exactly as before (Posterize, or accumulation without
    // the toggle).
    if let Some(mb) = force_mb {
        below_comp.motion_blur = mb;
        for l in &mut below_comp.layers {
            l.switches.motion_blur = true;
        }
    }
    // No nested-frame keyer: a held re-render strips the temporal
    // inputs below, so a Precomp in it is not the picture its name would claim.
    let mut draws = comp_walk(
        doc,
        &below_comp,
        tau,
        frame_t,
        pixels_by_layer,
        visited,
        keys,
        false,
        Walk {
            clone,
            values: &[],
            rebuild: true,
            seen_through: None,
        },
        None,
    );
    strip_temporal_inputs(&mut draws);
    (draws, Some(crate::track::camera_pose(doc, comp, tau)))
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
    clone: (u32, u32),
) -> Option<TemporalBelow> {
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let p = lumit_core::fx::temporal::stack_posterize(&layer.effects, layer.switches.fx, lt)?;
    // The below-render reach is implied by the carrier: only an
    // adjustment layer's Posterize holds the composite beneath it.
    if !layer.is_adjustment() {
        return None;
    }
    let tau = lumit_core::fx::temporal::posterize_held_time(t_comp, p.rate, p.phase);
    let below = &comp.layers[idx + 1..];
    // Posterize never forces per-layer motion blur (that is accumulation MB's
    // Force on all layers).
    let (draws, camera) = below_draws_cloned(
        doc,
        comp,
        below,
        tau,
        frame_t,
        None,
        pixels_by_layer,
        visited,
        clone,
        None,
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
/// export drives, so preview equals export. `frame_t` threads the
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
    clone: (u32, u32),
) -> Option<AccumulationBelow> {
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let p = lumit_core::fx::temporal::stack_accumulation_mb(&layer.effects, layer.switches.fx, lt)?;
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
            // The footage at this moment (docs/08 §3.26): the decode
            // planner fetched each covered clip at every offset of the
            // shutter, and this sample's below-stack reads those pixels in
            // place of the frame-time ones, so a clip playing under the
            // adjustment smears the way a moving layer does. A clip with no
            // picture for this moment (a Sequence clip, a dropped decode)
            // keeps its frame-time pixels, which is the held behaviour this
            // effect always had. References only: the moment's pixels live on
            // the frame-time entry, so no frame is copied here.
            let mut at_moment_by_layer = pixels_by_layer.clone();
            for lp in pixels_by_layer.values() {
                if let Some((_, Some(moment))) = lp
                    .shutter
                    .iter()
                    .find(|(o, _)| o.to_bits() == off.to_bits())
                {
                    at_moment_by_layer.insert(lp.layer, &**moment);
                }
            }
            below_draws_cloned(
                doc,
                comp,
                below,
                tau,
                frame_t,
                force_mb,
                &at_moment_by_layer,
                visited,
                clone,
                None,
            )
        })
        .collect();
    Some(AccumulationBelow {
        samples,
        mix: p.mix as f32,
        // Filled in by the caller, which has the layer-rendering helper this
        // free function does not.
        matte: LayerInputDraw::Absent,
        matte_channel: p.matte_channel,
        matte_invert: p.matte_invert,
        anchor: p.shutter_anchor() as f32,
    })
}

/// A clip's picture when the layer itself carries accumulation motion blur
/// (docs/08 §3.26): the clip averaged over its own shutter moments, which the
/// decode planner fetched onto [`CompLayerPixels::shutter`] for exactly this.
/// The frame-time pixels unchanged when the layer carries no live copy of the
/// effect, which is every ordinary layer, and that case hands back the decode's
/// own allocation rather than a copy of it.
///
/// A plain carrier has nothing beneath it to re-render, so this is what the
/// effect means there: the motion inside the footage, averaged the way a Blend
/// retime averages its two frames, in the decoded bytes. The layer's transform
/// is not sampled here; that is the motion blur switch's job, and the two
/// stack. A moment the worker could not make (a Sequence clip, a dropped
/// decode) stands in as the frame-time picture, and Mix blends the average
/// back toward it. The Matte is not read on a plain carrier.
///
/// ponytail: a scalar byte average, N reads of the frame on the CPU each
/// render. Sum the moments on the card if a profile ever shows it; the
/// planner and worker need not change.
fn own_shutter_average(
    layer: &lumit_core::model::Layer,
    lp: &CompLayerPixels,
    lt: f64,
) -> Arc<Vec<u8>> {
    let offsets = match lumit_core::fx::temporal::stack_accumulation_mb(
        &layer.effects,
        layer.switches.fx,
        lt,
    ) {
        Some(p) if !layer.is_adjustment() => (p.sample_offsets(), p.mix),
        _ => return Arc::clone(&lp.rgba),
    };
    let (offsets, mix) = offsets;
    if offsets.is_empty() {
        return Arc::clone(&lp.rgba);
    }
    let n = lp.rgba.len();
    let mut sum = vec![0u32; n];
    for off in &offsets {
        let moment = lp
            .shutter
            .iter()
            .find(|(o, _)| o.to_bits() == off.to_bits())
            .and_then(|(_, m)| m.as_deref())
            .map(|m| m.rgba.as_slice())
            .filter(|m| m.len() == n)
            .unwrap_or(&lp.rgba);
        for (s, &b) in sum.iter_mut().zip(moment) {
            *s += u32::from(b);
        }
    }
    let count = offsets.len() as u32;
    let average: Vec<u8> = sum
        .iter()
        .map(|&s| ((s + count / 2) / count) as u8)
        .collect();
    Arc::new(if mix >= 1.0 {
        average
    } else {
        lumit_core::pixels::blend_rgba(&lp.rgba, &average, mix as f32)
    })
}

/// The neighbour below-stacks a temporal effect on an **adjustment** layer
/// reads (docs/08 §3.2), empty when the layer carries no such effect.
///
/// An adjustment layer's picture is the composite of everything below it, which
/// the decode worker never sees — so Motion blur and Datamosh on one were
/// a silent passthrough, on exactly the layer docs/08 §3.2 calls the most common
/// place to put the effect. The answer is the one the temporal re-renders
/// already use: build the below-stack again at the neighbour time through the
/// shared `below_draws_at`, so preview and export measure the identical pair.
/// `idx` is the layer's document index, so the below-set is
/// `comp.layers[idx + 1..]`.
///
/// One entry per offset the stack asked for, in ascending order — Fast motion
/// blur's `+1` and Datamosh's `-1` are different measurements, and each consumer
/// gets its own. An Echo or a plugin that only reads the neighbour picture
/// gets its entry with no measurement asked for.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn adjustment_flow_below(
    doc: &Arc<lumit_core::model::Document>,
    comp: &lumit_core::model::Composition,
    layer: &lumit_core::model::Layer,
    idx: usize,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    visited: &mut Vec<uuid::Uuid>,
    clone: (u32, u32),
    keys: Option<&dyn crate::cache::NestedKeyer>,
) -> Vec<(
    i32,
    Vec<CompLayerDraw>,
    Option<lumit_core::model::CameraPose>,
    Option<i32>,
)> {
    // The comp's own frame interval: the shutter this effect smears over is a
    // frame of the render, not a frame of some source nobody here has.
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let lt = lumit_core::time::layer_time(t_comp, layer.start_offset.0);
    let below = &comp.layers[idx + 1..];
    let crosses = |o| {
        lumit_core::fx::temporal::cut_between(
            below,
            t_comp,
            lumit_core::time::frames_on(t_comp, o, dt),
        )
    };
    crate::plan::rebuild_offsets(layer, lt, dt, crosses)
        .into_iter()
        .map(|(offset, measure)| {
            let (draws, camera) = below_draws_cloned(
                doc,
                comp,
                below,
                lumit_core::time::frames_on(t_comp, offset, dt),
                frame_t,
                None,
                pixels_by_layer,
                visited,
                clone,
                keys,
            );
            (offset, draws, camera, measure)
        })
        .collect()
}

/// The decoded pixels a rebuild of `comp` at `t_comp` draws from, when that
/// is another moment than the frame's own `frame_t`: each clip the planner
/// fetched at that moment ([`CompLayerPixels::shutter`]) stands in for its
/// frame-time picture. `None` on an ordinary build, and when no clip here has
/// the moment, so the caller's map is used as it is and nothing is copied.
///
/// A clip with no picture for the moment (a dropped decode) keeps its
/// frame-time pixels, which is what every rebuild used to draw. A Sequence
/// layer in a gap at the moment is left out, so it draws nothing.
fn pixels_at_moment<'a>(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    t_comp: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &'a CompLayerPixels>,
) -> Option<std::collections::HashMap<uuid::Uuid, &'a CompLayerPixels>> {
    if t_comp.to_bits() == frame_t.to_bits() {
        return None;
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let offset = crate::plan::moment_offset(t_comp, frame_t, dt);
    let mut out = None;
    // What this comp's clips are filed under: its layers, or the boxes of
    // its node graph, whose Reads are keyed by their own ids.
    let boxes = comp.graph.iter().flat_map(|g| &g.nodes).map(|n| n.id());
    for id in comp.layers.iter().map(|l| l.id).chain(boxes) {
        let Some(lp) = pixels_by_layer.get(&id).copied() else {
            continue;
        };
        if let Some((_, moment)) = lp
            .shutter
            .iter()
            .find(|(o, _)| o.to_bits() == offset.to_bits())
        {
            let out = out.get_or_insert_with(|| pixels_by_layer.clone());
            match moment {
                Some(moment) => out.insert(id, &**moment),
                None => out.remove(&id),
            };
        }
    }
    // A graph a layer applies files its clips under its own boxes too, and
    // runs on that layer's clock. The planner fetched them by the offset on
    // that clock in frames of the graph's own comp, so that is the offset
    // they are looked up by.
    for layer in &comp.layers {
        let mut boxes = Vec::new();
        applied_boxes(doc, &layer.effects, &mut Vec::new(), &mut boxes);
        for (id, graph_dt) in boxes {
            let offset = crate::plan::moment_offset(
                lumit_core::time::layer_time(t_comp, layer.start_offset.0),
                lumit_core::time::layer_time(frame_t, layer.start_offset.0),
                graph_dt,
            );
            let moment = pixels_by_layer.get(&id).and_then(|lp| {
                lp.shutter
                    .iter()
                    .find(|(o, _)| o.to_bits() == offset.to_bits())
            });
            if let Some((_, moment)) = moment {
                let out = out.get_or_insert_with(|| pixels_by_layer.clone());
                match moment {
                    Some(moment) => out.insert(id, &**moment),
                    None => out.remove(&id),
                };
            }
        }
    }
    out
}

/// Whether a layer's own effects come out the same in a rebuild at another
/// moment as in that moment's own frame.
///
/// A rebuild gives a stack no neighbour frames and no flow, and holds an
/// effect that is not sampled in time at the frame's own time. A stack that
/// asks for none of those is the same either way.
fn plain_stack(layer: &lumit_core::model::Layer, lt: f64) -> bool {
    let fx_on = layer.switches.fx;
    let fx = &layer.effects;
    fx.iter().chain(&layer.styles).all(|e| e.sample_temporally)
        && !lumit_core::fx::temporal::stack_is_temporal(fx, fx_on)
        && lumit_core::fx::temporal::stack_flow_neighbours(fx, fx_on).is_empty()
        && lumit_core::fx::temporal::stack_posterize(fx, fx_on, lt).is_none()
        && lumit_core::fx::temporal::stack_accumulation_mb(fx, fx_on, lt).is_none()
        // Both of these read footage at other moments by roads of their own.
        && fx
            .iter()
            .all(|e| !matches!(e.effect.match_name.as_str(), "clone_to_points" | "node_graph"))
}

/// Whether `comp`, built again at `t` inside a frame whose own time in it is
/// `frame_t`, is pixel for pixel the picture its own frame at `t` shows.
///
/// That is what lets a rebuild's Precomp keep its frame's name. It answers
/// yes only for what is known to be the same. Every layer the walk draws at
/// `t`, or holds pixels for, is footage the planner fetched at that moment
/// and picks one whole frame of, another such comp, or a kind with no decoded
/// picture at all, and none carries a stack a rebuild treats differently
/// ([`plain_stack`]). Anything else is a no, and the Precomp is built
/// unnamed as it always was.
fn rebuilt_as_itself(
    doc: &lumit_core::model::Document,
    comp: &lumit_core::model::Composition,
    t: f64,
    frame_t: f64,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &CompLayerPixels>,
    depth: u32,
) -> bool {
    use lumit_core::model::LayerKind;
    // A node graph files its clips under its own boxes, a group's effects
    // are a stack nobody asked about here, and a cycle has no answer. None is
    // worth following.
    if comp.graph.is_some() || comp.groups.iter().any(|g| !g.effects.is_empty()) || depth > 8 {
        return false;
    }
    let dt = 1.0 / comp.frame_rate.fps().max(1.0);
    let offset = crate::plan::moment_offset(t, frame_t, dt);
    // The walk's own questions, asked at `t` as it asks them.
    let any_solo = comp.layers.iter().any(|l| l.switches.solo);
    let occluder = lumit_core::occlusion::occluder_index(doc, comp, t);
    comp.layers.iter().enumerate().all(|(idx, layer)| {
        if layer.audio_only || t < layer.in_point.0.to_f64() || t >= layer.out_point.0.to_f64() {
            return true;
        }
        let drawn = layer.switches.visible
            && !layer.graph.out_unwired
            && !(any_solo && !layer.switches.solo)
            && !occluder.is_some_and(|o| idx > o);
        let fetched = pixels_by_layer.get(&layer.id);
        // Not drawn and nothing fetched for it: no layer reads it either, so
        // it is nothing in both.
        if !drawn && fetched.is_none() && !matches!(layer.kind, LayerKind::Precomp { .. }) {
            return true;
        }
        let lt = lumit_core::time::layer_time(t, layer.start_offset.0);
        if !plain_stack(layer, lt) {
            return false;
        }
        // An adjustment draws the composite beneath it, whatever its kind.
        if layer.is_adjustment() || backdrop_fx(layer) {
            return true;
        }
        match &layer.kind {
            // The moment's picture is one whole source frame. The frame's own
            // is too only when nothing blends or synthesises between two.
            LayerKind::Footage { .. } => fetched.is_some_and(|lp| {
                offset == 0.0
                    || (matches!(
                        layer.interpolation,
                        lumit_core::retime::Interpolation::Nearest
                    ) && lp
                        .shutter
                        .iter()
                        .any(|(o, moment)| o.to_bits() == offset.to_bits() && moment.is_some()))
            }),
            // Spliced into its parent, a collapsed one is walked with no
            // cull, which is another set of questions than these.
            LayerKind::Precomp { comp: id } => doc.comp(*id).is_some_and(|nested| {
                let frame_lt = lumit_core::time::layer_time(frame_t, layer.start_offset.0);
                !matches!(
                    lumit_core::model::collapse_state(doc, comp, layer, lt),
                    lumit_core::model::CollapseState::Active
                ) && rebuilt_as_itself(
                    doc,
                    nested,
                    lumit_core::model::nested_source_time(layer, nested, lt),
                    lumit_core::model::nested_source_time(layer, nested, frame_lt),
                    pixels_by_layer,
                    depth + 1,
                )
            }),
            LayerKind::Solid { .. }
            | LayerKind::Text { .. }
            | LayerKind::Shape { .. }
            | LayerKind::Adjustment
            | LayerKind::Null
            | LayerKind::Camera { .. }
            | LayerKind::Light { .. } => true,
            LayerKind::Sequence { .. } => false,
        }
    })
}

/// `pixels_by_layer` without the Sequence layers of `comp` that are in a gap
/// at the frame's own time. The planner fetches such a layer for the clip it
/// shows at another moment, and files the frame's own moment as a gap
/// ([`crate::decode::Cut::Moment`]). `None` when there are none, which is
/// nearly always, so nothing is copied.
fn without_gaps<'a>(
    comp: &lumit_core::model::Composition,
    pixels_by_layer: &std::collections::HashMap<uuid::Uuid, &'a CompLayerPixels>,
) -> Option<std::collections::HashMap<uuid::Uuid, &'a CompLayerPixels>> {
    let mut out = None;
    for layer in &comp.layers {
        let in_gap = pixels_by_layer.get(&layer.id).is_some_and(|lp| {
            let mut moments = lp.shutter.iter();
            moments.any(|(offset, clip)| *offset == 0.0 && clip.is_none())
        });
        if in_gap {
            let out = out.get_or_insert_with(|| pixels_by_layer.clone());
            out.remove(&layer.id);
        }
    }
    out
}

/// Every box of the node graphs `effects` apply, graphs nested in them
/// included, each with one frame of its own comp in seconds. A graph's Read
/// of footage is filed under its box. `seen` stops a graph that applies
/// itself.
fn applied_boxes(
    doc: &lumit_core::model::Document,
    effects: &[lumit_core::model::EffectInstance],
    seen: &mut Vec<Uuid>,
    out: &mut Vec<(Uuid, f64)>,
) {
    for e in effects.iter().filter(|e| e.enabled) {
        let Some(named) = lumit_core::fx::effects::node_graph::comp_of(e)
            .filter(|id| !seen.contains(id))
            .and_then(|id| doc.comp(id))
        else {
            continue;
        };
        let Some(graph) = named.graph.as_ref() else {
            continue;
        };
        seen.push(named.id);
        let dt = 1.0 / named.frame_rate.fps().max(1.0);
        for node in &graph.nodes {
            out.push((node.id(), dt));
            if let lumit_core::comp_graph::GraphNode::Fx(inst) = node {
                applied_boxes(doc, std::slice::from_ref(inst), seen, out);
            }
        }
    }
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
        d.flow_fields = Vec::new();
        // The composite measurement (§3.2) is temporal too. A walk that is a
        // rebuild never builds one (`Walk::rebuild`), which is what bounds the
        // work. This is for a draw that came from anywhere else.
        d.flow_below = Vec::new();
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
    use lumit_core::{time::CompTime, time::Duration, time::FrameRate, time::Rational};

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
            pan: lumit_core::anim::Property::zero(),
            audio_only: false,
            audio_streams: Default::default(),
            adjustment: false,
            pitch_follows_speed: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: BlendMode::Normal,
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn place_of(l: &Layer) -> [[f32; 4]; 4] {
        let tr = &l.transform;
        lumit_gpu::composite::place_matrix(
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
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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
        let expected = lumit_gpu::composite::concat_place(place_of(&gp), place_of(&parent));
        assert_eq!(world, expected);
        // A 2D grandparent's hidden z and tilt leave the chain as it was.
        let mut tilted = gp.clone();
        tilted.transform.position_z = Property::fixed(-50.0);
        tilted.transform.rotation_x = Property::fixed(60.0);
        let c = comp(vec![tilted, parent, child.clone()]);
        let flat = parent_world_placement(&c, &child, 0.0, Arc::new(ExpressionContext::detached()));
        assert_eq!(flat, Some(expected));

        // A layer on a two-node camera turns with its aim, and an imported
        // camera's anchor (After Effects' point of interest) moves nothing.
        let mut camera = layer(100.0, 50.0, None);
        camera.transform.anchor_x = Property::fixed(400.0);
        camera.transform.position_z = Property::fixed(-300.0);
        camera.kind = LayerKind::Camera {
            zoom: Property::fixed(800.0),
            solve_link: None,
            correction_base: None,
            options: Box::new(CameraOptions {
                two_node: true,
                ..CameraOptions::fresh(800.0, (400.0, 0.0, 0.0))
            }),
        };
        let mut hud = layer(0.0, 0.0, Some(camera.id));
        hud.switches.three_d = true;
        let c = comp(vec![camera, hud.clone()]);
        let aim =
            lumit_core::camera::look_at((100.0, 50.0, -300.0), (400.0, 0.0, 0.0), (0.0, 0.0, 0.0))
                .unwrap();
        let expected = lumit_gpu::composite::place_matrix(
            (100.0, 50.0),
            (0.0, 0.0),
            (100.0, 100.0),
            aim.2 as f32,
            -300.0,
            aim.0 as f32,
            aim.1 as f32,
        );
        let on_camera =
            parent_world_placement(&c, &hud, 0.0, Arc::new(ExpressionContext::detached()));
        assert_eq!(on_camera, Some(expected));
    }

    #[test]
    fn taking_and_losing_a_parent_leaves_the_layer_where_it_was() {
        let ctx = || Arc::new(ExpressionContext::detached());
        let world = |c: &Composition, l: &Layer| {
            let own = own_placement(l, 0.0, &ctx());
            match parent_world_placement(c, l, 0.0, ctx()) {
                Some(pre) => lumit_gpu::composite::concat_place(pre, own),
                None => own,
            }
        };
        // A scaled parent and a tilted one are separate rigs: scale has no z,
        // so a tilt under a scaled parent has no exact answer.
        for tilted in [false, true] {
            let (tilt, scale) = if tilted { (15.0, 100.0) } else { (0.0, 50.0) };
            let mut gp = layer(10.0, 20.0, None);
            gp.transform.rotation_y = Property::fixed(tilt);
            let mut parent = layer(100.0, 40.0, Some(gp.id));
            parent.transform.anchor_x = Property::fixed(7.0);
            parent.transform.rotation = Property::fixed(30.0);
            parent.transform.scale_x = Property::fixed(scale);
            parent.transform.scale_y = Property::fixed(scale);
            // Flipped, more than a turn round and tilted: the awkward ones to
            // read back out of a matrix.
            let mut child = layer(5.0, 60.0, None);
            child.transform.anchor_y = Property::fixed(4.0);
            child.transform.rotation = Property::fixed(400.0);
            child.transform.rotation_x = Property::fixed(tilt);
            child.transform.scale_x = Property::fixed(80.0);
            child.transform.scale_y = Property::fixed(-80.0);
            // A tilt only counts on a 3D layer.
            gp.switches.three_d = tilted;
            child.switches.three_d = tilted;

            let before = world(&comp(vec![child.clone()]), &child);
            for new_parent in [Some(parent.id), None] {
                let c = comp(vec![gp.clone(), parent.clone(), child.clone()]);
                let edits = reparented(&c, &child, new_parent, 0.0, ctx());
                assert!(!edits.is_empty());
                for (prop, animation) in edits {
                    child.transform.get_mut(prop).unwrap().animation = animation;
                }
                child.parent = new_parent;
                let c = comp(vec![gp.clone(), parent.clone(), child.clone()]);
                let after = world(&c, &child);
                for (b, a) in before.iter().flatten().zip(after.iter().flatten()) {
                    assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
                }
            }
            // Back where it started, in the numbers too.
            assert!((child.transform.rotation.value_at(0.0) - 400.0).abs() < 1e-2);
            assert!((child.transform.scale_y.value_at(0.0) + 80.0).abs() < 1e-2);

            // A camera keeps its eye, its turn and what it is aimed at, under
            // the same parents.
            for two_node in [false, true] {
                let mut camera = layer(30.0, 70.0, None);
                camera.kind = LayerKind::Camera {
                    zoom: Property::fixed(800.0),
                    solve_link: None,
                    correction_base: None,
                    options: Box::new(CameraOptions {
                        two_node,
                        ..CameraOptions::fresh(800.0, (90.0, 20.0, 300.0))
                    }),
                };
                camera.transform.position_z = Property::fixed(-500.0);
                camera.transform.rotation_x = Property::fixed(12.0);
                camera.transform.rotation = Property::fixed(-25.0);
                let seen = |c: &Composition, l: &Layer| {
                    let pose = c.camera_pose(0.0).unwrap();
                    let own = |p: TransformProp| l.prop(p).unwrap().value_at(0.0);
                    let poi = (
                        own(TransformProp::PoiX),
                        own(TransformProp::PoiY),
                        own(TransformProp::PoiZ),
                    );
                    // A one-node camera's point of interest is inert.
                    let context = ctx();
                    let read = lumit_core::model::context_reader(&context, 0.0);
                    let poi = match parent_space(c, l, 0.0, &read, None) {
                        _ if !two_node => (0.0, 0.0, 0.0),
                        Some(s) => s.point(poi),
                        None => poi,
                    };
                    let axes = lumit_core::camera::axes(pose.rotation_deg);
                    [pose.position, poi, axes[0], axes[1], axes[2]]
                };
                let before = seen(&comp(vec![camera.clone()]), &camera);
                for new_parent in [Some(parent.id), None] {
                    let c = comp(vec![camera.clone(), gp.clone(), parent.clone()]);
                    let edits = reparented(&c, &camera, new_parent, 0.0, ctx());
                    assert!(!edits.is_empty());
                    for (prop, animation) in edits {
                        camera.prop_mut(prop).unwrap().animation = animation;
                    }
                    camera.parent = new_parent;
                    let c = comp(vec![camera.clone(), gp.clone(), parent.clone()]);
                    let after = seen(&c, &camera);
                    for (b, a) in before.iter().zip(&after) {
                        let gap = (b.0 - a.0).abs() + (b.1 - a.1).abs() + (b.2 - a.2).abs();
                        assert!(gap < 1e-2, "{two_node}: {before:?} became {after:?}");
                    }
                }
            }
        }

        // A keyed camera keeps its whole move under a null turned a quarter
        // round, where x and z trade places: every key is carried, not just
        // the one under the playhead, and an eased move keeps its ease.
        let mut null = layer(200.0, 100.0, None);
        null.switches.three_d = true;
        null.transform.rotation_y = Property::fixed(90.0);
        let mut camera = layer(0.0, 50.0, None);
        camera.kind = LayerKind::Camera {
            zoom: Property::fixed(800.0),
            solve_link: None,
            correction_base: None,
            options: Box::new(CameraOptions::fresh(800.0, (0.0, 0.0, 0.0))),
        };
        let key = |s: i64, v: f64| lumit_core::anim::Keyframe {
            time: Rational::new(s, 1).unwrap(),
            value: v,
            interp_in: lumit_core::anim::SideInterp::Linear,
            interp_out: lumit_core::anim::SideInterp::Linear,
        };
        let keyed = |a: f64, b: f64| Property {
            animation: lumit_core::anim::Animation::Keyframed(vec![key(0, a), key(2, b)]),
            ..Property::fixed(0.0)
        };
        let eased = |a: f64, b: f64, speed: f64| {
            let ease = lumit_core::anim::SideInterp::Bezier {
                speed,
                influence: 0.4,
            };
            let mut keys = [key(0, a), key(2, b)];
            keys[0].interp_out = ease;
            keys[1].interp_in = ease;
            Property {
                animation: lumit_core::anim::Animation::Keyframed(keys.to_vec()),
                ..Property::fixed(0.0)
            }
        };
        camera.transform.position_x = eased(0.0, 300.0, 400.0);
        camera.transform.position_y = eased(50.0, 50.0, 0.0);
        camera.transform.position_z = eased(-500.0, -200.0, -100.0);
        camera.transform.rotation_y = keyed(0.0, 40.0);
        camera.transform.orientation[0] = Property::fixed(10.0);
        camera.transform.orientation[2] = Property {
            animation: lumit_core::anim::Animation::Keyframed(vec![key(1, 0.0), key(3, 30.0)]),
            ..Property::fixed(0.0)
        };
        let poses = |c: &Composition| [0.0, 0.5, 1.0, 2.0].map(|t| c.camera_pose(t).unwrap());
        let before = poses(&comp(vec![camera.clone(), null.clone()]));
        let c = comp(vec![camera.clone(), null.clone()]);
        for (prop, animation) in reparented(&c, &camera, Some(null.id), 1.0, ctx()) {
            camera.prop_mut(prop).unwrap().animation = animation;
        }
        camera.parent = Some(null.id);
        let after = poses(&comp(vec![camera.clone(), null.clone()]));
        for (b, a) in before.iter().zip(&after) {
            let (pb, pa) = (b.position, a.position);
            let gap = (pb.0 - pa.0).abs() + (pb.1 - pa.1).abs() + (pb.2 - pa.2).abs();
            // Every axis, so a roll shows as well as where it looks.
            let (ab, aa) = (
                lumit_core::camera::axes(b.rotation_deg),
                lumit_core::camera::axes(a.rotation_deg),
            );
            let turn: f64 = ab
                .iter()
                .zip(&aa)
                .map(|(u, v)| (u.0 - v.0).abs() + (u.1 - v.1).abs() + (u.2 - v.2).abs())
                .sum();
            assert!(gap < 1e-2 && turn < 1e-6, "{b:?} became {a:?}");
        }
        // The rows keep their keys, and the Orientation keeps its own two
        // rather than gaining one at every position key.
        assert_eq!(camera.transform.rotation_y, keyed(0.0, 40.0));
        for p in &camera.transform.orientation {
            let times: Vec<_> = match &p.animation {
                lumit_core::anim::Animation::Keyframed(keys) => {
                    keys.iter().map(|k| k.time.to_f64()).collect()
                }
                _ => Vec::new(),
            };
            assert_eq!(times, [1.0, 3.0], "{p:?}");
        }

        // After Effects' own numbers: a 3D layer put under a turned null
        // keeps its X, Y and Z Rotation, and Orientation takes the turn.
        let mut rider = layer(1000.0, 600.0, None);
        rider.switches.three_d = true;
        rider.transform.after_effects_order = true;
        rider.transform.anchor_x = Property::fixed(400.0);
        rider.transform.anchor_y = Property::fixed(200.0);
        rider.transform.rotation_x = Property::fixed(30.0);
        rider.transform.rotation_y = Property::fixed(40.0);
        let mut turner = layer(800.0, 500.0, None);
        turner.switches.three_d = true;
        turner.transform.position_z = Property::fixed(100.0);
        turner.transform.orientation[0] = Property::fixed(20.0);
        turner.transform.rotation_y = Property::fixed(25.0);
        let before = world(&comp(vec![rider.clone()]), &rider);
        let c = comp(vec![rider.clone(), turner.clone()]);
        for (prop, animation) in reparented(&c, &rider, Some(turner.id), 0.0, ctx()) {
            rider.transform.get_mut(prop).unwrap().animation = animation;
        }
        rider.parent = Some(turner.id);
        let after = world(&comp(vec![rider.clone(), turner.clone()]), &rider);
        for (b, a) in before.iter().flatten().zip(after.iter().flatten()) {
            assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
        }
        let tr = &rider.transform;
        let read = |p: &Property| p.value_at(0.0);
        let turn = |a: f64, b: f64| ((a - b) % 360.0 + 360.0) % 360.0;
        for (got, want) in [
            (read(&tr.rotation_x), 30.0),
            (read(&tr.rotation_y), 40.0),
            (read(&tr.rotation), 0.0),
            (read(&tr.orientation[0]), 338.1198),
            (read(&tr.orientation[1]), 336.6010),
            (read(&tr.orientation[2]), 350.9384),
        ] {
            let off = turn(got, want);
            assert!(off.min(360.0 - off) < 1e-2, "{got} for {want}");
        }
        let at = (
            read(&tr.position_x),
            read(&tr.position_y),
            read(&tr.position_z),
        );
        let want = (235.4291, 59.7672, -31.6390);
        let gap = (at.0 - want.0).abs() + (at.1 - want.1).abs() + (at.2 - want.2).abs();
        assert!(gap < 1e-2, "{at:?}");

        // Under a mirrored parent with an Orientation of its own, there and
        // back again.
        let mut mirror = turner.clone();
        mirror.id = uuid::Uuid::now_v7();
        mirror.transform.scale_x = Property::fixed(-100.0);
        mirror.transform.orientation = [15.0, -30.0, 10.0].map(Property::fixed);
        let mut card = layer(1000.0, 600.0, None);
        card.switches.three_d = true;
        card.transform.anchor_x = Property::fixed(400.0);
        card.transform.anchor_y = Property::fixed(200.0);
        card.transform.rotation_x = Property::fixed(30.0);
        card.transform.rotation_y = Property::fixed(40.0);
        card.transform.orientation = [5.0, 0.0, 20.0].map(Property::fixed);
        let before = world(&comp(vec![card.clone()]), &card);
        for new_parent in [Some(mirror.id), None] {
            let c = comp(vec![card.clone(), mirror.clone()]);
            for (prop, animation) in reparented(&c, &card, new_parent, 0.0, ctx()) {
                card.transform.get_mut(prop).unwrap().animation = animation;
            }
            card.parent = new_parent;
            let after = world(&comp(vec![card.clone(), mirror.clone()]), &card);
            for (b, a) in before.iter().flatten().zip(after.iter().flatten()) {
                assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
            }
        }

        // A keyed Orientation has every key turned, so the whole move stays.
        let mut spun = layer(1000.0, 600.0, None);
        spun.switches.three_d = true;
        spun.transform.rotation_x = keyed(30.0, -10.0);
        spun.transform.orientation = [keyed(0.0, 60.0), keyed(10.0, 80.0), keyed(0.0, 20.0)];
        let worlds = |c: &Composition, l: &Layer| {
            [0.0, 0.5, 1.0, 2.0].map(|t| {
                let own = own_placement(l, t, &ctx());
                match parent_world_placement(c, l, t, ctx()) {
                    Some(pre) => lumit_gpu::composite::concat_place(pre, own),
                    None => own,
                }
            })
        };
        let before = worlds(&comp(vec![spun.clone()]), &spun);
        let c = comp(vec![spun.clone(), turner.clone()]);
        for (prop, animation) in reparented(&c, &spun, Some(turner.id), 1.0, ctx()) {
            assert!(!matches!(prop, TransformProp::RotationX));
            spun.transform.get_mut(prop).unwrap().animation = animation;
        }
        spun.parent = Some(turner.id);
        let after = worlds(&comp(vec![spun.clone(), turner.clone()]), &spun);
        for (b, a) in before
            .iter()
            .flatten()
            .flatten()
            .zip(after.iter().flatten().flatten())
        {
            assert!((b - a).abs() < 1e-2, "{before:?} became {after:?}");
        }
    }

    /// A 2D layer pinned to a 3D null with `toComp` sits where the compositor
    /// draws the null, through parents and a turned camera, and `fromComp`
    /// takes a picture point back to the null's own pixels. A 2D parent's
    /// leftover z and tilt count for nothing, as they do in the picture.
    /// Orientation, Anchor Z and Scale Z are on the way, and the picture's own
    /// placement, with Anchor Z in its position, lands where the walk does.
    #[test]
    #[allow(clippy::panic)]
    fn to_comp_lands_where_the_compositor_draws_and_from_comp_takes_it_back() {
        use lumit_core::expression::{evaluate_value_in, script::Language, ExprValue};

        let mut flat = layer(40.0, -30.0, None);
        flat.transform.position_z = Property::fixed(-50.0);
        flat.transform.rotation_x = Property::fixed(60.0);
        flat.transform.rotation = Property::fixed(10.0);
        let mut card = layer(15.0, 25.0, Some(flat.id));
        card.name = "Card".into();
        card.transform.rotation_y = Property::fixed(40.0);
        let mut rig = layer(900.0, 500.0, Some(flat.id));
        rig.switches.three_d = true;
        rig.transform.position_z = Property::fixed(300.0);
        rig.transform.anchor_x = Property::fixed(20.0);
        rig.transform.rotation = Property::fixed(-12.0);
        rig.transform.rotation_y = Property::fixed(35.0);
        rig.transform.scale_x = Property::fixed(150.0);
        rig.transform.scale_y = Property::fixed(80.0);
        rig.transform.orientation = [10.0, -25.0, 5.0].map(Property::fixed);
        rig.transform.anchor_z = Property::fixed(30.0);
        rig.transform.scale_z = Property::fixed(60.0);
        let mut marker = layer(120.0, -60.0, Some(rig.id));
        marker.name = "Marker".into();
        marker.switches.three_d = true;
        marker.transform.position_z = Property::fixed(-80.0);
        marker.transform.rotation_x = Property::fixed(20.0);
        marker.transform.orientation[1] = Property::fixed(20.0);
        marker.transform.anchor_z = Property::fixed(-15.0);
        let mut camera = layer(700.0, 450.0, None);
        camera.kind = LayerKind::Camera {
            zoom: Property::fixed(1400.0),
            options: Box::default(),
            solve_link: None,
            correction_base: None,
        };
        camera.transform.position_z = Property::fixed(-1500.0);
        camera.transform.rotation_x = Property::fixed(-10.0);
        camera.transform.rotation_y = Property::fixed(15.0);
        camera.transform.rotation = Property::fixed(5.0);
        // Below the active camera, a two-node one with a layer on it, and a
        // null moved by an expression on time with a layer on that.
        let mut aimer = layer(300.0, 200.0, None);
        aimer.kind = LayerKind::Camera {
            zoom: Property::fixed(900.0),
            options: Box::new(CameraOptions {
                two_node: true,
                ..CameraOptions::fresh(900.0, (800.0, 600.0, 400.0))
            }),
            solve_link: None,
            correction_base: None,
        };
        aimer.transform.position_z = Property::fixed(-700.0);
        aimer.transform.rotation = Property::fixed(20.0);
        let mut hud = layer(50.0, -40.0, Some(aimer.id));
        hud.name = "Hud".into();
        hud.switches.three_d = true;
        hud.transform.position_z = Property::fixed(600.0);
        let mut drift = layer(0.0, 300.0, None);
        drift.switches.three_d = true;
        drift.transform.position_x = Property {
            animation: lumit_core::anim::Animation::Expression("100.0 + time * 40.0".into()),
            ..Property::fixed(0.0)
        };
        drift.transform.rotation_y = Property::fixed(25.0);
        let mut rider = layer(60.0, 10.0, Some(drift.id));
        rider.name = "Rider".into();
        rider.switches.three_d = true;
        let mut c = comp(vec![
            camera,
            flat,
            card.clone(),
            rig,
            marker.clone(),
            aimer,
            hud,
            drift,
            rider,
        ]);
        (c.width, c.height) = (1920, 1080);
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(c.clone()));
        let context = Arc::new(ExpressionContext {
            document: Arc::new(doc),
            comp: Some(c.id),
            layer: None,
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        });
        let run = |language, text: &str| match evaluate_value_in(
            language,
            text,
            Some(context.clone()),
            &[],
        ) {
            Ok(ExprValue::Number(n)) => n,
            other => panic!("{text}: {other:?}"),
        };

        let world = world_placement(&c, &marker, 0.0, context.clone());
        let d = drawn(&marker, 0.0, &context);
        let own = lumit_gpu::composite::place_matrix(
            d.position,
            d.anchor,
            d.scale,
            d.rotation_deg,
            d.z,
            d.rotation_x_deg,
            d.rotation_y_deg,
        );
        let pre = parent_world_placement(&c, &marker, 0.0, context.clone()).unwrap();
        let picture = lumit_gpu::composite::concat_place(pre, own);
        for (x, y) in [(0.0, 0.0), (64.0, 0.0), (0.0, 48.0)] {
            let at = |m: &[[f32; 4]; 4], r: usize| m[0][r] * x + m[1][r] * y + m[3][r];
            for r in 0..3 {
                assert!((at(&world, r) - at(&picture, r)).abs() < 1e-2, "{x}, {y}");
            }
        }
        let pose = c.camera_pose(0.0).unwrap();
        let cam = crate::export::camera_mat(c.width, c.height, pose);
        let a = (cam * glam::Mat4::from_cols_array_2d(&world)).to_cols_array_2d();
        // After Effects' third number, zoom · (1 − 1/z), from the same divide:
        // the compositor's w is z over the zoom.
        let depth = pose.zoom - 1.0 / f64::from(a[3][3]);
        let drawn = [
            f64::from(a[3][0] / a[3][3]),
            f64::from(a[3][1] / a[3][3]),
            depth,
        ];
        for (axis, want) in drawn.into_iter().enumerate() {
            let js = run(
                Language::JavaScript,
                &format!("thisComp.layer('Marker').toComp([0, 0, 0])[{axis}]"),
            );
            let rhai = run(
                Language::Rhai,
                &format!("layer(\"Marker\").to_comp([0, 0, 0])[{axis}]"),
            );
            for got in [js, rhai] {
                assert!((got - want).abs() < 1e-2, "{got} is not {want}");
            }
        }
        // A 2D layer is drawn flat, with no camera.
        let flat_world = world_placement(&c, &card, 0.0, context.clone());
        let on_card = |r: usize| flat_world[0][r] * 5.0 + flat_world[1][r] * 7.0 + flat_world[3][r];
        for (axis, want) in [on_card(0), on_card(1)].into_iter().enumerate() {
            let got = run(
                Language::JavaScript,
                &format!("thisComp.layer('Card').toComp([5, 7])[{axis}]"),
            );
            assert!((got - f64::from(want)).abs() < 1e-2, "{got} is not {want}");
        }
        for (axis, want) in [30.0, -40.0, 25.0].into_iter().enumerate() {
            let back = run(
                Language::JavaScript,
                &format!(
                    "var m = thisComp.layer('Marker'); m.fromComp(m.toComp([30, -40, 25]))[{axis}]"
                ),
            );
            assert!((back - want).abs() < 1e-6, "{back} is not {want}");
        }
        for (axis, want) in [30.0, -40.0].into_iter().enumerate() {
            let back = run(
                Language::JavaScript,
                &format!(
                    "var m = thisComp.layer('Marker'); m.fromCompToSurface(m.toComp([30, -40]))[{axis}]"
                ),
            );
            assert!((back - want).abs() < 1e-6, "{back} is not {want}");
        }

        // A layer on a two-node camera turns with the camera's aim, and one
        // on a null moved by `time` follows it at that time.
        let later = Arc::new(ExpressionContext {
            comp_time: 0.5,
            ..(*context).clone()
        });
        for (name, t, at) in [("Hud", 0.0, &context), ("Rider", 0.5, &later)] {
            let l = c.layers.iter().find(|l| l.name == name).unwrap();
            let world = world_placement(&c, l, t, at.clone());
            let a = (cam * glam::Mat4::from_cols_array_2d(&world)).to_cols_array_2d();
            for axis in 0..2 {
                let want = a[3][axis] / a[3][3];
                let text = format!("thisComp.layer('{name}').toComp([0, 0, 0])[{axis}]");
                let got =
                    match evaluate_value_in(Language::JavaScript, &text, Some(at.clone()), &[]) {
                        Ok(ExprValue::Number(n)) => n,
                        other => panic!("{text}: {other:?}"),
                    };
                assert!(
                    (got - f64::from(want)).abs() < 1e-2,
                    "{name}: {got} is not {want}"
                );
            }
        }
    }

    /// The rig of aecheck's tocomp-check.jsx, and After Effects' own numbers
    /// for it: toComp's third number, fromComp off it, the Vec forms, a point
    /// behind the eye, and the facing sign of toCompVec.
    #[test]
    #[allow(clippy::panic)]
    fn to_comp_reads_what_after_effects_reads() {
        use lumit_core::expression::{evaluate_value_in, script::Language, ExprValue};

        let mut rig = layer(900.0, 500.0, None);
        rig.switches.three_d = true;
        rig.transform.position_z = Property::fixed(300.0);
        rig.transform.anchor_x = Property::fixed(20.0);
        rig.transform.rotation = Property::fixed(-12.0);
        rig.transform.rotation_y = Property::fixed(35.0);
        rig.transform.scale_x = Property::fixed(150.0);
        rig.transform.scale_y = Property::fixed(80.0);
        let mut marker = layer(120.0, -60.0, Some(rig.id));
        marker.name = "Marker".into();
        marker.switches.three_d = true;
        marker.transform.position_z = Property::fixed(-80.0);
        marker.transform.rotation_x = Property::fixed(20.0);
        let mut face = layer(960.0, 540.0, None);
        face.name = "Face".into();
        face.switches.three_d = true;
        let mut back = face.clone();
        back.id = uuid::Uuid::now_v7();
        back.name = "Back".into();
        back.transform.rotation_y = Property::fixed(180.0);
        let mut camera = layer(700.0, 450.0, None);
        camera.kind = LayerKind::Camera {
            zoom: Property::fixed(1400.0),
            options: Box::default(),
            solve_link: None,
            correction_base: None,
        };
        camera.transform.position_z = Property::fixed(-1500.0);
        camera.transform.rotation_y = Property::fixed(15.0);
        camera.transform.rotation = Property::fixed(5.0);
        let mut c = comp(vec![camera, rig, marker, face, back]);
        (c.width, c.height) = (1920, 1080);
        let pose = c.camera_pose(0.0).unwrap();
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(c.clone()));
        let context = Arc::new(ExpressionContext {
            document: Arc::new(doc),
            comp: Some(c.id),
            layer: None,
            comp_time: 0.0,
            current_depth: 0,
            inputs: None,
        });
        let run = |language, text: &str| match evaluate_value_in(
            language,
            text,
            Some(context.clone()),
            &[],
        ) {
            Ok(ExprValue::Number(n)) => n,
            other => panic!("{text}: {other:?}"),
        };

        // 100 px behind the eye, as the camera sees it.
        let m = lumit_core::camera::rotation(pose.rotation_deg);
        let a = (30.0, 20.0, -100.0);
        let eye = [pose.position.0, pose.position.1, pose.position.2];
        let at = |r: usize| eye[r] + m[r][0] * a.0 + m[r][1] * a.1 + m[r][2] * a.2;
        let behind = format!("[{}, {}, {}]", at(0), at(1), at(2));
        let behind_js = format!("toComp(thisComp.layer('Marker').fromWorld({behind}))");
        let behind_rhai = format!("to_comp(layer(\"Marker\").from_world({behind}))");
        for (in_js, in_rhai, want) in [
            (
                "toComp([0, 0, 0])",
                "to_comp([0, 0, 0])",
                [814.5410, 529.0258, 1399.1609],
            ),
            (
                "toComp([30, -40, 25])",
                "to_comp([30, -40, 25])",
                [842.3415, 488.1836, 1399.1593],
            ),
            (
                "fromComp([960, 540, 0])",
                "from_comp([960, 540, 0])",
                [473.1247, -304.8729, -1494.0807],
            ),
            (
                "fromComp([960, 540])",
                "from_comp([960, 540])",
                [473.1247, -304.8729, -1494.0807],
            ),
            (
                "fromComp([960, 540, -1400])",
                "from_comp([960, 540, -1400])",
                [473.2362, -304.9918, -1494.5374],
            ),
            (
                "fromComp([100, 900, 1399])",
                "from_comp([100, 900, 1399])",
                [-422.4201, 46.1342, -546.0801],
            ),
            (
                "toCompVec([10, 20, 30])",
                "to_comp_vec([10, 20, 30])",
                [20.9047, 7.2530, 46218.5615],
            ),
            (
                "fromCompVec([10, 20, 30])",
                "from_comp_vec([10, 20, 30])",
                [-17.5206, 29.4023, 36.8613],
            ),
            (
                "fromCompVec([10, 20])",
                "from_comp_vec([10, 20])",
                [-2.4660, 21.3642, 9.7869],
            ),
            (
                behind_js.as_str(),
                behind_rhai.as_str(),
                [540.0, 260.0, 1414.0],
            ),
        ] {
            for (axis, want) in want.into_iter().enumerate() {
                let js = run(
                    Language::JavaScript,
                    &format!("thisComp.layer('Marker').{in_js}[{axis}]"),
                );
                let rhai = run(
                    Language::Rhai,
                    &format!("layer(\"Marker\").{in_rhai}[{axis}]"),
                );
                for got in [js, rhai] {
                    assert!((got - want).abs() < 2e-3, "{in_js}: {got} is not {want}");
                }
            }
        }
        // The manual's promise: above nought while the front faces the camera.
        let facing = |name: &str| {
            run(
                Language::JavaScript,
                &format!("thisComp.layer('{name}').toCompVec([0, 0, 1])[2]"),
            )
        };
        assert!(facing("Face") > 0.0);
        assert!(facing("Back") < 0.0);
    }
}

/// **The points projection**: the restriction really is the camera.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod points_projection_tests {
    use super::*;

    /// One camera and one placement, and the full comp-space projection of a
    /// point in the layer's own pixels — the compositor's own arithmetic,
    /// written here as the thing the restriction is measured against.
    fn seen_by_the_compositor(cam: glam::Mat4, place: [[f32; 4]; 4], p: [f32; 3]) -> [f32; 2] {
        let a = (cam * glam::Mat4::from_cols_array_2d(&place)).to_cols_array_2d();
        let at = |r: usize| a[0][r] * p[0] + a[1][r] * p[1] + a[2][r] * p[2] + a[3][r];
        let w = at(3);
        [at(0) / w, at(1) / w]
    }

    /// **The claim**, stated as a test: a particle drawn at the projection's
    /// answer, *on the layer's flat plane*, ends up exactly where the
    /// compositor would have put the particle at its true depth.
    ///
    /// That is the whole of what the third axis does, and it is the one thing a
    /// hand-written 3×4 could get subtly wrong in a way no picture test would
    /// name. Both sides read `lumit_gpu`'s own matrices, so what is being
    /// checked is the restriction and its inverse, not a second camera.
    #[test]
    fn a_projected_particle_lands_where_the_camera_would_have_put_it() {
        let cam = lumit_gpu::composite::camera_matrix(
            1920.0,
            1080.0,
            900.0,
            (860.0, 600.0, -900.0),
            (8.0, -20.0, 5.0),
        );
        let place = lumit_gpu::composite::place_matrix(
            (960.0, 540.0),
            (400.0, 300.0),
            (120.0, 80.0),
            17.0,
            -150.0,
            25.0,
            -40.0,
        );
        // The restriction, built the way `points_projection` builds it.
        let a = (cam * glam::Mat4::from_cols_array_2d(&place)).to_cols_array_2d();
        let row = |r: usize| [a[0][r], a[1][r], a[2][r], a[3][r]];
        let (rx, ry, rw) = (row(0), row(1), row(3));
        let h = [
            [rx[0], rx[1], rx[3]],
            [ry[0], ry[1], ry[3]],
            [rw[0], rw[1], rw[3]],
        ];
        let inv = invert3(&h).expect("a placed layer is not singular");
        let mut m = [[0.0f32; 4]; 3];
        for (r, mr) in m.iter_mut().enumerate() {
            for (c, out) in mr.iter_mut().enumerate() {
                *out = inv[r][0] * rx[c] + inv[r][1] * ry[c] + inv[r][2] * rw[c];
            }
        }
        let proj = lumit_core::fx::points::Projection { m };

        for p in [
            [0.0f32, 0.0, 0.0],
            [640.0, 200.0, 0.0],
            [640.0, 200.0, 250.0],
            [100.0, 700.0, -300.0],
            [800.0, 800.0, 120.0],
        ] {
            let (flat, scale) = proj.apply(p);
            let drawn = seen_by_the_compositor(cam, place, [flat[0], flat[1], 0.0]);
            let truth = seen_by_the_compositor(cam, place, p);
            assert!(
                (drawn[0] - truth[0]).abs() < 5e-2 && (drawn[1] - truth[1]).abs() < 5e-2,
                "a particle at {p:?} is drawn at {drawn:?} and belongs at {truth:?}"
            );
            // The plane itself is untouched, and off it the scale is the
            // perspective divide the compositor applied.
            if p[2] == 0.0 {
                assert!((scale - 1.0).abs() < 1e-4, "the plane foreshortened");
                assert!((flat[0] - p[0]).abs() < 1e-2 && (flat[1] - p[1]).abs() < 1e-2);
            }
        }
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
                    path: None,
                    path_offset: lumit_core::anim::Property::zero(),
                    animators: Vec::new(),
                    style: Default::default(),
                    paragraph: Default::default(),
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
            pan: lumit_core::anim::Property::zero(),
            audio_only: false,
            audio_streams: Default::default(),
            adjustment: false,
            pitch_follows_speed: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: Vec::new(),
            styles: Vec::new(),
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
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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
    /// existed (docs/06). The second half is the compatibility promise:
    /// every project ever saved has no Light layers, and none of them may
    /// shift by a byte.
    ///
    /// Also pinned here: the Accepts lights switch really switches off. That
    /// is the escape hatch a compositor reaches for when a layer must not be
    /// touched, and a switch that quietly does nothing is worse than none.
    #[test]
    fn a_light_layer_lights_the_comp_and_no_lights_changes_nothing() {
        use lumit_core::model::{LightDef, LightKind};

        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = ctx.colour();
        let compositor = ctx.compositor();
        let fx = ctx.fx();
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine,
            compositor,
            fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
            colour_config: None,
            flow: None,
            late_clones: None,
            graph_reach: None,
        };
        // A softbox in front of the comp, big enough to rake across it.
        let mut light = text_layer(160.0);
        light.kind = LayerKind::Light {
            light: Box::new(LightDef {
                kind: LightKind::Area,
                size: [Property::fixed(240.0), Property::fixed(160.0)],
                ..LightDef::default()
            }),
        };
        light.transform.position_z = Property::fixed(-200.0);

        let comp_with = |layers: Vec<Layer>| Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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

    /// A plain solid is drawn from a small tile, which is no picture to light:
    /// a spot falling on one must come out as it does on the same solid made
    /// full size by a mask that covers it, and not as the tile's light
    /// stretched over the layer.
    #[test]
    fn a_lit_plain_solid_is_lit_at_its_own_size() {
        use lumit_core::model::{LightDef, LightKind, ProjectItem, SolidDef};

        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = ctx.colour();
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine,
            compositor: ctx.compositor(),
            fx: ctx.fx(),
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
            colour_config: None,
            flow: None,
            late_clones: None,
            graph_reach: None,
        };
        let (w, h) = (320u32, 180u32);
        let def = Uuid::now_v7();
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Solid(SolidDef {
            id: def,
            name: "grey".into(),
            colour: LinearColour([0.2, 0.2, 0.2, 1.0]),
            width: w,
            height: h,
            extra: serde_json::Map::new(),
        }));
        let doc = std::sync::Arc::new(doc);

        // A narrow spot over one side, so the pool has an edge to smear.
        let mut spot = text_layer(100.0);
        spot.kind = LayerKind::Light {
            light: Box::new(LightDef {
                kind: LightKind::Spot,
                cone_angle: Property::fixed(40.0),
                ..LightDef::default()
            }),
        };
        spot.transform.position_y = Property::fixed(90.0);
        spot.transform.position_z = Property::fixed(-150.0);
        let solid = |masked: bool| {
            let mut l = text_layer(f64::from(w) / 2.0);
            l.kind = LayerKind::Solid { def };
            l.transform.anchor_x = Property::fixed(f64::from(w) / 2.0);
            l.transform.anchor_y = Property::fixed(f64::from(h) / 2.0);
            l.transform.position_y = Property::fixed(f64::from(h) / 2.0);
            if masked {
                l.masks.push(lumit_core::mask::Mask::rectangle(
                    0.0,
                    0.0,
                    f64::from(w),
                    f64::from(h),
                ));
            }
            l
        };
        let render = |layers: Vec<Layer>| {
            let comp = Composition {
                graph: None,
                master_volume_db: 0.0,
                lighting: Default::default(),
                sound_mix: false,
                groups: Vec::new(),
                beat_grid: None,
                id: Uuid::now_v7(),
                name: "c".into(),
                width: w,
                height: h,
                frame_rate: FrameRate::new(30, 1).unwrap(),
                duration: Duration(Rational::new(10, 1).unwrap()),
                background: LinearColour([0.0, 0.0, 0.0, 1.0]),
                work_area: None,
                layers,
                markers: Vec::new(),
                motion_blur: Default::default(),
                extra: serde_json::Map::new(),
            };
            let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
            let mut visited = vec![comp.id];
            let draws = build_comp_draws(&doc, &comp, 0.0, &pixels, &mut visited);
            let tex = realiser.realise(None, w, h, [0.0, 0.0, 0.0, 1.0], &draws);
            engine
                .readback8(
                    &ctx,
                    &engine.display(&ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL),
                )
                .unwrap()
        };

        let plain = render(vec![spot.clone(), solid(false)]);
        let masked = render(vec![spot.clone(), solid(true)]);
        assert_ne!(plain, render(vec![solid(false)]), "the spot lights it");
        // The mask's own antialiased edge is the one place the two may
        // differ, so the outermost pixels are left out.
        let worst = (2..h - 2)
            .flat_map(|y| (2..w - 2).map(move |x| ((y * w + x) * 4) as usize))
            .flat_map(|i| (0..3).map(move |c| i + c))
            .map(|i| plain[i].abs_diff(masked[i]))
            .max()
            .unwrap_or(0);
        assert!(worst <= 1, "a plain solid lit as a tile, {worst}/255 off");
    }

    /// A card between a light and a floor shades the floor, Only keeps the
    /// shadow without the card, a floor that takes no shadows comes out
    /// exactly as if nothing cast one, and one taking only shadows shows
    /// the grey plate behind it, darkened red where the light comes through
    /// a red card.
    #[test]
    fn a_card_casts_its_shadow_and_only_hides_the_card() {
        use lumit_core::model::{LightDef, LightKind, Material, ProjectItem, ShadowMode, SolidDef};

        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = ctx.colour();
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine,
            compositor: ctx.compositor(),
            fx: ctx.fx(),
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
            colour_config: None,
            flow: None,
            late_clones: None,
            graph_reach: None,
        };
        let (w, h) = (320u32, 180u32);
        let mut doc = Document::new();
        let mut solid = |size: (u32, u32), rgb: [f32; 3]| {
            let def = Uuid::now_v7();
            doc.items.push(ProjectItem::Solid(SolidDef {
                id: def,
                name: "grey".into(),
                colour: LinearColour([rgb[0], rgb[1], rgb[2], 1.0]),
                width: size.0,
                height: size.1,
                extra: serde_json::Map::new(),
            }));
            def
        };
        let (floor_def, card_def) = (
            solid((w, h), [0.5, 0.5, 0.5]),
            solid((80, 60), [0.9, 0.1, 0.05]),
        );
        let doc = std::sync::Arc::new(doc);
        let mut light = text_layer(40.0);
        light.kind = LayerKind::Light {
            light: Box::new(LightDef {
                kind: LightKind::Point,
                casts_shadows: true,
                ..LightDef::default()
            }),
        };
        light.transform.position_y = Property::fixed(90.0);
        light.transform.position_z = Property::fixed(-200.0);
        let card_layer = |def: Uuid, size: (u32, u32), z: f64, casts, accepts, through| {
            let mut l = text_layer(f64::from(w) / 2.0);
            l.kind = LayerKind::Solid { def };
            l.switches.three_d = true;
            l.transform.anchor_x = Property::fixed(f64::from(size.0) / 2.0);
            l.transform.anchor_y = Property::fixed(f64::from(size.1) / 2.0);
            l.transform.position_y = Property::fixed(f64::from(h) / 2.0);
            l.transform.position_z = Property::fixed(z);
            l.transform.material = Box::new(Material {
                casts_shadows: casts,
                accepts_shadows: accepts,
                light_transmission: Property::fixed(through),
                ..Material::default()
            });
            l
        };
        let render = |casts: ShadowMode, floor_takes: ShadowMode, through: f64| {
            let comp = Composition {
                graph: None,
                master_volume_db: 0.0,
                lighting: Default::default(),
                sound_mix: false,
                groups: Vec::new(),
                beat_grid: None,
                id: Uuid::now_v7(),
                name: "c".into(),
                width: w,
                height: h,
                frame_rate: FrameRate::new(30, 1).unwrap(),
                duration: Duration(Rational::new(10, 1).unwrap()),
                background: LinearColour([0.2, 0.2, 0.2, 1.0]),
                work_area: None,
                layers: vec![
                    light.clone(),
                    card_layer(card_def, (80, 60), -80.0, casts, ShadowMode::On, through),
                    card_layer(floor_def, (w, h), 0.0, ShadowMode::Off, floor_takes, 0.0),
                ],
                markers: Vec::new(),
                motion_blur: Default::default(),
                extra: serde_json::Map::new(),
            };
            let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
            let mut visited = vec![comp.id];
            let draws = build_comp_draws(&doc, &comp, 0.0, &pixels, &mut visited);
            let tex = realiser.realise(None, w, h, [0.2, 0.2, 0.2, 1.0], &draws);
            engine
                .readback8(
                    &ctx,
                    &engine.display(&ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL),
                )
                .unwrap()
        };

        let none = render(ShadowMode::Off, ShadowMode::On, 0.0);
        let cast = render(ShadowMode::On, ShadowMode::On, 0.0);
        let only = render(ShadowMode::Only, ShadowMode::On, 0.0);
        assert_ne!(none, cast, "the card's shadow falls on the floor");
        assert!(
            cast.iter().zip(&none).all(|(a, b)| a <= b),
            "a shadow only ever takes light away"
        );
        assert_ne!(only, cast, "Only hides the card");
        assert_eq!(
            cast,
            render(ShadowMode::On, ShadowMode::On, 0.0),
            "deterministic"
        );
        assert_eq!(
            none,
            render(ShadowMode::On, ShadowMode::Off, 0.0),
            "a floor that takes no shadows is lit as if none were cast"
        );
        let tinted = render(ShadowMode::On, ShadowMode::Only, 100.0);
        let at = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            tinted[i..i + 4].to_vec()
        };
        // The grey plate behind the floor shows through it, and the red
        // shadow only darkens it.
        let plate = at(10, 170);
        assert!(
            plate[0] == plate[1] && plate[1] == plate[2] && plate[0] > 100,
            "{plate:?}"
        );
        let red = at(260, 90);
        assert!(
            red[0] <= plate[0] && red[0] > red[1] + 60 && red[1] < 40 && red[2] < 40,
            "red where the light comes through the card: {red:?} over {plate:?}"
        );
    }

    /// A region of interest composites a window of the comp, and that window
    /// must be **the same pixels** the full frame has there. This is
    /// the whole promise: a region changes how much is computed, never what
    /// the picture is. If the two ever disagree, working inside a region is
    /// working on a lie.
    ///
    /// Also pinned: the returned texture is the region's size, and a region
    /// covering everything is refused so its frames keep the full frame's
    /// names rather than banking a duplicate set under new ones.
    #[test]
    fn a_region_of_interest_is_the_same_pixels_the_full_frame_has_there() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = ctx.colour();
        let compositor = ctx.compositor();
        let fx = ctx.fx();
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine,
            compositor,
            fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
            colour_config: None,
            flow: None,
            late_clones: None,
            graph_reach: None,
        };
        let comp = Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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
        let region = lumit_gpu::composite::Region {
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
            None,
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
            None,
        );
        assert_eq!(win_px, bytes(&again), "a windowed composite is bit-stable");
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
        let mut post = lumit_core::fx::builtins::instantiate("posterize_time").unwrap();
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
            pan: lumit_core::anim::Property::zero(),
            audio_only: false,
            audio_streams: Default::default(),
            adjustment: false,
            pitch_follows_speed: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: vec![post],
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn posterize_comp() -> Composition {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, 100.0); // x = 100·t
        Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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

    // docs/08 §3.25: the whole preview Posterize path (detect → held
    // below → adjustment blend) must reduce, at full coverage, to a plain render
    // of the below-stack at the held time. So a posterised frame at t = 0.35
    // equals `render_below_at` at tau = 0.3 bit-for-bit — the moving-scene
    // pixel check. (If the code held at the frame time instead, the two would
    // differ, because the text has moved between 0.3 and 0.35.)
    #[test]
    fn posterised_frame_equals_a_plain_render_at_the_held_time() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return;
        };
        let engine = ctx.colour();
        let compositor = ctx.compositor();
        let fx = ctx.fx();
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine,
            compositor,
            fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
            colour_config: None,
            flow: None,
            late_clones: None,
            graph_reach: None,
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
        let mut e = lumit_core::fx::builtins::instantiate("accumulation_mb").unwrap();
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
            pan: lumit_core::anim::Property::zero(),
            audio_only: false,
            audio_streams: Default::default(),
            adjustment: false,
            pitch_follows_speed: false,
            retime: None,
            interpolation: Default::default(),
            parked_flow: None,
            graph_inputs: None,
            blend: Default::default(),
            masks: Vec::new(),
            paint: Vec::new(),
            puppet: None,
            effects: vec![e],
            styles: Vec::new(),
            switches: Switches::default(),
            extra: serde_json::Map::new(),
        }
    }

    fn comp_with(fps: u32, layers: Vec<Layer>) -> Composition {
        Composition {
            graph: None,
            master_volume_db: 0.0,
            lighting: Default::default(),
            sound_mix: false,
            groups: Vec::new(),
            beat_grid: None,
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

    // docs/08 §3.26: a still scene averaged over N is bit-identical to the
    // plain composite (the accumulation adjustment is a pure identity when nothing
    // moves), while a moving scene smears — differs from the plain composite and
    // covers a wider horizontal extent. The same combine drives the export path.
    #[test]
    fn accumulation_still_scene_is_identity_and_moving_scene_smears() {
        let Some(ctx) = lumit_gpu::test_support::lease() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let engine = ctx.colour();
        let compositor = ctx.compositor();
        let fx = ctx.fx();
        let lut_cache = std::cell::RefCell::new(crate::fxops::LutCache::default());
        let fx_cache = std::cell::RefCell::new(crate::fxops::FxCache::default());
        let realiser = Realiser {
            ctx: ctx.clone_handle(),
            engine,
            compositor,
            fx,
            lut_cache: &lut_cache,
            fx_cache: &fx_cache,
            render_scale: 1.0,
            samples: 1,
            profiler: None,
            colour_inputs: None,
            colour_config: None,
            flow: None,
            late_clones: None,
            graph_reach: None,
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

    // An adjustment layer carrying the named built-in effects at their
    // defaults — Motion blur (docs/08 §3.2) and Datamosh (§3.12) are the
    // two that want measured motion.
    fn flow_adjustment(names: &[&str]) -> Layer {
        let mut l = accumulation_adjustment(4.0);
        l.name = "flow".into();
        l.effects = names
            .iter()
            .map(|n| lumit_core::fx::builtins::instantiate(n).unwrap())
            .collect();
        l
    }

    // A text layer sweeping x at `per_second` px/s from the origin.
    fn sweeping_text(per_second: f64) -> Layer {
        let mut text = text_layer(0.0);
        text.transform.position_x = ramp(0.0, per_second);
        text
    }

    // The two GPU engines plus the flow backend a composite measurement needs,
    // as the headless renderer's owner holds them.
    struct FlowRig {
        ctx: lumit_gpu::test_support::Lease,
        lut_cache: std::cell::RefCell<crate::fxops::LutCache>,
        fx_cache: std::cell::RefCell<crate::fxops::FxCache>,
        flow: std::cell::RefCell<crate::realise::CompositeFlow>,
    }

    impl FlowRig {
        fn new() -> Option<Self> {
            let ctx = lumit_gpu::test_support::lease()?;
            Some(FlowRig {
                ctx,
                lut_cache: std::cell::RefCell::new(crate::fxops::LutCache::default()),
                fx_cache: std::cell::RefCell::new(crate::fxops::FxCache::default()),
                flow: std::cell::RefCell::new(crate::realise::CompositeFlow::default()),
            })
        }

        fn realiser(&self) -> Realiser<'_> {
            Realiser {
                ctx: self.ctx.clone_handle(),
                engine: self.ctx.colour(),
                compositor: self.ctx.compositor(),
                fx: self.ctx.fx(),
                lut_cache: &self.lut_cache,
                fx_cache: &self.fx_cache,
                render_scale: 1.0,
                samples: 1,
                profiler: None,
                colour_inputs: None,
                colour_config: None,
                flow: Some(&self.flow),
                late_clones: None,
                graph_reach: None,
            }
        }

        fn render(&self, doc: &Document, comp: &Composition, t: f64) -> Vec<u8> {
            self.render_with(doc, comp, t, &HashMap::new())
        }

        // The same render over decoded footage.
        fn render_with(
            &self,
            doc: &Document,
            comp: &Composition,
            t: f64,
            pixels: &HashMap<Uuid, &CompLayerPixels>,
        ) -> Vec<u8> {
            let mut v = vec![comp.id];
            let draws =
                build_comp_draws(&std::sync::Arc::new(doc.clone()), comp, t, pixels, &mut v);
            let tex = self.realiser().realise(
                comp.camera_pose(t),
                comp.width,
                comp.height,
                comp.background.0.map(f64::from),
                &draws,
            );
            self.ctx
                .colour()
                .readback8(
                    &self.ctx,
                    &self
                        .ctx
                        .colour()
                        .display(&self.ctx, &tex, lumit_gpu::DisplayParams::NEUTRAL),
                )
                .unwrap()
        }
    }

    // How many columns carry something brighter than the comp's background —
    // the frame's horizontal extent, which a smear widens.
    fn lit_columns(px: &[u8], w: usize, h: usize) -> usize {
        // Top-left is background in every scene here.
        let floor = px[0].saturating_add(15);
        (0..w)
            .filter(|&x| (0..h).any(|y| px[(y * w + x) * 4] > floor))
            .count()
    }

    // docs/08 §3.2: **the headline case**. Motion blur on an
    // adjustment layer over a moving scene must actually smear it — the effect's
    // commonest placement, and a silent passthrough until the composite below
    // could be measured. Deterministic across two runs, as every render is.
    #[test]
    fn an_adjustment_fast_motion_blur_smears_the_composite_below() {
        let Some(rig) = FlowRig::new() else {
            return; // no GPU here — skip, as the gpu crate's own tests do
        };
        let doc = Document::new();
        let text = sweeping_text(400.0); // 40 px a frame at 10 fps
        let plain = comp_with(10, vec![text.clone()]);
        let blurred = comp_with(10, vec![flow_adjustment(&["motion_blur"]), text]);

        let a = rig.render(&doc, &plain, 0.5);
        let b = rig.render(&doc, &blurred, 0.5);
        assert_ne!(
            a, b,
            "Motion blur on an adjustment layer must smear the composite \
             below it, not pass it through"
        );
        assert_eq!(
            b,
            rig.render(&doc, &blurred, 0.5),
            "and the measurement is deterministic: two runs, one picture"
        );
        let (w, h) = (320usize, 180usize);
        assert!(
            lit_columns(&b, w, h) > lit_columns(&a, w, h),
            "the streak must widen what the frame covers: plain {}, smeared {}",
            lit_columns(&a, w, h),
            lit_columns(&b, w, h)
        );
    }

    // On the frame before a cut the next frame is another shot, and the
    // difference between the two is not motion. Motion blur is measured a
    // frame back there, so the shot after the cut changes nothing.
    #[test]
    fn an_adjustment_motion_blur_does_not_measure_across_a_cut() {
        let Some(rig) = FlowRig::new() else {
            return;
        };
        let doc = Document::new();
        let cut = lumit_core::time::CompTime(Rational::new(6, 10).unwrap());
        let mut shot = sweeping_text(100.0);
        shot.out_point = cut;
        let mut next = text_layer(0.0);
        next.transform.position_x = ramp(300.0, -100.0);
        next.in_point = cut;
        let blur = flow_adjustment(&["motion_blur"]);
        let with_next = comp_with(10, vec![blur.clone(), next, shot.clone()]);
        let alone = comp_with(10, vec![blur, shot.clone()]);
        let smeared = rig.render(&doc, &with_next, 0.5);
        assert!(
            smeared == rig.render(&doc, &alone, 0.5),
            "the shot after the cut is not read as motion"
        );
        let plain = rig.render(&doc, &comp_with(10, vec![shot]), 0.5);
        let (w, h) = (320usize, 180usize);
        assert!(
            lit_columns(&smeared, w, h) > lit_columns(&plain, w, h),
            "and the shot still smears by its own motion"
        );
    }

    // A stand-in for a plugin that reads the frame before this one, as a
    // motion blur does: it paints the picture with its -1 neighbour, and
    // leaves it alone when it is handed none.
    struct Previous(&'static lumit_core::fx::schema::EffectSchema);

    impl lumit_core::fx::registry::EffectDef for Previous {
        fn schema(&self) -> &'static lumit_core::fx::schema::EffectSchema {
            self.0
        }
        fn apply_cpu_temporal(
            &self,
            _inst: Uuid,
            _lt: f64,
            rgba: &mut [f32],
            _w: u32,
            _h: u32,
            _p: lumit_core::fx::params::Params<'_>,
            neighbours: &[(i32, &[f32])],
        ) {
            if let Some((_, previous)) = neighbours.iter().find(|(o, _)| *o == -1) {
                rgba.copy_from_slice(previous);
            }
        }
    }

    // One instance of it, registered the way a scanned plugin is: a window of
    // a frame either side, which is what a plugin with temporal access declares.
    fn previous_frame_effect() -> lumit_core::model::EffectInstance {
        const NAME: &str = "ofx:test.build.previous";
        static REGISTERED: std::sync::Once = std::sync::Once::new();
        REGISTERED.call_once(|| {
            let schema = Box::leak(Box::new(lumit_core::fx::schema::EffectSchema {
                match_name: NAME,
                label: "Previous frame",
                version: 1,
                category: lumit_core::fx::schema::FxCategory::Utility,
                traits: lumit_core::fx::schema::EffectTraits {
                    cost: lumit_core::fx::schema::CostClass::Heavy,
                    roi: lumit_core::fx::schema::Roi::FullFrame,
                    temporal: &[-1, 0, 1],
                    premultiplied: true,
                    seeded: false,
                    beat_input: false,
                },
                params: &[],
                groups: &[],
                enabled_when: &[],
                matte: lumit_core::fx::schema::MatteRole::None,
            }));
            assert!(crate::gpufx::ofx::register(Box::leak(Box::new(Previous(
                schema
            )))));
        });
        lumit_core::fx::builtins::instantiate(NAME).unwrap()
    }

    // A clip that fills the frame, and the item and probe the planner reads
    // it by: 10 fps, the rate of the comps here, so a comp frame is a source
    // frame.
    fn clip(doc: &mut Document) -> (Layer, HashMap<Uuid, crate::source::SourceProbe>) {
        use lumit_core::model::{FootageItem, MediaRef, ProjectItem};
        let item = Uuid::now_v7();
        doc.items.push(ProjectItem::Footage(FootageItem {
            sequence: None,
            id: item,
            name: "f".into(),
            media: MediaRef {
                relative_path: "f.mp4".into(),
                absolute_path: "/f.mp4".into(),
                fingerprint: None,
                extra: serde_json::Map::new(),
            },
            extra: serde_json::Map::new(),
            colour_space: None,
            source_layer: None,
            interpretation: Default::default(),
        }));
        let mut l = text_layer(0.0);
        l.kind = LayerKind::Footage { item };
        l.transform.position_y = Property::fixed(0.0);
        let probe = crate::source::SourceProbe::Video {
            fps: 10.0,
            width: 320,
            height: 180,
            frames: 100,
            audio: false,
        };
        (l, [(item, probe)].into_iter().collect())
    }

    // The clip as the decode worker hands it over for a planned job: grey at
    // the frame's own time and orange at every other moment the plan asked
    // for, filed under the plan's own keys.
    fn decoded(job: &crate::decode::CompJob) -> CompLayerPixels {
        use crate::decode::Cut;
        let flat = |rgb: [u8; 3]| CompLayerPixels {
            layer: job.layer,
            width: 8,
            height: 8,
            rgba: [rgb[0], rgb[1], rgb[2], 255].repeat(64).into(),
            format: lumit_media::decode::PixelFormat::Srgb8,
            natural_w: 320,
            natural_h: 180,
            temporal: Vec::new(),
            flow_fields: Vec::new(),
            shutter: Vec::new(),
            source_key: 0,
            source_frame: 0,
        };
        let neighbour = || flat(ORANGE).rgba.to_vec();
        let moment = || Some(Box::new(flat(ORANGE)));
        let mut lp = flat(GREY);
        lp.temporal = job
            .temporal
            .iter()
            .map(|(o, _)| (*o, neighbour()))
            .collect();
        lp.shutter = job.shutter.iter().map(|s| (s.offset, moment())).collect();
        // Another clip's frame arrives where a frame of this one would, and
        // a gap arrives as nothing.
        for cut in &job.cuts {
            match cut {
                Cut::Neighbour(o, _) => lp.temporal.push((*o, neighbour())),
                Cut::Moment(o, clip) => lp.shutter.push((*o, clip.as_ref().and_then(|_| moment()))),
                // Mixed into the frame's own pixels where it is decoded.
                Cut::Under(..) => {}
            }
        }
        lp
    }
    const GREY: [u8; 3] = [60, 60, 60];
    const ORANGE: [u8; 3] = [230, 120, 20];

    // The colour a draw list's first clip is drawn in.
    fn clip_colour(draws: &[CompLayerDraw]) -> [u8; 3] {
        match &draws.first().expect("the clip draws").source {
            DrawSource::Pixels { rgba, .. } => [rgba[0], rgba[1], rgba[2]],
            // A node graph's clip is its Read box.
            DrawSource::Graph(plan) => match plan.steps.first() {
                Some(crate::draw::GraphStep::Read(read)) => {
                    clip_colour(std::slice::from_ref(&**read))
                }
                _ => panic!("the graph reads a clip"),
            },
            _ => panic!("a clip draws from decoded pixels"),
        }
    }

    // The red in the middle of a rendered frame.
    fn centre_red(px: &[u8]) -> u8 {
        px[(90 * 320 + 160) * 4]
    }

    // A plugin on a **Precomp** layer that reads the previous frame must be
    // handed the nested comp as it was a frame ago, footage and all. The
    // nested rebuild used to draw the clip inside from the one decode in
    // hand, so the plugin compared a frame with itself, saw no motion and
    // gave the picture back unchanged. The footage is tried on a footage
    // layer, as a clip on a Sequence layer and read by a node graph, since
    // each is planned by its own arm. The Sequence layer has an edit point
    // at this frame, so its frame before is another clip's footage, and it
    // is tried again with the plugin on the layer itself.
    #[test]
    fn a_plugin_on_a_precomp_reads_the_nested_footage_a_frame_back() {
        use crate::decode::Cut;
        use lumit_core::comp_graph::{CompGraph, GraphEdge, GraphNode};
        use lumit_core::model::ProjectItem;
        use lumit_core::sequence::{Clip, ClipSource};
        let mut doc = Document::new();
        let (footage, mut probes) = clip(&mut doc);
        let LayerKind::Footage { item } = footage.kind else {
            panic!("a footage layer");
        };
        let (earlier, more) = clip(&mut doc);
        let LayerKind::Footage { item: other } = earlier.kind else {
            panic!("a footage layer");
        };
        probes.extend(more);
        let at = |n, d| Rational::new(n, d).unwrap();
        let mut cut = footage.clone();
        cut.id = Uuid::now_v7();
        cut.kind = LayerKind::Sequence {
            clips: vec![
                Clip::new(
                    ClipSource::Footage(other),
                    at(0, 1),
                    at(1, 2),
                    at(0, 1),
                    at(1, 2),
                ),
                Clip::new(
                    ClipSource::Footage(item),
                    at(1, 2),
                    at(10, 1),
                    at(1, 2),
                    at(19, 2),
                ),
            ],
        };
        let sequence = cut.clone();
        let (read, out) = (Uuid::now_v7(), Uuid::now_v7());
        let mut graph = comp_with(10, Vec::new());
        graph.graph = Some(CompGraph {
            notes: Vec::new(),
            nodes: vec![
                GraphNode::Read {
                    id: read,
                    item,
                    custom_name: None,
                },
                GraphNode::Output { id: out },
            ],
            edges: vec![GraphEdge {
                from: read,
                from_port: "output".into(),
                to: out,
                to_port: "input".into(),
            }],
            layout: Vec::new(),
            exposed: Vec::new(),
            groups: Vec::new(),
        });
        // What the clip's pixels are filed under, the footage it shows a
        // frame back, and the comp it sits in.
        let cases = [
            (
                "a footage layer",
                footage.id,
                item,
                comp_with(10, vec![footage]),
            ),
            ("a Sequence layer", cut.id, other, comp_with(10, vec![cut])),
            ("a node graph", read, item, graph),
        ]
        .map(|(what, clip_id, before, nested)| {
            let nested_id = nested.id;
            doc.items.push(ProjectItem::Composition(nested));
            (what, clip_id, before, nested_id)
        });
        let rig = FlowRig::new();
        let precomp = |nested_id: Uuid, effects: Vec<lumit_core::model::EffectInstance>| {
            let mut l = text_layer(0.0);
            l.kind = LayerKind::Precomp { comp: nested_id };
            l.transform.position_y = Property::fixed(0.0);
            l.effects = effects;
            comp_with(10, vec![l])
        };
        // A job's other moments, whichever way they were asked for: the
        // offset, whose footage, and which frame of it.
        let moments = |job: &crate::decode::CompJob| -> Vec<(f64, Uuid, usize)> {
            let own = job.shutter.iter();
            own.map(|s| (s.offset.round(), job.item, s.source_frame))
                .chain(job.cuts.iter().filter_map(|c| match c {
                    Cut::Moment(o, Some(clip)) => Some((o.round(), clip.item, clip.source_frame)),
                    _ => None,
                }))
                .collect()
        };

        for (what, clip_id, before, nested_id) in cases {
            let outer = precomp(nested_id, vec![previous_frame_effect()]);

            // The plan fetches the clip at both of the plugin's neighbours,
            // each the one real frame it shows then.
            let jobs = crate::plan::plan_comp_frame(
                &doc,
                &outer,
                0.5,
                crate::plan::Quality::default(),
                &probes,
            );
            let job = jobs.iter().find(|j| j.layer == clip_id).expect("planned");
            assert_eq!(job.source_frame, 5, "{what}");
            assert_eq!(
                moments(job),
                vec![(-1.0, before, 4), (1.0, item, 6)],
                "{what}: the clip is planned a frame either side"
            );

            // And the builder draws each neighbour from those pixels.
            let lp = decoded(job);
            let pixels: HashMap<Uuid, &CompLayerPixels> = [(clip_id, &lp)].into_iter().collect();
            let mut v = vec![outer.id];
            let draws = build_comp_draws(
                &std::sync::Arc::new(doc.clone()),
                &outer,
                0.5,
                &pixels,
                &mut v,
            );
            let d = draws.first().expect("the Precomp draws");
            match &d.source {
                DrawSource::Nested { draws, .. } => assert_eq!(clip_colour(draws), GREY),
                _ => panic!("a Precomp layer draws a nested comp"),
            }
            assert_eq!(
                d.flow_below
                    .iter()
                    .map(|(o, inner, _, measure)| (*o, clip_colour(inner), *measure))
                    .collect::<Vec<_>>(),
                vec![(-1, ORANGE, None), (1, ORANGE, None)],
                "{what}: each neighbour is the clip at its own moment, with no flow asked for"
            );

            let Some(rig) = &rig else {
                continue;
            };
            let plain = rig.render_with(&doc, &precomp(nested_id, Vec::new()), 0.5, &pixels);
            let read = rig.render_with(&doc, &outer, 0.5, &pixels);
            assert!(
                centre_red(&read) > centre_red(&plain).saturating_add(60),
                "{what}: the plugin was handed the previous frame ({} → {})",
                centre_red(&plain),
                centre_red(&read)
            );
        }

        // The plugin on the Sequence layer itself reads its neighbours
        // through the clips. The frame after is one more frame of this clip,
        // and the frame before the edit point is the other clip's own job.
        let mut direct = sequence.clone();
        direct.effects = vec![previous_frame_effect()];
        let comp = comp_with(10, vec![direct]);
        let jobs = crate::plan::plan_comp_frame(
            &doc,
            &comp,
            0.5,
            crate::plan::Quality::default(),
            &probes,
        );
        let job = jobs.first().expect("planned");
        assert_eq!(job.temporal, vec![(1, 6)]);
        assert!(
            matches!(&job.cuts[..], [Cut::Neighbour(-1, n)] if n.item == other && n.source_frame == 4),
            "the frame before the cut is the other clip's"
        );
        if let Some(rig) = &rig {
            let lp = decoded(job);
            let pixels: HashMap<Uuid, &CompLayerPixels> =
                [(sequence.id, &lp)].into_iter().collect();
            let plain = comp_with(10, vec![sequence.clone()]);
            let (plain, read) = (
                rig.render_with(&doc, &plain, 0.5, &pixels),
                rig.render_with(&doc, &comp, 0.5, &pixels),
            );
            assert!(
                centre_red(&read) > centre_red(&plain).saturating_add(60),
                "a plugin on the Sequence layer was handed the previous frame ({} → {})",
                centre_red(&plain),
                centre_red(&read)
            );
        }

        // With nothing before the edit point the frame before is a gap, so
        // that neighbour is drawn without the layer and not with this frame
        // held.
        let mut alone = sequence;
        if let LayerKind::Sequence { clips } = &mut alone.kind {
            clips.remove(0);
        }
        let alone_id = alone.id;
        let nested = comp_with(10, vec![alone]);
        let outer = precomp(nested.id, vec![previous_frame_effect()]);
        doc.items.push(ProjectItem::Composition(nested));
        let jobs = crate::plan::plan_comp_frame(
            &doc,
            &outer,
            0.5,
            crate::plan::Quality::default(),
            &probes,
        );
        let lp = decoded(jobs.first().expect("planned"));
        let pixels: HashMap<Uuid, &CompLayerPixels> = [(alone_id, &lp)].into_iter().collect();
        let mut v = vec![outer.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &outer,
            0.5,
            &pixels,
            &mut v,
        );
        assert_eq!(
            draws[0]
                .flow_below
                .iter()
                .map(|(o, inner, ..)| (*o, inner.len()))
                .collect::<Vec<_>>(),
            vec![(-1, 0), (1, 1)],
            "a gap a frame back is empty"
        );
    }

    // The same on an **adjustment** layer, whose picture is the composite
    // below it: the clip beneath is planned and drawn a frame either side.
    #[test]
    fn a_plugin_on_an_adjustment_reads_the_footage_below_a_frame_back() {
        let mut doc = Document::new();
        let (footage, probes) = clip(&mut doc);
        let footage_id = footage.id;
        let mut adjust = flow_adjustment(&[]);
        adjust.effects = vec![previous_frame_effect()];
        let plain = comp_with(10, vec![footage.clone()]);
        // The frame before a cut: a shot that starts on the next frame is not
        // on screen yet, and is planned all the same, for the frame the
        // plugin reads ahead. Without it that frame is drawn with a hole
        // where the next shot is.
        let mut next = footage.clone();
        next.id = Uuid::now_v7();
        next.in_point = lumit_core::time::CompTime(Rational::new(6, 10).unwrap());
        let cut = comp_with(10, vec![adjust.clone(), next.clone(), footage.clone()]);
        let jobs =
            crate::plan::plan_comp_frame(&doc, &cut, 0.5, crate::plan::Quality::default(), &probes);
        assert!(
            jobs.iter().any(|j| j.layer == next.id),
            "the shot after the cut is planned for the neighbour it is in"
        );
        // The same for a Sequence layer in a gap now whose clip starts on the
        // next frame. The clip is fetched for that frame and drawn in it, and
        // nothing of it is drawn in this one or in the gap a frame back.
        let LayerKind::Footage { item } = footage.kind else {
            panic!("a footage layer");
        };
        let at = |n, d| Rational::new(n, d).unwrap();
        let mut gap = footage.clone();
        gap.id = Uuid::now_v7();
        gap.kind = LayerKind::Sequence {
            clips: vec![lumit_core::sequence::Clip::new(
                lumit_core::sequence::ClipSource::Footage(item),
                at(0, 1),
                at(94, 10),
                at(6, 10),
                at(94, 10),
            )],
        };
        let gap_id = gap.id;
        let cut = comp_with(10, vec![adjust.clone(), gap]);
        let jobs =
            crate::plan::plan_comp_frame(&doc, &cut, 0.5, crate::plan::Quality::default(), &probes);
        let lp = decoded(jobs.first().expect("the clip after the gap is planned"));
        let pixels: HashMap<Uuid, &CompLayerPixels> = [(gap_id, &lp)].into_iter().collect();
        let mut v = vec![cut.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &cut,
            0.5,
            &pixels,
            &mut v,
        );
        assert_eq!(draws.len(), 1, "a layer in a gap draws nothing");
        assert_eq!(
            draws[0]
                .flow_below
                .iter()
                .map(|(o, below, ..)| (*o, below.len()))
                .collect::<Vec<_>>(),
            vec![(-1, 0), (1, 1)],
            "the clip after the gap is in the neighbour it starts in"
        );
        let comp = comp_with(10, vec![adjust, footage]);

        let jobs = crate::plan::plan_comp_frame(
            &doc,
            &comp,
            0.5,
            crate::plan::Quality::default(),
            &probes,
        );
        let job = jobs
            .iter()
            .find(|j| j.layer == footage_id)
            .expect("planned");
        assert_eq!(
            job.shutter
                .iter()
                .map(|s| (s.offset.round(), s.source_frame, s.blend))
                .collect::<Vec<_>>(),
            vec![(-1.0, 4, None), (1.0, 6, None)],
            "the clip below is planned a frame either side"
        );

        let lp = decoded(job);
        let pixels: HashMap<Uuid, &CompLayerPixels> = [(footage_id, &lp)].into_iter().collect();
        let mut v = vec![comp.id];
        let draws = build_comp_draws(
            &std::sync::Arc::new(doc.clone()),
            &comp,
            0.5,
            &pixels,
            &mut v,
        );
        let adj = draws
            .iter()
            .find(|d| matches!(d.source, DrawSource::Adjust))
            .expect("the adjustment draws");
        assert_eq!(
            adj.flow_below
                .iter()
                .map(|(o, below, _, measure)| (*o, clip_colour(below), *measure))
                .collect::<Vec<_>>(),
            vec![(-1, ORANGE, None), (1, ORANGE, None)],
            "each neighbour is the clip below at its own moment"
        );

        let Some(rig) = FlowRig::new() else {
            return;
        };
        let before = rig.render_with(&doc, &plain, 0.5, &pixels);
        let read = rig.render_with(&doc, &comp, 0.5, &pixels);
        assert!(
            centre_red(&read) > centre_red(&before).saturating_add(60),
            "the plugin was handed the previous frame ({} → {})",
            centre_red(&before),
            centre_red(&read)
        );
    }

    // What it costs, and who pays. A Precomp whose stack reads only its own
    // frame plans one decode and builds no neighbour, and a held frame still
    // saves its decodes. A temporal stack rebuilds once per frame of its
    // window, and only a flow consumer's offset is measured.
    #[test]
    fn only_a_temporal_stack_on_a_precomp_pays_for_neighbours() {
        use lumit_core::model::ProjectItem;
        let mut doc = Document::new();
        let (footage, probes) = clip(&mut doc);
        let footage_id = footage.id;
        let nested = comp_with(10, vec![footage]);
        let nested_id = nested.id;
        doc.items.push(ProjectItem::Composition(nested));
        let precomp = |names: &[&str]| {
            let mut l = text_layer(0.0);
            l.kind = LayerKind::Precomp { comp: nested_id };
            l.effects = names
                .iter()
                .map(|n| lumit_core::fx::builtins::instantiate(n).unwrap())
                .collect();
            comp_with(10, vec![l])
        };
        let plan = |comp: &Composition, held: bool| {
            crate::plan::plan_comp_frame_held(
                &doc,
                comp,
                0.5,
                crate::plan::Quality::default(),
                &probes,
                Some(&|_, _, _| held),
            )
        };
        let rebuilds = |comp: &Composition| -> Vec<(i32, Option<i32>)> {
            let pixels: HashMap<Uuid, &CompLayerPixels> = HashMap::new();
            let mut v = vec![comp.id];
            build_comp_draws(
                &std::sync::Arc::new(doc.clone()),
                comp,
                0.5,
                &pixels,
                &mut v,
            )
            .first()
            .expect("the Precomp draws")
            .flow_below
            .iter()
            .map(|(o, _, _, measure)| (*o, *measure))
            .collect()
        };

        let bare = precomp(&[]);
        let blurred = precomp(&["blur"]);
        let jobs = plan(&blurred, false);
        let job = jobs
            .iter()
            .find(|j| j.layer == footage_id)
            .expect("planned");
        assert!(job.shutter.is_empty(), "one frame, no other moment");
        assert_eq!(
            job.source_key(),
            plan(&bare, false)[0].source_key(),
            "and the decode keeps its name"
        );
        assert!(rebuilds(&blurred).is_empty(), "no neighbour is built");
        assert!(
            plan(&blurred, true).is_empty(),
            "a held frame wants no decodes"
        );

        // Echo reads back through its window and measures nothing.
        let echoed = precomp(&["echo"]);
        let trail = rebuilds(&echoed);
        assert_eq!(trail.len(), 16, "one rebuild per frame of the window");
        assert!(trail.iter().all(|(o, measure)| *o < 0 && measure.is_none()));
        // A rebuild is made from pixels, so its footage is decoded even
        // where the frame itself is held.
        let jobs = plan(&echoed, true);
        let job = jobs
            .iter()
            .find(|j| j.layer == footage_id)
            .expect("planned");
        assert!(!job.shutter.is_empty());

        // Motion blur still gets its measurement.
        assert_eq!(rebuilds(&precomp(&["motion_blur"])), vec![(1, Some(1))]);
    }

    // Echo on a Precomp layer had the same fault as the plugin: every tap
    // was the frame in hand, so there was no trail.
    #[test]
    fn an_echo_on_a_precomp_trails_the_comp_inside_it() {
        use lumit_core::model::ProjectItem;
        let Some(rig) = FlowRig::new() else {
            return;
        };
        let nested = comp_with(10, vec![sweeping_text(400.0)]);
        let nested_id = nested.id;
        let mut doc = Document::new();
        doc.items.push(ProjectItem::Composition(nested));
        let precomp = |names: &[&str]| {
            let mut l = text_layer(0.0);
            l.kind = LayerKind::Precomp { comp: nested_id };
            l.transform.position_y = Property::fixed(0.0);
            l.effects = names
                .iter()
                .map(|n| lumit_core::fx::builtins::instantiate(n).unwrap())
                .collect();
            comp_with(10, vec![l])
        };
        assert_ne!(
            rig.render(&doc, &precomp(&[]), 0.5),
            rig.render(&doc, &precomp(&["echo"]), 0.5),
            "Echo on a Precomp layer must trail the comp inside it"
        );
    }
}
