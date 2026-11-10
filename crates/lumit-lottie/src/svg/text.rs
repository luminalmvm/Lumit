//! Text as the letters' outlines, which every browser draws the same and no
//! font has to be installed for.
//!
//! The outlines are the ones Lumit converts a Text layer to shapes with
//! ([`lumit_text::shape_items`]), a group per letter, so they sit exactly on
//! the ink the Viewer draws. Animators on a straight line move each letter
//! as Lumit's rasteriser does: pushed, turned and scaled about the middle of
//! its advance on the baseline, faded and tinted, sampled at every frame.
//! Words an expression writes are a set of letters per distinct line, each
//! shown while it is the one on screen. Text whose path or offset moves is
//! drawn frame by frame.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::anim::Animation;
use lumit_core::model::{Layer, TextDocument};
use lumit_core::shape::modifier::ShapeEntry;
use lumit_core::shape::ShapeItem;

use super::anim::{self, Value};
use super::{Scope, Svg};
use crate::check::Place;
use crate::fit::tolerance;
use crate::LottieError;

impl Svg<'_> {
    /// A Text layer's letters.
    pub(super) fn text_layer(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        document: &TextDocument,
        place: &Place,
        out: &mut String,
    ) -> Result<(), LottieError> {
        let clock = Svg::layer_clock(scope, layer);
        // The words at every frame, through the one resolver the renderer
        // reads, when an expression writes them.
        let words: Vec<String> = match &document.expression {
            None => vec![document.text.clone()],
            Some(_) => self
                .span
                .grid
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    self.check_cancelled()?;
                    let context = self.context(scope, layer, scope.clock.at(i, *t));
                    Ok(document.resolved_text(context).into_owned())
                })
                .collect::<Result<_, LottieError>>()?,
        };
        let mut distinct: Vec<&String> = Vec::new();
        for w in &words {
            self.check_cancelled()?;
            if !distinct.contains(&w) {
                distinct.push(w);
            }
        }
        let on_path = document
            .path
            .is_some_and(|id| layer.masks.iter().any(|m| m.id == id));
        let path_moves = on_path
            && (!matches!(document.path_offset.animation, Animation::Static(_))
                || layer
                    .masks
                    .iter()
                    .any(|m| Some(m.id) == document.path && m.path_is_animated()));
        if !document.animators.is_empty() && (on_path || !plain(document)) {
            self.report.left_out(
                place,
                "text_animators",
                String::from("Text animators"),
                Vec::new(),
            );
        }
        if path_moves {
            return self.text_frames(scope, layer, document, &words, out);
        }
        for text in distinct {
            self.check_cancelled()?;
            let shown: Vec<(f64, bool)> = if words.len() == 1 {
                vec![(self.span.start, true)]
            } else {
                let mut runs: Vec<(f64, bool)> = Vec::new();
                for (t, w) in self.span.grid.iter().zip(&words) {
                    let now = w == text;
                    if runs.last().is_none_or(|(_, was)| *was != now) {
                        runs.push((*t, now));
                    }
                }
                runs
            };
            let steps = shown.len() > 1 || shown.first().is_some_and(|(_, s)| !s);
            if steps {
                let word = |s: bool| if s { "inline" } else { "none" };
                let _ = write!(
                    out,
                    "<g display=\"{}\">",
                    word(shown.first().is_some_and(|(_, s)| *s))
                );
                let times: Vec<f64> = shown.iter().map(|(t, _)| *t).collect();
                let values: Vec<String> = shown.iter().map(|(_, s)| word(*s).to_owned()).collect();
                anim::write_steps(out, "display", &times, &values, &self.span);
            }
            let lt0 = clock.at(0, self.span.start);
            let spine = document
                .path
                .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt0))
                .filter(|p| !p.is_empty());
            let block = lumit_text::TextBlock::of(document, text);
            let glyphs = lumit_text::shape_items(
                &block,
                spine.as_ref(),
                document.path_offset.value_at(lt0) as f32,
            );
            let animated = !document.animators.is_empty() && spine.is_none() && plain(document);
            if animated {
                self.animated_glyphs(scope, layer, document, text, &glyphs, place, out)?;
            } else {
                for glyph in &glyphs {
                    if let ShapeEntry::Group(g) = glyph {
                        self.shape_entries(scope, layer, &g.entries, place, None, out)?;
                    }
                }
            }
            if steps {
                out.push_str("</g>");
            }
        }
        Ok(())
    }

    /// Letters on a straight line with their animators: each one's own move,
    /// turn, size, fade and tint, sampled at every frame.
    #[allow(clippy::too_many_arguments)]
    fn animated_glyphs(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        document: &TextDocument,
        text: &str,
        glyphs: &[ShapeEntry],
        place: &Place,
        out: &mut String,
    ) -> Result<(), LottieError> {
        let block = lumit_text::TextBlock::of(document, text);
        let laid = lumit_text::layout(&block, true);
        let Some(line) = laid.lines.first() else {
            return Ok(());
        };
        let margin = lumit_text::animator_margin(document.size as f32);
        let chars: Vec<char> = text.chars().collect();
        let clock = Svg::layer_clock(scope, layer);
        let frames: Vec<Vec<lumit_core::text::GlyphXform>> = self
            .span
            .grid
            .iter()
            .enumerate()
            .map(|(i, t)| {
                lumit_core::text::glyph_xforms(&document.animators, text, clock.at(i, *t), &[])
            })
            .collect();
        let fill = document.fill.0.map(f64::from);
        // The outlines are laid out as if nothing moved; an animated line
        // sits a margin in from the layer's edge.
        let _ = write!(
            out,
            "<g transform=\"translate({},{})\">",
            self.px(f64::from(margin)),
            self.px(f64::from(margin))
        );
        let mut index = 0usize;
        for glyph in glyphs {
            let ShapeEntry::Group(g) = glyph else {
                continue;
            };
            // The letter this group draws: the next character with its name.
            let ch = g.name.chars().next();
            while index < chars.len() && chars.get(index).copied() != ch {
                index += 1;
            }
            let i = index;
            index += 1;
            let (Some(a), Some(b)) = (line.carets.get(i), line.carets.get(i + 1)) else {
                self.shape_entries(scope, layer, &g.entries, place, None, out)?;
                continue;
            };
            // The middle of its advance on the baseline, a margin less since
            // the group above puts the margin back.
            let pivot = (
                f64::from((a + b) * 0.5 - margin),
                f64::from(line.baseline - margin),
            );
            let samples: Vec<Vec<f64>> = frames
                .iter()
                .map(|xs| {
                    let x = xs.get(i).copied().unwrap_or_default();
                    let tint = self.host.display([
                        fill[0] + f64::from(x.fill[0]),
                        fill[1] + f64::from(x.fill[1]),
                        fill[2] + f64::from(x.fill[2]),
                    ]);
                    vec![
                        f64::from(x.position[0]),
                        f64::from(x.position[1]),
                        f64::from(x.rotation),
                        f64::from(x.scale[0]),
                        f64::from(x.scale[1]),
                        f64::from(x.opacity).clamp(0.0, 1.0),
                        tint[0],
                        tint[1],
                        tint[2],
                    ]
                })
                .collect();
            let tol = [
                tolerance::PIXELS,
                tolerance::PIXELS,
                tolerance::DEGREES,
                1e-4,
                1e-4,
                1e-4,
                tolerance::CHANNEL,
                tolerance::CHANNEL,
                tolerance::CHANNEL,
            ];
            let value = Value::sampled(&self.span.grid, samples, &tol);
            let pick = |r: std::ops::Range<usize>| super::shapes::pick(&value, r);
            let (moves, turns, sizes, fades) = (pick(0..2), pick(2..3), pick(3..5), pick(5..6));
            let unit = |v: &[f64]| anim::num(v.first().copied().unwrap_or(1.0), 4);
            out.push_str("<g");
            self.attr(out, "opacity", &fades, &unit);
            out.push('>');
            self.animate(out, "opacity", &fades, &[1e-4], &unit);
            let moved = |v: &[f64]| {
                format!(
                    "{},{}",
                    self.px(pivot.0 + v.first().copied().unwrap_or(0.0)),
                    self.px(pivot.1 + v.get(1).copied().unwrap_or(0.0))
                )
            };
            let turn = |v: &[f64]| anim::num(v.first().copied().unwrap_or(0.0), 3);
            let size = |v: &[f64]| {
                format!(
                    "{},{}",
                    self.unit(v.first().copied().unwrap_or(1.0)),
                    self.unit(v.get(1).copied().unwrap_or(1.0))
                )
            };
            for (kind, part, tol, fmt) in [
                (
                    "translate",
                    &moves,
                    &[tolerance::PIXELS; 2][..],
                    &moved as &dyn Fn(&[f64]) -> String,
                ),
                ("rotate", &turns, &[tolerance::DEGREES][..], &turn),
                ("scale", &sizes, &[1e-4; 2][..], &size),
            ] {
                let _ = write!(
                    out,
                    "<g transform=\"{kind}({})\">",
                    fmt(&part.at(self.span.start))
                );
                if let Some(smil) = anim::smil(part, &self.span, tol) {
                    anim::write(out, "transform", Some(kind), &smil, &self.span, fmt);
                }
            }
            let _ = write!(
                out,
                "<g transform=\"translate({},{})\">",
                self.px(-pivot.0),
                self.px(-pivot.1)
            );
            let tint = pick(6..9);
            self.shape_entries(scope, layer, &g.entries, place, Some(&tint), out)?;
            out.push_str("</g></g></g></g></g>");
        }
        out.push_str("</g>");
        Ok(())
    }

    /// Text whose path or place along it moves, drawn frame by frame: each
    /// letter's fill and outline a path of its own, filled even-odd, so
    /// letters that overlap join as Lumit draws them.
    fn text_frames(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        document: &TextDocument,
        words: &[String],
        out: &mut String,
    ) -> Result<(), LottieError> {
        let clock = Svg::layer_clock(scope, layer);
        // Each path's paint and its shape at every frame. A letter missing
        // from a frame draws nothing there.
        let mut paths: Vec<(ShapeItem, Vec<(f64, String)>)> = Vec::new();
        for (i, t) in self.span.grid.iter().enumerate() {
            self.check_cancelled()?;
            let lt = clock.at(i, *t);
            let text = words
                .get(i)
                .or_else(|| words.first())
                .cloned()
                .unwrap_or_default();
            let spine = document
                .path
                .map(|id| lumit_core::mask::mask_path_at(&layer.masks, Some(id), false, lt))
                .filter(|p| !p.is_empty());
            let block = lumit_text::TextBlock::of(document, &text);
            let glyphs = lumit_text::shape_items(
                &block,
                spine.as_ref(),
                document.path_offset.value_at(lt) as f32,
            );
            // A letter's contours after its first are its holes.
            let mut runs: Vec<(&ShapeItem, String)> = Vec::new();
            for item in lumit_core::shape::modifier::items(&glyphs) {
                let d = self.d(&super::paths::numbers(&item.path), item.path.closed, false);
                match runs.last_mut() {
                    Some((_, run)) if item.combine != 0 => run.push_str(&d),
                    _ => runs.push((item, d)),
                }
            }
            let count = runs.len();
            for (n, (item, d)) in runs.into_iter().enumerate() {
                match paths.get_mut(n) {
                    Some((_, steps)) => {
                        if steps.last().is_none_or(|(_, last)| *last != d) {
                            steps.push((*t, d));
                        }
                    }
                    None => {
                        let mut steps = Vec::new();
                        if i > 0 {
                            steps.push((self.span.start, String::new()));
                        }
                        steps.push((*t, d));
                        paths.push((item.clone(), steps));
                    }
                }
            }
            for (_, steps) in paths.iter_mut().skip(count) {
                if steps.last().is_some_and(|(_, last)| !last.is_empty()) {
                    steps.push((*t, String::new()));
                }
            }
        }
        for (item, steps) in paths {
            let base = steps.first().map(|(_, d)| d.clone()).unwrap_or_default();
            let mut tag = format!("<path d=\"{base}\" fill-rule=\"evenodd\"");
            let mut animations = String::new();
            self.paint(scope, layer, &item, None, &mut tag, &mut animations);
            let times: Vec<f64> = steps.iter().map(|(t, _)| *t).collect();
            let values: Vec<String> = steps.into_iter().map(|(_, d)| d).collect();
            anim::write_steps(&mut animations, "d", &times, &values, &self.span);
            let _ = write!(out, "{tag}>{animations}</path>");
        }
        Ok(())
    }
}

/// One unstyled line, which the animators' straight layout covers.
fn plain(document: &TextDocument) -> bool {
    document.style.is_default()
        && document.paragraph.is_default()
        && !document.text.contains(['\n', '\r'])
}
