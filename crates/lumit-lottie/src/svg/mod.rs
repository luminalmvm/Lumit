//! The SVG emitter: a comp as an SVG that animates by itself in a browser.
//!
//! # In plain terms
//!
//! The file is the comp drawn once, as nested groups, with SMIL animations
//! moving the numbers that change ([`anim`]). Nothing in it needs a script or
//! a player: it plays in an `<img>`, a page or a browser tab, and a viewer
//! that doesn't play SMIL shows the first frame. Every animated attribute's
//! own value is its value at that first frame for this reason.
//!
//! A layer is a stack of groups, outermost first: its matte, opacity, blend
//! and in and out; its parents' transforms; its own position, rotation, scale
//! and anchor, one group each; its effects; its masks; then what it draws
//! ([`layers`], [`shapes`], [`text`]). A precomp is drawn inline with its
//! animations moved onto the file's clock.
//!
//! Runs on the export's thread, which is a bridge worker or the command
//! line's, never the UI thread.

use std::fmt::Write;
use std::sync::Arc;

use lumit_core::anim::{Animation, Property};
use lumit_core::expression::ExpressionContext;
use lumit_core::model::{Composition, Document, Layer};
use uuid::Uuid;

use crate::check::{Place, Players, Report};
use crate::keys;
use crate::settings::Settings;
use crate::time::Timebase;
use crate::{Exported, Host, LottieError};

pub mod anim;
mod effects;
mod gradient;
mod layers;
mod masks;
mod paths;
mod shapes;
mod text;

use anim::{Clock, Span, Value};

/// `comp` as an animated SVG.
pub fn export(
    doc: &Arc<Document>,
    comp: &Composition,
    settings: &Settings,
    host: &dyn Host,
) -> Result<Exported, LottieError> {
    let tb = Timebase::of(comp, settings);
    let grid: Vec<f64> = tb
        .grid(tb.start, tb.end)
        .into_iter()
        .map(|f| tb.seconds(f))
        .collect();
    let mut svg = Svg {
        doc,
        settings,
        host,
        span: Span {
            start: tb.start,
            end: tb.end,
            grid,
            looping: settings.loop_playback,
        },
        // An SVG plays in browsers, so the check judges against the ticked
        // browsers, or all three when none is ticked.
        report: Report::new(match settings.players.and(Players::BROWSERS) {
            p if p.is_empty() => Players::BROWSERS,
            p => p,
        }),
        defs: String::new(),
        next_id: 0,
        images: Vec::new(),
        beside: Vec::new(),
        blends: false,
        depth: 0,
    };
    let scope = Scope {
        comp,
        clock: Clock::Shift(0.0),
        place: Place::comp(comp.id, &comp.name),
    };
    let mut body = String::new();
    svg.comp_layers(&scope, &mut body)?;
    let (w, h) = (comp.width, comp.height);
    let mut out = String::with_capacity(body.len() + svg.defs.len() + 256);
    let _ = write!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\""
    );
    if svg.blends {
        // A blend mixes only with what this file drew, as it does in Lumit,
        // never with the page behind it.
        out.push_str(" style=\"isolation:isolate\"");
    }
    out.push('>');
    svg.nl(&mut out);
    if !svg.defs.is_empty() {
        out.push_str("<defs>");
        out.push_str(&svg.defs);
        out.push_str("</defs>");
        svg.nl(&mut out);
    }
    out.push_str(&body);
    out.push_str("</svg>");
    svg.nl(&mut out);
    Ok(Exported {
        bytes: out.into_bytes(),
        beside: svg.beside,
        issues: svg.report.finish(),
        frames: tb.frame_count(),
        fps: tb.fps,
        width: w,
        height: h,
    })
}

/// The export as it goes: what it reads and what it has written so far.
pub(crate) struct Svg<'a> {
    pub doc: &'a Arc<Document>,
    pub settings: &'a Settings,
    pub host: &'a dyn Host,
    pub span: Span,
    pub report: Report,
    /// Masks, clips, gradients, filters and pictures, referenced by id.
    pub defs: String,
    next_id: u32,
    /// Each footage item's picture, written once and used by id.
    pub images: Vec<(Uuid, Option<Image>)>,
    pub beside: Vec<(String, Vec<u8>)>,
    /// True once any layer blends, so the root keeps blends to itself.
    pub blends: bool,
    /// How many precomps deep the walk is, to stop a cycle.
    pub depth: u32,
}

