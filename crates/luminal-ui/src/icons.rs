//! Flat monochrome stroke glyphs, drawn straight onto the egui painter
//! (docs/15-DESIGN.md §5): thin strokes in the theme's `currentColor`, no emoji,
//! no icon font, no image files.
//!
//! In plain terms: instead of shipping little picture files or a special icon
//! font, Luminal *draws* each icon from a handful of lines and curves every
//! frame. Two upsides: the icons stay razor-sharp at any size or screen zoom,
//! and they are always exactly the theme colour we ask for (so they dim on
//! hover and turn accent when active, like the rest of the UI). Each icon is
//! described inside a notional 0..1 square and scaled to fit wherever it is
//! placed, so the same definition works at 16px in a toolbar or larger.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};
use std::f32::consts::PI;

/// One drawable glyph. Add a variant here and a match arm in [`paint`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    /// Selection arrow (the Select tool).
    Pointer,
    /// Four-way arrows: pan the view (the Hand tool).
    Move,
    /// Mask/shape tool — rectangle.
    Rectangle,
    /// Mask/shape tool — ellipse.
    Ellipse,
    /// Mask/shape tool — star.
    Star,
    /// Pen (nib) tool.
    Pen,
    /// Transport: play.
    Play,
    /// Transport: pause.
    Pause,
    /// Closed padlock (aspect ratio locked).
    Lock,
    /// Open padlock (aspect ratio free).
    Unlock,
    /// Chain link (linked values, e.g. linked scale).
    Link,
    /// Folder (a project folder).
    Folder,
    /// Film frame (a composition — kept for the "new composition" button).
    Film,
    /// Graph editor view: an animation curve.
    GraphCurve,
    /// Layer/timeline view: stacked bars.
    TimelineBars,
    /// Node graph view (the future node system).
    Nodes,
    /// Footage item: a media clip.
    Footage,
    /// Composition item: stacked layers.
    Comp,
    /// Solid item: a filled block of colour.
    Solid,
    /// Sequence layer: clips cut back-to-back on a row.
    Sequence,
    /// Text layer: a capital T.
    Text,
    /// Camera layer: a video camera.
    Camera,
    /// Layer visibility switch: an eye.
    Eye,
    /// Audible layer: a speaker with sound waves.
    Audio,
    /// Muted layer: a speaker, struck through.
    Mute,
}

impl Icon {
    /// Every variant, for exhaustive iteration (tests, palettes).
    pub const ALL: [Icon; 25] = [
        Icon::Pointer,
        Icon::Move,
        Icon::Rectangle,
        Icon::Ellipse,
        Icon::Star,
        Icon::Pen,
        Icon::Play,
        Icon::Pause,
        Icon::Lock,
        Icon::Unlock,
        Icon::Link,
        Icon::Folder,
        Icon::Film,
        Icon::GraphCurve,
        Icon::TimelineBars,
        Icon::Nodes,
        Icon::Footage,
        Icon::Comp,
        Icon::Solid,
        Icon::Sequence,
        Icon::Text,
        Icon::Camera,
        Icon::Eye,
        Icon::Audio,
        Icon::Mute,
    ];
}

