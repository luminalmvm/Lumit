//! Text rasterisation (docs/03-DATA-MODEL.md §9.1, Phase 1 v1 scope).
//!
//! In plain terms: turning a line of text into pixels. v1 is one embedded
//! font (Inter, OFL-licensed, vendored in assets/fonts per the household's
//! self-hosted rule), single-style runs, simple advance-based layout. Styled
//! runs, font selection, shaping/kerning via a full text stack (cosmic-text)
//! and per-character animators follow the data model doc.
//!
//! **A line can also run along a path** (K-607). The advance walk is the same
//! one — each glyph steps the pen by its own advance — but the pen measures
//! **arc length along a curve** instead of a distance to the right, and each
//! glyph is stamped turned to the direction the curve is running in there. The
//! curve is a mask's own flattened polyline, so there is one arc-length walk in
//! this engine and Stroke, the emitters and a line of type all read it.

use std::sync::OnceLock;

use lumit_core::mask::MaskPolyline;

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
/// straight-alpha RGBA8, the layer's own pixels (K-607).
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

    // ---- Text on a path (K-607) ------------------------------------------

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
