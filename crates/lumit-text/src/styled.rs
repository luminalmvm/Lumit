//! Styled text: a block of lines set in any installed font.
//!
//! The words are shaped a line at a time, so kerning, ligatures and joined
//! scripts come out the way the font means them. Each glyph is then drawn from
//! its outline, which is what lets it be scaled, slanted and outlined without
//! going soft.
//!
//! An unstyled single line never comes here. It takes the path it always took
//! in `lib.rs`, so it draws the same bytes it always drew.
//!
//! ponytail: one direction per line, guessed from its script. A line mixing
//! left-to-right and right-to-left words needs the bidi algorithm.

use std::collections::HashMap;
use std::ops::Range;

use lumit_core::mask::MaskPolyline;
use lumit_core::model::LinearColour;
use lumit_core::text::{Caps, GlyphXform, Kerning, Script, TextAlign};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::MetadataProvider;
use zeno::{Angle, Command, Fill, Join, Mask, PathBuilder, Stroke, Transform};

use crate::fonts::Face;
use crate::{animator_margin, BlockLayout, BlockLine, GlyphOutline, RasterText, TextBlock};

/// The largest a styled letter is set, in px.
const MAX_SIZE: f32 = 2000.0;
/// The most pixels a block is allowed to ask for, so a long title at a huge
/// size can't ask for gigabytes (docs/14 §5).
const MAX_AREA: f64 = 8192.0 * 8192.0;
/// Super and subscript letters are this much of the size, and sit this much of
/// the size off the baseline. After Effects' own numbers.
const SCRIPT_SCALE: f32 = 0.583;
const SCRIPT_SHIFT: f32 = 0.333;
/// Small capitals are capitals at this much of the size.
const SMALL_CAPS_SCALE: f32 = 0.7;
/// How far faux italic leans: the tangent of 12 degrees.
const FAUX_SLANT: f32 = 0.2126;
/// Faux bold thickens every stem by this much of the size.
const FAUX_BOLD: f32 = 0.04;

/// One glyph, placed.
struct Placed {
    glyph: u32,
    /// The character it came from, counted over the whole text. A ligature
    /// belongs to its first character.
    ch: usize,
    line: usize,
    /// The pen position along the line, from the block's left edge.
    x: f32,
    /// How far below the line's baseline the glyph's own baseline sits.
    dy: f32,
    advance: f32,
    /// Px per font unit, before the style's own scale.
    scale: f32,
}

struct Line {
    /// The index of the line's first character in the whole text.
    start: usize,
    baseline: f32,
    /// One x per gap between characters, from the block's left edge.
    carets: Vec<f32>,
}

/// A block laid out, before any pixels.
struct Laid {
    glyphs: Vec<Placed>,
    lines: Vec<Line>,
    /// The words' own box, without the room round it.
    width: f32,
    height: f32,
    /// Room kept round the words for an outline or a slant to draw into.
    pad: f32,
    pad_top: f32,
    pad_bottom: f32,
    ascent: f32,
    descent: f32,
    size: f32,
    scale_x: f32,
    scale_y: f32,
}

impl Laid {
    /// Where the words' box starts inside the raster.
    fn origin(&self, margin: f32) -> [f32; 2] {
        [self.pad + margin, self.pad_top + margin]
    }

    /// The raster's size, with `margin` more a side for animated letters.
    fn raster_size(&self, margin: f32) -> (u32, u32) {
        if self.glyphs.is_empty() {
            return (1, 1);
        }
        let w = f64::from((self.width.ceil() + 2.0 * (self.pad + margin)).max(1.0))
            .min(f64::from(crate::MAX_PATH_BOX_PX));
        let h = f64::from(
            (self.height.ceil() + self.pad_top + self.pad_bottom + 2.0 * margin).max(1.0),
        )
        .min(f64::from(crate::MAX_PATH_BOX_PX))
        .min((MAX_AREA / w).floor().max(1.0));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (w as u32, h as u32)
    }
}

fn clamped_scale(per_cent: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let s = (per_cent / 100.0) as f32;
    if s.is_finite() {
        s.clamp(0.01, 100.0)
    } else {
        1.0
    }
}

