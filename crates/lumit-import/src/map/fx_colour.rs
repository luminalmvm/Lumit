//! The effect mapping table, colour / blur / generate / temporal half
//! ([docs/11-AE-IMPORT.md](../../../../docs/11-AE-IMPORT.md) §5).
//!
//! # In plain terms
//!
//! An After Effects effect and its Lumit counterpart do the same job with
//! controls that are *nearly* the same and never quite. This file is the list
//! of the differences, written down once per effect: which AE dial becomes
//! which Lumit dial, what arithmetic turns one number into the other, and —
//! the part that matters most — what could not come across at all, so the
//! import report can say so rather than the picture quietly changing.
//!
//! Four rules run through the whole of it.
//!
//! **A dial is converted, not guessed.** Where the two sides measure in
//! different units the conversion is arithmetic on every value *and on every
//! keyframe*: a glow threshold that After Effects measured as a display value
//! becomes scene-linear light, including the number on each key and the speed
//! of each handle. A blur radius needs no arithmetic — After Effects' pixels
//! are Lumit's px@comp (docs/08 §2.3, K-419). Where the
//! conversion is not a straight line — a colour crossing from display space
//! into scene-linear light — the values are still exact and the *handles* are
//! not, and that is a report row of its own.
//!
//! **What has no counterpart is reported, never approximated into something
//! that looks similar.** docs/11 §5 says this effect by effect and this file
//! obeys it literally: an After Effects control Lumit does not have leaves a
//! row naming it, and the effect still imports.
//!
//! **An option list maps by position, and the position is pinned.** After
//! Effects stores a dropdown as a number, so a wrong order is a silently wrong
//! picture. Each list below is anchored on the value `tools/ae-audit/
//! ae-audit-report.json` recorded as that effect's *default*, which is the one
//! index a live After Effects has confirmed.
//!
//! **Where a base is undocumented, the defaults are the anchor.** A handful of
//! After Effects controls are per cents of a private base — Fractal Noise's
//! Scale is the standing example — and docs/11 says only that the import
//! "converts through AE's own base". Where that base is not published, this
//! file picks the factor that lands After Effects' own default on Lumit's
//! declared default for the same control, which is the one point at which both
//! specifications claim the two effects look alike.

use lumit_core::anim::{Animation, Keyframe, Property as LumProperty, SideInterp};
use lumit_core::model::{EffectInstance, EffectValue};
use uuid::Uuid;

use crate::capture::Property;
use crate::report::{ItemPath, Outcome, Reason};

use super::props::{display_name, find, from_node, match_name_of, ramp};
use super::{srgb_to_linear, Conv};

/// This half of the table's claim on a match name.
///
/// `Some` when the effect maps; `None` sends the instance down the placeholder
/// road, which is what an unrecognised match name — and the three rows docs/11
/// §5 places there deliberately — are meant to do.
pub(super) fn claim(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let before = conv.report.rows.len();
    let mapped = build(conv, path, node)?;
    // An effect that came across without a single row is one that came across
    // whole, which is what the summary line's first number counts.
    if conv.report.rows.len() == before {
        conv.report.imported();
    }
    Some(mapped)
}

#[allow(clippy::too_many_lines)]
fn build(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    match match_name_of(node) {
        // --- Blur and sharpen -------------------------------------------
        "ADBE Gaussian Blur 2" => gaussian_blur(conv, path, node),
        "ADBE Motion Blur" => directional_blur(conv, path, node),
        "ADBE Radial Blur" => radial_blur(conv, path, node),
        "ADBE Glo2" => glow(conv, path, node),

        // --- Colour ------------------------------------------------------
        "ADBE Easy Levels2" => levels(conv, path, node),
        "ADBE HUE SATURATION" => hue_saturation(conv, path, node),
        "ADBE Brightness & Contrast 2" => brightness(conv, path, node),
        "ADBE Tint" => tint(conv, path, node),
        "ADBE Photo Filter" => photo_filter(conv, path, node),
        "ADBE Black&White" => black_and_white(conv, path, node),
        "ADBE ShadowHighlight" => shadow_highlight(conv, path, node),
        "ADBE Tritone" => tritone(conv, path, node),
        "ADBE Posterize" => posterize(conv, path, node),
        "ADBE Threshold" => threshold(conv, path, node),
        "ADBE Broadcast Colors" => broadcast_safe(conv, path, node),

        // --- Generate ----------------------------------------------------
        "ADBE Fill" => fill(conv, path, node),
        "ADBE Ramp" => gradient(conv, path, node),
        "ADBE Noise" => noise(conv, path, node),
        "ADBE Fractal Noise" => fractal_noise(conv, path, node),
        "ADBE Laser" => beam(conv, path, node),
        "ADBE Lightning 2" => lightning(conv, path, node),
        "APC Radio Waves" => radio_waves(conv, path, node),
        "APC Vegas" => vegas(conv, path, node),
        "VISINF Grain Implant" => add_grain(conv, path, node),
        "ADBE Scribble Fill" => scribble(conv, path, node),
        "ADBE Stroke" => stroke(conv, path, node),

        // --- Temporal ----------------------------------------------------
        "ADBE Echo" => echo(conv, path, node),
        "ADBE Posterize Time" => posterize_time(conv, path, node),

        // --- Deliberate placeholders (docs/11 §5) ------------------------
        //
        // Curves is not here: its point list is the one property After Effects'
        // own scripting cannot read (K-410), so the instance takes the ordinary
        // placeholder road and `PropertyUnreadable` says why.
        "VISINF Grain Removal" => {
            suggest(
                conv,
                path,
                node,
                "removing grain is a programme of its own rather than an effect port",
            );
            None
        }
        "ADBE Timewarp" => {
            suggest(
                conv,
                path,
                node,
                "Retime with flow frame interpolation does the same job",
            );
            None
        }
        _ => None,
    }
}

/// A row on a match name the table places at a placeholder on purpose, naming
/// what does the job instead (docs/11 §5's two "placeholder + report" rows).
fn suggest(conv: &mut Conv<'_>, path: &ItemPath, node: &Property, instead: &str) {
    let match_name = match_name_of(node).to_string();
    conv.report.row(
        path.property(display_name(node, &match_name)),
        Outcome::Placeholder,
        Reason::EffectSuggestion {
            match_name,
            instead: instead.to_string(),
        },
    );
}

// ---------------------------------------------------------------------------
// The builder
// ---------------------------------------------------------------------------

/// One instance under construction: the capture node on one side, a Lumit
/// instance carrying its schema defaults on the other, and a report to write
/// the difference into.
struct Fx<'a> {
    /// After Effects' name for the effect, as the report row says it.
    ae: &'static str,
    node: &'a Property,
    /// Where a row about this effect is filed.
    here: ItemPath,
    /// Where a row about one of its parameters is filed — the layer, because
    /// [`from_node`] narrows it to the leaf's own name itself.
    at: ItemPath,
    inst: EffectInstance,
}

