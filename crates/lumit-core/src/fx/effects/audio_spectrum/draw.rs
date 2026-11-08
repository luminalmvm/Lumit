//! Audio spectrum's drawing: the reading laid out as straight pieces in raster
//! pixels, and those pieces drawn the way `fx_audio_spectrum.wgsl` draws them.
//!
//! The pieces are filed into the tiles Energy line uses, so a pixel measures only
//! the pieces near it and hundreds of bars cost what a few do.

use crate::fx::effects::energy_line::geometry::{bins, Bins, Pieces};
use crate::fx::points::{PointsStream, Projection};
use crate::mask::MaskPolyline;

use super::{AudioSpectrum, SHAPE_LINE, SHAPE_RING, SHAPE_WAVEFORM};

/// The sides a circle is flattened to. Round to the eye at any height the Height
/// row reaches, and a bar lands on it wherever it falls.
const CIRCLE_SIDES: usize = 180;

/// One frame's drawing.
#[derive(Debug, Clone, Default)]
pub struct Laid {
    /// Three per piece: `(ax, ay, bx, by)`, `(r, g, b at a, 0)`, `(r, g, b at b, 0)`.
    pub pieces: Vec<[f32; 4]>,
    pub tiles: Bins,
    /// Half the width, raster pixels.
    pub half: f32,
    /// How wide the edge's fade is, raster pixels: never under a pixel.
    pub edge: f32,
    pub composite: bool,
    pub mix: f32,
}

/// A straight run from `a` to `b`.
fn line(a: [f32; 2], b: [f32; 2]) -> MaskPolyline {
    MaskPolyline {
        points: vec![a, b],
        arc: vec![0.0, (b[0] - a[0]).hypot(b[1] - a[1])],
        corners: vec![0, 1],
        ..MaskPolyline::default()
    }
}

/// The circle half as wide as From is from To and centred between them, starting
/// at the top and going round with the clock, so a bar's outward is away from the
/// middle. Height then has room inside the frame the line runs across.
fn circle(a: [f32; 2], b: [f32; 2]) -> MaskPolyline {
    let centre = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let r = (b[0] - a[0]).hypot(b[1] - a[1]) / 4.0;
    let mut points = Vec::with_capacity(CIRCLE_SIDES + 1);
    let mut arc = Vec::with_capacity(CIRCLE_SIDES + 1);
    let step = std::f32::consts::TAU / CIRCLE_SIDES as f32;
    for i in 0..=CIRCLE_SIDES {
        let t = -std::f32::consts::FRAC_PI_2 + step * i as f32;
        points.push([centre[0] + r * t.cos(), centre[1] + r * t.sin()]);
        arc.push(r * step * i as f32);
    }
    MaskPolyline {
        points,
        arc,
        closed: true,
        ..MaskPolyline::default()
    }
}

/// `rgb` turned `degrees` round the wheel, about the grey axis, so it needs no
/// clamp to stay continuous and an open-ended value stays open-ended.
fn turned(rgb: [f32; 3], degrees: f32) -> [f32; 3] {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let third = (1.0 - cos) / 3.0;
    let root = sin * (1.0f32 / 3.0).sqrt();
    let (a, b, c) = (cos + third, third - root, third + root);
    [
        rgb[0] * a + rgb[1] * b + rgb[2] * c,
        rgb[0] * c + rgb[1] * a + rgb[2] * b,
        rgb[0] * b + rgb[1] * c + rgb[2] * a,
    ]
}

fn rgb(c: [f32; 4]) -> [f32; 3] {
    [c[0], c[1], c[2]]
}

