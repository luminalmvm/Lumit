//! Which of a rebuilt layer's neighbours are its own picture again.
//!
//! A Precomp or adjustment layer has no decoded frames, so its neighbours are
//! its picture built again a frame away. Over a still, a freeze frame or
//! footage slower than the comp that is the same picture, and Denoise must not
//! count the same noise twice. Pictures are told apart by name: the nested
//! comp's frame name, or for an adjustment layer the name the layers below it
//! would have as a comp of their own. A picture with no name is kept, as it
//! always was. Runs on the render thread.

use lumit_core::fx::temporal::{leave_out_repeats, FlowWant};
use lumit_core::model::{Composition, Layer, LayerKind};

use crate::cache::NestedKeyer;

/// What a rebuilt layer's neighbours are measured by (`rebuild_offsets`).
type Offsets = [(i32, Option<(i32, FlowWant)>)];

/// Takes the field off each neighbour of a Precomp layer whose nested comp,
/// named at `at(offset)`, is the frame's own picture again.
pub(super) fn in_nested(
    offsets: &mut Offsets,
    keys: Option<&dyn NestedKeyer>,
    layer: &Layer,
    nested: &Composition,
    at: impl Fn(i32) -> f64,
) {
    let Some(keys) = keys.filter(|_| wants_some(offsets)) else {
        return;
    };
    let inputs = layer.graph_inputs.as_ref();
    leave_out_repeats(offsets, |o| keys.nested_key_with(nested, at(o), inputs));
}

/// The same for the adjustment layer at `idx` in `comp`, whose picture is the
/// layers below it at comp time `at(offset)`.
pub(super) fn below(
    offsets: &mut Offsets,
    keys: Option<&dyn NestedKeyer>,
    comp: &Composition,
    idx: usize,
    at: impl Fn(i32) -> f64,
) {
    let Some(keys) = keys.filter(|_| wants_some(offsets)) else {
        return;
    };
    // The adjustment and every layer above it are switched off rather than
    // taken out, so the layers below keep their parents. Cameras and lights
    // still frame and light them.
    let mut alone = comp.clone();
    for l in alone.layers.iter_mut().take(idx + 1) {
        if !matches!(l.kind, LayerKind::Camera { .. } | LayerKind::Light { .. }) {
            l.switches.visible = false;
            l.switches.solo = false;
        }
    }
    leave_out_repeats(offsets, |o| keys.nested_key(&alone, at(o)));
}

/// Whether any neighbour is measured for Denoise, the one reader that cares.
fn wants_some(offsets: &Offsets) -> bool {
    offsets
        .iter()
        .any(|(_, m)| matches!(m, Some((_, w)) if *w != FlowWant::Full))
}