impl<'a> Fx<'a> {
    /// A fresh Lumit instance of `lumit`, carrying every declared default, or
    /// `None` when this build does not ship that effect (which cannot happen
    /// for the names in [`build`], and is a placeholder rather than a panic if
    /// it ever does).
    fn new(path: &ItemPath, node: &'a Property, lumit: &str, ae: &'static str) -> Option<Fx<'a>> {
        let mut inst = lumit_core::fx::instantiate(lumit)?;
        // An effect switched off in After Effects imports switched off.
        inst.enabled = node.enabled.unwrap_or(true);
        Some(Fx {
            ae,
            node,
            here: path.property(display_name(node, match_name_of(node))),
            at: path.clone(),
            inst,
        })
    }

    /// One of the effect's parameter leaves. After Effects presents an
    /// effect's parameters flat, with its groups as marker nodes among them,
    /// so a direct search of the children finds everything; [`find`]'s deeper
    /// walk covers a future Bridge that nests them.
    fn leaf(&self, ae_id: &str) -> Option<&'a Property> {
        find(self.node.children(), ae_id)
    }

    /// A parameter's still value, for the switches and option lists that do
    /// not animate in Lumit.
    fn still(&self, ae_id: &str) -> Option<f64> {
        let node = self.leaf(ae_id)?;
        super::props::axis_of(node.value.as_ref()?, 0)
    }

    fn set(&mut self, lumit_id: &str, value: EffectValue) {
        if let Some(p) = self.inst.params.iter_mut().find(|p| p.id == lumit_id) {
            p.value = value;
        }
    }

    /// One AE float onto one Lumit float, through `value × k + c` — applied to
    /// the still value, to every keyframe's value, and to every bezier
    /// handle's speed, which is in value-units a second and so scales with the
    /// value (K-025).
    fn float(&mut self, conv: &mut Conv<'_>, ae_id: &str, lumit_id: &str, k: f64, c: f64) {
        self.float_axis(conv, ae_id, 0, lumit_id, k, c);
    }

    /// The same, from one axis of a multi-dimensional AE parameter — a point's
    /// x or y.
    fn float_axis(
        &mut self,
        conv: &mut Conv<'_>,
        ae_id: &str,
        axis: usize,
        lumit_id: &str,
        k: f64,
        c: f64,
    ) {
        let Some(node) = self.leaf(ae_id) else {
            return;
        };
        let p = from_node(conv, &self.at, node, axis, 0.0);
        if (k - 1.0).abs() > f64::EPSILON || c.abs() > f64::EPSILON {
            // An expression computes After Effects' number in After Effects'
            // units, and rewriting somebody's JavaScript is not this stage's
            // job (docs/11 §2.2 item 8 — never rewritten).
            if matches!(p.animation, Animation::Expression(_)) {
                let param = display_name(node, ae_id).to_string();
                self.approx_named(conv, &param, "an expression still in After Effects' units");
            }
        }
        self.set(lumit_id, EffectValue::Float(affine(p, k, c)));
    }

    /// An AE point onto Lumit's adjacent `_x`/`_y` pair (docs/08 §1.1 — a
    /// point is two floats and a naming convention).
    fn point(&mut self, conv: &mut Conv<'_>, ae_id: &str, x: &str, y: &str, k: f64) {
        self.float_axis(conv, ae_id, 0, x, k, 0.0);
        self.float_axis(conv, ae_id, 1, y, k, 0.0);
    }

    /// An AE colour onto a Lumit colour. The three light channels cross from
    /// the project's display space into scene-linear (K-026) and the alpha
    /// lane passes through, exactly as a solid's colour does.
    fn colour(&mut self, conv: &mut Conv<'_>, ae_id: &str, lumit_id: &str) {
        let Some(node) = self.leaf(ae_id) else {
            return;
        };
        let at = self.at.clone();
        let mut handles_left_behind = false;
        let mut lane = |conv: &mut Conv<'_>, axis: usize, linearise: bool| {
            let p = from_node(conv, &at, node, axis, f64::from(u8::from(axis == 3)));
            if !linearise {
                return p;
            }
            let (p, adjusted) = map_values(p, |v| f64::from(srgb_to_linear(v)));
            handles_left_behind |= adjusted;
            p
        };
        let value = EffectValue::Colour([
            lane(conv, 0, true),
            lane(conv, 1, true),
            lane(conv, 2, true),
            lane(conv, 3, false),
        ]);
        if handles_left_behind {
            let param = display_name(node, ae_id).to_string();
            self.approx_named(
                conv,
                &param,
                "scene-linear light, with its keyframe handles still shaped for display values",
            );
        }
        self.set(lumit_id, value);
    }

    /// An AE checkbox — stored as a number — onto a Lumit switch.
    fn toggle(&mut self, ae_id: &str, lumit_id: &str) {
        if let Some(v) = self.still(ae_id) {
            self.set(lumit_id, EffectValue::Bool(v.abs() > f64::EPSILON));
        }
    }

    /// An AE dropdown onto a Lumit one, by index. `f` returns the Lumit index
    /// and whether the entry is the same entry rather than the nearest.
    fn choice(
        &mut self,
        conv: &mut Conv<'_>,
        ae_id: &str,
        lumit_id: &str,
        f: impl Fn(i64) -> (u32, Option<&'static str>),
    ) {
        let Some(v) = self.still(ae_id) else {
            return;
        };
        let (index, approximated) = f(v.round() as i64);
        self.set(lumit_id, EffectValue::Choice(index));
        if let Some(as_) = approximated {
            let param = self
                .leaf(ae_id)
                .map_or_else(|| ae_id.to_string(), |n| display_name(n, ae_id).to_string());
            self.approx_named(conv, &param, as_);
        }
    }

    /// An AE Random Seed onto a Lumit Seed.
    fn seed(&mut self, ae_id: &str, lumit_id: &str) {
        if let Some(v) = self.still(ae_id) {
            let v = if v.is_finite() {
                v.clamp(0.0, f64::from(u32::MAX))
            } else {
                0.0
            };
            self.set(lumit_id, EffectValue::Seed(v as u32));
        }
    }

    /// An AE mask reference onto the K-408 mask-path row. Unset is Lumit's
    /// "First mask" entry, which is what an unset AE reference means too;
    /// an index naming a mask the import did not bring over falls back to it
    /// and says so.
    fn mask(&mut self, conv: &mut Conv<'_>, ae_id: &str, lumit_id: &str) -> Option<f64> {
        let index = self.still(ae_id).unwrap_or(0.0).round() as i64;
        let found = usize::try_from(index - 1)
            .ok()
            .and_then(|i| conv.masks.get(i).copied());
        if index > 0 && found.is_none() {
            let param = self
                .leaf(ae_id)
                .map_or_else(|| ae_id.to_string(), |n| display_name(n, ae_id).to_string());
            self.approx_named(conv, &param, "the layer's first mask");
        }
        self.set(lumit_id, EffectValue::MaskPath(found.map(|(id, _)| id)));
        // The perimeter of whichever mask the row ends up on — Vegas turns a
        // count of segments into a length with it.
        found
            .map(|(_, perimeter)| perimeter)
            .or_else(|| conv.masks.first().map(|(_, p)| *p))
    }

    // --- report rows ----------------------------------------------------

    /// An After Effects control with no Lumit counterpart.
    fn drop_param(&mut self, conv: &mut Conv<'_>, param: &str) {
        conv.report.row(
            self.here.clone(),
            Outcome::Adjusted,
            Reason::EffectParamNotCarried {
                effect: self.ae.to_string(),
                param: param.to_string(),
            },
        );
    }

    /// Several of them at once — an After Effects group Lumit has nothing for.
    fn drop_params(&mut self, conv: &mut Conv<'_>, params: &[&str]) {
        for param in params {
            self.drop_param(conv, param);
        }
    }

    fn approx_named(&mut self, conv: &mut Conv<'_>, param: &str, imported_as: &str) {
        conv.report.row(
            self.here.clone(),
            Outcome::Adjusted,
            Reason::EffectParamApproximated {
                effect: self.ae.to_string(),
                param: param.to_string(),
                imported_as: imported_as.to_string(),
            },
        );
    }

    /// The same number in the other side's units (docs/08 §2.3). Nothing was
    /// lost; the figure on the dial is not the figure After Effects showed.
    fn rebased(&mut self, conv: &mut Conv<'_>, param: &str) {
        conv.report.row(
            self.here.clone(),
            Outcome::Adjusted,
            Reason::EffectParamRebased {
                effect: self.ae.to_string(),
                param: param.to_string(),
            },
        );
    }

    /// The effect mapped whole and evaluates differently by construction.
    fn differs(&mut self, conv: &mut Conv<'_>, detail: &str) {
        conv.report.row(
            self.here.clone(),
            Outcome::Adjusted,
            Reason::EffectDiffers {
                effect: self.ae.to_string(),
                detail: detail.to_string(),
            },
        );
    }

    /// An After Effects control that read the clock, become keyframes.
    fn clock(&mut self, conv: &mut Conv<'_>, param: &str) {
        conv.report.row(
            self.here.clone(),
            Outcome::Adjusted,
            Reason::EffectSpeedAsKeyframes {
                effect: self.ae.to_string(),
                param: param.to_string(),
            },
        );
    }

    fn done(self) -> Option<EffectInstance> {
        Some(self.inst)
    }
}

