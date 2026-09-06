//! Text rasterisation (docs/03-DATA-MODEL.md §9.1, Phase 1 v1 scope).
//!
//! In plain terms: turning a line of text into pixels. v1 is one embedded
//! font (Inter, OFL-licensed, vendored in assets/fonts per the household's
//! self-hosted rule), single-style runs, simple advance-based layout. Styled
//! runs, font selection, shaping/kerning via a full text stack (cosmic-text)
//! and per-character animators follow the data model doc.
//!
//! **A line can also run along a path**. The advance walk is the same
//! one — each glyph steps the pen by its own advance — but the pen measures
//! **arc length along a curve** instead of a distance to the right, and each
//! glyph is stamped turned to the direction the curve is running in there. The
//! curve is a mask's own flattened polyline, so there is one arc-length walk in
//! this engine and Stroke, the emitters and a line of type all read it.

use std::sync::OnceLock;

use lumit_core::mask::{BezierPath, MaskPolyline, Vertex};
use lumit_core::model::LinearColour;
use lumit_core::text::GlyphXform;

/// Inter Regular, embedded at compile time — deterministic across machines.
static INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.otf");

fn font() -> &'static fontdue::Font {
    static FONT: OnceLock<fontdue::Font> = OnceLock::new();
    FONT.get_or_init(|| {
        #[allow(clippy::expect_used)] // compile-time asset; failure = broken build
        fontdue::Font::from_bytes(INTER, fontdue::FontSettings::default())
            .expect("embedded Inter font parses")
    })
}

/// A rasterised line: straight-alpha RGBA8, tightly cropped.
pub struct RasterText {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Measure and rasterise a single line at `size` px with the given linear
/// colour (encoded to sRGB bytes here; alpha carries the glyph coverage).
pub fn rasterise_line(text: &str, size: f32, rgb8: [u8; 3]) -> RasterText {
    let font = font();
    let size = size.clamp(4.0, 512.0);

    // First pass: measure.
    let mut pen_x = 0.0f32;
    let mut glyphs = Vec::new();
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, size);
        let top = -(metrics.ymin as f32) - metrics.height as f32;
        min_y = min_y.min(top);
        max_y = max_y.max(top + metrics.height as f32);
        glyphs.push((ch, pen_x, metrics));
        pen_x += metrics.advance_width;
    }
    if glyphs.is_empty() || min_y > max_y {
        return RasterText {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
    }
    let width = pen_x.ceil().max(1.0) as u32;
    let height = (max_y - min_y).ceil().max(1.0) as u32;

    // Second pass: blit coverage into the buffer.
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for (ch, x0, metrics) in glyphs {
        let (_, bitmap) = font.rasterize(ch, size);
        let gx = x0.round() as i64 + i64::from(metrics.xmin);
        let gy = (-(metrics.ymin as f32) - metrics.height as f32 - min_y).round() as i64;
        for row in 0..metrics.height {
            for col in 0..metrics.width {
                let px = gx + col as i64;
                let py = gy + row as i64;
                if px < 0 || py < 0 || px >= i64::from(width) || py >= i64::from(height) {
                    continue;
                }
                let cov = bitmap[row * metrics.width + col];
                let idx = ((py as u32 * width + px as u32) * 4) as usize;
                rgba[idx] = rgb8[0];
                rgba[idx + 1] = rgb8[1];
                rgba[idx + 2] = rgb8[2];
                rgba[idx + 3] = rgba[idx + 3].max(cov);
            }
        }
    }
    RasterText {
        width,
        height,
        rgba,
    }
}

/// The widest a line on a path is allowed to make its layer, in pixels either
/// way — a mask dragged somewhere absurd must not ask for an absurd buffer
/// (docs/14 §5, budgeted allocations).
const MAX_PATH_BOX_PX: f32 = 16_384.0;

/// The pixel box a line running along `path` is drawn into, at text `size`.
///
/// **The origin stays at the layer's own (0, 0)**, which is the whole reason
/// this is not the path's bounding box: the path *is* one of the layer's masks,
/// and its vertices are in layer pixels measured from that corner. Moving the
/// corner to fit the curve would move every mask on the layer with it. So the
/// box grows to reach the far side of the curve, plus one text size of room for
/// what sits above and below the baseline, and a curve dragged to a negative
/// coordinate is off the layer exactly as a mask point there already is.
#[must_use]
pub fn path_box(path: &MaskPolyline, size: f32) -> (u32, u32) {
    let (mut mx, mut my) = (1.0f32, 1.0f32);
    for p in &path.points {
        if p[0].is_finite() && p[1].is_finite() {
            mx = mx.max(p[0]);
            my = my.max(p[1]);
        }
    }
    let round = |v: f32| (v + size).ceil().clamp(1.0, MAX_PATH_BOX_PX) as u32;
    (round(mx), round(my))
}

