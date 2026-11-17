//! A Shape layer's art: each piece of art one `<path>`, painted as Lumit
//! paints it.
//!
//! - **The path** is the item's own: still, morphing between its keys, or
//!   a parametric shape drawn at every frame its numbers move.
//! - **The paint** is the fill and the outline with their own opacities,
//!   round joins and caps as Lumit draws, dashes, and gradients placed in the
//!   art's own pixels. A gradient gets stops in between its own so it mixes
//!   in linear light, as Lumit's does.
//! - **A trim on an outline alone** is the outline's dash, measured with
//!   `pathLength="1"`. A trim that cuts a fill, and every modifier only Lumit
//!   has (twist, deform, smooth, offset, a combine other than Xor), is Lumit's
//!   own outline written as the path: once when nothing moves, at every frame
//!   when something does, or left out when that would pass 2 MB.
//! - **A repeater** is its copies written out, each placed and faded.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::anim::{Animation, Property};
use lumit_core::model::Layer;
use lumit_core::shape::modifier::{self, Lowered, ModifierKind, ShapeEntry, ShapeModifier};
use lumit_core::shape::ShapeItem;

use super::anim::{self, Value};
use super::paths::PathValue;
use super::{Scope, Svg};
use crate::check::{arg, Place};
use crate::fit::tolerance;
use crate::LottieError;

/// The most a frame-by-frame outline may add to the file.
const MAX_BAKE_BYTES: usize = 2 * 1024 * 1024;

