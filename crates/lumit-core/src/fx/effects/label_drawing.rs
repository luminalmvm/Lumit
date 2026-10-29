//! How Label points and Type fill put their words on the frame: each wording
//! drawn once by the text engine, then placed, turned and sized from that
//! drawing, read between texels so a label can sit between pixels.

use super::label_points::{Label, Tint};

/// One wording laid out by the text engine: scene-linear,
/// premultiplied texels, row by row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LabelDrawing {
    pub width: usize,
    pub height: usize,
    pub px: Vec<[f32; 4]>,
}

/// One label as it lands on the frame: its drawing, where on the drawing and
/// on the frame its anchor is, how much smaller than the drawing it is drawn
/// across and down, its turn, its colour, and the pixels round it (left, top,
/// right, bottom, right and bottom not included).
#[derive(Debug, Clone, PartialEq)]
pub struct Placed<'a> {
    pub drawing: &'a LabelDrawing,
    pub anchor: [f32; 2],
    pub at: [f32; 2],
    pub shrink: [f32; 2],
    pub cos: f32,
    pub sin: f32,
    pub tint: Option<Tint>,
    pub bounds: [i64; 4],
}

impl<'a> Placed<'a> {
    /// `label` drawn from `drawing`, made `drawn_at` times the Text layer's
    /// size, with its anchor `along` the drawing (0 its left end, 1 its
    /// right) and `down` it (0 its top, 1 its bottom). `None` for nothing
    /// to draw.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn new(
        drawing: &'a LabelDrawing,
        label: &Label,
        drawn_at: f32,
        [along, down]: [f32; 2],
    ) -> Option<Self> {
        if drawing.width == 0
            || drawing.height == 0
            || drawn_at <= 0.0
            || !label.at.iter().all(|v| v.is_finite())
        {
            return None;
        }
        let (w, h) = (drawing.width as f32, drawing.height as f32);
        let anchor = [along * w, down * h];
        let shrink = label.scale / drawn_at;
        let (sin, cos) = label.rotation.to_radians().sin_cos();
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for corner in [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]] {
            let q = [
                (corner[0] - anchor[0]) * shrink,
                (corner[1] - anchor[1]) * shrink,
            ];
            let p = [
                label.at[0] + q[0] * cos - q[1] * sin,
                label.at[1] + q[0] * sin + q[1] * cos,
            ];
            for a in 0..2 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let whole = |v: f32| v.clamp(-1e9, 1e9) as i64;
        Some(Self {
            drawing,
            anchor,
            at: label.at,
            shrink: [shrink; 2],
            cos,
            sin,
            tint: label.tint,
            bounds: [
                whole(lo[0].floor()) - 1,
                whole(lo[1].floor()) - 1,
                whole(hi[0].ceil()) + 1,
                whole(hi[1].ceil()) + 1,
            ],
        })
    }

    /// `drawing` stretched to fill `rect` (left, top, right, bottom, px on
    /// the frame) exactly, across and down independently, unturned
    /// (Type fill). `None` for nothing to draw.
    #[must_use]
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
    pub fn boxed(drawing: &'a LabelDrawing, rect: [f32; 4], tint: Option<Tint>) -> Option<Self> {
        let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
        if drawing.width == 0
            || drawing.height == 0
            || w <= 0.0
            || h <= 0.0
            || !rect.iter().all(|v| v.is_finite())
        {
            return None;
        }
        let whole = |v: f32| v.clamp(-1e9, 1e9) as i64;
        Some(Self {
            drawing,
            anchor: [0.0, 0.0],
            at: [rect[0], rect[1]],
            shrink: [w / drawing.width as f32, h / drawing.height as f32],
            cos: 1.0,
            sin: 0.0,
            tint,
            bounds: [
                whole(rect[0].floor()) - 1,
                whole(rect[1].floor()) - 1,
                whole(rect[2].ceil()) + 1,
                whole(rect[3].ceil()) + 1,
            ],
        })
    }

    /// Only the slice between `left` and `right` (px on the frame) drawn:
    /// a word a shape cuts in two (Type fill).
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn clipped_x(mut self, [left, right]: [f32; 2]) -> Self {
        let whole = |v: f32| v.clamp(-1e9, 1e9) as i64;
        self.bounds[0] = self.bounds[0].max(whole(left.round()));
        self.bounds[2] = self.bounds[2].min(whole(right.round()));
        self
    }
}