/// Paint `icon` centred in `rect`, stroked in `color` at `width` px. The glyph
/// is drawn in the largest centred square that fits, inset slightly so strokes
/// never clip the edge.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32, width: f32) {
    let side = rect.width().min(rect.height());
    let b = Rect::from_center_size(rect.center(), Vec2::splat(side)).shrink(side * 0.12);
    // Normalised (0..1, y-down) → screen position inside the icon box.
    let p = |nx: f32, ny: f32| b.min + Vec2::new(nx * b.width(), ny * b.height());
    let stroke = Stroke::new(width, color);
    let poly = |pts: &[(f32, f32)]| pts.iter().map(|&(x, y)| p(x, y)).collect::<Vec<_>>();
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(poly(pts), stroke));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::closed_line(poly(pts), stroke));
    };
    // A partial circle from `a0` to `a1` radians (y-down), centred at (cx,cy).
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32| {
        let n = 16;
        let pts: Vec<Pos2> = (0..=n)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / n as f32;
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        painter.add(Shape::line(pts, stroke));
    };

    match icon {
        Icon::Pointer => {
            closed(&[
                (0.16, 0.08),
                (0.16, 0.84),
                (0.36, 0.64),
                (0.49, 0.92),
                (0.60, 0.87),
                (0.47, 0.60),
                (0.70, 0.60),
            ]);
        }
        Icon::Move => {
            line(&[(0.5, 0.06), (0.5, 0.94)]);
            line(&[(0.06, 0.5), (0.94, 0.5)]);
            // Arrow heads on each of the four ends.
            line(&[(0.39, 0.19), (0.5, 0.06), (0.61, 0.19)]);
            line(&[(0.39, 0.81), (0.5, 0.94), (0.61, 0.81)]);
            line(&[(0.19, 0.39), (0.06, 0.5), (0.19, 0.61)]);
            line(&[(0.81, 0.39), (0.94, 0.5), (0.81, 0.61)]);
        }
        Icon::Rectangle => {
            closed(&[(0.12, 0.20), (0.88, 0.20), (0.88, 0.80), (0.12, 0.80)]);
        }
        Icon::Ellipse => {
            painter.add(Shape::circle_stroke(b.center(), b.width() * 0.38, stroke));
        }
        Icon::Star => {
            let (cx, cy) = (b.center().x, b.center().y);
            let (ro, ri) = (b.width() * 0.46, b.width() * 0.19);
            let pts: Vec<Pos2> = (0..10)
                .map(|i| {
                    let r = if i % 2 == 0 { ro } else { ri };
                    let a = -PI / 2.0 + i as f32 * PI / 5.0;
                    Pos2::new(cx + r * a.cos(), cy + r * a.sin())
                })
                .collect();
            painter.add(Shape::closed_line(pts, stroke));
        }
        Icon::Pen => {
            // A downward nib: triangle, central slit, and the vent hole.
            closed(&[(0.28, 0.16), (0.72, 0.16), (0.5, 0.88)]);
            line(&[(0.5, 0.52), (0.5, 0.88)]);
            painter.add(Shape::circle_stroke(p(0.5, 0.40), b.width() * 0.05, stroke));
        }
        Icon::Play => {
            closed(&[(0.30, 0.16), (0.82, 0.5), (0.30, 0.84)]);
        }
        Icon::Pause => {
            closed(&[(0.30, 0.16), (0.44, 0.16), (0.44, 0.84), (0.30, 0.84)]);
            closed(&[(0.56, 0.16), (0.70, 0.16), (0.70, 0.84), (0.56, 0.84)]);
        }
        Icon::Lock => {
            closed(&[(0.26, 0.46), (0.74, 0.46), (0.74, 0.86), (0.26, 0.86)]);
            // Shackle: a closed top arc with both legs meeting the body.
            arc(0.5, 0.46, 0.16, PI, 2.0 * PI);
            line(&[(0.34, 0.46), (0.34, 0.36)]);
            line(&[(0.66, 0.46), (0.66, 0.36)]);
        }
        Icon::Unlock => {
            closed(&[(0.26, 0.46), (0.74, 0.46), (0.74, 0.86), (0.26, 0.86)]);
            // Same shackle, hinged open on the right (only the left leg is down).
            arc(0.62, 0.42, 0.16, PI, 2.0 * PI);
            line(&[(0.46, 0.42), (0.46, 0.46)]);
        }
        Icon::Link => {
            painter.add(Shape::circle_stroke(p(0.38, 0.5), b.width() * 0.17, stroke));
            painter.add(Shape::circle_stroke(p(0.62, 0.5), b.width() * 0.17, stroke));
        }
        Icon::Folder => {
            closed(&[
                (0.12, 0.30),
                (0.40, 0.30),
                (0.48, 0.40),
                (0.88, 0.40),
                (0.88, 0.80),
                (0.12, 0.80),
            ]);
        }
        Icon::Film => {
            closed(&[(0.16, 0.22), (0.84, 0.22), (0.84, 0.78), (0.16, 0.78)]);
            closed(&[(0.32, 0.34), (0.68, 0.34), (0.68, 0.66), (0.32, 0.66)]);
        }
        Icon::GraphCurve => {
            // An ease S-curve, the graph editor's signature.
            line(&[
                (0.10, 0.80),
                (0.30, 0.74),
                (0.48, 0.50),
                (0.66, 0.26),
                (0.90, 0.20),
            ]);
        }
        Icon::TimelineBars => {
            closed(&[(0.14, 0.22), (0.64, 0.22), (0.64, 0.36), (0.14, 0.36)]);
            closed(&[(0.14, 0.43), (0.86, 0.43), (0.86, 0.57), (0.14, 0.57)]);
            closed(&[(0.14, 0.64), (0.50, 0.64), (0.50, 0.78), (0.14, 0.78)]);
        }
        Icon::Nodes => {
            line(&[(0.28, 0.30), (0.5, 0.70)]);
            line(&[(0.72, 0.28), (0.5, 0.70)]);
            painter.add(Shape::circle_stroke(
                p(0.28, 0.30),
                b.width() * 0.12,
                stroke,
            ));
            painter.add(Shape::circle_stroke(
                p(0.72, 0.28),
                b.width() * 0.12,
                stroke,
            ));
            painter.add(Shape::circle_stroke(p(0.5, 0.72), b.width() * 0.12, stroke));
        }
        Icon::Footage => {
            closed(&[(0.14, 0.24), (0.86, 0.24), (0.86, 0.76), (0.14, 0.76)]);
            closed(&[(0.42, 0.37), (0.66, 0.5), (0.42, 0.63)]);
        }
        Icon::Comp => {
            closed(&[(0.30, 0.16), (0.82, 0.16), (0.82, 0.54), (0.30, 0.54)]);
            closed(&[(0.18, 0.46), (0.70, 0.46), (0.70, 0.84), (0.18, 0.84)]);
        }
        Icon::Solid => {
            painter.rect_filled(Rect::from_min_max(p(0.22, 0.22), p(0.78, 0.78)), 2.0, color);
        }
        Icon::Sequence => {
            // Clips cut back-to-back on a row.
            closed(&[(0.12, 0.32), (0.88, 0.32), (0.88, 0.68), (0.12, 0.68)]);
            line(&[(0.38, 0.32), (0.38, 0.68)]);
            line(&[(0.62, 0.32), (0.62, 0.68)]);
        }
        Icon::Text => {
            line(&[(0.24, 0.28), (0.76, 0.28)]);
            line(&[(0.5, 0.28), (0.5, 0.76)]);
        }
        Icon::Camera => {
            closed(&[(0.12, 0.36), (0.62, 0.36), (0.62, 0.70), (0.12, 0.70)]);
            closed(&[(0.62, 0.45), (0.84, 0.37), (0.84, 0.69), (0.62, 0.61)]);
        }
        Icon::Eye => {
            // Almond outline with a pupil.
            closed(&[
                (0.10, 0.50),
                (0.30, 0.32),
                (0.50, 0.28),
                (0.70, 0.32),
                (0.90, 0.50),
                (0.70, 0.68),
                (0.50, 0.72),
                (0.30, 0.68),
            ]);
            painter.add(Shape::circle_stroke(p(0.5, 0.5), b.width() * 0.13, stroke));
        }
        Icon::Audio => {
            // Speaker box + cone, with two sound-wave arcs.
            closed(&[
                (0.14, 0.40),
                (0.30, 0.40),
                (0.46, 0.26),
                (0.46, 0.74),
                (0.30, 0.60),
                (0.14, 0.60),
            ]);
            arc(0.46, 0.5, 0.20, -PI / 3.0, PI / 3.0);
            arc(0.46, 0.5, 0.33, -PI / 3.0, PI / 3.0);
        }
        Icon::Mute => {
            closed(&[
                (0.14, 0.40),
                (0.30, 0.40),
                (0.46, 0.26),
                (0.46, 0.74),
                (0.30, 0.60),
                (0.14, 0.60),
            ]);
            line(&[(0.58, 0.36), (0.86, 0.64)]);
            line(&[(0.86, 0.36), (0.58, 0.64)]);
        }
    }
}