/// `value × k + c` on the still value, on every key, and on every handle's
/// speed.
fn affine(p: LumProperty, k: f64, c: f64) -> LumProperty {
    if (k - 1.0).abs() < f64::EPSILON && c.abs() < f64::EPSILON {
        return p;
    }
    let side = |s: SideInterp| match s {
        SideInterp::Bezier { speed, influence } => SideInterp::Bezier {
            speed: speed * k,
            influence,
        },
        other => other,
    };
    LumProperty {
        animation: match p.animation {
            Animation::Static(v) => Animation::Static(v * k + c),
            Animation::Keyframed(keys) => Animation::Keyframed(
                keys.into_iter()
                    .map(|key| Keyframe {
                        time: key.time,
                        value: key.value * k + c,
                        interp_in: side(key.interp_in),
                        interp_out: side(key.interp_out),
                    })
                    .collect(),
            ),
            other => other,
        },
        extra: p.extra,
    }
}

/// A conversion that is not a straight line — a colour crossing into
/// scene-linear light. Every *value* is exact; a bezier handle's speed is in
/// the old units and cannot be rescaled by one factor, so the second return
/// says whether one was left behind and the caller reports it.
fn map_values(p: LumProperty, f: impl Fn(f64) -> f64) -> (LumProperty, bool) {
    let mut left_behind = false;
    let animation = match p.animation {
        Animation::Static(v) => Animation::Static(f(v)),
        Animation::Keyframed(keys) => Animation::Keyframed(
            keys.into_iter()
                .map(|key| {
                    for side in [key.interp_in, key.interp_out] {
                        if matches!(side, SideInterp::Bezier { speed, .. } if speed.abs() > 1e-9) {
                            left_behind = true;
                        }
                    }
                    Keyframe {
                        value: f(key.value),
                        ..key
                    }
                })
                .collect(),
        ),
        other => other,
    };
    (
        LumProperty {
            animation,
            extra: p.extra,
        },
        left_behind,
    )
}

// ---------------------------------------------------------------------------
// Blur and sharpen
// ---------------------------------------------------------------------------

/// "Gaussian Blur" → **Gaussian blur** (docs/08 §3.8). One control carries the
/// look — AE's pixels are Lumit's px@comp, so the number is the same — and
/// both of the others are switches Lumit's Gaussian does not have: it always
/// blurs on both axes, and its edge policy is the fixed Repeat that K-137
/// settled on.
fn gaussian_blur(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "blur", "Gaussian Blur")?;
    fx.float(conv, "ADBE Gaussian Blur 2-0001", "radius", 1.0, 0.0);
    fx.drop_params(conv, &["Blur Dimensions", "Repeat Edge Pixels"]);
    fx.done()
}

/// "Directional Blur" → **Directional blur** (docs/08 §3.9). Both controls
/// carry unchanged: the angle is degrees from straight up clockwise on both
/// sides, and the length is pixels on both (px@comp in Lumit).
fn directional_blur(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "directional_blur", "Directional Blur")?;
    fx.float(conv, "ADBE Motion Blur-0001", "angle", 1.0, 0.0);
    fx.float(conv, "ADBE Motion Blur-0002", "length", 1.0, 0.0);
    fx.done()
}

/// "Radial Blur" → **Radial blur** (docs/08 §3.10). Amount carries as pixels
/// and the centre is a point on both sides since K-558 — AE's layer pixels are
/// Lumit's px@comp, so the two numbers copy across — while AE's antialiasing
/// and seed have no counterpart.
fn radial_blur(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "radial_blur", "Radial Blur")?;
    fx.float(conv, "ADBE Radial Blur-0001", "amount", 1.0, 0.0);
    fx.point(conv, "ADBE Radial Blur-0002", "centre_x", "centre_y", 1.0);
    // 1 Spin, 2 Zoom — AE's default of 1 is Spin, which is Lumit's 0.
    fx.choice(conv, "ADBE Radial Blur-0003", "radial_type", |v| {
        (u32::from(v == 2), None)
    });
    fx.drop_params(conv, &["Antialiasing (Best Quality)", "Random Seed"]);
    fx.done()
}

/// "Glow" → **Glow** (docs/08 §3.28). Lumit's glow is exposure-aware: it is
/// four numbers (a threshold in light, a softness, a radius and an intensity)
/// where AE's is fourteen, most of them describing a colour ramp applied to
/// the halo. Threshold, Radius and Intensity carry; the rest is reported.
fn glow(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "glow", "Glow")?;
    // AE's Glow Threshold is an eight-bit display value (its 60% default
    // arrives as 153); Lumit's is scene-linear light.
    if let Some(leaf) = fx.leaf("ADBE Glo2-0002") {
        let p = from_node(conv, &fx.at.clone(), leaf, 0, 153.0);
        let (p, handles) = map_values(p, |v| f64::from(srgb_to_linear(v / 255.0)));
        fx.set("threshold", EffectValue::Float(p));
        if handles {
            fx.approx_named(
                conv,
                "Glow Threshold",
                "scene-linear light, with its keyframe handles still shaped for display values",
            );
        }
    }
    fx.float(conv, "ADBE Glo2-0003", "radius", 1.0, 0.0);
    fx.float(conv, "ADBE Glo2-0004", "intensity", 1.0, 0.0);
    fx.differs(
        conv,
        "its halo is built from scene-linear light rather than from an eight-bit copy of the \
         picture, so the output is brighter and cleaner than After Effects'",
    );
    fx.drop_params(
        conv,
        &[
            "Glow Based On",
            "Glow Operation",
            "Glow Dimensions",
            "Composite Original",
            "the Glow Colors group",
        ],
    );
    fx.done()
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// The channel group an AE picker selects, as Lumit's parameter prefix.
/// 1 RGB, 2 Red, 3 Green, 4 Blue, 5 Alpha — After Effects' own channel order,
/// anchored on the audit's default of 1 (the effect opens on RGB).
fn channel_group(v: i64) -> Option<&'static str> {
    match v {
        1 => Some("master"),
        2 => Some("red"),
        3 => Some("green"),
        4 => Some("blue"),
        _ => None,
    }
}

/// "Levels" → **Levels** (docs/08 §3.31). The scripting DOM exposes one set of
/// five numbers plus the Channel picker that says which channel they belong
/// to, so the import writes that channel's group and leaves the others neutral.
fn levels(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "levels", "Levels")?;
    let channel = fx.still("ADBE Easy Levels2-0001").unwrap_or(1.0).round() as i64;
    let Some(g) = channel_group(channel) else {
        fx.approx_named(
            conv,
            "Channel",
            "the master group, the alpha having no lane",
        );
        return fx.done();
    };
    for (ae, lumit) in [
        ("ADBE Easy Levels2-0003", "in_black"),
        ("ADBE Easy Levels2-0004", "in_white"),
        ("ADBE Easy Levels2-0005", "gamma"),
        ("ADBE Easy Levels2-0006", "out_black"),
        ("ADBE Easy Levels2-0007", "out_white"),
    ] {
        fx.float(conv, ae, &format!("{g}_{lumit}"), 1.0, 0.0);
    }
    fx.differs(
        conv,
        "a value above Input white carries on through the curve instead of being clipped, \
         because the working space is scene-linear",
    );
    fx.drop_params(conv, &["Clip To Output Black", "Clip To Output White"]);
    fx.done()
}