fn finite(v: f64) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    let v = v as f32;
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// The text cut into lines. Each is its first character's index and its
/// characters, without the break that ended it.
fn lines_of(text: &str) -> Vec<(usize, Vec<char>)> {
    let mut out = Vec::new();
    let mut current: Vec<char> = Vec::new();
    let mut start = 0usize;
    let mut previous = '\0';
    for (i, ch) in text.chars().enumerate() {
        match ch {
            // The second half of a Windows line ending. The line is already
            // broken, so it only moves where the next one starts.
            '\n' if previous == '\r' => start = i + 1,
            '\n' | '\r' => {
                out.push((start, std::mem::take(&mut current)));
                start = i + 1;
            }
            _ => current.push(ch),
        }
        previous = ch;
    }
    out.push((start, current));
    out
}

fn lay(block: &TextBlock<'_>, face: &Face) -> Laid {
    let style = block.style;
    let paragraph = block.paragraph;
    let size = if block.size.is_finite() {
        block.size.clamp(1.0, MAX_SIZE)
    } else {
        72.0
    };
    let (scale_x, scale_y) = (clamped_scale(style.scale_x), clamped_scale(style.scale_y));
    let upem = f32::from(face.shaper.units_per_em()).max(1.0);
    let unit = size / upem;
    let (ascent, descent) = face.font().map_or((size * 0.8, size * 0.2), |font| {
        let m = font.metrics(
            Size::new(size),
            LocationRef::new(face.shaper.normalized_coords()),
        );
        (m.ascent * scale_y, -m.descent * scale_y)
    });

    let script_scale = match style.script {
        Script::Normal => 1.0,
        Script::Superscript | Script::Subscript => SCRIPT_SCALE,
    };
    // Up is positive here, the way the panel shows it.
    let lift = finite(style.baseline_shift)
        + match style.script {
            Script::Normal => 0.0,
            Script::Superscript => size * SCRIPT_SHIFT * scale_y,
            Script::Subscript => -size * SCRIPT_SHIFT * scale_y,
        };
    let tracking = finite(style.tracking) / 1000.0 * size * scale_x;

    let off = |tag: &[u8; 4]| harfrust::Feature::new(harfrust::Tag::new(tag), 0, ..);
    let mut features = Vec::new();
    if style.kerning == Kerning::Off {
        features.push(off(b"kern"));
    }
    if !style.ligatures {
        features.push(off(b"liga"));
        features.push(off(b"clig"));
    }
    let shaper = harfrust::ShaperFont::new(&face.shaper);

    let mut glyphs: Vec<Placed> = Vec::new();
    // Each line's glyphs, first character, width and carets, all measured from
    // the line's own left end.
    let mut rows: Vec<(usize, Range<usize>, f32, Vec<f32>)> = Vec::new();
    for (line, (start, chars)) in lines_of(block.text).into_iter().enumerate() {
        let mut buffer = harfrust::Buffer::new();
        let mut small = vec![false; chars.len()];
        for (k, ch) in chars.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)] // a line is far shorter than 4 billion
            let cluster = k as u32;
            match style.caps {
                Caps::Normal => buffer.push(u32::from(*ch), cluster),
                Caps::All => {
                    for up in ch.to_uppercase() {
                        buffer.push(u32::from(up), cluster);
                    }
                }
                Caps::Small => {
                    small[k] = ch.is_lowercase();
                    for up in ch.to_uppercase() {
                        buffer.push(u32::from(up), cluster);
                    }
                }
            }
        }
        buffer.guess_segment_properties();
        let backwards = buffer.direction() == harfrust::Direction::RightToLeft;
        let first = glyphs.len();
        let mut pen = 0.0f32;
        // The x each cluster of characters starts and ends at.
        let mut spans: Vec<(usize, f32, f32)> = Vec::new();
        if !chars.is_empty()
            && harfrust::shape(
                &shaper,
                &mut buffer,
                harfrust::ShapeOptions::new().features(&features),
            )
            .is_ok()
        {
            for (info, at) in buffer.glyph_infos().iter().zip(buffer.glyph_positions()) {
                let k = info.cluster as usize;
                let scale = unit
                    * script_scale
                    * if small.get(k).copied().unwrap_or(false) {
                        SMALL_CAPS_SCALE
                    } else {
                        1.0
                    };
                #[allow(clippy::cast_precision_loss)] // font units, far inside f32
                let (advance, dx, dy) = (
                    at.x_advance as f32 * scale * scale_x,
                    at.x_offset as f32 * scale * scale_x,
                    at.y_offset as f32 * scale * scale_y,
                );
                // A mark sits on the letter before it and takes no room, so it
                // takes no tracking either.
                let advance = if advance > 0.0 {
                    advance + tracking
                } else {
                    advance
                };
                glyphs.push(Placed {
                    glyph: info.glyph_id,
                    ch: start + k,
                    line,
                    x: pen + dx,
                    dy: -lift - dy,
                    advance,
                    scale,
                });
                match spans.last_mut() {
                    Some(span) if span.0 == k => span.2 = pen + advance,
                    _ => spans.push((k, pen, pen + advance)),
                }
                pen += advance;
            }
        }
        // Tracking is room between letters, so the last letter's share is
        // given back. A centred line is then centred on its ink.
        let width = if glyphs.len() > first {
            (pen - tracking).max(0.0)
        } else {
            0.0
        };

        // A cluster's characters share its width evenly, which is where a
        // caret stands inside a ligature.
        let mut carets = vec![if backwards { 0.0 } else { width }; chars.len() + 1];
        spans.sort_by_key(|s| s.0);
        for (i, &(k, x0, x1)) in spans.iter().enumerate() {
            let next = spans.get(i + 1).map_or(chars.len(), |s| s.0);
            let count = next.saturating_sub(k).max(1);
            for j in 0..count {
                #[allow(clippy::cast_precision_loss)]
                let t = j as f32 / count as f32;
                if let Some(caret) = carets.get_mut(k + j) {
                    *caret = if backwards {
                        x1 - (x1 - x0) * t
                    } else {
                        x0 + (x1 - x0) * t
                    };
                }
            }
        }
        rows.push((start, first..glyphs.len(), width, carets));
    }

    let (left, right, first_line) = (
        finite(paragraph.indent_left),
        finite(paragraph.indent_right),
        finite(paragraph.indent_first),
    );
    let width = rows
        .iter()
        .map(|row| left + first_line + row.2 + right)
        .fold(0.0f32, f32::max);
    let leading = style
        .leading
        .map_or(size * 1.2, finite)
        .clamp(0.0, MAX_SIZE * 4.0);
    let gap = leading + finite(paragraph.space_before) + finite(paragraph.space_after);

    let mut lines = Vec::with_capacity(rows.len());
    let mut baseline = ascent;
    for (start, range, row_width, mut carets) in rows {
        let free = width - (left + first_line + row_width + right);
        let x = left
            + first_line
            + match paragraph.align {
                TextAlign::Left => 0.0,
                TextAlign::Centre => free * 0.5,
                TextAlign::Right => free,
            };
        for glyph in glyphs.get_mut(range).into_iter().flatten() {
            glyph.x += x;
        }
        for caret in &mut carets {
            *caret += x;
        }
        lines.push(Line {
            start,
            baseline,
            carets,
        });
        baseline += gap;
    }
    let height = lines.last().map_or(ascent, |l| l.baseline) + descent;

    let stroke = if style.stroke_on {
        finite(style.stroke_width).max(0.0) * 0.5
    } else {
        0.0
    };
    let pad = (stroke + size * 0.15 * scale_x.max(scale_y).max(1.0)).ceil();
    Laid {
        glyphs,
        lines,
        width,
        height,
        pad,
        pad_top: pad + lift.max(0.0).ceil(),
        pad_bottom: pad + (-lift).max(0.0).ceil(),
        ascent,
        descent,
        size,
        scale_x,
        scale_y,
    }
}

