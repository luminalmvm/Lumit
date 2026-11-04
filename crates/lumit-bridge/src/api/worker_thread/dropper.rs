//! The dropper's helpers: the window cut out of a picture, and the composition
//! with one layer soloed for a layer read.

use flutter_rust_bridge::frb;
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use super::{quality_for, LayerSample};
use super::{viewed_through, SamplePixelsRequest, WorkerState, MAX_WINDOW};

/// A window cut out of a picture: the pixels, and where its centre landed.
#[frb(ignore)]
pub(crate) struct Patch {
    pub window: u32,
    pub rgba: Vec<u8>,
    pub x: u32,
    pub y: u32,
}

/// Cut a `window × window` square centred on the fraction `(u, v)` of a
/// picture.
///
/// `window` is forced odd and capped at [`MAX_WINDOW`] here rather than
/// trusted: the centre must be a single pixel for the magnifier's centre cell
/// to mean anything, and the payload must stay small enough to be a reading
/// rather than a picture. Pixels off the edge repeat the edge, so the square is
/// always exactly `window × window` and the caller never has a ragged one to
/// draw — and the frontend can index it without a bounds case at the picture's
/// border.
#[frb(ignore)]
pub(crate) fn cut_patch(
    rgba: &[u8],
    width: u32,
    height: u32,
    u: f64,
    v: f64,
    window: u32,
) -> Option<Patch> {
    if width == 0 || height == 0 || rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }
    let grid = window.clamp(1, MAX_WINDOW) | 1;
    let w = width as i64;
    let h = height as i64;
    let cx = ((u * width as f64) as i64).clamp(0, w - 1);
    let cy = ((v * height as f64) as i64).clamp(0, h - 1);
    let half = i64::from(grid / 2);

    let mut out = Vec::with_capacity((grid * grid * 4) as usize);
    for dy in 0..i64::from(grid) {
        for dx in 0..i64::from(grid) {
            let px = (cx - half + dx).clamp(0, w - 1);
            let py = (cy - half + dy).clamp(0, h - 1);
            let i = ((py * w + px) * 4) as usize;
            out.extend_from_slice(&rgba[i..i + 4]);
        }
    }
    Some(Patch {
        window: grid,
        rgba: out,
        x: cx as u32,
        y: cy as u32,
    })
}

/// The composition with `layer` soloed and shown, seen through the view's own
/// 3D view so the read lands where the pointer is.
#[frb(ignore)]
pub(super) fn soloed(
    document: &lumit_core::model::Document,
    layer: Uuid,
    req: &SamplePixelsRequest,
    state: &WorkerState,
) -> Option<std::sync::Arc<lumit_core::model::Document>> {
    // A patched *copy* of the snapshot: soloing for the read must never be
    // something the document remembers, so nothing here goes near `commit`.
    let mut patched = lumit_core::model::Document::clone(document);
    let comp = patched.comp_mut(req.comp.id)?;
    for l in &mut comp.layers {
        l.switches.solo = l.id == layer;
        // Soloed and still hidden is nothing at all — and a depth pass is very
        // often hidden, which is exactly why this read exists.
        if l.id == layer {
            l.switches.visible = true;
        }
    }

    // Seen through the view's own 3D view, so the read lands where the
    // pointer is.
    Some(viewed_through(
        None,
        state.camera_views.get(&req.view).copied(),
        std::sync::Arc::new(patched),
    ))
}

/// The composition rendered with one layer soloed — that layer alone, in its
/// own place, on nothing.
///
/// Held in `state.layer_sample` against `(comp, frame, layer, revision,
/// view)`: the dropper asks again on every pointer move, and re-compositing
/// the whole composition per move is not a thing to do while someone is
/// dragging a pointer. An edit moves the document's revision, which retires
/// the entry.
#[frb(ignore)]
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn sample_layer_alone(
    document: &lumit_core::model::Document,
    revision: u64,
    layer: Uuid,
    req: &SamplePixelsRequest,
    state: &mut WorkerState,
) -> Option<(u32, u32, Vec<u8>)> {
    let stamp = (req.comp.id, req.frame, layer, revision, req.view);
    if let Some(held) = &state.layer_sample {
        if held.stamp == stamp {
            return Some((held.width, held.height, held.rgba.clone()));
        }
    }

    let patched = soloed(document, layer, req, state)?;
    let (rgba, w, h) = state
        .renderer
        .render_preview(
            &patched,
            req.comp.id,
            req.frame,
            quality_for(req.scale),
            req.scale,
        )
        .ok()?;
    state.layer_sample = Some(LayerSample {
        stamp,
        width: w,
        height: h,
        rgba: rgba.clone(),
    });
    Some((w, h, rgba))
}
