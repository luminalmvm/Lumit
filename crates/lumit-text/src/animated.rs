//! Text animators: each letter of a line moved, turned, scaled, faded and
//! tinted on its own, straight or along a path.

use lumit_core::mask::MaskPolyline;
use lumit_core::model::LinearColour;
use lumit_core::text::GlyphXform;

use crate::{
    font, rasterise_line, rasterise_on_path, sample_coverage, walk, RasterText, TextBlock, Walk,
    MAX_PATH_BOX_PX,
};

/// The room an animated line is given round the words, in pixels a side.
///
/// One text size, which is about as far as a letter is usually thrown in a
/// cascade — far enough for a drop-in from above or a hundred per cent scale,
/// and small enough that a title does not quietly become a four-times-larger
/// buffer. A letter thrown further than this is clipped at the layer's edge,
/// exactly as a mask point dragged off the layer is.
///
/// It is a **constant** of the text size rather than a fit to the letters'
/// actual reach, so the layer's box does not change from frame to frame as the
/// range sweeps: a box that breathed would move the picture inside it.
#[must_use]
pub fn animator_margin(size: f32) -> f32 {
    size.clamp(4.0, 512.0).round()
}

/// Where one glyph lands once its animators have moved it: the frame it is
/// drawn in and the box it covers, in raster pixels.
struct Moved {
    origin: [f32; 2],
    along: [f32; 2],
    down: [f32; 2],
    /// The letter's own middle on the baseline, which it turns and scales about.
    anchor: [f32; 2],
    /// The bitmap's top-left corner in the glyph's own pixels.
    corner: [f32; 2],
    /// The skew, as the matrix [xx, yx, xy, yy] applied before the turn.
    skew: [f32; 4],
    bounds: [f32; 4],
}

/// The glyph `metrics` describes at `pen`, moved by `x`, as [`stamp_glyph`]
/// draws it. Nothing for a glyph with no ink, or one scaled or faded to
/// nothing.
fn moved(
    metrics: &fontdue::Metrics,
    pen: [f32; 2],
    tan: [f32; 2],
    nrm: [f32; 2],
    x: &GlyphXform,
) -> Option<Moved> {
    if metrics.width == 0 || metrics.height == 0 {
        return None;
    }
    let (sx, sy) = (x.scale[0], x.scale[1]);
    if x.opacity <= 0.0 || sx.abs() < 1e-4 || sy.abs() < 1e-4 || !x.opacity.is_finite() {
        return None;
    }
    // Skew, about the same middle, before the turn; its inverse undoes it.
    let skew = crate::blur::skew_matrix(x.skew, x.skew_axis).unwrap_or([1.0, 0.0, 0.0, 1.0]);
    let [kxx, kyx, kxy, kyy] = skew;
    #[allow(clippy::cast_precision_loss)]
    let (gw, gh) = (metrics.width as f32, metrics.height as f32);
    let lx0 = metrics.xmin as f32;
    let ly0 = -(metrics.ymin as f32) - gh;
    let anchor = [metrics.advance_width * 0.5, 0.0];

    let (sin, cos) = x.rotation.to_radians().sin_cos();
    let along = [cos * tan[0] + sin * nrm[0], cos * tan[1] + sin * nrm[1]];
    let down = [-sin * tan[0] + cos * nrm[0], -sin * tan[1] + cos * nrm[1]];
    let origin = [
        pen[0] + (x.position[0] + anchor[0]) * tan[0] + (x.position[1] + anchor[1]) * nrm[0],
        pen[1] + (x.position[0] + anchor[0]) * tan[1] + (x.position[1] + anchor[1]) * nrm[1],
    ];
    let to_world = |lx: f32, ly: f32| {
        let (ax, ay) = ((lx - anchor[0]) * sx, (ly - anchor[1]) * sy);
        let (ax, ay) = (kxx * ax + kxy * ay, kyx * ax + kyy * ay);
        [
            origin[0] + ax * along[0] + ay * down[0],
            origin[1] + ax * along[1] + ay * down[1],
        ]
    };
    let corners = [
        to_world(lx0, ly0),
        to_world(lx0 + gw, ly0),
        to_world(lx0, ly0 + gh),
        to_world(lx0 + gw, ly0 + gh),
    ];
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for c in corners {
        bounds = [
            bounds[0].min(c[0]),
            bounds[1].min(c[1]),
            bounds[2].max(c[0]),
            bounds[3].max(c[1]),
        ];
    }
    bounds.iter().all(|v| v.is_finite()).then_some(Moved {
        origin,
        along,
        down,
        anchor,
        corner: [lx0, ly0],
        skew,
        bounds,
    })
}

