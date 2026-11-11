//! A Viewer view's 3D view as the worker holds it: the copy of the document
//! seen through it, its ground, and letting go of both.

use flutter_rust_bridge::frb;
use uuid::Uuid;

use super::{viewed, ViewLatch, WorkerState};

/// The id the Viewer's view camera is inserted under.
#[frb(ignore)]
pub(super) const VIEW_CAMERA_ID: Uuid = lumit_core::view_camera::ID;

/// [`viewed_through`], kept between frames.
///
/// A view or a prefix point makes a copy of the whole document, which for a
/// large project is several milliseconds, and playback and the idle fill asked
/// for one every frame. The copy made for a revision and a way of looking is
/// the copy for the next frame too. A few are kept, so a 3D viewport playing
/// along beside the Viewer doesn't throw away the Viewer's copy, or its own.
#[frb(ignore)]
pub(super) fn viewed_kept(
    kept: &mut Vec<Viewed>,
    prefix: Option<crate::api::state::prefix::BridgePrefixPoint>,
    view: Option<ViewLatch>,
    revision: u64,
    document: std::sync::Arc<lumit_core::model::Document>,
) -> std::sync::Arc<lumit_core::model::Document> {
    // A data row of a graphics template being looked at, which is a way of
    // looking as the other two are.
    let row = crate::api::template::rows::row_look();
    // Nothing is copied, so there is nothing to keep.
    if prefix.is_none() && view.is_none() && row.is_none() {
        return document;
    }
    if let Some(at) = kept.iter().position(|was| {
        (was.revision, was.prefix, was.view, was.row) == (revision, prefix, view, row)
    }) {
        let was = kept.remove(at);
        let document = was.document.clone();
        kept.insert(0, was);
        return document;
    }
    let document = viewed_through(prefix, view, document);
    // A copy of an older revision is never asked for again.
    kept.retain(|was| was.revision == revision);
    kept.insert(
        0,
        Viewed {
            revision,
            prefix,
            view,
            row,
            document: document.clone(),
        },
    );
    kept.truncate(VIEWED_KEPT);
    document
}

/// How many looks at the document [`viewed_kept`] keeps: the Viewer's, a 3D
/// viewport's, and one more.
#[frb(ignore)]
const VIEWED_KEPT: usize = 3;

/// What [`viewed_kept`] keeps.
#[frb(ignore)]
pub(super) struct Viewed {
    revision: u64,
    prefix: Option<crate::api::state::prefix::BridgePrefixPoint>,
    view: Option<ViewLatch>,
    row: Option<crate::api::template::rows::RowLook>,
    document: std::sync::Arc<lumit_core::model::Document>,
}

/// [`viewed`], then looked through the Viewer's 3D view when one is set
/// (docs/impl/camera.md §6): a Camera layer holding the view's pose is put at
/// the top of the named comp, where it is the active camera for the render,
/// the frame key and every nested reading alike. No second render path: a
/// patched copy of the snapshot goes down the ordinary one. Another comp's
/// view leaves the document alone. A 3D viewport's draft view also leaves out
/// the comp's motion blur, and a view drawing its models as lines draws
/// nothing else, as it outlines every other layer over the picture.
#[frb(ignore)]
pub(super) fn viewed_through(
    prefix: Option<crate::api::state::prefix::BridgePrefixPoint>,
    view: Option<ViewLatch>,
    document: std::sync::Arc<lumit_core::model::Document>,
) -> std::sync::Arc<lumit_core::model::Document> {
    let document = viewed(prefix, document);
    // The data row being looked at first, so the view's camera sees it.
    let document = match crate::api::template::rows::row_look() {
        Some(look) => crate::api::template::rows::looked(look, document),
        None => document,
    };
    let Some(ViewLatch {
        comp: comp_id,
        pose,
        draft,
        canvas,
        display,
    }) = view
    else {
        return document;
    };
    // Only a fixed or custom view outlines the layers it leaves out.
    let lines = display.shading == lumit_gpu::mesh::display::Shading::Wireframe && pose.is_some();
    let Some(comp) = document.comp(comp_id).filter(|_| pose.is_some() || lines) else {
        return document;
    };
    let camera = pose.map(|pose| {
        let mut layer = crate::edits::fresh_camera(comp, "View".into(), None);
        layer.id = VIEW_CAMERA_ID;
        if let lumit_core::model::LayerKind::Camera { zoom, .. } = &mut layer.kind {
            *zoom = lumit_core::anim::Property::fixed(pose.zoom);
        }
        let tr = &mut layer.transform;
        tr.position_x = lumit_core::anim::Property::fixed(pose.position.0);
        tr.position_y = lumit_core::anim::Property::fixed(pose.position.1);
        tr.position_z = lumit_core::anim::Property::fixed(pose.position.2);
        tr.rotation_x = lumit_core::anim::Property::fixed(pose.rotation_deg.0);
        tr.rotation_y = lumit_core::anim::Property::fixed(pose.rotation_deg.1);
        tr.rotation = lumit_core::anim::Property::fixed(pose.rotation_deg.2);
        layer
    });
    let mut doc = (*document).clone();
    if let Some(c) = doc.comp_mut(comp_id) {
        if let Some(layer) = camera {
            c.layers.insert(0, layer);
        }
        // Hidden rather than taken out, so they still parent what they parent.
        for l in c.layers.iter_mut().filter(|_| lines) {
            l.switches.visible &= matches!(l.kind, lumit_core::model::LayerKind::Camera { .. })
                || lumit_core::extrude::draws_mesh(l);
        }
        c.motion_blur.enabled &= !draft;
        // A 3D viewport fills its pane: a wider picture round the comp's
        // middle, the 2D layers kept where they sit on the comp. The camera
        // looks at the picture's middle, so the 3D scene needs no moving.
        let (cw, ch) = canvas;
        if cw > 0 && ch > 0 && (cw, ch) != (c.width, c.height) {
            let dx = (f64::from(cw) - f64::from(c.width)) / 2.0;
            let dy = (f64::from(ch) - f64::from(c.height)) / 2.0;
            (c.width, c.height) = (cw, ch);
            for l in &mut c.layers {
                let placed_in_3d = l.switches.three_d
                    || matches!(
                        l.kind,
                        lumit_core::model::LayerKind::Camera { .. }
                            | lumit_core::model::LayerKind::Light { .. }
                    );
                if placed_in_3d || l.parent.is_some() {
                    continue;
                }
                shift(&mut l.transform.position_x, dx);
                shift(&mut l.transform.position_y, dy);
            }
        }
    }
    std::sync::Arc::new(doc)
}