/// "Hue/Saturation" → **Hue and saturation** (docs/08 §3.33), on the same
/// shape as Levels: one set of three sliders and a picker naming the range
/// they belong to. A colourised instance is the one case that takes the
/// placeholder road, because Colorize discards the source hue and no Lumit
/// control does that.
fn hue_saturation(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "hue_saturation", "Hue/Saturation")?;
    if fx.still("ADBE HUE SATURATION-0007").unwrap_or(0.0).abs() > f64::EPSILON {
        conv.report.row(
            fx.here.clone(),
            Outcome::Placeholder,
            Reason::EffectParamNotCarried {
                effect: "Hue/Saturation".to_string(),
                param: "Colorize".to_string(),
            },
        );
        return None;
    }
    // 1 Master, then AE's six ranges in the order Lumit declares them.
    let g = match fx.still("ADBE HUE SATURATION-0002").unwrap_or(1.0).round() as i64 {
        2 => "reds",
        3 => "yellows",
        4 => "greens",
        5 => "cyans",
        6 => "blues",
        7 => "magentas",
        _ => "master",
    };
    fx.float(
        conv,
        "ADBE HUE SATURATION-0004",
        &format!("{g}_hue"),
        1.0,
        0.0,
    );
    fx.float(
        conv,
        "ADBE HUE SATURATION-0005",
        &format!("{g}_saturation"),
        1.0,
        0.0,
    );
    fx.float(
        conv,
        "ADBE HUE SATURATION-0006",
        &format!("{g}_lightness"),
        1.0,
        0.0,
    );
    fx.differs(
        conv,
        "each range's weight is scaled by the pixel's own saturation, so a grey takes the \
         master adjustment alone",
    );
    fx.drop_param(conv, "Channel Range");
    fx.done()
}

/// "Brightness & Contrast" → **Brightness** (docs/08 §3.32, K-397). One effect
/// carrying both sliders under AE's names and AE's neutral point, so both
/// numbers cross unchanged.
fn brightness(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "brightness", "Brightness & Contrast")?;
    fx.float(
        conv,
        "ADBE Brightness & Contrast 2-0001",
        "brightness",
        1.0,
        0.0,
    );
    fx.float(
        conv,
        "ADBE Brightness & Contrast 2-0002",
        "contrast",
        1.0,
        0.0,
    );
    fx.drop_param(conv, "Use Legacy (supports HDR)");
    fx.done()
}

/// "Tint" → **Tint** (docs/08 §3.23). Both colours and the amount carry; AE's
/// Amount to Tint is a per cent and so is Lumit's Mix.
fn tint(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "tint", "Tint")?;
    fx.colour(conv, "ADBE Tint-0001", "black");
    fx.colour(conv, "ADBE Tint-0002", "white");
    fx.float(conv, "ADBE Tint-0003", "mix", 1.0, 0.0);
    fx.done()
}

/// "Photo Filter" → **Photo filter** (docs/08 §3.61). Every control has a
/// counterpart under the same name; the twenty named filters are Lumit's own
/// chromaticities, Adobe's not being published, so it is a look-for-look
/// conversion.
fn photo_filter(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "photo_filter", "Photo Filter")?;
    // Twenty-one entries on both sides, in Adobe's order, ending on Custom;
    // AE's list is 1-based and Lumit's is 0-based.
    fx.choice(conv, "ADBE Photo Filter-0001", "filter", |v| {
        (u32::try_from((v - 1).clamp(0, 20)).unwrap_or(0), None)
    });
    fx.colour(conv, "ADBE Photo Filter-0002", "colour");
    fx.float(conv, "ADBE Photo Filter-0003", "density", 1.0, 0.0);
    fx.toggle("ADBE Photo Filter-0004", "preserve_luminosity");
    fx.differs(
        conv,
        "the twenty named filters are Lumit's own chromaticities under Adobe's names",
    );
    fx.done()
}

/// "Black & White" → **Black and white** (docs/08 §3.62). The six weights and
/// the tint carry; the tint colour is divided through by its own luma inside
/// the effect, so an imported dark tint tints rather than darkens.
fn black_and_white(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "black_and_white", "Black & White")?;
    for (i, lumit) in ["reds", "yellows", "greens", "cyans", "blues", "magentas"]
        .into_iter()
        .enumerate()
    {
        fx.float(
            conv,
            &format!("ADBE Black&White-{:04}", i + 1),
            lumit,
            1.0,
            0.0,
        );
    }
    fx.toggle("ADBE Black&White-0007", "tint");
    fx.colour(conv, "ADBE Black&White-0008", "tint_colour");
    fx.approx_named(
        conv,
        "Tint Color",
        "a hue divided through by its own luma, so it tints without changing the exposure",
    );
    fx.done()
}

/// "Shadow/Highlight" → **Shadow highlight** (docs/08 §3.63). The manual pair
/// and the tonal widths carry; AE's two radii average into Lumit's one, and
/// the controls whose answer at a frame depends on the shot around it are
/// reported rather than approximated.
fn shadow_highlight(
    conv: &mut Conv<'_>,
    path: &ItemPath,
    node: &Property,
) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "shadow_highlight", "Shadow/Highlight")?;
    fx.float(conv, "ADBE ShadowHighlight-0002", "shadow_amount", 1.0, 0.0);
    fx.float(
        conv,
        "ADBE ShadowHighlight-0003",
        "highlight_amount",
        1.0,
        0.0,
    );
    fx.float(
        conv,
        "ADBE ShadowHighlight-0007",
        "shadow_tonal_width",
        1.0,
        0.0,
    );
    fx.float(
        conv,
        "ADBE ShadowHighlight-0009",
        "highlight_tonal_width",
        1.0,
        0.0,
    );
    // One gaussian, so one radius: the mean of AE's two, px@comp.
    let shadow = fx.still("ADBE ShadowHighlight-0008").unwrap_or(30.0);
    let highlight = fx.still("ADBE ShadowHighlight-0010").unwrap_or(30.0);
    let radius = (shadow + highlight) * 0.5;
    fx.set("radius", EffectValue::Float(LumProperty::fixed(radius)));
    fx.approx_named(
        conv,
        "Shadow Radius and Highlight Radius",
        "one radius at their mean",
    );
    fx.float(
        conv,
        "ADBE ShadowHighlight-0011",
        "colour_correction",
        1.0,
        0.0,
    );
    fx.float(
        conv,
        "ADBE ShadowHighlight-0012",
        "midtone_contrast",
        1.0,
        0.0,
    );
    fx.float(conv, "ADBE ShadowHighlight-0016", "mix", -1.0, 100.0);
    if fx.still("ADBE ShadowHighlight-0001").unwrap_or(0.0).abs() > f64::EPSILON {
        fx.approx_named(
            conv,
            "Auto Amounts",
            "the manual pair After Effects was holding, a grade that reads the shot around it \
             not being this effect",
        );
    }
    fx.drop_params(
        conv,
        &[
            "Temporal Smoothing (seconds)",
            "Scene Detect",
            "Black Clip",
            "White Clip",
        ],
    );
    fx.done()
}

/// "Tritone" → **Tritone** (docs/08 §3.60). The three stops and the blend
/// carry; the stops are placed perceptually and a highlight above white is
/// scaled rather than clamped.
fn tritone(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "tritone", "Tritone")?;
    fx.colour(conv, "ADBE Tritone-0001", "highlights");
    fx.colour(conv, "ADBE Tritone-0002", "midtones");
    fx.colour(conv, "ADBE Tritone-0003", "shadows");
    fx.float(conv, "ADBE Tritone-0004", "mix", -1.0, 100.0);
    fx.differs(
        conv,
        "the three stops sit on a perceptual position rather than on half the light, and a \
         highlight above white is scaled rather than clamped",
    );
    fx.done()
}