/// A picture in the defs: its id, width and height.
pub(crate) type Image = (String, u32, u32);

/// One comp as the walk draws it: which comp, how the file's clock reads on
/// it, and where it is for the check.
#[derive(Debug, Clone)]
pub(crate) struct Scope<'c> {
    pub comp: &'c Composition,
    pub clock: Clock,
    pub place: Place,
}

/// The most precomps inside each other the export follows.
const MAX_DEPTH: u32 = 32;

impl<'a> Svg<'a> {
    /// A fresh id with `prefix`, the same every export of the same comp.
    pub fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    /// A line break between elements, unless minified.
    pub fn nl(&self, out: &mut String) {
        if !self.settings.minify {
            out.push('\n');
        }
    }

    /// A coordinate, to the precision the file keeps.
    pub fn px(&self, v: f64) -> String {
        anim::num(v, if self.settings.minify { 2 } else { 3 })
    }

    /// A number from 0 to 1, or a factor.
    pub fn unit(&self, v: f64) -> String {
        anim::num(v, 4)
    }

    /// Stops when the export is no longer wanted.
    pub fn check_cancelled(&self) -> Result<(), LottieError> {
        if self.host.cancelled() {
            Err(LottieError::Cancelled)
        } else {
            Ok(())
        }
    }

    /// The comp seconds of `scope` at each grid frame.
    pub fn comp_times(&self, scope: &Scope<'_>) -> Vec<f64> {
        self.span
            .grid
            .iter()
            .enumerate()
            .map(|(i, t)| scope.clock.at(i, *t))
            .collect()
    }

    /// `layer`'s own clock in `scope`.
    pub fn layer_clock(scope: &Scope<'_>, layer: &Layer) -> Clock {
        scope.clock.shifted(layer.start_offset.0.to_f64())
    }

    /// What an expression on `layer` reads at comp second `t`.
    pub fn context(&self, scope: &Scope<'_>, layer: &Layer, t: f64) -> Arc<ExpressionContext> {
        Arc::new(ExpressionContext {
            document: self.doc.clone(),
            comp: Some(scope.comp.id),
            layer: Some(layer.id),
            comp_time: t,
            current_depth: 0,
            inputs: None,
        })
    }

    /// `props` of `layer` as one value: still, keyed exactly, or sampled at
    /// every frame when an expression or a Retime above drives them. `tol` is
    /// how closely a sampled value is kept, per number.
    pub fn value(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        props: &[&Property],
        tol: &[f64],
    ) -> Value {
        if props
            .iter()
            .all(|p| matches!(p.animation, Animation::Static(_)))
        {
            return Value::Still(props.iter().map(|p| p.value_at(0.0)).collect());
        }
        let clock = Self::layer_clock(scope, layer);
        let expression = props
            .iter()
            .any(|p| matches!(p.animation, Animation::Expression(_)));
        if let (false, Clock::Shift(shift)) = (expression, &clock) {
            if let Ok(track) = keys::track(&props.iter().map(|p| curve(p)).collect::<Vec<_>>()) {
                let shift = *shift;
                return Value::from_track(&track.map_time(|t| t + shift));
            }
        }
        if expression {
            self.report.changed(
                &scope.place.layer(layer.id, &layer.name),
                "expression_baked",
                String::from("Expressions are baked into keys"),
                Vec::new(),
            );
        }
        let offset = layer.start_offset.0;
        let samples = self
            .comp_times(scope)
            .into_iter()
            .map(|t| {
                let lt = lumit_core::time::layer_time(t, offset);
                let context = expression.then(|| self.context(scope, layer, t));
                props
                    .iter()
                    .map(|p| match &context {
                        Some(c) => p.value_at_with_context(lt, c.clone()),
                        None => p.value_at(lt),
                    })
                    .collect()
            })
            .collect();
        Value::sampled(&self.span.grid, samples, tol)
    }

