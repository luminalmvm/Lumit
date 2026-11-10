//! Plates: the box behind a block or its lines, and the boxes an animator puts
//! behind the words it reaches. Both go down before any letter.

use lumit_core::model::LinearColour;
use lumit_core::text::GlyphXform;
use zeno::{Command, Fill, PathBuilder, Transform};

use super::{finite, paint, Laid, TextBlock};

/// One rounded box from `r`, left, top, right and bottom in raster px.
fn rounded(
    rgba: &mut [u8],
    (w, h): (u32, u32),
    r: [f32; 4],
    radius: f32,
    colour: [u8; 4],
    alpha: f32,
) {
    let radius = radius
        .min((r[2] - r[0]) * 0.5)
        .min((r[3] - r[1]) * 0.5)
        .max(0.0);
    // A box far past the raster is cut to it first, so a huge padding can't
    // ask for a huge mask. The cut edges are outside the picture anyway.
    #[allow(clippy::cast_precision_loss)] // raster sizes, far inside f32
    let (most_x, most_y) = (w as f32 + radius + 1.0, h as f32 + radius + 1.0);
    let least = -radius - 1.0;
    let (x0, y0) = (r[0].max(least), r[1].max(least));
    let (x1, y1) = (r[2].min(most_x), r[3].min(most_y));
    if !(x1 > x0 && y1 > y0) {
        return;
    }
    let mut path: Vec<Command> = Vec::new();
    path.add_round_rect([x0, y0], x1 - x0, y1 - y0, radius, radius);
    paint(
        rgba,
        (w, h),
        &path,
        Transform::IDENTITY,
        Fill::NonZero,
        colour,
        alpha,
    );
}

/// The leftmost and rightmost of some carets, or nothing when they enclose no
/// room.
fn span(carets: &[f32]) -> Option<(f32, f32)> {
    let left = carets.iter().copied().fold(f32::INFINITY, f32::min);
    let right = carets.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    (right > left).then_some((left, right))
}

pub(super) fn draw(
    rgba: &mut [u8],
    size: (u32, u32),
    block: &TextBlock<'_>,
    laid: &Laid,
    origin: [f32; 2],
    xforms: &[GlyphXform],
) {
    let style = block.style;
    let pad = finite(style.plate_padding).max(0.0);
    let radius = finite(style.plate_radius).max(0.0);
    // A line's box, from its carets and the font's ascent and descent.
    let line_box = |baseline: f32, (left, right): (f32, f32)| {
        [
            origin[0] + left - pad,
            origin[1] + baseline - laid.ascent - pad,
            origin[0] + right + pad,
            origin[1] + baseline + laid.descent + pad,
        ]
    };

    if style.plate_on {
        let colour = lumit_core::pixels::solid_rgba(style.plate);
        let boxes = laid
            .lines
            .iter()
            .filter_map(|line| Some(line_box(line.baseline, span(&line.carets)?)));
        if style.plate_lines {
            for r in boxes {
                rounded(rgba, size, r, radius, colour, 1.0);
            }
        } else {
            let whole = boxes.reduce(|a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            });
            if let Some(r) = whole {
                rounded(rgba, size, r, radius, colour, 1.0);
            }
        }
    }

    if !xforms.iter().any(|x| x.plate[3] > 0.0) {
        return;
    }
    // Each run of boxed letters with no space between them is one word, and
    // gets one box in the colour of its strongest letter.
    let chars: Vec<char> = block.text.chars().collect();
    let boxed = |ch: usize| {
        xforms
            .get(ch)
            .filter(|x| x.plate[3] > 0.0)
            .filter(|_| chars.get(ch).is_some_and(|c| !c.is_whitespace()))
            .map(|x| x.plate)
    };
    for line in &laid.lines {
        let letters = line.carets.len().saturating_sub(1);
        let mut run: Option<(usize, [f32; 4])> = None;
        for k in 0..=letters {
            let plate = (k < letters).then(|| boxed(line.start + k)).flatten();
            match (plate, run) {
                (Some(p), Some((from, strongest))) => {
                    run = Some((from, if p[3] > strongest[3] { p } else { strongest }));
                }
                (Some(p), None) => run = Some((k, p)),
                (None, Some((from, p))) => {
                    run = None;
                    let Some(edges) = line.carets.get(from..=k).and_then(span) else {
                        continue;
                    };
                    let colour =
                        lumit_core::pixels::solid_rgba(LinearColour([p[0], p[1], p[2], 1.0]));
                    rounded(
                        rgba,
                        size,
                        line_box(line.baseline, edges),
                        radius,
                        colour,
                        p[3],
                    );
                }
                (None, None) => {}
            }
        }
    }
}