/// "Posterize" → **Posterize** (docs/08 §3.58). One number, unchanged; the
/// bands land in the same places rather than at the same numbers, because
/// Lumit quantises a perceptual position in scene-linear light.
fn posterize(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "posterize", "Posterize")?;
    fx.float(conv, "ADBE Posterize-0001", "levels", 1.0, 0.0);
    fx.differs(
        conv,
        "the bands are cut on a perceptual position in scene-linear light, so they land where \
         After Effects' did rather than at the same numbers",
    );
    fx.done()
}

/// "Threshold" → **Threshold** (docs/08 §3.59). AE's Level is an eight-bit
/// display value and Lumit's is a per cent of the same perceptual placement.
fn threshold(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "threshold", "Threshold")?;
    fx.float(conv, "ADBE Threshold-0001", "level", 100.0 / 255.0, 0.0);
    fx.rebased(conv, "Level");
    fx.done()
}

/// "Broadcast Colors" → **Broadcast safe** (docs/08 §3.69). Every control has
/// a counterpart in the same units; the signal is encoded with the square root
/// both render paths already agree about, which is under two IRE from the real
/// transfer function.
fn broadcast_safe(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "broadcast_safe", "Broadcast Colors")?;
    // 1 NTSC, 2 PAL — AE's default of 1 is NTSC, which is Lumit's 0.
    fx.choice(conv, "ADBE Broadcast Colors-0001", "standard", |v| {
        (u32::from(v == 2), None)
    });
    // Reduce Luminance, Reduce Saturation, Key Out Unsafe, Key Out Safe —
    // the four Lumit ships, in the same order.
    fx.choice(conv, "ADBE Broadcast Colors-0002", "how_to_treat", |v| {
        (u32::try_from((v - 1).clamp(0, 3)).unwrap_or(0), None)
    });
    fx.float(
        conv,
        "ADBE Broadcast Colors-0003",
        "maximum_signal",
        1.0,
        0.0,
    );
    fx.differs(
        conv,
        "the signal is encoded with the square root both render paths agree about rather than \
         with the real transfer function, which is under two IRE across the range",
    );
    fx.done()
}

// ---------------------------------------------------------------------------
// Generate
// ---------------------------------------------------------------------------

/// "Fill" → **Fill** (docs/08 §3.34). Exact for a whole-alpha fill; a
/// mask-targeted one, with its Invert and the two Feather controls, is
/// reported rather than approximated.
fn fill(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "fill", "Fill")?;
    fx.colour(conv, "ADBE Fill-0002", "colour");
    fx.float(conv, "ADBE Fill-0005", "mix", 100.0, 0.0);
    let targeted = fx.still("ADBE Fill-0001").unwrap_or(0.0) > 0.5
        || fx.still("ADBE Fill-0007").unwrap_or(0.0).abs() > f64::EPSILON;
    if targeted {
        fx.approx_named(
            conv,
            "Fill Mask",
            "a fill of the layer's whole alpha, Lumit having no per-mask effect targeting",
        );
        fx.drop_params(conv, &["Invert", "Horizontal Feather", "Vertical Feather"]);
    }
    fx.done()
}

/// "Gradient Ramp" → **Gradient** (docs/08 §3.35). The two points, the two
/// colours, Ramp Scatter and Blend With Original all have counterparts; the
/// ramp interpolates in scene-linear light, so its midpoint sits where the
/// light says.
fn gradient(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "gradient", "Gradient Ramp")?;
    fx.point(conv, "ADBE Ramp-0001", "start_x", "start_y", 1.0);
    fx.colour(conv, "ADBE Ramp-0002", "start_colour");
    fx.point(conv, "ADBE Ramp-0003", "end_x", "end_y", 1.0);
    fx.colour(conv, "ADBE Ramp-0004", "end_colour");
    // 1 Linear Ramp, 2 Radial Ramp — AE's default of 1 is Lumit's 0.
    fx.choice(conv, "ADBE Ramp-0005", "shape", |v| {
        (u32::from(v == 2), None)
    });
    fx.float(conv, "ADBE Ramp-0006", "scatter", 1.0, 0.0);
    fx.float(conv, "ADBE Ramp-0007", "mix", -1.0, 100.0);
    fx.differs(
        conv,
        "the ramp interpolates in scene-linear light, so a long ramp's midpoint sits where the \
         light says rather than where the display range put it",
    );
    fx.done()
}

/// "Noise" → **Noise** (docs/08 §3.36). Amount and colour noise map directly;
/// AE's "Clip result values" has no counterpart, scene-linear having headroom.
fn noise(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "noise", "Noise")?;
    fx.float(conv, "ADBE Noise-0001", "amount", 1.0, 0.0);
    fx.toggle("ADBE Noise-0002", "colour_noise");
    fx.differs(
        conv,
        "nothing is clipped: grain rides on top of a highlight instead of flattening it, which \
         is what scene-linear headroom is for",
    );
    fx.drop_param(conv, "Clipping");
    fx.done()
}

/// After Effects' Fractal Noise Scale is a per cent of a base it does not
/// publish. Lumit's is the size of one noise cell in px@comp, and the factor
/// below lands AE's default of 100% on Lumit's declared default of 200 px —
/// the one point at which both specifications claim the two look alike.
const AE_FRACTAL_SCALE_BASE: f64 = 2.0;

/// "Fractal Noise" → **Fractal noise** (docs/08 §3.37). Contrast, brightness,
/// the transform, the sub settings and the evolution cycle convert directly;
/// Scale converts through AE's own base, and AE's dozen fractal types and four
/// noise types collapse onto the two apiece Lumit ships.
fn fractal_noise(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "fractal_noise", "Fractal Noise")?;
    // 1 is Basic on both sides; every other fold is the turbulent sum.
    fx.choice(conv, "ADBE Fractal Noise-0001", "fractal_type", |v| {
        if v == 1 {
            (0, None)
        } else {
            (
                1,
                Some("Turbulent, the folded sum being the nearer of the two Lumit ships"),
            )
        }
    });
    // 1 Block, 2 Linear, 3 Soft Linear, 4 Spline — Block is the lattice's own
    // value, everything smoother is Perlin.
    fx.choice(conv, "ADBE Fractal Noise-0002", "noise_type", |v| match v {
        1 => (0, None),
        3 => (1, None),
        _ => (1, Some("Perlin, the smoother of the two bases Lumit ships")),
    });
    fx.toggle("ADBE Fractal Noise-0003", "invert");
    fx.float(conv, "ADBE Fractal Noise-0004", "contrast", 1.0, 0.0);
    fx.float(conv, "ADBE Fractal Noise-0005", "brightness", 1.0, 0.0);
    fx.float(conv, "ADBE Fractal Noise-0008", "rotation", 1.0, 0.0);
    fx.toggle("ADBE Fractal Noise-0009", "uniform_scaling");
    for (ae, lumit) in [
        ("ADBE Fractal Noise-0010", "scale"),
        ("ADBE Fractal Noise-0011", "scale_width"),
        ("ADBE Fractal Noise-0012", "scale_height"),
    ] {
        fx.float(conv, ae, lumit, AE_FRACTAL_SCALE_BASE, 0.0);
    }
    fx.rebased(conv, "Scale");
    fx.point(conv, "ADBE Fractal Noise-0013", "offset_x", "offset_y", 1.0);
    fx.float(conv, "ADBE Fractal Noise-0015", "complexity", 1.0, 0.0);
    fx.float(conv, "ADBE Fractal Noise-0017", "sub_influence", 1.0, 0.0);
    fx.float(conv, "ADBE Fractal Noise-0018", "sub_scaling", 1.0, 0.0);
    fx.float(conv, "ADBE Fractal Noise-0023", "evolution", 1.0, 0.0);
    fx.toggle("ADBE Fractal Noise-0025", "cycle_evolution");
    fx.float(conv, "ADBE Fractal Noise-0026", "cycle", 1.0, 0.0);
    fx.seed("ADBE Fractal Noise-0027", "seed");
    fx.float(conv, "ADBE Fractal Noise-0029", "mix", 1.0, 0.0);
    fx.drop_params(
        conv,
        &[
            "Overflow",
            "Sub Rotation",
            "Sub Offset",
            "Center Subscale",
            "Perspective Offset",
            "Blending Mode",
        ],
    );
    fx.done()
}

