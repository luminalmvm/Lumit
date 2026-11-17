//! A layer's effects as SVG filters: Drop shadow and Gaussian blur, which a
//! browser draws the way Lumit does, with Levels as a table, Transform as
//! groups and Set matte as a mask. Everything else is left out by name.
//!
//! Each effect is a group of its own around what came before it, so the
//! stack runs in Lumit's order. A filter's region is set in the layer's own
//! pixels round what it draws, padded by three deviations of the blur and the
//! shadow's distance, so nothing soft is cut at the edge of the default
//! region.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::model::{EffectInstance, EffectValue, Layer};

use super::anim::Value;
use super::layers::pair;
use super::{Scope, Svg};
use crate::check::{arg, Place};
use crate::fit::tolerance;
use crate::walk::SetMatte;
use crate::LottieError;

impl Svg<'_> {
    /// `layer`'s effects as groups around what it draws, the first effect
    /// innermost. How many groups it opened.
    pub(super) fn effects(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        place: &Place,
        out: &mut String,
    ) -> Result<usize, LottieError> {
        for style in &layer.styles {
            if style.enabled {
                let label = label(style, true);
                self.report.left_out(
                    &place.part(&label),
                    "style",
                    format!("{label} is left out"),
                    vec![arg("label", label.clone())],
                );
            }
        }
        // Each effect's opening tags, and how many groups they open.
        let mut groups: Vec<(String, usize)> = Vec::new();
        let filter = |id: String| (format!("<g filter=\"url(#{id})\">"), 1);
        for e in layer.effects.iter().filter(|e| e.enabled) {
            let group = match e.effect.match_name.as_str() {
                "drop_shadow" => {
                    // A shadow under its layer mixed in part is the same
                    // shadow, lighter. Alone, or with a Mix that moves, it is
                    // not.
                    let shadow_only = e.bool_of("shadow_only").unwrap_or(false);
                    if mix(e).is_none_or(|m| shadow_only && m < 1.0) {
                        self.control_left_out(place, e, "effect_mix", "Mix");
                    }
                    Some(filter(self.drop_shadow(scope, layer, e)))
                }
                "blur" => {
                    if mix(e).is_none_or(|m| m < 1.0) {
                        self.control_left_out(place, e, "effect_mix", "Mix");
                    }
                    // The blur here spreads past the layer's edge, as it
                    // does in Lumit with the switch off.
                    if e.bool_of("repeat_edge_pixels").unwrap_or(true) {
                        let what = "Repeat edge pixels";
                        self.control_left_out(place, e, "effect_repeat_edge", what);
                    }
                    Some(filter(self.blur(scope, layer, e)))
                }
                "levels" => self.levels(scope, layer, e).map(filter),
                "transform" => self.transform_effect(scope, layer, e),
                "set_matte" => match crate::walk::set_matte(self.doc, scope.comp, layer, e) {
                    SetMatte::Unset => continue,
                    SetMatte::Alpha {
                        source,
                        inverted,
                        from,
                        to,
                    } => {
                        let id = self.set_matte(scope, source, inverted, from, to)?;
                        Some((format!("<g mask=\"url(#{id})\">"), 1))
                    }
                    SetMatte::LeftOut(reason) => {
                        let label = label(e, false);
                        self.report.left_out(
                            &place.part(&label),
                            reason,
                            crate::walk::set_matte_english(reason, &label),
                            vec![arg("label", label.clone())],
                        );
                        continue;
                    }
                },
                _ => None,
            };
            match group {
                Some(g) => groups.push(g),
                None => {
                    let label = label(e, false);
                    self.report.left_out(
                        &place.part(&label),
                        "effect",
                        format!("{label} is left out"),
                        vec![arg("label", label.clone())],
                    );
                }
            }
        }
        // The last effect outermost, so the first runs first.
        for (g, _) in groups.iter().rev() {
            out.push_str(g);
        }
        Ok(groups.iter().map(|(_, n)| n).sum())
    }

    /// A line saying `e`'s control `what` is left out.
    fn control_left_out(
        &mut self,
        place: &Place,
        e: &EffectInstance,
        reason: &'static str,
        what: &str,
    ) {
        let label = label(e, false);
        self.report.changed(
            &place.part(&label),
            reason,
            format!("{label}'s {what} is left out"),
            vec![arg("label", label.clone())],
        );
    }

    /// Set matte as a mask in the layer's own coordinates: `source`'s own
    /// drawing, its masks included and its transform not, stretched from its
    /// box `from` onto the layer's box `to` as Lumit stretches it, read by
    /// its alpha or by the opposite. The mask's id.
    fn set_matte(
        &mut self,
        scope: &Scope<'_>,
        source: &Layer,
        inverted: bool,
        from: [f64; 4],
        to: [f64; 4],
    ) -> Result<String, LottieError> {
        let place = scope.place.layer(source.id, &source.name);
        let (sx, sy) = (to[2] / from[2].max(1e-9), to[3] / from[3].max(1e-9));
        let mut inner = format!(
            "<g transform=\"translate({},{}) scale({},{})\">",
            self.px(to[0] - from[0] * sx),
            self.px(to[1] - from[1] * sy),
            self.unit(sx),
            self.unit(sy)
        );
        let open = self.masks(scope, source, &place, &mut inner);
        self.content(scope, source, &place, &mut inner)?;
        for _ in 0..=open {
            inner.push_str("</g>");
        }
        let id = self.id("m");
        let region = format!(
            "maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
            self.px(to[0]),
            self.px(to[1]),
            self.px(to[2]),
            self.px(to[3])
        );
        if inverted {
            // A white box with the source drawn on it in black, read by its
            // brightness: clear where the source is solid.
            let f = self.id("f");
            let _ = write!(
                self.defs,
                "<filter id=\"{f}\" filterUnits=\"userSpaceOnUse\" color-interpolation-filters=\"sRGB\" x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\"><feColorMatrix type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\"/></filter>\
                 <mask id=\"{id}\" mask-type=\"luminance\" {region}><rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" fill=\"#fff\"/><g filter=\"url(#{f})\">{inner}</g></mask>",
                x = self.px(to[0]),
                y = self.px(to[1]),
                w = self.px(to[2]),
                h = self.px(to[3]),
            );
        } else {
            let _ = write!(
                self.defs,
                "<mask id=\"{id}\" mask-type=\"alpha\" {region}>{inner}</mask>"
            );
        }
        Ok(id)
    }

    /// A Transform effect as groups that move what they hold, Lumit's own
    /// map inverted: a still matrix inside a translate that follows the
    /// corner of a Shape layer's art, where the effect's points are measured
    /// from. `None` when any of its numbers moves or its Mix is below 100,
    /// which would show the layer twice.
    fn transform_effect(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        e: &EffectInstance,
    ) -> Option<(String, usize)> {
        let v = |id: &str| still(e, id).map(|v| v as f32);
        if v("mix")? < 100.0 {
            return None;
        }
        let opacity = f64::from(v("opacity")?).clamp(0.0, 100.0) / 100.0;
        let hidden = || Some((String::from("<g opacity=\"0\">"), 1));
        let Some((m, o)) = lumit_core::fx::maths::transform_inverse(
            [v("anchor_x")?, v("anchor_y")?],
            [v("position_x")?, v("position_y")?],
            [v("scale_x")? / 100.0, v("scale_y")? / 100.0],
            v("rotation")?,
            [v("skew")?, v("skew_axis")?],
        ) else {
            // Scaled to nothing.
            return hidden();
        };
        let [m0, m1, m2, m3] = m.map(f64::from);
        let det = m0 * m3 - m1 * m2;
        if det == 0.0 {
            return hidden();
        }
        // The forward map `p = m⁻¹·(q − o)` in the layer's pixels, which
        // start at the art's corner `s`: in the art's coordinates it is
        // `q ↦ M·q + (s − M·(s + o))`.
        let (a, b, c, d) = (m3 / det, -m2 / det, -m1 / det, m0 / det);
        let (ox, oy) = (f64::from(o[0]), f64::from(o[1]));
        let lift = |s: &[f64]| {
            let (sx, sy) = (
                s.first().copied().unwrap_or(0.0),
                s.get(1).copied().unwrap_or(0.0),
            );
            let (px, py) = (sx + ox, sy + oy);
            vec![sx - (a * px + c * py), sy - (b * px + d * py)]
        };
        let lifted = match self.art_corner(scope, layer) {
            Value::Still(s) => Value::Still(lift(&s)),
            moving => {
                let samples = self
                    .span
                    .grid
                    .iter()
                    .map(|t| lift(&moving.at(*t)))
                    .collect();
                Value::sampled(&self.span.grid, samples, &[tolerance::PIXELS; 2])
            }
        };
        let mut g = String::new();
        let mut open = self.factor(&mut g, "translate", &lifted, &[tolerance::PIXELS; 2], &pair);
        let num = |v: f64| super::anim::num(v, 4);
        let _ = write!(
            g,
            "<g transform=\"matrix({} {} {} {} 0 0)\"",
            num(a),
            num(b),
            num(c),
            num(d)
        );
        if opacity < 1.0 {
            let _ = write!(g, " opacity=\"{}\"", num(opacity));
        }
        g.push('>');
        open += 1;
        Some((g, open))
    }

    /// Levels as a table per channel, worked out by Lumit's own curve and
    /// laid on linear light, where Lumit grades and where an SVG filter
    /// works by default. `None` when any of its numbers moves.
    fn levels(&mut self, scope: &Scope<'_>, layer: &Layer, e: &EffectInstance) -> Option<String> {
        let v = |id: &str| still(e, id).map(|v| v as f32);
        let levels = lumit_core::fx::effects::levels::Levels {
            master_in_black: v("master_in_black")?,
            master_in_white: v("master_in_white")?,
            master_gamma: v("master_gamma")?,
            master_out_black: v("master_out_black")?,
            master_out_white: v("master_out_white")?,
            red_in_black: v("red_in_black")?,
            red_in_white: v("red_in_white")?,
            red_gamma: v("red_gamma")?,
            red_out_black: v("red_out_black")?,
            red_out_white: v("red_out_white")?,
            green_in_black: v("green_in_black")?,
            green_in_white: v("green_in_white")?,
            green_gamma: v("green_gamma")?,
            green_out_black: v("green_out_black")?,
            green_out_white: v("green_out_white")?,
            blue_in_black: v("blue_in_black")?,
            blue_in_white: v("blue_in_white")?,
            blue_gamma: v("blue_gamma")?,
            blue_out_black: v("blue_out_black")?,
            blue_out_white: v("blue_out_white")?,
            mix: v("mix")?,
        };
        let (r, mix) = levels.packed();
        let (id, mut f) = self.filter_open(scope, layer, 1.0);
        f.insert_str(f.len() - 1, " color-interpolation-filters=\"linearRGB\"");
        f.push_str("<feComponentTransfer>");
        for (c, name) in ["feFuncR", "feFuncG", "feFuncB"].iter().enumerate() {
            let table: Vec<String> = (0..=255u16)
                .map(|i| {
                    let x = f32::from(i) / 255.0;
                    let graded = lumit_core::fx::cpu::level_at(
                        lumit_core::fx::cpu::level_at(x, &r, c + 1),
                        &r,
                        0,
                    );
                    self.unit(f64::from(x + (graded - x) * mix).clamp(0.0, 1.0))
                })
                .collect();
            let _ = write!(
                f,
                "<{name} type=\"table\" tableValues=\"{}\"/>",
                table.join(" ")
            );
        }
        f.push_str("</feComponentTransfer></filter>");
        self.defs.push_str(&f);
        Some(id)
    }

    /// One number of an effect as a value on `layer`'s clock.
    fn param(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        e: &EffectInstance,
        id: &str,
        tol: f64,
    ) -> Value {
        match e.param(id) {
            Some(EffectValue::Float(p)) => self.value(scope, layer, &[p], &[tol]),
            _ => Value::Still(vec![0.0]),
        }
    }

    /// The filter element opening for `layer`, its region padded by `pad`.
    fn filter_open(&mut self, scope: &Scope<'_>, layer: &Layer, pad: f64) -> (String, String) {
        let (x0, y0, x1, y1) = self.content_box(scope, layer);
        let id = self.id("f");
        let open = format!(
            "<filter id=\"{id}\" filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">",
            self.px(x0 - pad),
            self.px(y0 - pad),
            self.px(x1 - x0 + 2.0 * pad),
            self.px(y1 - y0 + 2.0 * pad)
        );
        (id, open)
    }

    /// Gaussian blur: its radius is two deviations, the reading Lumit's
    /// kernel takes.
    fn blur(&mut self, scope: &Scope<'_>, layer: &Layer, e: &EffectInstance) -> String {
        let sigma = self
            .param(scope, layer, e, "radius", tolerance::PIXELS)
            .map(|_, v| v.max(0.0) * 0.5);
        let pad = 3.0 * most(&sigma, &self.span.grid) + 2.0;
        let (id, mut f) = self.filter_open(scope, layer, pad);
        f.push_str("<feGaussianBlur");
        let fmt = |v: &[f64]| self.px(v.first().copied().unwrap_or(0.0));
        self.attr(&mut f, "stdDeviation", &sigma, &fmt);
        f.push('>');
        self.animate(&mut f, "stdDeviation", &sigma, &[tolerance::PIXELS], &fmt);
        f.push_str("</feGaussianBlur></filter>");
        self.defs.push_str(&f);
        id
    }

    /// Drop shadow: the shape's alpha blurred, moved, tinted and put under
    /// it, or alone when Shadow only is on.
    fn drop_shadow(&mut self, scope: &Scope<'_>, layer: &Layer, e: &EffectInstance) -> String {
        let sigma = self
            .param(scope, layer, e, "softness", tolerance::PIXELS)
            .map(|_, v| v.max(0.0) * 0.5);
        let distance = self.param(scope, layer, e, "distance", tolerance::PIXELS);
        let direction = self.param(scope, layer, e, "direction", tolerance::DEGREES);
        let fade = mix(e).unwrap_or(1.0).clamp(0.0, 1.0);
        let opacity = self
            .param(scope, layer, e, "opacity", tolerance::PERCENT)
            .map(|_, v| v.clamp(0.0, 100.0) / 100.0 * fade);
        // Measured from straight up and turning clockwise, on a picture whose
        // y runs down: sin and minus cos.
        let offset = if distance.is_still() && direction.is_still() {
            let (d, a) = (
                distance.at(0.0).first().copied().unwrap_or(0.0),
                direction
                    .at(0.0)
                    .first()
                    .copied()
                    .unwrap_or(0.0)
                    .to_radians(),
            );
            Value::Still(vec![d * a.sin(), -d * a.cos()])
        } else {
            let samples = self
                .span
                .grid
                .iter()
                .map(|t| {
                    let d = distance.at(*t).first().copied().unwrap_or(0.0);
                    let a = direction
                        .at(*t)
                        .first()
                        .copied()
                        .unwrap_or(0.0)
                        .to_radians();
                    vec![d * a.sin(), -d * a.cos()]
                })
                .collect();
            Value::sampled(&self.span.grid, samples, &[tolerance::PIXELS; 2])
        };
        let colour = match e.param("shadow_colour") {
            Some(EffectValue::Colour(ch)) => {
                let ch = [&ch[0], &ch[1], &ch[2]];
                self.colour(scope, layer, ch)
            }
            _ => Value::Still(vec![0.0, 0.0, 0.0]),
        };
        let shadow_only = e.bool_of("shadow_only").unwrap_or(false);
        let pad = 3.0 * most(&sigma, &self.span.grid) + most(&distance, &self.span.grid) + 2.0;
        let (id, mut f) = self.filter_open(scope, layer, pad);
        let px = |v: &[f64]| self.px(v.first().copied().unwrap_or(0.0));
        let unit = |v: &[f64]| self.unit(v.first().copied().unwrap_or(0.0));
        let hex = |v: &[f64]| {
            crate::colour::hex([
                v.first().copied().unwrap_or(0.0),
                v.get(1).copied().unwrap_or(0.0),
                v.get(2).copied().unwrap_or(0.0),
            ])
        };
        let x_of = |v: &[f64]| self.px(v.first().copied().unwrap_or(0.0));
        let y_of = |v: &[f64]| self.px(v.get(1).copied().unwrap_or(0.0));
        // Blur the alpha, move it, tint it, then lay the shape over it
        // unless only the shadow is wanted.
        f.push_str("<feGaussianBlur in=\"SourceAlpha\"");
        self.attr(&mut f, "stdDeviation", &sigma, &px);
        f.push('>');
        self.animate(&mut f, "stdDeviation", &sigma, &[tolerance::PIXELS], &px);
        f.push_str("</feGaussianBlur><feOffset result=\"s\"");
        self.attr(&mut f, "dx", &offset, &x_of);
        self.attr(&mut f, "dy", &offset, &y_of);
        f.push('>');
        self.animate(&mut f, "dx", &offset, &[tolerance::PIXELS; 2], &x_of);
        self.animate(&mut f, "dy", &offset, &[tolerance::PIXELS; 2], &y_of);
        f.push_str("</feOffset><feFlood");
        self.attr(&mut f, "flood-color", &colour, &hex);
        self.attr(&mut f, "flood-opacity", &opacity, &unit);
        f.push('>');
        self.animate(
            &mut f,
            "flood-color",
            &colour,
            &[tolerance::CHANNEL; 3],
            &hex,
        );
        self.animate(&mut f, "flood-opacity", &opacity, &[1e-4], &unit);
        f.push_str("</feFlood><feComposite in2=\"s\" operator=\"in\"/>");
        if !shadow_only {
            f.push_str("<feMerge><feMergeNode/><feMergeNode in=\"SourceGraphic\"/></feMerge>");
        }
        f.push_str("</filter>");
        self.defs.push_str(&f);
        id
    }
}

