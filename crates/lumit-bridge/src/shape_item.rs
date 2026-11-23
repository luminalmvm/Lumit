//! A shape layer's items across the seam: reading the engine's item into the
//! frontend's, and writing the frontend's back over the engine's.

use lumit_core::time::Rational;

use crate::api::effect::{BridgeKeyframe, BridgeRational, BridgeScalar, BridgeSideInterp};
use crate::api::layer::{clamped_property, BridgeShapeItem, BridgeShapeTaper, BridgeVertex};
use crate::api::shape::{BridgeGradientStop, BridgeStrokeGradient};
use crate::api::BridgeError;
use crate::mask_shape::same_shape;
use crate::shape_paint::{read as read_colour, write as write_colour};

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
            fill: item.fill.as_ref().map(|c| read_colour(c, offset)),
            stroke: item.stroke.as_ref().map(|c| read_colour(c, offset)),
            stroke_width: BridgeScalar::read_at(&item.stroke_width, offset),
            stroke_gradient: item.stroke_gradient.as_ref().map(|g| BridgeStrokeGradient {
                kind: g.kind,
                colour: g.colour.as_ref().map(|c| read_colour(c, offset)),
                start_x: BridgeScalar::read_at(&g.start_x, offset),
                start_y: BridgeScalar::read_at(&g.start_y, offset),
                end_x: BridgeScalar::read_at(&g.end_x, offset),
                end_y: BridgeScalar::read_at(&g.end_y, offset),
                start_opacity: BridgeScalar::read_at(&g.start_opacity, offset),
                end_opacity: BridgeScalar::read_at(&g.end_opacity, offset),
                highlight: BridgeScalar::read_at(&g.highlight, offset),
                highlight_angle: BridgeScalar::read_at(&g.highlight_angle, offset),
                stops: read_stops(&g.stops, offset),
            }),
            opacity: BridgeScalar::read_at(&item.opacity, offset),
            dashes: item
                .dashes
                .iter()
                .map(|d| BridgeScalar::read_at(d, offset))
                .collect(),
            dash_offset: BridgeScalar::read_at(&item.dash_offset, offset),
            gradient: item.gradient,
            gradient_colour: item
                .gradient_colour
                .as_ref()
                .map(|c| read_colour(c, offset)),
            gradient_start_x: BridgeScalar::read_at(&item.gradient_start_x, offset),
            gradient_start_y: BridgeScalar::read_at(&item.gradient_start_y, offset),
            gradient_end_x: BridgeScalar::read_at(&item.gradient_end_x, offset),
            gradient_end_y: BridgeScalar::read_at(&item.gradient_end_y, offset),
            gradient_start_opacity: BridgeScalar::read_at(&item.gradient_start_opacity, offset),
            gradient_end_opacity: BridgeScalar::read_at(&item.gradient_end_opacity, offset),
            gradient_highlight: BridgeScalar::read_at(&item.gradient_highlight, offset),
            gradient_highlight_angle: BridgeScalar::read_at(&item.gradient_highlight_angle, offset),
            gradient_stops: read_stops(&item.gradient_stops, offset),
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
            fill: self
                .fill
                .as_ref()
                .map(|c| write_colour(c, offset))
                .transpose()?,
            stroke: self
                .stroke
                .as_ref()
                .map(|c| write_colour(c, offset))
                .transpose()?,
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
            gradient_colour: self
                .gradient_colour
                .as_ref()
                .map(|c| write_colour(c, offset))
                .transpose()?,
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
            // Per cent of the radius either way, and degrees.
            gradient_highlight: clamped_property(&self.gradient_highlight, offset, -100.0, 100.0)?,
            gradient_highlight_angle: clamped_property(
                &self.gradient_highlight_angle,
                offset,
                -360_000.0,
                360_000.0,
            )?,
            gradient_stops: write_stops(&self.gradient_stops, offset)?,
            stroke_gradient: match &self.stroke_gradient {
                Some(g) => Some(g.write(offset)?),
                None => None,
            },
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
    /// does not describe: the **shape keyframes** and the forward-compatibility
    /// `extra` a newer Lumit may have written (docs/10 §1.1 makes preserving it
    /// mandatory).
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
            extra: previous.extra.clone(),
            ..self.write_item(offset)?
        };
        if let (Some(g), Some(was)) = (&mut written.stroke_gradient, &previous.stroke_gradient) {
            g.extra = was.extra.clone();
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

impl BridgeStrokeGradient {
    /// The engine's outline gradient this describes.
    fn write(
        &self,
        offset: Rational,
    ) -> Result<lumit_core::shape::gradient::StrokeGradient, BridgeError> {
        // The art's own coordinates, held to the same reach a vertex has.
        let point = |s: &BridgeScalar| clamped_property(s, offset, -100_000.0, 100_000.0);
        Ok(lumit_core::shape::gradient::StrokeGradient {
            // A number naming no reading is the flat colour, which draws.
            kind: if self.kind <= lumit_core::shape::gradient::ANGLE {
                self.kind
            } else {
                0
            },
            colour: self
                .colour
                .as_ref()
                .map(|c| write_colour(c, offset))
                .transpose()?,
            start_x: point(&self.start_x)?,
            start_y: point(&self.start_y)?,
            end_x: point(&self.end_x)?,
            end_y: point(&self.end_y)?,
            start_opacity: clamped_property(&self.start_opacity, offset, 0.0, 100.0)?,
            end_opacity: clamped_property(&self.end_opacity, offset, 0.0, 100.0)?,
            highlight: clamped_property(&self.highlight, offset, -100.0, 100.0)?,
            highlight_angle: clamped_property(
                &self.highlight_angle,
                offset,
                -360_000.0,
                360_000.0,
            )?,
            stops: write_stops(&self.stops, offset)?,
            extra: serde_json::Map::new(),
        })
    }
}

fn read_stops(
    stops: &[lumit_core::shape::GradientStop],
    offset: Rational,
) -> Vec<BridgeGradientStop> {
    stops
        .iter()
        .map(|s| BridgeGradientStop {
            position: BridgeScalar::read_at(&s.position, offset),
            colour: read_colour(&s.colour, offset),
            opacity: BridgeScalar::read_at(&s.opacity, offset),
        })
        .collect()
}

/// Held inside the ramp, every key of them, and left in the order they came
/// in: a stop's rows are found by where it is in the list.
fn write_stops(
    stops: &[BridgeGradientStop],
    offset: Rational,
) -> Result<Vec<lumit_core::shape::GradientStop>, BridgeError> {
    stops
        .iter()
        .take(lumit_core::shape::MAX_GRADIENT_STOPS)
        .map(|s| {
            Ok(lumit_core::shape::GradientStop {
                position: clamped_property(&s.position, offset, 0.0, 100.0)?,
                colour: write_colour(&s.colour, offset)?,
                opacity: clamped_property(&s.opacity, offset, 0.0, 100.0)?,
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use lumit_core::anim::{Animation, Keyframe, Property, SideInterp};
    use lumit_core::mask::{BezierPath, Vertex};
    use lumit_core::model::LinearColour;
    use lumit_core::shape::gradient::StrokeGradient;
    use lumit_core::shape::{colour, GradientStop, ShapeItem};
    use lumit_core::time::Rational;

    use crate::api::layer::BridgeShapeItem;

    /// The panels send the whole item back on every edit, so everything about
    /// its gradients has to come back as it went: keyed colours, the
    /// highlight, and the outline's ramp with what a newer Lumit wrote on it.
    #[test]
    fn a_panel_edit_keeps_every_gradient_setting() {
        let keyed = |from: f64, to: f64| Property {
            animation: Animation::Keyframed(
                [(0, from), (1, to)]
                    .map(|(secs, value)| Keyframe {
                        time: Rational::new(secs, 1).expect("a whole second"),
                        value,
                        interp_in: SideInterp::Linear,
                        interp_out: SideInterp::Linear,
                    })
                    .to_vec(),
            ),
            extra: serde_json::Map::new(),
        };
        let corner = |x: f64, y: f64| Vertex {
            pos: (x, y),
            tan_in: (0.0, 0.0),
            tan_out: (0.0, 0.0),
        };
        let path = BezierPath {
            vertices: vec![corner(0.0, 0.0), corner(40.0, 0.0), corner(40.0, 40.0)],
            closed: true,
        };
        let mut end = colour::still(LinearColour([0.2, 0.0, 0.4, 1.0]));
        end[0] = keyed(0.2, 0.9);
        let stops = vec![GradientStop {
            position: Property::fixed(40.0),
            colour: end.clone(),
            opacity: keyed(100.0, 20.0),
        }];
        let mut item = ShapeItem::filled("Orb", path, LinearColour([1.0, 0.9, 0.8, 1.0]));
        item.gradient = 2;
        item.gradient_colour = Some(end.clone());
        item.gradient_highlight = keyed(0.0, 60.0);
        item.gradient_highlight_angle = Property::fixed(-135.0);
        item.gradient_stops = stops.clone();
        item.stroke = Some(colour::still(LinearColour([0.0, 1.0, 1.0, 1.0])));
        item.stroke_width = Property::fixed(12.0);
        let mut extra = serde_json::Map::new();
        extra.insert("newer".into(), serde_json::json!(1));
        item.stroke_gradient = Some(StrokeGradient {
            kind: 3,
            colour: Some(end),
            start_x: Property::zero(),
            start_y: Property::zero(),
            end_x: keyed(0.0, 40.0),
            end_y: Property::zero(),
            start_opacity: Property::fixed(100.0),
            end_opacity: Property::fixed(50.0),
            highlight: Property::fixed(30.0),
            highlight_angle: Property::zero(),
            stops,
            extra,
        });
        let zero = Rational::new(0, 1).expect("zero");
        let sent = BridgeShapeItem::read_at(&item, zero);
        let back = sent.write_item_over(&item, zero, None).expect("writes");
        assert_eq!(back, item);
    }
}