/// "Beam" → **Beam** (docs/08 §3.73). Every control carries under the same
/// name; the two thicknesses cross from raster pixels to px@comp, Softness is
/// measured against the rim rather than the whole width, and AE's 3D
/// Perspective foreshortens from a camera Lumit keeps on the composition.
///
/// AE's Length is a fraction of the run between the two points and Lumit's is
/// px@comp (K-558), so it is multiplied by the run the points just converted
/// describe — read at time zero, since a keyframed pair means AE's fraction
/// stood for a distance that moved and no single pixel number is all of them.
fn beam(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "beam", "Beam")?;
    fx.point(conv, "ADBE Laser-0001", "start_x", "start_y", 1.0);
    fx.point(conv, "ADBE Laser-0002", "end_x", "end_y", 1.0);
    let at_zero = |fx: &Fx<'_>, id: &str| match fx.inst.param(id) {
        Some(EffectValue::Float(p)) => p.value_at(0.0),
        _ => 0.0,
    };
    let run = (at_zero(&fx, "end_x") - at_zero(&fx, "start_x"))
        .hypot(at_zero(&fx, "end_y") - at_zero(&fx, "start_y"));
    fx.float(conv, "ADBE Laser-0003", "length", run, 0.0);
    fx.float(conv, "ADBE Laser-0004", "time", 100.0, 0.0);
    fx.float(conv, "ADBE Laser-0005", "start_thickness", 1.0, 0.0);
    fx.float(conv, "ADBE Laser-0006", "end_thickness", 1.0, 0.0);
    fx.float(conv, "ADBE Laser-0007", "softness", 100.0, 0.0);
    fx.approx_named(
        conv,
        "Softness",
        "a softness measured against the beam's rim rather than against its whole width",
    );
    fx.colour(conv, "ADBE Laser-0008", "inside_colour");
    fx.colour(conv, "ADBE Laser-0009", "outside_colour");
    fx.toggle("ADBE Laser-0011", "composite_on_original");
    fx.drop_param(conv, "3D Perspective");
    fx.done()
}

/// After Effects' Advanced Lightning Turbulence is a multiplier about 1.0 with
/// no published range; Lumit's Amplitude is a per cent. The factor lands AE's
/// default on Lumit's declared default, the same anchor Fractal noise's Scale
/// takes.
const AE_LIGHTNING_TURBULENCE_BASE: f64 = 12.0;

/// "Advanced Lightning" → **Lightning** (docs/08 §3.74). Origin, direction,
/// conductivity state, forking, decay and the two colour groups convert; four
/// of AE's eight types are built and the other four map to the nearest; the
/// bolt's own shape is Lumit's, AE's displacement being undocumented.
fn lightning(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "lightning", "Advanced Lightning")?;
    // 1 Direction, 2 Strike, 3 Breaking, 4 Bouncey, 5 Omni, 6 Anywhere,
    // 7 Vertical, 8 Two-Way Strike. AE's default of 1 is Direction, Lumit's 0.
    fx.choice(
        conv,
        "ADBE Lightning 2-0001",
        "lightning_type",
        |v| match v {
            1 => (0, None),
            2 => (1, None),
            5 => (2, None),
            8 => (3, None),
            3 | 4 => (
                1,
                Some("Strike, the nearest of the four types Lumit builds"),
            ),
            6 => (2, Some("Omni, the nearest of the four types Lumit builds")),
            _ => (
                0,
                Some("Direction, the nearest of the four types Lumit builds"),
            ),
        },
    );
    fx.point(conv, "ADBE Lightning 2-0002", "origin_x", "origin_y", 1.0);
    fx.point(
        conv,
        "ADBE Lightning 2-0003",
        "direction_x",
        "direction_y",
        1.0,
    );
    fx.float(conv, "ADBE Lightning 2-0004", "conductivity", 1.0, 0.0);
    fx.float(conv, "ADBE Lightning 2-0006", "core_radius", 1.0, 0.0);
    fx.colour(conv, "ADBE Lightning 2-0008", "core_colour");
    fx.float(conv, "ADBE Lightning 2-0011", "glow_radius", 1.0, 0.0);
    fx.float(conv, "ADBE Lightning 2-0012", "glow_opacity", 1.0, 0.0);
    fx.colour(conv, "ADBE Lightning 2-0013", "glow_colour");
    fx.float(
        conv,
        "ADBE Lightning 2-0016",
        "amplitude",
        AE_LIGHTNING_TURBULENCE_BASE,
        0.0,
    );
    fx.rebased(conv, "Turbulence");
    fx.float(conv, "ADBE Lightning 2-0017", "forking", 100.0, 0.0);
    fx.float(conv, "ADBE Lightning 2-0018", "decay", 100.0, 0.0);
    fx.toggle("ADBE Lightning 2-0020", "composite_on_original");
    fx.differs(
        conv,
        "the bolt's shape is Lumit's own, After Effects' displacement being undocumented",
    );
    fx.drop_params(
        conv,
        &[
            "Core Opacity",
            "Alpha Obstacle",
            "Decay Main Core",
            "the Expert Settings group",
        ],
    );
    fx.done()
}

/// "Radio Waves" → **Radio waves** (docs/08 §3.75). The producer point, the
/// wave motion and the stroke convert; AE's clock becomes Lumit's Time
/// control, its start and end widths become one width, and only the Polygon
/// wave type is built.
fn radio_waves(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "radio_waves", "Radio Waves")?;
    fx.point(conv, "APC Radio Waves-0004", "centre_x", "centre_y", 1.0);

    // §2.4 forbids an effect that reads the clock, so the clock becomes two
    // keyframes running at one second a second across the layer's own span —
    // AE's motion exactly, and deterministic.
    let (from, to) = conv.span;
    let seconds = to.checked_sub(from).unwrap_or(from).to_f64();
    if seconds > 0.0 {
        fx.set("time", EffectValue::Float(ramp(from, 0.0, to, seconds)));
    }
    fx.clock(conv, "Time");

    fx.float(conv, "APC Radio Waves-0034", "frequency", 1.0, 0.0);
    fx.float(conv, "APC Radio Waves-0036", "expansion", 1.0, 0.0);
    fx.float(conv, "APC Radio Waves-0038", "rotation", 1.0, 0.0);
    fx.float(conv, "APC Radio Waves-0044", "spin", 1.0, 0.0);
    let lifespan = fx.still("APC Radio Waves-0056").unwrap_or(10.0).max(1e-6);
    fx.float(conv, "APC Radio Waves-0056", "lifespan", 1.0, 0.0);
    fx.float(conv, "APC Radio Waves-0008", "sides", 1.0, 0.0);
    fx.toggle("APC Radio Waves-0014", "star");
    // AE's Star Depth is signed and Lumit's is a depth; the magnitude is the
    // shape and the sign is which way the points face.
    if let Some(depth) = fx.still("APC Radio Waves-0016") {
        fx.set(
            "star_depth",
            EffectValue::Float(LumProperty::fixed((depth.abs() * 100.0).clamp(0.0, 100.0))),
        );
        if depth < 0.0 {
            fx.approx_named(
                conv,
                "Star Depth",
                "its magnitude, Lumit's depth being unsigned",
            );
        }
    }
    fx.float(conv, "APC Radio Waves-0050", "stroke_width", 1.0, 0.0);
    fx.approx_named(
        conv,
        "Start Width and End Width",
        "one stroke width taken from the start, Lumit's ring not tapering",
    );
    fx.colour(conv, "APC Radio Waves-0046", "colour");
    fx.float(conv, "APC Radio Waves-0054", "opacity", 100.0, 0.0);
    // AE times the two fades in seconds; Lumit states them as a share of the
    // lifespan, so the ratio is the conversion.
    fx.float(
        conv,
        "APC Radio Waves-0058",
        "fade_in",
        100.0 / lifespan,
        0.0,
    );
    fx.float(
        conv,
        "APC Radio Waves-0060",
        "fade_out",
        100.0 / lifespan,
        0.0,
    );
    fx.rebased(conv, "Fade-in Time and Fade-out Time");

    // 1 Polygon, 2 Image Contours, 3 Mask.
    match fx.still("APC Radio Waves-0002").unwrap_or(1.0).round() as i64 {
        2 => fx.approx_named(
            conv,
            "Wave Type",
            "a polygon — Vegas is the effect that marches a stroke round a found contour",
        ),
        3 => fx.approx_named(
            conv,
            "Wave Type",
            "a polygon — Vegas on its Mask/Path source is the effect that marches a stroke \
             round a mask",
        ),
        _ => {}
    }
    fx.drop_params(
        conv,
        &[
            "Parameters are set at",
            "Render Quality",
            "Reflection",
            "Direction",
            "Velocity",
            "Curvyness",
        ],
    );
    fx.done()
}