impl LabelDrawing {
    /// The drawing cut to the box round its letters (Type fill), so
    /// a word stretched into a box fills it edge to edge. An empty drawing
    /// stays empty.
    #[must_use]
    pub fn trimmed(&self) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.px.get(y * self.width + x).is_some_and(|t| t[3] > 0.0) {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        if x1 <= x0 || y1 <= y0 {
            return Self::default();
        }
        let width = x1 - x0;
        let px = (y0..y1)
            .flat_map(|y| (x0..x1).map(move |x| (x, y)))
            .map(|(x, y)| self.px[y * self.width + x])
            .collect();
        Self {
            width,
            height: y1 - y0,
            px,
        }
    }
}

/// The labels laid one over the other, in order, into a picture `size`
/// pixels big whose top left corner is `origin` on the frame: scene-linear,
/// premultiplied RGBA. Each pixel's middle is turned and grown back onto its
/// label's drawing and read between four texels, so labels sit between
/// pixels at any size and turn. Bands of rows are shared out over the cores.
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap
)]
pub fn composite(placed: &[Placed<'_>], origin: [i64; 2], size: [i64; 2]) -> Vec<f32> {
    use rayon::prelude::*;
    const BAND: i64 = 16;
    let [x0, y0] = origin;
    let [bw, bh] = [size[0].max(0), size[1].max(0)];
    let mut px = vec![0.0f32; (bw * bh * 4) as usize];
    if bw == 0 {
        return px;
    }
    px.par_chunks_mut((BAND * bw * 4) as usize)
        .enumerate()
        .for_each(|(band, rows)| {
            let top = y0 + band as i64 * BAND;
            let bottom = top + rows.len() as i64 / (bw * 4);
            for l in placed
                .iter()
                .filter(|l| l.bounds[1] < bottom && l.bounds[3] > top)
            {
                let d = l.drawing;
                let (dw, dh) = (d.width as i64, d.height as i64);
                let texel = |x: i64, y: i64| -> [f32; 4] {
                    if (0..dw).contains(&x) && (0..dh).contains(&y) {
                        d.px[(y * dw + x) as usize]
                    } else {
                        [0.0; 4]
                    }
                };
                for y in l.bounds[1].max(top)..l.bounds[3].min(bottom) {
                    for x in l.bounds[0].max(x0)..l.bounds[2].min(x0 + bw) {
                        let (dx, dy) = (x as f32 + 0.5 - l.at[0], y as f32 + 0.5 - l.at[1]);
                        let u = (dx * l.cos + dy * l.sin) / l.shrink[0] + l.anchor[0] - 0.5;
                        let v = (-dx * l.sin + dy * l.cos) / l.shrink[1] + l.anchor[1] - 0.5;
                        if u <= -1.0 || v <= -1.0 || u >= dw as f32 || v >= dh as f32 {
                            continue;
                        }
                        let (fu, fv) = (u.floor(), v.floor());
                        let (tu, tv) = (u - fu, v - fv);
                        let (iu, iv) = (fu as i64, fv as i64);
                        let (a, b, c, e) = (
                            texel(iu, iv),
                            texel(iu + 1, iv),
                            texel(iu, iv + 1),
                            texel(iu + 1, iv + 1),
                        );
                        let s: [f32; 4] = std::array::from_fn(|n| {
                            let upper = a[n] + (b[n] - a[n]) * tu;
                            let lower = c[n] + (e[n] - c[n]) * tu;
                            upper + (lower - upper) * tv
                        });
                        let alpha = s[3];
                        if alpha <= 0.0 {
                            continue;
                        }
                        // A flashing label keeps its letters' coverage and
                        // takes the flash colour in place of the Text layer's;
                        // one coloured by strength keeps its own, multiplied.
                        let over = match l.tint {
                            Some(Tint::Replace(c)) => [c[0] * alpha, c[1] * alpha, c[2] * alpha],
                            Some(Tint::Multiply(m)) => [s[0] * m[0], s[1] * m[1], s[2] * m[2]],
                            None => [s[0], s[1], s[2]],
                        };
                        let at = (((y - top) * bw + (x - x0)) * 4) as usize;
                        let Some(under) = rows.get_mut(at..at + 4) else {
                            continue;
                        };
                        for (under, top) in under.iter_mut().zip(over.into_iter().chain([alpha])) {
                            *under = top + *under * (1.0 - alpha);
                        }
                    }
                }
            }
        });
    px
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::{composite, LabelDrawing, Placed};
    use crate::fx::effects::label_points::{Label, Tint};

    /// The drawing of labels: a label lands where it is asked, between
    /// pixels when it sits between them, turns about its anchor, shrinks from
    /// its drawing and takes its tint. Guards labels drawn in the wrong
    /// place, snapped back to whole pixels or never turned.
    #[test]
    fn labels_are_drawn_between_pixels_turned_and_tinted() {
        // A 4 x 2 white bar.
        let bar = LabelDrawing {
            width: 4,
            height: 2,
            px: vec![[1.0; 4]; 8],
        };
        let label = |at: [f32; 2], rotation: f32, tint: Option<Tint>| Label {
            text: String::new(),
            at,
            scale: 1.0,
            rotation,
            tint,
        };
        let alpha = |px: &[f32], x: usize, y: usize| px[(y * 10 + x) * 4 + 3];
        // Centred on (5, 5): it covers x 3..7 and y 4..6, nothing above.
        let flat = label([5.0, 5.0], 0.0, None);
        let px = composite(
            &[Placed::new(&bar, &flat, 1.0, [0.5, 0.5]).unwrap()],
            [0, 0],
            [10, 10],
        );
        assert!(alpha(&px, 4, 4) > 0.99 && alpha(&px, 5, 5) > 0.99);
        assert!(alpha(&px, 5, 2) == 0.0 && alpha(&px, 1, 5) == 0.0);
        // Half a pixel across, the edge pixels are half covered.
        let half = label([5.5, 5.0], 0.0, None);
        let px = composite(
            &[Placed::new(&bar, &half, 1.0, [0.5, 0.5]).unwrap()],
            [0, 0],
            [10, 10],
        );
        assert!(
            (alpha(&px, 3, 4) - 0.5).abs() < 0.26,
            "{}",
            alpha(&px, 3, 4)
        );
        // Turned a quarter, it stands up: y 3..7, x 4..6.
        let up = label([5.0, 5.0], 90.0, None);
        let px = composite(
            &[Placed::new(&bar, &up, 1.0, [0.5, 0.5]).unwrap()],
            [0, 0],
            [10, 10],
        );
        assert!(alpha(&px, 4, 3) > 0.99 && alpha(&px, 5, 6) > 0.99);
        assert!(alpha(&px, 2, 5) == 0.0 && alpha(&px, 7, 5) == 0.0);
        // Drawn twice as big as it is shown, it covers half as far.
        let small = label([5.0, 5.5], 0.0, Some(Tint::Multiply([1.0, 0.5, 0.0, 1.0])));
        let placed = Placed::new(&bar, &small, 2.0, [0.5, 0.5]).unwrap();
        let px = composite(&[placed], [0, 0], [10, 10]);
        assert!(alpha(&px, 4, 5) > 0.99 && alpha(&px, 2, 5) == 0.0 && alpha(&px, 4, 3) == 0.0);
        assert_eq!(
            &px[(5 * 10 + 4) * 4..(5 * 10 + 4) * 4 + 3],
            &[1.0, 0.5, 0.0]
        );
    }
}