    /// Three linear channels as the display colour a file states, 0 to 1. A
    /// colour that moves is read in linear light at every frame and turned
    /// into display colour there, so the file passes through the colours
    /// Lumit does rather than a straight line between two encoded ends.
    pub fn colour(&mut self, scope: &Scope<'_>, layer: &Layer, ch: [&Property; 3]) -> Value {
        let linear = self.value(scope, layer, &ch, &[1e-6; 3]);
        let shown = |v: Vec<f64>| -> Vec<f64> {
            let c = [
                v.first().copied().unwrap_or(0.0),
                v.get(1).copied().unwrap_or(0.0),
                v.get(2).copied().unwrap_or(0.0),
            ];
            self.host.display(c).to_vec()
        };
        match linear {
            Value::Still(v) => Value::Still(shown(v)),
            moving => {
                let samples = self
                    .span
                    .grid
                    .iter()
                    .map(|t| shown(moving.at(*t)))
                    .collect();
                Value::sampled(
                    &self.span.grid,
                    samples,
                    &[crate::fit::tolerance::CHANNEL; 3],
                )
            }
        }
    }

    /// Values worked out frame by frame by `f(layer seconds)`, on `layer`'s
    /// clock, thinned to `tol`.
    pub fn sampled(
        &self,
        scope: &Scope<'_>,
        layer: &Layer,
        tol: &[f64],
        f: &mut dyn FnMut(f64) -> Vec<f64>,
    ) -> Value {
        let clock = Self::layer_clock(scope, layer);
        let samples = self
            .span
            .grid
            .iter()
            .enumerate()
            .map(|(i, t)| f(clock.at(i, *t)))
            .collect();
        Value::sampled(&self.span.grid, samples, tol)
    }

    /// Writes `attr="base"` for `value` into an open tag. The animation goes
    /// in afterwards, with [`Self::animate`].
    pub fn attr(
        &self,
        out: &mut String,
        attr: &str,
        value: &Value,
        fmt: &dyn Fn(&[f64]) -> String,
    ) {
        let _ = write!(out, " {attr}=\"{}\"", fmt(&value.at(self.span.start)));
    }

    /// The `<animate>` for `value`, when it moves inside the span.
    pub fn animate(
        &self,
        out: &mut String,
        attr: &str,
        value: &Value,
        tol: &[f64],
        fmt: &dyn Fn(&[f64]) -> String,
    ) {
        if let Some(smil) = anim::smil(value, &self.span, tol) {
            anim::write(out, attr, None, &smil, &self.span, fmt);
        }
    }

    /// The layers of `scope`, bottom of the stack first as SVG draws them.
    pub fn comp_layers(&mut self, scope: &Scope<'_>, out: &mut String) -> Result<(), LottieError> {
        if self.depth > MAX_DEPTH {
            return Ok(());
        }
        let tb = self.scope_timebase(scope);
        let list = crate::walk::walk(self.doc, scope.comp, &scope.place, &tb, &mut self.report);
        let total = list.len().max(1);
        for (n, o) in list.iter().enumerate().rev() {
            self.check_cancelled()?;
            if self.depth == 0 {
                self.host.progress((total - n) as f32 / total as f32);
            }
            if !matches!(o.role, crate::walk::Role::Drawn) {
                continue;
            }
            let matte = o
                .matte
                .and_then(|m| list.iter().find(|c| c.ind == m.ind).map(|c| (c, m)));
            self.layer(scope, o, matte, out)?;
        }
        Ok(())
    }

    /// The span written, read on `scope`'s comp clock: what the walk asks a
    /// layer to overlap.
    fn scope_timebase(&self, scope: &Scope<'_>) -> Timebase {
        let times = self.comp_times(scope);
        let (lo, hi) = times
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), t| {
                (a.min(*t), b.max(*t))
            });
        let rate = scope.comp.frame_rate;
        // The last frame's moment is in the span; the walk's end is exclusive.
        let pad = 1.0 / rate.fps().max(1.0);
        Timebase {
            fps: rate.fps(),
            rate,
            start: if lo.is_finite() { lo } else { 0.0 },
            end: if hi.is_finite() { hi + pad } else { 0.0 },
        }
    }
}

/// A property's keys as a curve: its spans, or its still value.
fn curve(p: &Property) -> keys::Curve {
    match &p.animation {
        Animation::Keyframed(k) if !k.is_empty() => keys::curve(k),
        _ => {
            let v = p.value_at(0.0);
            keys::Curve {
                spans: Vec::new(),
                first: v,
                last: v,
            }
        }
    }
}

/// XML text and attribute values with the five special characters escaped.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