/// A filled disclosure triangle: points right when closed, down when open. Drawn
/// rather than a font glyph (`▸`/`▾`), because egui's bundled fonts don't carry
/// those code points — so as glyphs the twirls simply vanish.
pub fn disclosure(painter: &Painter, rect: Rect, open: bool, color: Color32) {
    painter.add(Shape::convex_polygon(
        disclosure_points(rect, open).to_vec(),
        color,
        Stroke::NONE,
    ));
}

/// A stopwatch, drawn rather than a font glyph (egui's fonts carry no stopwatch
/// emoji, so `⏱` simply vanishes): a ring with a top button, and a filled centre
/// when the property is animated, so its animation state reads at a glance.
pub fn stopwatch(painter: &Painter, center: Pos2, radius: f32, animated: bool, color: Color32) {
    let stroke = Stroke::new(1.2_f32, color);
    painter.circle_stroke(center, radius, stroke);
    painter.line_segment(
        [
            center + Vec2::new(0.0, -radius),
            center + Vec2::new(0.0, -radius - 2.0),
        ],
        stroke,
    );
    if animated {
        painter.circle_filled(center, radius * 0.45, color);
    }
}

/// The twirl triangle's three corners inside `rect`. Kept separate from the
/// painting so a test can pin the glyph's size: an earlier 0.30 shrink (on top
/// of the triangle's own inset) left a ~4 px sliver that was invisible in
/// practice, so the box now uses the same 0.12 inset as every other icon.
fn disclosure_points(rect: Rect, open: bool) -> [Pos2; 3] {
    let s = rect.width().min(rect.height());
    let b = Rect::from_center_size(rect.center(), Vec2::splat(s)).shrink(s * 0.12);
    let p = |nx: f32, ny: f32| b.min + Vec2::new(nx * b.width(), ny * b.height());
    if open {
        [p(0.12, 0.30), p(0.88, 0.30), p(0.5, 0.82)]
    } else {
        [p(0.30, 0.12), p(0.82, 0.5), p(0.30, 0.88)]
    }
}