/// "Vegas" → **Vegas** (docs/08 §3.76), both halves since K-408. The stroke
/// and the segments convert; AE's count of segments becomes a length, exactly
/// on the Mask/Path half where the perimeter can be measured and approximately
/// on the Image Contours half where it cannot.
fn vegas(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "vegas", "Vegas")?;
    // 1 Image Contours, 2 Mask/Path.
    let mask_half = fx.still("APC Vegas-0052").unwrap_or(1.0).round() as i64 == 2;
    let perimeter = fx.mask(conv, "APC Vegas-0050", "path");

    if mask_half {
        fx.set("source", EffectValue::Choice(2));
    } else {
        // AE's Channel picks what the contour is a contour *of*; Lumit's
        // Source is the same question with a shorter list, and its first entry
        // is AE's default.
        fx.set("source", EffectValue::Choice(0));
        if fx.still("APC Vegas-0010").unwrap_or(1.0).round() as i64 != 1 {
            fx.approx_named(conv, "Channel", "a level set of the perceptual luma");
        }
        if fx.still("APC Vegas-0002").unwrap_or(0.0).abs() > f64::EPSILON {
            fx.drop_param(conv, "Input Layer");
        }
        if fx.still("APC Vegas-0004").unwrap_or(0.0).abs() > f64::EPSILON {
            fx.drop_param(conv, "Invert Input");
        }
        fx.float(conv, "APC Vegas-0012", "threshold", 100.0 / 255.0, 0.0);
        fx.rebased(conv, "Threshold");
        fx.differs(
            conv,
            "the contour is a level set of the perceptual luma rather than an edge detector's \
             output, so Threshold converts in meaning rather than in kind",
        );
    }

    // AE counts segments round a contour; Lumit spaces them along it, so the
    // count becomes a length through the path's own perimeter.
    let segments = fx.still("APC Vegas-0028").unwrap_or(32.0).max(1.0);
    let (w, h) = conv.size;
    let length = match (mask_half, perimeter) {
        (true, Some(p)) if p > 0.0 => p / segments,
        _ => {
            fx.approx_named(
                conv,
                "Segments",
                "a segment length taken from the frame's own perimeter, no contour being \
                 measurable before the picture exists",
            );
            2.0 * (w + h) / segments
        }
    };
    fx.set(
        "segment_length",
        EffectValue::Float(LumProperty::fixed(length.max(1.0))),
    );

    fx.colour(conv, "APC Vegas-0018", "colour");
    fx.float(conv, "APC Vegas-0020", "width", 1.0, 0.0);
    fx.float(conv, "APC Vegas-0022", "hardness", 100.0, 0.0);
    fx.float(conv, "APC Vegas-0024", "length", 100.0, 0.0);
    fx.float(conv, "APC Vegas-0030", "rotation", 1.0, 0.0);
    fx.float(conv, "APC Vegas-0036", "opacity", 100.0, 0.0);
    fx.drop_params(
        conv,
        &[
            "Pre-Blur",
            "Tolerance",
            "Render",
            "Selected Contour",
            "Shorter Contours Have",
            "Segment Distribution",
            "Random Phase",
            "Random Seed",
            "Mid-point Opacity",
            "Mid-point Position",
            "End Opacity",
            "Blend Mode",
        ],
    );
    fx.done()
}

/// "Add Grain" → **Add grain** (docs/08 §3.77). Intensity, size, softness, the
/// colour balances and the tonal amounts convert; AE's Animation Speed becomes
/// Lumit's Animate switch, its movable tonal boundaries become three fixed
/// ones, and the grain field itself is Lumit's own.
fn add_grain(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "add_grain", "Add Grain")?;
    // AE states these as multipliers about 1.0 and Lumit as per cents, which
    // its own neutral 100 for the channel balances and the tonal amounts pins.
    fx.float(conv, "VISINF Grain Implant-0008", "intensity", 100.0, 0.0);
    fx.float(conv, "VISINF Grain Implant-0007", "size", 1.0, 0.0);
    fx.rebased(conv, "Size");
    // Softness alone has no neutral point to pin it, so it takes the anchor
    // this file's header names: AE's default lands on Lumit's.
    fx.float(conv, "VISINF Grain Implant-0130", "softness", 50.0, 0.0);
    fx.float(conv, "VISINF Grain Implant-0002", "red", 100.0, 0.0);
    fx.float(conv, "VISINF Grain Implant-0003", "green", 100.0, 0.0);
    fx.float(conv, "VISINF Grain Implant-0004", "blue", 100.0, 0.0);
    fx.toggle("VISINF Grain Implant-0005", "monochrome");
    fx.float(conv, "VISINF Grain Implant-0040", "shadows", 100.0, 0.0);
    fx.float(conv, "VISINF Grain Implant-0041", "midtones", 100.0, 0.0);
    fx.float(conv, "VISINF Grain Implant-0042", "highlights", 100.0, 0.0);
    // A grain that redraws at a *rate* reads the clock, which §2.4 forbids: a
    // non-zero speed is Animate on, zero is off.
    if let Some(speed) = fx.still("VISINF Grain Implant-0039") {
        fx.set("animate", EffectValue::Bool(speed.abs() > f64::EPSILON));
        fx.approx_named(
            conv,
            "Animation Speed",
            "the Animate switch, a grain redrawing at a rate having to read the clock",
        );
    }
    fx.seed("VISINF Grain Implant-0013", "seed");
    fx.approx_named(
        conv,
        "the Tonal Ranges boundaries",
        "three fixed ranges, their three amounts carrying one for one",
    );
    fx.differs(
        conv,
        "the grain field is Lumit's own rather than a sampled film stock",
    );
    fx.drop_params(
        conv,
        &[
            "Viewing Mode",
            "Preset",
            "Aspect Ratio",
            "Saturation",
            "Blending Mode",
            "Animate Smoothly",
            "the Channel Size group",
            "the Channel Balance group",
            "the Application group",
        ],
    );
    fx.done()
}