/// Lay `text` along `path` and stamp it into a `width` × `height` buffer —
/// straight-alpha RGBA8, the layer's own pixels.
///
/// `offset` slides the whole line along the curve in the same pixels the curve
/// is measured in (px@comp). A **closed** path wraps, so sliding a ring of type
/// runs it round for ever; an **open** one simply drops the glyphs that fall off
/// either end, which is what running out of curve looks like rather than a pile
/// of letters at the last vertex.
///
/// An empty polyline draws nothing at all — the caller lays the line straight
/// instead, and never arrives here with a path that names nothing.
#[must_use]
pub fn rasterise_on_path(
    text: &str,
    size: f32,
    rgb8: [u8; 3],
    path: &MaskPolyline,
    offset: f32,
    width: u32,
    height: u32,
) -> RasterText {
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
    for ch in text.chars() {
        let (metrics, bitmap) = font.rasterize(ch, size);
        let wanted = offset + pen;
        pen += metrics.advance_width;
        if metrics.width == 0 || metrics.height == 0 {
            continue; // a space carries advance and no ink
        }
        // Where this glyph's origin sits on the curve. A closed path has no
        // ends to fall off, so it wraps; an open one drops what does not fit.
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
        let o = path.point_at(s);
        let tan = path.tangent_at(s);
        // The baseline normal, in the layer's y-down pixels: +x turned a
        // quarter turn the way the picture's y runs, so a positive local y is
        // *below* the baseline exactly as it is in the straight layout.
        let nrm = [-tan[1], tan[0]];

        // The glyph's own box in baseline coordinates: x from the bearing,
        // y measured down from the baseline (fontdue's `ymin` is the descent).
        let (gw, gh) = (metrics.width as f32, metrics.height as f32);
        let lx0 = metrics.xmin as f32;
        let ly0 = -(metrics.ymin as f32) - gh;
        let to_world = |lx: f32, ly: f32| {
            [
                o[0] + lx * tan[0] + ly * nrm[0],
                o[1] + lx * tan[1] + ly * nrm[1],
            ]
        };
        // The turned glyph's axis-aligned footprint, clipped to the buffer.
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
            continue;
        }
        let px0 = x0.floor().max(0.0) as u32;
        let py0 = y0.floor().max(0.0) as u32;
        let px1 = (x1.ceil().max(0.0) as u32).min(w);
        let py1 = (y1.ceil().max(0.0) as u32).min(h);

        // Inverse mapping — walk the target pixels and ask the glyph what it
        // has there — rather than scattering the glyph's pixels forward, which
        // leaves holes the moment the turn is not a right angle.
        for py in py0..py1 {
            for px in px0..px1 {
                let dx = px as f32 + 0.5 - o[0];
                let dy = py as f32 + 0.5 - o[1];
                let lx = dx * tan[0] + dy * tan[1];
                let ly = dx * nrm[0] + dy * nrm[1];
                let cov =
                    sample_coverage(&bitmap, metrics.width, metrics.height, lx - lx0, ly - ly0);
                if cov == 0 {
                    continue;
                }
                let idx = ((py * w + px) * 4) as usize;
                rgba[idx] = rgb8[0];
                rgba[idx + 1] = rgb8[1];
                rgba[idx + 2] = rgb8[2];
                rgba[idx + 3] = rgba[idx + 3].max(cov);
            }
        }
    }

    RasterText {
        width: w,
        height: h,
        rgba,
    }
}

/// Bilinear coverage from a glyph bitmap at continuous `(fx, fy)`, where a
/// whole number lands on a pixel's top-left corner. Anything outside the bitmap
/// reads as nothing, so a turned glyph fades at its own edge instead of
/// smearing the last row along the curve.
fn sample_coverage(bitmap: &[u8], bw: usize, bh: usize, fx: f32, fy: f32) -> u8 {
    if bw == 0 || bh == 0 {
        return 0;
    }
    let (x, y) = (fx - 0.5, fy - 0.5);
    let (ix, iy) = (x.floor(), y.floor());
    if ix < -1.0 || iy < -1.0 || ix >= bw as f32 || iy >= bh as f32 {
        return 0;
    }
    let (tx, ty) = (x - ix, y - iy);
    let at = |cx: f32, cy: f32| -> f32 {
        if cx < 0.0 || cy < 0.0 || cx >= bw as f32 || cy >= bh as f32 {
            return 0.0;
        }
        f32::from(bitmap[cy as usize * bw + cx as usize])
    };
    let top = at(ix, iy) * (1.0 - tx) + at(ix + 1.0, iy) * tx;
    let bottom = at(ix, iy + 1.0) * (1.0 - tx) + at(ix + 1.0, iy + 1.0) * tx;
    (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8
}

// ---- Text animators -----------------------------------------------------

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
    let mut pen_x = 0.0f32;
    let mut glyphs = Vec::new();
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, size);
        #[allow(clippy::cast_precision_loss)]
        let top = -(metrics.ymin as f32) - metrics.height as f32;
        min_y = min_y.min(top);
        #[allow(clippy::cast_precision_loss)]
        let bottom = top + metrics.height as f32;
        max_y = max_y.max(bottom);
        glyphs.push((ch, pen_x, metrics));
        pen_x += metrics.advance_width;
    }
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

