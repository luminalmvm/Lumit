//! **Values per copy**: drivers resolved once for each point of a points
//! stream, so a driver that reads the copy gives every stamp or label its own
//! value.

use crate::fx::drivers::clone_index;
use crate::fx::params::{ParamId, Value};
use crate::fx::points::{self, PointsStream};

/// The named columns [`drive_rows`] writes and Clone to points' stamps read: Scale, Rotation and Opacity per stamp, in the rows' own units.
pub const COPY_SCALE: &str = "copy:scale";
pub const COPY_ROTATION: &str = "copy:rotation";
pub const COPY_OPACITY: &str = "copy:opacity";

/// The rows a driver can give each stamp its own value for, with the column
/// each is written into.
pub const COPY_ROWS: &[(&str, &str)] = &[
    ("scale", COPY_SCALE),
    ("rotation", COPY_ROTATION),
    ("opacity", COPY_OPACITY),
];

/// The named column `name` of `stream`, one value per point, or `row` for
/// every point where the column was never written.
#[must_use]
pub fn column_or(stream: &PointsStream, name: &str, row: f32) -> Vec<f32> {
    let at = match stream.column(name) {
        points::Column::Named(at) => stream.named.get(at),
        _ => None,
    };
    (0..stream.len())
        .map(|i| at.and_then(|c| c.values.get(i)).map_or(row, |v| v[0]))
        .collect()
}

/// The most points [`drive_rows`] evaluates one by one; past it the rest
/// repeat the last value. Each point is a resolve of the layer's drivers.
pub const COPY_DRIVE_MAX: usize = 4096;

/// **Values per copy**, for Clone to points' Scale, Rotation and Opacity and
/// Label points' driven rows (`rows`: a row id and the named column to write).
///
/// A Stagger, Random, Oscillator, Value noise or Clone index
/// wired into Clone to points' own Scale, Rotation or Opacity gives every
/// stamp its own value, as a Cavalry behaviour wired into a Duplicator does.
/// The layer's drivers are resolved once per point, as copy `i` of `n`
/// standing at the point's place (which Oscillator and Value noise can vary
/// by), and the values are written into `stream` as the [`COPY_SCALE`], [`COPY_ROTATION`] and
/// [`COPY_OPACITY`] columns, which `stamps` reads in
/// place of the row. The stamp is turned and sized as a whole, so nothing is
/// cropped the way a Transform on the cloned layer crops.
///
/// Does nothing unless one of the three rows is wired and the layer's graph
/// holds a driver that reads the copy ([`clone_index::present`]); every other
/// layer keeps its one value per row, as before.
#[allow(clippy::too_many_arguments)]
pub fn drive_rows(
    graph: &crate::graph::LayerGraph,
    consumer: uuid::Uuid,
    stream: &mut PointsStream,
    rows: &[(&'static str, &'static str)],
    lt: f64,
    context: &std::sync::Arc<crate::expression::ExpressionContext>,
    audio: Option<&dyn crate::fx::registry::AudioTap>,
    projection: points::Projection,
) {
    use crate::graph::{InputRef, NodeRef};
    let wired: Vec<(&str, &str)> = rows
        .iter()
        .copied()
        .filter(|(row, _)| {
            graph
                .wire_into(&InputRef::Param {
                    node: NodeRef::Effect(consumer),
                    port: (*row).to_owned(),
                })
                .is_some()
        })
        .collect();
    if wired.is_empty() || !clone_index::present(graph) {
        return;
    }
    let n = stream.len().min(COPY_DRIVE_MAX);
    let count = u32::try_from(stream.len()).unwrap_or(u32::MAX);
    // Each point resolves the whole graph on its own, so the points are
    // shared out over the cores.
    // Where every point is, for a driver that keeps away from them.
    let cloud = points::Cloud::new(stream.position.iter().map(|p| [p[0], p[1]]));
    use rayon::prelude::*;
    let per_point: Vec<Vec<Option<f32>>> = (0..n)
        .into_par_iter()
        .map(|i| {
            let index = u32::try_from(i).unwrap_or(u32::MAX);
            let at = stream.position.get(i).map_or([0.0; 2], |p| [p[0], p[1]]);
            let resolved = crate::fx::drivers::resolve_drivers_at_point(
                graph,
                lt,
                context.clone(),
                audio,
                projection,
                (index, count),
                at,
                Some(&cloud),
            );
            wired
                .iter()
                .map(|(row, _)| {
                    match resolved.param(NodeRef::Effect(consumer), ParamId::new(row)) {
                        Some(Value::Float(v)) => Some(v),
                        #[allow(clippy::cast_precision_loss)]
                        Some(Value::Int(v)) => Some(v as f32),
                        _ => None,
                    }
                })
                .collect()
        })
        .collect();
    let values: Vec<Vec<Option<f32>>> = (0..wired.len())
        .map(|r| per_point.iter().map(|p| p[r]).collect())
        .collect();
    for ((_, name), column) in wired.iter().zip(values) {
        // A row the driver did not answer for keeps the column unwritten, and
        // so its one value.
        if column.iter().any(Option::is_none) {
            continue;
        }
        let Some(cells) = stream.named_mut(name, 1) else {
            continue;
        };
        let last = column.last().copied().flatten().unwrap_or(0.0);
        for (i, cell) in cells.iter_mut().enumerate() {
            cell[0] = column.get(i).copied().flatten().unwrap_or(last);
        }
    }
}
