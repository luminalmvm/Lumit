//! Video Copilot's Twitch onto Jolt. Its match names could not be checked
//! against an installed copy, so its controls are found by the names they
//! show rather than by number.

use lumit_core::model::EffectInstance;

use crate::capture::Property as AeProperty;
use crate::report::ItemPath;

use super::fx_colour::Fx;
use super::props::match_name_of;
use super::Conv;

/// This file's claim on a match name: the row whose conversion is `twitch`.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &AeProperty,
) -> Option<EffectInstance> {
    let row = super::table::table().row(match_name_of(node))?;
    if row.conversion != "twitch" {
        return None;
    }
    let before = conv.report.rows.len();
    let mut fx = Fx::new(path, node, row)?;
    twitch(conv, node, &mut fx);
    if conv.report.rows.len() == before {
        conv.report.imported();
    }
    fx.done()
}

/// Every leaf under `props`, with the groups opened.
fn leaves<'a>(props: &'a [AeProperty], out: &mut Vec<&'a AeProperty>) {
    for p in props {
        match &p.group {
            Some(kids) => leaves(kids, out),
            None => out.push(p),
        }
    }
}

/// Twitch's numbers onto Jolt's rows, by the name Twitch shows: the name, the
/// row, then `value × k + c`.
const CARRIED: [(&str, &str, f64, f64); 9] = [
    ("Amount", "amount", 1.0, 0.0),
    ("Speed", "rate", 1.0, 0.0),
    ("Blur Amount", "blur_amount", 1.0, 0.0),
    ("Color Amount", "colour_amount", 1.0, 0.0),
    // A per cent there, stops here.
    ("Light Amount", "light_amount", 0.01, 0.0),
    ("Scale Amount", "scale_amount", 1.0, 0.0),
    ("Slide Amount", "slide_amount", 1.0, 0.0),
    // After Effects measures an angle from straight up, Lumit from the right.
    ("Slide Direction", "slide_direction", 1.0, -90.0),
    ("Time Amount", "time_amount", 1.0, 0.0),
];

/// Twitch's checkboxes onto Jolt's switches.
const SWITCHES: [(&str, &str); 6] = [
    ("Blur", "blur_on"),
    ("Color", "colour_on"),
    ("Light", "light_on"),
    ("Scale", "scale_on"),
    ("Slide", "slide_on"),
    ("Time", "time_on"),
];

/// Everything Jolt has a row for comes across. Anything else that is set is
/// reported as not carried.
fn twitch(conv: &mut Conv<'_>, node: &AeProperty, fx: &mut Fx<'_>) {
    let mut all = Vec::new();
    leaves(node.children(), &mut all);
    for leaf in all {
        let Some(id) = leaf.match_name.as_deref() else {
            continue;
        };
        let name = leaf.name.as_deref().unwrap_or(id);
        if let Some(&(_, to, k, c)) = CARRIED.iter().find(|r| r.0 == name) {
            fx.float(conv, id, to, k, c);
            if to == "light_amount" {
                fx.approx_named(conv, name, "one stop for every 100");
            }
        } else if let Some(&(_, to)) = SWITCHES.iter().find(|r| r.0 == name) {
            fx.toggle(conv, id, to);
        } else if name == "Colorize" {
            fx.colour(conv, id, "colour");
        } else if name == "Random Seed" {
            fx.seed(conv, id, "seed");
        } else if name == "Light Behaviour" {
            // 1 Brighter, 2 Darker, 3 Both, as Jolt's Direction lists them.
            fx.choice(conv, id, "light_direction", |v| {
                ((v - 1).clamp(0, 2) as u32, None)
            });
        } else if name == "Time Direction" {
            // 1 Both, 2 Forward Only, 3 Backward Only.
            fx.choice(conv, id, "time_direction", |v| {
                ((v - 1).clamp(0, 2) as u32, None)
            });
        } else if fx.still(conv, id).is_some_and(|v| v != 0.0) {
            fx.drop_param(conv, name);
        }
    }
}