// ---- Glyph outlines -----------------------------------------------------

/// One glyph's curves, in the same layer pixels the raster would be drawn into.
///
/// A glyph is **several** contours whenever it has a counter — the ring of an
/// `o`, the two eyes of an `8` — and the outer and inner rings wind opposite
/// ways in the font. They are kept together here because what makes the hole is
/// the pair, so whatever draws them has to know which contours belong to one
/// letter.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphOutline {
    pub ch: char,
    pub contours: Vec<BezierPath>,
}

/// Where one glyph sits: its origin, and the two directions its own x and y run
/// in there. Straight layout is the unturned frame; a line on a path gets one
/// frame per glyph from the curve.
#[derive(Clone, Copy)]
struct Frame {
    origin: [f32; 2],
    along: [f32; 2],
    down: [f32; 2],
}

impl Frame {
    fn point(&self, lx: f32, ly: f32) -> (f64, f64) {
        (
            f64::from(self.origin[0] + lx * self.along[0] + ly * self.down[0]),
            f64::from(self.origin[1] + lx * self.along[1] + ly * self.down[1]),
        )
    }
}

/// The outlines of `text` at `size`, laid out **exactly where the rasteriser
/// would put the ink** — straight, or along `path` when one is given.
///
/// The same advance walk as [`rasterise_line`] and [`rasterise_on_path`], read
/// from the same metrics, so a converted layer sits on top of the layer it came
/// from rather than near it. Empty text, and a font that cannot be parsed, both
/// come back as an empty list: nothing to convert is not an error.
#[must_use]
pub fn glyph_outlines(
    text: &str,
    size: f32,
    path: Option<&MaskPolyline>,
    offset: f32,
) -> Vec<GlyphOutline> {
    let font = font();
    let size = size.clamp(4.0, 512.0);
    let Ok(face) = ttf_parser::Face::parse(INTER, 0) else {
        return Vec::new();
    };
    let upem = f32::from(face.units_per_em());
    if upem <= 0.0 {
        return Vec::new();
    }
    let scale = size / upem;

    // The straight layout's own top edge, so a converted line lands on the
    // raster it replaces rather than a text size above it.
    let mut min_y = f32::MAX;
    for ch in text.chars() {
        let (m, _) = font.rasterize(ch, size);
        min_y = min_y.min(-(m.ymin as f32) - m.height as f32);
    }
    if min_y == f32::MAX {
        return Vec::new();
    }
    let total = path.map_or(0.0, MaskPolyline::length);

    let mut out = Vec::new();
    let mut pen = 0.0f32;
    for ch in text.chars() {
        let (metrics, _) = font.rasterize(ch, size);
        let wanted = offset + pen;
        pen += metrics.advance_width;
        let frame = match path {
            None => Frame {
                origin: [wanted, -min_y],
                along: [1.0, 0.0],
                down: [0.0, 1.0],
            },
            Some(p) if !p.is_empty() => {
                let s = if p.closed {
                    if total > 0.0 {
                        wanted.rem_euclid(total)
                    } else {
                        0.0
                    }
                } else if wanted < 0.0 || wanted > total {
                    continue; // ran off the end, exactly as the raster does
                } else {
                    wanted
                };
                let tan = p.tangent_at(s);
                Frame {
                    origin: p.point_at(s),
                    along: tan,
                    down: [-tan[1], tan[0]],
                }
            }
            // A path that names nothing lays straight, the one reading the
            // whole feature keeps.
            Some(_) => Frame {
                origin: [wanted, -min_y],
                along: [1.0, 0.0],
                down: [0.0, 1.0],
            },
        };
        let Some(id) = face.glyph_index(ch) else {
            continue;
        };
        let mut outliner = Outliner {
            scale,
            frame,
            contours: Vec::new(),
            current: Vec::new(),
            start: (0.0, 0.0),
        };
        if face.outline_glyph(id, &mut outliner).is_none() {
            continue; // a space, or a glyph the font draws with nothing
        }
        outliner.finish();
        if !outliner.contours.is_empty() {
            out.push(GlyphOutline {
                ch,
                contours: outliner.contours,
            });
        }
    }
    out
}

