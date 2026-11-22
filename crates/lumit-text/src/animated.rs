//! Text animators: each letter of a line moved, turned, scaled, faded and
//! tinted on its own, straight or along a path.

use lumit_core::mask::MaskPolyline;
use lumit_core::model::LinearColour;
use lumit_core::text::GlyphXform;

use crate::{
    font, rasterise_line, rasterise_on_path, sample_coverage, walk, RasterText, Walk,
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
) {
    if metrics.width == 0 || metrics.height == 0 {
        return; // a space carries advance and no ink
    }
    let (sx, sy) = (x.scale[0], x.scale[1]);
    if x.opacity <= 0.0 || sx.abs() < 1e-4 || sy.abs() < 1e-4 || !x.opacity.is_finite() {
        return; // scaled or faded to nothing
    }
    #[allow(clippy::cast_precision_loss)]
    let (gw, gh) = (metrics.width as f32, metrics.height as f32);
    let lx0 = metrics.xmin as f32;
    let ly0 = -(metrics.ymin as f32) - gh;
    // The letter's own middle on the baseline: what it turns and scales about.
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
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for c in corners {
        x0 = x0.min(c[0]);
        y0 = y0.min(c[1]);
        x1 = x1.max(c[0]);
        y1 = y1.max(c[1]);
    }
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return;
    }
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
            let u = (dx * along[0] + dy * along[1]) / sx + anchor[0];
            let v = (dx * down[0] + dy * down[1]) / sy + anchor[1];
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
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let width = (pen_x.ceil().max(1.0) + 2.0 * margin).min(MAX_PATH_BOX_PX) as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let height = ((max_y - min_y).ceil().max(1.0) + 2.0 * margin).min(MAX_PATH_BOX_PX) as u32;
    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];

    for (i, (ch, x0, metrics)) in glyphs.into_iter().enumerate() {
        let (_, bitmap) = font.rasterize(ch, size);
        let x = xforms.get(i).copied().unwrap_or_default();
        stamp_glyph(
            &mut rgba,
            width,
            height,
            &bitmap,
            &metrics,
            [x0 + margin, -min_y + margin],
            [1.0, 0.0],
            [0.0, 1.0],
            &x,
            tinted(fill, &x),
        );
    }
    RasterText {
        width,
        height,
        rgba,
    }
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
    for (i, ch) in text.chars().enumerate() {
        let (metrics, bitmap) = font.rasterize(ch, size);
        let wanted = offset + pen;
        pen += metrics.advance_width;
        let s = if path.closed {
            if total > 0.0 {
                wanted.rem_euclid(total)
            } else {
                0.0
            }
        } else if wanted < 0.0 || wanted > total {
            continue; // ran off the end, exactly as the un-animated walk does
        } else {
            wanted
        };
        let tan = path.tangent_at(s);
        let x = xforms.get(i).copied().unwrap_or_default();
        stamp_glyph(
            &mut rgba,
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
    }
    RasterText {
        width: w,
        height: h,
        rgba,
    }
}