pub(crate) fn layout(block: &TextBlock<'_>, animated: bool) -> BlockLayout {
    let face = crate::fonts::face(&block.style.family, &block.style.face);
    let laid = lay(block, &face);
    let margin = if animated {
        animator_margin(laid.size)
    } else {
        0.0
    };
    let (width, height) = laid.raster_size(margin);
    let [ox, oy] = laid.origin(margin);
    BlockLayout {
        width,
        height,
        ascent: laid.ascent,
        descent: laid.descent,
        left: ox,
        right: ox + laid.width,
        lines: laid
            .lines
            .iter()
            .map(|line| BlockLine {
                start: line.start,
                baseline: line.baseline + oy,
                carets: line.carets.iter().map(|x| x + ox).collect(),
            })
            .collect(),
    }
}

/// A glyph's outline in font units, y up, as the rasteriser's own commands.
struct Path(Vec<Command>);

impl OutlinePen for Path {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to([x, y]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to([x, y]);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to([cx0, cy0], [x, y]);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to([cx0, cy0], [cx1, cy1], [x, y]);
    }
    fn close(&mut self) {
        self.0.close();
    }
}

/// Font units to the glyph's own px: scaled, slanted if faux italic, and
/// turned the right way up. The origin is the pen position on the baseline.
fn glyph_matrix(glyph: &Placed, laid: &Laid, slanted: bool) -> Transform {
    let slant = if slanted { FAUX_SLANT } else { 0.0 };
    Transform::new(
        laid.scale_x * glyph.scale,
        0.0,
        laid.scale_x * slant * glyph.scale,
        -laid.scale_y * glyph.scale,
        0.0,
        0.0,
    )
}