/// Move a property's values by `by`, keys and all. An expression is left as
/// it is, so in a 3D viewport a 2D layer placed by one sits off by the shift,
/// and `thisComp.width` and `height` read the wider picture.
#[frb(ignore)]
fn shift(property: &mut lumit_core::anim::Property, by: f64) {
    match &mut property.animation {
        lumit_core::anim::Animation::Static(v) => *v += by,
        lumit_core::anim::Animation::Keyframed(keys) => {
            for key in keys {
                key.value += by;
            }
        }
        lumit_core::anim::Animation::Expression(_) => {}
    }
}

/// A 3D viewport's ground in `colour`, on the floor through the middle of
/// `comp` at half its height, as the Viewer's own ground sits.
#[frb(ignore)]
pub(super) fn ground_of(
    state: &WorkerState,
    comp: Uuid,
    colour: [f32; 4],
) -> Option<lumit_render::realise::ViewGround> {
    let project = state.project.state().ok()?;
    let project = project.read().ok()?;
    let document = project.store.snapshot();
    let c = document.comp(comp)?;
    let (w, h) = (f64::from(c.width), f64::from(c.height));
    Some(lumit_render::realise::ViewGround {
        colour,
        middle: (w * 0.5) as f32,
        floor: (h * 0.5) as f32,
        step: lumit_core::camera::nice(w.max(h) / 8.0) as f32,
    })
}

/// Let go of everything held for a closed view.
#[frb(ignore)]
pub(super) fn close_view(state: &mut WorkerState, view: u32) {
    if state
        .last_shown
        .as_ref()
        .is_some_and(|shown| shown.0 == view)
    {
        state.last_shown = None;
    }
    state.looks.remove(&view);
    state.beside_shown.retain(|shown| shown.0 != view);
    state.camera_views.remove(&view);
    state.grounds.remove(&view);
    state.names.forget_view(view);
    state.stand_ins.retain(|(v, ..)| *v != view);
    if state
        .angles
        .as_ref()
        .is_some_and(|latch| latch.view == view)
    {
        state.angles = None;
    }
    state.renderer.drop_view_targets(view);
}

/// Latch one view's 3D view and its ground, the way [`set_prefix`] latches
/// the chip: a change renames every frame of the comp in that view, so that
/// view's names, the bar and the fill start over.
#[frb(ignore)]
pub(super) fn set_view(
    state: &mut WorkerState,
    view: u32,
    pose: Option<ViewLatch>,
    ground: Option<lumit_render::realise::ViewGround>,
) {
    if state.camera_views.get(&view).copied() == pose && state.grounds.get(&view).copied() == ground
    {
        return;
    }
    match pose {
        Some(pose) => state.camera_views.insert(view, pose),
        None => state.camera_views.remove(&view),
    };
    match ground {
        Some(ground) => state.grounds.insert(view, ground),
        None => state.grounds.remove(&view),
    };
    state.names.forget_view(view);
    state.layer_sample = None;
    state.published_bar = None;
    state.fill_exhausted = false;
}
