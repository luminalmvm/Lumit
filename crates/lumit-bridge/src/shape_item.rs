//! A shape layer's items across the seam: reading the engine's item into the
//! frontend's, and writing the frontend's back over the engine's.

use lumit_core::time::Rational;

use crate::api::effect::{BridgeKeyframe, BridgeRational, BridgeScalar, BridgeSideInterp};
use crate::api::layer::{
    clamped_property, BridgeGradientStop, BridgeShapeItem, BridgeShapeTaper, BridgeVertex,
};
use crate::api::BridgeError;
use crate::mask_shape::same_shape;

impl BridgeShapeItem {
    pub(crate) fn read_at(item: &lumit_core::shape::ShapeItem, offset: Rational) -> Self {
        Self {
            id: item.id,
            name: item.name.clone(),
            vertices: item.path.vertices.iter().map(BridgeVertex::read).collect(),
            closed: item.path.closed,
            hidden: item.hidden,
            fill_hidden: item.fill_hidden,
            stroke_hidden: item.stroke_hidden,
            fill: crate::shape_paint::read(item.fill.as_ref(), offset),
            stroke: crate::shape_paint::read(item.stroke.as_ref(), offset),
            stroke_width: BridgeScalar::read_at(&item.stroke_width, offset),
            opacity: BridgeScalar::read_at(&item.opacity, offset),
            dashes: item
                .dashes
                .iter()
                .map(|d| BridgeScalar::read_at(d, offset))
                .collect(),
            dash_offset: BridgeScalar::read_at(&item.dash_offset, offset),
            gradient: item.gradient,
            gradient_colour: item.gradient_colour.as_ref().map(shown),
            gradient_start_x: BridgeScalar::read_at(&item.gradient_start_x, offset),
            gradient_start_y: BridgeScalar::read_at(&item.gradient_start_y, offset),
            gradient_end_x: BridgeScalar::read_at(&item.gradient_end_x, offset),
            gradient_end_y: BridgeScalar::read_at(&item.gradient_end_y, offset),
            gradient_start_opacity: BridgeScalar::read_at(&item.gradient_start_opacity, offset),
            gradient_end_opacity: BridgeScalar::read_at(&item.gradient_end_opacity, offset),
            gradient_stops: item
                .gradient_stops
                .iter()
                .map(|s| BridgeGradientStop {
                    position: BridgeScalar::read_at(&s.position, offset),
                    colour: shown(&s.colour),
                    opacity: BridgeScalar::read_at(&s.opacity, offset),
                })
                .collect(),
            combine: item.combine,
            path_keys: item
                .path_keys
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let time = k.time.checked_add(offset).unwrap_or(k.time);
                    BridgeKeyframe {
                        time: BridgeRational {
                            num: time.num(),
                            den: time.den(),
                        },
                        value: i as f64,
                        interp_in: BridgeSideInterp::read(k.interp_in),
                        interp_out: BridgeSideInterp::read(k.interp_out),
                    }
                })
                .collect(),
            taper: item.taper.as_ref().map(|w| BridgeShapeTaper {
                start: BridgeScalar::read_at(&w.start, offset),
                end: BridgeScalar::read_at(&w.end, offset),
                start_length: BridgeScalar::read_at(&w.start_length, offset),
                end_length: BridgeScalar::read_at(&w.end_length, offset),
                ease: BridgeScalar::read_at(&w.ease, offset),
                scope: w.scope,
            }),
            primitive: item
                .primitive
                .as_ref()
                .map(|p| crate::api::shape::BridgeShapePrimitive::read_at(p, offset)),
        }
    }

    /// The engine's item this describes. Public to the crate because the
    /// composition builds a whole layer out of a list of them.
    pub(crate) fn write_item(
        &self,
        offset: Rational,
    ) -> Result<lumit_core::shape::ShapeItem, BridgeError> {
        let primitive = match &self.primitive {
            Some(p) => Some(p.write(offset)?),
            None => None,
        };
        Ok(lumit_core::shape::ShapeItem {
            id: self.id,
            name: self.name.clone(),
            // A parametric shape's path is whatever its numbers make.
            path: match &primitive {
                Some(p) => p.path_at(0.0),
                None => self.sent_path(),
            },
            primitive,
            hidden: self.hidden,
            fill_hidden: self.fill_hidden,
            stroke_hidden: self.stroke_hidden,
            fill: crate::shape_paint::write(self.fill.as_ref(), offset)?,
            stroke: crate::shape_paint::write(self.stroke.as_ref(), offset)?,
            stroke_width: clamped_property(&self.stroke_width, offset, 0.0, 10_000.0)?,
            opacity: clamped_property(&self.opacity, offset, 0.0, 100.0)?,
            // Lengths in layer pixels: a negative dash is not a shorter dash,
            // it is a number with no meaning, so zero is where it lands. The
            // offset may be negative — it slides both ways.
            dashes: self
                .dashes
                .iter()
                .map(|d| clamped_property(d, offset, 0.0, 100_000.0))
                .collect::<Result<_, _>>()?,
            dash_offset: clamped_property(&self.dash_offset, offset, -100_000.0, 100_000.0)?,
            // A number naming no reading a fill has is the flat fill, which
            // is the answer that draws something.
            gradient: match self.gradient {
                0..=2 | lumit_core::shape::gradient::ANGLE => self.gradient,
                _ => 0,
            },
            gradient_colour: self.gradient_colour.map(still),
            // The art's own coordinates, held to the same reach a vertex has.
            gradient_start_x: clamped_property(
                &self.gradient_start_x,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            gradient_start_y: clamped_property(
                &self.gradient_start_y,
                offset,
                -100_000.0,
                100_000.0,
            )?,
            gradient_end_x: clamped_property(&self.gradient_end_x, offset, -100_000.0, 100_000.0)?,
            gradient_end_y: clamped_property(&self.gradient_end_y, offset, -100_000.0, 100_000.0)?,
            gradient_start_opacity: clamped_property(
                &self.gradient_start_opacity,
                offset,
                0.0,
                100.0,
            )?,
            gradient_end_opacity: clamped_property(&self.gradient_end_opacity, offset, 0.0, 100.0)?,
            // Held inside the ramp, every key of them, and left in the order
            // they came in: a stop's rows are found by where it is in the list.
            gradient_stops: self
                .gradient_stops
                .iter()
                .take(lumit_core::shape::MAX_GRADIENT_STOPS)
                .map(|s| {
                    Ok(lumit_core::shape::GradientStop {
                        position: clamped_property(&s.position, offset, 0.0, 100.0)?,
                        colour: still(s.colour),
                        opacity: clamped_property(&s.opacity, offset, 0.0, 100.0)?,
                    })
                })
                .collect::<Result<_, BridgeError>>()?,
            // The frontend does not edit these yet.
            stroke_gradient: None,
            gradient_highlight: lumit_core::anim::Property::zero(),
            gradient_highlight_angle: lumit_core::anim::Property::zero(),
            // Five readings and no sixth: a number naming none of them draws
            // the art on its own, which is the answer that shows something.
            combine: if self.combine <= 4 { self.combine } else { 0 },
            // What this type does not carry. An item edited from the frontend
            // must not LOSE its shape keys, so `write_item_over` patches them
            // back from the item it is replacing; this bare form is only for
            // art that did not exist a moment ago, which has none.
            path_keys: Vec::new(),
            // All per cent, held to 0..100.
            taper: match &self.taper {
                Some(w) => Some(lumit_core::shape::Taper {
                    start: clamped_property(&w.start, offset, 0.0, 100.0)?,
                    end: clamped_property(&w.end, offset, 0.0, 100.0)?,
                    start_length: clamped_property(&w.start_length, offset, 0.0, 100.0)?,
                    end_length: clamped_property(&w.end_length, offset, 0.0, 100.0)?,
                    ease: clamped_property(&w.ease, offset, 0.0, 100.0)?,
                    scope: if w.scope <= 1 { w.scope } else { 0 },
                }),
                None => None,
            },
            extra: serde_json::Map::new(),
        })
    }

    /// [`Self::write_item`], but keeping what `previous` carries and this type
    /// does not describe: the **shape keyframes**, the outline's gradient, the
    /// fill's highlight, the keys on its gradient colours and the
    /// forward-compatibility `extra` a newer Lumit may have written (docs/10
    /// §1.1 makes preserving it mandatory).
    ///
    /// **A shape edit on a morphing item lands on the key under the
    /// playhead**, exactly as a mask's does. Once a path is keyed,
    /// `path` is no longer what the item draws — `path_at` reads the keys — so
    /// writing the dragged vertices there would move nothing at all and the art
    /// would appear frozen under the pointer. `at` is where the playhead is;
    /// without it — an edit that is not a shape edit, such as an opacity drag —
    /// the keys are carried through untouched.
    ///
    /// An item that came back with the shape it already had is not a shape
    /// edit either, even with `at` given. The list is sent whole, so the items
    /// nobody touched arrive holding their still path (from the Timeline) or
    /// the shape showing at the playhead (from the Viewer), and keying either
    /// would snap the item or plant a key nobody asked for.
    pub(crate) fn write_item_over(
        &self,
        previous: &lumit_core::shape::ShapeItem,
        offset: Rational,
        at: Option<Rational>,
    ) -> Result<lumit_core::shape::ShapeItem, BridgeError> {
        let mut written = lumit_core::shape::ShapeItem {
            path_keys: previous.path_keys.clone(),
            stroke_gradient: previous.stroke_gradient.clone(),
            gradient_highlight: previous.gradient_highlight.clone(),
            gradient_highlight_angle: previous.gradient_highlight_angle.clone(),
            extra: previous.extra.clone(),
            ..self.write_item(offset)?
        };
        // A keyed gradient colour crosses still, so one that comes back as it
        // went keeps its keys.
        if let (Some(was), Some(sent)) = (&previous.gradient_colour, self.gradient_colour) {
            if shown(was) == sent {
                written.gradient_colour = Some(was.clone());
            }
        }
        for ((stop, was), sent) in written
            .gradient_stops
            .iter_mut()
            .zip(&previous.gradient_stops)
            .zip(&self.gradient_stops)
        {
            if shown(&was.colour) == sent.colour {
                stop.colour = was.colour.clone();
            }
        }
        // A parametric shape keeps its numbers until one of its points is
        // dragged, which makes it a drawn path holding the dragged points.
        if let Some(p) = &mut written.primitive {
            if let Some(was) = &previous.primitive {
                p.extra = was.extra.clone();
            }
            let sent = self.sent_path();
            let showing = match at {
                Some(at) => at
                    .checked_sub(offset)
                    .map(|at| same_shape(&sent, &previous.path_at(at.to_f64())))
                    .unwrap_or(false),
                None => false,
            };
            if !showing && !same_shape(&sent, &previous.path) {
                written.primitive = None;
                written.path = sent;
            }
            return Ok(written);
        }
        if let (false, Some(at)) = (written.path_keys.is_empty(), at) {
            let at = at
                .checked_sub(offset)
                .map_err(|_| BridgeError::InvalidKeyframes)?;
            let path = std::mem::replace(&mut written.path, previous.path.clone());
            if same_shape(&path, &previous.path)
                || same_shape(&path, &previous.path_at(at.to_f64()))
            {
                return Ok(written);
            }
            match written.path_keys.iter_mut().find(|k| k.time == at) {
                Some(key) => key.path = path,
                None => {
                    let i = written
                        .path_keys
                        .iter()
                        .position(|k| k.time > at)
                        .unwrap_or(written.path_keys.len());
                    written.path_keys.insert(
                        i,
                        lumit_core::mask::PathKeyframe {
                            time: at,
                            path,
                            interp_in: lumit_core::anim::SideInterp::Linear,
                            interp_out: lumit_core::anim::SideInterp::Linear,
                        },
                    );
                }
            }
        }
        Ok(written)
    }

    /// The path as the frontend sent it.
    fn sent_path(&self) -> lumit_core::mask::BezierPath {
        lumit_core::mask::BezierPath {
            vertices: self.vertices.iter().map(BridgeVertex::write).collect(),
            closed: self.closed,
        }
    }
}

/// A gradient colour as the frontend shows it: still, at the head of the
/// layer, as it does not key one yet.
pub(crate) fn shown(c: &[lumit_core::anim::Property; 4]) -> crate::api::assets::BridgeColourRgba {
    crate::api::assets::colour_of(lumit_core::shape::colour::at(c, 0.0))
}

/// A colour the frontend sent, as a gradient colour.
pub(crate) fn still(c: crate::api::assets::BridgeColourRgba) -> [lumit_core::anim::Property; 4] {
    lumit_core::shape::colour::still(crate::api::assets::linear_of(c))
}
