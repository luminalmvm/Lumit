//! The puppet warp at the paint and masks seam, and the shared mesh cache it
//! builds through.

use super::LayerPixels;
use std::sync::Arc;
use uuid::Uuid;

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
pub(super) fn warp_puppet(
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
pub(super) fn preview_puppet(pixels: &LayerPixels, layer: Uuid, density: f64, expansion: f64) {
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