/// The largest the first number of `value` gets across `times`.
fn most(value: &Value, times: &[f64]) -> f64 {
    match value {
        Value::Still(v) => v.first().copied().unwrap_or(0.0).abs(),
        Value::Moving(_) => times
            .iter()
            .map(|t| value.at(*t).first().copied().unwrap_or(0.0).abs())
            .fold(0.0, f64::max),
    }
}

/// What an effect or a style is called: its own name, else its kind's.
pub(super) fn label(e: &EffectInstance, style: bool) -> String {
    if let Some(name) = e.custom_name.as_ref().filter(|n| !n.is_empty()) {
        return name.clone();
    }
    let name = e.effect.match_name.as_str();
    let found = if style {
        lumit_core::fx::styles::STYLE_DEFS.get(name)
    } else {
        lumit_core::fx::catalogue::BUILTIN_DEFS.get(name)
    };
    found.map_or_else(|| name.to_owned(), |d| d.schema().label.to_owned())
}

/// The Mix every effect ends with, as a fraction, or `None` when it moves.
fn mix(e: &EffectInstance) -> Option<f64> {
    e.param("mix")
        .map_or(Some(1.0), |_| still(e, "mix").map(|v| v / 100.0))
}

/// A still parameter's number; `None` when it is missing or moves.
fn still(e: &EffectInstance, id: &str) -> Option<f64> {
    match e.param(id) {
        Some(EffectValue::Float(p)) => match p.animation {
            lumit_core::anim::Animation::Static(v) => Some(v),
            _ => None,
        },
        _ => None,
    }
}
