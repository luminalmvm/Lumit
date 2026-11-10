//! The angle viewer's picture: a page of a multicam comp's angles, each drawn
//! into its own cell of one scratch composition.
//!
//! # In plain terms
//!
//! Each cell shows what a cut to its angle shows. For every angle on the page
//! the scratch document gains the multicam comp seen through that angle, and
//! the grid comp plays it from the start as a Precomp layer, scaled to fit its
//! cell and kept to its shape. A nested comp is clipped to its own frame, so an
//! angle punched in or larger than the frame never spills into its
//! neighbours, and parents, 3D and the comp's other layers (a title over every
//! angle) behave as they do in the program. Nothing in the grid is heard. A
//! sound angle has no picture and its cell stays dark. The names and the
//! outlines are drawn by the interface over the picture, in the cells
//! [`cells`] lays out.

use std::sync::Arc;

use flutter_rust_bridge::frb;
use lumit_core::anim::Property;
use lumit_core::model::{
    Composition, Document, LayerKind, LinearColour, ProjectItem, TransformGroup,
};
use lumit_core::time::{Duration, FrameRate};
use uuid::Uuid;

/// How a page of the angle viewer is laid out: columns and rows.
#[frb(non_opaque)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeAngleGrid {
    pub columns: u32,
    pub rows: u32,
    /// How many angles the page shows, and the first of them, counting from
    /// nought among the comp's angles.
    pub count: u32,
    pub first: u32,
    /// How many pages the comp's angles take.
    pub pages: u32,
}

/// The layout of page `page` of `angles` angles.
#[frb(sync)]
#[must_use]
pub fn angle_grid(angles: u32, page: u32) -> BridgeAngleGrid {
    let per = lumit_core::multicam::PAGE as u32;
    let pages = angles.div_ceil(per).max(1);
    let page = page.min(pages - 1);
    let first = page * per;
    let count = angles.saturating_sub(first).min(per);
    let (columns, rows) = lumit_core::multicam::grid(count as usize);
    BridgeAngleGrid {
        columns: columns as u32,
        rows: rows as u32,
        count,
        first,
        pages,
    }
}

/// The document with the grid composition for page `page` of `comp`'s angles
/// added under `id`, `size` pixels, or `None` when the comp has gone.
#[frb(ignore)]
pub(crate) fn document(
    document: &Arc<Document>,
    id: Uuid,
    comp: Uuid,
    page: u32,
    size: (u32, u32),
    rate: FrameRate,
    duration: Duration,
) -> Option<Arc<Document>> {
    let mc = document.comp(comp)?;
    let angles: Vec<&lumit_core::model::Layer> = lumit_core::multicam::offered(mc).collect();
    let grid = angle_grid(angles.len() as u32, page);
    let (w, h) = (size.0.clamp(16, 16384), size.1.clamp(16, 16384));
    let (cell_w, cell_h) = (
        f64::from(w) / f64::from(grid.columns.max(1)),
        f64::from(h) / f64::from(grid.rows.max(1)),
    );
    // The multicam comp's whole frame fitted into a cell, kept to its shape.
    let k = (cell_w / f64::from(mc.width.max(1))).min(cell_h / f64::from(mc.height.max(1)));
    let mut doc = document.as_ref().clone();
    let mut layers = Vec::new();
    for (i, angle) in angles
        .iter()
        .enumerate()
        .skip(grid.first as usize)
        .take(grid.count as usize)
    {
        if !super::draws(angle) {
            continue;
        }
        let at = (i - grid.first as usize) as u32;
        let (col, row) = (at % grid.columns.max(1), at / grid.columns.max(1));
        let x0 = f64::from(col) * cell_w + (cell_w - k * f64::from(mc.width)) / 2.0;
        let y0 = f64::from(row) * cell_h + (cell_h - k * f64::from(mc.height)) / 2.0;
        // Derived, never minted, so the cell's frames keep their names.
        let named = |what: &str| {
            let name = format!("{id}/{}{what}", angle.id);
            Uuid::new_v5(&crate::scratch::SCRATCH_NAMESPACE, name.as_bytes())
        };
        let mut seen = lumit_core::multicam::seen_through(mc, angle.id);
        seen.id = named("");
        // The multicam comp's frame, its top-left corner on the cell's.
        let place = TransformGroup {
            position_x: Property::fixed(x0),
            position_y: Property::fixed(y0),
            scale_x: Property::fixed(100.0 * k),
            scale_y: Property::fixed(100.0 * k),
            ..TransformGroup::default()
        };
        let kind = LayerKind::Precomp { comp: seen.id };
        let mut layer = crate::edits::base_layer(angle.name.clone(), kind, duration.0, place);
        layer.id = named("/layer");
        // Never heard: the angle viewer has no sound of its own.
        layer.switches.audible = false;
        doc.items.push(ProjectItem::Composition(seen));
        layers.push(layer);
    }
    let scratch = Composition {
        id,
        name: mc.name.clone(),
        width: w,
        height: h,
        frame_rate: rate,
        duration,
        // The cells stand on black, so a camera that was not rolling reads as
        // a dark cell rather than as the board behind the Viewer.
        background: LinearColour([0.0, 0.0, 0.0, 1.0]),
        work_area: None,
        layers,
        groups: Vec::new(),
        markers: Vec::new(),
        motion_blur: lumit_core::model::MotionBlur::default(),
        graph: None,
        master_volume_db: 0.0,
        lighting: Default::default(),
        sound_mix: false,
        beat_grid: None,
        extra: serde_json::Map::new(),
    };
    // Onto the clone and never through an op, as every scratch composition.
    doc.items.push(ProjectItem::Composition(scratch));
    Some(Arc::new(doc))
}