/// "Scribble" → **Scribble** (docs/08 §3.78), the first import to carry a mask
/// reference across (K-408). The mask, the stroke and the wiggle convert; the
/// edge options, the variations and the multi-mask fill types are reported.
fn scribble(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "scribble", "Scribble")?;
    fx.mask(conv, "ADBE Scribble Fill-0002", "path");
    fx.colour(conv, "ADBE Scribble Fill-0006", "colour");
    fx.float(conv, "ADBE Scribble Fill-0010", "angle", 1.0, 0.0);
    fx.float(conv, "ADBE Scribble Fill-0008", "stroke_width", 1.0, 0.0);
    fx.float(conv, "ADBE Scribble Fill-0060", "spacing", 1.0, 0.0);
    fx.float(conv, "ADBE Scribble Fill-0038", "path_overlap", 1.0, 0.0);
    fx.rebased(conv, "Stroke Width, Spacing and Path Overlap");
    fx.float(conv, "ADBE Scribble Fill-0030", "start", 1.0, 0.0);
    fx.float(conv, "ADBE Scribble Fill-0032", "end", 1.0, 0.0);
    // 1 Static, 2 Jagged, 3 Wiggly — the same three, in the same order.
    fx.choice(conv, "ADBE Scribble Fill-0048", "wiggle_type", |v| {
        (u32::try_from((v - 1).clamp(0, 2)).unwrap_or(0), None)
    });
    fx.float(
        conv,
        "ADBE Scribble Fill-0044",
        "wiggles_per_second",
        1.0,
        0.0,
    );
    fx.seed("ADBE Scribble Fill-0046", "seed");
    fx.float(conv, "ADBE Scribble Fill-0024", "opacity", 100.0, 0.0);
    // 1 On Transparent, 2 On Original Image — AE's default of 2 is Lumit's on.
    if let Some(v) = fx.still("ADBE Scribble Fill-0026") {
        fx.set(
            "composite_on_original",
            EffectValue::Bool(v.round() as i64 != 1),
        );
    }
    if fx.still("ADBE Scribble Fill-0064").unwrap_or(1.0).round() as i64 != 1 {
        fx.approx_named(
            conv,
            "Scribble",
            "one mask, docs/08 §1.2's mask-path row naming a single mask by design",
        );
    }
    if fx.still("ADBE Scribble Fill-0050").unwrap_or(1.0).round() as i64 != 1 {
        fx.approx_named(conv, "Fill Type", "the plain fill");
    }
    fx.drop_params(
        conv,
        &[
            "the Edge Options group",
            "Start/End Apply To",
            "Curviness",
            "Curviness Variation",
            "Spacing Variation",
            "Path Overlap Variation",
            "Fill Paths Sequentially",
        ],
    );
    fx.done()
}

/// "Stroke" → **Stroke** (docs/08 §3.79). Every control has a counterpart and
/// Paint Style maps option for option; AE's Brush Size is a radius where
/// Lumit's Brush size is a width, so it doubles.
fn stroke(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "stroke", "Stroke")?;
    fx.mask(conv, "ADBE Stroke-0001", "path");
    fx.colour(conv, "ADBE Stroke-0002", "colour");
    fx.float(conv, "ADBE Stroke-0003", "brush_size", 2.0, 0.0);
    fx.rebased(conv, "Brush Size");
    fx.float(conv, "ADBE Stroke-0004", "hardness", 100.0, 0.0);
    fx.float(conv, "ADBE Stroke-0005", "opacity", 100.0, 0.0);
    fx.float(conv, "ADBE Stroke-0008", "start", 1.0, 0.0);
    fx.float(conv, "ADBE Stroke-0009", "end", 1.0, 0.0);
    fx.float(conv, "ADBE Stroke-0006", "spacing", 1.0, 0.0);
    // 1 On Original Image, 2 On Transparent, 3 Reveal Original Image — the
    // same three, in the same order.
    fx.choice(conv, "ADBE Stroke-0007", "paint_style", |v| {
        (u32::try_from((v - 1).clamp(0, 2)).unwrap_or(0), None)
    });
    if fx.still("ADBE Stroke-0010").unwrap_or(0.0).abs() > f64::EPSILON {
        fx.approx_named(
            conv,
            "All Masks",
            "one mask, docs/08 §1.2's mask-path row naming a single mask by design",
        );
    }
    fx.drop_param(conv, "Stroke Sequentially");
    fx.done()
}

// ---------------------------------------------------------------------------
// Temporal
// ---------------------------------------------------------------------------

/// "Echo" → **Echo** (docs/08 §3.25). The count, the decay and the operator
/// carry; Lumit's echoes are one frame apart by declaration (its temporal
/// window is the previous sixteen frames), so AE's Echo Time is reported
/// unless it already names a single frame back.
fn echo(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "echo", "Echo")?;
    fx.float(conv, "ADBE Echo-0002", "echoes", 1.0, 0.0);
    fx.float(conv, "ADBE Echo-0004", "decay", 1.0, 0.0);
    // 1 Add, 2 Maximum, 3 Minimum, 4 Screen, 5 Composite in Back,
    // 6 Composite in Front, 7 Blend. AE's default of 1 is Add.
    fx.choice(conv, "ADBE Echo-0005", "mode", |v| match v {
        1 => (2, None),
        2 => (8, None),
        3 => (9, None),
        4 => (3, None),
        5 => (0, None),
        6 => (1, None),
        _ => (2, Some("Add, Lumit's list having no averaging entry")),
    });
    // AE's Echo Time is seconds a frame back; Lumit's samples are whole frames.
    let step = -1.0 / conv.tb.rate().fps();
    if fx
        .still("ADBE Echo-0001")
        .is_some_and(|t| (t - step).abs() > 1e-4)
    {
        fx.approx_named(
            conv,
            "Echo Time (seconds)",
            "one frame a step, Lumit's echoes being declared on whole frames",
        );
    }
    fx.drop_param(conv, "Starting Intensity");
    fx.done()
}

/// "Posterize Time" → **Posterize time** (docs/08 §3.26). One number, and it
/// means the same thing; Lumit adds a Phase AE does not have, which defaults
/// to zero and so changes nothing on import.
fn posterize_time(conv: &mut Conv<'_>, path: &ItemPath, node: &Property) -> Option<EffectInstance> {
    let mut fx = Fx::new(path, node, "posterize_time", "Posterize Time")?;
    fx.float(conv, "ADBE Posterize Time-0001", "rate", 1.0, 0.0);
    fx.done()
}

/// The perimeter of a mask's path, in layer pixels — what turns AE's *count*
/// of Vegas segments into Lumit's segment *length*.
///
/// A cubic's length is between its chord and its control net, and the mean of
/// the two weighted towards the chord is the standard cheap estimate; a
/// polygon mask, where the two coincide, comes out exact.
pub(crate) fn perimeter(path: &lumit_core::mask::BezierPath) -> f64 {
    let v = &path.vertices;
    if v.len() < 2 {
        return 0.0;
    }
    let last = if path.closed { v.len() } else { v.len() - 1 };
    let mut total = 0.0;
    for i in 0..last {
        let a = &v[i];
        let b = &v[(i + 1) % v.len()];
        let p0 = a.pos;
        let p1 = (a.pos.0 + a.tan_out.0, a.pos.1 + a.tan_out.1);
        let p2 = (b.pos.0 + b.tan_in.0, b.pos.1 + b.tan_in.1);
        let p3 = b.pos;
        let d = |a: (f64, f64), b: (f64, f64)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let chord = d(p0, p3);
        let net = d(p0, p1) + d(p1, p2) + d(p2, p3);
        total += (2.0 * chord + net) / 3.0;
    }
    total
}

/// The id and perimeter of every mask on the layer, in After Effects' order —
/// what [`Conv::masks`](super::Conv) carries for the effects that name one.
pub(crate) fn mask_refs(masks: &[lumit_core::mask::Mask]) -> Vec<(Uuid, f64)> {
    masks
        .iter()
        .map(|m| {
            let path = m
                .path_keys
                .first()
                .map_or(&m.path, |key: &lumit_core::mask::PathKeyframe| &key.path);
            (m.id, perimeter(path))
        })
        .collect()
}

#[cfg(test)]
mod tests;