/// What an animator does to a letter, about its own middle on the baseline.
fn animated(matrix: Transform, advance: f32, x: &GlyphXform) -> Transform {
    let middle = advance * 0.5;
    matrix
        .then_translate(-middle, 0.0)
        .then_scale(x.scale[0], x.scale[1])
        .then_rotate(Angle::from_degrees(x.rotation))
        .then_translate(middle + x.position[0], x.position[1])
}

/// Where each glyph's pen sits and which way its line runs there.
enum Run<'a> {
    Straight { origin: [f32; 2] },
    Along { path: &'a MaskPolyline, offset: f32 },
}

impl Run<'_> {
    /// The glyph's own px to the raster's, or nothing for a glyph that fell
    /// off the end of an open path.
    fn place(&self, laid: &Laid, glyph: &Placed, flat_x: f32) -> Option<Transform> {
        match self {
            Run::Straight { origin } => {
                let baseline = laid.lines.get(glyph.line).map_or(0.0, |l| l.baseline);
                Some(Transform::translation(
                    origin[0] + glyph.x,
                    origin[1] + baseline + glyph.dy,
                ))
            }
            Run::Along { path, offset } => {
                let total = path.length();
                let wanted = offset + flat_x;
                let s = if path.closed {
                    if total > 0.0 {
                        wanted.rem_euclid(total)
                    } else {
                        0.0
                    }
                } else if wanted < 0.0 || wanted > total {
                    return None;
                } else {
                    wanted
                };
                let o = path.point_at(s);
                let tan = path.tangent_at(s);
                Some(
                    Transform::translation(0.0, glyph.dy)
                        .then(&Transform::new(tan[0], tan[1], -tan[1], tan[0], o[0], o[1])),
                )
            }
        }
    }
}

/// Every glyph with the matrix that takes its outline to the raster. On a
/// path the lines run on one after another, since a curve has one baseline.
fn placements(
    laid: &Laid,
    run: &Run<'_>,
    xforms: &[GlyphXform],
    slanted: bool,
) -> Vec<(usize, Transform, GlyphXform)> {
    let mut out = Vec::with_capacity(laid.glyphs.len());
    let mut flat = 0.0f32;
    for (i, glyph) in laid.glyphs.iter().enumerate() {
        let flat_x = flat;
        flat += glyph.advance;
        let x = xforms.get(glyph.ch).copied().unwrap_or_default();
        let (sx, sy) = (x.scale[0], x.scale[1]);
        if !(x.opacity > 0.0 && sx.abs() >= 1e-4 && sy.abs() >= 1e-4) {
            continue; // scaled or faded to nothing
        }
        let Some(place) = run.place(laid, glyph, flat_x) else {
            continue;
        };
        let matrix = animated(glyph_matrix(glyph, laid, slanted), glyph.advance, &x);
        out.push((i, matrix.then(&place), x));
    }
    out
}

