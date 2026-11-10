//! One layer as groups: its matte, opacity, blend and in and out on the
//! outermost; then its parents' transforms and its own, a group for each of
//! position, rotation, scale and anchor; then its effects and masks; then
//! what it draws.
//!
//! A group per transform factor, each with at most one `animateTransform`,
//! is a little longer than one matrix but is exact for every ease of every
//! number, and a viewer that plays nothing still shows the first frame.
//!
//! Runs on the export's thread.

use std::fmt::Write;

use lumit_core::anim::Animation;
use lumit_core::model::{BlendMode, Layer, LayerKind};
use lumit_core::orient::Orient;

use super::anim::{self, Clock, Value};
use super::{escape, Scope, Svg};
use crate::check::{arg, Feature, Place};
use crate::fit::tolerance;
use crate::walk::{Matte, Out, Role};
use crate::{LottieError, Picture};

/// What of a layer is drawn: a matte copy may leave its masks or its
/// effects behind.
#[derive(Debug, Clone, Copy)]
pub(super) struct Draw {
    pub masks: bool,
    pub effects: bool,
}

impl Svg<'_> {
    /// `o` as its outermost group and everything inside it.
    pub(super) fn layer(
        &mut self,
        scope: &Scope<'_>,
        o: &Out<'_>,
        matte: Option<(&Out<'_>, Matte)>,
        out: &mut String,
    ) -> Result<(), LottieError> {
        let layer = o.layer;
        if matches!(layer.kind, LayerKind::Null) {
            // A null draws nothing; its children carry its transform.
            return Ok(());
        }
        let shown = self.shown(scope, layer);
        if shown.iter().all(|(_, s)| !s) {
            return Ok(());
        }
        self.notes(scope, layer, &o.place);
        let mut tag = String::from("<g");
        if let Some((copy, m)) = matte {
            let id = self.matte(scope, copy, m)?;
            let _ = write!(tag, " mask=\"url(#{id})\"");
        }
        if let Some(mode) = self.blend(layer, &o.place) {
            let _ = write!(tag, " style=\"mix-blend-mode:{mode}\"");
            self.blends = true;
        }
        let animations = self.outer(scope, layer, &shown, &mut tag);
        if !self.settings.minify {
            let _ = write!(tag, " data-name=\"{}\"", escape(&layer.name));
        }
        out.push_str(&tag);
        out.push('>');
        out.push_str(&animations);
        self.nl(out);
        self.body(
            scope,
            layer,
            &o.place,
            Draw {
                masks: true,
                effects: true,
            },
            out,
        )?;
        out.push_str("</g>");
        self.nl(out);
        Ok(())
    }

    /// The opacity and the in and out of `layer`'s outermost group: the
    /// attributes go on `tag`, the animations come back.
    fn outer(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        shown: &[(f64, bool)],
        tag: &mut String,
    ) -> String {
        let mut animations = String::new();
        let opacity = self
            .value(
                scope,
                layer,
                &[&layer.transform.opacity],
                &[tolerance::PERCENT],
            )
            .map(|_, v| v.clamp(0.0, 100.0) / 100.0);
        if !matches!(&opacity, Value::Still(v) if v.first().is_some_and(|o| *o >= 1.0)) {
            let unit = |v: &[f64]| anim::num(v.first().copied().unwrap_or(1.0), 4);
            self.attr(tag, "opacity", &opacity, &unit);
            self.animate(&mut animations, "opacity", &opacity, &[1e-4], &unit);
        }
        if shown.len() > 1 || shown.first().is_some_and(|(_, s)| !s) {
            let word = |s: bool| if s { "inline" } else { "none" };
            let first = shown.first().is_some_and(|(_, s)| *s);
            let _ = write!(tag, " display=\"{}\"", word(first));
            let times: Vec<f64> = shown.iter().map(|(t, _)| *t).collect();
            let values: Vec<String> = shown.iter().map(|(_, s)| word(*s).to_owned()).collect();
            anim::write_steps(&mut animations, "display", &times, &values, &self.span);
        }
        animations
    }

    /// Everything inside a layer's outermost group: its parents' transforms
    /// and its own, its effects, its masks and what it draws.
    pub(super) fn body(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        place: &Place,
        draw: Draw,
        out: &mut String,
    ) -> Result<(), LottieError> {
        let mut open = 0;
        let chain = lumit_core::model::layer_parent_chain(scope.comp, layer.id);
        for id in chain.iter().rev() {
            if let Some(parent) = scope.comp.layers.iter().find(|l| l.id == *id) {
                open += self.transform(scope, parent, out);
                // A child's position is in its parent's own pixels, which
                // start at the corner of a Shape parent's art.
                let corner = self.art_corner(scope, parent);
                let px = [tolerance::PIXELS; 2];
                open += self.factor(out, "translate", &corner, &px, &pair);
            }
        }
        open += self.transform(scope, layer, out);
        if draw.effects && layer.switches.fx {
            open += self.effects(scope, layer, place, out)?;
        }
        if draw.masks {
            open += self.masks(scope, layer, place, out);
        }
        self.content(scope, layer, place, out)?;
        for _ in 0..open {
            out.push_str("</g>");
        }
        self.nl(out);
        Ok(())
    }

    /// When `layer` shows across the span, as (root second, shown) from the
    /// span's start on, one entry per change.
    fn shown(&self, scope: &Scope<'_>, layer: &Layer) -> Vec<(f64, bool)> {
        let (lo, hi) = (layer.in_point.0.to_f64(), layer.out_point.0.to_f64());
        let (start, end) = (self.span.start, self.span.end);
        match &scope.clock {
            Clock::Shift(s) => {
                let (a, b) = (lo + s, hi + s);
                let mut out = vec![(start, a <= start && start < b)];
                for edge in [a, b] {
                    if edge > start && edge < end && b > a {
                        let now = a <= edge && edge < b;
                        if out.last().is_some_and(|(_, was)| *was != now) {
                            out.push((edge, now));
                        }
                    }
                }
                out
            }
            Clock::Map(_) => {
                let mut out: Vec<(f64, bool)> = Vec::new();
                for (i, t) in self.span.grid.iter().enumerate() {
                    let c = scope.clock.at(i, *t);
                    let now = lo <= c && c < hi;
                    if out.last().is_none_or(|(_, was)| *was != now) {
                        out.push((*t, now));
                    }
                }
                out
            }
        }
    }

    /// What the check says about `layer` as a whole.
    fn notes(&mut self, scope: &Scope<'_>, layer: &Layer, place: &Place) {
        if layer.switches.three_d {
            self.report.changed(
                place,
                "three_d_flat",
                String::from("Drawn flat"),
                Vec::new(),
            );
        }
        if layer.switches.motion_blur && scope.comp.motion_blur.enabled {
            self.report.left_out(
                place,
                "motion_blur",
                String::from("Motion blur is left out"),
                Vec::new(),
            );
        }
        if !layer.paint.is_empty() {
            self.report.left_out(
                place,
                "paint",
                String::from("Paint strokes are left out"),
                Vec::new(),
            );
        }
        if layer.puppet.is_some() {
            self.report.left_out(
                place,
                "puppet",
                String::from("Puppet pins are left out"),
                Vec::new(),
            );
        }
    }

    /// The CSS name of `layer`'s blend, when it has one a browser draws.
    fn blend(&mut self, layer: &Layer, place: &Place) -> Option<&'static str> {
        let mode = match layer.blend {
            BlendMode::Normal => return None,
            BlendMode::Multiply => "multiply",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::Darken => "darken",
            BlendMode::Lighten => "lighten",
            BlendMode::ColourDodge => "color-dodge",
            BlendMode::ColourBurn => "color-burn",
            BlendMode::HardLight => "hard-light",
            BlendMode::SoftLight => "soft-light",
            BlendMode::Difference => "difference",
            BlendMode::Exclusion => "exclusion",
            BlendMode::Hue => "hue",
            BlendMode::Saturation => "saturation",
            BlendMode::Colour => "color",
            BlendMode::Luminosity => "luminosity",
            BlendMode::Add => "plus-lighter",
            other => {
                self.report.left_out(
                    place,
                    "blend_mode",
                    format!("{} draws as Normal", other.name()),
                    vec![arg("mode", other.name())],
                );
                return None;
            }
        };
        self.report.players(
            place,
            Feature::SvgBlend,
            "blend_mode",
            format!("{} blend", layer.blend.name()),
            vec![arg("mode", layer.blend.name())],
        );
        Some(mode)
    }

    /// `layer`'s own transform as nested groups, outermost first: position,
    /// rotation, scale, then the anchor. How many groups it opened.
    fn transform(&mut self, scope: &Scope<'_>, layer: &Layer, out: &mut String) -> usize {
        if matches!(
            layer.kind,
            LayerKind::Camera { .. } | LayerKind::Light { .. }
        ) {
            return self.eye(scope, layer, out);
        }
        let tr = &layer.transform;
        let px = tolerance::PIXELS;
        let mut open = 0;

        // Position: one translate when its two numbers key together, two
        // when they can't (one holds while the other moves).
        let keyed = |p: &lumit_core::anim::Property| matches!(p.animation, Animation::Keyframed(_));
        let apart = keyed(&tr.position_x)
            && keyed(&tr.position_y)
            && matches!(Svg::layer_clock(scope, layer), Clock::Shift(_))
            && crate::keys::track(&[super::curve(&tr.position_x), super::curve(&tr.position_y)])
                .is_err();
        if apart {
            let x = self.value(scope, layer, &[&tr.position_x], &[px]);
            let y = self.value(scope, layer, &[&tr.position_y], &[px]);
            open += self.factor(out, "translate", &x, &[px], &|s, v| {
                format!("{},0", s.px(v.first().copied().unwrap_or(0.0)))
            });
            open += self.factor(out, "translate", &y, &[px], &|s, v| {
                format!("0,{}", s.px(v.first().copied().unwrap_or(0.0)))
            });
        } else {
            let p = self.value(scope, layer, &[&tr.position_x, &tr.position_y], &[px, px]);
            open += self.factor(out, "translate", &p, &[px, px], &pair);
        }

        // Rotation, with the turn Along path adds sampled in.
        let rotation = if tr.orient == Orient::AlongPath && !layer.switches.three_d {
            // The heading kept continuous, so it never spins the long way
            // round between two frames.
            let mut last = 0.0;
            self.sampled(scope, layer, &[tolerance::DEGREES], &mut |lt| {
                let turn = unwrapped(lumit_core::orient::flat_turn(tr, lt), &mut last);
                vec![tr.rotation.value_at(lt) + turn]
            })
        } else {
            self.value(scope, layer, &[&tr.rotation], &[tolerance::DEGREES])
        };
        open += self.factor(out, "rotate", &rotation, &[tolerance::DEGREES], &|s, v| {
            anim::num(
                v.first().copied().unwrap_or(0.0),
                if s.settings.minify { 2 } else { 3 },
            )
        });

        let scale = self
            .value(
                scope,
                layer,
                &[&tr.scale_x, &tr.scale_y],
                &[tolerance::PERCENT; 2],
            )
            .map(|_, v| v / 100.0);
        open += self.factor(out, "scale", &scale, &[1e-4, 1e-4], &|s, v| {
            format!(
                "{},{}",
                s.unit(v.first().copied().unwrap_or(1.0)),
                s.unit(v.get(1).copied().unwrap_or(1.0))
            )
        });

        // A Shape layer's art is drawn in its own coordinates, and its
        // anchor is read from the corner of the art's box at the head of the
        // layer (`model::art_shift`), so that corner joins the anchor.
        let (sx, sy) = match &layer.kind {
            LayerKind::Shape { contents } => lumit_core::shape::contents_bounds(contents, 0.0)
                .map_or((0.0, 0.0), |(x, y, _, _)| (x, y)),
            _ => (0.0, 0.0),
        };
        let anchor = self
            .value(scope, layer, &[&tr.anchor_x, &tr.anchor_y], &[px, px])
            .map(|d, v| -(v + if d == 0 { sx } else { sy }));
        open += self.factor(out, "translate", &anchor, &[px, px], &pair);
        open
    }

    /// A camera or a light as a parent, placed as Lumit places one
    /// (`model::placement`): no anchor or scale, and a two-node one turned by
    /// its aim, read at every frame. Drawn flat, as every 3D layer is here.
    fn eye(&mut self, scope: &Scope<'_>, layer: &Layer, out: &mut String) -> usize {
        let mut last = 0.0;
        let samples = self
            .comp_times(scope)
            .into_iter()
            .map(|t| {
                let context = lumit_core::model::comp_context(self.doc, scope.comp, t);
                let read = lumit_core::model::context_reader(&context, t);
                let p = lumit_core::model::placement(layer, t, &read);
                vec![
                    p.position.0,
                    p.position.1,
                    unwrapped(p.rotation_deg.2, &mut last),
                ]
            })
            .collect();
        let (px, deg) = (tolerance::PIXELS, tolerance::DEGREES);
        let placed = Value::sampled(&self.span.grid, samples, &[px, px, deg]);
        let pick = |r| super::shapes::pick(&placed, r);
        let mut open = self.factor(out, "translate", &pick(0..2), &[px, px], &pair);
        open += self.factor(out, "rotate", &pick(2..3), &[deg], &|_, v| {
            anim::num(v.first().copied().unwrap_or(0.0), 3)
        });
        open
    }

    /// Where `layer`'s own pixels start, in the coordinates its art is drawn
    /// in: a Shape layer's art box corner at each moment (the corner at its
    /// head moved by `model::art_shift`), nought for any other kind. Its
    /// children and its masks are placed from there.
    pub(super) fn art_corner(&self, scope: &Scope<'_>, layer: &Layer) -> Value {
        let LayerKind::Shape { contents } = &layer.kind else {
            return Value::Still(vec![0.0, 0.0]);
        };
        let (x0, y0) = lumit_core::shape::contents_bounds(contents, 0.0)
            .map_or((0.0, 0.0), |(x, y, _, _)| (x, y));
        self.sampled(scope, layer, &[tolerance::PIXELS; 2], &mut |lt| {
            let (dx, dy) = lumit_core::model::art_shift(layer, lt);
            vec![x0 + dx, y0 + dy]
        })
    }

    /// One transform factor as a group, or nothing for the identity.
    pub(super) fn factor(
        &self,
        out: &mut String,
        kind: &str,
        value: &Value,
        tol: &[f64],
        fmt: &dyn Fn(&Svg<'_>, &[f64]) -> String,
    ) -> usize {
        let identity = if kind == "scale" { 1.0 } else { 0.0 };
        if let Value::Still(v) = value {
            if v.iter().all(|x| (x - identity).abs() < 1e-9) {
                return 0;
            }
        }
        let f = |v: &[f64]| fmt(self, v);
        let _ = write!(
            out,
            "<g transform=\"{kind}({})\">",
            f(&value.at(self.span.start))
        );
        if let Some(smil) = anim::smil(value, &self.span, tol) {
            anim::write(out, "transform", Some(kind), &smil, &self.span, &f);
        }
        1
    }

    /// A mask of the matte copy `copy`'s drawing, for the layer below it.
    fn matte(
        &mut self,
        scope: &Scope<'_>,
        copy: &Out<'_>,
        m: Matte,
    ) -> Result<String, LottieError> {
        let draw = match copy.role {
            Role::MatteCopy { masks, effects } => Draw { masks, effects },
            _ => Draw {
                masks: true,
                effects: true,
            },
        };
        let shown = self.shown(scope, copy.layer);
        let mut inner = String::from("<g");
        let animations = self.outer(scope, copy.layer, &shown, &mut inner);
        inner.push('>');
        inner.push_str(&animations);
        if shown.iter().any(|(_, s)| *s) {
            self.body(scope, copy.layer, &copy.place, draw, &mut inner)?;
        }
        inner.push_str("</g>");
        let (w, h) = (scope.comp.width, scope.comp.height);
        let region =
            format!("maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\"");
        let id = self.id("m");
        if m.inverted {
            // A white page with the matte drawn on it in black: what the
            // matte leaves clear shows, as lottie-web inverts a matte. Luma
            // is read from the display colours, as Lumit and a plain luma
            // mask read it, not from linear light.
            let f = self.id("f");
            let primitives = if m.luma {
                "<feColorMatrix type=\"luminanceToAlpha\"/><feComposite in2=\"SourceAlpha\" operator=\"in\"/>"
            } else {
                "<feColorMatrix type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\"/>"
            };
            let _ = write!(
                self.defs,
                "<filter id=\"{f}\" filterUnits=\"userSpaceOnUse\" color-interpolation-filters=\"sRGB\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\">{primitives}</filter>\
                 <mask id=\"{id}\" mask-type=\"luminance\" {region}><rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/><g filter=\"url(#{f})\">{inner}</g></mask>"
            );
        } else {
            let kind = if m.luma { "luminance" } else { "alpha" };
            let _ = write!(
                self.defs,
                "<mask id=\"{id}\" mask-type=\"{kind}\" {region}>{inner}</mask>"
            );
        }
        Ok(id)
    }

    /// What `layer` draws, in its own coordinates.
    pub(super) fn content(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        place: &Place,
        out: &mut String,
    ) -> Result<(), LottieError> {
        match &layer.kind {
            LayerKind::Solid { def } => {
                if let Some(solid) = self.doc.solid(*def) {
                    let c = solid.colour.0;
                    let rgb =
                        self.host
                            .display([f64::from(c[0]), f64::from(c[1]), f64::from(c[2])]);
                    let _ = write!(
                        out,
                        "<rect width=\"{}\" height=\"{}\" fill=\"{}\"",
                        solid.width,
                        solid.height,
                        crate::colour::hex(rgb)
                    );
                    if c[3] < 1.0 {
                        let _ = write!(
                            out,
                            " fill-opacity=\"{}\"",
                            self.unit(f64::from(c[3]).clamp(0.0, 1.0))
                        );
                    }
                    out.push_str("/>");
                }
            }
            LayerKind::Footage { item } => self.picture(*item, place, out)?,
            LayerKind::Precomp { comp } => self.precomp(scope, layer, *comp, place, out)?,
            LayerKind::Shape { contents } => {
                self.shape_layer(scope, layer, contents, place, out)?
            }
            LayerKind::Text { document } => self.text_layer(scope, layer, document, place, out)?,
            _ => {}
        }
        Ok(())
    }

    /// A footage item's still, drawn at its own size from its top-left.
    fn picture(
        &mut self,
        item: uuid::Uuid,
        place: &Place,
        out: &mut String,
    ) -> Result<(), LottieError> {
        if let Some((id, _, _)) = self.image(item, place)? {
            let _ = write!(out, "<use href=\"#{id}\"/>");
        }
        Ok(())
    }

    /// The id of a footage item's picture in the defs, and its size, written
    /// the first time it is asked for. `None` when there is no still to draw,
    /// which the check is told once.
    pub(super) fn image(
        &mut self,
        item: uuid::Uuid,
        place: &Place,
    ) -> Result<Option<super::Image>, LottieError> {
        if let Some((_, found)) = self.images.iter().find(|(i, _)| *i == item) {
            return Ok(found.clone());
        }
        let still = match self.host.picture(item) {
            Picture::Still(still) => Some(still),
            Picture::Video => {
                self.report.left_out(
                    place,
                    "video_footage",
                    String::from("Video is left out"),
                    Vec::new(),
                );
                None
            }
            Picture::Missing => {
                self.report.left_out(
                    place,
                    "missing_footage",
                    String::from("The file is missing"),
                    Vec::new(),
                );
                None
            }
        };
        let Some(still) = still else {
            self.images.push((item, None));
            return Ok(None);
        };
        let pic = crate::picture::encode(&still, self.settings.image_quality)
            .ok_or_else(|| LottieError::Json(String::from("a picture couldn't be encoded")))?;
        let href = match self.settings.images {
            crate::settings::ImageMode::Embed => {
                use base64::Engine;
                format!(
                    "data:{};base64,{}",
                    pic.mime,
                    base64::engine::general_purpose::STANDARD.encode(&pic.bytes)
                )
            }
            crate::settings::ImageMode::Beside => {
                let name = format!(
                    "{}_{}.{}",
                    self.settings.file_stem,
                    self.beside.len() + 1,
                    pic.ext
                );
                self.beside.push((name.clone(), pic.bytes));
                escape(&name)
            }
        };
        let id = self.id("i");
        let _ = write!(
            self.defs,
            "<image id=\"{id}\" width=\"{}\" height=\"{}\" href=\"{href}\"/>",
            still.width, still.height
        );
        let found = Some((id, still.width, still.height));
        self.images.push((item, found.clone()));
        Ok(found)
    }

    /// A precomp drawn inline, cut to its comp's frame, with its clock moved
    /// onto the file's.
    fn precomp(
        &mut self,
        scope: &Scope<'_>,
        layer: &Layer,
        id: uuid::Uuid,
        place: &Place,
        out: &mut String,
    ) -> Result<(), LottieError> {
        let Some(nested) = self.doc.comp(id) else {
            return Ok(());
        };
        let own = Svg::layer_clock(scope, layer);
        // Without a Retime Lumit reads the comp inside straight through: its
        // layers draw at any time inside their own spans, past its end too.
        let clock = match &layer.retime {
            None => own.clone(),
            Some(_) => {
                // A Retime bends the clock, so the comp inside is read frame
                // by frame through the same mapping the renderer uses.
                let times = self
                    .span
                    .grid
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        lumit_core::model::nested_source_time(layer, nested, own.at(i, *t))
                    })
                    .collect();
                Clock::Map(times)
            }
        };
        let inner = super::Scope {
            comp: nested,
            clock,
            place: place.inside(nested.id),
        };
        let clip = self.id("c");
        let _ = write!(
            self.defs,
            "<clipPath id=\"{clip}\"><rect width=\"{}\" height=\"{}\"/></clipPath>",
            nested.width, nested.height
        );
        let blends = nested.layers.iter().any(|l| l.blend != BlendMode::Normal);
        let _ = write!(out, "<g clip-path=\"url(#{clip})\"");
        if blends {
            out.push_str(" style=\"isolation:isolate\"");
        }
        out.push('>');
        self.nl(out);
        self.depth += 1;
        let drawn = self.comp_layers(&inner, out);
        self.depth -= 1;
        drawn?;
        out.push_str("</g>");
        Ok(())
    }
}

/// An angle `r` moved by whole turns to lie nearest `last`, which it then
/// becomes: a turn read frame by frame, kept continuous.
fn unwrapped(r: f64, last: &mut f64) -> f64 {
    let r = r + 360.0 * ((*last - r) / 360.0).round();
    *last = r;
    r
}

/// Two numbers as `x,y`.
pub(super) fn pair(s: &Svg<'_>, v: &[f64]) -> String {
    format!(
        "{},{}",
        s.px(v.first().copied().unwrap_or(0.0)),
        s.px(v.get(1).copied().unwrap_or(0.0))
    )
}
