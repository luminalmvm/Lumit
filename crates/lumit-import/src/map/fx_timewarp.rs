//! After Effects' Timewarp onto Timewarp.

use lumit_core::anim::Property;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property as AeProperty;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::props::match_name_of;
use super::Conv;

/// This file's claim on a match name: the row whose conversion is `timewarp`.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &AeProperty,
) -> Option<EffectInstance> {
    let row = super::table::table().row(match_name_of(node))?;
    if row.conversion != "timewarp" {
        return None;
    }
    let before = conv.report.rows.len();
    let mut fx = Fx::new(path, node, row)?;
    timewarp(conv, &mut fx);
    if conv.report.rows.len() == before {
        conv.report.imported();
    }
    fx.done()
}

/// The parameter's id, `ADBE Timewarp-00nn`.
fn id(n: u32) -> String {
    format!("ADBE Timewarp-{n:04}")
}

/// Drops an After Effects control Lumit has nothing for, when it is set to
/// anything but the value that does nothing.
fn unless(fx: &mut Fx<'_>, conv: &mut Conv<'_>, n: u32, name: &str, neutral: f64) {
    if fx
        .still(conv, &id(n))
        .is_some_and(|v| (v - neutral).abs() > 1e-6)
    {
        fx.drop_param(conv, name);
    }
}

/// Method, Adjust Time By, Speed and Source Frame carry as they are. Vector
/// Detail becomes Quality and Global Smoothness the Smoothness, both near
/// enough. The motion blur carries when its shutter is set by hand.
fn timewarp(conv: &mut Conv<'_>, fx: &mut Fx<'_>) {
    // After Effects' own defaults first: the file only keeps what changed.
    fx.set("method", EffectValue::Choice(2));
    fx.set("speed", EffectValue::Float(Property::fixed(50.0)));
    // 1 Whole Frames, 2 Frame Mix, 3 Pixel Motion.
    fx.choice(conv, &id(1), "method", |v| {
        ((v - 1).clamp(0, 2) as u32, None)
    });
    // 1 Speed, 2 Source Frame.
    fx.choice(conv, &id(2), "adjust", |v| (u32::from(v == 2), None));
    fx.float(conv, &id(3), "speed", 1.0, 0.0);
    fx.float(conv, &id(4), "source_frame", 1.0, 0.0);
    if let Some(detail) = fx.still(conv, &id(6)) {
        let quality = if detail < 15.0 {
            0
        } else if detail <= 40.0 {
            1
        } else {
            2
        };
        fx.set("quality", EffectValue::Choice(quality));
        if (detail - 20.0).abs() > 1e-6 {
            fx.approx_named(conv, "Vector Detail", "Quality");
        }
    }
    if let Some(smooth) = fx.still(conv, &id(8)) {
        fx.set(
            "smoothness",
            EffectValue::Float(Property::fixed((smooth / 10.0).clamp(0.0, 100.0))),
        );
        if (smooth - 500.0).abs() > 1e-6 {
            fx.approx_named(conv, "Global Smoothness", "Smoothness");
        }
    }
    unless(fx, conv, 9, "Local Smoothness", 500.0);
    unless(fx, conv, 10, "Smoothing Iterations", 5.0);
    unless(fx, conv, 12, "Build From One Image", 0.0);
    unless(fx, conv, 13, "Correct Luminance Changes", 0.0);
    unless(fx, conv, 14, "Filtering", 1.0);
    unless(fx, conv, 15, "Error Threshold", 1.0);
    unless(fx, conv, 16, "Block Size", 6.0);
    fx.toggle(conv, &id(24), "motion_blur");
    // 1 Automatic follows the composition's shutter, which Lumit's Timewarp
    // does not read; 2 Manual is the effect's own.
    match fx.still(conv, &id(25)).map(|v| v.round() as i64) {
        Some(1) => fx.approx_named(conv, "Shutter Control", "a shutter of its own"),
        _ => {
            fx.float(conv, &id(26), "shutter_angle", 1.0, 0.0);
            if let Some(samples) = fx.still(conv, &id(27)) {
                fx.set(
                    "samples",
                    EffectValue::Float(Property::fixed(samples.clamp(2.0, 64.0))),
                );
            }
        }
    }
    unless(fx, conv, 29, "Matte Layer", 0.0);
    unless(fx, conv, 31, "Warp Layer", 0.0);
    unless(fx, conv, 32, "Show", 1.0);
    unless(fx, conv, 34, "Source Crops", 0.0);
    unless(fx, conv, 35, "Source Crops", 100.0);
    unless(fx, conv, 36, "Source Crops", 0.0);
    unless(fx, conv, 37, "Source Crops", 100.0);
}