/// Turns `ttf-parser`'s pen strokes into the document's own `BezierPath`.
///
/// Two conversions happen on the way: the font's y runs **up** and the
/// picture's runs down, and a **quadratic** curve — what a TrueType glyph is
/// made of — becomes the cubic every path in this document is made of, by the
/// exact equivalence (the two cubic handles sit two thirds of the way to the
/// quadratic's single control point). Nothing is approximated.
struct Outliner {
    scale: f32,
    frame: Frame,
    contours: Vec<BezierPath>,
    current: Vec<Vertex>,
    start: (f64, f64),
}

impl Outliner {
    /// A font point, placed.
    fn at(&self, x: f32, y: f32) -> (f64, f64) {
        self.frame.point(x * self.scale, -y * self.scale)
    }

    /// A font-space offset, turned. `from` is the point it leaves.
    fn handle(&self, from: (f64, f64), cx: f32, cy: f32) -> (f64, f64) {
        let c = self.at(cx, cy);
        (c.0 - from.0, c.1 - from.1)
    }

    fn push(&mut self, pos: (f64, f64), tan_in: (f64, f64)) {
        self.current.push(Vertex {
            pos,
            tan_in,
            tan_out: (0.0, 0.0),
        });
    }

    /// Close the contour being walked, if there is one.
    ///
    /// A font's contour ends where it started, so the last vertex is usually the
    /// first one over again: it is dropped and its incoming handle moves onto
    /// the first vertex, which is what makes the join a curve rather than a
    /// corner.
    fn end_contour(&mut self) {
        let mut vertices = std::mem::take(&mut self.current);
        if vertices.len() >= 2 {
            let last = vertices[vertices.len() - 1];
            let first = vertices[0];
            let same =
                (last.pos.0 - first.pos.0).abs() < 1e-6 && (last.pos.1 - first.pos.1).abs() < 1e-6;
            if same {
                vertices.pop();
                if let Some(v) = vertices.first_mut() {
                    v.tan_in = last.tan_in;
                }
            }
        }
        if vertices.len() >= 3 {
            self.contours.push(BezierPath {
                vertices,
                closed: true,
            });
        }
    }

    fn finish(&mut self) {
        self.end_contour();
    }
}

impl ttf_parser::OutlineBuilder for Outliner {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end_contour();
        self.start = self.at(x, y);
        self.push(self.start, (0.0, 0.0));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.push(p, (0.0, 0.0));
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let Some(previous) = self.current.last().map(|v| v.pos) else {
            return;
        };
        let end = self.at(x, y);
        let control = self.at(x1, y1);
        // The exact quadratic-to-cubic equivalence: both handles two thirds of
        // the way from their own end to the single control point.
        let out = (
            (control.0 - previous.0) * 2.0 / 3.0,
            (control.1 - previous.1) * 2.0 / 3.0,
        );
        let into = (
            (control.0 - end.0) * 2.0 / 3.0,
            (control.1 - end.1) * 2.0 / 3.0,
        );
        if let Some(v) = self.current.last_mut() {
            v.tan_out = out;
        }
        self.push(end, into);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let Some(previous) = self.current.last().map(|v| v.pos) else {
            return;
        };
        let end = self.at(x, y);
        let out = self.handle(previous, x1, y1);
        let into = self.handle(end, x2, y2);
        if let Some(v) = self.current.last_mut() {
            v.tan_out = out;
        }
        self.push(end, into);
    }

    fn close(&mut self) {
        self.end_contour();
    }
}

