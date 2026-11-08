//! The order a segment's 3D layers draw in, and the runs it cuts them into.

use super::placed_in_3d;
use super::shadows::shadow_only;
use crate::draw::CompLayerDraw;
use crate::model_layer::{middle, model_run_order};

/// The order a segment's layers draw in: each run of adjacent 3D layers
/// farthest first, as After Effects draws them, with how each draw is cut
/// where layers pass through each other (a layer cut into pieces draws once
/// per piece). A 2D layer ends a run, even one drawn on a collapsed 3D
/// precomp's plane, and so does an adjustment layer, which already splits
/// the segments. With no 3D layer, or no camera (only a Sequence mix, whose
/// parts are 2D), the stack order stands.
pub(super) fn draw_order(
    layers: &[CompLayerDraw],
    textures: &[wgpu::Texture],
    grown: &[(f32, f32)],
    camera: Option<(glam::Mat4, [f32; 3])>,
    emitters: &[(usize, glam::Vec3)],
) -> Vec<(usize, Option<lumit_gpu::composite::Cut>)> {
    // A layer there only for its shadows draws nothing, so it neither takes
    // a place nor cuts anything.
    let shown = |i: &usize| !shadow_only(&layers[*i]);
    let Some((view_proj, eye)) = camera.filter(|_| layers.iter().any(|l| l.three_d)) else {
        return (0..layers.len()).filter(shown).map(|i| (i, None)).collect();
    };
    let mut order = Vec::with_capacity(layers.len());
    let mut start = 0;
    while start < layers.len() {
        let end = (start..layers.len())
            .find(|&i| !layers[i].three_d || layers[i].ends_run)
            .unwrap_or(layers.len());
        let run: Vec<usize> = (start..end).filter(shown).collect();
        let worlds: Vec<glam::Mat4> = run
            .iter()
            .map(|&i| placed_in_3d(&textures[i], &layers[i], grown[i]).world())
            .collect();
        // A motion-blurred layer draws as a flat smear, which can't be cut
        // itself, so whatever it crosses is cut along its plane instead.
        let flat: Vec<bool> = run.iter().map(|&i| !layers[i].mb.is_empty()).collect();
        // A run holding a model or particles in the scene is cut by depth
        // throughout, the model placed by the middle of its box and the
        // particles by their emitter.
        let middles: Vec<_> = run
            .iter()
            .map(|&i| {
                let emitter = emitters.iter().find(|(k, _)| *k == i).map(|(_, at)| *at);
                middle(&layers[i]).or(emitter)
            })
            .collect();
        let sorted = if middles.iter().any(Option::is_some) {
            model_run_order(&worlds, &middles, &flat, view_proj)
        } else {
            lumit_gpu::composite::order::depth_order(&worlds, &flat, view_proj, eye)
        };
        order.extend(
            sorted
                .into_iter()
                .filter_map(|(i, cut)| Some((*run.get(i)?, cut))),
        );
        // The 2D layer that ended the run keeps its place.
        if end < layers.len() && shown(&end) {
            order.push((end, None));
        }
        start = end + 1;
    }
    order
}

/// The draws in their order cut into the runs [`draw_order`] sorted: each
/// run of 3D layers, and each layer that ended one on its own.
pub(super) fn runs_of<'a, 'b>(
    layers: &[CompLayerDraw],
    order: &[usize],
    placed: &'b [lumit_gpu::composite::CompositeLayer<'a>],
) -> Vec<&'b [lumit_gpu::composite::CompositeLayer<'a>]> {
    let mut runs = Vec::new();
    let mut start = 0;
    for (k, &i) in order.iter().enumerate() {
        if layers.get(i).is_some_and(|l| !l.three_d || l.ends_run) {
            runs.push(&placed[start..k]);
            runs.push(&placed[k..=k]);
            start = k + 1;
        }
    }
    runs.push(&placed[start..]);
    runs
}
