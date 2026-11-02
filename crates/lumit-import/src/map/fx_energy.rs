//! The plug-in's light-sabre effect → **Energy line**.
//!
//! Every control is found by the plug-in's own name for it, which a project
//! can't rename. The core and the glow each have a distortion group with the
//! same names in it, so a name can be asked for a second time. The ranges and
//! defaults below are the plug-in's own (version 1.0.39). The two effects
//! measure their lights differently, so a size or strength is scaled to put
//! the plug-in's default on Energy line's, and a number in the same units is
//! carried as it is.

use lumit_core::model::{EffectInstance, EffectValue};

use crate::capture::Property;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::table::Row;
use super::Conv;

/// The `nth` leaf (from 0) the plug-in calls `name`, in the order it lists
/// them, and its match name.
fn named(node: &Property, name: &str, nth: usize) -> Option<String> {
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
        .filter(|p| p.name.as_deref() == Some(name))
        .nth(nth)
        .and_then(|p| p.match_name.clone())
}

pub(super) fn energy_line(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
    row: &'static Row,
) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, row)?;
    let id = |name: &str, nth: usize| named(node, name, nth).unwrap_or_default();
    // Core Type: 1 between two points, 2 the layer's masks, 3 a Text layer.
    fx.choice(conv, &id("Core Type", 0), "source", |v| match v {
        2 => (1, None),
        3 => (
            3,
            Some("Text, which draws the letters of the layer the effect is on"),
        ),
        _ => (0, None),
    });
    // Its masks and letters light together.
    if fx
        .still(conv, &id("Core Type", 0))
        .is_some_and(|v| v.round() as i64 > 1)
    {
        fx.set("paths", EffectValue::Choice(1));
    }
    fx.point(conv, &id("Core Start", 0), "start_x", "start_y", 1.0);
    fx.point(conv, &id("Core End", 0), "end_x", "end_y", 1.0);
    // Core Size 3 is Energy line's 10 px core.
    fx.float(conv, &id("Core Size", 0), "core_size", 10.0 / 3.0, 0.0);
    // 0 to 25 onto Energy line's 40 to 100 per cent: its softest core is
    // already soft.
    fx.float(conv, &id("Core Softness", 0), "core_softness", 2.4, 40.0);
    // Its 100 per cent default is Energy line's 60.
    fx.float(conv, &id("Halo Intensity", 0), "halo", 0.6, 0.0);
    fx.float(conv, &id("Start Size", 0), "start_size", 1.0, 0.0);
    fx.float(conv, &id("End Size", 0), "end_size", 1.0, 0.0);
    fx.float(conv, &id("Start Offset", 0), "start", 1.0, 0.0);
    fx.float(conv, &id("End Offset", 0), "end", 1.0, 0.0);
    // A turn of Mask Evolution is once round the path.
    fx.float(conv, &id("Mask Evolution", 0), "offset", 100.0 / 360.0, 0.0);
    fx.colour(conv, &id("Glow Color", 0), "glow_colour");
    // Its defaults, 50 per cent, 0.5 and 0.5, are Energy line's 100 per cent,
    // 60 px and 50 per cent.
    fx.float(conv, &id("Glow Intensity", 0), "glow_intensity", 2.0, 0.0);
    fx.float(conv, &id("Glow Spread", 0), "glow_size", 120.0, 0.0);
    fx.float(conv, &id("Glow Bias", 0), "glow_falloff", 100.0, 0.0);
    // Enable Glow off is no glow at all.
    if let Some(on) = fx.still(conv, &id("Enable Glow", 0)) {
        if on.abs() < f64::EPSILON {
            fx.set(
                "glow_intensity",
                EffectValue::Float(lumit_core::anim::Property::zero()),
            );
        }
    }
    fx.float(conv, &id("Flicker Intensity", 0), "flicker", 1.0, 0.0);
    fx.float(conv, &id("Flicker Speed", 0), "flicker_speed", 1.0, 0.0);
    fx.seed(conv, &id("Random Seed", 0), "seed");
    // Smoke, Fluid and Energy: smoke is Smoke, a fluid is the smooth swell,
    // and energy is the crackle.
    fx.choice(
        conv,
        &id("Distortion Type", 0),
        "distortion_type",
        |v| match v {
            1 => (1, None),
            3 => (2, None),
            _ => (0, None),
        },
    );
    fx.float(
        conv,
        &id("Distortion Amount", 0),
        "glow_distortion",
        1.0,
        0.0,
    );
    fx.float(
        conv,
        &id("Distortion Amount", 1),
        "core_distortion",
        1.0,
        0.0,
    );
    fx.float(conv, &id("Wind Speed", 0), "wind_speed", 100.0, 0.0);
    fx.float(
        conv,
        &id("Wind Direction Offset", 0),
        "wind_direction",
        1.0,
        0.0,
    );
    fx.float(conv, &id("Noise Speed", 0), "distortion_speed", 1.0, 0.0);
    fx.float(conv, &id("Noise Scale", 0), "distortion_size", 1.0, 0.0);
    fx.float(
        conv,
        &id("Noise Complexity", 0),
        "distortion_detail",
        1.0,
        0.0,
    );
    // Wind blowing backwards is wind the other way.
    if let Some(speed) = fx.still(conv, &id("Wind Speed", 0)) {
        if speed < 0.0 {
            let turn = fx
                .still(conv, &id("Wind Direction Offset", 0))
                .unwrap_or(0.0);
            let fixed = |v: f64| EffectValue::Float(lumit_core::anim::Property::fixed(v));
            fx.set("wind_speed", fixed(-speed * 100.0));
            fx.set("wind_direction", fixed(turn + 180.0));
        }
    }
    // Composite Settings: 1 Transparent, 2 Black (its default), 3 Add.
    if let Some(v) = fx.still(conv, &id("Composite Settings", 0)) {
        let v = v.round() as i64;
        fx.set("composite_on_original", EffectValue::Bool(v == 3));
        if v == 2 {
            fx.approx_named(
                conv,
                "Composite Settings",
                "the light on its own over the layers below, rather than over black",
            );
        }
    }
    fx.differs(
        conv,
        "its sizes and strengths are measured differently, so they are scaled to put its \
         defaults on Energy line's",
    );
    fx.drop_params(
        conv,
        &[
            "Start Roundness",
            "End Roundness",
            "Offset Size",
            "Halo Size",
            "Mask Randomization",
            "Noise Bias",
            "Noise Aspect Ratio",
            "Lock Noise to Saber",
            "the Glow Settings group",
            "the Render Settings group's Motion Blur, Gamma, Brightness, Saturation and Alpha Boost",
            "Alpha Mode",
            "Invert Masks",
            "Use Text Alpha",
        ],
    );
    fx.done()
}