/// The shape items a Type layer converts to: its glyph outlines, in the
/// layer's own pixels, painted with the layer's fill.
///
/// **A letter's counter is a hole because the contours are combined `Xor`.**
/// This crate's rasteriser fills by the even-odd rule, and `Xor` *is* even-odd,
/// so the ring of an `o` comes out hollow with no winding rule to reason about
/// and no per-glyph special case. Each glyph starts a run of its own, so two
/// letters that happen to overlap union rather than cancel.
#[must_use]
pub fn shape_items_for(
    text: &str,
    size: f32,
    fill: lumit_core::model::LinearColour,
    path: Option<&MaskPolyline>,
    offset: f32,
) -> Vec<lumit_core::shape::ShapeItem> {
    let mut items = Vec::new();
    for glyph in glyph_outlines(text, size, path, offset) {
        for (i, contour) in glyph.contours.into_iter().enumerate() {
            let mut item =
                lumit_core::shape::ShapeItem::filled(glyph.ch.to_string(), contour, fill);
            item.combine = u32::from(i > 0) * 4;
            items.push(item);
        }
    }
    items
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn renders_visible_deterministic_text() {
        let a = rasterise_line("Lumit", 48.0, [255, 255, 255]);
        assert!(a.width > 60 && a.height > 20, "{}x{}", a.width, a.height);
        let ink: u64 = a.rgba.chunks_exact(4).map(|p| u64::from(p[3])).sum();
        assert!(ink > 10_000, "ink {ink}");
        // Deterministic: identical run, identical bytes.
        let b = rasterise_line("Lumit", 48.0, [255, 255, 255]);
        assert_eq!(a.rgba, b.rgba);
    }

    #[test]
    fn empty_text_yields_a_transparent_pixel() {
        let r = rasterise_line("", 48.0, [255, 0, 0]);
        assert_eq!((r.width, r.height), (1, 1));
        assert_eq!(r.rgba[3], 0);
    }

    #[test]
    fn size_scales_the_raster() {
        let small = rasterise_line("Aa", 16.0, [255, 255, 255]);
        let large = rasterise_line("Aa", 64.0, [255, 255, 255]);
        assert!(large.width > small.width * 3);
        assert!(large.height > small.height * 3);
    }

    // ---- Text on a path --------------------------------------------------

    /// A polyline through `points`, arc-length measured the way `flatten_path`
    /// measures one.
    fn poly(points: Vec<[f32; 2]>, closed: bool) -> MaskPolyline {
        let mut arc = Vec::with_capacity(points.len());
        let mut total = 0.0f32;
        arc.push(0.0);
        for w in points.windows(2) {
            total += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            arc.push(total);
        }
        MaskPolyline {
            points,
            arc,
            closed,
            feather: 0.0,
            expansion: 0.0,
        }
    }

    /// The bounding box of everything with any alpha in it, or `None`.
    fn ink_box(r: &RasterText) -> Option<(u32, u32, u32, u32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        for y in 0..r.height {
            for x in 0..r.width {
                if r.rgba[((y * r.width + x) * 4 + 3) as usize] > 0 {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
            }
        }
        (x0 != u32::MAX).then_some((x0, y0, x1, y1))
    }

    /// **The offset is measured in arc length, and nothing else.** Sliding the
    /// line 50 px along a straight path moves its ink exactly 50 px — the one
    /// number this whole layout is built on, so it is asserted to the pixel.
    #[test]
    fn the_offset_walks_the_line_along_the_path_by_arc_length() {
        let path = poly(vec![[10.0, 120.0], [400.0, 120.0]], false);
        let at = |offset: f32| {
            let r = rasterise_on_path("Lu", 48.0, [255, 255, 255], &path, offset, 420, 200);
            ink_box(&r).expect("a straight run of type has ink")
        };
        let (x0, y0, x1, y1) = at(0.0);
        let (sx0, sy0, sx1, sy1) = at(50.0);
        assert_eq!((sx0 - x0, sx1 - x1), (50, 50), "the walk is not arc length");
        assert_eq!((sy0, sy1), (y0, y1), "a straight path moved the baseline");
        // And it starts where the path starts: the first glyph's left bearing
        // is small at 48 px, so the ink begins within a couple of pixels of the
        // path's own first vertex.
        assert!(
            (x0 as i64 - 10).abs() <= 6,
            "the line did not start at the path (x0 {x0}, path starts at 10)"
        );
    }

    /// The baseline **follows the tangent**: the same words on a path running
    /// down the picture come out turned a quarter turn, so the run that was
    /// wide is now tall by the same amount.
    #[test]
    fn a_turned_path_turns_the_glyphs() {
        let across = poly(vec![[20.0, 200.0], [380.0, 200.0]], false);
        let down = poly(vec![[200.0, 20.0], [200.0, 380.0]], false);
        let run = |p: &MaskPolyline| {
            let r = rasterise_on_path("Lumit", 48.0, [255, 255, 255], p, 0.0, 400, 400);
            let (x0, y0, x1, y1) = ink_box(&r).expect("ink");
            (x1 - x0, y1 - y0)
        };
        let (w, h) = run(&across);
        let (tw, th) = run(&down);
        let close = |a: u32, b: u32| (a as i64 - b as i64).abs() <= 2;
        assert!(
            close(w, th) && close(h, tw),
            "across {w}x{h}, down {tw}x{th}"
        );
    }

    /// **A closed path wraps; an open one runs out.** Past the end of an open
    /// curve there is nowhere to put a glyph, so it is not drawn — rather than
    /// piling every remaining letter on the last vertex.
    #[test]
    fn a_closed_path_wraps_where_an_open_one_runs_out() {
        let open = poly(vec![[10.0, 120.0], [200.0, 120.0]], false);
        let ring = poly(
            vec![
                [40.0, 40.0],
                [200.0, 40.0],
                [200.0, 200.0],
                [40.0, 200.0],
                [40.0, 40.0],
            ],
            true,
        );
        let ran_off = rasterise_on_path("Lu", 48.0, [255, 255, 255], &open, 900.0, 260, 200);
        assert_eq!(ink_box(&ran_off), None, "an open path grew a tail");
        let wrapped = rasterise_on_path("Lu", 48.0, [255, 255, 255], &ring, 900.0, 300, 300);
        assert!(ink_box(&wrapped).is_some(), "a ring lost its type");
    }

    /// Nothing to say, and a path to say it on: the layer's own box, empty.
    #[test]
    fn empty_text_on_a_path_draws_nothing() {
        let path = poly(vec![[10.0, 40.0], [180.0, 40.0]], false);
        let r = rasterise_on_path("", 48.0, [255, 0, 0], &path, 0.0, 200, 100);
        assert_eq!((r.width, r.height), (200, 100));
        assert!(
            r.rgba.iter().all(|b| *b == 0),
            "an empty line drew something"
        );
    }

    /// A path that names nothing — the empty polyline every "no mask" reading
    /// comes to — draws nothing rather than faulting (docs/14 §4).
    #[test]
    fn an_empty_path_draws_nothing() {
        let r = rasterise_on_path(
            "Lumit",
            48.0,
            [255, 255, 255],
            &MaskPolyline::default(),
            0.0,
            64,
            64,
        );
        assert_eq!((r.width, r.height), (64, 64));
        assert!(r.rgba.iter().all(|b| *b == 0));
    }

    /// Same document, same frame, same bytes — the rule every rasteriser in
    /// this engine owes the frame cache.
    #[test]
    fn a_line_on_a_path_is_deterministic() {
        let path = poly(vec![[20.0, 100.0], [120.0, 40.0], [240.0, 160.0]], false);
        let go = || rasterise_on_path("Lumit", 36.0, [200, 40, 90], &path, 7.5, 300, 220).rgba;
        assert_eq!(go(), go());
    }

    // ---- Glyph outlines --------------------------------------------------

    /// **The outlines land where the ink lands.** A converted layer that sat a
    /// few pixels off the words it came from would be useless, so the two are
    /// measured against each other: the same line, rasterised twice — once as
    /// glyph coverage, once as the vector art the conversion makes — and the
    /// two pictures have to agree about where the letters are and roughly how
    /// much of them there is. This is the round-trip test for the whole
    /// quadratic-to-cubic conversion; a handle two thirds wrong shows up here
    /// as ink in the wrong place.
    #[test]
    fn the_outlines_land_where_the_ink_lands() {
        let (text, size) = ("Lumit", 96.0f32);
        let raster = rasterise_line(text, size, [255, 255, 255]);
        let items = shape_items_for(
            text,
            size,
            lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0]),
            None,
            0.0,
        );
        assert!(!items.is_empty(), "no outlines came back");

        // Draw the art into the raster's own box, so the two are comparable
        // pixel for pixel.
        let (w, h) = (raster.width, raster.height);
        let art = lumit_core::shape::rasterise_contents(
            &items,
            w,
            h,
            0.0,
            0.0,
            f64::from(w),
            f64::from(h),
            0.0,
        );
        let bbox = |rgba: &[u8]| {
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
            for y in 0..h {
                for x in 0..w {
                    if rgba[((y * w + x) * 4 + 3) as usize] > 8 {
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x);
                        y1 = y1.max(y);
                    }
                }
            }
            (x0, y0, x1, y1)
        };
        let ink =
            |rgba: &[u8]| -> f64 { rgba.chunks_exact(4).map(|p| f64::from(p[3])).sum::<f64>() };
        let (ax0, ay0, ax1, ay1) = bbox(&art);
        let (rx0, ry0, rx1, ry1) = bbox(&raster.rgba);
        let near = |a: u32, b: u32| (a as i64 - b as i64).abs() <= 3;
        assert!(
            near(ax0, rx0) && near(ay0, ry0) && near(ax1, rx1) && near(ay1, ry1),
            "art {ax0},{ay0}..{ax1},{ay1} vs ink {rx0},{ry0}..{rx1},{ry1}"
        );
        let ratio = ink(&art) / ink(&raster.rgba);
        assert!((0.85..1.2).contains(&ratio), "ink ratio {ratio}");
    }

    /// **A counter is a hole.** An `o` is two rings, and the second combines
    /// into the first by `Xor`, which is exactly the even-odd rule this crate's
    /// rasteriser fills by — so the middle comes out empty with no winding rule
    /// to reason about.
    #[test]
    fn a_letter_with_a_counter_is_two_rings_and_a_hole() {
        let items = shape_items_for(
            "o",
            96.0,
            lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0]),
            None,
            0.0,
        );
        assert_eq!(items.len(), 2, "an o is an outer ring and a counter");
        assert_eq!(items[0].combine, 0, "the first ring starts a run");
        assert_eq!(items[1].combine, 4, "the counter cuts by even-odd");

        // And the middle of the drawn `o` really is empty.
        let art =
            lumit_core::shape::rasterise_contents(&items, 100, 100, 0.0, 0.0, 100.0, 100.0, 0.0);
        let centre = ((50 * 100 + 50) * 4 + 3) as usize;
        assert_eq!(art[centre], 0, "the counter filled in");

        // Two letters do not cancel each other: each starts a run of its own.
        let pair = shape_items_for(
            "oo",
            96.0,
            lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0]),
            None,
            0.0,
        );
        assert_eq!(
            pair.iter().filter(|i| i.combine == 0).count(),
            2,
            "the second letter joined the first letter's run"
        );
    }

    /// A line on a path converts **curved**: the outlines take the same frames
    /// the glyphs are stamped in, so the copy sits on the words it came from.
    #[test]
    fn outlines_follow_the_path_the_words_follow() {
        let down = poly(vec![[200.0, 20.0], [200.0, 380.0]], false);
        let straight = shape_items_for(
            "Lumit",
            48.0,
            lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0]),
            None,
            0.0,
        );
        let curved = shape_items_for(
            "Lumit",
            48.0,
            lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0]),
            Some(&down),
            0.0,
        );
        assert_eq!(straight.len(), curved.len(), "the same letters either way");
        let (sx0, sy0, sx1, sy1) =
            lumit_core::shape::contents_bounds(&straight, 0.0).expect("straight art");
        let (cx0, cy0, cx1, cy1) =
            lumit_core::shape::contents_bounds(&curved, 0.0).expect("curved art");
        // Wide across, tall down — the run turned with the curve.
        assert!(sx1 - sx0 > sy1 - sy0, "straight run is not wide");
        assert!(cy1 - cy0 > cx1 - cx0, "the run did not turn");
    }

    /// Nothing to convert is not an error here: an empty line, and a line of
    /// spaces, both come back with no outlines and the command above refuses.
    #[test]
    fn a_line_with_no_ink_has_no_outlines() {
        assert!(glyph_outlines("", 48.0, None, 0.0).is_empty());
        assert!(glyph_outlines("   ", 48.0, None, 0.0).is_empty());
    }

    // ---- Text animators --------------------------------------------------

    fn white() -> lumit_core::model::LinearColour {
        lumit_core::model::LinearColour([1.0, 1.0, 1.0, 1.0])
    }

    /// **A layer with no animators draws exactly what it always drew.** This is
    /// the gate for the whole feature: adding animators to the model must not
    /// change one byte of a line nobody has animated, or every frame every
    /// project has banked is quietly wrong.
    #[test]
    fn a_line_with_no_animators_is_byte_identical() {
        let plain = rasterise_line("Lumit", 48.0, [255, 255, 255]);
        let through = rasterise_line_animated("Lumit", 48.0, white(), &[]);
        assert_eq!((plain.width, plain.height), (through.width, through.height));
        assert_eq!(plain.rgba, through.rgba);

        let path = poly(vec![[20.0, 100.0], [120.0, 40.0], [240.0, 160.0]], false);
        let plain = rasterise_on_path("Lumit", 36.0, [255, 255, 255], &path, 7.5, 300, 220);
        let through = rasterise_on_path_animated("Lumit", 36.0, white(), &path, 7.5, 300, 220, &[]);
        assert_eq!(plain.rgba, through.rgba);
    }

    /// **The weight is what reaches the picture.** An opacity animator over the
    /// first half of the run takes the first half of the letters away and
    /// leaves the rest alone — the shortest end-to-end check that the selector,
    /// the weight and the draw agree about which letter is which.
    #[test]
    fn an_opacity_animator_takes_away_the_letters_the_selector_names() {
        use lumit_core::anim::Property;
        use lumit_core::text::{glyph_xforms, TextAnimator};
        let mut a = TextAnimator::new("Fade");
        a.selector.end = Property::fixed(50.0);
        a.opacity = Property::fixed(0.0);
        let xforms = glyph_xforms(&[a], "AAAA", 0.0);
        let faded = rasterise_line_animated("AAAA", 48.0, white(), &xforms);
        let (x0, _, x1, _) = ink_box(&faded).expect("two letters are still there");
        let whole = rasterise_line_animated("AAAA", 48.0, white(), &[]);
        let (wx0, _, wx1, _) = ink_box(&whole).expect("ink");
        // The right-hand half survived: the ink now starts about half a line
        // in, and still ends where the line ended.
        let width = wx1 - wx0;
        assert!(
            x0 > wx0 + width / 3,
            "the wrong half faded (ink {x0}..{x1}, whole {wx0}..{wx1})"
        );
    }

    /// A push moves the letters it reaches, and the grown box gives them
    /// somewhere to be pushed **to**: a letter lifted a quarter of a text size
    /// is still drawn rather than clipped off the top.
    #[test]
    fn a_push_moves_the_letters_and_the_box_has_room_for_them() {
        use lumit_core::anim::Property;
        use lumit_core::text::{glyph_xforms, TextAnimator};
        let still = rasterise_line_animated("Lu", 48.0, white(), &[]);
        let mut a = TextAnimator::new("Drop");
        a.position_y = Property::fixed(-12.0);
        let xforms = glyph_xforms(&[a], "Lu", 0.0);
        let pushed = rasterise_line_animated("Lu", 48.0, white(), &xforms);
        // One text size of room a side, and the words sit that far in.
        let margin = animator_margin(48.0) as u32;
        assert_eq!(pushed.width, still.width + 2 * margin);
        assert_eq!(pushed.height, still.height + 2 * margin);
        let (_, sy0, _, _) = ink_box(&still).expect("ink");
        let (_, py0, _, _) = ink_box(&pushed).expect("ink");
        // Un-animated the ink would start `margin` further down; the push
        // lifted it 12 px from there, and none of it was lost.
        assert_eq!(py0 as i64, sy0 as i64 + margin as i64 - 12);
    }

    /// Same document, same frame, same bytes — the rule the frame cache lives
    /// by, and the one a per-letter walk is easiest to break.
    #[test]
    fn an_animated_line_is_deterministic() {
        use lumit_core::anim::Property;
        use lumit_core::text::{glyph_xforms, SelectorShape, TextAnimator};
        let mut a = TextAnimator::new("Cascade");
        a.selector.shape = SelectorShape::Ramp;
        a.position_y = Property::fixed(-20.0);
        a.rotation = Property::fixed(35.0);
        a.scale_x = Property::fixed(160.0);
        a.fill_r = Property::fixed(-0.4);
        let xforms = glyph_xforms(&[a], "Lumit", 0.0);
        let go = || rasterise_line_animated("Lumit", 36.0, white(), &xforms).rgba;
        assert_eq!(go(), go());
        assert!(ink_box(&rasterise_line_animated("Lumit", 36.0, white(), &xforms)).is_some());
    }

    /// A letter scaled to nothing, and one faded to nothing, draw nothing —
    /// and neither divides by the zero it was scaled by (docs/14 §4).
    #[test]
    fn a_letter_scaled_or_faded_to_nothing_draws_nothing() {
        use lumit_core::text::GlyphXform;
        let gone = [
            GlyphXform {
                scale: [0.0, 0.0],
                ..GlyphXform::default()
            },
            GlyphXform {
                opacity: 0.0,
                ..GlyphXform::default()
            },
        ];
        let r = rasterise_line_animated("Lu", 48.0, white(), &gone);
        assert_eq!(ink_box(&r), None, "a letter with no size drew something");
    }

    /// The box reaches the far side of the curve with room for the letters,
    /// and keeps its corner at the layer's own origin.
    #[test]
    fn the_path_box_covers_the_curve() {
        let path = poly(vec![[10.0, 20.0], [300.0, 90.0]], false);
        let (w, h) = path_box(&path, 48.0);
        assert_eq!((w, h), (348, 138));
        // A curve dragged somewhere absurd cannot ask for an absurd buffer.
        let wild = poly(vec![[0.0, 0.0], [1.0e9, 1.0e9]], false);
        assert_eq!(path_box(&wild, 48.0), (16_384, 16_384));
    }
}