/// Lay `colour` over one straight-alpha pixel by `alpha`.
fn over(px: &mut [u8], colour: [u8; 3], alpha: f32) {
    let under = f32::from(px[3]) / 255.0;
    let out = alpha + under * (1.0 - alpha);
    if out <= 0.0 {
        return;
    }
    for (c, top) in colour.iter().enumerate() {
        let mixed = (f32::from(*top) * alpha + f32::from(px[c]) * under * (1.0 - alpha)) / out;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            px[c] = mixed.round().clamp(0.0, 255.0) as u8;
        }
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        px[3] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// Draw one path into the raster, filled or stroked, in one colour.
fn paint<'s>(
    rgba: &mut [u8],
    (w, h): (u32, u32),
    path: &[Command],
    matrix: Transform,
    style: impl Into<zeno::Style<'s>>,
    colour: [u8; 4],
    opacity: f32,
) {
    let alpha = f32::from(colour[3]) / 255.0 * opacity.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let (mask, at) = Mask::new(path)
        .style(style)
        .transform(Some(matrix))
        .render();
    for row in 0..at.height {
        let Some(y) = i64::from(at.top)
            .checked_add(i64::from(row))
            .filter(|y| (0..i64::from(h)).contains(y))
        else {
            continue;
        };
        for col in 0..at.width {
            let x = i64::from(at.left) + i64::from(col);
            if x < 0 || x >= i64::from(w) {
                continue;
            }
            let coverage = mask
                .get((row * at.width + col) as usize)
                .copied()
                .unwrap_or(0);
            if coverage == 0 {
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let i = (y as usize * w as usize + x as usize) * 4;
            if let Some(px) = rgba.get_mut(i..i + 4) {
                over(
                    px,
                    [colour[0], colour[1], colour[2]],
                    f32::from(coverage) / 255.0 * alpha,
                );
            }
        }
    }
}

fn draw(
    block: &TextBlock<'_>,
    face: &Face,
    laid: &Laid,
    run: &Run<'_>,
    xforms: &[GlyphXform],
    (w, h): (u32, u32),
) -> Vec<u8> {
    let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
    let Some(font) = face.font() else {
        return rgba;
    };
    let style = block.style;
    let outlines = font.outline_glyphs();
    let location = LocationRef::new(face.shaper.normalized_coords());
    let mut paths: HashMap<u32, Vec<Command>> = HashMap::new();
    let stroke_colour = lumit_core::pixels::solid_rgba(style.stroke);
    let stroke_width = finite(style.stroke_width).max(0.0);

    for (i, matrix, x) in placements(laid, run, xforms, style.faux_italic) {
        let Some(glyph) = laid.glyphs.get(i) else {
            continue;
        };
        let path = paths.entry(glyph.glyph).or_insert_with(|| {
            let mut pen = Path(Vec::new());
            if let Some(outline) = outlines.get(skrifa::GlyphId::new(glyph.glyph)) {
                // A glyph that fails to draw is left out, the frame carries on.
                let _ = outline.draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen);
            }
            pen.0
        });
        if path.is_empty() {
            continue; // a space carries advance and no ink
        }
        let fill = {
            let tinted = LinearColour([
                block.fill.0[0] + x.fill[0],
                block.fill.0[1] + x.fill[1],
                block.fill.0[2] + x.fill[2],
                block.fill.0[3],
            ]);
            lumit_core::pixels::solid_rgba(tinted)
        };
        let mut outline = Stroke::new(stroke_width);
        outline.join(Join::Miter).miter_limit(4.0).scale(false);
        let mut bold = Stroke::new(laid.size * FAUX_BOLD);
        bold.join(Join::Round).scale(false);

        let stroked = style.stroke_on && stroke_width > 0.0;
        if stroked && !style.stroke_over {
            paint(
                &mut rgba,
                (w, h),
                path,
                matrix,
                outline,
                stroke_colour,
                x.opacity,
            );
        }
        if style.fill_on {
            if style.faux_bold {
                paint(&mut rgba, (w, h), path, matrix, bold, fill, x.opacity);
            }
            paint(
                &mut rgba,
                (w, h),
                path,
                matrix,
                Fill::NonZero,
                fill,
                x.opacity,
            );
        }
        if stroked && style.stroke_over {
            paint(
                &mut rgba,
                (w, h),
                path,
                matrix,
                outline,
                stroke_colour,
                x.opacity,
            );
        }
    }
    rgba
}

