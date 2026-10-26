//! After Effects' Grid onto Grid's Lines type.

use lumit_core::anim::Property as LumProperty;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::props::match_name_of;
use super::Conv;

/// This file's claim on a match name: the row whose conversion is `grid`.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let row = super::table::table().row(match_name_of(node))?;
    if row.conversion != "grid" {
        return None;
    }
    let before = conv.report.rows.len();
    let mut fx = Fx::new(path, node, row)?;
    fx.set("kind", EffectValue::Choice(1));
    ae_grid(conv, node, &mut fx);
    if conv.report.rows.len() == before {
        conv.report.imported();
    }
    fx.done()
}

/// A still number on a Lumit row. The `.aep` route only carries the
/// controls someone changed, so After Effects' own defaults go in first.
fn seed(fx: &mut Fx<'_>, lumit_id: &str, v: f64) {
    fx.set(lumit_id, EffectValue::Float(LumProperty::fixed(v)));
}

/// Drops an After Effects control that Lumit has nothing for, when it is set
/// to anything but the value that does nothing.
fn unless(fx: &mut Fx<'_>, conv: &mut Conv<'_>, ae_id: &str, name: &str, neutral: f64) {
    if fx
        .still(conv, ae_id)
        .is_some_and(|v| (v - neutral).abs() > 1e-6)
    {
        fx.drop_param(conv, name);
    }
}

/// "Grid": Anchor and Corner are two corners of one cell, and the two slider
/// sizes become the corner the same cell would have. Border is the line's
/// width.
fn ae_grid(conv: &mut Conv<'_>, node: &Property, fx: &mut Fx<'_>) {
    let (w, h) = conv.size;
    fx.set("size_from", EffectValue::Choice(1));
    seed(fx, "position_x", 0.5 * w);
    seed(fx, "position_y", 0.5 * h);
    seed(fx, "line_width", 5.0);
    seed(fx, "major_line_spacing", 0.0);
    fx.point(conv, "ADBE Grid-0001", "position_x", "position_y", 1.0);
    let anchor = |axis| super::props::still(node.children(), "ADBE Grid-0001", axis);
    let (ax, ay) = (anchor(0).unwrap_or(0.5 * w), anchor(1).unwrap_or(0.5 * h));
    // 1 Corner point, 2 Width slider (square cells), 3 Width and height.
    match fx.still(conv, "ADBE Grid-0002").map(|v| v.round() as i64) {
        Some(2) => {
            seed(fx, "corner_x", ax + 32.0);
            seed(fx, "corner_y", ay + 32.0);
            fx.float(conv, "ADBE Grid-0004", "corner_x", 1.0, ax);
            fx.float(conv, "ADBE Grid-0004", "corner_y", 1.0, ay);
        }
        Some(3) => {
            seed(fx, "corner_x", ax + 32.0);
            seed(fx, "corner_y", ay + 24.0);
            fx.float(conv, "ADBE Grid-0004", "corner_x", 1.0, ax);
            fx.float(conv, "ADBE Grid-0005", "corner_y", 1.0, ay);
        }
        _ => {
            seed(fx, "corner_x", 0.6 * w);
            seed(fx, "corner_y", 0.6 * h);
            fx.point(conv, "ADBE Grid-0003", "corner_x", "corner_y", 1.0);
        }
    }
    fx.float(conv, "ADBE Grid-0006", "line_width", 1.0, 0.0);
    unless(fx, conv, "ADBE Grid-0008", "Feather", 0.0);
    unless(fx, conv, "ADBE Grid-0009", "Feather", 0.0);
    unless(fx, conv, "ADBE Grid-0011", "Invert Grid", 0.0);
    fx.colour(conv, "ADBE Grid-0012", "colour");
    fx.float(conv, "ADBE Grid-0013", "mix", 1.0, 0.0);
    // None draws the grid on its own; Normal draws it over the layer, as
    // Lumit's does.
    match fx.still(conv, "ADBE Grid-0014").map(|v| v.round() as i64) {
        Some(1) => fx.approx_named(conv, "Blending Mode", "the grid drawn over the layer"),
        Some(2) | None => {}
        Some(_) => fx.drop_param(conv, "Blending Mode"),
    }
}
