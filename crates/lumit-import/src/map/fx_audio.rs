//! Audio Spectrum and Audio Waveform → **Audio spectrum**.
//!
//! Every control is found by the effect's own name for it, which a project can't
//! rename. Both effects' Audio Layer picks a layer of the comp, which comes across
//! as Source: Layer.

use lumit_core::fx::drivers::audio_level::SOURCE_LAYER;
use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::table::Row;
use super::Conv;

/// The leaf the effect calls `name`, and its match name.
fn named(node: &Property, name: &str) -> String {
    fn walk<'a>(props: &'a [Property], out: &mut Vec<&'a Property>) {
        for p in props {
            if p.group.is_some() {
                walk(p.children(), out);
            } else {
                out.push(p);
            }
        }
    }
    let mut leaves = Vec::new();
    walk(node.children(), &mut leaves);
    leaves
        .into_iter()
        .find(|p| p.name.as_deref() == Some(name))
        .and_then(|p| p.match_name.clone())
        .unwrap_or_default()
}

/// The controls the two effects share.
fn shared(conv: &mut Conv<'_>, fx: &mut Fx<'_>, node: &Property) {
    let id = |name: &str| named(node, name);
    fx.set("source", EffectValue::Choice(SOURCE_LAYER));
    fx.point(conv, &id("Start Point"), "from_x", "from_y", 1.0);
    fx.point(conv, &id("End Point"), "to_x", "to_y", 1.0);
    // A path chosen is what the bands are laid along.
    if fx.mask(conv, &id("Path"), "path").is_some_and(|v| v > 0.0) {
        fx.set(
            "along",
            EffectValue::Choice(lumit_core::fx::effects::audio_spectrum::ALONG_MASK),
        );
    }
    fx.float(conv, &id("Maximum Height"), "height", 1.0, 0.0);
    fx.float(conv, &id("Thickness"), "thickness", 1.0, 0.1);
    fx.float(conv, &id("Softness"), "softness", 1.0, 0.0);
    fx.colour(conv, &id("Inside Color"), "inside_colour");
    fx.colour(conv, &id("Outside Color"), "outside_colour");
    fx.toggle(conv, &id("Composite On Original"), "composite_on_original");
    fx.drop_params(
        conv,
        &[
            "Audio Layer",
            "Audio Duration (milliseconds)",
            "Audio Offset (milliseconds)",
            "Random Seed (Multi-Processor)",
        ],
    );
}

pub(super) fn audio_spectrum(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    row: &'static Row,
) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, row)?;
    let id = |name: &str| named(node, name);
    shared(conv, &mut fx, node);
    // Its Polar Path is a circle round the Start Point, and Ring is a circle
    // between From and To.
    if fx
        .still(conv, &id("Use Polar Path"))
        .is_some_and(|v| v.abs() > f64::EPSILON)
    {
        fx.set(
            "shape",
            EffectValue::Choice(lumit_core::fx::effects::audio_spectrum::SHAPE_RING),
        );
        fx.approx_named(
            conv,
            "Use Polar Path",
            "a Ring between its Start and End points",
        );
    }
    fx.float(conv, &id("Start Frequency"), "start_frequency", 1.0, 1.0);
    fx.float(conv, &id("End Frequency"), "end_frequency", 1.0, 1.0);
    fx.float(conv, &id("Frequency bands"), "bands", 1.0, 2.0);
    fx.float(conv, &id("Hue Interpolation"), "hue_spread", 1.0, 0.0);
    // Display Options: 1 Digital, 2 Analog lines, 3 Analog dots.
    fx.choice(conv, &id("Display Options"), "shape", |v| match v {
        2 | 3 => (lumit_core::fx::effects::audio_spectrum::SHAPE_LINE, None),
        _ => (lumit_core::fx::effects::audio_spectrum::SHAPE_BARS, None),
    });
    side_options(conv, &mut fx, &id("Side Options"));
    fx.float(conv, &id("Duration Averaging"), "smoothing", 1.0, 0.0);
    fx.drop_params(
        conv,
        &[
            "Blend Overlapping Colors",
            "Dynamic Hue Phase",
            "Color Symmetry",
        ],
    );
    fx.done()
}

pub(super) fn audio_waveform(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    row: &'static Row,
) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, row)?;
    let id = |name: &str| named(node, name);
    fx.set(
        "shape",
        EffectValue::Choice(lumit_core::fx::effects::audio_spectrum::SHAPE_WAVEFORM),
    );
    shared(conv, &mut fx, node);
    fx.float(conv, &id("Displayed Samples"), "bands", 1.0, 2.0);
    side_options(conv, &mut fx, &id("Waveform Options"));
    fx.drop_params(conv, &["Display Options"]);
    fx.done()
}

/// Which way a band reaches: 1 one side, 2 the other, 3 both, which is Mirror.
fn side_options(conv: &mut Conv<'_>, fx: &mut Fx<'_>, id: &str) {
    if let Some(v) = fx.still(conv, id) {
        fx.set("mirror", EffectValue::Bool(v.round() as i64 == 3));
    }
}