pub(crate) fn rasterise(block: &TextBlock<'_>, xforms: &[GlyphXform]) -> RasterText {
    let face = crate::fonts::face(&block.style.family, &block.style.face);
    let laid = lay(block, &face);
    let margin = if xforms.is_empty() {
        0.0
    } else {
        animator_margin(laid.size)
    };
    let (width, height) = laid.raster_size(margin);
    if laid.glyphs.is_empty() {
        return RasterText {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
    }
    let run = Run::Straight {
        origin: laid.origin(margin),
    };
    RasterText {
        width,
        height,
        rgba: draw(block, &face, &laid, &run, xforms, (width, height)),
    }
}

pub(crate) fn rasterise_on_path(
    block: &TextBlock<'_>,
    path: &MaskPolyline,
    offset: f32,
    width: u32,
    height: u32,
    xforms: &[GlyphXform],
) -> RasterText {
    let (w, h) = (width.max(1), height.max(1));
    if path.is_empty() || !path.length().is_finite() {
        return RasterText {
            width: w,
            height: h,
            rgba: vec![0u8; (w as usize) * (h as usize) * 4],
        };
    }
    let face = crate::fonts::face(&block.style.family, &block.style.face);
    let laid = lay(block, &face);
    let run = Run::Along { path, offset };
    RasterText {
        width: w,
        height: h,
        rgba: draw(block, &face, &laid, &run, xforms, (w, h)),
    }
}

pub(crate) fn outlines(
    block: &TextBlock<'_>,
    path: Option<&MaskPolyline>,
    offset: f32,
) -> Vec<GlyphOutline> {
    let face = crate::fonts::face(&block.style.family, &block.style.face);
    let Some(font) = face.font() else {
        return Vec::new();
    };
    let laid = lay(block, &face);
    let run = match path.filter(|p| !p.is_empty()) {
        Some(path) => Run::Along { path, offset },
        None => Run::Straight {
            origin: laid.origin(0.0),
        },
    };
    let glyphs = font.outline_glyphs();
    let location = LocationRef::new(face.shaper.normalized_coords());
    let chars: Vec<char> = block.text.chars().collect();
    let mut out = Vec::new();
    for (i, matrix, _) in placements(&laid, &run, &[], block.style.faux_italic) {
        let Some(glyph) = laid.glyphs.get(i) else {
            continue;
        };
        let Some(outline) = glyphs.get(skrifa::GlyphId::new(glyph.glyph)) else {
            continue;
        };
        let mut pen = crate::Outliner::new(|x, y| {
            let p = matrix.transform_point(zeno::Point::new(x, y));
            (f64::from(p.x), f64::from(p.y))
        });
        if outline
            .draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen)
            .is_err()
        {
            continue;
        }
        let contours = pen.finish();
        if !contours.is_empty() {
            out.push(GlyphOutline {
                ch: chars.get(glyph.ch).copied().unwrap_or(' '),
                contours,
            });
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use lumit_core::text::{ParagraphStyle, TextStyle};

    use super::*;

    const WHITE: LinearColour = LinearColour([1.0, 1.0, 1.0, 1.0]);

    /// Kerned Inter, which is enough to take the styled path.
    fn kerned() -> TextStyle {
        TextStyle {
            kerning: Kerning::Metrics,
            ..TextStyle::default()
        }
    }

    fn block<'a>(
        text: &'a str,
        style: &'a TextStyle,
        paragraph: &'a ParagraphStyle,
    ) -> TextBlock<'a> {
        TextBlock {
            text,
            size: 72.0,
            fill: WHITE,
            style,
            paragraph,
        }
    }

    /// The width of the first line's letters.
    fn line_width(text: &str, style: &TextStyle) -> f32 {
        let paragraph = ParagraphStyle::default();
        let l = layout(&block(text, style, &paragraph), false);
        let carets = &l.lines[0].carets;
        carets[carets.len() - 1] - carets[0]
    }

    fn ink(r: &RasterText) -> u64 {
        r.rgba.chunks_exact(4).map(|p| u64::from(p[3])).sum()
    }

    #[test]
    fn the_layout_box_is_the_rasters_own_size() {
        let paragraph = ParagraphStyle::default();
        let stroked = TextStyle {
            stroke_on: true,
            stroke_width: 9.0,
            baseline_shift: 14.0,
            ..kerned()
        };
        for style in [kerned(), stroked] {
            for text in ["Hello", "two\nlines", "gjpq", ""] {
                let b = block(text, &style, &paragraph);
                let r = rasterise(&b, &[]);
                let l = layout(&b, false);
                assert_eq!((l.width, l.height), (r.width, r.height), "{text:?}");
                let a = rasterise(&b, &[GlyphXform::default()]);
                let la = layout(&b, true);
                assert_eq!(
                    (la.width, la.height),
                    (a.width, a.height),
                    "{text:?} animated"
                );
            }
        }
    }

    #[test]
    fn styled_text_is_drawn_and_drawn_the_same_twice() {
        let (style, paragraph) = (kerned(), ParagraphStyle::default());
        let b = block("Lumit", &style, &paragraph);
        let a = rasterise(&b, &[]);
        assert!(ink(&a) > 10_000, "ink {}", ink(&a));
        assert_eq!(a.rgba, rasterise(&b, &[]).rgba);
    }

    #[test]
    fn empty_styled_text_is_one_transparent_pixel_with_a_caret() {
        let (style, paragraph) = (kerned(), ParagraphStyle::default());
        let b = block("", &style, &paragraph);
        let r = rasterise(&b, &[]);
        assert_eq!((r.width, r.height, r.rgba[3]), (1, 1, 0));
        let l = layout(&b, false);
        assert_eq!(l.lines.len(), 1);
        assert_eq!(l.lines[0].carets.len(), 1);
        assert!(l.lines[0].baseline > 0.0);
    }

    /// Inter kerns "AV" together, and switching kerning off gives the room back.
    #[test]
    fn kerning_pulls_a_pair_together() {
        let off = TextStyle {
            ligatures: false,
            ..TextStyle::default()
        };
        let on = TextStyle {
            ligatures: false,
            ..kerned()
        };
        assert!(line_width("AV", &on) < line_width("AV", &off) - 1.0);
    }

    /// 100 thousandths of 72 px is 7.2 px between each pair, and nothing after
    /// the last letter.
    #[test]
    fn tracking_adds_room_between_letters_only() {
        let tracked = TextStyle {
            tracking: 100.0,
            ..kerned()
        };
        let grown = line_width("HHHH", &tracked) - line_width("HHHH", &kerned());
        assert!((grown - 3.0 * 7.2).abs() < 0.01, "{grown}");
    }

    #[test]
    fn horizontal_scale_squeezes_the_line() {
        let half = TextStyle {
            scale_x: 50.0,
            ..kerned()
        };
        let (full, squeezed) = (line_width("Lumit", &kerned()), line_width("Lumit", &half));
        assert!((squeezed - full * 0.5).abs() < 0.01, "{squeezed} vs {full}");
    }

    #[test]
    fn all_caps_sets_the_capitals() {
        let caps = TextStyle {
            caps: Caps::All,
            ..kerned()
        };
        assert!((line_width("lumit", &caps) - line_width("LUMIT", &kerned())).abs() < 0.01);
        let small = TextStyle {
            caps: Caps::Small,
            ..kerned()
        };
        assert!(line_width("lumit", &small) < line_width("LUMIT", &kerned()) * 0.8);
    }

    /// A break starts a new line one leading further down, and each line knows
    /// which character it starts at, whichever line ending the text uses.
    #[test]
    fn a_break_starts_a_new_line_one_leading_down() {
        let paragraph = ParagraphStyle::default();
        let auto = layout(&block("one\ntwo", &kerned(), &paragraph), false);
        assert_eq!(auto.lines.len(), 2);
        assert_eq!((auto.lines[0].start, auto.lines[1].start), (0, 4));
        assert_eq!(auto.lines[0].carets.len(), 4);
        assert!((auto.lines[1].baseline - auto.lines[0].baseline - 72.0 * 1.2).abs() < 0.01);

        let set = TextStyle {
            leading: Some(100.0),
            ..kerned()
        };
        let spaced = ParagraphStyle {
            space_before: 6.0,
            space_after: 4.0,
            ..ParagraphStyle::default()
        };
        let l = layout(&block("one\r\ntwo\rthree", &set, &spaced), false);
        assert_eq!(
            l.lines.iter().map(|l| l.start).collect::<Vec<_>>(),
            [0, 5, 9]
        );
        assert!((l.lines[1].baseline - l.lines[0].baseline - 110.0).abs() < 0.01);
    }

    #[test]
    fn alignment_moves_the_short_line() {
        let start = |align| {
            let paragraph = ParagraphStyle {
                align,
                ..ParagraphStyle::default()
            };
            let l = layout(
                &block("a long first line\nshort", &kerned(), &paragraph),
                false,
            );
            (l.lines[0].carets[0], l.lines[1].carets[0])
        };
        let (left, centre, right) = (
            start(TextAlign::Left),
            start(TextAlign::Centre),
            start(TextAlign::Right),
        );
        // The longest line fills the block, so it never moves.
        assert!((left.0 - centre.0).abs() < 0.01 && (left.0 - right.0).abs() < 0.01);
        assert!((left.1 - left.0).abs() < 0.01);
        assert!(right.1 > centre.1 && centre.1 > left.1);
        assert!(((right.1 - left.1) * 0.5 - (centre.1 - left.1)).abs() < 0.01);

        let indented = ParagraphStyle {
            indent_left: 30.0,
            indent_first: 10.0,
            ..ParagraphStyle::default()
        };
        let l = layout(&block("short", &kerned(), &indented), false);
        assert!((l.lines[0].carets[0] - left.0 - 40.0).abs() < 0.01);
    }

    #[test]
    fn an_outline_adds_ink_and_room_for_it() {
        let paragraph = ParagraphStyle::default();
        let outlined = TextStyle {
            stroke_on: true,
            stroke: LinearColour([1.0, 0.0, 0.0, 1.0]),
            stroke_width: 8.0,
            ..kerned()
        };
        let plain = rasterise(&block("Lumit", &kerned(), &paragraph), &[]);
        let stroked = rasterise(&block("Lumit", &outlined, &paragraph), &[]);
        assert!(ink(&stroked) > ink(&plain));
        assert_eq!(stroked.width, plain.width + 8);
        // Some pixel is the outline's red, not the fill's white.
        assert!(stroked
            .rgba
            .chunks_exact(4)
            .any(|p| p[3] == 255 && p[0] == 255 && p[1] == 0));

        let hollow = TextStyle {
            fill_on: false,
            ..outlined
        };
        let ring = rasterise(&block("Lumit", &hollow, &paragraph), &[]);
        assert!(ring.rgba.chunks_exact(4).all(|p| p[3] == 0 || p[1] < 255));
    }

    #[test]
    fn faux_bold_and_superscript_change_the_ink() {
        let paragraph = ParagraphStyle::default();
        let regular = ink(&rasterise(&block("Lumit", &kerned(), &paragraph), &[]));
        let bold = TextStyle {
            faux_bold: true,
            ..kerned()
        };
        assert!(ink(&rasterise(&block("Lumit", &bold, &paragraph), &[])) > regular);
        let raised = TextStyle {
            script: Script::Superscript,
            ..kerned()
        };
        assert!(ink(&rasterise(&block("Lumit", &raised, &paragraph), &[])) < regular / 2);
    }

    /// A font this machine doesn't have draws in the built-in face, the same
    /// as a layer that never named one.
    #[test]
    fn a_missing_font_falls_back_to_the_built_in_face() {
        let paragraph = ParagraphStyle::default();
        let missing = TextStyle {
            family: "No Such Family 6f1c".into(),
            face: "Bold".into(),
            ..kerned()
        };
        let a = rasterise(&block("Lumit", &missing, &paragraph), &[]);
        let b = rasterise(&block("Lumit", &kerned(), &paragraph), &[]);
        assert_eq!((a.width, a.height), (b.width, b.height));
        assert_eq!(a.rgba, b.rgba);
    }

    #[test]
    fn an_animator_fades_a_styled_letter() {
        let paragraph = ParagraphStyle::default();
        let style = kerned();
        let b = block("HH", &style, &paragraph);
        let still = rasterise(&b, &[GlyphXform::default(), GlyphXform::default()]);
        let gone = GlyphXform {
            opacity: 0.0,
            ..GlyphXform::default()
        };
        let faded = rasterise(&b, &[GlyphXform::default(), gone]);
        assert_eq!((still.width, still.height), (faded.width, faded.height));
        let (whole, half) = (ink(&still), ink(&faded));
        assert!(half * 2 > whole - whole / 10 && half * 2 < whole + whole / 10);
    }

    #[test]
    fn styled_text_runs_along_a_path_and_converts_to_outlines() {
        let paragraph = ParagraphStyle::default();
        let style = TextStyle {
            stroke_on: true,
            stroke_width: 3.0,
            ..kerned()
        };
        let b = block("Lumit", &style, &paragraph);
        let path = MaskPolyline {
            points: vec![[20.0, 120.0], [400.0, 120.0]],
            arc: vec![0.0, 380.0],
            closed: false,
            feather: 0.0,
            expansion: 0.0,
        };
        let r = rasterise_on_path(&b, &path, 0.0, 420, 200, &[]);
        assert_eq!((r.width, r.height), (420, 200));
        assert!(ink(&r) > 10_000);
        // The ink sits on the path's baseline, not at the top of the box.
        let top: u64 = r.rgba[..420 * 40 * 4]
            .chunks_exact(4)
            .map(|p| u64::from(p[3]))
            .sum();
        assert_eq!(top, 0);

        assert_eq!(outlines(&b, None, 0.0).len(), 5);
        let items = crate::shape_items(&b, None, 0.0);
        assert!(items
            .iter()
            .all(|i| i.stroke.is_some() && i.stroke_width == 3.0));
    }
}