fn mixed(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// One band's place on the base: the point, the way out from it, and how far
/// round the base it is.
pub(super) struct Place {
    pub point: [f32; 2],
    pub out: [f32; 2],
    pub u: f32,
}

/// Where each band goes, in raster pixels. A closed base shares its last gap with
/// its first, and an open one starts and ends half a gap in.
pub(super) fn places(
    s: &AudioSpectrum,
    px_scale: f32,
    count: usize,
    path: &MaskPolyline,
) -> Vec<Place> {
    let from = [s.from_x, s.from_y];
    let to = [s.to_x, s.to_y];
    let base = if !path.is_empty() {
        path.clone()
    } else if s.shape == SHAPE_RING {
        circle(from, to)
    } else {
        line(from, to)
    };
    let length = base.length();
    if length <= 0.0 || count == 0 {
        return Vec::new();
    }
    (0..count)
        .map(|i| {
            let u = if base.closed {
                i as f32 / count as f32
            } else if s.shape == SHAPE_WAVEFORM || s.shape == SHAPE_LINE {
                i as f32 / (count - 1).max(1) as f32
            } else {
                (i as f32 + 0.5) / count as f32
            };
            let far = u * length;
            let p = base.point_at(far);
            let t = base.tangent_at(far);
            Place {
                point: [p[0] * px_scale, p[1] * px_scale],
                // Left of the way along: up the frame on a line that runs across,
                // and away from the middle on the circle above.
                out: [t[1], -t[0]],
                u,
            }
        })
        .collect()
}

/// What the rows and the reading draw at this raster.
///
/// `levels` is one number per band; a driven Bands that disagrees with it reads
/// the nearest. `path` is a mask in px@comp, or empty for From and To.
#[must_use]
pub fn laid_out(
    s: &AudioSpectrum,
    px_scale: f32,
    levels: &[f32],
    path: &MaskPolyline,
    w: u32,
    h: u32,
) -> Laid {
    let count = s.bands.clamp(2, 512) as usize;
    let at = |i: usize| -> f32 {
        if levels.is_empty() {
            return 0.0;
        }
        levels[(i * levels.len() / count).min(levels.len() - 1)]
    };
    let height = s.height.max(0.0) * px_scale;
    let inside = rgb(s.inside_colour);
    let outside = rgb(s.outside_colour);
    let spread = s.hue_spread;
    let places = places(s, px_scale, count, path);
    let mut out = Pieces::default();
    let mut colours: Vec<[[f32; 3]; 2]> = Vec::new();
    let mut piece = |a: [f32; 2], b: [f32; 2], from: [f32; 3], to: [f32; 3]| {
        out.segs.push([a[0], a[1], b[0], b[1]]);
        out.along.push([0.0; 4]);
        colours.push([from, to]);
    };
    let tip =
        |p: &Place, reach: f32| [p.point[0] + p.out[0] * reach, p.point[1] + p.out[1] * reach];
    let heat = |p: &Place, level: f32| turned(mixed(inside, outside, level.abs()), spread * p.u);
    if s.shape == SHAPE_LINE || s.shape == SHAPE_WAVEFORM {
        // One line through the tips, and a second the other way when mirrored.
        // On a closed base it comes back round to its first tip.
        let closes = path.closed || (path.is_empty() && s.shape == SHAPE_RING);
        let sides: &[f32] = if s.mirror { &[1.0, -1.0] } else { &[1.0] };
        for side in sides {
            let mut last: Option<([f32; 2], [f32; 3])> = None;
            let mut first = None;
            for (i, p) in places.iter().enumerate() {
                let level = at(i);
                let now = (tip(p, side * level * height), heat(p, level));
                if let Some(was) = last {
                    piece(was.0, now.0, was.1, now.1);
                } else {
                    first = Some(now);
                }
                last = Some(now);
            }
            if let (true, Some(was), Some(start)) = (closes, last, first) {
                piece(was.0, start.0, was.1, start.1);
            }
        }
    } else {
        for (i, p) in places.iter().enumerate() {
            let level = at(i).max(0.0);
            let reach = level * height;
            let (near, far) = (turned(inside, spread * p.u), heat(p, level));
            piece(p.point, tip(p, reach), near, far);
            if s.mirror {
                piece(p.point, tip(p, -reach), near, far);
            }
        }
    }
    let half = (s.thickness.max(0.0) * px_scale) / 2.0;
    let edge = (half * 2.0 * (s.softness / 100.0).clamp(0.0, 1.0)).max(1.0);
    let tiles = bins(&out, half + edge / 2.0 + 1.0, w, h);
    let mut pieces = Vec::with_capacity(out.segs.len() * 3);
    for (seg, [a, b]) in out.segs.iter().zip(&colours) {
        pieces.push(*seg);
        pieces.push([a[0], a[1], a[2], 0.0]);
        pieces.push([b[0], b[1], b[2], 0.0]);
    }
    Laid {
        pieces,
        tiles,
        half,
        edge,
        composite: s.composite_on_original,
        mix: (s.mix / 100.0).clamp(0.0, 1.0),
    }
}

/// One point at each band's tip, in px@comp, sized and coloured as it is drawn.
#[must_use]
pub fn tips(
    s: &AudioSpectrum,
    levels: &[f32],
    path: &MaskPolyline,
    projection: Projection,
) -> PointsStream {
    let count = s.bands.clamp(2, 512) as usize;
    let mut out = PointsStream {
        projection,
        ..PointsStream::default()
    };
    let height = s.height.max(0.0);
    for (i, p) in places(s, 1.0, count, path).iter().enumerate() {
        let level = levels
            .get(i * levels.len().max(1) / count)
            .copied()
            .unwrap_or(0.0);
        let reach = level * height;
        let colour = turned(
            mixed(rgb(s.inside_colour), rgb(s.outside_colour), level.abs()),
            s.hue_spread * p.u,
        );
        out.position.push([
            p.point[0] + p.out[0] * reach,
            p.point[1] + p.out[1] * reach,
            0.0,
        ]);
        out.speed.push([0.0; 3]);
        out.age.push(0.0);
        // 1 rather than 0, so a consumer dividing by it reads a young point.
        out.life.push(1.0);
        out.size.push(s.thickness.max(0.0));
        out.rotation.push(p.out[1].atan2(p.out[0]));
        out.colour.push([colour[0], colour[1], colour[2], 1.0]);
        out.id.push(i as u64);
        out.index.push(p.u);
    }
    out
}

/// The piece that covers the pixel most, its coverage and its colour.
fn covered(laid: &Laid, x: u32, y: u32) -> (f32, [f32; 3]) {
    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
    let (mut best, mut colour) = (0.0f32, [0.0f32; 3]);
    for &k in laid.tiles.at(x, y) {
        let k = k as usize * 3;
        let (s, a, b) = (laid.pieces[k], laid.pieces[k + 1], laid.pieces[k + 2]);
        let (dx, dy) = (s[2] - s[0], s[3] - s[1]);
        let (rx, ry) = (px - s[0], py - s[1]);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        let t = ((rx * dx + ry * dy) / len2).clamp(0.0, 1.0);
        let (ox, oy) = (rx - t * dx, ry - t * dy);
        let dist = (ox * ox + oy * oy).sqrt();
        let cover = ((laid.half - dist) / laid.edge + 0.5).clamp(0.0, 1.0);
        if cover > best {
            best = cover;
            colour = [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ];
        }
    }
    (best, colour)
}

/// Draw `laid` over a premultiplied picture, as `fx_audio_spectrum.wgsl` does.
pub fn render(rgba: &mut [f32], w: u32, h: u32, laid: &Laid) {
    if laid.mix <= 0.0 {
        return;
    }
    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) as usize * 4;
            let Some(px) = rgba.get_mut(at..at + 4) else {
                return;
            };
            let (cover, colour) = covered(laid, x, y);
            let keep = if laid.composite { 1.0 - cover } else { 0.0 };
            let (m, un) = (laid.mix, 1.0 - laid.mix);
            for c in 0..3 {
                let drawn = colour[c] * cover + px[c] * keep;
                px[c] = px[c] * un + drawn * m;
            }
            let drawn = cover + px[3] * keep;
            px[3] = px[3] * un + drawn * m;
        }
    }
}