/// A small downward caret marking a control as a dropdown. Drawn, for the same
/// font reason as [`disclosure`].
pub fn caret_down(painter: &Painter, center: Pos2, color: Color32) {
    painter.add(Shape::convex_polygon(
        vec![
            Pos2::new(center.x - 3.0, center.y - 1.5),
            Pos2::new(center.x + 3.0, center.y - 1.5),
            Pos2::new(center.x, center.y + 2.5),
        ],
        color,
        Stroke::NONE,
    ));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Every icon paints without panicking (guards the match arms and the
    /// point/arc maths against empty paths or bad indices).
    #[test]
    fn every_icon_paints() {
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let painter = ui.painter().clone();
                for icon in Icon::ALL {
                    paint(
                        &painter,
                        Rect::from_min_size(Pos2::ZERO, Vec2::splat(16.0)),
                        icon,
                        Color32::WHITE,
                        1.5,
                    );
                }
            });
        });
    }

    /// Regression (invisible twirl): the disclosure triangle must occupy a
    /// readable share of its rect. The old geometry (a 0.30 shrink on top of the
    /// triangle's own inset) produced a ~4 px sliver inside the timeline's 16 px
    /// slot, which users could not see at all.
    #[test]
    fn disclosure_triangle_is_a_readable_size() {
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(16.0, 20.0));
        let s = rect.width().min(rect.height());
        for open in [false, true] {
            let pts = disclosure_points(rect, open);
            let bbox = Rect::from_points(&pts);
            let min_side = bbox.width().min(bbox.height());
            assert!(
                min_side >= 0.30 * s,
                "twirl (open = {open}) spans {min_side} px of a {s} px slot — too small to see"
            );
            // And it must stay inside the rect it was given.
            assert!(rect.contains_rect(bbox));
        }
    }
}