impl Svg<'_> {
    /// A Shape layer's contents.
    pub(super) fn shape_layer(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        contents: &[ShapeEntry],
        place: &Place,
        out: &mut String,
    ) -> Result<(), LottieError> {
        self.shape_entries(scope, layer, contents, place, None, out)
    }

    /// `contents` drawn in list order, which is the order Lumit paints them
    /// in. `fill`, when given, is the display colour every fill takes
    /// instead of its own (a tinted letter).
    pub(super) fn shape_entries(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        contents: &[ShapeEntry],
        place: &Place,
        fill: Option<&Value>,
        out: &mut String,
    ) -> Result<(), LottieError> {
        let items = modifier::lowered(contents);
        for run in runs(&items) {
            self.check_cancelled()?;
            let Some(members) = items.get(run) else {
                continue;
            };
            let Some(first) = members.first() else {
                continue;
            };
            let item = first.item;
            if item.fill_shown().is_none() && item.stroke_shown().is_none() {
                continue;
            }
            let at = place.part(&item.name);
            if item.taper.is_some() && item.stroke_shown().is_some() {
                self.report.left_out(
                    &at,
                    "taper",
                    String::from("Stroke taper is left out"),
                    Vec::new(),
                );
            }
            let (repeaters, others): (Vec<&ShapeModifier>, Vec<&ShapeModifier>) = first
                .mods
                .iter()
                .copied()
                .partition(|m| matches!(m.kind, ModifierKind::Repeat(_)));
            let element = self.run_element(scope, layer, members, &others, &at, fill)?;
            if element.is_empty() {
                continue;
            }
            self.repeated(scope, layer, &repeaters, &element, out)?;
            self.nl(out);
        }
        Ok(())
    }

    /// One run's `<path>`, painted and animated.
    fn run_element(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        members: &[Lowered<'_>],
        others: &[&ShapeModifier],
        place: &Place,
        fill: Option<&Value>,
    ) -> Result<String, LottieError> {
        let Some(item) = members.first().map(|m| m.item) else {
            return Ok(String::new());
        };
        let xor = members.len() > 1 && members.iter().skip(1).all(|m| m.item.combine == 4);
        let trims: Vec<&ShapeModifier> = others
            .iter()
            .copied()
            .filter(|m| matches!(m.kind, ModifierKind::Trim(_)))
            .collect();
        let mut extra = String::new();
        let mut animations = String::new();
        let mut evenodd = false;
        let path = if members.len() == 1 && others.is_empty() {
            self.path_value(
                scope,
                layer,
                &item.path_keys,
                &item.path,
                item.primitive.as_ref(),
            )
        } else if xor && others.is_empty() {
            // Xor is even-odd across the contours: exact, curves and all.
            evenodd = true;
            self.joined(scope, layer, members)
        } else if members.len() == 1
            && trims.len() == 1
            && others.len() == 1
            && item.fill_shown().is_none()
            && item.dashes.is_empty()
        {
            if let Some(ModifierKind::Trim(trim)) = trims.first().map(|m| &m.kind) {
                let closed = item
                    .path_keys
                    .first()
                    .map_or(item.path.closed, |k| k.path.closed);
                self.dash_trim(scope, layer, trim, closed, &mut extra, &mut animations);
            }
            self.path_value(
                scope,
                layer,
                &item.path_keys,
                &item.path,
                item.primitive.as_ref(),
            )
        } else {
            for m in others
                .iter()
                .filter(|m| !matches!(m.kind, ModifierKind::Trim(_)))
            {
                let label = m.kind.label();
                self.report.changed(
                    place,
                    "baked_outline",
                    format!("{label} is baked into the path"),
                    vec![arg("label", label)],
                );
            }
            if members.len() > 1 {
                self.report.changed(
                    place,
                    "baked_outline",
                    String::from("Combine is baked into the path"),
                    vec![arg("label", "Combine")],
                );
            }
            match self.baked(scope, layer, members, others)? {
                Some(path) => path,
                None => {
                    for m in others {
                        let label = m.kind.label();
                        self.report.left_out(
                            place,
                            "modifier_animated",
                            format!("{label} is left out"),
                            vec![arg("label", label)],
                        );
                    }
                    self.path_value(
                        scope,
                        layer,
                        &item.path_keys,
                        &item.path,
                        item.primitive.as_ref(),
                    )
                }
            }
        };
        let mut tag = format!("<path d=\"{}\"", self.path_base(&path));
        self.path_animation(&mut animations, &path);
        if evenodd {
            tag.push_str(" fill-rule=\"evenodd\"");
        }
        self.paint(scope, layer, item, fill, &mut tag, &mut animations);
        tag.push_str(&extra);
        if animations.is_empty() {
            tag.push_str("/>");
        } else {
            tag.push('>');
            tag.push_str(&animations);
            tag.push_str("</path>");
        }
        Ok(tag)
    }

    /// The contours of an Xor run as one path.
    fn joined(&self, scope: &Scope<'_>, layer: &Layer, members: &[Lowered<'_>]) -> PathValue {
        let moves = members.iter().any(|m| m.item.path_is_animated());
        let d_at = |lt: f64| -> String {
            members
                .iter()
                .map(|m| {
                    let p = m.item.path_at(lt);
                    self.d(&super::paths::numbers(&p), p.closed, false)
                })
                .collect::<Vec<_>>()
                .concat()
        };
        if !moves {
            return PathValue::Steps(vec![(self.span.start, d_at(0.0))]);
        }
        let clock = Svg::layer_clock(scope, layer);
        let mut steps: Vec<(f64, String)> = Vec::new();
        for (i, t) in self.span.grid.iter().enumerate() {
            let d = d_at(clock.at(i, *t));
            if steps.last().is_none_or(|(_, last)| *last != d) {
                steps.push((*t, d));
            }
        }
        PathValue::Steps(steps)
    }

    /// The fill, the outline and their opacities, onto `tag`, their
    /// animations onto `animations`.
    pub(super) fn paint(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        item: &ShapeItem,
        tint: Option<&Value>,
        tag: &mut String,
        animations: &mut String,
    ) {
        let opacity = self
            .value(scope, layer, &[&item.opacity], &[tolerance::PERCENT])
            .map(|_, v| v.clamp(0.0, 100.0) / 100.0);
        let hex = |v: &[f64]| {
            crate::colour::hex([
                v.first().copied().unwrap_or(0.0),
                v.get(1).copied().unwrap_or(0.0),
                v.get(2).copied().unwrap_or(0.0),
            ])
        };
        match item.fill_shown() {
            None => tag.push_str(" fill=\"none\""),
            Some(_) if item.gradient != 0 => {
                let id = self.gradient(scope, layer, item);
                let _ = write!(tag, " fill=\"url(#{id})\"");
                self.opacity_attr("fill-opacity", &opacity, tag, animations);
            }
            Some(ch) => {
                let colour = match tint {
                    Some(t) => t.clone(),
                    None => self.colour(scope, layer, [&ch[0], &ch[1], &ch[2]]),
                };
                self.attr(tag, "fill", &colour, &hex);
                self.animate(animations, "fill", &colour, &[tolerance::CHANNEL; 3], &hex);
                let alpha = self.times(scope, layer, &ch[3], &opacity);
                self.opacity_attr("fill-opacity", &alpha, tag, animations);
            }
        }
        let Some(ch) = item.stroke_shown() else {
            return;
        };
        let width = self.value(scope, layer, &[&item.stroke_width], &[tolerance::PIXELS]);
        if matches!(&width, Value::Still(w) if w.first().is_none_or(|w| *w <= 0.0)) {
            return;
        }
        let colour = self.colour(scope, layer, [&ch[0], &ch[1], &ch[2]]);
        self.attr(tag, "stroke", &colour, &hex);
        self.animate(
            animations,
            "stroke",
            &colour,
            &[tolerance::CHANNEL; 3],
            &hex,
        );
        let px = |v: &[f64]| self.px(v.first().copied().unwrap_or(0.0).max(0.0));
        self.attr(tag, "stroke-width", &width, &px);
        self.animate(
            animations,
            "stroke-width",
            &width,
            &[tolerance::PIXELS],
            &px,
        );
        let alpha = self.times(scope, layer, &ch[3], &opacity);
        self.opacity_attr("stroke-opacity", &alpha, tag, animations);
        tag.push_str(" stroke-linejoin=\"round\" stroke-linecap=\"round\"");
        if !item.dashes.is_empty() {
            let mut props: Vec<&Property> = item.dashes.iter().collect();
            if props.len() % 2 == 1 {
                props.extend(item.dashes.iter());
            }
            let pattern = self.value(scope, layer, &props, &vec![tolerance::PIXELS; props.len()]);
            let list = |v: &[f64]| {
                v.iter()
                    .map(|d| self.px(d.max(0.0)))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let n = props.len();
            self.attr(tag, "stroke-dasharray", &pattern, &list);
            self.animate(
                animations,
                "stroke-dasharray",
                &pattern,
                &vec![tolerance::PIXELS; n],
                &list,
            );
            let offset = self.value(scope, layer, &[&item.dash_offset], &[tolerance::PIXELS]);
            if !matches!(&offset, Value::Still(v) if v.first().is_some_and(|o| *o == 0.0)) {
                let px = |v: &[f64]| self.px(v.first().copied().unwrap_or(0.0));
                self.attr(tag, "stroke-dashoffset", &offset, &px);
                self.animate(
                    animations,
                    "stroke-dashoffset",
                    &offset,
                    &[tolerance::PIXELS],
                    &px,
                );
            }
        }
    }

    /// A colour's alpha channel, 0 to 1, times `opacity`.
    fn times(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        alpha: &Property,
        opacity: &Value,
    ) -> Value {
        let alpha = self.value(scope, layer, &[alpha], &[1e-4]);
        match (&alpha, opacity) {
            (Value::Still(a), Value::Still(o)) => Value::Still(vec![
                a.first().copied().unwrap_or(1.0).clamp(0.0, 1.0)
                    * o.first().copied().unwrap_or(1.0),
            ]),
            _ => {
                let samples = self
                    .span
                    .grid
                    .iter()
                    .map(|t| {
                        let a = alpha.at(*t).first().copied().unwrap_or(1.0).clamp(0.0, 1.0);
                        vec![a * opacity.at(*t).first().copied().unwrap_or(1.0)]
                    })
                    .collect();
                Value::sampled(&self.span.grid, samples, &[1e-4])
            }
        }
    }

    /// An opacity attribute, left off while it is a still 1.
    fn opacity_attr(&self, attr: &str, value: &Value, tag: &mut String, animations: &mut String) {
        if matches!(value, Value::Still(v) if v.first().is_some_and(|o| *o >= 1.0)) {
            return;
        }
        let unit = |v: &[f64]| anim::num(v.first().copied().unwrap_or(1.0).clamp(0.0, 1.0), 4);
        self.attr(tag, attr, value, &unit);
        self.animate(animations, attr, value, &[1e-4], &unit);
    }

    /// A trim on an outline alone as the outline's one dash: `pathLength`
    /// measures the path as 1, the dash is the trimmed length, and the gap
    /// the rest. A closed path's pattern repeats once round, so the dash
    /// wraps through the seam; an open path's gap is longer than the path so
    /// nothing wraps. A trim that leaves nothing hides the path, since a round
    /// cap draws a dot even for a dash of no length.
    fn dash_trim(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        trim: &modifier::Trim,
        closed: bool,
        extra: &mut String,
        animations: &mut String,
    ) {
        let numbers = self.value(
            scope,
            layer,
            &[&trim.start, &trim.end, &trim.offset],
            &[tolerance::PERCENT, tolerance::PERCENT, tolerance::DEGREES],
        );
        let dash = |v: &[f64]| -> (Vec<f64>, bool) {
            let get = |i: usize| v.get(i).copied().unwrap_or(0.0);
            let (s, e, o) = (
                get(0).clamp(0.0, 100.0) / 100.0,
                get(1).clamp(0.0, 100.0) / 100.0,
                get(2) / 360.0,
            );
            if closed {
                let len = (e - s).clamp(0.0, 1.0);
                (vec![len, 1.0 - len, -(s + o)], len > 0.0)
            } else {
                let (a, b) = ((s + o).clamp(0.0, 1.0), (e + o).clamp(0.0, 1.0));
                ((vec![(b - a).max(0.0), 2.0, -a]), b > a)
            }
        };
        let (value, shown) = match &numbers {
            Value::Still(v) => {
                let (d, s) = dash(v);
                (Value::Still(d), vec![(self.span.start, s)])
            }
            moving => {
                let mut shown: Vec<(f64, bool)> = Vec::new();
                let mut samples = Vec::with_capacity(self.span.grid.len());
                for t in &self.span.grid {
                    let (d, s) = dash(&moving.at(*t));
                    if shown.last().is_none_or(|(_, was)| *was != s) {
                        shown.push((*t, s));
                    }
                    samples.push(d);
                }
                (Value::sampled(&self.span.grid, samples, &[1e-4; 3]), shown)
            }
        };
        let array = |v: &[f64]| {
            format!(
                "{},{}",
                anim::num(v.first().copied().unwrap_or(0.0), 5),
                anim::num(v.get(1).copied().unwrap_or(1.0), 5)
            )
        };
        let offset = |v: &[f64]| anim::num(v.get(2).copied().unwrap_or(0.0), 5);
        extra.push_str(" pathLength=\"1\"");
        self.attr(extra, "stroke-dasharray", &value, &array);
        self.attr(extra, "stroke-dashoffset", &value, &offset);
        self.animate(animations, "stroke-dasharray", &value, &[1e-4; 3], &array);
        self.animate(animations, "stroke-dashoffset", &value, &[1e-4; 3], &offset);
        let word = |s: bool| if s { "visible" } else { "hidden" };
        if shown.first().is_some_and(|(_, s)| !s) {
            let _ = write!(extra, " visibility=\"hidden\"");
        }
        let times: Vec<f64> = shown.iter().map(|(t, _)| *t).collect();
        let values: Vec<String> = shown.iter().map(|(_, s)| word(*s).to_owned()).collect();
        anim::write_steps(animations, "visibility", &times, &values, &self.span);
    }

    /// The run drawn through `others` as Lumit's own outline: once when
    /// nothing moves, at every frame when something does. `None` when that
    /// would pass [`MAX_BAKE_BYTES`].
    fn baked(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        members: &[Lowered<'_>],
        others: &[&ShapeModifier],
    ) -> Result<Option<PathValue>, LottieError> {
        // The run's geometry alone: its paths and the modifiers on them, the
        // paint taken off so a keyed colour doesn't make a still outline
        // move.
        let mut sub: Vec<ShapeEntry> = members
            .iter()
            .map(|m| {
                let mut bare = ShapeItem::filled(
                    m.item.name.clone(),
                    m.item.path.clone(),
                    lumit_core::model::LinearColour::BLACK,
                );
                bare.fill = None;
                bare.path_keys = m.item.path_keys.clone();
                bare.primitive = m.item.primitive.clone();
                bare.combine = m.item.combine;
                bare.id = m.item.id;
                ShapeEntry::Item(bare)
            })
            .collect();
        sub.extend(others.iter().map(|m| {
            let mut m = (*m).clone();
            m.own = false;
            m.hidden = false;
            ShapeEntry::Modifier(m)
        }));
        let moves = serde_json::to_string(&sub)
            .map(|s| {
                s.contains("\"Keyframed\"")
                    || s.contains("\"Expression\"")
                    || s.contains("\"path_keys\"")
            })
            .unwrap_or(true);
        let clock = Svg::layer_clock(scope, layer);
        let outline = |lt: f64, frozen: &[ShapeEntry]| -> String {
            let line = lumit_core::shape::outline_at(frozen, lt);
            let (x0, y0) =
                lumit_core::shape::contents_bounds(frozen, lt).map_or((0.0, 0.0), |b| (b.0, b.1));
            self.polyline_d(&lines(&line, x0, y0), false)
        };
        if !moves {
            return Ok(Some(PathValue::Steps(vec![(
                self.span.start,
                outline(0.0, &sub),
            )])));
        }
        let mut steps: Vec<(f64, String)> = Vec::new();
        let mut bytes = 0usize;
        for (i, t) in self.span.grid.iter().enumerate() {
            self.check_cancelled()?;
            let comp_t = scope.clock.at(i, *t);
            let lt = clock.at(i, *t);
            let context = self.context(scope, layer, comp_t);
            let frozen = lumit_core::shape::contents_at(&sub, lt, &context);
            let d = outline(lt, &frozen);
            if steps.last().is_none_or(|(_, last)| *last != d) {
                bytes += d.len();
                if bytes > MAX_BAKE_BYTES {
                    return Ok(None);
                }
                steps.push((*t, d));
            }
        }
        Ok(Some(PathValue::Steps(steps)))
    }

    /// `element` drawn once per copy `repeaters` make, the last copy first so
    /// the original ends on top, as Lumit draws them.
    fn repeated(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        repeaters: &[&ShapeModifier],
        element: &str,
        out: &mut String,
    ) -> Result<(), LottieError> {
        if repeaters.is_empty() {
            out.push_str(element);
            return Ok(());
        }
        let moves = serde_json::to_string(repeaters)
            .map(|s| s.contains("\"Keyframed\"") || s.contains("\"Expression\""))
            .unwrap_or(true);
        let id = self.id("p");
        let _ = write!(self.defs, "<g id=\"{id}\">{element}</g>");
        if !moves {
            for (m, ramp) in modifier::copies_at(repeaters, 0.0).into_iter().rev() {
                let a = m.0;
                let _ = write!(
                    out,
                    "<use href=\"#{id}\" transform=\"matrix({},{},{},{},{},{})\"",
                    self.unit(a[0]),
                    self.unit(a[1]),
                    self.unit(a[2]),
                    self.unit(a[3]),
                    self.px(a[4]),
                    self.px(a[5])
                );
                if ramp < 1.0 {
                    let _ = write!(out, " opacity=\"{}\"", self.unit(ramp.clamp(0.0, 1.0)));
                }
                out.push_str("/>");
            }
            return Ok(());
        }
        // Copies that move: each one's place, turn, size and fade sampled at
        // every frame. A copy beyond the count at some frame is hidden there.
        let entries: Vec<ShapeEntry> = repeaters
            .iter()
            .map(|m| ShapeEntry::Modifier((*m).clone()))
            .collect();
        let clock = Svg::layer_clock(scope, layer);
        let frames: Vec<Vec<(lumit_core::shape::geometry::Affine, f64)>> = self
            .span
            .grid
            .iter()
            .enumerate()
            .map(|(i, t)| {
                self.check_cancelled()?;
                let lt = clock.at(i, *t);
                let context = self.context(scope, layer, scope.clock.at(i, *t));
                let frozen = lumit_core::shape::contents_at(&entries, lt, &context);
                let mods: Vec<&ShapeModifier> = frozen
                    .iter()
                    .filter_map(|e| match e {
                        ShapeEntry::Modifier(m) => Some(m),
                        _ => None,
                    })
                    .collect();
                Ok(modifier::copies_at(&mods, lt))
            })
            .collect::<Result<_, LottieError>>()?;
        let most = frames.iter().map(Vec::len).max().unwrap_or(0);
        for k in (0..most).rev() {
            self.check_cancelled()?;
            let mut turn = 0.0f64;
            let mut samples = Vec::with_capacity(frames.len());
            let mut shown: Vec<(f64, bool)> = Vec::new();
            for (t, copies) in self.span.grid.iter().zip(&frames) {
                let here = copies.get(k);
                let (m, ramp) = here
                    .copied()
                    .unwrap_or((lumit_core::shape::geometry::Affine::IDENTITY, 0.0));
                let a = m.0;
                // Every copy is a turn and an even scale, so it splits into
                // those and a move; the turn is kept continuous.
                let mut r = a[1].atan2(a[0]).to_degrees();
                r += 360.0 * ((turn - r) / 360.0).round();
                turn = r;
                samples.push(vec![a[4], a[5], r, a[0].hypot(a[1]), ramp]);
                let now = here.is_some();
                if shown.last().is_none_or(|(_, was)| *was != now) {
                    shown.push((*t, now));
                }
            }
            if shown.iter().all(|(_, s)| !s) {
                continue;
            }
            let value = Value::sampled(
                &self.span.grid,
                samples,
                &[
                    tolerance::PIXELS,
                    tolerance::PIXELS,
                    tolerance::DEGREES,
                    1e-4,
                    1e-4,
                ],
            );
            let pick = |range: std::ops::Range<usize>| -> Value { pick(&value, range) };
            let (place, rotate, scale, fade) = (pick(0..2), pick(2..3), pick(3..4), pick(4..5));
            out.push_str("<g");
            let mut animations = String::new();
            let unit = |v: &[f64]| anim::num(v.first().copied().unwrap_or(1.0).clamp(0.0, 1.0), 4);
            self.attr(out, "opacity", &fade, &unit);
            self.animate(&mut animations, "opacity", &fade, &[1e-4], &unit);
            if shown.len() > 1 || shown.first().is_some_and(|(_, s)| !s) {
                let word = |s: bool| if s { "inline" } else { "none" };
                let _ = write!(
                    out,
                    " display=\"{}\"",
                    word(shown.first().is_some_and(|(_, s)| *s))
                );
                let times: Vec<f64> = shown.iter().map(|(t, _)| *t).collect();
                let values: Vec<String> = shown.iter().map(|(_, s)| word(*s).to_owned()).collect();
                anim::write_steps(&mut animations, "display", &times, &values, &self.span);
            }
            out.push('>');
            out.push_str(&animations);
            let xy = |v: &[f64]| {
                format!(
                    "{},{}",
                    self.px(v.first().copied().unwrap_or(0.0)),
                    self.px(v.get(1).copied().unwrap_or(0.0))
                )
            };
            let deg = |v: &[f64]| anim::num(v.first().copied().unwrap_or(0.0), 3);
            let factor = |v: &[f64]| self.unit(v.first().copied().unwrap_or(1.0));
            for (kind, value, tol, fmt) in [
                (
                    "translate",
                    &place,
                    &[tolerance::PIXELS; 2][..],
                    &xy as &dyn Fn(&[f64]) -> String,
                ),
                ("rotate", &rotate, &[tolerance::DEGREES][..], &deg),
                ("scale", &scale, &[1e-4][..], &factor),
            ] {
                let _ = write!(
                    out,
                    "<g transform=\"{kind}({})\">",
                    fmt(&value.at(self.span.start))
                );
                if let Some(smil) = anim::smil(value, &self.span, tol) {
                    anim::write(out, "transform", Some(kind), &smil, &self.span, fmt);
                }
            }
            let _ = write!(out, "<use href=\"#{id}\"/></g></g></g></g>");
        }
        Ok(())
    }
}

/// The numbers `range` of a value, as a value of their own.
pub(super) fn pick(value: &Value, range: std::ops::Range<usize>) -> Value {
    let cut = |v: &[f64]| {
        v.get(range.clone())
            .map(<[f64]>::to_vec)
            .unwrap_or_default()
    };
    match value {
        Value::Still(v) => Value::Still(cut(v)),
        Value::Moving(segs) => Value::Moving(
            segs.iter()
                .map(|s| anim::Seg {
                    t0: s.t0,
                    t1: s.t1,
                    v0: cut(&s.v0),
                    v1: cut(&s.v1),
                    ease: match &s.ease {
                        anim::Ease::Hold => anim::Ease::Hold,
                        anim::Ease::Curve(c) if c.len() <= 1 => anim::Ease::Curve(c.clone()),
                        anim::Ease::Curve(c) => anim::Ease::Curve(
                            c.get(range.clone())
                                .map(<[[f64; 4]]>::to_vec)
                                .unwrap_or_default(),
                        ),
                    },
                })
                .collect(),
        ),
    }
}

/// True for a property that never moves.
pub(super) fn still(p: &Property) -> bool {
    matches!(p.animation, Animation::Static(_))
}

/// The runs a layer's art draws as: an item with no combine starts one, and
/// each item after it that combines and sits right under an item joins it.
/// The same reading as the rasteriser's own.
fn runs(items: &[Lowered<'_>]) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, art) in items.iter().enumerate() {
        match out.last_mut() {
            Some(run) if art.joins => run.end = i + 1,
            _ => out.push(i..i + 1),
        }
    }
    out
}

/// The separate lines of an outline that [`lumit_core::shape::outline_at`]
/// handed back as one, moved back from the art's box into the art's own
/// pixels by (`x0`, `y0`). A step from one line to the next has no length,
/// which is how the joins are told from the lines.
fn lines(poly: &lumit_core::mask::MaskPolyline, x0: f64, y0: f64) -> Vec<Vec<(f64, f64)>> {
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    for (i, p) in poly.points.iter().enumerate() {
        let here = (f64::from(p[0]) + x0, f64::from(p[1]) + y0);
        if i > 0 {
            let jump = poly.arc.get(i) == poly.arc.get(i - 1)
                && poly.points.get(i - 1).is_some_and(|q| q != p);
            if jump && !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        }
        current.push(here);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}
