//! What the Viewer's puppet overlay draws, published by the render that already
//! worked it out (docs/impl/puppet.md §5).
//!
//! # In plain terms
//!
//! The puppet's mesh is never in the project file and never crosses the bridge
//! as part of the document — it is rebuilt from the layer's own alpha, deep
//! inside the render, and thrown away. But the overlay has to draw it: the thin
//! wireframe you see under a puppet tool *is* that mesh, bent by the pins.
//!
//! Rather than build a second copy of it on the frontend's side of the wall,
//! the render leaves the one it just used here, in a small pigeonhole per layer,
//! and the bridge reads it out. Nothing is computed for the overlay that was not
//! computed for the picture, and the wireframe cannot disagree with the pixels
//! because it is the same mesh.
//!
//! There is one thing the picture does not need and the overlay does: the mesh
//! **before the first pin**. Placing that pin means aiming at the mesh, and the
//! mesh does not exist until a block does. So a puppet tool being armed on a
//! layer [arms a preview][arm]: the seam builds the mesh from the alpha it has
//! in hand at this frame and publishes it unmoved. That costs a mesh build (once
//! — it is cached by content) and no extra render at all, because the pixels the
//! alpha comes from are the ones the layer was going to draw anyway.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use lumit_core::puppet::PuppetMesh;
use uuid::Uuid;

/// The wireframe one layer is showing at the frame just built.
#[derive(Debug, Clone)]
pub struct Ghost {
    /// The rest mesh — vertices in layer px at natural size, and the triangles
    /// over them. Shared with the render's own cache, so publishing costs a
    /// refcount rather than a copy.
    pub mesh: Arc<PuppetMesh>,
    /// Where those vertices are at this frame, layer px, one per rest vertex.
    /// Equal to the rest positions when nothing is pinned yet.
    pub deformed: Vec<[f64; 2]>,
    /// Pins whose rest position fell outside the mesh: kept in the document,
    /// drawn hollow, contributing nothing (docs/impl/puppet.md §6).
    pub inert: Vec<Uuid>,
}

/// The preview a puppet tool arms: which layer wants a mesh before it has a
/// block, and the density and expansion to build it at.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Preview {
    layer: Uuid,
    density: f64,
    expansion: f64,
}

#[derive(Default)]
struct State {
    ghosts: HashMap<Uuid, Arc<Ghost>>,
    preview: Option<Preview>,
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(Mutex::default)
}

/// Leave `ghost` where the overlay can find it, replacing whatever this layer
/// had before.
///
/// Called from the paint seam, once per puppeted layer per built frame. A
/// poisoned lock drops the update: an overlay one frame stale is not worth a
/// panic (docs/14 §4).
pub fn publish(layer: Uuid, ghost: Ghost) {
    if let Ok(mut held) = state().lock() {
        held.ghosts.insert(layer, Arc::new(ghost));
    }
}

/// Take this layer's wireframe away — the mesh could not be built at this
/// frame, and a stale one would be a wireframe over a picture it no longer fits.
pub fn forget(layer: Uuid) {
    if let Ok(mut held) = state().lock() {
        held.ghosts.remove(&layer);
    }
}

/// What this layer is showing, or `None` when no frame carrying a puppet on it
/// has been built yet.
#[must_use]
pub fn ghost(layer: Uuid) -> Option<Arc<Ghost>> {
    state().lock().ok()?.ghosts.get(&layer).cloned()
}

/// Ask for a mesh on a layer that has no block yet — what arming a puppet tool
/// does, and what makes the first pin placeable.
///
/// `None` stands the preview down. Only one layer at a time: a puppet tool acts
/// on the selected layer, and a second preview would be a mesh nobody is
/// looking at.
pub fn arm(layer: Option<(Uuid, f64, f64)>) {
    if let Ok(mut held) = state().lock() {
        let next = layer.map(|(layer, density, expansion)| Preview {
            layer,
            density,
            expansion,
        });
        if held.preview != next {
            // The mesh the old preview left is not this layer's any more.
            if let Some(old) = held.preview {
                held.ghosts.remove(&old.layer);
            }
            held.preview = next;
        }
    }
}

/// The density and expansion this layer's preview wants, when it has one.
#[must_use]
pub fn previewing(layer: Uuid) -> Option<(f64, f64)> {
    let held = state().lock().ok()?;
    held.preview
        .filter(|p| p.layer == layer)
        .map(|p| (p.density, p.expansion))
}

/// Forget everything — closing a project, as the track store does.
pub fn clear() {
    if let Ok(mut held) = state().lock() {
        held.ghosts.clear();
        held.preview = None;
    }
}
