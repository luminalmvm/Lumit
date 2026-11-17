//! After Effects' Warp Stabilizer onto Stabilise. After Effects keeps its
//! analysis in the project, where Lumit cannot read it, so the layer is
//! analysed again when it is first drawn.

use lumit_core::anim::Property;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property as AeProperty;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::props::match_name_of;
use super::Conv;

/// This file's claim on a match name: the row whose conversion is
/// `warp_stabilizer`.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &AeProperty,
) -> Option<EffectInstance> {
    let row = super::table::table().row(match_name_of(node))?;
    if row.conversion != "warp_stabilizer" {
        return None;
    }
    let before = conv.report.rows.len();
    let mut fx = Fx::new(path, node, row)?;
    let live = warp_stabilizer(conv, &mut fx);
    if conv.report.rows.len() == before {
        conv.report.imported();
    }
    let mut inst = fx.done()?;
    inst.enabled &= live;
    Some(inst)
}

/// The parameter's id, `ADBE SubspaceStabilizer-00nn`.
fn id(n: u32) -> String {
    format!("ADBE SubspaceStabilizer-{n:04}")
}

/// The four Synthesis Edge Cropping sides onto the four trims.
const TRIMS: [(u32, &str); 4] = [
    (31, "trim_left"),
    (32, "trim_top"),
    (33, "trim_right"),
    (34, "trim_bottom"),
];

/// Every control Stabilise has a row for comes across. Subspace Warp becomes
/// Perspective, and Reverse Stabilization becomes Reverse on this layer until
/// the stabilised layer is chosen. Rolling Shutter Ripple's two reductions
/// both become Reduce: After Effects has no setting that leaves it alone,
/// where Lumit's default is Off. Fast Analysis, Hide Warning Banner and
/// Auto-delete Points Across Time are left behind. Answers `false` for Apply
/// Motion to Target, which moves another layer with the shake rather than
/// taking it out of this one: the Stabilise comes across switched off.
fn warp_stabilizer(conv: &mut Conv<'_>, fx: &mut Fx<'_>) -> bool {
    // After Effects' own defaults where they differ from Lumit's: the file
    // only keeps what changed.
    fx.set("fill_feather", EffectValue::Float(Property::fixed(10.0)));
    fx.set("steady_crop", EffectValue::Float(Property::fixed(50.0)));
    // 1 Automatic Reduction, 2 Enhanced Reduction, and 1 when the file leaves
    // it out.
    fx.set("rolling_shutter", EffectValue::Choice(1));
    fx.choice(conv, &id(37), "rolling_shutter", |_| (1, None));
    // 1 Smooth Motion, 2 No Motion.
    fx.choice(conv, &id(19), "result", |v| (u32::from(v == 2), None));
    fx.float(conv, &id(20), "smoothness", 1.0, 0.0);
    // 1 Position, 2 Position, Scale, Rotation, 3 Perspective, 4 Subspace
    // Warp, which is After Effects' default.
    let method = fx.still(conv, &id(28)).map_or(4, |v| v.round() as i64);
    fx.set(
        "method",
        EffectValue::Choice((method - 1).clamp(0, 2) as u32),
    );
    if method >= 4 {
        fx.approx_named(conv, "Method", "Perspective, the nearest Lumit has");
    }
    fx.toggle(conv, &id(44), "keep_scale");
    // 1 Stabilize Only, 2 Stabilize, Crop, 3 Stabilize, Crop, Auto-scale,
    // 4 Stabilize, Synthesize Edges, in Edges' own order.
    fx.choice(conv, &id(22), "edges", |v| {
        ((v - 1).clamp(0, 3) as u32, None)
    });
    fx.float(conv, &id(4), "max_scale", 1.0, 0.0);
    fx.float(conv, &id(3), "margin", 1.0, 0.0);
    fx.float(conv, &id(16), "extra_scale", 1.0, 0.0);
    // Both run from cropping least at nought to one crop for the whole shot
    // at 100.
    fx.float(conv, &id(23), "steady_crop", 1.0, 0.0);
    fx.float(conv, &id(38), "fill_from", 1.0, 0.0);
    fx.float(conv, &id(35), "fill_feather", 1.0, 0.0);
    for (n, to) in TRIMS {
        fx.float(conv, &id(n), to, 1.0, 0.0);
    }
    // Detailed Analysis looks for more to follow: Feature density's High.
    if fx.still(conv, &id(36)).is_some_and(|v| v != 0.0) {
        fx.set("density", EffectValue::Choice(2));
    }
    fx.toggle(conv, &id(280), "show_points");
    fx.float(conv, &id(301), "point_size", 1.0, 0.0);
    // 1 Stabilize, 2 a separator, 3 Reversible Stabilization, 4 Reverse
    // Stabilization, 5 Apply Motion to Target, 6 the same over the original.
    // Reversible is the same analysis, and Edges says what shows round it.
    match fx.still(conv, &id(47)).map(|v| v.round() as i64) {
        Some(4) => {
            fx.set("mode", EffectValue::Choice(1));
            fx.approx_named(
                conv,
                "Objective",
                "Reverse, with this layer as the stabilised layer until another is chosen",
            );
        }
        Some(5 | 6) => {
            fx.approx_named(
                conv,
                "Objective",
                "a Stabilise switched off: Lumit does not move another layer with the shake",
            );
            return false;
        }
        _ => {}
    }
    true
}