/// Stamp one glyph into `rgba`, turned, scaled, faded and tinted by `x`.
///
/// `pen` is where the glyph sits on the baseline and `tan`/`nrm` are the
/// directions its own x and y run in there — `(1, 0)` and `(0, 1)` for a
/// straight line, the curve's own frame for a line on a path. Everything the
/// animator does happens **inside that frame**, so a letter on a curve is
/// pushed along and away from the curve rather than across the picture.
///
/// The turn is about the letter's own middle on the baseline, which is where a
/// letter looks like it is pivoting from; pivoting about the pen point swings
/// tall letters out of the line.
///
/// Inverse mapping, the same reason [`rasterise_on_path`] gives: walking the
/// target pixels and asking the glyph what it has there leaves no holes when
/// the turn is not a right angle.
#[allow(clippy::too_many_arguments)]
fn stamp_glyph(
    rgba: &mut [u8],
    w: u32,
    h: u32,
    bitmap: &[u8],
    metrics: &fontdue::Metrics,
    pen: [f32; 2],
    tan: [f32; 2],
    nrm: [f32; 2],
    x: &GlyphXform,
    rgb8: [u8; 3],
) -> Option<[u32; 4]> {
    let Moved {
        origin,
        along,
        down,
        anchor,
        corner: [lx0, ly0],
        skew: [kxx, kyx, kxy, kyy],
        bounds: [x0, y0, x1, y1],
    } = moved(metrics, pen, tan, nrm, x)?;
    let (sx, sy) = (x.scale[0], x.scale[1]);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (px0, py0, px1, py1) = (
        x0.floor().max(0.0) as u32,
        y0.floor().max(0.0) as u32,
        (x1.ceil().max(0.0) as u32).min(w),
        (y1.ceil().max(0.0) as u32).min(h),
    );

    let opacity = x.opacity.clamp(0.0, 1.0);
    for py in py0..py1 {
        for px in px0..px1 {
            #[allow(clippy::cast_precision_loss)]
            let (dx, dy) = (px as f32 + 0.5 - origin[0], py as f32 + 0.5 - origin[1]);
            let (ax, ay) = (dx * along[0] + dy * along[1], dx * down[0] + dy * down[1]);
            // The skew's inverse: its determinant is one.
            let (ax, ay) = (kyy * ax - kxy * ay, -kyx * ax + kxx * ay);
            let u = ax / sx + anchor[0];
            let v = ay / sy + anchor[1];
            let cov = sample_coverage(bitmap, metrics.width, metrics.height, u - lx0, v - ly0);
            if cov == 0 {
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let cov = (f32::from(cov) * opacity).round().clamp(0.0, 255.0) as u8;
            if cov == 0 {
                continue;
            }
            let idx = ((py * w + px) * 4) as usize;
            // Each letter carries its own colour once an animator tints it, so
            // where two letters overlap the one with more ink there wins —
            // rather than the first one's colour keeping the second one's
            // coverage, which is what a plain `max` on alpha would do.
            if cov >= rgba[idx + 3] {
                rgba[idx] = rgb8[0];
                rgba[idx + 1] = rgb8[1];
                rgba[idx + 2] = rgb8[2];
                rgba[idx + 3] = cov;
            }
        }
    }
    Some([px0, py0, px1, py1])
}

/// This letter's fill: the layer's own, plus whatever the animators added,
/// encoded the way every other solid colour in the engine is.
fn tinted(fill: LinearColour, x: &GlyphXform) -> [u8; 3] {
    let c = LinearColour([
        fill.0[0] + x.fill[0],
        fill.0[1] + x.fill[1],
        fill.0[2] + x.fill[2],
        fill.0[3],
    ]);
    let rgba = lumit_core::pixels::solid_rgba(c);
    [rgba[0], rgba[1], rgba[2]]
}

/// A straight line with its animators applied.
///
/// **With no animators this is [`rasterise_line`], byte for byte** — it calls
/// it, rather than taking a second path that happens to agree. That is the
/// guarantee: adding the feature cannot change one pixel of a layer nobody
/// has animated, and cannot retire one frame anybody has cached.
///
/// With animators the box grows by [`animator_margin`] a side and the words sit
/// that far in, so a letter dropping in from above has somewhere to drop from.
#[must_use]
pub fn rasterise_line_animated(
    text: &str,
    size: f32,
    fill: LinearColour,
    xforms: &[GlyphXform],
) -> RasterText {
    rasterise_line_grown(text, size, fill, xforms, [0; 4])
}

/// [`rasterise_line_animated`] with the box grown by `grow` more on the left,
/// top, right and bottom, for letters thrown further than their room.
pub(crate) fn rasterise_line_grown(
    text: &str,
    size: f32,
    fill: LinearColour,
    xforms: &[GlyphXform],
    grow: [u32; 4],
) -> RasterText {
    let rgba8 = lumit_core::pixels::solid_rgba(fill);
    let rgb8 = [rgba8[0], rgba8[1], rgba8[2]];
    if xforms.is_empty() {
        return rasterise_line(text, size, rgb8);
    }
    let font = font();
    let size = size.clamp(4.0, 512.0);
    let margin = animator_margin(size);

    // The same measuring walk as `rasterise_line`, so the un-animated words
    // land in the same place inside the grown box.
    let Walk {
        glyphs,
        pen_x,
        min_y,
        max_y,
    } = walk(text, size);
    if glyphs.is_empty() || min_y > max_y {
        return RasterText {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
    }
    #[allow(clippy::cast_precision_loss)] // px
    let [left, top, right, bottom] = grow.map(|g| g as f32);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width = (pen_x.ceil().max(1.0) + 2.0 * margin + left + right).min(MAX_PATH_BOX_PX) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height =
        ((max_y - min_y).ceil().max(1.0) + 2.0 * margin + top + bottom).min(MAX_PATH_BOX_PX) as u32;
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    let mut scratch: Vec<u8> = Vec::new();
    // Tracking an animator adds pushes on the letters after.
    let mut tracked = 0.0f32;

    for (i, (ch, x0, metrics)) in glyphs.into_iter().enumerate() {
        let (_, bitmap) = font.rasterize(ch, size);
        let x = xforms.get(i).copied().unwrap_or_default();
        let blurred = x.blur.is_finite() && x.blur >= 0.5;
        if blurred && scratch.is_empty() {
            scratch = vec![0u8; rgba.len()];
        }
        let rect = stamp_glyph(
            if blurred { &mut scratch } else { &mut rgba },
            width,
            height,
            &bitmap,
            &metrics,
            [x0 + tracked + margin + left, -min_y + margin + top],
            [1.0, 0.0],
            [0.0, 1.0],
            &x,
            tinted(fill, &x),
        );
        if let (true, Some(rect)) = (blurred, rect) {
            crate::blur::blur_over(&mut scratch, &mut rgba, (width, height), rect, x.blur);
        }
        if x.tracking.is_finite() {
            tracked += x.tracking * size / 1000.0;
        }
    }
    RasterText {
        width,
        height,
        rgba,
    }
}

/// The most raster pixels a grown box may hold, the most a shape layer is
/// drawn at.
const GROWN_PIXELS: f64 = 4096.0 * 4096.0;

/// How far past their room a block's animated letters reach at this frame, in
/// whole layer pixels: left, top, right and bottom. Nothing when they stay
/// inside it, which is every block nobody animates.
#[must_use]
pub fn overhang(block: &TextBlock<'_>, xforms: &[GlyphXform]) -> [u32; 4] {
    if xforms.is_empty() {
        [0; 4]
    } else if block.plain() {
        line_reach(block.text, block.size, xforms)
    } else {
        crate::styled::reach::overhang(block, xforms)
    }
}

/// [`overhang`] for an unstyled line, from each letter's own ink.
fn line_reach(text: &str, size: f32, xforms: &[GlyphXform]) -> [u32; 4] {
    let size = size.clamp(4.0, 512.0);
    let margin = animator_margin(size);
    let Walk {
        glyphs,
        pen_x,
        min_y,
        max_y,
    } = walk(text, size);
    if glyphs.is_empty() || min_y > max_y {
        return [0; 4];
    }
    let width = pen_x.ceil().max(1.0) + 2.0 * margin;
    let height = (max_y - min_y).ceil().max(1.0) + 2.0 * margin;
    let mut ink = [0.0, 0.0, width, height];
    let mut tracked = 0.0f32;
    for (i, (_, x0, metrics)) in glyphs.iter().enumerate() {
        let x = xforms.get(i).copied().unwrap_or_default();
        let pen = [x0 + tracked + margin, -min_y + margin];
        if let Some(m) = moved(metrics, pen, [1.0, 0.0], [0.0, 1.0], &x) {
            ink = joined(ink, m.bounds);
        }
        if x.tracking.is_finite() {
            tracked += x.tracking * size / 1000.0;
        }
    }
    beyond(ink, (width, height))
}

/// The box round two boxes, each left, top, right, bottom.
pub(crate) fn joined(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// How far `ink` reaches outside a `w` by `h` box, in whole pixels a side.
pub(crate) fn beyond(ink: [f32; 4], (w, h): (f32, f32)) -> [u32; 4] {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
    let whole = |p: f32| p.ceil().clamp(0.0, MAX_PATH_BOX_PX) as u32;
    [-ink[0], -ink[1], ink[2] - w, ink[3] - h].map(whole)
}

/// `reach` cut down, every side alike, so a `w` by `h` box grown by it and
/// drawn at `zoom` stays within the most a raster may hold. A letter thrown
/// further than that is clipped at the layer's edge, as it always was.
#[must_use]
pub fn trimmed(reach: [u32; 4], (w, h): (u32, u32), zoom: f32) -> [u32; 4] {
    let (w, h) = (f64::from(w), f64::from(h));
    let ex = f64::from(reach[0]) + f64::from(reach[2]);
    let ey = f64::from(reach[1]) + f64::from(reach[3]);
    if ex + ey <= 0.0 {
        return reach;
    }
    let z = f64::from(zoom.max(1.0));
    let (most, side) = (GROWN_PIXELS / (z * z), f64::from(MAX_PATH_BOX_PX) / z);
    // The share of the reach that fits: (w + f ex)(h + f ey) = most.
    let (a, b, c) = (ex * ey, w * ey + h * ex, w * h - most);
    let mut share = if c >= 0.0 {
        0.0
    } else if a > 0.0 {
        (-b + (b * b - 4.0 * a * c).sqrt()) / (2.0 * a)
    } else {
        -c / b
    };
    if ex > 0.0 {
        share = share.min((side - w) / ex);
    }
    if ey > 0.0 {
        share = share.min((side - h) / ey);
    }
    let share = share.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // clamped
    let cut = |r: u32| (f64::from(r) * share).floor() as u32;
    reach.map(cut)
}

/// A line on a path with its animators applied.
///
/// With no animators this is [`rasterise_on_path`], byte for byte, for the
/// reason [`rasterise_line_animated`] gives. The box is the one `path_box`
/// already hands out — its corner is the layer's own origin and it already
/// carries a text size of room, so an animated line on a curve neither grows
/// nor moves the layer.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn rasterise_on_path_animated(
    text: &str,
    size: f32,
    fill: LinearColour,
    path: &MaskPolyline,
    offset: f32,
    width: u32,
    height: u32,
    xforms: &[GlyphXform],
) -> RasterText {
    let rgba8 = lumit_core::pixels::solid_rgba(fill);
    let rgb8 = [rgba8[0], rgba8[1], rgba8[2]];
    if xforms.is_empty() {
        return rasterise_on_path(text, size, rgb8, path, offset, width, height);
    }
    let font = font();
    let size = size.clamp(4.0, 512.0);
    let (w, h) = (width.max(1), height.max(1));
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    let total = path.length();
    if path.is_empty() || !total.is_finite() {
        return RasterText {
            width: w,
            height: h,
            rgba,
        };
    }

    let mut pen = 0.0f32;
    let mut scratch: Vec<u8> = Vec::new();
    for (i, ch) in text.chars().enumerate() {
        let (metrics, bitmap) = font.rasterize(ch, size);
        let x = xforms.get(i).copied().unwrap_or_default();
        let wanted = offset + pen;
        pen += metrics.advance_width;
        if x.tracking.is_finite() {
            pen += x.tracking * size / 1000.0;
        }
        let s = if path.closed {
            if total > 0.0 {
                wanted.rem_euclid(total)
            } else {
                0.0
            }
        } else if wanted < 0.0 || wanted > total {
            continue;
        } else {
            wanted
        };
        let tan = path.tangent_at(s);
        let blurred = x.blur.is_finite() && x.blur >= 0.5;
        if blurred && scratch.is_empty() {
            scratch = vec![0u8; rgba.len()];
        }
        let rect = stamp_glyph(
            if blurred { &mut scratch } else { &mut rgba },
            w,
            h,
            &bitmap,
            &metrics,
            path.point_at(s),
            tan,
            [-tan[1], tan[0]],
            &x,
            tinted(fill, &x),
        );
        if let (true, Some(rect)) = (blurred, rect) {
            crate::blur::blur_over(&mut scratch, &mut rgba, (w, h), rect, x.blur);
        }
    }
    RasterText {
        width: w,
        height: h,
        rgba,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// A letter thrown past the line's room grows the raster on that side,
    /// rather than being clipped, and the letters inside it keep their pixels.
    #[test]
    fn a_letter_thrown_past_its_room_is_drawn_not_clipped() {
        let white = LinearColour([1.0; 4]);
        let mut xforms = vec![GlyphXform::default(); 2];
        xforms[1].position = [400.0, 0.0];
        let reach = line_reach("AB", 48.0, &xforms);
        assert!(
            reach[2] > 300 && [reach[0], reach[1], reach[3]] == [0; 3],
            "{reach:?}"
        );
        let boxed = rasterise_line_animated("AB", 48.0, white, &xforms);
        let grown = rasterise_line_grown("AB", 48.0, white, &xforms, reach);
        assert_eq!(
            (grown.width, grown.height),
            (boxed.width + reach[2], boxed.height)
        );
        let alpha = |r: &RasterText, x: u32, y: u32| r.rgba[((y * r.width + x) * 4 + 3) as usize];
        let past = (boxed.width..grown.width)
            .flat_map(|x| (0..grown.height).map(move |y| (x, y)))
            .filter(|&(x, y)| alpha(&grown, x, y) > 0)
            .count();
        assert!(past > 100, "the thrown letter is drawn past the old edge");
        for y in 0..boxed.height {
            for x in 0..boxed.width {
                assert_eq!(
                    alpha(&grown, x, y),
                    alpha(&boxed, x, y),
                    "moved at {x}, {y}"
                );
            }
        }
    }
}
