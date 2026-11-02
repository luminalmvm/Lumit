//! After Effects' Gradient Wipe onto Luma fade.

use lumit_core::anim::Property;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property as AeProperty;
use crate::report::{ItemPath, Outcome, Reason};

use super::fx_colour::Fx;
use super::props::match_name_of;
use super::Conv;

/// This file's claim on a match name: the row whose conversion is
/// `gradient_wipe`.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &AeProperty,
) -> Option<EffectInstance> {
    let row = super::table::table().row(match_name_of(node))?;
    if row.conversion != "gradient_wipe" {
        return None;
    }
    let before = conv.report.rows.len();
    let mut fx = Fx::new(path, node, row)?;
    gradient_wipe(conv, path, &mut fx);
    if conv.report.rows.len() == before {
        conv.report.imported();
    }
    fx.done()
}

/// Completion and Softness are fractions in After Effects and per cents here.
/// The Gradient Layer becomes the Matte, except when it is the layer the
/// effect is on, which is After Effects' default and Lumit's empty Matte.
fn gradient_wipe(conv: &mut Conv<'_>, path: &ItemPath, fx: &mut Fx<'_>) {
    // After Effects' own defaults first: the file only keeps what changed.
    fx.set("completion", EffectValue::Float(Property::fixed(0.0)));
    fx.set("softness", EffectValue::Float(Property::fixed(0.0)));
    fx.float(conv, "ADBE Gradient Wipe-0001", "completion", 100.0, 0.0);
    fx.float(conv, "ADBE Gradient Wipe-0002", "softness", 100.0, 0.0);
    // No leaf is the default, which is this layer.
    let source = if fx.leaf("ADBE Gradient Wipe-0003").is_some() {
        fx.still(conv, "ADBE Gradient Wipe-0003")
            .map_or(0, |v| v.round().max(0.0) as u32)
    } else {
        conv.self_index
    };
    if source != 0 && source != conv.self_index {
        match conv.layer_ids.get(&source).copied() {
            Some(layer) => fx.set("matte", EffectValue::Layer(Some(layer))),
            None => conv.report.row(
                path.clone(),
                Outcome::Adjusted,
                Reason::MatteTargetMissing { index: source },
            ),
        }
        // 1 Tile, 2 Center, 3 Stretch to Fit, which is the default.
        fx.set("matte_placement", EffectValue::Choice(1));
        fx.choice(
            conv,
            "ADBE Gradient Wipe-0004",
            "matte_placement",
            |v| match v {
                1 => (1, Some("Stretch to fit")),
                2 => (2, None),
                _ => (1, None),
            },
        );
    }
    fx.toggle(conv, "ADBE Gradient Wipe-0005", "matte_invert");
}
