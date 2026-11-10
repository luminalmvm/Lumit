//! A layer's masks: a clip when every mask is a plain hard Add, else a
//! `<mask>` that builds the stack up the way Lumit does, mask by mask.
//!
//! In the mask, white shows the layer and black hides it. Add draws a mask
//! white, Subtract draws it black, and Intersect keeps only what the stack
//! so far shares with the mask, by masking the stack with it. Feather is a
//! blur of the mask's shape and expansion a morphology, both inside the
//! mask, where browsers draw them cheaply. Lighten, Darken and Difference
//! have no such drawing and are written as Add, Intersect and Add.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::anim::Animation;
use lumit_core::mask::{Mask, MaskMode};
use lumit_core::model::{Layer, LayerKind};

use super::anim::{self, Value};
use super::layers::pair;
use super::paths::PathValue;
use super::{Scope, Svg};
use crate::check::{arg, Place};
use crate::fit::tolerance;

/// How much of a feather's width a Gaussian's deviation is: the soft edge's
/// slope at its middle matches a straight ramp that wide.
const FEATHER_SIGMA: f64 = 0.4;

impl Svg<'_> {
    /// `layer`'s masks as a group around what it draws. How many groups it
    /// opened.
    pub(super) fn masks(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        place: &Place,
        out: &mut String,
    ) -> usize {
        let active: Vec<&Mask> = layer
            .masks
            .iter()
            .filter(|m| {
                let closed = m.path_keys.first().map_or(m.path.closed, |k| k.path.closed);
                let off = matches!(m.opacity.animation, Animation::Static(v) if v <= 0.0);
                m.mode != MaskMode::None && closed && !off
            })
            .collect();
        let Some(first) = active.first() else {
            return 0;
        };
        // Masks are drawn in the layer's own pixels, which start at the
        // corner of a Shape layer's art: a group moves them there and one
        // inside moves the art back.
        let corner = self.art_corner(scope, layer);
        let back = corner.clone().map(|_, v| -v);
        let px = [tolerance::PIXELS; 2];
        let open = self.factor(out, "translate", &corner, &px, &pair);
        let still = |p: &lumit_core::anim::Property, v: f64| matches!(p.animation, Animation::Static(x) if x == v);
        let hard = active.iter().all(|m| {
            m.mode == MaskMode::Add
                && !m.inverted
                && still(&m.opacity, 100.0)
                && still(&m.feather, 0.0)
                && still(&m.expansion, 0.0)
                && m.vertex_feather.is_empty()
        });
        if hard {
            let id = self.id("c");
            let mut clip = format!("<clipPath id=\"{id}\">");
            for m in &active {
                let path = self.path_value(scope, layer, &m.path_keys, &m.path, None);
                let _ = write!(
                    clip,
                    "<path d=\"{}\" clip-rule=\"evenodd\">",
                    self.path_base(&path)
                );
                self.path_animation(&mut clip, &path);
                clip.push_str("</path>");
            }
            clip.push_str("</clipPath>");
            self.defs.push_str(&clip);
            let _ = write!(out, "<g clip-path=\"url(#{id})\">");
            return open + 1 + self.factor(out, "translate", &back, &px, &pair);
        }

        let pad = active
            .iter()
            .map(|m| reach(&m.feather) * 2.0 + reach(&m.expansion))
            .fold(0.0, f64::max)
            + 4.0;
        // The art's box in the layer's own pixels, wherever its corner goes.
        let (x0, y0, x1, y1) = self.content_box(scope, layer);
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for t in &self.span.grid {
            let c = corner.at(*t);
            for (d, v) in c.iter().take(2).enumerate() {
                lo[d] = lo[d].min(*v);
                hi[d] = hi[d].max(*v);
            }
        }
        if !lo[0].is_finite() || !lo[1].is_finite() {
            (lo, hi) = ([0.0; 2], [0.0; 2]);
        }
        let (x0, y0, x1, y1) = (x0 - hi[0], y0 - hi[1], x1 - lo[0], y1 - lo[1]);
        let region = (x0 - pad, y0 - pad, x1 - x0 + 2.0 * pad, y1 - y0 + 2.0 * pad);
        let rect = |s: &Svg<'_>, fill: &str| {
            format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{fill}\"/>",
                s.px(region.0),
                s.px(region.1),
                s.px(region.2),
                s.px(region.3)
            )
        };
        let mut acc = if matches!(first.mode, MaskMode::Add | MaskMode::Lighten) {
            String::new()
        } else {
            rect(self, "#fff")
        };
        for m in active {
            // Lumit shows the layer whole where a keyed opacity reaches 0,
            // and a browser hides it.
            if matches!(
                &m.opacity.animation,
                Animation::Keyframed(k) if k.iter().any(|k| k.value <= 0.0)
            ) {
                self.report.changed(
                    &place.part(&m.name),
                    "mask_opacity",
                    String::from("Mask opacity"),
                    Vec::new(),
                );
            }
            if !m.vertex_feather.is_empty() {
                self.report.changed(
                    &place.part(&m.name),
                    "mask_feather",
                    String::from("Mask feather is drawn the same all the way round"),
                    Vec::new(),
                );
            }
            if matches!(
                m.mode,
                MaskMode::Lighten | MaskMode::Darken | MaskMode::Difference
            ) {
                let mode = match m.mode {
                    MaskMode::Lighten => "Lighten",
                    MaskMode::Darken => "Darken",
                    _ => "Difference",
                };
                self.report.changed(
                    &place.part(&m.name),
                    "mask_mode",
                    format!("{mode} masks draw as Add"),
                    vec![arg("mode", mode)],
                );
            }
            let white = self.mask_shape(scope, layer, m, "#fff", region);
            match m.mode {
                MaskMode::Subtract => {
                    acc.push_str(&self.mask_shape(scope, layer, m, "#000", region))
                }
                MaskMode::Intersect | MaskMode::Darken => {
                    let id = self.id("m");
                    let _ = write!(
                        self.defs,
                        "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">{white}</mask>",
                        self.px(region.0),
                        self.px(region.1),
                        self.px(region.2),
                        self.px(region.3)
                    );
                    acc = format!("<g mask=\"url(#{id})\">{acc}</g>");
                }
                _ => acc.push_str(&white),
            }
        }
        let id = self.id("m");
        let _ = write!(
            self.defs,
            "<mask id=\"{id}\" maskUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">{acc}</mask>",
            self.px(region.0),
            self.px(region.1),
            self.px(region.2),
            self.px(region.3)
        );
        let _ = write!(out, "<g mask=\"url(#{id})\">");
        open + 1 + self.factor(out, "translate", &back, &px, &pair)
    }

    /// One mask's shape painted `fill`, at its opacity, inverted, feathered
    /// and expanded as it asks. `region` is the mask's own, x, y, w, h.
    fn mask_shape(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        m: &Mask,
        fill: &str,
        region: (f64, f64, f64, f64),
    ) -> String {
        let path = self.path_value(scope, layer, &m.path_keys, &m.path, None);
        let mut base = self.path_base(&path);
        let mut out = String::new();
        // Inverted: the region with the shape cut out of it, which even-odd
        // gives from one path.
        let frame = format!(
            "M{},{}h{}v{}h{}Z",
            self.px(region.0),
            self.px(region.1),
            self.px(region.2),
            self.px(region.3),
            self.px(-region.2)
        );
        if m.inverted {
            base = format!("{frame}{base}");
        }
        let filter = self.mask_filter(scope, layer, m, region);
        if let Some(f) = &filter {
            let _ = write!(out, "<g filter=\"url(#{f})\">");
        }
        let _ = write!(
            out,
            "<path d=\"{base}\" fill=\"{fill}\" fill-rule=\"evenodd\""
        );
        let opacity = self
            .value(scope, layer, &[&m.opacity], &[tolerance::PERCENT])
            .map(|_, v| v.clamp(0.0, 100.0) / 100.0);
        let unit = |v: &[f64]| anim::num(v.first().copied().unwrap_or(1.0), 4);
        if !matches!(&opacity, Value::Still(v) if v.first().is_some_and(|o| *o >= 1.0)) {
            self.attr(&mut out, "fill-opacity", &opacity, &unit);
        }
        out.push('>');
        self.animate(&mut out, "fill-opacity", &opacity, &[1e-4], &unit);
        if m.inverted {
            // The frame rides along in front of every value, so the commands
            // still match from value to value.
            let framed = match &path {
                PathValue::Curves { value, closed } => {
                    let tol = vec![tolerance::PIXELS; value.at(self.span.start).len()];
                    anim::smil(value, &self.span, &tol).map(|smil| {
                        let mut s = String::new();
                        anim::write(&mut s, "d", None, &smil, &self.span, &|v| {
                            format!("{frame}{}", self.d(v, *closed, true))
                        });
                        s
                    })
                }
                PathValue::Steps(steps) => {
                    let times: Vec<f64> = steps.iter().map(|(t, _)| *t).collect();
                    let values: Vec<String> =
                        steps.iter().map(|(_, d)| format!("{frame}{d}")).collect();
                    let mut s = String::new();
                    anim::write_steps(&mut s, "d", &times, &values, &self.span);
                    Some(s)
                }
            };
            out.push_str(&framed.unwrap_or_default());
        } else {
            self.path_animation(&mut out, &path);
        }
        out.push_str("</path>");
        if filter.is_some() {
            out.push_str("</g>");
        }
        out
    }

    /// The filter a mask's feather and expansion draw with, or `None` when
    /// it has neither.
    fn mask_filter(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        m: &Mask,
        region: (f64, f64, f64, f64),
    ) -> Option<String> {
        let none = |p: &lumit_core::anim::Property| matches!(p.animation, Animation::Static(v) if v == 0.0);
        if none(&m.feather) && none(&m.expansion) {
            return None;
        }
        let id = self.id("f");
        let mut f = format!(
            "<filter id=\"{id}\" filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\">",
            self.px(region.0),
            self.px(region.1),
            self.px(region.2),
            self.px(region.3)
        );
        if !none(&m.expansion) {
            let grow = self.value(scope, layer, &[&m.expansion], &[tolerance::PIXELS]);
            // Shrinking erodes and growing dilates, switching wherever the
            // expansion crosses zero.
            let mut steps: Vec<(f64, String)> = Vec::new();
            for t in &self.span.grid {
                let op = if grow.at(*t).first().copied().unwrap_or(0.0) < 0.0 {
                    "erode"
                } else {
                    "dilate"
                };
                if steps.last().is_none_or(|(_, last)| last != op) {
                    steps.push((*t, op.to_owned()));
                }
            }
            let operator = steps.first().map_or("dilate", |(_, op)| op.as_str());
            let radius = grow.map(|_, v| v.abs());
            f.push_str(&format!("<feMorphology operator=\"{operator}\""));
            self.attr(&mut f, "radius", &radius, &|v| {
                self.px(v.first().copied().unwrap_or(0.0))
            });
            f.push('>');
            self.animate(&mut f, "radius", &radius, &[tolerance::PIXELS], &|v| {
                self.px(v.first().copied().unwrap_or(0.0))
            });
            let (times, ops): (Vec<f64>, Vec<String>) = steps.into_iter().unzip();
            anim::write_steps(&mut f, "operator", &times, &ops, &self.span);
            f.push_str("</feMorphology>");
        }
        if !none(&m.feather) {
            let sigma = self
                .value(scope, layer, &[&m.feather], &[tolerance::PIXELS])
                .map(|_, v| v.max(0.0) * FEATHER_SIGMA);
            f.push_str("<feGaussianBlur");
            self.attr(&mut f, "stdDeviation", &sigma, &|v| {
                self.px(v.first().copied().unwrap_or(0.0))
            });
            f.push('>');
            self.animate(&mut f, "stdDeviation", &sigma, &[tolerance::PIXELS], &|v| {
                self.px(v.first().copied().unwrap_or(0.0))
            });
            f.push_str("</feGaussianBlur>");
        }
        f.push_str("</filter>");
        self.defs.push_str(&f);
        Some(id)
    }

    /// Where `layer` draws, in its own coordinates, left, top, right,
    /// bottom: what a mask region and a filter region have to cover.
    pub(super) fn content_box(&mut self, scope: &Scope<'_>, layer: &Layer) -> (f64, f64, f64, f64) {
        let comp = (
            0.0,
            0.0,
            f64::from(scope.comp.width),
            f64::from(scope.comp.height),
        );
        match &layer.kind {
            LayerKind::Solid { def } => self.doc.solid(*def).map_or(comp, |s| {
                (0.0, 0.0, f64::from(s.width), f64::from(s.height))
            }),
            LayerKind::Precomp { comp: id } => self.doc.comp(*id).map_or(comp, |c| {
                (0.0, 0.0, f64::from(c.width), f64::from(c.height))
            }),
            LayerKind::Footage { item } => {
                let place = scope.place.layer(layer.id, &layer.name);
                match self.image(*item, &place) {
                    Ok(Some((_, w, h))) => (0.0, 0.0, f64::from(w), f64::from(h)),
                    _ => comp,
                }
            }
            LayerKind::Shape { contents } => {
                // The art's box at a spread of frames: art keyed to move
                // takes its box with it.
                let clock = Svg::layer_clock(scope, layer);
                let n = self.span.grid.len();
                let step = (n / 64).max(1);
                let mut b: Option<(f64, f64, f64, f64)> = None;
                for i in (0..n).step_by(step).chain(n.checked_sub(1)) {
                    let lt = clock.at(i, self.span.grid.get(i).copied().unwrap_or(0.0));
                    if let Some(c) = lumit_core::shape::contents_bounds(contents, lt) {
                        b = Some(b.map_or(c, |a| {
                            (a.0.min(c.0), a.1.min(c.1), a.2.max(c.2), a.3.max(c.3))
                        }));
                    }
                }
                b.unwrap_or(comp)
            }
            LayerKind::Text { document } => {
                let laid = lumit_text::layout(
                    &lumit_text::TextBlock::of(document, &document.text),
                    !document.animators.is_empty(),
                );
                let size = document.size;
                (
                    -size,
                    -size,
                    f64::from(laid.width) + size,
                    f64::from(laid.height) + size,
                )
            }
            _ => comp,
        }
    }
}

/// The most a mask number reaches across its keys, for padding a region.
fn reach(p: &lumit_core::anim::Property) -> f64 {
    match &p.animation {
        Animation::Static(v) => v.abs(),
        Animation::Keyframed(keys) => keys.iter().map(|k| k.value.abs()).fold(0.0, f64::max),
        Animation::Expression(_) => 0.0,
    }
}
